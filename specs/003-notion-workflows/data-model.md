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

## Implemented additional trees

operations-v1: explicit mutation intent, authored input, remote identity, outcomes
and per-item batch state. comments-v1 stores complete unresolved target snapshots;
comment-objects-v1 stores individually observed posted comments. schemas-v1,
database-containers-v1, queries-v1 and record-properties-v1 keep typed structured
metadata/complete baselines separate from Markdown. Query cache keys include the
source, filter/sort and limit. Ephemeral file URLs are stripped from structured
snapshots without inventing their contents.

files-v1 stores page discovery snapshots; file-refs-v1 stores owner/slot/asset
identity with no signed URL. files-local-v1 records downloaded filename, directory,
checksum and byte count. uploads-v1 tracks input checksum/size/name/type and part
hashes, confirmed parts, remote state and initial operation ID. upload-inputs/
contains retained exact binary input; files/ contains default downloads. Attachment
records contain the page, upload and verified new block identity. These trees do
not alter body baselines. All records remain after completion, without pruning.
