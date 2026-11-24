# Data model

`Document`: UUID string ID, title, URL, remote `last_edited_time`, current Markdown,
baseline Markdown. Dirty means current Markdown differs from the baseline.
One serialized JSON record per canonical UUID key in sled's documents tree.

`RemoteDocument`: metadata, Markdown snapshot and completeness diagnostics.
Incomplete reads remain useful for local reading but are rejected for pushing.
Metadata reads bracket Markdown retrieval so an edit during retrieval is rejected.

Workspace: `.quillship/` by default, configurable via `--workspace` or
`QUILLSHIP_WORKSPACE`; `store/` is sled, `docs/<uuid>.md` is the edit interface.
No title/frontmatter rewriting: files contain exact Notion-flavored Markdown.
Read-only metadata remains in sled and JSON CLI output.

Repair: documents also store `incomplete` (default false for existing records)
and `sync_warnings` (default empty). Incomplete describes the saved baseline;
warnings may also describe a newer partial response whose body was not adopted.
Either condition blocks push. A later complete pull can clear the warnings and
promote a clean partial document. A complete baseline is never downgraded by a
partial response. Remote snapshots carry the same diagnostics as warnings.
