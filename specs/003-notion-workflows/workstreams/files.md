# Attachments plan
US7 depends on operation recovery and block/property discovery.

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
