use quillship::{
    commands,
    db::Store,
    document::{Document, RemoteDocument},
    journal::{recover, Intent, State},
    notion::{CreateParent, Notion},
};
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

const ID: &str = "b55c9c91-384d-452b-81db-d1ef79372b75";
fn document() -> Document {
    let mut doc = Document::from_remote(RemoteDocument {
        id: ID.into(),
        title: "Notes".into(),
        url: "url".into(),
        last_edited_time: "time".into(),
        markdown: "base".into(),
        sync_warnings: vec![],
    });
    doc.markdown = "submitted".into();
    doc
}
fn client(server: &MockServer) -> Notion {
    Notion::new("test", &format!("{}/v1", server.uri())).unwrap()
}
async fn snapshot(server: &MockServer, body: &str) {
    Mock::given(method("GET")).and(path(format!("/v1/pages/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":ID,"url":"url","last_edited_time":"time","properties":{"title":{"title":[{"text":{"content":"Notes"}}]}}})))
        .expect(2).mount(server).await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}/markdown")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"markdown":body,"truncated":false,"unknown_block_ids":[]})),
        )
        .expect(1)
        .mount(server)
        .await;
}

#[test]
fn every_transition_survives_restart_and_terminal_states_cannot_be_reopened() {
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let mut op = store
        .prepare(
            Intent::Push {
                document: document(),
            },
            Some(99),
        )
        .unwrap();
    for state in [
        State::Prepared,
        State::Sent,
        State::Uncertain,
        State::RemoteConfirmed,
        State::LocallySaved,
    ] {
        if state != State::Prepared {
            store.transition(&mut op, state).unwrap();
        }
        drop(store);
        store = Store::open(dir.path()).unwrap();
        op = store.operation(op.id).unwrap();
        assert_eq!(op.state, state);
        assert_eq!(op.batch_id, Some(99));
    }
    assert!(store.transition(&mut op, State::Sent).is_err());
    assert!(!op.pending());
}

#[tokio::test]
async fn uncertain_creation_cannot_be_replayed_by_recovery() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({"message":"failure"})))
        .expect(1)
        .mount(&server)
        .await;
    assert!(commands::create_document(
        &store,
        &client(&server),
        &CreateParent::Workspace,
        "Notes",
        "authored"
    )
    .await
    .is_err());
    let op = store.operations().unwrap().pop().unwrap();
    assert_eq!(op.state, State::Uncertain);
    assert!(recover(&store, &client(&server), op.id).await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    match op.intent {
        Intent::Create { markdown, .. } => assert_eq!(markdown, "authored"),
        _ => panic!("wrong input"),
    }
}

#[tokio::test]
async fn confirmed_create_with_failed_readback_recovers_identity_and_authored_input() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/pages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"object":"page","id":ID,"url":"url"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(commands::create_document(
        &store,
        &client(&server),
        &CreateParent::Workspace,
        "Notes",
        "authored"
    )
    .await
    .is_err());
    let op = store.operations().unwrap().pop().unwrap();
    assert_eq!(op.state, State::RemoteConfirmed);
    assert_eq!(op.remote_id.as_deref(), Some(ID));
    snapshot(&server, "normalized").await;
    let op = recover(&store, &client(&server), op.id).await.unwrap();
    assert_eq!(op.state, State::LocallySaved);
    let doc = store.get(ID).unwrap();
    assert_eq!(doc.markdown, "authored");
    assert_eq!(doc.baseline, "normalized");
}

#[tokio::test]
async fn push_recovery_retains_edits_made_after_interruption() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let doc = document();
    store.save(&doc, None).unwrap();
    let mut op = store.prepare(Intent::Push { document: doc }, None).unwrap();
    store.transition(&mut op, State::Sent).unwrap();
    op.remote_body = Some("normalized".into());
    store.transition(&mut op, State::RemoteConfirmed).unwrap();
    std::fs::write(store.path(ID).unwrap(), "later edit").unwrap();
    let server = MockServer::start().await;
    snapshot(&server, "normalized").await;
    recover(&store, &client(&server), op.id).await.unwrap();
    assert_eq!(store.get(ID).unwrap().markdown, "later edit");
    assert_eq!(store.get(ID).unwrap().baseline, "normalized");
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method == "GET"));
}

#[tokio::test]
async fn batch_retains_failed_and_unattempted_items() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let doc = document();
    store.save(&doc, None).unwrap();
    let mut second = doc.clone();
    second.id = "b55c9c91-384d-452b-81db-d1ef79372b76".into();
    store.save(&second, None).unwrap();
    let server = MockServer::start().await;
    assert!(
        commands::push_batch(&store, &client(&server), &[doc, second])
            .await
            .is_err()
    );
    let ops = store.operations().unwrap();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].state, State::Failed);
    assert_eq!(ops[1].state, State::Prepared);
    assert_eq!(ops[0].batch_id, ops[1].batch_id);
}

#[tokio::test]
async fn corrupt_journal_prevents_recovery_network_calls() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    // Corrupt journal data must fail closed before observation or mutation.
    let op = store
        .prepare(
            Intent::Push {
                document: document(),
            },
            None,
        )
        .unwrap();
    drop(store);
    let db = sled::open(dir.path().join("store")).unwrap();
    db.open_tree("operations-v1")
        .unwrap()
        .insert(op.id.to_be_bytes(), b"corrupt")
        .unwrap();
    db.flush().unwrap();
    drop(db);
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    assert!(recover(&store, &client(&server), op.id).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
}
