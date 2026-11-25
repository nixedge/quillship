# Initial CLI and API contracts

Commands below belong to the initial implementation; tasks.md records validation
and remaining workstreams.

## Identity and local review
All page arguments accept canonical/compact UUIDs and recognised HTTPS Notion page
URLs (including app /p/ links); query/fragment are not identity. Reject other hosts,
userinfo and non-HTTPS URLs.

- diff PAGE [--remote] [--json]: baseline/local unified diff offline; --remote adds
  baseline/current-remote comparison through strict retrieval. No baseline changes.
- reconcile PAGE --take baseline|local|remote [--json]: baseline is offline;
  local/remote retrieve fresh complete remote. Backup precedes any local replacement.
- reconcile PAGE --file PATH|- [--json]: merged body input, fresh remote baseline.
- snapshots PAGE [--json]: saved recovery records; snapshot SNAPSHOT_ID
  --side local|baseline|remote|selected reads exact text. Missing remote side is an error.
No reconcile command writes to Notion; push remains explicit.

## Organisation
- inspect PAGE [--offline] [--json]: GET /pages/ID and persist safe metadata;
  --offline reads the last cached snapshot and requires no credentials.
- rename PAGE --title TITLE [--json]: PATCH /pages/ID using the actual title
  property key discovered from page metadata.
- move PAGE --parent PAGE [--json]: POST /pages/ID/move with
  parent {type:page_id,page_id:UUID}. Reject self-parenting locally.
- tree PAGE [--max-depth N] [--max-blocks N] [--json]:
  GET /blocks/ID/children with page_size=100/start_cursor. Follow actual child_page
  blocks and nested containers. Report missing access, cycles and traversal limits.
  The block budget also caps child-list requests to bound empty pagination.
- pull PAGE --recursive [--max-depth N] [--max-blocks N]:
  complete bounded discovery, then existing pull semantics for all selected pages.
  Continue per-page errors and return nonzero for traversal or pull failures.

Rename/move never replace bodies, discard local work or silently rebase a dirty
document. Output remote success before local metadata persistence and include
recovery guidance on failed local refresh.
