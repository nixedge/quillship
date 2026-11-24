# Tasks

## Tolerant pull repair

- [x] T009 Update spec/plan and add backwards-compatible completeness metadata.
- [x] T010 Implement tolerant pull reads, preservation rules and strict push guard.
- [x] T011 Continue batches after per-page errors and display diagnostics/counts.
- [x] T012 Cover partial reads, upgrades, persistence, push protection and continuation.
- [x] T013 Run Nix formatting/Clippy/tests and rerun the authorized live pull.

Repair validation: all 22 integration tests, formatting, Clippy with warnings
denied, and `nix build --no-link` passed. The authorized live pull completed with
16 saved pages, four containing Notion placeholders, and zero failures. Local
status confirmed the completeness/warning fields persisted. No Notion writes
were performed. The four partial pages include custom emoji or a button; exact
placeholders remain available in their local Markdown files.

## Initial implementation

- [x] T001 Bootstrap Spec Kit constitution, Rust project, Nix shell and package.
- [x] T002 [US1] Implement document model and durable sled/Markdown workspace.
- [x] T003 [US1] Implement Notion page discovery and complete Markdown retrieval.
- [x] T004 [US1] Implement pull, list, view and search CLI commands.
- [x] T005 [US2] Implement editor, file/stdin edits and external-file ingestion.
- [x] T006 [US3] Implement preflight push, dry-run, conflict preservation and status.
- [x] T007 Cover persistence, editing and Notion sync boundaries with tests.
- [x] T008 Document quickstart and limitations; run Nix formatting/Clippy/tests.

## Validation — 2026-11-24

- `nix develop -c cargo test`: all 17 integration tests passed.
- `nix develop -c cargo fmt --check`: passed.
- `nix develop -c cargo clippy --all-targets -- -D warnings`: passed.
- `nix develop -c cargo run -- --help`: passed.
- `nix build --no-link`: release package built; packaged tests passed.
- `git diff --cached --check`: passed after trimming trailing blank lines from
  two bundled Spec Kit scripts.

Live Notion acceptance has not been run. Automated tests use a mock HTTP server;
no real Notion pages were read or changed. See README for normalization and
remote preflight race limitations.
