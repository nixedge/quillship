# Quillship

Use the installed `speckit` skill for new features. Read
`.specify/memory/constitution.md` and the relevant `specs/` artifacts first.

Rust CLI, separate command, Notion client, document and sled modules, modeled
after `/home/sam/work/intersect/security-council`. Markdown files are working
copies; sled stores documents and remote baselines. Preserve local changes.

Use `nix develop -c cargo test`, `nix develop -c cargo fmt --check`, and
`nix develop -c cargo clippy --all-targets -- -D warnings`.
Use `direnv exec .` for commands requiring project secrets. Never read or print
`.envrc.local` or tokens. No live Notion writes during automated validation.

The repository started with unborn `master`; initial spec 001 is bootstrapped
here without a synthetic base commit. Once a base commit exists, allocate new
numbered features in sibling worktrees after inspecting all numbers.
