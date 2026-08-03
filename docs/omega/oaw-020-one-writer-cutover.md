# OAW-020 one-writer cutover

The internal Work writer cutover is an explicit state transition, not a side
effect of importing issues, rendering Omega, passing tests, or receiving a
signed relay message.

`omega_effectd::WorkCutoverLedger` starts in `LegacyGithub` shadow mode with an
exact source digest and cursor. Native activation requires the current ledger
revision and generation, the same reconciled source digest and cursor, an
explicit receipt reference, and a zero GitHub-write count. Activation advances
the generation and makes `NativeOmega` the only admitted internal Work writer.

Each admitted native write advances the native high-water cursor. Rollback is
not a blind writer flip: it requires another explicit receipt and proof that
the rollback reconciliation covers every native event through that high-water
cursor. A stale revision/generation, changed legacy source, GitHub write
attempt, repeated cursor, wrong writer, or incomplete reconciliation refuses
before mutation.

The public-safe ledger is stored through an atomic temporary-file replacement
with restrictive directory/file permissions and an `fsync` before rename. A
load validates schema, revision, generation, digest, writer, and receipt refs;
invalid or partial state never becomes an implicit shadow or native default.

This state machine does not activate the cutover and is not yet the complete
OAW-020 journey. Activation remains blocked until all issue dependencies close,
the imported graph and claims reconcile, policy/tooling refuse new internal
GitHub work, and the packaged two-client cutover/rollback run produces exact
receipts and zero-GitHub-write evidence. GitHub remains authoritative for Git
refs, pull requests, checks, code review, merge, and release transport.
