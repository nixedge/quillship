use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{ensure, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;

use quillship::{
    commands,
    db::Store,
    document::{page_id, Document},
    notion::{validate_create, CreateParent, Notion},
    review::unified_diff,
};

#[derive(Clone, Copy, ValueEnum)]
enum Take {
    Baseline,
    Local,
    Remote,
}

#[derive(Clone, Copy, ValueEnum)]
enum Side {
    Local,
    Baseline,
    Remote,
    Selected,
}

#[derive(Clone, Copy, ValueEnum)]
enum InsertPosition {
    Start,
    End,
}

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
    /// Insert Markdown at the start/end of a clean synced page
    Append {
        #[arg(value_parser = page_id)]
        id: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = InsertPosition::End)]
        position: InsertPosition,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// Replace one exact match, or explicitly all matches, on a clean synced page
    Replace {
        #[arg(value_parser = page_id)]
        id: String,
        #[arg(long)]
        old_file: PathBuf,
        #[arg(long)]
        new_file: PathBuf,
        #[arg(long)]
        all_matches: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// Read cached discussions or explicitly post a comment/reply
    Comments {
        #[command(subcommand)]
        command: CommentAction,
    },
    /// List durable mutation records offline
    Operations {
        #[arg(long)]
        pending: bool,
        #[arg(long)]
        json: bool,
    },
    /// Inspect exact input and outcome of one mutation offline
    Operation {
        id: u64,
        #[arg(long)]
        json: bool,
    },
    /// Observe an interrupted mutation without resubmitting it
    Recover {
        id: u64,
        #[arg(long)]
        json: bool,
    },
    /// Inspect remote metadata, or read its cached snapshot offline
    Inspect {
        #[arg(value_parser = page_id)]
        id: String,
        #[arg(long)]
        offline: bool,
        #[arg(long)]
        json: bool,
    },
    /// Rename a page without changing its body
    Rename {
        #[arg(value_parser = page_id)]
        id: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        json: bool,
    },
    /// Move a page under another existing page
    Move {
        #[arg(value_parser = page_id)]
        id: String,
        #[arg(long, value_parser = page_id)]
        parent: String,
        #[arg(long)]
        json: bool,
    },
    /// Discover a bounded tree of actual child pages
    Tree {
        #[arg(value_parser = page_id)]
        id: String,
        #[arg(long, default_value_t = 8)]
        max_depth: usize,
        #[arg(long, default_value_t = 10000)]
        max_blocks: usize,
        #[arg(long)]
        json: bool,
    },
    /// Compare exact local and baseline bodies, optionally with fresh remote content
    Diff {
        #[arg(value_parser = page_id)]
        id: String,
        #[arg(long)]
        remote: bool,
        #[arg(long)]
        json: bool,
    },
    /// Back up inputs and resolve a local conflict; never writes to Notion
    Reconcile {
        #[arg(value_parser = page_id)]
        id: String,
        #[arg(
            long,
            value_enum,
            required_unless_present = "file",
            conflicts_with = "file"
        )]
        take: Option<Take>,
        #[arg(long, required_unless_present = "take", conflicts_with = "take")]
        file: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// List durable recovery snapshots for a page (offline)
    Snapshots {
        #[arg(value_parser = page_id)]
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Print an exact archived body (offline)
    Snapshot {
        id: u64,
        #[arg(long, value_enum)]
        side: Side,
    },
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
        /// Traverse child pages before pulling; requires one root page
        #[arg(long, conflicts_with = "query")]
        recursive: bool,
        #[arg(long, default_value_t = 8, requires = "recursive")]
        max_depth: usize,
        #[arg(long, default_value_t = 10000, requires = "recursive")]
        max_blocks: usize,
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

#[derive(Subcommand)]
enum CommentAction {
    /// Fetch unresolved comments, retaining prior cache on any failure
    Pull {
        #[arg(value_parser = page_id)]
        target: String,
        #[arg(long)]
        recursive: bool,
        #[arg(long, default_value_t = 10000)]
        max_blocks: usize,
        #[arg(long)]
        json: bool,
    },
    /// Read the last complete cached unresolved-comment snapshot offline
    List {
        #[arg(value_parser = page_id)]
        target: String,
        #[arg(long)]
        recursive: bool,
        #[arg(long)]
        json: bool,
    },
    /// Explicitly post inline Markdown to one page, block or discussion
    Add {
        #[arg(long, value_parser = page_id, conflicts_with_all = ["block", "discussion"], required_unless_present_any = ["block", "discussion"])]
        page: Option<String>,
        #[arg(long, value_parser = page_id, conflicts_with_all = ["page", "discussion"], required_unless_present_any = ["page", "discussion"])]
        block: Option<String>,
        #[arg(long, value_parser = page_id, conflicts_with_all = ["page", "block"], required_unless_present_any = ["page", "block"])]
        discussion: Option<String>,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        json: bool,
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

fn print_info(info: &quillship::notion::PageInfo, as_json: bool) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(info)?);
    } else {
        println!("{}\t{}\t{}", info.id, info.title, info.url);
        println!("parent: {}", serde_json::to_string(&info.parent)?);
        println!(
            "edited: {}; archived: {}; in trash: {}",
            info.last_edited_time, info.archived, info.in_trash
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = Store::open(&cli.workspace)?;
    match cli.command {
        Action::Append {
            id,
            file,
            position,
            dry_run,
            json: _,
        } => {
            use quillship::targeted::{self, Edit, Position};
            let edit = Edit::Append {
                content: commands::read_markdown(&file)?,
                position: match position {
                    InsertPosition::Start => Position::Start,
                    InsertPosition::End => Position::End,
                },
            };
            let notion = if dry_run {
                None
            } else {
                Some(Notion::from_env()?)
            };
            let outcome = targeted::run(&store, notion.as_ref(), &id, edit, dry_run).await?;
            println!("{}", serde_json::to_string_pretty(&outcome)?);
            Ok(())
        }
        Action::Replace {
            id,
            old_file,
            new_file,
            all_matches,
            dry_run,
            json: _,
        } => {
            ensure!(
                old_file.as_path() != std::path::Path::new("-")
                    || new_file.as_path() != std::path::Path::new("-"),
                "only one input may read stdin"
            );
            let edit = quillship::targeted::Edit::Replace {
                old: commands::read_markdown(&old_file)?,
                new: commands::read_markdown(&new_file)?,
                all_matches,
            };
            let notion = if dry_run {
                None
            } else {
                Some(Notion::from_env()?)
            };
            let outcome =
                quillship::targeted::run(&store, notion.as_ref(), &id, edit, dry_run).await?;
            println!("{}", serde_json::to_string_pretty(&outcome)?);
            Ok(())
        }
        Action::Comments { command } => {
            use quillship::comments;
            let value = match command {
                CommentAction::Pull {
                    target,
                    recursive,
                    max_blocks,
                    json: _,
                } => serde_json::to_value(
                    comments::pull(&store, &Notion::from_env()?, &target, recursive, max_blocks)
                        .await?,
                )?,
                CommentAction::List {
                    target,
                    recursive,
                    json: _,
                } => serde_json::to_value(store.comments(&target, recursive)?)?,
                CommentAction::Add {
                    page,
                    block,
                    discussion,
                    file,
                    json: _,
                } => {
                    let target = if let Some(id) = page {
                        comments::Target::Page(id)
                    } else if let Some(id) = block {
                        comments::Target::Block(id)
                    } else {
                        comments::Target::Discussion(discussion.expect("clap requires a target"))
                    };
                    let markdown = commands::read_markdown(&file)?;
                    comments::validate(&markdown)?;
                    comments::add(&store, &Notion::from_env()?, target, &markdown).await?
                }
            };
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        Action::Operations {
            pending,
            json: as_json,
        } => {
            let ops = store
                .operations()?
                .into_iter()
                .filter(|op| !pending || op.pending())
                .collect::<Vec<_>>();
            if as_json {
                println!("{}", serde_json::to_string_pretty(&ops)?);
            } else {
                for op in ops {
                    println!(
                        "{}\t{:?}\t{}\tbatch {:?}",
                        op.id,
                        op.state,
                        op.target().unwrap_or("identity unknown"),
                        op.batch_id
                    );
                }
            }
            Ok(())
        }
        Action::Operation { id, json: _ } => {
            println!("{}", serde_json::to_string_pretty(&store.operation(id)?)?);
            Ok(())
        }
        Action::Recover { id, json: _ } => {
            let op = quillship::journal::recover(&store, &Notion::from_env()?, id).await?;
            println!("{}", serde_json::to_string_pretty(&op)?);
            Ok(())
        }
        Action::Inspect {
            id,
            offline,
            json: as_json,
        } => {
            let info = if offline {
                store.metadata(&id)?
            } else {
                let info = Notion::from_env()?.inspect(&id).await?;
                store.save_metadata(&info)?;
                info
            };
            print_info(&info, as_json)
        }
        Action::Rename {
            id,
            title,
            json: as_json,
        } => {
            validate_create(&CreateParent::Page(id.clone()), &title, "")?;
            let info = commands::rename_page(&store, &Notion::from_env()?, &id, &title).await?;
            print_info(&info, as_json)
        }
        Action::Move {
            id,
            parent,
            json: as_json,
        } => {
            ensure!(id != parent, "a page cannot be its own parent");
            let info = commands::move_page(&store, &Notion::from_env()?, &id, &parent).await?;
            print_info(&info, as_json)
        }
        Action::Tree {
            id,
            max_depth,
            max_blocks,
            json: as_json,
        } => {
            ensure!(max_blocks > 0, "max-blocks must be positive");
            let report = Notion::from_env()?.tree(&id, max_depth, max_blocks).await?;
            if as_json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                for page in &report.pages {
                    println!("{}{}\t{}", "  ".repeat(page.depth), page.id, page.title);
                }
                for (id, error) in &report.failures {
                    eprintln!("failed {id}: {error}");
                }
            }
            ensure!(
                !report.truncated && report.failures.is_empty(),
                "tree incomplete: truncated={}, {} failures",
                report.truncated,
                report.failures.len()
            );
            Ok(())
        }
        Action::Diff {
            id,
            remote,
            json: as_json,
        } => {
            let doc = store.get(&id)?;
            let remote = if remote {
                Some(Notion::from_env()?.retrieve(&id).await?)
            } else {
                None
            };
            let local_diff = unified_diff(&doc.baseline, &doc.markdown, "baseline", "local");
            let remote_diff = remote
                .as_ref()
                .map(|r| unified_diff(&doc.baseline, &r.markdown, "baseline", "remote"));
            if as_json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "id": id, "local": doc.markdown, "baseline": doc.baseline,
                        "incomplete": doc.incomplete, "sync_warnings": doc.sync_warnings,
                        "remote": remote, "local_diff": local_diff, "remote_diff": remote_diff
                    }))?
                );
            } else {
                for warning in &doc.sync_warnings {
                    eprintln!("warning: {warning}");
                }
                print!("{local_diff}");
                if let Some(diff) = remote_diff {
                    print!("{diff}");
                }
            }
            Ok(())
        }
        Action::Reconcile {
            id,
            take,
            file,
            json: as_json,
        } => {
            let choice = if let Some(file) = file {
                commands::Reconcile::Merged(commands::read_markdown(&file)?)
            } else {
                match take.expect("clap requires take or file") {
                    Take::Baseline => commands::Reconcile::Baseline,
                    Take::Local => commands::Reconcile::Local,
                    Take::Remote => commands::Reconcile::Remote,
                }
            };
            let notion = if matches!(choice, commands::Reconcile::Baseline) {
                None
            } else {
                Some(Notion::from_env()?)
            };
            let (doc, snapshot) = commands::reconcile(&store, notion.as_ref(), &id, choice).await?;
            if as_json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"id": id, "snapshot_id": snapshot.id, "dirty": doc.dirty()})
                    )?
                );
            } else {
                println!(
                    "reconciled {id}; recovery snapshot {}; {}",
                    snapshot.id,
                    if doc.dirty() {
                        "modified; push to publish"
                    } else {
                        "clean"
                    }
                );
            }
            Ok(())
        }
        Action::Snapshots { id, json: as_json } => {
            let snapshots = store.snapshots(&id)?;
            if as_json {
                println!("{}", serde_json::to_string_pretty(&snapshots)?);
            } else {
                for s in snapshots {
                    println!("{}\t{}\t{}", s.id, s.local.id, s.action);
                }
            }
            Ok(())
        }
        Action::Snapshot { id, side } => {
            let s = store.snapshot(id)?;
            let body = match side {
                Side::Local => &s.local.markdown,
                Side::Baseline => &s.local.baseline,
                Side::Remote => {
                    &s.remote
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("snapshot has no remote body"))?
                        .markdown
                }
                Side::Selected => &s.selected_markdown,
            };
            io::stdout().lock().write_all(body.as_bytes())?;
            Ok(())
        }
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
        Action::Pull {
            mut ids,
            query,
            recursive,
            max_depth,
            max_blocks,
        } => {
            ensure!(
                !recursive || ids.len() == 1,
                "recursive pull requires exactly one root page"
            );
            ensure!(!recursive || max_blocks > 0, "max-blocks must be positive");
            let notion = Notion::from_env()?;
            let mut traversal_complete = true;
            if recursive {
                let tree = notion.tree(&ids[0], max_depth, max_blocks).await?;
                for (id, error) in &tree.failures {
                    eprintln!("tree failed {id}: {error}");
                }
                traversal_complete = !tree.truncated && tree.failures.is_empty();
                if tree.truncated {
                    eprintln!("warning: tree traversal limit reached; only discovered pages will be pulled");
                }
                ids = tree.pages.into_iter().map(|p| p.id).collect();
            }
            let report = commands::pull(&store, &notion, &ids, query.as_deref()).await?;
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
            ensure!(
                traversal_complete,
                "recursive pull saved discovered pages, but traversal was incomplete"
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
            commands::push_batch(&store, &notion, &pending).await
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
