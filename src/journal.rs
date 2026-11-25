use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{db::Store, document::Document, notion::Notion};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Intent {
    RecordMove {
        id: String,
        source: String,
    },
    RecordCreate {
        source: String,
        properties: serde_json::Value,
        markdown: Option<String>,
    },
    RecordUpdate {
        id: String,
        source: String,
        properties: serde_json::Value,
        expected: serde_json::Value,
    },
    Edit {
        document: Document,
        edit: crate::targeted::Edit,
        selected: String,
    },
    Comment {
        target: crate::comments::Target,
        markdown: String,
    },
    Create {
        parent: Option<String>,
        title: String,
        markdown: String,
    },
    Push {
        document: Document,
    },
    Rename {
        id: String,
        title: String,
    },
    Move {
        id: String,
        parent: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Prepared,
    Sent,
    RemoteConfirmed,
    LocallySaved,
    Uncertain,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Operation {
    pub id: u64,
    pub batch_id: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
    pub intent: Intent,
    pub state: State,
    pub remote_id: Option<String>,
    pub remote_body: Option<String>,
}

impl Operation {
    pub fn new(id: u64, batch_id: Option<u64>, intent: Intent) -> Result<Self> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        Ok(Self {
            id,
            batch_id,
            created_at: now,
            updated_at: now,
            intent,
            state: State::Prepared,
            remote_id: None,
            remote_body: None,
        })
    }

    pub fn pending(&self) -> bool {
        !matches!(self.state, State::LocallySaved | State::Failed)
    }

    pub fn target(&self) -> Option<&str> {
        self.remote_id.as_deref().or(match &self.intent {
            Intent::Create { .. } | Intent::Comment { .. } | Intent::RecordCreate { .. } => None,
            Intent::RecordUpdate { id, .. } | Intent::RecordMove { id, .. } => Some(id),
            Intent::Push { document } | Intent::Edit { document, .. } => Some(&document.id),
            Intent::Rename { id, .. } | Intent::Move { id, .. } => Some(id),
        })
    }
}

impl Store {
    pub fn prepare(&self, intent: Intent, batch: Option<u64>) -> Result<Operation> {
        let operation = Operation::new(self.journal_id()?, batch, intent)?;
        self.write_operation(&operation)?;
        Ok(operation)
    }

    pub fn transition(&self, operation: &mut Operation, state: State) -> Result<()> {
        ensure!(
            matches!(
                (operation.state, state),
                (
                    State::Prepared,
                    State::Sent | State::Failed | State::LocallySaved
                ) | (State::Sent, State::RemoteConfirmed | State::Uncertain)
                    | (State::Uncertain, State::RemoteConfirmed)
                    | (State::RemoteConfirmed, State::LocallySaved)
            ),
            "invalid journal transition"
        );
        let mut next = operation.clone();
        next.state = state;
        next.updated_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        self.write_operation(&next)?;
        *operation = next;
        Ok(())
    }

    pub fn operation_result<T>(&self, op: &mut Operation, result: Result<T>) -> Result<T> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                let state = match op.state {
                    State::Prepared => Some(State::Failed),
                    State::Sent => Some(State::Uncertain),
                    _ => None,
                };
                if let Some(state) = state {
                    self.transition(op, state)
                        .context("persist interrupted operation state")?;
                }
                let detail = format!("{error:#}");
                Err(error).with_context(|| format!("operation {} retained ({:?}): {detail}; inspect with operation/recover before retrying", op.id, op.state))
            }
        }
    }
}

/// Observe only: never resubmit a mutation or overwrite a divergent working copy.
pub async fn recover(store: &Store, notion: &Notion, id: u64) -> Result<Operation> {
    let mut op = store.operation(id)?;
    ensure!(
        op.pending(),
        "operation is already complete or failed before sending"
    );
    ensure!(
        op.state != State::Prepared,
        "operation was never sent; inspect the input and issue a new explicit command"
    );
    let target = op.target().context("creation identity is unknown; inspect Notion manually; recovery will not repeat create")?.to_owned();
    let result = async {
        match &op.intent {
            Intent::RecordMove {source,..} => {
                let info=notion.inspect(&target).await?;
                ensure!(info.parent.as_ref().and_then(|p|p["data_source_id"].as_str()).and_then(|id|crate::document::page_id(id).ok()).as_deref()==Some(source),"record parent differs from requested source; no move repeated");
                if op.state!=State::RemoteConfirmed {store.transition(&mut op,State::RemoteConfirmed)?;}
                store.save_metadata(&info)?;
            }
            Intent::RecordCreate {..} | Intent::RecordUpdate {..} => {
                crate::structured::recover_record(store,notion,&mut op,&target).await?;
            }
            Intent::Comment { .. } => {
                let comment = notion.comment(&target).await?;
                if op.state != State::RemoteConfirmed { store.transition(&mut op, State::RemoteConfirmed)?; }
                store.save_comment(&comment)?;
            }
            Intent::Create { markdown, title, .. } => {
                let authored = markdown.clone();
                let title = title.clone();
                let remote = notion.retrieve(&target).await?;
                ensure!(remote.title == title, "created page title changed; manual reconciliation required");
                op.remote_id = Some(target.clone());
                op.remote_body = Some(remote.markdown.clone());
                if op.state != State::RemoteConfirmed { store.transition(&mut op, State::RemoteConfirmed)?; }
                if store.contains(&target)? {
                    let current = store.get(&target)?;
                    ensure!(current.baseline == remote.markdown, "local baseline differs; reconcile explicitly");
                } else {
                    let mut doc = Document::from_remote(remote);
                    doc.markdown = authored;
                    store.save(&doc, None)?;
                }
            }
            Intent::Push { document } | Intent::Edit {document,..} => {
                let document = document.clone();
                let submitted = match &op.intent {Intent::Edit {selected,..}=>selected.clone(),_=>document.markdown.clone()};
                let remote = notion.retrieve(&target).await?;
                let expected = op.remote_body.as_deref().unwrap_or(&submitted);
                ensure!(remote.markdown == expected, "remote body does not match the recorded outcome; normalization or later edits require explicit reconciliation");
                let current = store.get(&target)?;
                ensure!(current.baseline == document.baseline || current.baseline == remote.markdown, "baseline changed since this operation; reconcile explicitly");
                op.remote_body = Some(remote.markdown.clone());
                if op.state != State::RemoteConfirmed { store.transition(&mut op, State::RemoteConfirmed)?; }
                let mut next = current.clone();
                if matches!(&op.intent,Intent::Edit {..}) && current.markdown == document.markdown {
                    next.markdown=submitted;
                }
                next.baseline = remote.markdown;
                next.title = remote.title;
                next.url = remote.url;
                next.last_edited_time = remote.last_edited_time;
                store.save(&next, Some(&current))?;
            }
            Intent::Rename { title, .. } => {
                let info = notion.inspect(&target).await?;
                ensure!(info.title == *title, "current title does not match the requested title; no write repeated");
                if op.state != State::RemoteConfirmed { store.transition(&mut op, State::RemoteConfirmed)?; }
                store.save_metadata(&info)?;
            }
            Intent::Move { parent, .. } => {
                let info = notion.inspect(&target).await?;
                ensure!(info.parent.as_ref().and_then(|v|v["page_id"].as_str()).and_then(|id|crate::document::page_id(id).ok()).as_deref() == Some(parent), "current parent does not match; no move repeated");
                if op.state != State::RemoteConfirmed { store.transition(&mut op, State::RemoteConfirmed)?; }
                store.save_metadata(&info)?;
            }
        }
        store.transition(&mut op, State::LocallySaved)
    }.await;
    store.operation_result(&mut op, result)?;
    Ok(op)
}
