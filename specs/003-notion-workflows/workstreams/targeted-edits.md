# Targeted Markdown editing plan
US5 depends on operation recovery. Implemented in targeted.rs, journal recovery
and the CLI. Dry-run is explicitly offline and reports remote_checked=false;
live writes use fresh complete preflight and readback.

CLI: append PAGE --file PATH|- [--position start|end] and replace PAGE
--old-file PATH --new-file PATH [--all-matches], each with --dry-run/--json.
Read inputs before requests. Retrieve complete fresh content, compare saved
baseline, validate unique/nonempty matches and refuse divergence. Preserve local
dirty work rather than mixing unsubmitted edits into an unrelated remote mutation;
require explicit reconciliation/push first or a separate local staging mode.

Recheck the official Markdown v2 insert_content/update_content contract at
implementation. Use targeted API operations only where block/subpage preservation
semantics are understood; never enable deleting child pages. Journal exact intent
before mutation, retain normalized returned baseline separately from authored input
and use fresh readback to observe the actual outcome.

Tests: empty/absent/multiple matches, explicit all-matches, repeated headings,
Unicode/line endings, unknown blocks, child-page blocks, local changes during
preflight, remote divergence, failure after success and normalization. Do not
claim targeted edits eliminate the API's last-request race.

Verified https://developers.notion.com/guides/data-apis/working-with-markdown-content
and https://developers.notion.com/reference/update-page-markdown:
insert_content uses content plus position.type=start|end; update_content uses
content_updates[{old_str,new_str,replace_all_matches}] with
allow_deleting_content=false at update_content level. Both explicitly select v2
and allow_async=false. Strict response validation rejects warnings/partial bodies.
Append's exact authored preview joins existing/input text with two newlines;
normalization stays visible as dirty. A successful targeted write is journalled
before fresh readback; changed readback requires recovery/reconciliation rather
than automatically rebasing the local document.
