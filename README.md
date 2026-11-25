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
the CLI reads it only for explicit network operations. For direnv, `.envrc`
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

## Create a page

Create a private workspace-level hub, or a child beneath an accessible page:

```sh
direnv exec . cargo run -- create --workspace-root \
  --title 'Project proposal' --file proposal.md --json
direnv exec . cargo run -- create --parent PAGE_UUID \
  --title 'Funding and accountability' --file funding.md --json
cat proposal.md | direnv exec . cargo run -- create --workspace-root \
  --title 'Proposal draft' --file -
```

Use exactly one of `--workspace-root` and `--parent`. Notion requires insert
content capability. Workspace-level private pages require an eligible public or
user-backed connection; internal integrations need an accessible parent page.
Titles are explicit, so keep the title out of the Markdown body if duplication
is unwanted. Native Notion Markdown v2 is used, with silent notifications.

The created UUID and URL are reported immediately on stderr. On success, the
page is saved in sled and `docs/<uuid>.md`, ready for normal view/edit/push.
`--json` emits id, title, url, working-file path, dirty/incomplete flags and
warnings on stdout. Exact input is retained; Notion's readback becomes the
baseline. Normalized input remains modified, and incomplete readbacks block push.

Creation is a live write. Repeating it can create duplicates. Transport errors,
invalid responses and 5xx failures are never automatically retried; inspect
Notion before repeating an uncertain create. Only explicit 429s use bounded
retries. If remote creation succeeds but readback/local saving fails, recover
using the reported UUID with `pull`; preserve your input for merging afterwards.
Existing local pages and untracked files are never overwritten on identity
collision. Automated tests use local mock servers and create no live pages.

The operation record motivating this command is in
[`specs/002-notion-page-create/curl-audit.md`](specs/002-notion-page-create/curl-audit.md).

## Local workspace

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
before syncing. Page arguments accept hyphenated/compact UUIDs and HTTPS Notion
page links, including title slugs and `/p/` links. Link queries/fragments are
ignored. Only recognised Notion hosts are accepted; pasted links are never fetched.

## Review and organise

```sh
# Offline review and explicit acceptance of the saved Notion formatting.
cargo run -- diff PAGE_UUID
cargo run -- reconcile PAGE_UUID --take baseline --json
cargo run -- snapshots PAGE_UUID
cargo run -- snapshot SNAPSHOT_ID --side local > recovered.md

# Fresh complete remote snapshots for reviewing/resolving divergence.
direnv exec . cargo run -- diff PAGE_UUID --remote --json
direnv exec . cargo run -- reconcile PAGE_UUID --take local
direnv exec . cargo run -- reconcile PAGE_UUID --take remote
direnv exec . cargo run -- reconcile PAGE_UUID --file merged.md

direnv exec . cargo run -- inspect PAGE_UUID --json
cargo run -- inspect PAGE_UUID --offline --json
direnv exec . cargo run -- rename PAGE_UUID --title 'Review notes'
direnv exec . cargo run -- move PAGE_UUID --parent PARENT_UUID
direnv exec . cargo run -- tree PAGE_UUID --max-depth 8 --max-blocks 10000 --json
direnv exec . cargo run -- pull PAGE_UUID --recursive --max-depth 8 --max-blocks 10000
```

`diff` compares exact text against the saved baseline; `--remote` adds a fresh
baseline/remote comparison. Diffs use a single context-limited hunk and may show
unchanged lines between separate edits as removals/additions. JSON includes exact
bodies and completeness diagnostics. Review does not advance the baseline.

`reconcile --take baseline` accepts the saved complete baseline offline, useful
after Notion normalizes Markdown. `--take local` keeps your body and adopts the
fresh remote baseline, preparing an intentional overwrite on the next push.
`--take remote` accepts fresh remote content. `--file` (or stdin `-`) selects a
merged body over the fresh remote baseline. All choices archive the old local
body, old baseline, selected result and any fetched remote body in sled before
changing files. `snapshot --side local|baseline|remote|selected` exports exact
archived text; missing sides fail. Snapshots survive restart and have no automatic
pruning. Reconcile never writes to Notion; inspect the diff and explicitly push
to publish. Incomplete reads are rejected, and concurrent file edits are retained.

`inspect` caches safe page metadata separately; `--offline` reads that cache.
Rename and move are live writes requiring update content capability. They keep
local bodies and body baselines intact. Move targets existing pages, rejects
self-parenting, and leaves descendant-cycle validation to Notion. If a mutation
is uncertain, inspect before repeating it. Successful identities are reported on
stderr before local cache refresh; cache errors do not undo remote success.

`tree` follows actual child pages, including pages nested inside container blocks,
and paginates block children. It does not follow ordinary links or query database
records. The depth limit counts page nesting; the block budget covers all visited
blocks. The same budget also caps child-list requests, bounding empty pages with
changing cursors. Traversal failures, cycles, repeated cursors and limits produce a nonzero
exit with partial results. Recursive pull requires one root, saves discovered
pages using normal conflict preservation, and fails if traversal or any pull fails.

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

To resolve divergence, use `diff PAGE_UUID --remote --json` to review all sides,
prepare a merged file, then `reconcile PAGE_UUID --file merged.md`. Review the
resulting diff and explicitly push. The reconciliation snapshot retains the
original bodies, including the submitted merge, for later recovery.

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

Existing page titles can be renamed. Other property updates, database queries,
comments, targeted remote edits, attachments and a remote-operation recovery
journal are planned in [`specs/003-notion-workflows`](specs/003-notion-workflows)
and remain unimplemented. Page deletion, automatic merging and background
synchronization are outside scope. Search-based discovery is
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
