use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One page's locally edited body and last complete remote snapshot.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Document {
    pub id: String,
    pub title: String,
    pub url: String,
    pub last_edited_time: String,
    pub markdown: String,
    pub baseline: String,
    /// Whether the stored baseline is a partial Notion representation.
    #[serde(default)]
    pub incomplete: bool,
    #[serde(default)]
    pub sync_warnings: Vec<String>,
}

impl Document {
    pub fn dirty(&self) -> bool {
        self.markdown != self.baseline
    }

    pub fn from_remote(remote: RemoteDocument) -> Self {
        Self {
            id: remote.id,
            title: remote.title,
            url: remote.url,
            last_edited_time: remote.last_edited_time,
            baseline: remote.markdown.clone(),
            markdown: remote.markdown,
            incomplete: !remote.sync_warnings.is_empty(),
            sync_warnings: remote.sync_warnings,
        }
    }

    /// Three-way reconciliation: preserve edits, accept remote, or reject divergence.
    pub fn pulled(&self, remote: RemoteDocument) -> Result<Self> {
        ensure!(self.id == remote.id, "remote page identity changed");
        if !self.incomplete && !remote.sync_warnings.is_empty() {
            // Keep this flag false: the saved baseline is still complete. Repeated
            // partial pulls must never replace that last complete snapshot.
            let mut next = self.clone();
            next.sync_warnings = remote.sync_warnings;
            next.sync_warnings.push(
                "Kept the previous complete local snapshot because the new response is partial"
                    .into(),
            );
            return Ok(next);
        }
        ensure!(
            !self.dirty() || remote.markdown == self.baseline || remote.markdown == self.markdown,
            "conflict on {}: local and Notion content both changed; local file preserved",
            self.id
        );
        let keep_local = self.dirty() && remote.markdown == self.baseline;
        let mut next = Self::from_remote(remote);
        if keep_local {
            next.markdown.clone_from(&self.markdown);
        }
        Ok(next)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RemoteDocument {
    pub id: String,
    pub title: String,
    pub url: String,
    pub last_edited_time: String,
    pub markdown: String,
    pub sync_warnings: Vec<String>,
}

pub fn page_id(input: &str) -> Result<String> {
    if let Ok(id) = Uuid::parse_str(input) {
        return Ok(id.hyphenated().to_string());
    }
    let url =
        reqwest::Url::parse(input).context("expected a page UUID or HTTPS Notion page URL")?;
    let host = url.host_str().context("Notion URL missing host")?;
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && ["notion.so", "notion.com", "notion.site"]
                .iter()
                .any(|domain| host == *domain || host.ends_with(&format!(".{domain}"))),
        "expected an HTTPS Notion page URL without credentials"
    );
    let segment = url
        .path_segments()
        .and_then(|parts| parts.filter(|s| !s.is_empty()).next_back())
        .context("Notion URL missing page identity")?;
    for len in [36, 32] {
        if let Some(candidate) = segment.get(segment.len().saturating_sub(len)..) {
            if let Ok(id) = Uuid::parse_str(candidate) {
                return Ok(id.hyphenated().to_string());
            }
        }
    }
    anyhow::bail!("Notion URL does not contain a page UUID")
}
