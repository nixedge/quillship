use quillship::{
    commands,
    db::Store,
    document::{Document, RemoteDocument},
    notion::{Notion, API_VERSION},
};

#[tokio::test]
async fn recursive_discovery_then_pull_preserves_dirty_root_and_saves_child() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(page("Root")))
        .expect(3)
        .mount(&server)
        .await;
    let mut child_page = page("Child");
    child_page["id"] = json!(CHILD);
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{CHILD}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(child_page))
        .expect(2)
        .mount(&server)
        .await;
    for (id, text) in [(ID, "baseline"), (CHILD, "child body")] {
        Mock::given(method("GET"))
            .and(path(format!("/v1/pages/{id}/markdown")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"markdown":text,"truncated":false,"unknown_block_ids":[]}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    children(&server, ID, vec![child(CHILD)], false, None).await;
    children(&server, CHILD, vec![], false, None).await;
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .save(
            &Document::from_remote(RemoteDocument {
                id: ID.into(),
                title: "Old".into(),
                url: "old".into(),
                last_edited_time: "old".into(),
                markdown: "baseline".into(),
                sync_warnings: vec![],
            }),
            None,
        )
        .unwrap();
    std::fs::write(store.path(ID).unwrap(), "authored").unwrap();
    let notion = client(&server);
    let tree = notion.tree(ID, 8, 100).await.unwrap();
    assert!(tree.failures.is_empty() && !tree.truncated);
    let ids = tree.pages.into_iter().map(|p| p.id).collect::<Vec<_>>();
    let report = commands::pull(&store, &notion, &ids, None).await.unwrap();
    assert_eq!(report.pulled, 2);
    assert!(report.failures.is_empty());
    assert_eq!(store.get(ID).unwrap().markdown, "authored");
    assert_eq!(store.get(CHILD).unwrap().markdown, "child body");
}
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{body_json, header, method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

const ID: &str = "b55c9c91-384d-452b-81db-d1ef79372b75";
const CHILD: &str = "b55c9c91-384d-452b-81db-d1ef79372b76";
const BLOCK: &str = "b55c9c91-384d-452b-81db-d1ef79372b77";

fn page(title: &str) -> Value {
    json!({"object":"page", "id":ID,"url":"https://notion.so/page", "last_edited_time":"now",
        "parent":{"type":"page_id", "page_id":CHILD},
        "properties":{"Name":{"type":"title", "title":[{"text":{"content":title}}]}}})
}
fn client(server: &MockServer) -> Notion {
    Notion::new("test", &format!("{}/v1", server.uri())).unwrap()
}
async fn metadata(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}")))
        .and(header("Notion-Version", API_VERSION))
        .and(header("authorization", "Bearer test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page("Notes")))
        .expect(1)
        .mount(server)
        .await;
}
async fn children(
    server: &MockServer,
    id: &str,
    blocks: Vec<Value>,
    more: bool,
    cursor: Option<&str>,
) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/blocks/{id}/children")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"results":blocks,"has_more":more,"next_cursor":cursor})),
        )
        .expect(1)
        .mount(server)
        .await;
}
fn child(id: &str) -> Value {
    json!({"id":id,"type":"child_page","has_children":true,"child_page":{"title":"Child"}})
}

#[tokio::test]
async fn empty_paginated_responses_are_bounded_by_request_budget() {
    let server = MockServer::start().await;
    metadata(&server).await;
    children(&server, ID, vec![], true, Some("next")).await;
    let report = client(&server).tree(ID, 8, 1).await.unwrap();
    assert!(report.truncated);
    assert_eq!(report.blocks_scanned, 0);
    assert_eq!(report.requests, 1);
}

#[tokio::test]
async fn rename_uses_actual_title_property_and_metadata_keeps_dirty_body() {
    let server = MockServer::start().await;
    metadata(&server).await;
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/pages/{ID}")))
        .and(body_json(
            json!({"properties":{"Name":{"title":[{"type":"text","text":{"content":"Renamed"}}]}}}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(page("Renamed")))
        .expect(1)
        .mount(&server)
        .await;
    let info = client(&server).rename(ID, "Renamed").await.unwrap();
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let doc = Document::from_remote(RemoteDocument {
        id: ID.into(),
        title: "Old".into(),
        url: "old".into(),
        last_edited_time: "old".into(),
        markdown: "baseline".into(),
        sync_warnings: vec![],
    });
    store.save(&doc, None).unwrap();
    std::fs::write(store.path(ID).unwrap(), "authored").unwrap();
    store.save_metadata(&info).unwrap();
    let doc = store.get(ID).unwrap();
    assert_eq!(doc.markdown, "authored");
    assert_eq!(doc.baseline, "baseline");
    assert_eq!(doc.title, "Renamed");
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        store.metadata(ID).unwrap().parent.unwrap()["page_id"],
        CHILD
    );
}

#[tokio::test]
async fn move_uses_dedicated_endpoint_and_rejects_self_before_network() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/pages/{ID}/move")))
        .and(body_json(
            json!({"parent":{"type":"page_id","page_id":CHILD}}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(page("Notes")))
        .expect(1)
        .mount(&server)
        .await;
    let notion = client(&server);
    assert!(notion.move_page(ID, ID).await.is_err());
    assert_eq!(notion.move_page(ID, CHILD).await.unwrap().id, ID);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn tree_follows_nested_containers_and_paginates() {
    let server = MockServer::start().await;
    metadata(&server).await;
    children(
        &server,
        ID,
        vec![json!({"id":BLOCK,"type":"toggle","has_children":true})],
        true,
        Some("cursor with spaces"),
    )
    .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/blocks/{ID}/children")))
        .and(query_param("start_cursor", "cursor with spaces"))
        .and(query_param("page_size", "100"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"results":[],"has_more":false,"next_cursor":null})),
        )
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    children(&server, BLOCK, vec![child(CHILD)], false, None).await;
    children(&server, CHILD, vec![], false, None).await;
    let report = client(&server).tree(ID, 8, 10000).await.unwrap();
    assert!(report.failures.is_empty());
    assert!(!report.truncated);
    assert_eq!(report.blocks_scanned, 2);
    assert_eq!(report.pages.len(), 2);
    assert_eq!(report.pages[1].parent_id.as_deref(), Some(ID));
    assert_eq!(report.pages[1].depth, 1);
}

#[tokio::test]
async fn depth_and_total_block_limits_are_explicit() {
    for (depth, blocks) in [(0, 100), (8, 1)] {
        let server = MockServer::start().await;
        metadata(&server).await;
        children(&server, ID, vec![child(CHILD)], false, None).await;
        let report = client(&server).tree(ID, depth, blocks).await.unwrap();
        assert!(report.truncated);
        assert_eq!(report.blocks_scanned, 1);
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn cycles_repeated_cursors_and_inaccessible_descendants_are_failures() {
    for mode in ["cycle", "cursor", "access"] {
        let server = MockServer::start().await;
        metadata(&server).await;
        match mode {
            "cycle" => children(&server, ID, vec![child(ID)], false, None).await,
            "cursor" => {
                Mock::given(method("GET"))
                    .and(path(format!("/v1/blocks/{ID}/children")))
                    .respond_with(ResponseTemplate::new(200).set_body_json(
                        json!({"results":[],"has_more":true,"next_cursor":"repeat"}),
                    ))
                    .expect(2)
                    .mount(&server)
                    .await;
            }
            _ => {
                children(&server, ID, vec![child(CHILD)], false, None).await;
                Mock::given(method("GET"))
                    .and(path(format!("/v1/blocks/{CHILD}/children")))
                    .respond_with(
                        ResponseTemplate::new(403).set_body_json(json!({"message":"no access"})),
                    )
                    .expect(1)
                    .mount(&server)
                    .await;
            }
        }
        let report = client(&server).tree(ID, 8, 100).await.unwrap();
        assert_eq!(report.failures.len(), 1, "{mode}");
        assert!(!report.truncated);
    }
}

#[tokio::test]
async fn metadata_allows_archived_inspection_but_checks_identity() {
    let server = MockServer::start().await;
    let mut archived = page("Archived");
    archived["archived"] = json!(true);
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(archived))
        .expect(1)
        .mount(&server)
        .await;
    assert!(client(&server).inspect(ID).await.unwrap().archived);
    let server = MockServer::start().await;
    let mut wrong = page("Wrong");
    wrong["id"] = json!(CHILD);
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(wrong))
        .expect(1)
        .mount(&server)
        .await;
    assert!(client(&server).inspect(ID).await.is_err());
}
