# Plan: create Notion pages
Branch: 002-notion-page-create | Date: 2026-11-24

## Design
Extend the existing Rust CLI, commands and Notion modules; no new dependency or
storage schema. CreateParent enum validates parent UUIDs before requests.
Notion::create returns a validated CreatedPage identity from POST /pages;
commands::create_document prints identity to stderr immediately after success,
retrieves through tolerant pull and saves a new Document. Keep the submitted body
as markdown and returned Markdown as baseline. Never retry POST after ambiguous
errors; the existing request function only retries explicit 429.
CLI requires a title, file (or -), and exactly one parent mode. It reads and
validates input before constructing the client. JSON output supports scripting.

## HTTP contract
POST /v1/pages with parent {workspace:true} or {page_id:UUID},
properties.title.title text, markdown_version v2, markdown BODY,
allow_async false, notifications.mode silent. Validate page response object, UUID,
URL before subsequent reads; retrieve validates metadata and edit timestamp.
GET metadata/Markdown uses existing retrieval completeness and race checks.

## Constitution
Rust, Nix, small modules, sled and preservation rules all apply. User explicitly
authorizes creation: amend scope to include an explicit create command alongside
pull/push. No automatic live tests. Recheck: no additional exception needed.

## Files
src/notion.rs: parent/created types, validation and create endpoint.
src/commands.rs: shared input reader, create persistence/recovery.
src/main.rs: create command and JSON/text output.
tests/workflow.rs: contracts, local persistence, failure recovery and argument tests.
README.md: usage and limitations.
specs/002-notion-page-create/curl-audit.md: session operation record.

## Validation
Use local mock HTTP and temporary sled stores; run project tests, fmt and
all-target strict Clippy through nix develop. No live acceptance creates extra pages.
