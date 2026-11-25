# Attachments plan
US7 depends on operation recovery and block/property discovery. Implemented in
files.rs, uploads.rs, the shared Notion transport, journal recovery and CLI.

CLI: files list PAGE; files fetch FILE_REF [--directory PATH]; files upload
--file PATH --content-type TYPE; files attach PAGE --upload ID --name NAME.
Cache stable file/block/property identities and local checksum/size/relative path
in files-v1. Discover signed URLs just in time; do not persist them as permanent
references. External URLs must be explicit; never crawl arbitrary linked sites.

Downloads use a separate unauthenticated reqwest client, no redirects by default,
streaming limits and atomic temporary-file persistence. Reject traversal names
and collisions; use identity paths and a validated extension. Failed/expired links
retain existing files; refresh through authenticated discovery once and report
failure if still unavailable.

Uploads begin with single-part <=20 MiB, then multipart upload/send/complete,
journalled by upload/part identity before attachment. Recheck API limits, MIME
rules and async completion before implementation. Attachment is a separate
explicit write; upload completion is not evidence that a page contains the file.

Tests: token never sent to download host, expiry refresh, redirect rejection,
unsafe filenames, collision, byte limits/checksums, partial download, upload-part
failure, restart with an existing upload and attach/readback uncertainty.

Implementation details: list is a bounded recursive page/block scan, with offline
cached references. References use owner/slot and a SHA-256 digest of URL host/path
(excluding query/fragment); property slots use stable property IDs. Refresh must
match that asset identity before download. URLs exist only in memory. Downloads
have independent unauthenticated transport, a 100 MiB default byte cap, identity
filenames, SHA-256 and atomic no-clobber persistence; external files require an
explicit flag. HTTP 401/403/404 refresh once. Existing bytes are never overwritten.

Upload snapshots exact input bytes locally before mutation, with whole/part hashes.
Single-part <=20 MiB; multipart uses 10 MiB parts, max 5 GiB. Each create/send/
complete operation has a durable journal record and each confirmed part is cached.
--resume requires matching saved input and only sends unattempted parts. Notion's
upload API has no per-part inventory: pending uploads with uncertain part outcomes
cannot be safely resumed automatically. Recovery observes; it never retransmits.
An uploaded status can confirm completed transfer, with content length checked.

Attach is an explicit separate append-block mutation with returned block identity
and verified block readback. It preserves local Markdown and body baselines;
subsequent pull/reconcile incorporates the attached block, and push detects any
resulting divergence. It never rewrites a body to attach a file.

Verified primary contracts under https://developers.notion.com/:
reference/create-file, upload-file, complete-file-upload, retrieve-file-upload,
file-upload, file-object and patch-block-children; guides/data-apis/retrieving-files
and sending-larger-files. Multipart sends use native form-data file/part_number;
attachment uses a file_upload block reference with explicit name. API-supplied
upload/complete URLs are never followed or stored: requests use the authenticated
configured API base and verified UUID paths.
