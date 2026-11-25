# Review and organisation quickstart

Use the feature worktree's Nix shell. Choose a disposable local workspace for
experiments. Commands below use generic page identities, not real documents.

1. Pull a shared page with `quillship pull PAGE_UUID` using the normal secret
   environment. Page links can replace page UUIDs.
2. Edit the path printed by `quillship path PAGE_UUID`.
3. Run `quillship diff PAGE_UUID` offline. JSON exposes exact bodies.
4. Review divergence with `quillship diff PAGE_UUID --remote --json`.
5. Resolve intentionally with `quillship reconcile PAGE_UUID --file merged.md`,
   `--take local`, or `--take remote`. No remote content is changed.
6. List retained inputs with `quillship snapshots PAGE_UUID`; export an original
   with `quillship snapshot SNAPSHOT_ID --side local`. Sides baseline, remote and
   selected are also available. A baseline-only snapshot has no remote side.
7. Inspect the resulting diff, then explicitly `quillship push PAGE_UUID`.
8. After normalization, `quillship reconcile PAGE_UUID --take baseline` accepts
   the saved complete formatting offline while retaining the original body.

For organisation, use `inspect PAGE_UUID --json`, `rename PAGE_UUID --title TITLE`,
or `move PAGE_UUID --parent PARENT_UUID`. Metadata operations preserve body edits
and baselines. `inspect PAGE_UUID --offline --json` reads cached metadata.

Discover actual child pages with `tree PAGE_UUID --max-depth 8 --max-blocks 10000
--json`. Pull the discovered set with `pull PAGE_UUID --recursive`. Partial
discovery produces partial output and a nonzero exit; increase a deliberate limit
or resolve the reported access problem before assuming coverage is complete.

Run `nix develop -c cargo test`, `nix develop -c cargo fmt --check`, and
`nix develop -c cargo clippy --all-targets -- -D warnings` for mock-only validation.
Journal, comments, targeted edits, data sources and files are implemented; use the
README command examples and workstreams for their contracts and documented limits.
The current complete workflow is validated with temporary workspaces/mock servers;
no live writes are performed by tests.
