use quillship::{
    db::Store,
    document::{Document, RemoteDocument},
    journal::{self, Intent, State},
    notion::Notion,
    targeted::{self, Edit, Position},
};
use serde_json::{json, Value};
use std::{
    fs,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tempfile::TempDir;
use wiremock::{
    matchers::{body_json, method, path},
    Mock, MockServer, Request, Respond, ResponseTemplate,
};
const ID: &str = "b55c9c91-384d-452b-81db-d1ef79372b75";

#[tokio::test]
async fn recovery_installs_selected_edit_without_repeating_it() {
    let (_dir, store) = workspace("base");
    let server = MockServer::start().await;
    reads(&server, vec!["new"], None).await;
    let mut op = store
        .prepare(
            Intent::Edit {
                document: store.get(ID).unwrap(),
                edit: Edit::Replace {
                    old: "base".into(),
                    new: "new".into(),
                    all_matches: false,
                },
                selected: "new".into(),
            },
            None,
        )
        .unwrap();
    store.transition(&mut op, State::Sent).unwrap();
    op.remote_body = Some("new".into());
    store.transition(&mut op, State::RemoteConfirmed).unwrap();
    journal::recover(&store, &client(&server), op.id)
        .await
        .unwrap();
    assert_eq!(store.get(ID).unwrap().markdown, "new");
    assert!(!store.get(ID).unwrap().dirty());
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method == "GET"));
}

#[tokio::test]
async fn edits_during_remote_write_survive_readback() {
    let (_dir, store) = workspace("base");
    let server = MockServer::start().await;
    reads(&server, vec!["base", "new"], None).await;
    let working = store.path(ID).unwrap();
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/pages/{ID}/markdown")))
        .respond_with(move |_: &Request| {
            fs::write(&working, "concurrent").unwrap();
            ResponseTemplate::new(200).set_body_json(markdown("new"))
        })
        .expect(1)
        .mount(&server)
        .await;
    targeted::run(
        &store,
        Some(&client(&server)),
        ID,
        Edit::Replace {
            old: "base".into(),
            new: "new".into(),
            all_matches: false,
        },
        false,
    )
    .await
    .unwrap();
    assert_eq!(store.get(ID).unwrap().markdown, "concurrent");
    assert_eq!(store.get(ID).unwrap().baseline, "new");
}
fn workspace(body: &str) -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .save(
            &Document::from_remote(RemoteDocument {
                id: ID.into(),
                title: "Notes".into(),
                url: "url".into(),
                last_edited_time: "time".into(),
                markdown: body.into(),
                sync_warnings: vec![],
            }),
            None,
        )
        .unwrap();
    (dir, store)
}
fn client(server: &MockServer) -> Notion {
    Notion::new("test", &format!("{}/v1", server.uri())).unwrap()
}
fn markdown(body: &str) -> Value {
    json!({"markdown":body,"truncated":false,"unknown_block_ids":[]})
}
struct ReadBodies {
    bodies: Vec<String>,
    count: Arc<AtomicUsize>,
    edit: Option<std::path::PathBuf>,
}
impl Respond for ReadBodies {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        if let Some(path) = &self.edit {
            fs::write(path, "concurrent").unwrap();
        }
        let index = self.count.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200).set_body_json(markdown(&self.bodies[index]))
    }
}
async fn reads(server: &MockServer, bodies: Vec<&str>, edit: Option<std::path::PathBuf>) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"id":ID,"url":"url","last_edited_time":"time","properties":{}}),
            ),
        )
        .expect((bodies.len() * 2) as u64)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}/markdown")))
        .respond_with(ReadBodies {
            bodies: bodies.iter().map(|s| s.to_string()).collect(),
            count: Arc::new(AtomicUsize::new(0)),
            edit,
        })
        .expect(bodies.len() as u64)
        .mount(server)
        .await;
}

#[test]
fn matching_is_exact_and_explicit_about_ambiguity() {
    let replace = |old: &str, new: &str, all_matches| Edit::Replace {
        old: old.into(),
        new: new.into(),
        all_matches,
    };
    assert!(replace("", "x", false).apply("body").is_err());
    assert!(replace("missing", "x", false).apply("body").is_err());
    assert!(replace("a", "b", false).apply("a a").is_err());
    assert_eq!(replace("a", "λ", true).apply("a\r\na").unwrap(), "λ\r\nλ");
    assert!(replace("a", "a", false).apply("a").is_err());
}

#[tokio::test]
async fn offline_preview_has_no_network_or_journal_and_dirty_input_is_refused() {
    let (_dir, store) = workspace("base");
    let edit = Edit::Append {
        content: "more".into(),
        position: Position::End,
    };
    let preview = targeted::run(&store, None, ID, edit.clone(), true)
        .await
        .unwrap();
    assert_eq!(preview.preview, "base\n\nmore");
    assert!(!preview.remote_checked);
    assert!(store.operations().unwrap().is_empty());
    fs::write(store.path(ID).unwrap(), "dirty").unwrap();
    assert!(targeted::run(&store, None, ID, edit, true).await.is_err());
}

#[tokio::test]
async fn append_uses_insert_api_and_retains_input_when_normalized() {
    let (_dir, store) = workspace("base");
    let server = MockServer::start().await;
    reads(&server, vec!["base", "base\nmore"], None).await;
    Mock::given(method("PATCH")).and(path(format!("/v1/pages/{ID}/markdown")))
        .and(body_json(json!({"type":"insert_content","markdown_version":"v2","allow_async":false,"insert_content":{"content":"more","position":{"type":"end"}}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(markdown("base\nmore"))).expect(1).mount(&server).await;
    let result = targeted::run(
        &store,
        Some(&client(&server)),
        ID,
        Edit::Append {
            content: "more".into(),
            position: Position::End,
        },
        false,
    )
    .await
    .unwrap();
    assert!(result.dirty);
    assert_eq!(result.preview, "base\n\nmore");
    assert_eq!(result.baseline, "base\nmore");
    assert_eq!(store.operations().unwrap()[0].state, State::LocallySaved);
}

#[tokio::test]
async fn replacement_keeps_child_page_deletion_disabled() {
    let before = "old\n<page url=\"https://notion.so/child\"/>";
    let after = before.replace("old", "new");
    let (_dir, store) = workspace(before);
    let server = MockServer::start().await;
    reads(&server, vec![before, &after], None).await;
    Mock::given(method("PATCH")).and(path(format!("/v1/pages/{ID}/markdown")))
        .and(body_json(json!({"type":"update_content","markdown_version":"v2","allow_async":false,"update_content":{"content_updates":[{"old_str":"old","new_str":"new","replace_all_matches":false}],"allow_deleting_content":false}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(markdown(&after))).expect(1).mount(&server).await;
    let result = targeted::run(
        &store,
        Some(&client(&server)),
        ID,
        Edit::Replace {
            old: "old".into(),
            new: "new".into(),
            all_matches: false,
        },
        false,
    )
    .await
    .unwrap();
    assert!(!result.dirty);
    assert_eq!(store.get(ID).unwrap().markdown, after);
}

#[tokio::test]
async fn concurrent_local_preflight_edit_and_remote_divergence_prevent_submission() {
    for local_edit in [false, true] {
        let (_dir, store) = workspace("base");
        let server = MockServer::start().await;
        reads(
            &server,
            vec![if local_edit { "base" } else { "diverged" }],
            if local_edit {
                Some(store.path(ID).unwrap())
            } else {
                None
            },
        )
        .await;
        assert!(targeted::run(
            &store,
            Some(&client(&server)),
            ID,
            Edit::Append {
                content: "more".into(),
                position: Position::End
            },
            false
        )
        .await
        .is_err());
        assert!(server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET"));
        assert_eq!(store.get(ID).unwrap().baseline, "base");
        assert!(store.operations().unwrap().is_empty());
    }
}

#[tokio::test]
async fn warning_after_mutation_is_uncertain_and_keeps_original_baseline() {
    let (_dir, store) = workspace("base");
    let server = MockServer::start().await;
    reads(&server, vec!["base"], None).await;
    let mut response = markdown("new");
    response["warnings"] = json!([{"message":"partial edit"}]);
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/pages/{ID}/markdown")))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .expect(1)
        .mount(&server)
        .await;
    assert!(targeted::run(
        &store,
        Some(&client(&server)),
        ID,
        Edit::Replace {
            old: "base".into(),
            new: "new".into(),
            all_matches: false
        },
        false
    )
    .await
    .is_err());
    assert_eq!(store.get(ID).unwrap().baseline, "base");
    assert_eq!(store.operations().unwrap()[0].state, State::Uncertain);
}

#[tokio::test]
async fn changed_readback_keeps_confirmed_result_for_recovery() {
    let (_dir, store) = workspace("base");
    let server = MockServer::start().await;
    reads(&server, vec!["base", "later edit"], None).await;
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/pages/{ID}/markdown")))
        .respond_with(ResponseTemplate::new(200).set_body_json(markdown("new")))
        .expect(1)
        .mount(&server)
        .await;
    assert!(targeted::run(
        &store,
        Some(&client(&server)),
        ID,
        Edit::Replace {
            old: "base".into(),
            new: "new".into(),
            all_matches: false
        },
        false
    )
    .await
    .is_err());
    assert_eq!(store.get(ID).unwrap().markdown, "base");
    let op = store.operations().unwrap().pop().unwrap();
    assert_eq!(op.state, State::RemoteConfirmed);
    assert_eq!(op.remote_body.as_deref(), Some("new"));
}
