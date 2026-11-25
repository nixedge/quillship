use quillship::{
    db::Store,
    document::{Document, RemoteDocument},
    files::{self, Downloader},
    journal::{self, State},
    notion::Notion,
    uploads,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tempfile::TempDir;
use wiremock::{
    matchers::{body_json, method, path, query_param},
    Mock, MockServer, Request, ResponseTemplate,
};
const ID: &str = "b55c9c91-384d-452b-81db-d1ef79372b75";
const UPLOAD: &str = "b55c9c91-384d-452b-81db-d1ef79372b76";
const BLOCK: &str = "b55c9c91-384d-452b-81db-d1ef79372b77";

#[tokio::test]
async fn discovery_includes_page_covers_and_media_inside_nested_containers() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let api = MockServer::start().await;
    Mock::given(method("GET")).and(path(format!("/v1/pages/{ID}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":ID,"cover":{"type":"file","file":{"url":"https://example.com/cover?signature=SECRET"}}}))).expect(1).mount(&api).await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/blocks/{ID}/children")))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"results":[{"id":BLOCK,"type":"toggle","has_children":true}],"has_more":false}),
        ))
        .expect(1)
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/blocks/{BLOCK}/children")))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"results":[{"id":UPLOAD,"type":"file","has_children":false}],"has_more":false}),
        ))
        .expect(1)
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/blocks/{BLOCK}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":BLOCK,"type":"toggle"})))
        .expect(1)
        .mount(&api)
        .await;
    Mock::given(method("GET")).and(path(format!("/v1/blocks/{UPLOAD}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":UPLOAD,"type":"file","file":{"type":"external","name":"report.pdf","external":{"url":"https://example.com/report.pdf"}}}))).expect(1).mount(&api).await;
    let snapshot = files::list(&store, Some(&client(&api)), ID, 100)
        .await
        .unwrap();
    assert_eq!(snapshot.files.len(), 2);
    assert_eq!(snapshot.files.iter().filter(|f| f.external).count(), 1);
    assert!(!serde_json::to_string(&snapshot).unwrap().contains("SECRET"));
}
fn client(server: &MockServer) -> Notion {
    Notion::new("test-token", &format!("{}/v1", server.uri())).unwrap()
}
fn asset(url: &str, external: bool) -> Value {
    json!({"object":"page","id":ID,"properties":{"Files":{"id":"files","type":"files","files":[if external {json!({"name":"report.pdf","type":"external","external":{"url":url}})}else{json!({"name":"report.pdf","type":"file","file":{"url":url,"expiry_time":"time"}})}]}}})
}
async fn discovery(server: &MockServer, url: &str, external: bool, gets: u64) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(asset(url, external)))
        .expect(gets)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/blocks/{ID}/children")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"results":[],"has_more":false})),
        )
        .expect(1)
        .mount(server)
        .await;
}
fn upload_status(status: &str, size: Option<u64>) -> Value {
    json!({"object":"file_upload","id":UPLOAD,"status":status,"content_length":size,"upload_url":"https://example.com/SECRET","complete_url":"https://example.com/SECRET"})
}

#[tokio::test]
async fn downloads_have_no_notion_headers_and_cache_exact_checksums_without_urls() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let api = MockServer::start().await;
    let download = MockServer::start().await;
    discovery(
        &api,
        &format!("{}/asset?signature=SECRET", download.uri()),
        false,
        2,
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/asset"))
        .respond_with(|request: &Request| {
            assert!(request.headers.get("authorization").is_none());
            assert!(request.headers.get("Notion-Version").is_none());
            ResponseTemplate::new(200).set_body_string("hello")
        })
        .expect(1)
        .mount(&download)
        .await;
    let snapshot = files::list(&store, Some(&client(&api)), ID, 100)
        .await
        .unwrap();
    assert!(!serde_json::to_string(&snapshot).unwrap().contains("SECRET"));
    let id = &snapshot.files[0].id;
    let local = files::fetch(
        &store,
        &client(&api),
        &Downloader::new(true).unwrap(),
        id,
        None,
        100,
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        local.sha256,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
    assert_eq!(local.size, 5);
    assert_eq!(
        fs::read(local.directory.join(&local.filename)).unwrap(),
        b"hello"
    );
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        files::list(&store, None, ID, 100)
            .await
            .unwrap()
            .files
            .len(),
        1
    );
    assert_eq!(
        files::fetch(
            &store,
            &client(&api),
            &Downloader::new(true).unwrap(),
            id,
            None,
            100,
            false
        )
        .await
        .unwrap()
        .sha256,
        local.sha256
    );
}

#[tokio::test]
async fn expired_signed_link_refreshes_once_without_persisting_access() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let api = MockServer::start().await;
    let download = MockServer::start().await;
    let count = Arc::new(AtomicUsize::new(0));
    let base = download.uri();
    Mock::given(method("GET"))
        .and(path(format!("/v1/pages/{ID}")))
        .respond_with(move |_: &Request| {
            let token = if count.fetch_add(1, Ordering::SeqCst) < 2 {
                "old"
            } else {
                "new"
            };
            ResponseTemplate::new(200)
                .set_body_json(asset(&format!("{base}/asset?signature={token}"), false))
        })
        .expect(3)
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/blocks/{ID}/children")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"results":[],"has_more":false})),
        )
        .expect(1)
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path("/asset"))
        .and(query_param("signature", "old"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&download)
        .await;
    Mock::given(method("GET"))
        .and(path("/asset"))
        .and(query_param("signature", "new"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hello"))
        .expect(1)
        .mount(&download)
        .await;
    let snapshot = files::list(&store, Some(&client(&api)), ID, 100)
        .await
        .unwrap();
    assert_eq!(
        files::fetch(
            &store,
            &client(&api),
            &Downloader::new(true).unwrap(),
            &snapshot.files[0].id,
            None,
            100,
            false
        )
        .await
        .unwrap()
        .size,
        5
    );
}

#[tokio::test]
async fn byte_limits_redirects_and_path_collisions_preserve_existing_files() {
    for mode in ["limit", "redirect", "collision"] {
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let api = MockServer::start().await;
        let download = MockServer::start().await;
        discovery(&api, &format!("{}/asset", download.uri()), false, 2).await;
        let snapshot = files::list(&store, Some(&client(&api)), ID, 100)
            .await
            .unwrap();
        let id = &snapshot.files[0].id;
        let destination = dir.path().join("files");
        fs::create_dir_all(&destination).unwrap();
        if mode == "collision" {
            fs::write(destination.join(format!("{id}.pdf")), b"existing").unwrap();
        } else {
            Mock::given(method("GET"))
                .and(path("/asset"))
                .respond_with(if mode == "redirect" {
                    ResponseTemplate::new(302)
                        .insert_header("Location", "https://example.com/never-follow")
                } else {
                    ResponseTemplate::new(200).set_body_string("hello")
                })
                .expect(1)
                .mount(&download)
                .await;
        }
        assert!(files::fetch(
            &store,
            &client(&api),
            &Downloader::new(true).unwrap(),
            id,
            None,
            2,
            false
        )
        .await
        .is_err());
        if mode == "collision" {
            assert_eq!(
                fs::read(destination.join(format!("{id}.pdf"))).unwrap(),
                b"existing"
            );
            assert!(download.received_requests().await.unwrap().is_empty());
        } else {
            assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
        }
    }
}

#[tokio::test]
async fn interrupted_download_leaves_no_partial_output() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0; 4096];
        let count = stream.read(&mut buffer).unwrap();
        assert!(!String::from_utf8_lossy(&buffer[..count])
            .to_lowercase()
            .contains("authorization"));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nsmall")
            .unwrap();
    });
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let api = MockServer::start().await;
    discovery(&api, &format!("http://{address}/asset"), false, 2).await;
    let snapshot = files::list(&store, Some(&client(&api)), ID, 100)
        .await
        .unwrap();
    assert!(files::fetch(
        &store,
        &client(&api),
        &Downloader::new(true).unwrap(),
        &snapshot.files[0].id,
        None,
        100,
        false
    )
    .await
    .is_err());
    thread.join().unwrap();
    assert_eq!(fs::read_dir(dir.path().join("files")).unwrap().count(), 0);
}

#[tokio::test]
async fn external_files_need_explicit_opt_in() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let api = MockServer::start().await;
    discovery(&api, "https://example.com/external", true, 1).await;
    let snapshot = files::list(&store, Some(&client(&api)), ID, 100)
        .await
        .unwrap();
    assert!(files::fetch(
        &store,
        &client(&api),
        &Downloader::new(false).unwrap(),
        &snapshot.files[0].id,
        None,
        100,
        false
    )
    .await
    .is_err());
}

#[test]
fn filenames_cannot_escape_or_inject_multipart_headers() {
    for name in [
        "../escape",
        "/escape",
        "a\\b",
        "line\nname",
        "\"quote\"",
        ".",
        "..",
    ] {
        assert!(files::filename(name).is_err());
    }
    assert!(files::filename("λ report.pdf").is_ok());
}

#[tokio::test]
async fn single_part_upload_retains_input_and_confirms_transfer_separately_from_attachment() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("workspace")).unwrap();
    let input = dir.path().join("upload.txt");
    fs::write(&input, b"hello").unwrap();
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/file_uploads"))
        .and(body_json(
            json!({"mode":"single_part","filename":"upload.txt","content_type":"text/plain"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(upload_status("pending", None)))
        .expect(1)
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/file_uploads/{UPLOAD}/send")))
        .respond_with(|request: &Request| {
            let body = String::from_utf8_lossy(&request.body);
            assert!(body.contains("name=\"file\""));
            assert!(body.contains("hello"));
            assert!(!body.contains("part_number"));
            assert_eq!(
                request.headers.get("authorization").unwrap(),
                "Bearer test-token"
            );
            ResponseTemplate::new(200).set_body_json(upload_status("uploaded", Some(5)))
        })
        .expect(1)
        .mount(&api)
        .await;
    let result = uploads::upload(&store, &client(&api), &input, "text/plain", None)
        .await
        .unwrap();
    assert_eq!(result.status, "uploaded");
    assert!(result.confirmed.contains(&1));
    assert_eq!(
        fs::read(
            dir.path()
                .join("workspace/upload-inputs")
                .join(format!("{}.bin", result.input.sha256))
        )
        .unwrap(),
        b"hello"
    );
    assert_eq!(store.operations().unwrap().len(), 2);
    assert!(store
        .operations()
        .unwrap()
        .iter()
        .all(|op| op.state == State::LocallySaved));
    assert!(!serde_json::to_string(&store.upload(UPLOAD).unwrap())
        .unwrap()
        .contains("SECRET"));
    let requests = api.received_requests().await.unwrap().len();
    fs::write(
        dir.path()
            .join("workspace/upload-inputs")
            .join(format!("{}.bin", result.input.sha256)),
        "tampered",
    )
    .unwrap();
    assert!(
        uploads::upload(&store, &client(&api), &input, "text/plain", Some(UPLOAD))
            .await
            .is_err()
    );
    assert_eq!(api.received_requests().await.unwrap().len(), requests);
}

#[tokio::test]
async fn multipart_failure_keeps_confirmed_parts_and_refuses_ambiguous_retransmission() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("workspace")).unwrap();
    let input = dir.path().join("large.bin");
    fs::File::create(&input)
        .unwrap()
        .set_len(21 * 1024 * 1024)
        .unwrap();
    let api = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/file_uploads")).and(body_json(json!({"mode":"multi_part","number_of_parts":3,"filename":"large.bin","content_type":"application/octet-stream"}))).respond_with(ResponseTemplate::new(200).set_body_json(upload_status("pending",None))).expect(1).mount(&api).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let sent = calls.clone();
    Mock::given(method("POST"))
        .and(path(format!("/v1/file_uploads/{UPLOAD}/send")))
        .respond_with(move |_: &Request| {
            if sent.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(200)
                    .set_body_json(upload_status("pending", Some(10 * 1024 * 1024)))
            } else {
                ResponseTemplate::new(500)
            }
        })
        .expect(2)
        .mount(&api)
        .await;
    assert!(uploads::upload(
        &store,
        &client(&api),
        &input,
        "application/octet-stream",
        None
    )
    .await
    .is_err());
    assert_eq!(
        store
            .upload(UPLOAD)
            .unwrap()
            .confirmed
            .into_iter()
            .collect::<Vec<_>>(),
        vec![1]
    );
    Mock::given(method("GET"))
        .and(path(format!("/v1/file_uploads/{UPLOAD}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(upload_status("pending", Some(10 * 1024 * 1024))),
        )
        .expect(1)
        .mount(&api)
        .await;
    assert!(uploads::upload(
        &store,
        &client(&api),
        &input,
        "application/octet-stream",
        Some(UPLOAD)
    )
    .await
    .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn attachment_readback_failure_retains_identity_and_recovers_without_append() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let api = MockServer::start().await;
    store
        .save(
            &Document::from_remote(RemoteDocument {
                id: ID.into(),
                title: "Notes".into(),
                url: "url".into(),
                last_edited_time: "time".into(),
                markdown: "baseline".into(),
                sync_warnings: vec![],
            }),
            None,
        )
        .unwrap();
    fs::write(store.path(ID).unwrap(), "authored").unwrap();
    Mock::given(method("GET"))
        .and(path(format!("/v1/file_uploads/{UPLOAD}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(upload_status("uploaded", Some(5))))
        .expect(1)
        .mount(&api)
        .await;
    Mock::given(method("PATCH")).and(path(format!("/v1/blocks/{ID}/children"))).and(body_json(json!({"children":[{"object":"block","type":"file","file":{"type":"file_upload","file_upload":{"id":UPLOAD},"name":"report.pdf"}}],"notifications":{"mode":"silent"}}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":[{"id":BLOCK}]}))).expect(1).mount(&api).await;
    assert!(
        uploads::attach(&store, &client(&api), ID, UPLOAD, "report.pdf")
            .await
            .is_err()
    );
    let op = store.operations().unwrap().pop().unwrap();
    assert_eq!(op.state, State::RemoteConfirmed);
    assert_eq!(op.remote_id.as_deref(), Some(BLOCK));
    Mock::given(method("GET"))
        .and(path(format!("/v1/blocks/{BLOCK}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":BLOCK,"type":"file","parent":{"page_id":ID}})),
        )
        .expect(1)
        .mount(&api)
        .await;
    journal::recover(&store, &client(&api), op.id)
        .await
        .unwrap();
    assert_eq!(store.get(ID).unwrap().markdown, "authored");
    assert_eq!(store.get(ID).unwrap().baseline, "baseline");
}
