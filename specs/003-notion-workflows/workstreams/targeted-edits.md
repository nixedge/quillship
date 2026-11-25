# Targeted Markdown editing plan
US5 depends on operation recovery.

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
