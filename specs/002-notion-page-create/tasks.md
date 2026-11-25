# Tasks: create Notion pages
Dependencies: spec.md and plan.md reviewed before implementation.

- [x] T001 Inspect all worktree/branch/spec numbers, allocate 002 and verify sibling path.
- [x] T002 Record specification, plan, requirements checklist and curl audit.
- [x] T003 [US1/US2] Implement parent/input validation and create contract in src/notion.rs.
- [x] T004 [US1/US3] Implement local persistence and identity-aware recovery in src/commands.rs.
- [x] T005 [US1/US2] Add create CLI file/stdin, parent modes and JSON output in src/main.rs.
- [x] T006 [US1/US2/US3] Add meaningful mock-server and argument tests in tests/workflow.rs.
- [x] T007 Update README.md and constitution scope for explicit create.
- [x] T008 Run Nix tests, formatting check and strict Clippy; inspect final diff.

Validation: 30 mock/offline tests passed, including eight creation/input/recovery
cases; cargo fmt --check and strict all-target Clippy passed through Nix.
No live creation was performed as automated validation. Implementation is complete
in the verified sibling worktree.
