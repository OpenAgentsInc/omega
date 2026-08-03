# Omega All Work contract

Omega reads Work through the generated OpenAgents boundary. The Full Auto
adapter shows durable runs as redacted Work summaries and snapshots. The
planning adapter reads the persistent OpenAgents planning authority. Neither
adapter creates a second writable store.

## Immutable source

| Field                 | Value                                                              |
| --------------------- | ------------------------------------------------------------------ |
| OpenAgents commit     | `07e79366bbdf0ea3133ba799175c72e4d925cb17`                         |
| Contract              | `openagents.all_work_boundary.v1`                                  |
| Definition SHA-256    | `2f3119cf7822fe9bd770d2c97f913d609dd49e8918fb3ea4c7df9662d20492c4` |
| Rust artifact SHA-256 | `87fe5a88e669276672027464635f484a2a01de218424a2c063e8502602abe985` |
| Omega receipt         | `crates/omega_effectd/all-work-contract/SOURCE.json`               |

The generated Rust file, compatibility manifest, and shared fixtures are
vendored together. `script/sync-all-work-contract` refuses a source checkout
that is not at the pinned commit, then verifies the Rust digest before it
copies any file.

## Protocol

The process transport remains `openagents.omega.effectd.v1` so existing Omega
clients continue to initialize. Its initialization payload negotiates a
separate All Work profile:

- an omitted All Work request selects `omega-effectd.v1` with no Work methods;
- `omega-effectd.v2` enables `work.index.read`, `work.snapshot.read`,
  `planning.graph.read`, `repository.claim.read`, and separately negotiated
  claim execution, signed Workroom, and `work.command.execute` capabilities;
- a v1 client that calls any All Work read method receives
  `incompatible_version`;
- request and result domain payloads use generated Rust and Effect types;
- objective and done-condition text do not enter the Work Index;
- the planning graph retains its OpenAgents revision, event cursor,
  reconciliation digest, source coordinates, completeness, and loss facts.

The Effect adapter reads the Full Auto registry. That registry remains the
writable authority. Omega combines its generated summaries with separate
read-only native adapters for Threads and Forensics. Each adapter retains its
own source authority and cursor. The qualified rows feed
[Omega Work Index](./omega-work-index.md); they do not create a second writable
store.

The planning authority is Effect-owned state beneath the injected application
data root. Its final bootstrap GitHub reconciliation contains 28 open and six
closed Work rows. GitHub remains an imported read-only source. A successful
read does not grant command, claim, delegation, verification, release, or
owner-disposition authority. Release Planning Records remain planning metadata.

## Dogfood planning refresh

The development-only v0.2.0 Work screens use one
`DogfoodPlanningViewModel` for fixture and owned-client data. Opening either
dogfood Project asks the supervised `omega-effectd.v2` client for a generated
`PlanningGraphReadResult`. The adapter stages and validates the whole revision
before it swaps the visible graph.

The screen keeps the last complete revision when a refresh is partial,
truncated, has a cursor gap, regresses its revision or adapter generation, or
fails. It updates the visible freshness and projection-loss facts without
mixing rows from two revisions. A complete live revision is written through an
atomic last-known-good file replacement. A later offline start restores that
file with an explicit `Offline` state. Native Work without a GitHub coordinate
keeps its canonical Work identity and is not presented as a GitHub issue.
The client makes at most three refresh attempts with bounded backoff. The
Effect-owned graph has already reconciled source pagination; the Omega adapter
rejects duplicate canonical identities and normalizes display ordering instead
of creating a second pagination authority.

The persisted cache is read only when the existing debug fixture gate is
enabled. The refresh does not make this development route available in a
release build, and neither the cache nor a successful read grants command or
release authority.

## Repository claim authority

The generated boundary now includes Work Packets, Repository Work Claims,
claim audit events, reads, and commands. OpenAgents keeps the only persistent
claim ledger under the injected application data root. Omega does not copy
that ledger into its planning cache. The Work Inspector reads it through the
supervised generated client and submits create, claim, status, heartbeat,
block, and release commands through the same client.

Every command names an Effective Principal and the repository-claim write
capability, uses optimistic ledger revision and claim-generation fences, and
returns a receipt. Omega refuses a receipt whose `githubWriteCount` is not
zero. A Repository Work Claim is collision coordination only. It is not an
Assignee, Agent Delegate, Lease, process-health proof, verification, merge,
release, or owner disposition.

Collision policy covers same Work, paths, hot files, hot contracts, generated
artifacts, migrations, route tables, lockfiles, and shared schemas. A claim
that has no evidence for 90 minutes is still owned until a named process and
worktree audit also proves abandonment. Imported GitHub claim comments remain
canceled historical packets and audit records; they cannot become native
post-cutover commands. See
[Omega repository work claims](./omega-repository-work-claims.md).

The generated profile also includes signed Workroom activity reads and a
persist-before-publish enqueue boundary. Signed identity, audience, causal
parents, generation, outbox state, and revocation remain projection facts.
They do not become command or effect authority. See
[Omega signed Workroom projection](./omega-signed-workroom.md).

## Work command admission

The generated profile includes `work.command.execute`. The Rust supervisor
validates the request and result, requires the negotiated capability, and
refuses any receipt that reports a GitHub write. The OpenAgents process remains
the command authority. Omega does not infer command authority from a planning
read or recreate command transitions in Rust.

Each request names the Organization, Effective Principal, capability, expected
Work revision, Intent, and idempotency key. Assignee, Agent Delegate,
Delegation Grant, Repository Work Claim, Lease, Thread, Session, Agent Session,
Run, provider event, loss, effect, review, verification, and Owner Disposition
remain separate facts. Revocation fences late generation effects. Verification
does not imply Owner Disposition.

The Work inspector recovers the durable command snapshot and uses its exact
revision for generated assign, unassign, delegate, and revoke requests. A
delegate candidate must be the exact owner of the active local Thread; a legacy
ambiguous owner or remote Thread without an admitted Host reference is not a
candidate. The grant names that Direct Agent, local Host, private scope,
bounded Thread capabilities, one-hour expiry, positive budget ceiling, and an
evidence requirement. It does not invent a Repository Work Claim or Lease.

The inspector enables command controls only when the displayed account supplies
both an enrolled Effective Principal and verified Organization membership. The
current account registry does not own Organization membership, so the controls
fail closed and explain that dependency. Fixture controls remain simulated.
Starting and controlling a real Agent Session, recording portable activity,
and the installed owner journey remain incomplete.

## Verify

Run:

```sh
cargo test -p omega_effectd typed_all_work_reads_cross_the_supervised_process_boundary
cargo test -p omega_effectd all_work
cargo test -p omega_work_index planning_refresh --features test-support
OPENAGENTS_ALL_WORK_SOURCE_ROOT=/path/to/pinned/openagents \
  cargo test -p omega_effectd typed_all_work_index_crosses_the_openagents_process_boundary \
  -- --ignored
./script/clippy -p omega_effectd
```

The OpenAgents `omega-effectd` package also runs an actual child-process test
against its TypeScript binary. The Omega test uses the Rust supervisor and a
newline-framed child process to verify negotiation, Work Index decoding, and
Work snapshot decoding. The ignored cross-repository smoke starts that actual
OpenAgents TypeScript process through the Omega Rust supervisor; it requires
the immutable source checkout named above and also verifies all 34 planning
rows through the generated `PlanningGraphReadResult`.
