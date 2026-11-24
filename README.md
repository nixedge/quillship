# Quillship

A Rust CLI for pulling Notion pages into a local sled store, searching them
offline, editing Markdown with an editor or LLM, and pushing changes explicitly.
The small command/API/database module structure follows `sc-reports` in the
security-council project.

## Development

```sh
nix develop
cargo build
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Or run each command with `nix develop -c`. `nix build` builds the packaged CLI and
runs tests. `nix run . -- --help` runs the packaged tool. The Nix and Cargo lock
files pin development/build inputs.

## Connect Notion

Create a Notion integration with read and update content capabilities, and share
the pages with it. Set `NOTION_TOKEN` through your normal secrets environment;
the CLI reads it only for pull and nonempty push operations. For direnv, `.envrc`
loads the Nix shell and optionally sources the ignored `.envrc.local`. Keep your
token in that private file or your secret manager. Use `direnv exec .` for
commands requiring those secrets. Never commit a token.

The implementation uses Notion API `2026-03-11` and its native Markdown v2
endpoints. Files contain Notion-flavored Markdown, including Notion extensions,
without Quillship frontmatter. Preserve those extensions when editing.

## Pull, edit, search and push

With the development shell active and your token loaded:

```sh
# Replace PAGE_UUID with the UUID from a Notion page URL.
cargo run -- pull PAGE_UUID

# Discover pages shared with the integration, optionally by title.
cargo run -- pull --query 'Project notes'
cargo run -- pull

cargo run -- list
cargo run -- view PAGE_UUID
cargo run -- search 'meeting notes' --json
cargo run -- edit PAGE_UUID

# Script/LLM workflow: replace the body from a UTF-8 Markdown file or stdin.
cargo run -- edit PAGE_UUID --file edited.md
cat edited.md | cargo run -- edit PAGE_UUID --file -

# Or edit the persistent working file directly.
cargo run -- path PAGE_UUID

cargo run -- status --json
cargo run -- push --dry-run
cargo run -- push PAGE_UUID
```

`edit` uses `$VISUAL`, falling back to `$EDITOR`; quoted editor arguments are
supported, without shell evaluation. A failed editor leaves its file available.
`view` prints the exact body to stdout, suitable for piping or redirecting.
`list`, `search` and `status` support `--json`. Search is a case-insensitive
substring match against title and full body, scanning the local sled records.

The default workspace is `.quillship/` relative to the current directory. Set
`QUILLSHIP_WORKSPACE` or pass `--workspace /path/to/workspace` to use the same
documents from anywhere. `docs/<canonical-page-uuid>.md` is the working copy;
`store/` contains sled records, metadata and remote baselines. Page titles never
become file paths. Back up the **whole workspace**, including sled and Markdown.

Working-file edits are ingested automatically before reads/searches and sync.
Do not delete working files to delete remote pages: missing files cause an error.
One Quillship process opens a workspace at a time; finish external editor saves
before syncing. Paths accept hyphenated or compact UUIDs, rather than full URLs.

## Sync behavior and conflicts

- Pull updates clean documents and preserves dirty local bodies when the remote
  body is unchanged. Identical changes on both sides converge to clean.
- Divergent changes preserve the local file and baseline and report a conflict.
  Pull continues with other pages; push stops at the conflict.
- Push checks fresh complete remote content against the last baseline before
  replacing a body. It never enables child-page/database deletion.
- `push --dry-run` reports pending documents with no token or network calls.
  `push` without IDs pushes all locally modified documents. `status` reports
  **local** modifications; it does not check for remote changes.
- Failed requests retain local edits and baselines. A transport failure or an API
  warning after a write can leave the remote outcome uncertain; inspect Notion
  before retrying. Only explicit HTTP 429 responses are retried, with a bound.
- If Notion returns different Markdown after a successful write, Quillship keeps
  your exact local file and records Notion's returned baseline. It reports that
  the file remains dirty; it does not silently discard text during normalization.

To resolve divergence manually, first save your local body elsewhere with
`view PAGE_UUID > local-backup.md`. Save the old baseline with
`view PAGE_UUID --baseline > old-baseline.md`. Replace the working body using
`edit PAGE_UUID --file old-baseline.md`, then `pull PAGE_UUID` to obtain the current
remote body. Merge your saved local edits into that file, inspect the result,
and push. Keep the backup until you have checked the result in Notion.

Notion doesn't expose atomic compare-and-swap on Markdown replacement, so a
remote edit between the preflight read and the write can race. This initial
implementation detects changes during retrieval and before push, but cannot
guarantee protection against that final remote race. Complete reads are required
for push. Pull retains readable Markdown even when Notion returns truncated
content, unknown/unsupported placeholders, missing completeness flags or warnings.
Those documents remain viewable, searchable and editable locally, but pushing is
disabled so unfetched content cannot be overwritten. Unknown placeholders remain
exactly as returned; Quillship does not invent or reconstruct missing content.

Warnings appear on stderr during pull/view, in plain list/search/status output,
and in JSON as `incomplete` and `sync_warnings`. `incomplete` describes the saved
baseline. If a newer partial response arrives for a previously complete local
snapshot, Quillship keeps the complete body/baseline and records the new warnings;
those warnings also disable push. Repeated partial pulls preserve that snapshot.
A later complete pull clears warnings and can upgrade a clean partial document.
If a partial document has local edits, save them elsewhere before upgrading and
merge them after the complete pull, following the conflict workflow above.

This first release edits bodies of existing pages. Titles/properties are read-only;
page creation/deletion, database queries, attachment downloads, automatic merging
and background synchronization are outside its scope. Search-based discovery is
eventually consistent and may not return every accessible page immediately; pull
known page IDs directly when needed. Pull attempts every discovered/requested
page, reports failed IDs and errors, and prints a saved/incomplete/failed summary.
Unreadable pages, access errors and conflicts are failures; they produce a
nonzero exit after the rest of the batch finishes. Readable partial pages alone
do not fail a pull. Push stops at the first error. Earlier successful documents
remain saved/synced in both cases.

## Design and validation

Feature artifacts are in [`specs/001-notion-doc-sync`](specs/001-notion-doc-sync).
HTTP contract tests use a local mock server and real temporary sled workspaces;
they do not read a token or write to live Notion pages.

API references:

- [Retrieve page Markdown](https://developers.notion.com/reference/retrieve-page-markdown)
- [Update page Markdown](https://developers.notion.com/reference/update-page-markdown)
- [Search pages](https://developers.notion.com/reference/post-search)
