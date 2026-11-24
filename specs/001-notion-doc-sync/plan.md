# Implementation plan

## Technical context

Rust 2021 library and Clap CLI; Tokio/Reqwest with rustls; serde JSON records;
sled 0.34 durable local storage; tempfile atomic file replacement and shlex editor
arguments. Nix flake provides Cargo, rustc, rustfmt, Clippy and a package/check.
No background daemon or additional database. All content stays workspace-local.

## Design

- `document.rs`: typed local/remote records and pure reconciliation decisions.
- `db.rs`: sled persistence and stable Markdown files, ingest external file edits.
- `notion.rs`: authenticated versioned HTTP, paginated page search, metadata and
  Markdown retrieval/replacement, bounded 429 retries and request timeouts.
- `commands.rs`: pull/push orchestration, preflight, editor/file/stdin input.
- `main.rs`: CLI parsing and plain/JSON rendering; no implicit remote operation.
- `tests/`: real temporary sled workspaces and mocked Notion HTTP contract tests.

Sled holds the current body, last remote Markdown baseline and completeness flags.
Working files are the edit interface and get ingested before CLI operations.
Write files atomically before committing their record; a failure cannot erase an
existing external edit. A failed DB commit can leave a newer file, ingested later.
Pull accepts remote changes only when local is clean, remote is unchanged, or
both sides agree. Push compares fresh remote Markdown to the baseline and
uses synchronous replacement; preserve submitted content if Notion normalizes it.
Never retry ambiguous transport/server failures on writes.

## Research decisions

Security-council's `src/{main,commands,db,clickup}.rs` provides the module pattern,
stored baseline design, offline search and editor workflow. Do not copy its
ClickUp fields or private content.

Notion API 2026-03-11 supports direct Markdown reads and writes. Explicitly use
`markdown_version: v2`; PATCH uses `type: replace_content` and
`replace_content.new_str`. Pull stores incomplete Markdown with diagnostics;
push still rejects incomplete responses and unknown placeholders.
Keep child-page deletion protection enabled. No bespoke block-to-Markdown codec.

Primary references (consulted 2026-11-24):
- https://developers.notion.com/reference/retrieve-page-markdown
- https://developers.notion.com/reference/update-page-markdown
- https://developers.notion.com/reference/post-search

## Constitution check

Pass: Rust, Nix, sled, simple modules, explicit sync, loss prevention, environment
credentials, spec before implementation and meaningful integration validation.
No exceptions. Rechecked after design: pass.

## Validation

Run formatting, Clippy with warnings denied, and Cargo tests in `nix develop`.
Use mocked HTTP only. Test process restarts, direct file edits, UUID validation,
pagination, auth/version/payload contracts, incomplete responses, conflicts,
idempotent clean sync and unsuccessful pushes. Smoke-test CLI without secrets.

## Repair plan — tolerant pull

Keep the existing modules and initial spec. Add serde-defaulted `incomplete` and
`sync_warnings` fields to stored documents; remote snapshots carry warnings.
Decode missing completeness fields conservatively as warnings. Keep strict
retrieval for push and expose tolerant retrieval for pull. Preserve complete
snapshots when a new partial response arrives, including across repeated pulls.
New partial documents retain exact Markdown and may be edited locally but never
pushed. Return a batch report with per-page failures instead of aborting early.
The CLI reports counts and exits unsuccessfully only after all attempted pages
when actual errors occurred. Surface warnings in view/list/status/search output.

Validate existing-record compatibility, partial document persistence/editing,
strict push protection, complete-to-partial preservation, partial-to-complete
promotion, per-page failure continuation and the actual previously blocked pull.
The live rerun is read-only in Notion and remains authorized by the user's pull.
