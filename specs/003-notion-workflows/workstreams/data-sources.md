# Structured records plan
US6 depends on operation recovery and metadata/property separation.

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
