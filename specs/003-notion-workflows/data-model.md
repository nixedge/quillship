# Local records and boundaries

## Implemented initial tranche

Existing documents-v1 records and docs/UUID.md working files remain compatible.
Document markdown is authored text; baseline is the last accepted remote text.
The incomplete flag and warnings keep their existing meanings. Metadata refresh
does not advance a body baseline.

reconciliation-v1 uses big-endian sled-generated u64 keys. Each Snapshot stores
id, action, original local Document (including baseline), optional strict complete
RemoteDocument, and selected_markdown. The selected body preserves merged input
even if local saving fails. Flush the record before document replacement. Listing
and side export remain offline. There is no automatic deletion/pruning.

page-metadata-v1 uses canonical page UUID keys. PageInfo stores id, title, URL,
last edit time, raw parent descriptor and archived/trash state. It excludes
credentials and page properties. Cache refresh updates document display metadata
while preserving exact working Markdown, baseline and completeness diagnostics.
Offline inspection reports the cached snapshot without implying freshness.

TreeReport is ephemeral: discovered page identities/title/owner/depth, failures,
truncation flag, blocks scanned and request count. Block-container traversal is
separate from page nesting. Duplicate identities/cursors and budgets prevent
unbounded work. Recursive pull feeds only discovered identities into normal pull.

## Planned subsequent trees

operations-v1: explicit mutation intent, authored input, remote identity, outcomes
and per-item batch state. Comments, schemas/properties and files each use separate
versioned records with completeness/freshness semantics defined in workstreams/.
They are planned records, not implemented migrations. Never mix them into body
baselines or persist authentication material. Read old records before introducing
new required fields, and add restart/compatibility validation with each tranche.
