# Review comments plan
US4 depends on operation recovery for writes.

CLI: comments pull PAGE|BLOCK [--recursive] --json; comments list TARGET --json
(offline); comments add --page PAGE|--block BLOCK|--discussion ID --file PATH|-.
Keep author/time/discussion/target/rich-text or Markdown separately from body
snapshots in comments-v1. Cache a full successful paginated unresolved-comment
snapshot; retain prior cache on failed pagination and indicate incomplete results.
Recursive collection uses known block identities, not Markdown line positions.

GET /comments?block_id=ID&page_size=100 with cursor cycle checks.
POST /comments targets exactly one page/block/discussion. Markdown writes use v2
inline syntax; report unsupported block-format expectations before submission.
Reply to known discussions without claiming to create selected-text anchors.
No auto-posting from pulled comments or unattended replies.

Tests: comments on nested blocks, pagination/cursor repetition, unresolved-only
semantics, offline cache/reopen, separate capabilities/403, journalled add/reply
and uncertain responses. No resolve/delete/history claims beyond exposed API.
