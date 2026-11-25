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

## Files and attachments

```sh
direnv exec . cargo run -- files list PAGE_UUID --json
cargo run -- files list PAGE_UUID --offline --json
direnv exec . cargo run -- files fetch FILE_REF --max-bytes 104857600 --json
direnv exec . cargo run -- files fetch FILE_REF --external --directory ./downloads --json
direnv exec . cargo run -- files upload --file report.pdf --content-type application/pdf --json
cargo run -- files status UPLOAD_UUID --json
direnv exec . cargo run -- files upload --resume UPLOAD_UUID --file report.pdf --content-type application/pdf --json
direnv exec . cargo run -- files attach PAGE_UUID --upload UPLOAD_UUID --name report.pdf --json
```

List scans page covers/icons, file properties and nested media blocks, including
actual child pages. `--max-blocks` bounds traversal (default 10000). Failed scans
retain the previous cache; cached references and upload status are readable
offline. File references use stable owners/slots and an asset digest; signed
access URLs are kept only in memory. Download refresh refuses changed identities.
Ordinary page links and database records are not crawled.

Fetch uses an independent client without Notion credentials, requires HTTPS, and
does not follow redirects. External file objects require `--external`. Expired or
denied links (401/403/404) refresh once. The default download cap is 100 MiB; stream
limits also apply when size is unknown. Atomic persistence uses an identity-based
filename and SHA-256 metadata, never overwrites existing bytes, and leaves no
partial output after failure. Matching cached bytes can be reused; modified or
untracked collisions fail. Download filenames cannot contain path separators or
control characters. Back up downloaded bytes with the complete workspace.

Upload requires explicit content type and a valid filename; Notion's supported
types and workspace size limits still apply. Exact input bytes are retained under
`upload-inputs/<sha256>.bin` before sending. Inputs at most 20 MiB use single-part;
larger inputs use 10 MiB parts, up to 5 GiB. Creation, each part and completion are
separately journalled, with whole/part checksums and confirmed-part tracking.
Resume verifies saved input, reuses the returned identity and sends unattempted
parts. Unknown part/completion outcomes on pending uploads require inspection:
Notion exposes no per-part inventory, so recovery never blindly retransmits.
Uploaded state is checked against the retained byte count. A completed upload
is separate from attaching it to a page.

Attach appends a file block and verifies returned block identity/parent. Remote
success is retained even if readback fails. Local Markdown and its baseline are
preserved; pull/reconcile explicitly to incorporate the attachment. Push checks
the resulting divergence before any body replacement. Recovery observes the
returned block and never appends it a second time. There is no automatic cleanup
of cached downloads, input snapshots, completed uploads or journal records.

## Databases, data sources and records

```sh
direnv exec . cargo run -- database inspect DATABASE_UUID --json
direnv exec . cargo run -- source inspect SOURCE_UUID --json
direnv exec . cargo run -- source query SOURCE_UUID --filter filter.json --sort sorts.json --limit 10000 --json
direnv exec . cargo run -- record properties PAGE_UUID --json
cargo run -- record properties PAGE_UUID --offline --json
direnv exec . cargo run -- record update PAGE_UUID --properties changes.json --json
direnv exec . cargo run -- record create --source SOURCE_UUID --properties values.json --file body.md --json
direnv exec . cargo run -- record move PAGE_UUID --source SOURCE_UUID --json
```

Database inspection lists all contained sources; callers choose a source UUID
explicitly. Inspection, queries and property reads support `--offline`. Schemas,
complete query snapshots and complete property baselines are separate sled records.
They never advance Markdown body baselines. Ephemeral file download URLs are
removed from durable structured snapshots; retrieve file access when needed.

Filter and sort files use native Notion JSON. Predicates are checked against
schema types and supported operators: text, number, checkbox, select/status, date
and relation conditions, plus bounded and/or groups. Sorts select a property or
creation/edit timestamp and an ascending/descending direction. Unknown conditions
fail explicitly. Query caps, requested limits, duplicate identities and pagination
problems are reported. Incomplete results do not replace a complete cached query;
the CLI emits results/diagnostics and exits nonzero. Offline queries require the
same filter/sort/limit as the cached complete query.

Property input is a native object such as
`{"Count":{"number":42},"Done":{"checkbox":true}}`. Names or stable property IDs
are accepted; duplicate/ambiguous selections fail. Writable types are title,
rich_text (text items), number, checkbox, select/status, date, URL and relation.
Select/status options must exist in the schema. Other types are retained on read
and rejected on write. Empty arrays explicitly clear text/relations; nullable
types accept null; checkbox clearing is false. Status null requests Notion's
default option. Unsupported normalization/default outcomes can require manual
inspection during recovery rather than assuming equivalence.

`record properties` fully paginates text, people, relations and rollups, checks
edit timestamps, and retains the previous baseline if retrieval fails. These
reads remain limited to the connection's permissions. Query rows alone do not
prove property completeness. Before update, fetch/review a complete property
baseline. The command verifies selected columns against a fresh snapshot; changes
to other columns are preserved. Updates send only selected properties and retain
the input in the journal. After a conflict, review fresh properties before another
explicit update. Property preflight has the same final remote race limitation as
body writes. Creation with body input retains exact authored Markdown and a remote
baseline; a properties-only create does not invent a local body working copy.
Moving into another source preserves the body baseline; fetch properties explicitly
before updating the moved record. Notion validates source compatibility.

## Targeted edits

```sh
cargo run -- append PAGE_UUID --file addition.md --dry-run --json
direnv exec . cargo run -- append PAGE_UUID --file addition.md --position end --json
direnv exec . cargo run -- replace PAGE_UUID --old-file old.txt --new-file new.md --json
direnv exec . cargo run -- replace PAGE_UUID --old-file old.txt --new-file new.md --all-matches --json
```

Append inserts whole Markdown content at the start/end of a clean synced page.
Replace uses exact case-sensitive matching and requires one nonempty match unless
`--all-matches` is explicit. Empty replacement text deletes matched text; child
page/database deletion stays disabled at the API. Inputs support stdin `-`, with
only one stdin source per replacement.

Dry-run is offline, creates no journal entry, and marks `remote_checked: false`.
Its preview uses the saved complete body; append separates old/new content with
two newlines. The live command retrieves complete current content and refuses
dirty, incomplete, divergent or concurrently changed inputs before submission.
It journals original/input/selected text, calls native insert/update Markdown v2,
and verifies fresh readback. Exact selected text is retained when Notion normalizes
formatting; the result can remain dirty until reviewed and accepted with reconcile.
Edits made during the write survive. Failed/changed readback leaves a recoverable
record; no mutation is automatically replayed. These checks do not close Notion's
final remote race window.

## Comments

```sh
direnv exec . cargo run -- comments pull PAGE_OR_BLOCK_UUID --recursive --json
cargo run -- comments list PAGE_OR_BLOCK_UUID --recursive --json
direnv exec . cargo run -- comments add --page PAGE_UUID --file review.md --json
direnv exec . cargo run -- comments add --block BLOCK_UUID --file review.md --json
direnv exec . cargo run -- comments add --discussion DISCUSSION_UUID --file reply.md --json
```

Reads require read-comment capability; writes require insert-comment capability.
`pull` caches a complete paginated unresolved snapshot separately from Markdown;
failed/partial pagination preserves the previous cache. `list` is offline and
shows the cached fetch time. Recursive and direct snapshots are separate; use
`--recursive` consistently when reading them. Recursion follows actual blocks
with a configurable `--max-blocks` budget (default 10000); pagination is bounded.
Results cover unresolved comments, not resolved history.

Add requires exactly one target. Comment Markdown supports inline formatting;
headings, lists, tables, blockquotes and fenced blocks are rejected before posting.
Use discussion IDs to reply to existing discussions. New selected-text anchors
are unsupported. Writes are journalled and report comment identity before local
persistence. Posting does not refresh the full unresolved snapshot; pull again.
Recovery of an uncertain post with no identity requires manual inspection rather
than automated reposting. Partial creation responses still retain input/identity.

## Interrupted writes

Every CLI mutation, including record creation/update, durably records intent
before submission.
Sent, confirmed and locally saved transitions distinguish unknown remote outcomes
from failed local readback. Exact authored inputs and push baselines are retained.
Each push-batch item has its own operation and shared batch ID; unattempted items
remain prepared when a batch stops.

```sh
cargo run -- operations --pending --json
cargo run -- operation OPERATION_ID --json
direnv exec . cargo run -- recover OPERATION_ID --json
```

Inspection is offline. Recovery only reads Notion and updates verified local
state; it never repeats a mutation. It retains concurrent local edits and refuses
to advance a baseline if the observed result differs from the recorded outcome.
An uncertain create without a returned identity needs manual inspection in
Notion. Unknown normalization or later remote changes need explicit reconcile.
Prepared entries were not sent; issue a new explicit command after reviewing
their inputs. Completed records are retained without automatic pruning.

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

Existing page titles, supported record properties and file attachments are
implemented. Page deletion, automatic merging and background
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
