use std::fs;
use std::process::Command;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use quillship::{
    commands,
    db::Store,
    document::{page_id, Document, RemoteDocument},
    notion::{Notion, API_VERSION},
};
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{body_json, header, method, path, query_param},
    Mock, MockServer, Request, Respond, ResponseTemplate,
};

const ID: &str = "b55c9c91-384d-452b-81db-d1ef79372b75";
const OTHER: &str = "b55c9c91-384d-452b-81db-d1ef79372b76";

fn remote(body: &str) -> RemoteDocument {
    RemoteDocument {
        id: ID.into(),
        title: "Design notes".into(),
        url: format!("https://www.notion.so/{ID}"),
        last_edited_time: "2026-11-24T10:00:00.000Z".into(),
        markdown: body.into(),
        sync_warnings: Vec::new(),
    }
}

fn workspace(body: &str) -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .save(&Document::from_remote(remote(body)), None)
        .unwrap();
    (dir, store)
}

fn edit(store: &Store, body: &str) -> Document {
    fs::write(store.path(ID).unwrap(), body).unwrap();
    store.get(ID).unwrap()
}

fn markdown(body: &str) -> Value {
    json!({"object": "page_markdown", "id": ID, "markdown": body, "truncated": false, "unknown_block_ids": []})
}

fn metadata(time: &str) -> Value {
    json!({"object": "page", "id": ID, "url": format!("https://www.notion.so/{ID}"),
        "last_edited_time": time, "archived": false, "in_trash": false,
        "properties": {"Name": {"type": "title", "title": [{"plain_text": "Design "}, {"text": {"content": "notes"}}]}}})
}

async fn snapshot(server: &MockServer, body: &str, gets: u64) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}")))
        .and(header("authorization", "Bearer test-token"))
        .and(header("Notion-Version", API_VERSION))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(metadata("2026-11-24T10:00:00.000Z")),
        )
        .expect(gets * 2)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}/markdown")))
        .and(query_param("markdown_version", "v2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(markdown(body)))
        .expect(gets)
        .mount(server)
        .await;
}

fn client(server: &MockServer) -> Notion {
    Notion::new("test-token", &format!("{}/v1", server.uri())).unwrap()
}

fn cli(dir: &TempDir, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_quillship"))
        .arg("--workspace")
        .arg(dir.path())
        .args(args)
        .env_remove("NOTION_TOKEN")
        .output()
        .unwrap()
}

#[test]
fn uuid_identity_is_canonical_and_cannot_escape_workspace() {
    assert_eq!(page_id("b55c9c91384d452b81dbd1ef79372b75").unwrap(), ID);
    for value in ["../escape", "", "not-a-page", "../../docs/a.md"] {
        assert!(page_id(value).is_err());
    }
}

#[test]
fn persistence_and_direct_file_edits_survive_reopen() {
    let (dir, store) = workspace("Original\n");
    drop(store);
    fs::write(dir.path().join(format!("docs/{ID}.md")), "LLM edited: λ\n").unwrap();
    let store = Store::open(dir.path()).unwrap();
    let doc = store.get(ID).unwrap();
    assert_eq!(doc.markdown, "LLM edited: λ\n");
    assert_eq!(doc.baseline, "Original\n");
    assert!(doc.dirty());
    drop(store);
    assert!(Store::open(dir.path()).unwrap().get(ID).unwrap().dirty());
}

#[test]
fn missing_working_file_is_not_an_empty_edit() {
    let (_dir, store) = workspace("Important");
    fs::remove_file(store.path(ID).unwrap()).unwrap();
    assert!(store
        .get(ID)
        .unwrap_err()
        .to_string()
        .contains("stored body remains"));
}

#[test]
fn concurrent_external_edit_is_not_overwritten() {
    let (_dir, store) = workspace("Original");
    let previous = store.get(ID).unwrap();
    fs::write(store.path(ID).unwrap(), "External edit").unwrap();
    let mut next = previous.clone();
    next.markdown = "Replacement".into();
    assert!(store.save(&next, Some(&previous)).is_err());
    assert_eq!(store.get(ID).unwrap().markdown, "External edit");
}

#[test]
fn untracked_working_files_are_preserved() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    fs::write(store.path(ID).unwrap(), "Untracked work").unwrap();
    assert!(store
        .save(&Document::from_remote(remote("Remote")), None)
        .is_err());
    assert_eq!(
        fs::read_to_string(store.path(ID).unwrap()).unwrap(),
        "Untracked work"
    );
}

#[test]
fn pull_three_way_rules() {
    let clean = Document::from_remote(remote("Original"));
    assert_eq!(
        clean.pulled(remote("Remote edit")).unwrap().markdown,
        "Remote edit"
    );
    let mut dirty = clean.clone();
    dirty.markdown = "Local edit".into();
    let retained = dirty.pulled(remote("Original")).unwrap();
    assert_eq!(retained.markdown, "Local edit");
    assert!(retained.dirty());
    assert!(!dirty.pulled(remote("Local edit")).unwrap().dirty());
    assert!(dirty.pulled(remote("Remote edit")).is_err());
}

#[test]
fn offline_cli_search_view_edit_status_and_dry_run() {
    let (dir, store) = workspace("Original\n");
    drop(store);
    let input = dir.path().join("input.md");
    fs::write(&input, "# LLM update\n\nRésumé about SHIPS\n").unwrap();
    assert!(cli(&dir, &["edit", ID, "--file", input.to_str().unwrap()])
        .status
        .success());
    let view = cli(&dir, &["view", ID]);
    assert!(view.status.success());
    assert_eq!(
        String::from_utf8(view.stdout).unwrap(),
        fs::read_to_string(input).unwrap()
    );
    let baseline = cli(&dir, &["view", ID, "--baseline"]);
    assert_eq!(baseline.stdout, b"Original\n");
    for args in [
        vec!["search", "ships", "--json"],
        vec!["search", "DESIGN", "--json"],
        vec!["status", "--json"],
        vec!["list", "--json"],
    ] {
        let output = cli(&dir, &args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(rows[0]["id"], ID);
        assert_eq!(rows[0]["dirty"], true);
    }
    assert!(cli(&dir, &["push", "--dry-run"]).status.success());
    assert!(cli(&dir, &["path", ID]).status.success());
    assert!(!cli(&dir, &["view", "../../escape"]).status.success());
    assert!(!cli(&dir, &["search", " "]).status.success());
}

#[tokio::test]
async fn pull_edit_push_complete_workflow() {
    let server = MockServer::start().await;
    snapshot(&server, "Original\n", 2).await;
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let notion = client(&server);
    commands::pull(&store, &notion, &[ID.into(), ID.into()], None)
        .await
        .unwrap();
    let doc = edit(&store, "# Edited\n\nLocal content\n");
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/pages/{ID}/markdown")))
        .and(body_json(
            json!({"type": "replace_content", "markdown_version": "v2", "allow_async": false,
            "replace_content": {"new_str": doc.markdown, "allow_deleting_content": false}}),
        ))
        .and(header("authorization", "Bearer test-token"))
        .and(header("Notion-Version", API_VERSION))
        .respond_with(ResponseTemplate::new(200).set_body_json(markdown(&doc.markdown)))
        .expect(1)
        .mount(&server)
        .await;
    commands::push_document(&store, &notion, &doc)
        .await
        .unwrap();
    assert!(!store.get(ID).unwrap().dirty());
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.get(ID).unwrap().markdown, doc.markdown);
    assert!(commands::pending(&store, &[]).unwrap().is_empty());
    // A clean push causes no requests.
    commands::push_document(&store, &notion, &store.get(ID).unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn divergent_pull_and_push_preserve_local_content() {
    let server = MockServer::start().await;
    snapshot(&server, "Remote edit", 2).await;
    let (_dir, store) = workspace("Original");
    let doc = edit(&store, "Local edit");
    let notion = client(&server);
    let report = commands::pull(&store, &notion, &[ID.into()], None)
        .await
        .unwrap();
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].1.contains("conflict"));
    assert!(commands::push_document(&store, &notion, &doc)
        .await
        .unwrap_err()
        .to_string()
        .contains("conflict"));
    assert_eq!(store.get(ID).unwrap(), doc);
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method.as_str() != "PATCH"));
}

#[tokio::test]
async fn unchanged_remote_pull_retains_dirty_working_file() {
    let server = MockServer::start().await;
    snapshot(&server, "Original", 1).await;
    let (_dir, store) = workspace("Original");
    let doc = edit(&store, "Local edit");
    commands::pull(&store, &client(&server), &[ID.into()], None)
        .await
        .unwrap();
    assert_eq!(store.get(ID).unwrap(), doc);
}

#[tokio::test]
async fn already_pushed_content_converges_without_a_write() {
    let server = MockServer::start().await;
    snapshot(&server, "Local edit", 1).await;
    let (_dir, store) = workspace("Original");
    let doc = edit(&store, "Local edit");
    commands::push_document(&store, &client(&server), &doc)
        .await
        .unwrap();
    assert!(!store.get(ID).unwrap().dirty());
}

#[tokio::test]
async fn failed_push_preserves_baseline_and_is_not_retried() {
    let server = MockServer::start().await;
    snapshot(&server, "Original", 1).await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    let (_dir, store) = workspace("Original");
    let doc = edit(&store, "Local edit");
    assert!(commands::push_document(&store, &client(&server), &doc)
        .await
        .is_err());
    assert_eq!(store.get(ID).unwrap(), doc);
}

#[tokio::test]
async fn normalized_response_preserves_exact_local_markdown() {
    let server = MockServer::start().await;
    snapshot(&server, "Original", 1).await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(200).set_body_json(markdown("Local edit\n")))
        .expect(1)
        .mount(&server)
        .await;
    let (_dir, store) = workspace("Original");
    let doc = edit(&store, "Local edit");
    commands::push_document(&store, &client(&server), &doc)
        .await
        .unwrap();
    let after = store.get(ID).unwrap();
    assert_eq!(after.markdown, "Local edit");
    assert_eq!(after.baseline, "Local edit\n");
    assert!(after.dirty());
}

#[tokio::test]
async fn partial_pull_keeps_complete_snapshot_even_on_repeated_pulls() {
    for response in [
        json!({"markdown": "Partial", "truncated": true, "unknown_block_ids": []}),
        json!({"markdown": "Partial", "truncated": false, "unknown_block_ids": [OTHER]}),
        markdown("<unknown url=\"example\"/>"),
        json!({"markdown": "Partial"}),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/pages/{ID}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(metadata("time")))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/pages/{ID}/markdown")))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&server)
            .await;
        let (_dir, store) = workspace("Original");
        let before = store.get(ID).unwrap();
        for _ in 0..2 {
            let report = commands::pull(&store, &client(&server), &[ID.into()], None)
                .await
                .unwrap();
            assert_eq!(report.pulled, 1);
            assert_eq!(report.incomplete, 1);
            assert!(report.failures.is_empty());
            let after = store.get(ID).unwrap();
            assert_eq!(after.markdown, before.markdown);
            assert_eq!(after.baseline, before.baseline);
            assert!(!after.incomplete);
            assert!(!after.sync_warnings.is_empty());
        }
        assert!(client(&server).retrieve(ID).await.is_err());
    }
}

#[tokio::test]
async fn new_partial_document_is_saved_searchable_and_never_pushed() {
    let server = MockServer::start().await;
    let body = "Readable notes\n\n<unknown url=\"example\"/>";
    snapshot(&server, body, 1).await;
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let report = commands::pull(&store, &client(&server), &[ID.into()], None)
        .await
        .unwrap();
    assert_eq!(report.pulled, 1);
    assert_eq!(report.incomplete, 1);
    assert!(report.failures.is_empty());
    let doc = store.get(ID).unwrap();
    assert!(doc.incomplete);
    assert_eq!(doc.markdown, body);
    drop(store);
    let output = cli(&dir, &["search", "Readable", "--json"]);
    assert!(output.status.success());
    let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows[0]["incomplete"], true);
    assert!(!rows[0]["sync_warnings"].as_array().unwrap().is_empty());
    let view = cli(&dir, &["view", ID]);
    assert_eq!(view.stdout, body.as_bytes());
    assert!(String::from_utf8_lossy(&view.stderr).contains("pushing disabled"));
    let store = Store::open(dir.path()).unwrap();
    // Removing the placeholders locally must not bypass the saved protection.
    let edited = edit(&store, "Locally rewritten partial document");
    let before_requests = server.received_requests().await.unwrap().len();
    assert!(commands::push_document(&store, &client(&server), &edited)
        .await
        .unwrap_err()
        .to_string()
        .contains("incomplete"));
    assert_eq!(
        before_requests,
        server.received_requests().await.unwrap().len()
    );
    assert_eq!(store.get(ID).unwrap(), edited);
    drop(store);
    let dry_run = cli(&dir, &["push", "--dry-run"]);
    assert!(dry_run.status.success());
    assert!(String::from_utf8_lossy(&dry_run.stdout).contains("blocked (incomplete)"));
}

#[tokio::test]
async fn page_error_does_not_prevent_later_page_from_pulling() {
    let server = MockServer::start().await;
    snapshot(&server, "Complete notes", 1).await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{OTHER}")))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let report = commands::pull(&store, &client(&server), &[OTHER.into(), ID.into()], None)
        .await
        .unwrap();
    assert_eq!(report.pulled, 1);
    assert_eq!(report.incomplete, 0);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].0, OTHER);
    assert!(report.failures[0].1.contains("404"));
    assert_eq!(store.get(ID).unwrap().markdown, "Complete notes");
}

#[test]
fn old_records_remain_readable_and_partial_snapshots_can_be_promoted() {
    let original = Document::from_remote(remote("Original"));
    let mut legacy = serde_json::to_value(&original).unwrap();
    legacy.as_object_mut().unwrap().remove("incomplete");
    legacy.as_object_mut().unwrap().remove("sync_warnings");
    assert_eq!(
        serde_json::from_value::<Document>(legacy).unwrap(),
        original
    );
    let mut partial = remote("Readable excerpt");
    partial.sync_warnings.push("Missing blocks".into());
    let mut doc = Document::from_remote(partial);
    assert!(doc.incomplete);
    let complete = doc.pulled(remote("Full document")).unwrap();
    assert!(!complete.incomplete);
    assert!(complete.sync_warnings.is_empty());
    assert_eq!(complete.markdown, "Full document");
    doc.markdown = "Local edit to excerpt".into();
    assert!(doc.pulled(remote("Full document")).is_err());
}

#[tokio::test]
async fn malformed_page_body_is_reported_and_batch_continues() {
    let server = MockServer::start().await;
    snapshot(&server, "Complete notes", 1).await;
    let mut other = metadata("time");
    other["id"] = json!(OTHER);
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{OTHER}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(other))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{OTHER}/markdown")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"truncated": false})))
        .mount(&server)
        .await;
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let report = commands::pull(&store, &client(&server), &[OTHER.into(), ID.into()], None)
        .await
        .unwrap();
    assert_eq!(report.pulled, 1);
    assert_eq!(report.failures.len(), 1);
    assert!(!store.contains(OTHER).unwrap());
    assert_eq!(store.get(ID).unwrap().markdown, "Complete notes");
}

#[tokio::test]
async fn warnings_or_missing_completeness_flags_are_saved_as_partial() {
    for response in [
        json!({"markdown": "Readable", "truncated": true, "unknown_block_ids": []}),
        json!({"markdown": "Readable", "truncated": false, "unknown_block_ids": [OTHER]}),
        json!({"markdown": "Readable"}),
        json!({"markdown": "Readable", "truncated": false, "unknown_block_ids": [], "warnings": [{"code": "unsupported", "message": "Missing block"}]}),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/pages/{ID}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(metadata("time")))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/pages/{ID}/markdown")))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&server)
            .await;
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let report = commands::pull(&store, &client(&server), &[ID.into()], None)
            .await
            .unwrap();
        assert_eq!(report.incomplete, 1);
        assert!(report.failures.is_empty());
        assert!(store.get(ID).unwrap().incomplete);
        assert_eq!(store.get(ID).unwrap().markdown, "Readable");
    }
}

#[tokio::test]
async fn paginated_discovery_follows_cursor_and_deduplicates() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .and(body_json(json!({"query": "notes", "page_size": 100, "filter": {"property": "object", "value": "page"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": [{"object": "page", "id": ID}], "has_more": true, "next_cursor": "next"})))
        .expect(1).mount(&server).await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .and(body_json(json!({"query": "notes", "start_cursor": "next", "page_size": 100, "filter": {"property": "object", "value": "page"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": [{"object": "page", "id": ID}, {"object": "page", "id": OTHER}], "has_more": false, "next_cursor": null})))
        .expect(1).mount(&server).await;
    assert_eq!(
        client(&server).discover(Some("notes")).await.unwrap(),
        vec![ID, OTHER]
    );
}

#[derive(Clone)]
struct ChangingPage(Arc<AtomicUsize>);
impl Respond for ChangingPage {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let number = self.0.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200).set_body_json(metadata(&number.to_string()))
    }
}

#[tokio::test]
async fn concurrent_remote_change_during_retrieval_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}")))
        .respond_with(ChangingPage(Arc::new(AtomicUsize::new(0))))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}/markdown")))
        .respond_with(ResponseTemplate::new(200).set_body_json(markdown("Remote")))
        .mount(&server)
        .await;
    assert!(client(&server)
        .retrieve(ID)
        .await
        .unwrap_err()
        .to_string()
        .contains("changed while"));
}

#[tokio::test]
async fn warning_after_write_does_not_advance_baseline() {
    let server = MockServer::start().await;
    snapshot(&server, "Original", 1).await;
    let mut response = markdown("Partial normalization");
    response["warnings"] = json!([{"code": "unsupported", "message": "omitted content"}]);
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .expect(1)
        .mount(&server)
        .await;
    let (_dir, store) = workspace("Original");
    let doc = edit(&store, "Local edit");
    assert!(commands::push_document(&store, &client(&server), &doc)
        .await
        .unwrap_err()
        .to_string()
        .contains("warnings"));
    assert_eq!(store.get(ID).unwrap(), doc);
}
