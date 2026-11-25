use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::Command;

use anyhow::{ensure, Context, Result};

use crate::db::Store;
use crate::document::{page_id, Document};
use crate::journal::{Intent, Operation, State};
use crate::notion::{CreateParent, Notion};
use crate::review::Snapshot;

pub enum Reconcile {
    Baseline,
    Local,
    Remote,
    Merged(String),
}

/// Archive every input before changing a working file or baseline. No remote writes.
pub async fn reconcile(
    store: &Store,
    notion: Option<&Notion>,
    id: &str,
    choice: Reconcile,
) -> Result<(Document, Snapshot)> {
    let previous = store.get(id)?;
    let mut next = previous.clone();
    let (remote, action) = match choice {
        Reconcile::Baseline => {
            ensure!(
                !previous.incomplete && previous.sync_warnings.is_empty(),
                "saved baseline is incomplete; fetch a complete remote snapshot instead"
            );
            next.markdown = previous.baseline.clone();
            (None, "take-baseline")
        }
        choice => {
            let remote = notion
                .context("remote reconciliation requires a Notion client")?
                .retrieve(&previous.id)
                .await?;
            let (body, action) = match choice {
                Reconcile::Local => (previous.markdown.clone(), "take-local"),
                Reconcile::Remote => (remote.markdown.clone(), "take-remote"),
                Reconcile::Merged(body) => (body, "merge"),
                Reconcile::Baseline => unreachable!(),
            };
            next.markdown = body;
            next.baseline = remote.markdown.clone();
            next.title = remote.title.clone();
            next.url = remote.url.clone();
            next.last_edited_time = remote.last_edited_time.clone();
            next.incomplete = false;
            next.sync_warnings.clear();
            (Some(remote), action)
        }
    };
    let snapshot = store.archive(&previous, remote, action, &next.markdown)?;
    store.save(&next, Some(&previous)).with_context(|| {
        format!(
            "reconciliation not completed; recovery snapshot {} retained",
            snapshot.id
        )
    })?;
    Ok((next, snapshot))
}

pub async fn create_document(
    store: &Store,
    notion: &Notion,
    parent: &CreateParent,
    title: &str,
    markdown: &str,
) -> Result<Document> {
    crate::notion::validate_create(parent, title, markdown)?;
    let mut operation = store.prepare(
        Intent::Create {
            parent: match parent {
                CreateParent::Workspace => None,
                CreateParent::Page(id) => Some(page_id(id)?),
            },
            title: title.to_owned(),
            markdown: markdown.to_owned(),
        },
        None,
    )?;
    let result = async {
    store.transition(&mut operation, State::Sent)?;
    let created = notion.create(parent, title, markdown).await?;
    // Report identity before any readback or persistence operation can fail.
    eprintln!("created {} {}", created.id, created.url);
    operation.remote_id = Some(created.id.clone());
    store.transition(&mut operation, State::RemoteConfirmed)?;
    let persist = async {
        let remote = notion.retrieve_for_pull(&created.id).await?;
        let mut doc = Document::from_remote(remote);
        doc.markdown = markdown.to_owned();
        store.save(&doc, None)?;
        store.transition(&mut operation, State::LocallySaved)?;
        Ok::<_, anyhow::Error>(doc)
    };
    persist.await.with_context(|| format!(
        "page {} exists at {}, but local save/readback failed; preserve the input and pull this UUID; do not repeat create",
        created.id, created.url
    ))
    }.await;
    store.operation_result(&mut operation, result)
}

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
    let mut operation = store.prepare(
        Intent::Push {
            document: doc.clone(),
        },
        None,
    )?;
    push_recorded(store, notion, doc, &mut operation).await
}

/// Prepare every batch item first, so an interrupted batch retains unattempted inputs.
pub async fn push_batch(store: &Store, notion: &Notion, docs: &[Document]) -> Result<()> {
    let batch = store.journal_id()?;
    let mut entries = docs
        .iter()
        .map(|document| {
            store.prepare(
                Intent::Push {
                    document: document.clone(),
                },
                Some(batch),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    for (doc, op) in docs.iter().zip(&mut entries) {
        push_recorded(store, notion, doc, op).await?;
        println!("pushed {} (operation {}, batch {})", doc.id, op.id, batch);
    }
    Ok(())
}

async fn push_recorded(
    store: &Store,
    notion: &Notion,
    doc: &Document,
    operation: &mut Operation,
) -> Result<()> {
    let result = async {
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
        store.save(&next, Some(&current))?;
        return store.transition(operation, State::LocallySaved);
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
    store.transition(operation, State::Sent)?;
    let canonical = notion.replace(&doc.id, &doc.markdown).await?;
    operation.remote_id = Some(doc.id.clone());
    operation.remote_body = Some(canonical.clone());
    store.transition(operation, State::RemoteConfirmed)?;
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
    store.save(&next, Some(&current))?;
    store.transition(operation, State::LocallySaved)
    }.await;
    store.operation_result(operation, result)
}

pub async fn rename_page(
    store: &Store,
    notion: &Notion,
    id: &str,
    title: &str,
) -> Result<crate::notion::PageInfo> {
    let id = page_id(id)?;
    crate::notion::validate_create(&CreateParent::Page(id.clone()), title, "")?;
    let mut op = store.prepare(
        Intent::Rename {
            id,
            title: title.to_owned(),
        },
        None,
    )?;
    let result = async {
        store.transition(&mut op, State::Sent)?;
        let info = notion
            .rename(op.target().expect("rename target"), title)
            .await?;
        eprintln!("renamed {} {}", info.id, info.url);
        op.remote_id = Some(info.id.clone());
        store.transition(&mut op, State::RemoteConfirmed)?;
        store.save_metadata(&info)?;
        store.transition(&mut op, State::LocallySaved)?;
        Ok(info)
    }
    .await;
    store.operation_result(&mut op, result)
}

pub async fn move_page(
    store: &Store,
    notion: &Notion,
    id: &str,
    parent: &str,
) -> Result<crate::notion::PageInfo> {
    let id = page_id(id)?;
    let parent = page_id(parent)?;
    ensure!(id != parent, "a page cannot be its own parent");
    let mut op = store.prepare(
        Intent::Move {
            id: id.clone(),
            parent: parent.clone(),
        },
        None,
    )?;
    let result = async {
        store.transition(&mut op, State::Sent)?;
        let info = notion.move_page(&id, &parent).await?;
        eprintln!("moved {} {}", info.id, info.url);
        op.remote_id = Some(info.id.clone());
        store.transition(&mut op, State::RemoteConfirmed)?;
        store.save_metadata(&info)?;
        store.transition(&mut op, State::LocallySaved)?;
        Ok(info)
    }
    .await;
    store.operation_result(&mut op, result)
}

pub fn read_markdown(source: &Path) -> Result<String> {
    if source == Path::new("-") {
        let mut content = String::new();
        io::stdin()
            .read_to_string(&mut content)
            .context("read Markdown from stdin")?;
        Ok(content)
    } else {
        fs::read_to_string(source).context("read Markdown input file")
    }
}

pub fn edit_file(store: &Store, id: &str, source: &Path) -> Result<()> {
    let previous = store.get(id)?;
    let markdown = read_markdown(source)?;
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
