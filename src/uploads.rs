use crate::{
    db::Store,
    document::page_id,
    files::{checksum, digest, filename, identity},
    journal::{self, Intent, Operation, State},
    notion::Notion,
};
use anyhow::{ensure, Context, Result};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
use tempfile::NamedTempFile;
const MIB: u64 = 1024 * 1024;
const PART: u64 = 10 * MIB;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Blob {
    pub sha256: String,
    pub size: u64,
    pub filename: String,
    pub content_type: String,
    pub part_hashes: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Upload {
    pub id: String,
    pub input: Blob,
    pub confirmed: BTreeSet<u32>,
    pub status: String,
    pub operation_id: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Attachment {
    pub page: String,
    pub upload: String,
    pub block: String,
    pub name: String,
}

pub(crate) struct Part {
    pub bytes: Vec<u8>,
    pub filename: String,
    pub content_type: String,
    pub number: Option<u32>,
}

fn blob_path(store: &Store, blob: &Blob) -> Result<std::path::PathBuf> {
    Ok(store
        .root()
        .join("upload-inputs")
        .join(format!("{}.bin", identity(&blob.sha256)?)))
}

fn snapshot(store: &Store, path: &Path, content_type: &str) -> Result<Blob> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .context("input needs a UTF-8 filename")?;
    filename(name)?;
    reqwest::multipart::Part::bytes(Vec::new()).mime_str(content_type)?;
    let mut source = fs::File::open(path)?;
    let size = source.metadata()?.len();
    ensure!(
        (1..=5 * 1024 * MIB).contains(&size),
        "upload size must be 1 byte..5 GiB (workspace limits also apply)"
    );
    let directory = store.root().join("upload-inputs");
    fs::create_dir_all(&directory)?;
    let mut tmp = NamedTempFile::new_in(&directory)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    let mut copied = 0u64;
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        copied += count as u64;
        ensure!(copied <= size, "input grew while snapshotting");
        hash.update(&buffer[..count]);
        tmp.write_all(&buffer[..count])?;
    }
    ensure!(copied == size, "input size changed while snapshotting");
    let sha256 = format!("{:x}", hash.finalize());
    let path = directory.join(format!("{sha256}.bin"));
    tmp.as_file().sync_all()?;
    if path.try_exists()? {
        ensure!(
            checksum(&path)? == sha256,
            "saved upload input changed; file preserved"
        );
    } else {
        tmp.persist_noclobber(&path)?;
        fs::File::open(&directory)?.sync_all()?;
    }
    let chunk_size = if size <= 20 * MIB { size } else { PART };
    let mut source = fs::File::open(path)?;
    let mut remaining = size;
    let mut part_hashes = Vec::new();
    while remaining > 0 {
        let length = remaining.min(chunk_size) as usize;
        let mut bytes = vec![0; length];
        source.read_exact(&mut bytes)?;
        part_hashes.push(digest(&bytes));
        remaining -= length as u64;
    }
    Ok(Blob {
        sha256,
        size,
        filename: name.into(),
        content_type: content_type.into(),
        part_hashes,
    })
}

#[derive(Debug)]
struct Status {
    id: String,
    status: String,
    size: Option<u64>,
}
fn status(value: &Value) -> Result<Status> {
    ensure!(
        value["object"] == "file_upload",
        "invalid upload response; inspect before retrying"
    );
    let id = page_id(
        value["id"]
            .as_str()
            .context("upload response missing identity")?,
    )?;
    let status = value["status"].as_str().context("upload status missing")?;
    ensure!(
        ["pending", "uploaded", "expired", "failed"].contains(&status),
        "unknown upload status"
    );
    Ok(Status {
        id,
        status: status.into(),
        size: value["content_length"].as_u64(),
    })
}
async fn observe(notion: &Notion, id: &str) -> Result<Status> {
    let id = page_id(id)?;
    let value = status(
        &notion
            .request(Method::GET, &format!("/file_uploads/{id}"), None)
            .await?,
    )?;
    ensure!(value.id == id, "upload identity mismatch");
    Ok(value)
}
fn usable(value: &Status, blob: &Blob) -> Result<()> {
    ensure!(
        !["expired", "failed"].contains(&value.status.as_str()),
        "upload is {}; start a new explicit upload",
        value.status
    );
    if value.status == "uploaded" {
        ensure!(
            value.size == Some(blob.size),
            "uploaded size does not match saved input"
        );
    }
    Ok(())
}

impl Store {
    pub fn upload(&self, id: &str) -> Result<Upload> {
        let id = page_id(id)?;
        let value: Upload = self.cache_get("uploads-v1", &id)?;
        ensure!(value.id == id, "upload cache identity mismatch");
        Ok(value)
    }
    fn save_upload(&self, upload: &Upload) -> Result<()> {
        self.cache_save("uploads-v1", &upload.id, upload)
    }
}

async fn create_upload(store: &Store, notion: &Notion, input: Blob) -> Result<Upload> {
    let mut op = store.prepare(
        Intent::UploadCreate {
            input: input.clone(),
        },
        None,
    )?;
    let result=async {
        store.transition(&mut op,State::Sent)?;
        let mut request=json!({"mode":if input.part_hashes.len()>1 {"multi_part"}else{"single_part"},"filename":input.filename,"content_type":input.content_type});
        if input.part_hashes.len()>1 {request["number_of_parts"]=json!(input.part_hashes.len());}
        let result=status(&notion.request(Method::POST,"/file_uploads",Some(request)).await?)?;
        eprintln!("created upload {} (operation {})",result.id,op.id);op.remote_id=Some(result.id.clone());store.transition(&mut op,State::RemoteConfirmed)?;
        usable(&result,&input)?;
        let upload=Upload{id:result.id,input,confirmed:BTreeSet::new(),status:result.status,operation_id:op.id};store.save_upload(&upload)?;store.transition(&mut op,State::LocallySaved)?;Ok(upload)
    }.await;
    store.operation_result(&mut op, result)
}

pub async fn upload(
    store: &Store,
    notion: &Notion,
    path: &Path,
    content_type: &str,
    resume: Option<&str>,
) -> Result<Upload> {
    let input = snapshot(store, path, content_type)?;
    let mut upload = if let Some(id) = resume {
        let mut upload = store.upload(id)?;
        ensure!(
            upload.input.sha256 == input.sha256
                && upload.input.size == input.size
                && upload.input.content_type == input.content_type,
            "resume input/type differs from saved upload"
        );
        let observed = observe(notion, &upload.id).await?;
        usable(&observed, &input)?;
        upload.status = observed.status.clone();
        for op in store.operations()? {
            let belongs = match &op.intent {
                Intent::UploadPart { upload: id, .. } | Intent::UploadComplete { upload: id } => {
                    id == &upload.id
                }
                _ => false,
            };
            if belongs && op.pending() && op.state != State::Prepared {
                ensure!(op.state==State::RemoteConfirmed || observed.status=="uploaded","part/completion outcome is uncertain; Notion exposes no per-part inventory; inspect operation {} before restarting",op.id);
                journal::recover(store, notion, op.id).await?;
            }
        }
        let mut saved = store.upload(id)?;
        saved.status = observed.status;
        store.save_upload(&saved)?;
        saved
    } else {
        create_upload(store, notion, input).await?
    };
    if upload.status == "uploaded" {
        return Ok(upload);
    }
    let multipart = upload.input.part_hashes.len() > 1;
    let chunk_size = if multipart { PART } else { upload.input.size };
    for number in 1..=upload.input.part_hashes.len() as u32 {
        if upload.confirmed.contains(&number) {
            continue;
        }
        let mut file = fs::File::open(blob_path(store, &upload.input)?)?;
        let offset = (number as u64 - 1) * chunk_size;
        let length = (upload.input.size - offset).min(chunk_size) as usize;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; length];
        file.read_exact(&mut bytes)?;
        ensure!(
            digest(&bytes) == upload.input.part_hashes[number as usize - 1],
            "saved upload part changed; no part sent"
        );
        let mut op = store.prepare(
            Intent::UploadPart {
                upload: upload.id.clone(),
                input: upload.input.clone(),
                number,
            },
            Some(upload.operation_id),
        )?;
        let result = async {
            store.transition(&mut op, State::Sent)?;
            let value = status(
                &notion
                    .send_upload(
                        &upload.id,
                        &Part {
                            bytes,
                            filename: upload.input.filename.clone(),
                            content_type: upload.input.content_type.clone(),
                            number: multipart.then_some(number),
                        },
                    )
                    .await?,
            )?;
            ensure!(
                value.id == upload.id,
                "upload part response identity mismatch"
            );
            usable(&value, &upload.input)?;
            op.remote_id = Some(upload.id.clone());
            store.transition(&mut op, State::RemoteConfirmed)?;
            upload.confirmed.insert(number);
            upload.status = value.status;
            store.save_upload(&upload)?;
            store.transition(&mut op, State::LocallySaved)
        }
        .await;
        store.operation_result(&mut op, result)?;
    }
    if multipart && upload.status != "uploaded" {
        let mut op = store.prepare(
            Intent::UploadComplete {
                upload: upload.id.clone(),
            },
            Some(upload.operation_id),
        )?;
        let result = async {
            store.transition(&mut op, State::Sent)?;
            let value = status(
                &notion
                    .request(
                        Method::POST,
                        &format!("/file_uploads/{}/complete", upload.id),
                        Some(json!({})),
                    )
                    .await?,
            )?;
            ensure!(
                value.id == upload.id && value.status == "uploaded",
                "upload completion not confirmed"
            );
            usable(&value, &upload.input)?;
            op.remote_id = Some(upload.id.clone());
            store.transition(&mut op, State::RemoteConfirmed)?;
            upload.status = value.status;
            store.save_upload(&upload)?;
            store.transition(&mut op, State::LocallySaved)
        }
        .await;
        store.operation_result(&mut op, result)?;
    }
    ensure!(
        upload.status == "uploaded",
        "upload remains pending; inspect before another explicit resume"
    );
    Ok(upload)
}

async fn verify_attachment(notion: &Notion, page: &str, block: &str) -> Result<()> {
    let value = notion
        .request(Method::GET, &format!("/blocks/{}", page_id(block)?), None)
        .await?;
    ensure!(
        page_id(
            value["id"]
                .as_str()
                .context("attachment block missing ID")?
        )? == block
            && value["type"] == "file"
            && value["parent"]["page_id"]
                .as_str()
                .and_then(|id| page_id(id).ok())
                .as_deref()
                == Some(page),
        "attachment readback identity/type/parent mismatch"
    );
    Ok(())
}

pub async fn attach(
    store: &Store,
    notion: &Notion,
    page: &str,
    upload_id: &str,
    name: &str,
) -> Result<Attachment> {
    filename(name)?;
    let page = page_id(page)?;
    let upload_id = page_id(upload_id)?;
    let observed = observe(notion, &upload_id).await?;
    ensure!(
        observed.status == "uploaded",
        "file upload is not ready to attach"
    );
    let mut op = store.prepare(
        Intent::Attach {
            page: page.clone(),
            upload: upload_id.clone(),
            name: name.into(),
        },
        None,
    )?;
    let result=async {
        store.transition(&mut op,State::Sent)?;
        let response=notion.request(Method::PATCH,&format!("/blocks/{page}/children"),Some(json!({"children":[{"object":"block","type":"file","file":{"type":"file_upload","file_upload":{"id":upload_id},"name":name}}],"notifications":{"mode":"silent"}}))).await?;
        let blocks=response["results"].as_array().context("attachment missing created block results")?;ensure!(blocks.len()==1,"attachment response must contain one block");
        let block=page_id(blocks[0]["id"].as_str().context("created attachment missing identity")?)?;
        eprintln!("attached block {block} (operation {})",op.id);op.remote_id=Some(block.clone());store.transition(&mut op,State::RemoteConfirmed)?;
        verify_attachment(notion,&page,&block).await?;
        let attachment=Attachment{page,upload:upload_id,block: block.clone(),name:name.into()};store.cache_save("attachments-v1",&block,&attachment)?;store.transition(&mut op,State::LocallySaved)?;Ok(attachment)
    }.await;
    store.operation_result(&mut op, result)
}

pub(crate) async fn recover_file(
    store: &Store,
    notion: &Notion,
    op: &mut Operation,
    target: &str,
) -> Result<()> {
    match op.intent.clone() {
        Intent::Attach { page, upload, name } => {
            verify_attachment(notion, &page, target).await?;
            if op.state != State::RemoteConfirmed {
                store.transition(op, State::RemoteConfirmed)?;
            }
            store.cache_save(
                "attachments-v1",
                target,
                &Attachment {
                    page,
                    upload,
                    name,
                    block: target.into(),
                },
            )?;
        }
        Intent::UploadCreate { input } => {
            let value = observe(notion, target).await?;
            usable(&value, &input)?;
            if op.state != State::RemoteConfirmed {
                store.transition(op, State::RemoteConfirmed)?;
            }
            let upload = match store.upload(target) {
                Ok(mut existing) => {
                    ensure!(
                        existing.input.sha256 == input.sha256,
                        "upload input mismatch"
                    );
                    existing.status = value.status;
                    existing
                }
                Err(_) => Upload {
                    id: target.into(),
                    input,
                    confirmed: BTreeSet::new(),
                    status: value.status,
                    operation_id: op.id,
                },
            };
            store.save_upload(&upload)?;
        }
        Intent::UploadPart {
            upload,
            input,
            number,
        } => {
            let value = observe(notion, &upload).await?;
            usable(&value, &input)?;
            ensure!(
                op.state == State::RemoteConfirmed || value.status == "uploaded",
                "part outcome is unknown while upload is pending; no part retransmitted"
            );
            if op.state != State::RemoteConfirmed {
                store.transition(op, State::RemoteConfirmed)?;
            }
            let mut record = store.upload(&upload)?;
            ensure!(record.input.sha256 == input.sha256, "upload input mismatch");
            record.confirmed.insert(number);
            record.status = value.status;
            store.save_upload(&record)?;
        }
        Intent::UploadComplete { upload } => {
            let value = observe(notion, &upload).await?;
            let mut record = store.upload(&upload)?;
            usable(&value, &record.input)?;
            ensure!(
                value.status == "uploaded",
                "completion is unconfirmed; no completion repeated"
            );
            if op.state != State::RemoteConfirmed {
                store.transition(op, State::RemoteConfirmed)?;
            }
            record.status = value.status;
            store.save_upload(&record)?;
        }
        _ => anyhow::bail!("not a file operation"),
    }
    Ok(())
}

#[cfg(test)]
mod restart {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use wiremock::{
        matchers::{body_json, method, path},
        Mock, MockServer, Request, ResponseTemplate,
    };

    #[tokio::test]
    async fn multipart_resume_after_creation_reuses_identity_and_completes_each_part_once() {
        let dir = tempfile::TempDir::new().unwrap();
        let workspace = dir.path().join("workspace");
        let input = dir.path().join("large.pdf");
        fs::File::create(&input).unwrap().set_len(21 * MIB).unwrap();
        let store = Store::open(&workspace).unwrap();
        let server = MockServer::start().await;
        let id = "b55c9c91-384d-452b-81db-d1ef79372b76";
        let response =
            json!({"object":"file_upload","id":id,"status":"pending","content_length":null});
        Mock::given(method("POST")).and(path("/v1/file_uploads")).and(body_json(json!({"mode":"multi_part","number_of_parts":3,"filename":"large.pdf","content_type":"application/pdf"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(response.clone())).expect(1).mount(&server).await;
        let notion = Notion::new("test", &format!("{}/v1", server.uri())).unwrap();
        create_upload(
            &store,
            &notion,
            snapshot(&store, &input, "application/pdf").unwrap(),
        )
        .await
        .unwrap();
        drop(store);
        let store = Store::open(&workspace).unwrap();
        Mock::given(method("GET"))
            .and(path(format!("/v1/file_uploads/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(response.clone()))
            .expect(1)
            .mount(&server)
            .await;
        let counter = Arc::new(AtomicUsize::new(0));
        let sent = counter.clone();
        Mock::given(method("POST"))
            .and(path(format!("/v1/file_uploads/{id}/send")))
            .respond_with(move |request: &Request| {
                let number = sent.fetch_add(1, Ordering::SeqCst) + 1;
                let marker = format!("name=\"part_number\"\r\n\r\n{number}\r\n");
                assert!(request
                    .body
                    .windows(marker.len())
                    .any(|bytes| bytes == marker.as_bytes()));
                assert!(
                    request.body.len()
                        > if number == 3 {
                            MIB as usize
                        } else {
                            PART as usize
                        }
                );
                ResponseTemplate::new(200).set_body_json(response.clone())
            })
            .expect(3)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/v1/file_uploads/{id}/complete")))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"object":"file_upload","id":id,"status":"uploaded","content_length":21*MIB}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let result = upload(&store, &notion, &input, "application/pdf", Some(id))
            .await
            .unwrap();
        assert_eq!(result.status, "uploaded");
        assert_eq!(result.confirmed.len(), 3);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
        assert_eq!(store.operations().unwrap().len(), 5);
    }
}
