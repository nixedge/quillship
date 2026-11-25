use quillship::{
    db::Store,
    document::{Document, RemoteDocument},
    journal::{self, Intent, State},
    notion::Notion,
    structured::{self, Schema},
};
use serde_json::{json, Value};
use std::{
    fs,
    sync::{Arc, Mutex},
};
use tempfile::TempDir;
use wiremock::{
    matchers::{body_json, method, path, query_param},
    Mock, MockServer, Request, ResponseTemplate,
};
const ID: &str = "b55c9c91-384d-452b-81db-d1ef79372b75";
const SOURCE: &str = "b55c9c91-384d-452b-81db-d1ef79372b76";
const OTHER: &str = "b55c9c91-384d-452b-81db-d1ef79372b77";

#[tokio::test]
async fn record_move_targets_data_source_and_keeps_local_body_baseline() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    source(&server, 1).await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/pages/{ID}/move")))
        .and(body_json(
            json!({"parent":{"type":"data_source_id","data_source_id":SOURCE}}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":ID})))
        .expect(1)
        .mount(&server)
        .await;
    let mut page = record(json!(1), false);
    page["url"] = json!("url");
    record_reads(&server, Arc::new(Mutex::new(page)), 1).await;
    store
        .save(
            &Document::from_remote(RemoteDocument {
                id: ID.into(),
                title: "Body".into(),
                url: "url".into(),
                last_edited_time: "time".into(),
                markdown: "baseline".into(),
                sync_warnings: vec![],
            }),
            None,
        )
        .unwrap();
    fs::write(store.path(ID).unwrap(), "authored").unwrap();
    structured::move_record(&store, &client(&server), ID, SOURCE)
        .await
        .unwrap();
    assert_eq!(store.get(ID).unwrap().baseline, "baseline");
    assert_eq!(store.get(ID).unwrap().markdown, "authored");
}

#[tokio::test]
async fn schema_drift_is_rejected_before_any_record_mutation() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    record_reads(&server, Arc::new(Mutex::new(record(json!(1), false))), 2).await;
    structured::properties(&store, Some(&client(&server)), ID)
        .await
        .unwrap();
    let mut changed = schema();
    changed.properties["Count"]["type"] = json!("checkbox");
    Mock::given(method("GET"))
        .and(path(format!("/v1/data_sources/{SOURCE}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"object":"data_source","id":SOURCE,"properties":changed.properties}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        structured::update(&store, &client(&server), ID, &json!({"Count":{"number":2}}))
            .await
            .is_err()
    );
    assert!(store.operations().unwrap().is_empty());
    assert_eq!(
        store.properties(ID).unwrap().properties["count"].kind,
        "number"
    );
}

#[tokio::test]
async fn requested_query_limit_is_reported_incomplete() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    source(&server, 1).await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/data_sources/{SOURCE}/query")))
        .and(body_json(json!({"page_size":1})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"results":[{"id":ID}],"has_more":true,"next_cursor":"more"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        !structured::query(&store, Some(&client(&server)), SOURCE, None, None, 1)
            .await
            .unwrap()
            .complete
    );
    assert!(structured::query(&store, None, SOURCE, None, None, 1)
        .await
        .is_err());
}

#[tokio::test]
async fn uncertain_record_creation_cannot_be_replayed() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    source(&server, 1).await;
    Mock::given(method("POST"))
        .and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    assert!(structured::create(
        &store,
        &client(&server),
        SOURCE,
        &json!({"Count":{"number":5}}),
        None
    )
    .await
    .is_err());
    let op = store.operations().unwrap().pop().unwrap();
    assert_eq!(op.state, State::Uncertain);
    let requests = server.received_requests().await.unwrap().len();
    assert!(journal::recover(&store, &client(&server), op.id)
        .await
        .is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), requests);
}
fn client(server: &MockServer) -> Notion {
    Notion::new("test", &format!("{}/v1", server.uri())).unwrap()
}
fn schema() -> Schema {
    Schema{id:SOURCE.into(),properties:serde_json::from_value(json!({"Count":{"id":"count","type":"number"},"Done":{"id":"done","type":"checkbox"},"Name":{"id":"title","type":"title"},"Status":{"id":"status","type":"status","status":{"options":[{"id":"open","name":"Open"}]}},"Computed":{"id":"computed","type":"formula"}})).unwrap()}
}
fn record(count: Value, done: bool) -> Value {
    json!({"object":"page","id":ID,"parent":{"data_source_id":SOURCE},"last_edited_time":"time","properties":{"Count":{"id":"count","type":"number","number":count},"Done":{"id":"done","type":"checkbox","checkbox":done}}})
}
async fn source(server: &MockServer, gets: u64) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/data_sources/{SOURCE}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"object":"data_source","id":SOURCE,"properties":schema().properties}),
        ))
        .expect(gets)
        .mount(server)
        .await;
}
async fn record_reads(server: &MockServer, state: Arc<Mutex<Value>>, gets: u64) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}")))
        .respond_with(move |_: &Request| {
            ResponseTemplate::new(200).set_body_json(state.lock().unwrap().clone())
        })
        .expect(gets)
        .mount(server)
        .await;
}

#[test]
fn typed_writes_allow_explicit_clearing_and_reject_unsupported_or_ambiguous_inputs() {
    let schema = schema();
    assert_eq!(
        structured::validate_properties(
            &schema,
            &json!({"Count":{"number":null},"Done":{"checkbox":false},"Name":{"title":[]}})
        )
        .unwrap(),
        json!({"count":{"number":null},"done":{"checkbox":false},"title":{"title":[]}})
    );
    for input in [
        json!({"Computed":{"formula":{}}}),
        json!({"Count":{"number":"1"}}),
        json!({"Count":null}),
        json!({"Count":{"checkbox":true}}),
        json!({"Status":{"status":{"name":"New option"}}}),
        json!({"Count":{"number":1},"count":{"number":2}}),
    ] {
        assert!(
            structured::validate_properties(&schema, &input).is_err(),
            "{input}"
        );
    }
    assert!(structured::validate_properties(
        &schema,
        &json!({"Status":{"status":{"name":"Open"}}})
    )
    .is_ok());
    structured::validate_filter(&schema,&json!({"and":[{"property":"Done","checkbox":{"equals":true}},{"property":"Count","number":{"greater_than":2}}]}),0).unwrap();
    assert!(structured::validate_filter(
        &schema,
        &json!({"property":"Done","checkbox":{"equals":"yes"}}),
        0
    )
    .is_err());
    assert!(structured::validate_filter(
        &schema,
        &json!({"property":"Count","rich_text":{"contains":"1"}}),
        0
    )
    .is_err());
}

#[tokio::test]
async fn database_lists_multiple_sources_without_choosing_one() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path(format!("/v1/databases/{ID}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"object":"database","id":ID,"data_sources":[{"id":SOURCE,"name":"A"},{"id":OTHER,"name":"B"}]}))).expect(1).mount(&server).await;
    assert_eq!(
        structured::database(&store, Some(&client(&server)), ID)
            .await
            .unwrap()["data_sources"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        structured::database(&store, None, ID).await.unwrap()["id"],
        ID
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn query_paginates_and_capped_refresh_preserves_complete_cache() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    source(&server, 2).await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/data_sources/{SOURCE}/query")))
        .and(body_json(json!({"page_size":100})))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"results":[{"id":ID,"object":"page"}],"has_more":true,"next_cursor":"next"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/data_sources/{SOURCE}/query")))
        .and(body_json(json!({"page_size":100,"start_cursor":"next"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"results":[{"id":OTHER,"object":"page"}],"has_more":false})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        structured::query(&store, Some(&client(&server)), SOURCE, None, None, 10000)
            .await
            .unwrap()
            .complete
    );
    Mock::given(method("POST")).and(path(format!("/v1/data_sources/{SOURCE}/query")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":[],"has_more":false,"request_status":{"type":"incomplete","incomplete_reason":"query_result_limit_reached"}}))).with_priority(1).expect(1).mount(&server).await;
    let capped = structured::query(&store, Some(&client(&server)), SOURCE, None, None, 10000)
        .await
        .unwrap();
    assert!(!capped.complete);
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        structured::query(&store, None, SOURCE, None, None, 10000)
            .await
            .unwrap()
            .results
            .len(),
        2
    );
}

#[tokio::test]
async fn relations_are_fully_paginated_and_partial_refresh_does_not_replace_baseline() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    let mut value = record(json!(1), false);
    value["properties"]["Related"] =
        json!({"id":"r%3A","type":"relation","relation":[],"has_more":true});
    record_reads(&server, Arc::new(Mutex::new(value)), 3).await;
    Mock::given(method("GET")).and(path(format!("/v1/pages/{ID}/properties/r%3A")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"object":"list","results":[{"relation":{"id":SOURCE}}],"has_more":true,"next_cursor":"next cursor"}))).expect(1).mount(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}/properties/r%3A")))
        .and(query_param("start_cursor", "next cursor"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"object":"list","results":[{"relation":{"id":OTHER}}],"has_more":false}),
        ))
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    let full = structured::properties(&store, Some(&client(&server)), ID)
        .await
        .unwrap();
    assert_eq!(full.properties["r%3A"].value.as_array().unwrap().len(), 2);
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}/properties/r%3A")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"object":"list","results":[],"has_more":true})),
        )
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    assert!(structured::properties(&store, Some(&client(&server)), ID)
        .await
        .is_err());
    assert_eq!(
        store.properties(ID).unwrap().properties["r%3A"].value,
        full.properties["r%3A"].value
    );
}

#[tokio::test]
async fn selected_updates_allow_unrelated_changes_and_preserve_dirty_markdown() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    source(&server, 1).await;
    let state = Arc::new(Mutex::new(record(json!(1), false)));
    record_reads(&server, state.clone(), 6).await;
    structured::properties(&store, Some(&client(&server)), ID)
        .await
        .unwrap();
    store
        .save(
            &Document::from_remote(RemoteDocument {
                id: ID.into(),
                title: "Body".into(),
                url: "url".into(),
                last_edited_time: "time".into(),
                markdown: "baseline".into(),
                sync_warnings: vec![],
            }),
            None,
        )
        .unwrap();
    fs::write(store.path(ID).unwrap(), "authored").unwrap();
    *state.lock().unwrap() = record(json!(1), true);
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/pages/{ID}")))
        .and(body_json(json!({"properties":{"count":{"number":null}}})))
        .respond_with(move |_: &Request| {
            *state.lock().unwrap() = record(Value::Null, true);
            ResponseTemplate::new(200).set_body_json(json!({"id":ID}))
        })
        .expect(1)
        .mount(&server)
        .await;
    let after = structured::update(
        &store,
        &client(&server),
        ID,
        &json!({"Count":{"number":null}}),
    )
    .await
    .unwrap();
    assert!(after.properties["count"].value.is_null());
    assert_eq!(after.properties["done"].value, true);
    assert_eq!(store.get(ID).unwrap().markdown, "authored");
    assert_eq!(store.get(ID).unwrap().baseline, "baseline");
    assert_eq!(store.operations().unwrap()[0].state, State::LocallySaved);
}

#[tokio::test]
async fn conflicting_selected_properties_prevent_mutation() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    source(&server, 1).await;
    let state = Arc::new(Mutex::new(record(json!(1), false)));
    record_reads(&server, state.clone(), 4).await;
    structured::properties(&store, Some(&client(&server)), ID)
        .await
        .unwrap();
    *state.lock().unwrap() = record(json!(2), false);
    assert!(
        structured::update(&store, &client(&server), ID, &json!({"Count":{"number":3}}))
            .await
            .is_err()
    );
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method == "GET"));
    assert_eq!(store.properties(ID).unwrap().properties["count"].value, 1);
}

#[tokio::test]
async fn record_creation_is_explicit_about_source_and_journalled() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    source(&server, 1).await;
    record_reads(&server, Arc::new(Mutex::new(record(json!(5), false))), 2).await;
    Mock::given(method("POST")).and(path("/v1/pages")).and(body_json(json!({"parent":{"type":"data_source_id","data_source_id":SOURCE},"properties":{"count":{"number":5}},"notifications":{"mode":"silent"},"allow_async":false})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"object":"page","id":ID}))).expect(1).mount(&server).await;
    structured::create(
        &store,
        &client(&server),
        SOURCE,
        &json!({"Count":{"number":5}}),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        store.operations().unwrap()[0].remote_id.as_deref(),
        Some(ID)
    );
    assert!(!store.contains(ID).unwrap());
}

#[tokio::test]
async fn uncertain_record_update_recovery_observes_properties_without_repeating_write() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let server = MockServer::start().await;
    record_reads(&server, Arc::new(Mutex::new(record(json!(5), false))), 2).await;
    let mut op = store
        .prepare(
            Intent::RecordUpdate {
                id: ID.into(),
                source: SOURCE.into(),
                properties: json!({"count":{"number":5}}),
                expected: json!({}),
            },
            None,
        )
        .unwrap();
    store.transition(&mut op, State::Sent).unwrap();
    store.transition(&mut op, State::Uncertain).unwrap();
    assert_eq!(
        journal::recover(&store, &client(&server), op.id)
            .await
            .unwrap()
            .state,
        State::LocallySaved
    );
    assert_eq!(store.properties(ID).unwrap().properties["count"].value, 5);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}
