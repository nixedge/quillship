use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{ensure, Result};
use clap::{Parser, Subcommand};
use serde_json::json;

use quillship::{
    commands,
    db::Store,
    document::{page_id, Document},
    notion::{validate_create, CreateParent, Notion},
};

#[derive(Parser)]
#[command(version, about = "Edit and search Notion documents locally")]
struct Cli {
    /// Local workspace containing sled storage and Markdown working files
    #[arg(
        long,
        global = true,
        env = "QUILLSHIP_WORKSPACE",
        default_value = ".quillship"
    )]
    workspace: PathBuf,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Create one Notion page and save its Markdown working copy
    Create {
        #[arg(long)]
        title: String,
        /// Existing parent page UUID
        #[arg(long, value_parser = page_id, conflicts_with = "workspace_root", required_unless_present = "workspace_root")]
        parent: Option<String>,
        /// Create a private workspace-level page (requires an eligible connection)
        #[arg(long, conflicts_with = "parent", required_unless_present = "parent")]
        workspace_root: bool,
        /// UTF-8 Markdown input file; - reads stdin
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Pull page UUIDs, or discover accessible pages (optionally by title)
    Pull {
        #[arg(value_parser = page_id)]
        ids: Vec<String>,
        #[arg(long, conflicts_with = "ids")]
        query: Option<String>,
    },
    /// Push edited bodies after checking fresh remote content against the baseline
    Push {
        #[arg(value_parser = page_id)]
        ids: Vec<String>,
        /// List pending IDs without credentials or network access
        #[arg(long)]
        dry_run: bool,
    },
    /// List locally stored pages
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show the Markdown body without extra headers
    View {
        #[arg(value_parser = page_id)]
        id: String,
        /// Print the last synced body, useful when resolving a conflict
        #[arg(long)]
        baseline: bool,
    },
    /// Edit a local body in $VISUAL/$EDITOR, or replace it from a file/stdin
    Edit {
        #[arg(value_parser = page_id)]
        id: String,
        /// Markdown source path; - reads stdin
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Search local titles and Markdown bodies (case-insensitive substring)
    Search {
        query: String,
        #[arg(long)]
        json: bool,
    },
    /// Show local clean/modified status; ingest any direct file edits
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Print a page's absolute Markdown working-file path
    Path {
        #[arg(value_parser = page_id)]
        id: String,
    },
}

fn list(docs: &[Document], as_json: bool) -> Result<()> {
    if as_json {
        let records = docs
            .iter()
            .map(|doc| {
                json!({
                    "id": doc.id, "title": doc.title, "url": doc.url,
                    "last_edited_time": doc.last_edited_time,
                    "dirty": doc.dirty(),
                    "incomplete": doc.incomplete,
                    "sync_warnings": doc.sync_warnings,
                })
            })
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&records)?);
    } else {
        for doc in docs {
            println!(
                "{}\t{}{}\t{}",
                doc.id,
                if doc.dirty() { "modified" } else { "clean" },
                if doc.incomplete || !doc.sync_warnings.is_empty() {
                    ", partial (push disabled)"
                } else {
                    ""
                },
                doc.title
            );
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = Store::open(&cli.workspace)?;
    match cli.command {
        Action::Create {
            title,
            parent,
            workspace_root: _,
            file,
            json: as_json,
        } => {
            let parent = parent
                .map(CreateParent::Page)
                .unwrap_or(CreateParent::Workspace);
            let markdown = commands::read_markdown(&file)?;
            validate_create(&parent, &title, &markdown)?;
            let doc =
                commands::create_document(&store, &Notion::from_env()?, &parent, &title, &markdown)
                    .await?;
            if as_json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "id": doc.id, "title": doc.title, "url": doc.url,
                        "path": store.path(&doc.id)?, "dirty": doc.dirty(),
                        "incomplete": doc.incomplete, "sync_warnings": doc.sync_warnings
                    }))?
                );
            } else {
                println!(
                    "created {}\t{}\t{}",
                    doc.id,
                    doc.url,
                    store.path(&doc.id)?.display()
                );
            }
            if doc.dirty() {
                eprintln!("Notion normalized the submitted Markdown; exact input retained; local document remains modified");
            }
            if doc.incomplete {
                eprintln!(
                    "warning: {}; pushing disabled",
                    doc.sync_warnings.join("; ")
                );
            }
            Ok(())
        }
        Action::Pull { ids, query } => {
            let report =
                commands::pull(&store, &Notion::from_env()?, &ids, query.as_deref()).await?;
            println!(
                "Pull complete: {} saved, {} with incomplete remote content, {} failed.",
                report.pulled,
                report.incomplete,
                report.failures.len()
            );
            ensure!(
                report.failures.is_empty(),
                "{} page(s) could not be pulled; see failures above",
                report.failures.len()
            );
            Ok(())
        }
        Action::Push { ids, dry_run } => {
            let pending = commands::pending(&store, &ids)?;
            if dry_run {
                for doc in &pending {
                    if doc.incomplete || !doc.sync_warnings.is_empty() {
                        println!("blocked (incomplete) {}\t{}", doc.id, doc.title);
                    } else {
                        println!("would push {}\t{}", doc.id, doc.title);
                    }
                }
                return Ok(());
            }
            if pending.is_empty() {
                println!("No local changes to push.");
                return Ok(());
            }
            let notion = Notion::from_env()?;
            for doc in pending {
                commands::push_document(&store, &notion, &doc).await?;
                println!("pushed {}", doc.id);
            }
            Ok(())
        }
        Action::List { json } | Action::Status { json } => list(&store.all()?, json),
        Action::View { id, baseline } => {
            let doc = store.get(&id)?;
            for warning in &doc.sync_warnings {
                eprintln!("warning {}: {warning}; pushing disabled", doc.id);
            }
            io::stdout().lock().write_all(if baseline {
                doc.baseline.as_bytes()
            } else {
                doc.markdown.as_bytes()
            })?;
            Ok(())
        }
        Action::Path { id } => {
            store.get(&id)?;
            println!("{}", store.path(&id)?.display());
            Ok(())
        }
        Action::Edit { id, file } => match file {
            Some(file) => commands::edit_file(&store, &id, &file),
            None => commands::edit(&store, &id),
        },
        Action::Search { query, json } => {
            ensure!(!query.trim().is_empty(), "search query cannot be empty");
            let query = query.to_lowercase();
            let docs: Vec<_> = store
                .all()?
                .into_iter()
                .filter(|doc| {
                    doc.title.to_lowercase().contains(&query)
                        || doc.markdown.to_lowercase().contains(&query)
                })
                .collect();
            list(&docs, json)
        }
    }
}
