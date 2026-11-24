# 001 — Local Notion document workflow

Status: specified | Date: 2026-11-24

## User stories and acceptance

### US1 (P1): Read and find documents offline

As a user, I pull shared Notion pages into local storage and then view, list and
search their titles and full Markdown content without a network connection.

- Given accessible page IDs, when I pull, then complete Markdown and metadata
  persist across process restarts in sled and working files.
- Given a title query (or all accessible pages), when I pull without IDs, then
  discovery follows every search cursor and deduplicates IDs.
- Given incomplete Markdown, when I pull, then readable content is stored with
  explicit warnings and cannot be pushed. A previously complete local snapshot
  is preserved rather than downgraded to partial content.
- Given an unreadable page, when I pull a batch, then its ID and error are
  reported, remaining pages are attempted, and a final summary records failures.
- Given local records, when I list/view/search with no token, then output is
  available; search is case-insensitive over title and body.

### US2 (P1): Edit using an editor or LLM

- Given a pulled document, when I run edit, then $VISUAL or $EDITOR opens a
  persistent Markdown file and successful edits become searchable in sled.
- Given a Markdown path or stdin, when I run edit --file, then the body is
  updated without invoking an interactive editor.
- Given an LLM changes the exposed working file directly, when I run a document
  command, then that content is ingested before viewing/searching/syncing.
- Given editor failure, then the file remains recoverable and no remote write
  occurs. Missing working files are errors, never interpreted as empty content.

### US3 (P1): Synchronize explicitly without silently losing edits

- Given local edits and an unchanged remote baseline, when I push, then only
  changed bodies are sent and a successful response establishes a new baseline.
- Given divergent local and remote changes, when I pull or push, then both
  copies are preserved and the command reports a conflict.
- Given a dirty file and unchanged remote content, when I pull, then my edits
  survive. Given identical edits on both sides, then sync converges to clean.
- Given push --dry-run, then changed IDs are shown without credentials or HTTP.
- Given HTTP failure, then local edits and the previous baseline remain intact.

## Requirements

FR-001: Provide explicit pull, push, list, view, edit, search, status and path commands.
FR-002: Store canonical UUID identity, title, URL, remote edit timestamp, Markdown
body and baseline in sled under a configurable workspace directory.
FR-003: Store stable docs/<uuid>.md working copies; page titles never become paths.
FR-004: Support machine-readable JSON for record listings and status.
FR-005: Validate UUIDs before filesystem or HTTP use. Never store credentials.
FR-006: Accept truncated/unknown content for local reading with explicit warnings;
reject incomplete documents for push. Preserve child pages by using Notion's
default deletion protection, report API errors, and bound timeouts/retries.
FR-007: Flush local changes durably; fail on corrupt records and missing files.
FR-008: Keep offline commands independent of token configuration.
FR-009: Pull continues after per-page errors/conflicts and summarizes successful,
incomplete and failed pages. Actual page failures produce a nonzero CLI exit after
the entire batch; incomplete but readable documents alone do not fail the command.
FR-010: Persist completeness and diagnostics with backwards-compatible defaults
for existing sled records. Display them in plain listings, view warnings and JSON.

## Clarification — 2026-11-24

After a live pull stopped on unknown Markdown, the user authorized handling
noncompliant documents and continuing the batch. Unknown placeholders remain in
the Markdown as returned by Notion; missing content is never invented. This
revision supersedes the initial strict-pull rejection policy, while retaining
strict push preflight. Block reconstruction is outside this repair's scope.

## Assumptions and scope

The first slice synchronizes bodies of existing pages shared with an integration.
Titles/properties are read-only metadata; creation, deletion, attachments download,
database querying, automatic merges and background sync are future features.
Discovery uses Notion search, which is eventually consistent and not a guarantee
of exhaustive workspace export. Partial pulls may persist earlier successful pages.
Search initially scans sled records, following security-council's approach.
Only one CLI process opens a workspace at a time; external editors should finish
saving before sync. Remote preflight is best-effort: Notion offers no atomic
compare-and-swap for a Markdown replacement.

## Success criteria

SC-001: Automated tests prove pull → offline edit → search → push → clean status.
SC-002: Tests prove divergence and failed HTTP writes preserve local work/baselines.
SC-003: CLI is buildable and runnable through the checked-in Nix environment.
SC-004: Pagination, incomplete content and HTTP request payloads have coverage.
