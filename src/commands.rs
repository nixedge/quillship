use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::Command;

use anyhow::{ensure, Context, Result};

use crate::db::Store;
use crate::document::{page_id, Document};
use crate::notion::Notion;

#[derive(Debug, Default)]
pub struct PullReport {
    pub pulled: usize,
    pub incomplete: usize,
    pub failures: Vec<(String, String)>,
}

pub async fn pull(
    store: &Store,
    notion: &Notion,
    ids: &[String],
    query: Option<&str>,
) -> Result<PullReport> {
    let ids = if ids.is_empty() {
        notion.discover(query).await?
    } else {
        ids.to_vec()
    };
    let mut seen = HashSet::new();
    let mut report = PullReport::default();
    for id in ids {
        let id = page_id(&id)?;
        if !seen.insert(id.clone()) {
            continue;
        }
        match pull_document(store, notion, &id).await {
            Ok(next) => {
                report.pulled += 1;
                if !next.sync_warnings.is_empty() || next.incomplete {
                    report.incomplete += 1;
                    eprintln!(
                        "warning {id}: {}; pushing disabled",
                        next.sync_warnings.join("; ")
                    );
                }
                println!(
                    "pulled {}{}",
                    id,
                    if next.dirty() {
                        " (local edits preserved)"
                    } else {
                        ""
                    }
                );
            }
            Err(error) => {
                let message = format!("{error:#}");
                eprintln!("failed {id}: {message}");
                report.failures.push((id, message));
            }
        }
    }
    Ok(report)
}

async fn pull_document(store: &Store, notion: &Notion, id: &str) -> Result<Document> {
    let remote = notion.retrieve_for_pull(id).await?;
    // Include edits made during the network read.
    let previous = if store.contains(id)? {
        Some(store.get(id)?)
    } else {
        None
    };
    let next = match &previous {
        Some(previous) => previous.pulled(remote)?,
        None => Document::from_remote(remote),
    };
    store.save(&next, previous.as_ref())?;
    Ok(next)
}

pub fn selected(store: &Store, ids: &[String]) -> Result<Vec<Document>> {
    if ids.is_empty() {
        return store.all();
    }
    let mut seen = HashSet::new();
    ids.iter()
        .map(|id| page_id(id))
        .collect::<Result<Vec<_>>>()?
        .iter()
        .filter(|id| seen.insert((*id).clone()))
        .map(|id| store.get(id))
        .collect()
}

pub fn pending(store: &Store, ids: &[String]) -> Result<Vec<Document>> {
    Ok(selected(store, ids)?
        .into_iter()
        .filter(Document::dirty)
        .collect())
}

/// Push one snapshot, leaving the baseline untouched on HTTP or validation failure.
pub async fn push_document(store: &Store, notion: &Notion, doc: &Document) -> Result<()> {
    if !doc.dirty() {
        return Ok(());
    }
    ensure!(!doc.incomplete && doc.sync_warnings.is_empty(),
        "cannot push {}: the last pull was incomplete; pull a complete snapshot before pushing; local edits preserved", doc.id);
    let remote = notion.retrieve(&doc.id).await?;
    if remote.markdown == doc.markdown {
        let current = store.get(&doc.id)?;
        let mut next = current.clone();
        next.baseline = remote.markdown;
        next.last_edited_time = remote.last_edited_time;
        next.title = remote.title;
        next.url = remote.url;
        return store.save(&next, Some(&current));
    }
    ensure!(
        remote.markdown == doc.baseline,
        "conflict on {}: Notion changed since pull; local file preserved",
        doc.id
    );
    // Check again immediately before the write. An external editor isn't locked.
    ensure!(
        store.get(&doc.id)?.markdown == doc.markdown,
        "local file {} changed during preflight; retry push",
        doc.id
    );
    let canonical = notion.replace(&doc.id, &doc.markdown).await?;
    let current = store.get(&doc.id)?;
    let mut next = current.clone();
    next.baseline = canonical;
    next.last_edited_time = remote.last_edited_time;
    next.title = remote.title;
    next.url = remote.url;
    // Retain the exact submitted Markdown, even if Notion normalized it. A
    // difference stays visible as dirty; it must never silently discard local text.
    if next.markdown != next.baseline {
        eprintln!("{}: local Markdown differs from Notion's response; local file retained and still marked dirty", doc.id);
    }
    store.save(&next, Some(&current))
}

pub fn edit_file(store: &Store, id: &str, source: &Path) -> Result<()> {
    let previous = store.get(id)?;
    let markdown = if source == Path::new("-") {
        let mut content = String::new();
        io::stdin()
            .read_to_string(&mut content)
            .context("read Markdown from stdin")?;
        content
    } else {
        fs::read_to_string(source).context("read Markdown input file")?
    };
    let mut next = previous.clone();
    next.markdown = markdown;
    store.save(&next, Some(&previous))
}

pub fn edit(store: &Store, id: &str) -> Result<()> {
    let doc = store.get(id)?;
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .context("set VISUAL or EDITOR, or use edit --file PATH (use - for stdin)")?;
    let parts = shlex::split(&editor).context("invalid quoting in VISUAL/EDITOR")?;
    let (program, args) = parts.split_first().context("empty editor command")?;
    let path = store.path(&doc.id)?;
    let status = Command::new(program)
        .args(args)
        .arg(&path)
        .status()
        .with_context(|| format!("start editor; working file: {}", path.display()))?;
    ensure!(
        status.success(),
        "editor failed; edits remain at {}; rerun view/status to ingest",
        path.display()
    );
    store.get(&doc.id)?;
    Ok(())
}
