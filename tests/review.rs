use std::{fs, process::Command};

use quillship::{
    commands::{reconcile, Reconcile},
    db::Store,
    document::{page_id, Document, RemoteDocument},
    notion::Notion,
    review::unified_diff,
};
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, Request, Respond, ResponseTemplate,
};

const ID: &str = "b55c9c91-384d-452b-81db-d1ef79372b75";

fn workspace() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let doc = Document::from_remote(RemoteDocument {
        id: ID.into(),
        title: "Notes".into(),
        url: format!("https://notion.so/{ID}"),
        last_edited_time: "time".into(),
        markdown: "baseline\n".into(),
        sync_warnings: vec![],
    });
    store.save(&doc, None).unwrap();
    fs::write(store.path(ID).unwrap(), "local\n").unwrap();
    (dir, store)
}

async fn remote(server: &MockServer, complete: bool) -> Notion {
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": ID, "url": "https://notion.so/page", "last_edited_time": "now", "properties": {}
        })))
        .expect(2)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}/markdown")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "markdown": "remote\n", "truncated": !complete, "unknown_block_ids": []
        })))
        .expect(0..=1)
        .mount(server)
        .await;
    Notion::new("test", &format!("{}/v1", server.uri())).unwrap()
}

#[test]
fn notion_links_normalize_without_host_confusion() {
    for url in [
        format!(
            "https://www.notion.so/Notes-{}?pvs=4#section",
            ID.replace('-', "")
        ),
        format!("https://www.notion.so/p/{ID}/"),
        format!("https://team.notion.site/{ID}"),
        format!("https://notion.com/{ID}"),
    ] {
        assert_eq!(page_id(&url).unwrap(), ID);
    }
    for url in [
        format!("https://notion.so.evil.example/{ID}"),
        format!("https://evilnotion.so/{ID}"),
        format!("http://notion.so/{ID}"),
        format!("https://token@notion.so/{ID}"),
        format!("https://notion.so/?p={ID}"),
    ] {
        assert!(page_id(&url).is_err(), "{url}");
    }
}

#[test]
fn diffs_cover_empty_inputs_context_and_missing_newlines() {
    assert_eq!(unified_diff("same", "same", "a", "b"), "");
    assert_eq!(
        unified_diff("", "hello\n", "a", "b"),
        "--- a\n+++ b\n@@ -0,0 +1,1 @@\n+hello\n"
    );
    assert_eq!(
        unified_diff("hello\n", "", "a", "b"),
        "--- a\n+++ b\n@@ -1,1 +0,0 @@\n-hello\n"
    );
    assert_eq!(unified_diff("x\nold", "x\nnew", "a", "b"),
        "--- a\n+++ b\n@@ -1,2 +1,2 @@\n x\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n");
    let diff = unified_diff(
        "1\n2\n3\n4\n5\n6\n7\n8\n",
        "1\n2\n3\n4\nx\n6\n7\n8\n",
        "a",
        "b",
    );
    assert!(diff.contains("@@ -2,7 +2,7 @@\n"));
    assert!(!diff.contains("\n 1\n"));
}

#[tokio::test]
async fn offline_acceptance_is_backed_up_and_survives_restart() {
    let (dir, store) = workspace();
    let (doc, snapshot) = reconcile(&store, None, ID, Reconcile::Baseline)
        .await
        .unwrap();
    assert!(!doc.dirty());
    assert_eq!(snapshot.local.markdown, "local\n");
    assert_eq!(snapshot.local.baseline, "baseline\n");
    assert!(snapshot.remote.is_none());
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        store.snapshot(snapshot.id).unwrap().local.markdown,
        "local\n"
    );
    assert_eq!(store.snapshots(ID).unwrap().len(), 1);
    assert!(store.snapshot(snapshot.id + 1).is_err());
}

#[tokio::test]
async fn remote_reconciliation_archives_three_sides_and_merge() {
    for choice in [
        Reconcile::Local,
        Reconcile::Remote,
        Reconcile::Merged("merged\n".into()),
    ] {
        let (_dir, store) = workspace();
        let server = MockServer::start().await;
        let notion = remote(&server, true).await;
        let expected = match &choice {
            Reconcile::Local => "local\n",
            Reconcile::Remote => "remote\n",
            _ => "merged\n",
        };
        let (doc, snapshot) = reconcile(&store, Some(&notion), ID, choice).await.unwrap();
        assert_eq!(doc.baseline, "remote\n");
        assert_eq!(doc.markdown, expected);
        assert_eq!(snapshot.local.markdown, "local\n");
        assert_eq!(snapshot.local.baseline, "baseline\n");
        assert_eq!(snapshot.remote.unwrap().markdown, "remote\n");
        assert_eq!(snapshot.selected_markdown, expected);
        assert!(server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET"));
    }
}

#[tokio::test]
async fn incomplete_reads_preserve_all_local_state() {
    let (_dir, store) = workspace();
    let before = store.get(ID).unwrap();
    let server = MockServer::start().await;
    let notion = remote(&server, false).await;
    assert!(reconcile(&store, Some(&notion), ID, Reconcile::Remote)
        .await
        .is_err());
    assert_eq!(store.get(ID).unwrap(), before);
    assert!(store.snapshots(ID).unwrap().is_empty());
    let mut partial = before.clone();
    partial.incomplete = true;
    store.save(&partial, Some(&before)).unwrap();
    assert!(reconcile(&store, None, ID, Reconcile::Baseline)
        .await
        .is_err());
}

struct EditDuringRead(std::path::PathBuf);
impl Respond for EditDuringRead {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        fs::write(&self.0, "concurrent\n").unwrap();
        ResponseTemplate::new(200)
            .set_body_json(json!({"markdown":"remote\n", "truncated":false,"unknown_block_ids":[]}))
    }
}

#[tokio::test]
async fn concurrent_file_edit_is_preserved_with_archived_inputs() {
    let (_dir, store) = workspace();
    let server = MockServer::start().await;
    let notion = remote(&server, true).await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}/markdown")))
        .respond_with(EditDuringRead(store.path(ID).unwrap()))
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    let error = reconcile(&store, Some(&notion), ID, Reconcile::Remote)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("working file changed"));
    assert_eq!(store.get(ID).unwrap().markdown, "concurrent\n");
    assert_eq!(store.get(ID).unwrap().baseline, "baseline\n");
    assert_eq!(store.snapshots(ID).unwrap()[0].local.markdown, "local\n");
}

#[tokio::test]
async fn corrupt_archive_is_reported_without_changing_documents() {
    let (dir, store) = workspace();
    let before = store.get(ID).unwrap();
    drop(store);
    // Corrupt recovery data must be reported, never silently omitted from a listing.
    let db = sled::open(dir.path().join("store")).unwrap();
    db.open_tree("reconciliation-v1")
        .unwrap()
        .insert(0u64.to_be_bytes(), b"bad-json")
        .unwrap();
    db.flush().unwrap();
    drop(db);
    let store = Store::open(dir.path()).unwrap();
    assert!(store.snapshots(ID).is_err());
    assert_eq!(store.get(ID).unwrap(), before);
}

#[tokio::test]
async fn cli_diff_and_snapshot_export_are_offline_and_exact() {
    let (dir, store) = workspace();
    let (_, snapshot) = reconcile(&store, None, ID, Reconcile::Baseline)
        .await
        .unwrap();
    drop(store);
    let output = Command::new(env!("CARGO_BIN_EXE_quillship"))
        .args([
            "--workspace",
            dir.path().to_str().unwrap(),
            "snapshot",
            &snapshot.id.to_string(),
            "--side",
            "local",
        ])
        .env_remove("NOTION_TOKEN")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"local\n");
    let output = Command::new(env!("CARGO_BIN_EXE_quillship"))
        .args([
            "--workspace",
            dir.path().to_str().unwrap(),
            "diff",
            ID,
            "--json",
        ])
        .env_remove("NOTION_TOKEN")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["local_diff"],
        ""
    );
}
