# OAW-020 one-writer cutover

The internal Work writer cutover is an explicit state transition, not a side
effect of importing issues, rendering Omega, passing tests, or receiving a
signed relay message.

The generated `work.cutover.read` and `work.cutover.execute` boundary reads and
transitions the OpenAgents-owned Effect ledger. That ledger starts in
`legacy_github` shadow mode with an exact source digest and cursor. Native
activation requires the current ledger revision and generation, the same
reconciled source digest and cursor, an explicit receipt reference, the
authorized Effective Principal and capability, and a structurally zero GitHub-
write count. Activation advances the generation and makes `native_omega` the
only admitted internal Work writer.

Each admitted native write advances the native high-water cursor. Rollback is
not a blind writer flip: it requires another explicit receipt and proof that
the rollback reconciliation covers every native event through that high-water
cursor. A stale revision/generation, changed legacy source, GitHub write
attempt, repeated cursor, wrong writer, or incomplete reconciliation refuses
before mutation.

The OpenAgents service stores the public-safe ledger through an atomic
temporary-file replacement with restrictive permissions. Omega vendors the
digest-bound generated Rust contract, negotiates both methods, validates every
request/result, and rejects any receipt with a nonzero GitHub-write count. The
former Omega-local state machine is removed; Omega is now a client, not a
second cutover authority.

This state machine does not activate the cutover and is not yet the complete
OAW-020 journey. Activation remains blocked until all issue dependencies close,
the imported graph and claims reconcile, policy/tooling refuse new internal
GitHub work, and the packaged two-client cutover/rollback run produces exact
receipts and zero-GitHub-write evidence. GitHub remains authoritative for Git
refs, pull requests, checks, code review, merge, and release transport.
