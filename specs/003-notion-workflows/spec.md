# Feature specification: Notion document workflows

Branch: 003-notion-workflows | Created: 2026-11-24 | Status: all seven user stories implemented

Request: plan all identified API and CLI gaps, then work through them. Keep
specifications generic. Extend the Rust CLI and existing local workspace.

## User scenarios and acceptance

### US1 Review and reconcile documents (P1)
Given a local working copy and baseline, diff shows their actual changes offline.
With --remote it also retrieves current complete remote content without changing
the local copy or saved baseline. Given divergence, an explicit reconcile choice
(local, remote or merged input) archives all sides before updating local state.
Accepting saved remote formatting is an explicit offline operation with a backup.
Users can list and read archived snapshots after reopening the workspace.

### US2 Organise document sets (P1)
Given a pasted Notion page URL or UUID, all page commands resolve the same identity.
Inspect reports title, parent and state. Rename and move explicitly update only
metadata, retain local body edits and report remote success before local refresh.
A bounded tree walk and recursive pull follow actual child pages, including pages
nested inside blocks, with pagination and cycle detection. Linked pages are not
implicitly children. Missing access and limits are reported, not silently omitted.

### US3 Recover interrupted operations (P1)
Before remote writes, durably record operation intent and input. Persist returned
identity/outcome before dependent readback. List interrupted/uncertain operations;
recover by observation/pull rather than repeating non-idempotent writes. Track
batch results per item. Never claim API-side atomicity or exactly-once creation.

### US4 Work with review comments (P2)
List unresolved page/block discussions and retrieve cached results offline.
Explicitly add a page/block comment or reply to an existing discussion, recording
the resulting identity. Keep discussions separate from document Markdown. Paginate;
do not imply resolved-history completeness or selected-text anchoring support.

### US5 Make targeted Markdown edits (P2)
Explicit append or unambiguous text replacement preserves unrelated content and
uses the same baseline/preflight protections as push. Empty/multiple matches fail
before writing unless the user explicitly requests all matches. Preserve authored
input, returned normalization and uncertain-operation recovery.

### US6 Work with structured records (P3)
Inspect databases and their data sources, retain schemas, query with typed filters
and sorting and read paginated properties. Create/update selected records with
schema validation and explicit property clearing. Preserve unknown/read-only types,
relations, partial results and local changes separately from body sync.

### US7 Work with attachments (P3)
Discover page/block/property files; download referenced bytes to safe local paths.
Upload a local file, track completion and attach it explicitly. Preserve stable
file/block identities separately from expiring download URLs. Never forward the
Notion token to signed URLs or follow unexpected redirects with credentials.

## Functional requirements
- FR01 Rust modules and existing sled/Markdown storage remain authoritative.
- FR02 Maintain old-record compatibility and preservation on incomplete reads,
  failed writes, concurrent local edits and divergent remote changes.
- FR03 Use explicit commands for every network operation and write. No background
  services, MCP server, live validation writes or automatic conflict merging.
- FR04 All new commands support useful structured output; offline reads need no token.
- FR05 Mutations preserve/recover input and identify partial or uncertain outcomes.
- FR06 Content, metadata, comments, schemas, properties and files have separate
  local records and completeness/baseline semantics.
- FR07 Recursive operations are bounded, paginated and identity-based; child data
  sources enter through US6, not guessed Markdown links.
- FR08 No organisation-specific content, real page identities or credentials in specs.
- FR09 URL parsing accepts recognised Notion page-link hosts only, rejects arbitrary
  URLs/paths and never follows a supplied URL with API credentials.
- FR10 New write operations retain bounded explicit-429 retry behaviour; no blind
  transport, 5xx or invalid-response retries.
- FR11 Tests cover HTTP contracts, pagination, restart recovery and preservation.
- FR12 Network verification, metadata changes and attachments must not silently
  mark unrelated local body edits clean.

## Edge cases
Invalid/ambiguous URL, absent or inaccessible parent, nested child blocks,
repeated cursors/cycles, changed local file during a request, incomplete remote
snapshot, failed backup, a remote write followed by failed refresh, expired file
URL, unsafe filename, truncated query/property pagination and unsupported types.

## Success criteria
The initial review/organisation tranche replaces manual backup/baseline shell
procedures and supports a complete page-tree pull in mock tests. Later workstreams
each satisfy their recorded contract and recovery tests before being marked done.
All implemented work passes Nix tests, formatting and strict all-target Clippy.
No live document or private workspace is touched by automated validation.

## Assumptions and scope
Use API version 2026-03-11 and Markdown v2. Start with page-to-page moves; data
source moves accompany US6. Trash/restore, permissions/sharing changes, duplication,
views, webhooks and async jobs are outside this backlog unless a selected operation
requires async completion. API limitations are reported honestly.
