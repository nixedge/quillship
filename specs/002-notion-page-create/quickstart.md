# Create a hub and child page

Load NOTION_TOKEN through the existing secrets environment; use direnv for
secret-requiring commands. Do not copy tokens or private environment files
between worktrees. Use --workspace with the existing absolute .quillship path
when running from another directory.

```sh
cargo run -- --workspace /path/to/.quillship create --workspace-root \
  --title 'Budget proposal' --file hub.md --json
cargo run -- --workspace /path/to/.quillship create --parent HUB_UUID \
  --title 'Funding' --file funding.md --json
cat page.md | cargo run -- create --parent HUB_UUID --title 'Roadmap' --file -
```

These are live authoring commands. Review the source before invoking them.
Do not rerun to test whether the first create succeeded. On uncertain outcomes,
inspect Notion; when a UUID is reported, pull that UUID to recover.

Automated validation:

```sh
nix develop -c cargo test
nix develop -c cargo fmt --check
nix develop -c cargo clippy --all-targets -- -D warnings
```

The mock-server suite tests both parent modes, empty/Unicode content,
normalization persistence, partial readback, non-retried failures, malformed
responses, post-create readback errors, local identity collisions and CLI input
validation. Existing sync tests remain passing.
