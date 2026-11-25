# Operation recovery plan
US3 follows the initial review/organisation tranche.

CLI: operations [--pending] --json; operation ID --json; recover ID [--json].
The recovery command observes state and saves a verified outcome; it does not
resubmit an uncertain non-idempotent request. Reissuing an action is a separate
explicit user command after inspection.

Persist JournalEntry in operations-v1: monotonic id, action, target/parent,
authored input, expected baseline/metadata, timestamps, state and safe response
identity. States: prepared -> sent -> remote-confirmed -> locally-saved, or
uncertain/failed. Flush each transition before the next irreversible step.
Prepare before requests; sent marks that remote outcome might be unknown.
Remote-confirmed is durable before readback. Do not store headers, tokens or
signed download URLs. Retain completed records until explicit local pruning.

Use domain-specific intent payloads, not arbitrary API calls. Integrate all
implemented mutations and per-item batch statuses. No exactly-once claim; an
uncertain create with no returned identity requires user inspection in Notion.

Tests: restart after every transition; failed journal flush prevents writes;
remote success/local failure retains identity/input; no automated resubmission;
batch partial success resumes observation without duplicate earlier mutations.
