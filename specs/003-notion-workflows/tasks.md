# Tasks: Notion document workflows

Execute in order. Later tasks remain open until implemented and tested.

## Planning
- [x] T001 Inspect branches/worktrees/specs, allocate 003 and verify sibling host path.
- [x] T002 Read existing architecture/constitution and primary API contracts.
- [x] T003 Specify all seven user stories and all nine requested capabilities.
- [x] T004 Write overall plan, command contracts, domain/storage designs and acceptance checklists.

## Initial review and organisation tranche
- [x] T005 [US1/US2] Accept UUIDs and recognised Notion page links in src/document.rs.
- [x] T006 [US1] Implement exact snapshot-based unified diff in src/review.rs.
- [x] T007 [US1] Persist/list/read archived reconciliation snapshots in src/db.rs.
- [x] T008 [US1] Implement guarded baseline/local/remote/merged reconciliation in src/commands.rs.
- [x] T009 [US2] Add typed metadata inspection and metadata-preserving refresh.
- [x] T010 [US2] Add rename and page-to-page move API/CLI operations.
- [x] T011 [US2] Add bounded/paginated page trees and recursive pull.
- [x] T012 [US1/US2] Expose CLI/JSON interfaces and document offline/live behaviour.
- [x] T013 [US1/US2] Add preservation, HTTP-contract and CLI acceptance tests.
- [x] T014 Run Nix tests, fmt and strict Clippy; inspect initial tranche diff.

## Operation recovery
- [x] T015 [US3] Add versioned journal models/tree, durable input records and state transitions.
- [x] T016 [US3] Integrate create/push/rename/move and per-item batch results.
- [x] T017 [US3] Add journal inspection and read-only recovery/reconciliation commands.
- [x] T018 [US3] Test crash points, uncertain outcomes, restart and duplicate-prevention messaging.

## Review comments
- [x] T019 [US4] Recheck comment contracts and add paginated retrieval with separate cached snapshots.
- [x] T020 [US4] Add explicit page/block comments and discussion replies, journalled writes.
- [x] T021 [US4] Test pagination, unresolved-only completeness, permissions and restart/offline access.

## Targeted editing
- [x] T022 [US5] Recheck Markdown v2 insertion/update contracts and matching semantics.
- [x] T023 [US5] Implement guarded append/replace, journalled input and normalized readback.
- [x] T024 [US5] Test ambiguous/absent matches, child-page preservation and uncertain-write recovery.

## Structured records
- [x] T025 [US6] Add database/data-source schema retrieval and separate persistence.
- [x] T026 [US6] Add typed paginated queries and paginated property-item reads.
- [x] T027 [US6] Add schema-validated record creation, selected property updates and data-source moves.
- [x] T028 [US6] Test unsupported/read-only properties, explicit clearing, limits and concurrent change preservation.

## Attachments
- [x] T029 [US7] Add file/block/property discovery and stable identity metadata.
- [x] T030 [US7] Add bounded download/refresh with no token forwarding and safe local paths.
- [x] T031 [US7] Add journalled upload completion and explicit attachment.
- [x] T032 [US7] Test expired URLs, redirects, partial downloads, multipart failures and restart recovery.

## Completion
- [x] T033 Update README/quickstart, data model and constitution for all completed workstreams.
- [x] T034 Run full Nix validation and generic-spec checks for all completed workstreams.

Initial validation: 46 tests passed (30 existing, 16 added), cargo fmt --check and
strict all-target Clippy passed through nix develop. git diff --check and generic
spec-content checks passed. Mock servers and temporary workspaces only; no live
writes. Recovery validation: 53 tests pass, including injected journal flush
failures preventing submission, uncertain creation without replay, interrupted
batch inputs and restart recovery. Comments validation: 60 tests pass, including
pagination failure/cache preservation, nested blocks, all posting targets,
capabilities, partial creation responses and observation-only recovery.
Formatting and strict Clippy pass. Targeted edits: nine focused tests cover
guarded append/replace, readback changes, normalization, concurrent edits and
recovery. The user approved bounded lock contention retry after intermittent
sled reopen failures; storage-lifecycle.md records the narrow implementation.
Full suite: 71 tests pass, including real reopen and exclusive-lock checks.
Structured records: 83 tests pass, with fmt and strict all-target Clippy. Coverage
includes multiple sources, schema drift, property pagination/cache preservation,
explicit clearing, selected-column conflicts, unrelated changes, body preservation,
capped/limited queries, journalled record creation/update/recovery and source moves.
Final attachment validation: 94 tests pass through nix develop; formatting,
strict all-target Clippy, git diff --check and generic-spec checks pass. Coverage
includes nested media/covers/properties, unsigned download transport, expired link
refresh, redirects, byte limits, path collisions, truncated streams, input tamper
detection, single/multipart uploads, retained parts, restart without recreation,
and attachment readback recovery without reappend. No live Notion writes.
All planned tasks complete; supported scope and API recovery limitations are in
README and workstreams. Each workstream is committed separately.
