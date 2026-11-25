use quillship::{
    comments::{self, Target},
    db::Store,
    journal::{recover, Intent, State},
    notion::Notion,
};
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{body_json, method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};
const ID: &str = "b55c9c91-384d-452b-81db-d1ef79372b75";
const CHILD: &str = "b55c9c91-384d-452b-81db-d1ef79372b76";

#[tokio::test]
async fn confirmed_comment_recovery_observes_identity_without_posting() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    let mut op = store
        .prepare(
            Intent::Comment {
                target: Target::Page(ID.into()),
                markdown: "review".into(),
            },
            None,
        )
        .unwrap();
    store.transition(&mut op, State::Sent).unwrap();
    op.remote_id = Some(CHILD.into());
    store.transition(&mut op, State::RemoteConfirmed).unwrap();
    Mock::given(method("GET"))
        .and(path(format!("/v1/comments/{CHILD}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(comment(CHILD)))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        recover(&store, &client(&server), op.id)
            .await
            .unwrap()
            .state,
        State::LocallySaved
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
fn client(server: &MockServer) -> Notion {
    Notion::new("test", &format!("{}/v1", server.uri())).unwrap()
}
fn comment(id: &str) -> Value {
    json!({"object":"comment","id":id,"discussion_id":ID,"parent":{"page_id":ID},"created_time":"time","last_edited_time":"time","created_by":{"id":ID},"rich_text":[{"plain_text":"review"}]})
}

#[tokio::test]
async fn paginated_unresolved_comments_cache_survives_restart_and_failed_refresh() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .and(query_param("block_id", ID))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"results":[comment(ID)],"has_more":true,"next_cursor":"next cursor"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .and(query_param("start_cursor", "next cursor"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"results":[comment(CHILD)],"has_more":false})),
        )
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    let snapshot = comments::pull(&store, &client(&server), ID, false, 100)
        .await
        .unwrap();
    assert_eq!(snapshot.comments.len(), 2);
    assert!(snapshot.unresolved_only);
    assert!(store.all().unwrap().is_empty());
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"results":[],"has_more":true})),
        )
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    assert!(comments::pull(&store, &client(&server), ID, false, 100)
        .await
        .is_err());
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.comments(ID, false).unwrap().comments.len(), 2);
    assert!(store.comments(ID, true).is_err());
}

#[tokio::test]
async fn recursive_comments_include_nested_blocks_without_text_anchors() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path(format!("/v1/blocks/{ID}/children")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":[{"id":CHILD,"type":"paragraph","has_children":false}],"has_more":false}))).expect(1).mount(&server).await;
    for id in [ID, CHILD] {
        Mock::given(method("GET"))
            .and(path("/v1/comments"))
            .and(query_param("block_id", id))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"results":[comment(id)],"has_more":false})),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    let snapshot = comments::pull(&store, &client(&server), ID, true, 100)
        .await
        .unwrap();
    assert_eq!(snapshot.comments.len(), 2);
}

#[tokio::test]
async fn posting_each_target_is_explicit_and_journalled_even_with_partial_response() {
    for (target, mut body) in [
        (Target::Page(ID.into()), json!({"parent":{"page_id":ID}})),
        (Target::Block(ID.into()), json!({"parent":{"block_id":ID}})),
        (Target::Discussion(ID.into()), json!({"discussion_id":ID})),
    ] {
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let server = MockServer::start().await;
        body["markdown"] = json!("**review**");
        body["markdown_version"] = json!("v2");
        Mock::given(method("POST"))
            .and(path("/v1/comments"))
            .and(body_json(body))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"object":"comment","id":CHILD})),
            )
            .expect(1)
            .mount(&server)
            .await;
        comments::add(&store, &client(&server), target, "**review**")
            .await
            .unwrap();
        let op = store.operations().unwrap().pop().unwrap();
        assert_eq!(op.state, State::LocallySaved);
        assert_eq!(op.remote_id.as_deref(), Some(CHILD));
        assert!(store.comments(ID, false).is_err()); // posting never claims to refresh the full unresolved snapshot
    }
}

#[tokio::test]
async fn permission_failures_and_uncertain_posts_are_never_replayed() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;
    assert!(comments::pull(&store, &client(&server), ID, false, 100)
        .await
        .is_err());
    assert!(store.comments(ID, false).is_err());
    Mock::given(method("POST"))
        .and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        comments::add(&store, &client(&server), Target::Page(ID.into()), "review")
            .await
            .is_err()
    );
    let op = store.operations().unwrap().pop().unwrap();
    assert_eq!(op.state, State::Uncertain);
    assert!(recover(&store, &client(&server), op.id).await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn repeated_comment_cursors_do_not_save_partial_cache() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"results":[],"has_more":true,"next_cursor":"same"})),
        )
        .expect(2)
        .mount(&server)
        .await;
    assert!(comments::pull(&store, &client(&server), ID, false, 100)
        .await
        .is_err());
    assert!(store.comments(ID, false).is_err());
}

#[test]
fn unsupported_block_markdown_is_rejected_before_posting() {
    for text in [
        "",
        "# heading",
        "- list",
        "1. list",
        "```rust\ncode\n```",
        "|table|",
        "> quote",
    ] {
        assert!(comments::validate(text).is_err(), "{text}");
    }
    assert!(comments::validate("**bold**, $x$ and [link](https://example.com)").is_ok());
}
