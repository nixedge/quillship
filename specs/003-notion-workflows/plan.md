# Plan: Notion document workflows

Branch: 003-notion-workflows | Date: 2026-11-24 | Input: spec.md

## Delivery order
1. US1/US2: URL inputs, local/remote diff, archived reconciliation and formatting
   acceptance; metadata inspection, rename/move, bounded tree and recursive pull.
2. US3: durable operation journal and per-item batch recovery.
3. US4: cached comments and explicit comments/replies.
4. US5: append/targeted replacements with current-body guards.
5. US6: database/data-source schemas, queries and selected property writes.
6. US7: file discovery/download and tracked upload/attachment.

All workstreams are planned before implementation begins. tasks.md records actual
completion; finishing a tranche does not imply the remaining work is implemented.

## Technical context and architecture
Rust 2021 CLI/library with clap, reqwest, serde, sled and tempfile. No daemon.
Retain commands/API/document/db boundaries; add review and typed domain modules
where growth justifies them. Use tests/workflow.rs and additional focused test
modules. No initial dependency addition: unified diff uses line-oriented
prefix/suffix context; JSON output exposes exact snapshots.

Storage remains one locked workspace at a time. Add separate versioned sled trees
for reconciliation snapshots, operation journal, comments, schemas/properties and
file metadata as those workstreams land. Body records remain backwards compatible.
Network data is not applied until its completeness and identity are established.
Create permanent recovery records before replacing local content or remote writes.

## Initial design
document::page_id accepts UUIDs and recognised Notion page URLs, returning only a
canonical UUID. No URL credentials are propagated.
review.rs renders diffs and snapshot/reconciliation models. db::Store archives
current Document plus optional fresh remote snapshot and chosen action using a
monotonic local ID, flushed before save. Snapshot list/view allow recovery.
reconcile --take baseline uses the saved complete baseline offline; remote/local/
merged choices fetch a complete fresh remote snapshot and retain all original
sides. Recheck the original working file with Store::save after network reads.

notion::PageInfo exposes safe metadata including parent/state. Rename/move use
explicit API calls and return a verified identity. Command refresh preserves body
edits/baselines even if remote content moved; report completed mutation identity
before fallible refresh. Body conflicts are not resolved by metadata operations.
Tree walking uses /blocks/ID/children, traverses nested containers and actual child
pages, and bounds page depth, total blocks and child-list requests (using the
block budget also as the request cap). Cycle/duplicate/cursor checks
are explicit. Recursive pull reuses existing per-page preservation logic and
reports traversal or per-page failures. Depth truncation is visible.

## Detailed designs
workstreams/recovery.md, comments.md, targeted-edits.md, data-sources.md and files.md
specify subsequent contracts, storage, acceptance and ordering. contracts.md
records initial command/API contracts. research.md holds primary API sources.

## Constitution check
Rust/Nix/small modules, authored-body preservation, credentials and mock-only
validation remain unchanged. Amend explicit-network-operation scope to enumerate
the new user-invoked commands and keep domain-specific records separate.
No other exception is required. Recheck each write workstream before implementation.

## Validation and risks
Test URL boundary cases, exact diff content, backup durability and backup failures,
concurrent edits, partial reads, state/metadata preservation, nested/paginated tree
walks, repeated cursors and inaccessible descendants. Follow with journal/comment/
query/file-specific tests. Run cargo test, cargo fmt --check and strict all-target
Clippy via nix develop. No live writes as tests.

Notion lacks atomic content compare-and-swap; preserve preflight checks without
claiming to close the last remote race. Creation is non-idempotent. Metadata and
body baselines must not be conflated. File retrieval needs a separate unauthenticated
client and bounded streaming. Unresolved later API details require rechecking the
official contract before implementation, not a generic JSON escape hatch.
