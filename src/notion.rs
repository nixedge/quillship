use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context, Result};
use reqwest::{header, Client, Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::document::{page_id, RemoteDocument};

pub const API_VERSION: &str = "2026-03-11";

pub enum CreateParent {
    Workspace,
    Page(String),
}

#[derive(Debug)]
pub struct CreatedPage {
    pub id: String,
    pub url: String,
}

/// Validate authoring inputs before constructing a client or sending a request.
pub fn validate_create(parent: &CreateParent, title: &str, markdown: &str) -> Result<()> {
    ensure!(!title.trim().is_empty(), "page title must not be empty");
    ensure!(
        title.chars().count() <= 2000,
        "page title exceeds 2000 characters"
    );
    if let CreateParent::Page(id) = parent {
        page_id(id)?;
    }
    ensure!(
        !markdown.contains("<unknown"),
        "cannot create unsupported/unknown block placeholders"
    );
    Ok(())
}

pub struct Notion {
    client: Client,
    base: String,
    last_request: Mutex<Option<Instant>>,
}

#[derive(Debug, Deserialize)]
struct Page {
    id: String,
    url: String,
    last_edited_time: String,
    properties: Value,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    in_trash: bool,
    #[serde(default)]
    is_archived: bool,
}

impl Page {
    fn title(&self) -> String {
        self.properties
            .as_object()
            .into_iter()
            .flat_map(|p| p.values())
            .find_map(|p| p.get("title").and_then(Value::as_array))
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| {
                        part.get("plain_text")
                            .and_then(Value::as_str)
                            .or_else(|| part.pointer("/text/content").and_then(Value::as_str))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)]
struct Markdown {
    markdown: String,
    truncated: Option<bool>,
    unknown_block_ids: Option<Vec<String>>,
    #[serde(default)]
    warnings: Vec<Value>,
}

impl Markdown {
    fn diagnostics(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        match self.truncated {
            Some(true) => warnings.push("Notion truncated this document".into()),
            None => {
                warnings.push("Notion omitted the truncation flag; completeness is unknown".into())
            }
            Some(false) => (),
        }
        match &self.unknown_block_ids {
            Some(ids) if !ids.is_empty() => warnings.push(format!(
                "Notion could not load {} block(s): {}",
                ids.len(),
                ids.join(", ")
            )),
            None => {
                warnings.push("Notion omitted unknown_block_ids; completeness is unknown".into())
            }
            _ => (),
        }
        if self.markdown.contains("<unknown") {
            warnings.push("Markdown contains unknown/unsupported block placeholders".into());
        }
        for warning in &self.warnings {
            warnings.push(format!("Notion Markdown warning: {warning}"));
        }
        warnings
    }

    fn complete(self) -> Result<String> {
        let warnings = self.diagnostics();
        ensure!(
            warnings.is_empty(),
            "Notion returned incomplete/unsupported Markdown or warnings: {}; local edits and baseline preserved; inspect Notion before retrying a write",
            warnings.join("; ")
        );
        Ok(self.markdown)
    }
}

impl Notion {
    pub fn from_env() -> Result<Self> {
        Self::new(
            &std::env::var("NOTION_TOKEN").context("set NOTION_TOKEN for pull/push/create")?,
            "https://api.notion.com/v1",
        )
    }

    /// Alternate base URL allows isolated HTTP contract tests.
    pub fn new(token: &str, base: &str) -> Result<Self> {
        ensure!(!token.trim().is_empty(), "NOTION_TOKEN is empty");
        let mut headers = header::HeaderMap::new();
        let mut auth = header::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| anyhow::anyhow!("invalid NOTION_TOKEN header"))?;
        auth.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, auth);
        headers.insert(
            "Notion-Version",
            header::HeaderValue::from_static(API_VERSION),
        );
        let client = Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .user_agent(concat!("quillship/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            client,
            base: base.trim_end_matches('/').to_owned(),
            last_request: Mutex::new(None),
        })
    }

    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        for attempt in 0..=3 {
            {
                let mut last = self.last_request.lock().await;
                if let Some(at) = *last {
                    let wait = Duration::from_millis(350).saturating_sub(at.elapsed());
                    tokio::time::sleep(wait).await;
                }
                *last = Some(Instant::now());
            }
            let mut request = self
                .client
                .request(method.clone(), format!("{}{path}", self.base));
            if let Some(body) = &body {
                request = request.json(body);
            }
            let response = request.send().await
                .with_context(|| format!("Notion {method} failed; write outcome may be uncertain; local baseline preserved"))?;
            let status = response.status();
            if status == StatusCode::TOO_MANY_REQUESTS && attempt < 3 {
                let delay = response
                    .headers()
                    .get("retry-after")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(1 << attempt);
                ensure!(delay <= 30, "Notion rate limited; retry after {delay}s");
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }
            ensure!(status.is_success(),
                "Notion {method} {path}: HTTP {status}; check access/capabilities, limits, and page state; local baseline preserved");
            ensure!(
                status != StatusCode::ACCEPTED,
                "unexpected async Notion response; inspect remote before retrying"
            );
            return response
                .json()
                .await
                .context("invalid Notion JSON response");
        }
        unreachable!()
    }

    pub async fn discover(&self, query: Option<&str>) -> Result<Vec<String>> {
        let mut cursor = None::<String>;
        let mut seen_cursors = HashSet::new();
        let mut ids = Vec::new();
        let mut seen_ids = HashSet::new();
        loop {
            let mut body =
                json!({"filter": {"property": "object", "value": "page"}, "page_size": 100});
            if let Some(query) = query {
                body["query"] = json!(query);
            }
            if let Some(cursor) = &cursor {
                body["start_cursor"] = json!(cursor);
            }
            let value = self.request(Method::POST, "/search", Some(body)).await?;
            let results = value["results"]
                .as_array()
                .context("Notion search missing results")?;
            for page in results {
                if page["object"] != "page" {
                    continue;
                }
                let id = page_id(page["id"].as_str().context("search page missing UUID")?)?;
                if seen_ids.insert(id.clone()) {
                    ids.push(id);
                }
            }
            if !value["has_more"]
                .as_bool()
                .context("search missing has_more")?
            {
                break;
            }
            let next = value["next_cursor"]
                .as_str()
                .context("search missing next_cursor")?
                .to_owned();
            ensure!(
                seen_cursors.insert(next.clone()),
                "Notion search repeated pagination cursor"
            );
            cursor = Some(next);
        }
        Ok(ids)
    }

    /// Create exactly one page. Never repeat a write after an ambiguous outcome.
    pub async fn create(
        &self,
        parent: &CreateParent,
        title: &str,
        markdown: &str,
    ) -> Result<CreatedPage> {
        validate_create(parent, title, markdown)?;
        let parent = match parent {
            CreateParent::Workspace => json!({"workspace": true}),
            CreateParent::Page(id) => json!({"page_id": page_id(id)?}),
        };
        let value = self
            .request(
                Method::POST,
                "/pages",
                Some(json!({
                    "parent": parent,
                    "properties": {"title": {"title": [{"type": "text", "text": {"content": title}}]}},
                    "markdown": markdown, "markdown_version": "v2", "allow_async": false,
                    "notifications": {"mode": "silent"}
                })),
            )
            .await
            .context("page creation failed; outcome may be uncertain; inspect Notion before repeating create")?;
        let identity = || -> Result<CreatedPage> {
            ensure!(value["object"] == "page", "creation response is not a page");
            Ok(CreatedPage {
                id: page_id(value["id"].as_str().context("created page missing UUID")?)?,
                url: value["url"]
                    .as_str()
                    .filter(|url| !url.is_empty())
                    .context("created page missing URL")?
                    .to_owned(),
            })
        };
        identity().context(
            "invalid creation response; page may exist; inspect Notion before repeating create",
        )
    }

    async fn metadata(&self, id: &str) -> Result<Page> {
        let page: Page = serde_json::from_value(
            self.request(Method::GET, &format!("/pages/{id}"), None)
                .await?,
        )?;
        ensure!(
            page_id(&page.id)? == id,
            "Notion returned unexpected page identity"
        );
        ensure!(
            !page.archived && !page.in_trash && !page.is_archived,
            "page {id} is archived or trashed"
        );
        Ok(page)
    }

    pub async fn retrieve(&self, id: &str) -> Result<RemoteDocument> {
        let remote = self.retrieve_for_pull(id).await?;
        ensure!(
            remote.sync_warnings.is_empty(),
            "Notion returned incomplete/unsupported Markdown: {}; local content preserved",
            remote.sync_warnings.join("; ")
        );
        Ok(remote)
    }

    /// Retain readable Markdown and diagnostics even when some blocks are unavailable.
    /// Push uses strict retrieval above and must never write a partial snapshot.
    pub async fn retrieve_for_pull(&self, id: &str) -> Result<RemoteDocument> {
        let id = page_id(id)?;
        let before = self.metadata(&id).await?;
        let markdown: Markdown = serde_json::from_value(
            self.request(
                Method::GET,
                &format!("/pages/{id}/markdown?markdown_version=v2"),
                None,
            )
            .await?,
        )?;
        let sync_warnings = markdown.diagnostics();
        let after = self.metadata(&id).await?;
        ensure!(
            before.last_edited_time == after.last_edited_time,
            "page {id} changed while being retrieved; retry pull"
        );
        Ok(RemoteDocument {
            id,
            title: after.title(),
            url: after.url,
            last_edited_time: after.last_edited_time,
            markdown: markdown.markdown,
            sync_warnings,
        })
    }

    pub async fn replace(&self, id: &str, markdown: &str) -> Result<String> {
        let id = page_id(id)?;
        if markdown.contains("<unknown") {
            bail!("cannot push unsupported/unknown block placeholders");
        }
        let response: Markdown = serde_json::from_value(
            self.request(
                Method::PATCH,
                &format!("/pages/{id}/markdown"),
                Some(json!({
                    "type": "replace_content", "markdown_version": "v2", "allow_async": false,
                    "replace_content": { "new_str": markdown, "allow_deleting_content": false }
                })),
            )
            .await?,
        )?;
        response.complete()
    }
}
