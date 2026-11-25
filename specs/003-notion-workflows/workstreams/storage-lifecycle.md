# Bounded lock wait

Full validation intermittently fails immediate reopen after Store drop, including
unchanged persistence/create tests. The same checks passed in intervening runs.
Targeted-edit tests pass independently. No live workspace is involved.

Installed sled 0.34.7 source shows RunningConfig holds an Arc<File>, and
pagecache/iobuf.rs maybe_seal spawns background writes retaining Arc<IoBufs>.
An operation can report stable persistence while a worker still owns its final
handle briefly. This provides a plausible cause for the observed short lock race;
it is not evidence of a corruption or remote-write failure. Package indexing was
attempted via agentic-nix but failed in its embedding backend; the exact installed
source was available and inspected independently.

Approved by the user's "retry" reply; implemented without changing storage:

- Keep sled, the existing record format, locking and explicit flushes.
- In Store::open only, retry lock contention. Sled 0.34.7 wraps the underlying
  WouldBlock in an Other error; recognise only its specific acquire-lock/OS-error
  wrapper, as well as a direct ErrorKind::WouldBlock. Other errors fail immediately.
- Use a monotonic deadline of 250 ms with short bounded delays.
- Return other errors immediately; never delete lock files, unlock another
  process's handle, retry mutations or bypass one-process-per-workspace locking.
- On sustained contention, retain the existing actionable open error.
- Verify rapid close/reopen with real stores and verify a second holder still
  fails within a bounded duration; test injected non-lock errors are not retried.

Real held-lock validation checks both the 250 ms wait and eventual rejection.
Repeated reopen verifies journal contents remain readable.
