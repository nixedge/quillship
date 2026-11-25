use crate::{
    db::Store,
    journal::{Intent, State},
    notion::Notion,
};
use anyhow::{ensure, Context, Result};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    Start,
    End,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Edit {
    Append {
        content: String,
        position: Position,
    },
    Replace {
        old: String,
        new: String,
        all_matches: bool,
    },
}

impl Edit {
    pub fn apply(&self, body: &str) -> Result<String> {
        let result = match self {
            Self::Append { content, position } => {
                ensure!(
                    !content.trim().is_empty(),
                    "inserted content must not be empty"
                );
                ensure!(
                    !content.contains("<unknown"),
                    "cannot insert unknown placeholders"
                );
                if body.is_empty() {
                    content.clone()
                } else {
                    match position {
                        Position::Start => format!("{content}\n\n{body}"),
                        Position::End => format!("{body}\n\n{content}"),
                    }
                }
            }
            Self::Replace {
                old,
                new,
                all_matches,
            } => {
                ensure!(!old.is_empty(), "replacement search must not be empty");
                ensure!(
                    !new.contains("<unknown"),
                    "cannot insert unknown placeholders"
                );
                let count = body.match_indices(old).count();
                ensure!(count > 0, "search content was not found");
                ensure!(
                    count == 1 || *all_matches,
                    "search matches {count} locations; use --all-matches explicitly"
                );
                if *all_matches {
                    body.replace(old, new)
                } else {
                    body.replacen(old, new, 1)
                }
            }
        };
        ensure!(result != body, "edit makes no change");
        Ok(result)
    }

    fn request(&self) -> Value {
        match self {
            Self::Append { content, position } => {
                json!({"type":"insert_content","markdown_version":"v2","allow_async":false,"insert_content":{"content":content,"position":{"type":position}}})
            }
            Self::Replace {
                old,
                new,
                all_matches,
            } => {
                json!({"type":"update_content","markdown_version":"v2","allow_async":false,"update_content":{"content_updates":[{"old_str":old,"new_str":new,"replace_all_matches":all_matches}],"allow_deleting_content":false}})
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub id: String,
    pub preview: String,
    pub baseline: String,
    pub dirty: bool,
    pub operation_id: Option<u64>,
    pub remote_checked: bool,
}

/// Offline preview or explicit guarded remote mutation. Never mixes dirty input.
pub async fn run(
    store: &Store,
    notion: Option<&Notion>,
    id: &str,
    edit: Edit,
    dry_run: bool,
) -> Result<Outcome> {
    let previous = store.get(id)?;
    ensure!(
        !previous.dirty(),
        "local body is modified; reconcile/push it before targeted remote editing"
    );
    ensure!(
        !previous.incomplete && previous.sync_warnings.is_empty(),
        "complete baseline required for targeted editing"
    );
    let selected = edit.apply(&previous.markdown)?;
    if dry_run {
        return Ok(Outcome {
            id: previous.id,
            dirty: selected != previous.baseline,
            preview: selected,
            baseline: previous.baseline,
            operation_id: None,
            remote_checked: false,
        });
    }
    let notion = notion.context("targeted writes require a Notion client")?;
    let remote = notion.retrieve(&previous.id).await?;
    ensure!(
        remote.markdown == previous.baseline,
        "remote body changed; pull/reconcile before targeted editing"
    );
    ensure!(
        store.get(&previous.id)?.markdown == previous.markdown,
        "local file changed during preflight; no write submitted"
    );
    let mut op = store.prepare(
        Intent::Edit {
            document: previous.clone(),
            edit: edit.clone(),
            selected: selected.clone(),
        },
        None,
    )?;
    let result=async {
        store.transition(&mut op,State::Sent)?;
        let canonical=notion.targeted(&previous.id,&edit).await?;
        op.remote_id=Some(previous.id.clone());op.remote_body=Some(canonical.clone());
        store.transition(&mut op,State::RemoteConfirmed)?;
        let after=notion.retrieve(&previous.id).await?;
        ensure!(after.markdown==canonical,"remote content changed after targeted write; recorded result retained; reconcile explicitly");
        let current=store.get(&previous.id)?;
        let mut next=current.clone();
        if current.markdown==previous.markdown {next.markdown=selected.clone();}
        next.baseline=canonical;next.title=after.title;next.url=after.url;next.last_edited_time=after.last_edited_time;
        store.save(&next,Some(&current))?;
        store.transition(&mut op,State::LocallySaved)?;
        Ok(Outcome {id:next.id.clone(),dirty:next.dirty(),preview:next.markdown,baseline:next.baseline,operation_id:Some(op.id),remote_checked:true})
    }.await;
    store.operation_result(&mut op, result)
}

impl Notion {
    pub async fn targeted(&self, id: &str, edit: &Edit) -> Result<String> {
        let id = crate::document::page_id(id)?;
        self.complete_markdown(
            self.request(
                Method::PATCH,
                &format!("/pages/{id}/markdown"),
                Some(edit.request()),
            )
            .await?,
        )
    }
}
