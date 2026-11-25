# Structured records plan
US6 depends on operation recovery and metadata/property separation. Implemented
in structured.rs, journal recovery, cache records and CLI.

CLI: database inspect ID; source inspect ID; source query ID --filter FILE
--sort FILE [--limit N]; record properties PAGE; record create --source ID
--properties FILE [--file BODY]; record update PAGE --properties FILE.
Structured commands emit JSON and cache schemas/query snapshots separately.
Use schema/property IDs, retain type/readonly metadata and completeness flags;
support common title, text, number, checkbox, select/status, date, URL and relation
types incrementally. Unknown/formula/rollup types remain read-only. Explicit null
and empty collections mean clear only for writable types with validated contracts.

Storage: schemas-v1 and record-properties-v1; property baseline separate from body
baseline. Paginate property items and queries, including relation completeness.
Select a source explicitly when a database has multiple sources. Detect capped
query request_status, not just has_more. Linked views do not grant source access.
Creating database/schema/view definitions is a later extension after record work.

Tests: multiple sources, schema drift, readonly/unknown types, explicit clearing,
partial relation/property reads, query caps and filters, conflicting property
changes, body-edit preservation, journalled creation/update and restart recovery.

Implementation contracts: inspect supports --offline. Query inputs use native
filter/sort JSON, validated against an explicit source schema and supported typed
conditions. Query --limit defaults to 10000; incomplete/capped results are returned
with a nonzero exit and do not replace a previous complete query cache. Query
reads do not imply complete per-record property pagination. Record properties
supports --offline; online reads fully paginate title/text/people/relations and
rollups, checking page edit times around retrieval. Update requires a saved
complete property baseline and compares only selected columns against fresh
values, allowing unrelated changes. Unknown/read-only types are preserved on read
and rejected for writes. Write keys resolve to stable schema property IDs.
Record creation always requires --source; no implicit database/source selection.
record move PAGE --source ID uses the dedicated page move endpoint, preserves
body baselines and requires explicit property refresh after changing sources.
Property operations never update body baselines. Creation with body input retains
authored Markdown through normal local document storage/readback.

Verified primary references: retrieve-database, retrieve-a-data-source,
query-a-data-source, retrieve-a-page-property, page-property-values,
filter-data-source-entries and sort-data-source-entries under
https://developers.notion.com/reference/.
