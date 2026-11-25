# Review comments plan
US4 depends on operation recovery for writes. Implemented in comments.rs, the
journal, cache helpers and CLI. Output is structured JSON by default.

CLI: comments pull PAGE|BLOCK [--recursive] --json; comments list TARGET --json
(offline); comments add --page PAGE|--block BLOCK|--discussion ID --file PATH|-.
Keep author/time/discussion/target/rich-text or Markdown separately from body
snapshots in comments-v1. Cache a full successful paginated unresolved-comment
snapshot; retain prior cache on failed pagination and indicate incomplete results.
Recursive collection uses known block identities, not Markdown line positions.
Direct/recursive snapshots have separate keys. Full comment reads require all
documented author/time/target/rich-text fields; partial lists fail without
replacing the previous cache. A partial creation response can still confirm its
identity and preserve authored input, without claiming the read cache is fresh.
Recursive traversal has a configurable total block/request budget; direct comment
pagination is capped at 1000 responses per target. GET /comments/ID supports
observation-only journal recovery when a returned identity is known.

Verified contracts: https://developers.notion.com/reference/list-comments,
https://developers.notion.com/reference/create-a-comment and
https://developers.notion.com/reference/retrieve-comment.

GET /comments?block_id=ID&page_size=100 with cursor cycle checks.
POST /comments targets exactly one page/block/discussion. Markdown writes use v2
inline syntax; report unsupported block-format expectations before submission.
Reply to known discussions without claiming to create selected-text anchors.
No auto-posting from pulled comments or unattended replies.

Tests: comments on nested blocks, pagination/cursor repetition, unresolved-only
semantics, offline cache/reopen, separate capabilities/403, journalled add/reply
and uncertain responses. No resolve/delete/history claims beyond exposed API.
