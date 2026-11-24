# Quickstart and acceptance

1. `nix develop -c cargo test`: run isolated tests with real sled stores and mock HTTP.
2. `nix develop -c cargo fmt --check` and
   `nix develop -c cargo clippy --all-targets -- -D warnings`.
3. Load `NOTION_TOKEN` through your secrets environment/direnv and share an
   existing Notion page with the integration.
4. `direnv exec . cargo run -- pull PAGE_UUID`.
5. `direnv exec . cargo run -- path PAGE_UUID`; edit that Markdown file with an LLM.
6. `direnv exec . cargo run -- search 'edited text' --json` and `status --json`.
7. `direnv exec . cargo run -- push --dry-run` to review pending IDs.
8. When you intend to update Notion, `direnv exec . cargo run -- push PAGE_UUID`.

Live steps are manual acceptance, not part of automated validation. See README
for conflict resolution, normalization, scope and remote race limitations.
