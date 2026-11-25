# Quillship Constitution

## Principles

1. Implement the maintained CLI and domain logic in Rust. Use Nix for development,
   builds and missing tools; never depend on globally installed tooling.
2. Follow security-council's small command/API/database module architecture.
   Sled is the durable local document store. Expose UTF-8 Markdown working files
   and scriptable CLI commands so humans and LLMs can edit offline.
3. Preserve user work. Keep a remote baseline, never overwrite divergent edits
   during pull, and check remote content before push. Never silently accept an
   incomplete remote document or advance a baseline after a failed write.
4. Credentials come from the environment and never enter document files, logs,
   source control or stored records. Offline commands require no credentials.
5. Write a specification, implementation plan and tasks before feature code.
   Verify persistence, sync conflicts and the HTTP contract with automated tests.
   Run formatting, Clippy and tests through the Nix development environment.

## Scope and workflow

Prefer a small library plus CLI over extra services. Explicit pull, push and create
are the only network operations. Existing body changes require an explicit push;
new pages require an explicit create command. Creation stores the authored body
and returned baseline, and reports a successful remote identity before fallible
readback/persistence. Never repeat an uncertain creation automatically.
Do not execute live writes as tests. Report failures and ask before changing the
agreed approach. Update task completion and document limitations honestly.

## Governance

Amend this constitution deliberately with the corresponding specification and
validation changes. User instructions take precedence.

Version: 1.1.0 | Ratified: 2026-11-24 | Last amended: 2026-11-24
