use crate::{db::Store, document::page_id, notion::Notion};
use anyhow::{ensure, Context, Result};
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::NamedTempFile;

pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn identity(value: &str) -> Result<&str> {
    ensure!(
        value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()),
        "expected a 64-character file identity"
    );
    Ok(value)
}
pub fn filename(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 900
            && value != "."
            && value != ".."
            && !value
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\' | '"')),
        "unsafe/invalid filename"
    );
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Location {
    Block { id: String, media: String },
    Page { id: String, slot: String },
    Property { page: String, property: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FileRef {
    pub id: String,
    pub location: Location,
    pub asset: String,
    pub name: String,
    pub external: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Snapshot {
    pub page: String,
    pub files: Vec<FileRef>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LocalFile {
    pub id: String,
    pub directory: PathBuf,
    pub filename: String,
    pub sha256: String,
    pub size: u64,
}

fn located(location: Location, value: &Value) -> Result<(FileRef, String)> {
    let external = value["type"] == "external";
    let kind = if external { "external" } else { "file" };
    let url = value[kind]["url"]
        .as_str()
        .context("file has no downloadable URL; refresh after processing")?
        .to_owned();
    let mut asset_url = reqwest::Url::parse(&url)?;
    asset_url.set_fragment(None);
    if !external {
        asset_url.set_query(None);
    }
    let asset = digest(asset_url.as_str().as_bytes());
    let name = value["name"].as_str().unwrap_or("attachment").to_owned();
    let id = digest(format!("{}:{asset}", serde_json::to_string(&location)?).as_bytes());
    Ok((
        FileRef {
            id,
            location,
            asset,
            name,
            external,
        },
        url,
    ))
}

fn page_files(id: &str, page: &Value) -> Result<Vec<(FileRef, String)>> {
    ensure!(
        page_id(page["id"].as_str().context("file source page missing ID")?)? == id,
        "file source page identity mismatch"
    );
    let mut files = Vec::new();
    for slot in ["icon", "cover"] {
        if page[slot]["type"] == "file" || page[slot]["type"] == "external" {
            files.push(located(
                Location::Page {
                    id: id.into(),
                    slot: slot.into(),
                },
                &page[slot],
            )?);
        }
    }
    if let Some(properties) = page["properties"].as_object() {
        for property in properties.values().filter(|p| p["type"] == "files") {
            let key = property["id"]
                .as_str()
                .context("file property missing identity")?;
            for file in property["files"]
                .as_array()
                .context("file property missing array")?
            {
                files.push(located(
                    Location::Property {
                        page: id.into(),
                        property: key.into(),
                    },
                    file,
                )?);
            }
        }
    }
    Ok(files)
}

impl Store {
    pub fn files(&self, page: &str) -> Result<Snapshot> {
        self.cache_get("files-v1", &page_id(page)?)
    }
    pub fn file(&self, id: &str) -> Result<FileRef> {
        let value: FileRef = self.cache_get("file-refs-v1", identity(id)?)?;
        ensure!(value.id == id, "file identity mismatch");
        Ok(value)
    }
}

pub async fn list(
    store: &Store,
    notion: Option<&Notion>,
    page: &str,
    max_blocks: usize,
) -> Result<Snapshot> {
    let page = page_id(page)?;
    let Some(notion) = notion else {
        return store.files(&page);
    };
    let value = notion
        .request(Method::GET, &format!("/pages/{page}"), None)
        .await?;
    ensure!(
        page_id(value["id"].as_str().context("page missing ID")?)? == page,
        "file discovery page identity mismatch"
    );
    let mut found = page_files(&page, &value)?;
    for id in notion
        .descendant_blocks(&page, max_blocks)
        .await?
        .into_iter()
        .skip(1)
    {
        let block = notion
            .request(Method::GET, &format!("/blocks/{id}"), None)
            .await?;
        ensure!(
            page_id(block["id"].as_str().context("block missing ID")?)? == id,
            "file discovery block identity mismatch"
        );
        if block["type"] == "child_page" {
            let value = notion
                .request(Method::GET, &format!("/pages/{id}"), None)
                .await?;
            found.extend(page_files(&id, &value)?);
        } else if let Some(media) = block["type"]
            .as_str()
            .filter(|m| ["file", "image", "pdf", "audio", "video"].contains(m))
        {
            found.push(located(
                Location::Block {
                    id,
                    media: media.into(),
                },
                &block[media],
            )?);
        }
    }
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for (file, _) in found {
        if seen.insert(file.id.clone()) {
            files.push(file);
        }
    }
    for file in &files {
        store.cache_save("file-refs-v1", &file.id, file)?;
    }
    let snapshot = Snapshot {
        page: page.clone(),
        files,
    };
    store.cache_save("files-v1", &page, &snapshot)?;
    Ok(snapshot)
}

async fn refresh(notion: &Notion, reference: &FileRef) -> Result<String> {
    let found = match &reference.location {
        Location::Block { id, media } => {
            let value = notion
                .request(Method::GET, &format!("/blocks/{}", page_id(id)?), None)
                .await?;
            ensure!(
                page_id(value["id"].as_str().context("block missing ID")?)? == *id
                    && value["type"] == *media,
                "file block identity/type changed"
            );
            vec![located(reference.location.clone(), &value[media])?]
        }
        Location::Page { id, slot } => {
            ensure!(
                ["icon", "cover"].contains(&slot.as_str()),
                "invalid file slot"
            );
            let value = notion
                .request(Method::GET, &format!("/pages/{}", page_id(id)?), None)
                .await?;
            page_files(id, &value)?
        }
        Location::Property { page, .. } => {
            let value = notion
                .request(Method::GET, &format!("/pages/{}", page_id(page)?), None)
                .await?;
            page_files(page, &value)?
        }
    };
    found
        .into_iter()
        .find(|(file, _)| file.id == reference.id && file.asset == reference.asset)
        .map(|(_, url)| url)
        .context("asset changed or disappeared; list files again before fetching")
}

#[derive(Debug)]
struct Expired;
impl std::fmt::Display for Expired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "file URL expired or access denied")
    }
}
impl std::error::Error for Expired {}

pub struct Downloader {
    client: Client,
    allow_loopback_http: bool,
}
impl Downloader {
    pub fn new(allow_loopback_http: bool) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(60))
                .build()?,
            allow_loopback_http,
        })
    }
    async fn download(
        &self,
        url: &str,
        reference: &FileRef,
        directory: &Path,
        max_bytes: u64,
    ) -> Result<LocalFile> {
        let parsed = reqwest::Url::parse(url)?;
        ensure!(
            parsed.username().is_empty()
                && parsed.password().is_none()
                && (parsed.scheme() == "https"
                    || (self.allow_loopback_http
                        && parsed.scheme() == "http"
                        && ["127.0.0.1", "localhost", "[::1]"]
                            .contains(&parsed.host_str().unwrap_or_default()))),
            "download requires HTTPS without credentials"
        );
        filename(&reference.name)?;
        identity(&reference.id)?;
        let extension = Path::new(&reference.name)
            .extension()
            .and_then(|s| s.to_str())
            .filter(|s| {
                !s.is_empty() && s.len() <= 10 && s.bytes().all(|b| b.is_ascii_alphanumeric())
            });
        let name = match extension {
            Some(ext) => format!("{}.{}", reference.id, ext.to_lowercase()),
            None => reference.id.clone(),
        };
        let path = directory.join(&name);
        ensure!(
            !path.try_exists()?,
            "download path already exists; file preserved"
        );
        let mut response = self
            .client
            .get(parsed)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("file download request failed; signed URL omitted"))?;
        if [401, 403, 404].contains(&response.status().as_u16()) {
            return Err(Expired.into());
        }
        ensure!(
            response.status().is_success(),
            "download HTTP {}; redirects are not followed",
            response.status()
        );
        ensure!(
            response.content_length().is_none_or(|n| n <= max_bytes),
            "download exceeds byte limit"
        );
        let mut tmp = NamedTempFile::new_in(directory)?;
        let mut hash = Sha256::new();
        let mut size = 0u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("download interrupted; existing files preserved"))?
        {
            size = size
                .checked_add(chunk.len() as u64)
                .context("download size overflow")?;
            ensure!(size <= max_bytes, "download exceeds byte limit");
            tmp.write_all(&chunk)?;
            hash.update(&chunk);
        }
        tmp.as_file().sync_all()?;
        tmp.persist_noclobber(path)
            .context("persist download without overwriting existing bytes")?;
        fs::File::open(directory)?.sync_all()?;
        Ok(LocalFile {
            id: reference.id.clone(),
            directory: directory.into(),
            filename: name,
            sha256: format!("{:x}", hash.finalize()),
            size,
        })
    }
}

pub(crate) fn checksum(path: &Path) -> Result<String> {
    let mut input = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub async fn fetch(
    store: &Store,
    notion: &Notion,
    downloader: &Downloader,
    id: &str,
    directory: Option<&Path>,
    max_bytes: u64,
    external: bool,
) -> Result<LocalFile> {
    ensure!(max_bytes > 0, "max-bytes must be positive");
    let reference = store.file(id)?;
    ensure!(
        !reference.external || external,
        "external file requires --external"
    );
    let directory = directory
        .map(Path::to_path_buf)
        .unwrap_or_else(|| store.root().join("files"));
    fs::create_dir_all(&directory)?;
    let directory = fs::canonicalize(directory)?;
    let key = format!("{}:{}", reference.id, directory.display());
    if let Ok(local) = store.cache_get::<LocalFile>("files-local-v1", &key) {
        filename(&local.filename)?;
        ensure!(
            local.id == reference.id
                && local.directory == directory
                && checksum(&directory.join(&local.filename))? == local.sha256,
            "local download changed; file preserved"
        );
        ensure!(local.size <= max_bytes, "cached file exceeds byte limit");
        return Ok(local);
    }
    for attempt in 0..2 {
        let url = refresh(notion, &reference).await?;
        match downloader
            .download(&url, &reference, &directory, max_bytes)
            .await
        {
            Ok(local) => {
                store.cache_save("files-local-v1", &key, &local)?;
                return Ok(local);
            }
            Err(error) if attempt == 0 && error.is::<Expired>() => continue,
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}
