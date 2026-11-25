use crate::{
    db::Store,
    document::page_id,
    journal::{Intent, State},
    notion::Notion,
};
use anyhow::{ensure, Context, Result};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Target {
    Page(String),
    Block(String),
    Discussion(String),
}

impl Target {
    pub fn canonical(&self) -> Result<Self> {
        Ok(match self {
            Self::Page(id) => Self::Page(page_id(id)?),
            Self::Block(id) => Self::Block(page_id(id)?),
            Self::Discussion(id) => Self::Discussion(page_id(id)?),
        })
    }
    pub fn id(&self) -> &str {
        match self {
            Self::Page(id) | Self::Block(id) | Self::Discussion(id) => id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Comment {
    pub id: String,
    pub discussion_id: String,
    pub parent: Value,
    pub created_time: String,
    pub last_edited_time: String,
    pub created_by: Value,
    pub rich_text: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Snapshot {
    pub target: String,
    pub fetched_at: u64,
    pub recursive: bool,
    pub unresolved_only: bool,
    pub comments: Vec<Comment>,
}

pub fn validate(markdown: &str) -> Result<()> {
    ensure!(!markdown.trim().is_empty(), "comment must not be empty");
    for line in markdown.lines() {
        let indented =
            !line.trim().is_empty() && (line.starts_with("    ") || line.starts_with('\t'));
        let line = line.trim_start();
        let separator =
            line.len() >= 3 && (line.chars().all(|c| c == '=') || line.chars().all(|c| c == '-'));
        let table_separator = line.contains('|')
            && line
                .chars()
                .all(|c| c.is_whitespace() || matches!(c, '|' | ':' | '-'));
        let numbered = line.split_once(". ").is_some_and(|(number, _)| {
            !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
        });
        ensure!(!["# ","## ","### ","#### ","##### ","###### ","> ","- ","* ","+ ","```","~~~","|"].iter().any(|p|line.starts_with(p)) && !numbered && !indented && !separator && !table_separator,
            "comments support inline Markdown only; headings, lists, tables, blockquotes and fenced blocks are unsupported");
    }
    Ok(())
}

impl Notion {
    /// Complete pagination or error; callers retain their previous cache on error.
    pub(crate) async fn list_all(
        &self,
        path: &str,
        params: &[(&str, &str)],
        max_pages: usize,
    ) -> Result<Vec<Value>> {
        let mut results = Vec::new();
        let mut cursor = None::<String>;
        let mut seen = HashSet::new();
        for _ in 0..max_pages {
            let mut url = reqwest::Url::parse(&format!("https://api.notion.com{path}"))?;
            url.query_pairs_mut()
                .append_pair("page_size", "100")
                .extend_pairs(params.iter().copied());
            if let Some(cursor) = &cursor {
                url.query_pairs_mut().append_pair("start_cursor", cursor);
            }
            let value = self
                .request(
                    Method::GET,
                    &format!("{}?{}", url.path(), url.query().unwrap_or_default()),
                    None,
                )
                .await?;
            results.extend(
                value["results"]
                    .as_array()
                    .context("missing paginated results")?
                    .iter()
                    .cloned(),
            );
            if !value["has_more"]
                .as_bool()
                .context("missing pagination flag")?
            {
                return Ok(results);
            }
            let next = value["next_cursor"]
                .as_str()
                .filter(|s| !s.is_empty())
                .context("missing next cursor")?
                .to_owned();
            ensure!(
                seen.insert(next.clone()),
                "repeated pagination cursor; incomplete result"
            );
            cursor = Some(next);
        }
        anyhow::bail!("pagination limit reached; incomplete result; prior cache preserved")
    }

    /// Bounded all-block traversal for comments/files; ordinary links are ignored.
    pub(crate) async fn descendant_blocks(
        &self,
        root: &str,
        max_blocks: usize,
    ) -> Result<Vec<String>> {
        ensure!(max_blocks > 0, "max-blocks must be positive");
        let root = page_id(root)?;
        let mut queue = VecDeque::from([(root.clone(), None::<String>)]);
        let mut ids = vec![root.clone()];
        let mut seen = HashSet::from([root]);
        let mut cursors = HashSet::new();
        let mut requests = 0;
        while let Some((id, cursor)) = queue.pop_front() {
            ensure!(
                requests < max_blocks,
                "block traversal request limit reached; incomplete result"
            );
            requests += 1;
            let mut url =
                reqwest::Url::parse(&format!("https://api.notion.com/blocks/{id}/children"))?;
            url.query_pairs_mut().append_pair("page_size", "100");
            if let Some(cursor) = cursor {
                url.query_pairs_mut().append_pair("start_cursor", &cursor);
            }
            let value = self
                .request(
                    Method::GET,
                    &format!("{}?{}", url.path(), url.query().unwrap_or_default()),
                    None,
                )
                .await?;
            for block in value["results"]
                .as_array()
                .context("missing block results")?
            {
                ensure!(
                    ids.len() <= max_blocks,
                    "block traversal limit reached; incomplete result"
                );
                let child = page_id(block["id"].as_str().context("block missing ID")?)?;
                ensure!(
                    seen.insert(child.clone()),
                    "cycle/duplicate in block traversal"
                );
                ids.push(child.clone());
                if block["type"] != "child_database"
                    && (block["has_children"] == true || block["type"] == "child_page")
                {
                    queue.push_back((child, None));
                }
            }
            if value["has_more"]
                .as_bool()
                .context("missing pagination flag")?
            {
                let cursor = value["next_cursor"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .context("missing cursor")?
                    .to_owned();
                ensure!(
                    cursors.insert((id.clone(), cursor.clone())),
                    "repeated block cursor"
                );
                queue.push_front((id, Some(cursor)));
            }
        }
        Ok(ids)
    }

    pub async fn comment(&self, id: &str) -> Result<Comment> {
        let id = page_id(id)?;
        let comment: Comment = serde_json::from_value(
            self.request(Method::GET, &format!("/comments/{id}"), None)
                .await?,
        )?;
        ensure!(page_id(&comment.id)? == id, "comment identity mismatch");
        Ok(comment)
    }
}

impl Store {
    pub fn comments(&self, target: &str, recursive: bool) -> Result<Snapshot> {
        self.cache_get("comments-v1", &format!("{}:{recursive}", page_id(target)?))
    }
    pub(crate) fn save_comment(&self, comment: &Comment) -> Result<()> {
        self.cache_save("comment-objects-v1", &page_id(&comment.id)?, comment)
    }
}

pub async fn pull(
    store: &Store,
    notion: &Notion,
    target: &str,
    recursive: bool,
    max_blocks: usize,
) -> Result<Snapshot> {
    let target = page_id(target)?;
    let targets = if recursive {
        notion.descendant_blocks(&target, max_blocks).await?
    } else {
        vec![target.clone()]
    };
    let mut comments = Vec::new();
    let mut seen = HashSet::new();
    for id in targets {
        for value in notion
            .list_all("/comments", &[("block_id", &id)], 1000)
            .await?
        {
            let mut comment: Comment = serde_json::from_value(value)
                .context("partial comment response; previous snapshot preserved")?;
            comment.id = page_id(&comment.id)?;
            if seen.insert(comment.id.clone()) {
                comments.push(comment);
            }
        }
    }
    let snapshot = Snapshot {
        target: target.clone(),
        fetched_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        recursive,
        unresolved_only: true,
        comments,
    };
    store.cache_save("comments-v1", &format!("{target}:{recursive}"), &snapshot)?;
    Ok(snapshot)
}

pub async fn add(store: &Store, notion: &Notion, target: Target, markdown: &str) -> Result<Value> {
    validate(markdown)?;
    let target = target.canonical()?;
    let mut op = store.prepare(
        Intent::Comment {
            target: target.clone(),
            markdown: markdown.to_owned(),
        },
        None,
    )?;
    let result = async {
        store.transition(&mut op, State::Sent)?;
        let mut body = json!({"markdown":markdown,"markdown_version":"v2"});
        match &target {
            Target::Page(id) => body["parent"] = json!({"page_id":id}),
            Target::Block(id) => body["parent"] = json!({"block_id":id}),
            Target::Discussion(id) => body["discussion_id"] = json!(id),
        }
        let value = notion
            .request(Method::POST, "/comments", Some(body))
            .await?;
        ensure!(
            value["object"] == "comment",
            "invalid comment creation response; inspect before repeating"
        );
        let id = page_id(
            value["id"]
                .as_str()
                .context("created comment missing identity; inspect before repeating")?,
        )?;
        eprintln!("created comment {id} (operation {})", op.id);
        op.remote_id = Some(id);
        store.transition(&mut op, State::RemoteConfirmed)?;
        // A partial API response still confirms creation. Keep exact input in the
        // journal; full details require read-comment capability on recovery/pull.
        if let Ok(comment) = serde_json::from_value::<Comment>(value.clone()) {
            store.save_comment(&comment)?;
        }
        store.transition(&mut op, State::LocallySaved)?;
        Ok(value)
    }
    .await;
    store.operation_result(&mut op, result)
}
