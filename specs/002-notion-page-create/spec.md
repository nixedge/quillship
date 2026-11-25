# Feature specification: create Notion pages
Branch: 002-notion-page-create | Created: 2026-11-24 | Status: specified

User request: keep track of curl usage and add the functionality to the Rust tool.

## User stories and acceptance

### US1 Create a proposal hub (P1)
Create a named workspace page from a UTF-8 Markdown file or stdin.
Given a user-backed connection with insert capability, when create is invoked with
--workspace-root --title TITLE --file PATH, one page is created, its UUID and URL
are reported, and the returned remote snapshot plus exact submitted working body
are stored in the normal workspace. A normalized response is a visible difference.
No existing page is edited. Workspace creation permissions remain Notion's decision.

### US2 Create supporting child pages (P1)
Given an accessible page UUID, when create is invoked with --parent UUID, the
new page has that parent. Exactly one of --parent and --workspace-root is required.
Invalid UUIDs, empty titles, unsupported placeholders and unreadable inputs fail
before any remote write. An empty body is valid.

### US3 Recover uncertain creation (P1)
Given a page creation succeeds but readback or local persistence fails, the error
must identify the created page and URL and advise pull rather than repeating create.
A transport/invalid creation response reports uncertain outcome without retry.
Only explicit HTTP 429 responses may retry under the existing bounded policy.

## Requirements
- FR1 Native Markdown v2 synchronous POST /pages, explicit title and parent,
  notifications silent, allow_async false; use existing token/version/rate limiting.
- FR2 File/stdin input, --json output with id/title/url/path and dirty/incomplete
  status, normal list/view/edit/push compatibility.
- FR3 Preserve exact authored Markdown; remote readback is the baseline, with
  completeness warnings enforced by existing push rules.
- FR4 Never overwrite an existing local record or working file on returned identity
  collision; retain source file and report created identity on persistence failure.
- FR5 No credentials in logs or artifacts and no live creation during tests.
- FR6 Document the actual curl operations, outcomes and their Rust replacement.

## Edge cases
Missing insert permission; internal connections unable to create workspace pages;
429 versus 5xx or malformed responses; normalized/partial Markdown; identity
collision; missing source file; invalid title and parent; successful write followed
by failed readback. A repeated explicit create can create duplicates: it is not a
sync operation and has no server idempotency guarantee.

## Success criteria
Mock-server tests demonstrate workspace and child creation contracts, persistence,
auth/version/notification settings, normalization and uncertain-write recovery.
Existing tests, fmt and strict all-target Clippy pass through Nix.
A user can create a hub and child using Rust commands without curl.

## Scope
Page/data-source movement, database creation, general metadata editing, async tasks,
bulk manifests and a generic HTTP escape hatch are deferred. Existing curl page and
identity GETs were diagnostics; normal pull already retrieves page metadata.
