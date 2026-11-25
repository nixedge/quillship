# API research and constraints
Checked 25 November 2026; API version remains 2026-03-11.

Primary references:
- https://developers.notion.com/reference/patch-page
- https://developers.notion.com/reference/move-page
- https://developers.notion.com/reference/get-block-children
- https://developers.notion.com/reference/update-page-markdown
- https://developers.notion.com/reference/create-a-comment
- https://developers.notion.com/reference/list-comments
- https://developers.notion.com/guides/data-apis/working-with-databases
- https://developers.notion.com/reference/query-a-data-source
- https://developers.notion.com/reference/create-file
- https://developers.notion.com/guides/data-apis/retrieving-files

Moves use a dedicated POST endpoint, not a parent field on PATCH /pages.
Comments can target pages/blocks or existing discussions; selected-text anchoring
is not available for new comments. List comments returns unresolved discussions.
Read and insert comment capabilities are separate.

Database containers list data sources. Current schemas belong to data sources;
query/create must use the selected source identity rather than guessing the first.
Query responses can be capped/incomplete independently of pagination completion.
Files expose expiring URLs; stable identities and authenticated discovery must
be separated from unauthenticated byte retrieval.

Markdown insert/update operations exist; exact v2 payload and partial-read/block
identity behaviour must be rechecked at T022. No assumed ETag/atomic CAS, permanent
file URL, full comment history or automatic conversion of arbitrary block types.
