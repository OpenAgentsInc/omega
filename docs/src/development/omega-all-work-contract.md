# Omega All Work contract

Omega reads Work through the generated OpenAgents boundary. The Full Auto
adapter shows durable runs as redacted Work summaries and snapshots. The
planning adapter reads the persistent OpenAgents planning authority. Neither
adapter creates a second writable store.

## Immutable source

| Field                 | Value                                                              |
| --------------------- | ------------------------------------------------------------------ |
| OpenAgents commit     | `ffee612faa7a4b330a0cbb30de959114874bfb57`                         |
| Contract              | `openagents.all_work_boundary.v1`                                  |
| Definition SHA-256    | `df850124a53029e549f4f8281c77ee9d8c583e617d2adcc36f0be7abe8bca654` |
| Rust artifact SHA-256 | `ecb0ebf3f237a48b50f76b54c2785abcb76e041145c47fce273f22af5f28faa7` |
| Omega receipt         | `crates/omega_effectd/all-work-contract/SOURCE.json`               |

The generated Rust file, compatibility manifest, and shared fixtures are
vendored together. `script/sync-all-work-contract` refuses a source checkout
that is not at the pinned commit, then verifies the Rust digest before it
copies any file.

The current pin includes the generated TypeScript client contract and the
implemented Work Index subscription request and event variants. Omega vendors
the matching Rust artifact and negotiates the subscription capability; the
OpenAgents compatibility manifest also pins the TypeScript client digest.

## Protocol

The process transport remains `openagents.omega.effectd.v1` so existing Omega
clients continue to initialize. Its initialization payload negotiates a
separate All Work profile:

- an omitted All Work request selects `omega-effectd.v1` with no Work methods;
- `omega-effectd.v2` enables `work.index.read`, `work.index.subscribe`,
  `work.snapshot.read`, `planning.graph.read`, `repository.claim.read`, and
  separately negotiated claim execution, signed Workroom
  read/prepare/commit/delivery/publication, Work command, and Work cutover,
  Organization membership read, and strict bug candidate capabilities;
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
prepare/commit boundary in front of the persist-before-publish outbox. The
OpenAgents authority fixes the canonical unsigned NIP-01 bytes, event identity,
revision, generation, expiry, and server relay-policy digest before Omega asks
the selected enrolled identity to sign. Commit accepts only that exact
preparation and signed event. Signed identity, audience, causal
parents, generation, outbox state, and revocation remain projection facts.
They do not become command or effect authority. See
[Omega signed Workroom projection](./omega-signed-workroom.md).

## Internal Work writer cutover

The generated profile includes `work.cutover.read` and
`work.cutover.execute`. OpenAgents owns the only persistent writer ledger;
Omega validates the generated request/result and refuses a receipt with a
nonzero GitHub-write count. The ledger starts in `legacy_github` shadow mode.
Import, startup, tests, and UI rendering cannot activate it.

Native activation requires the exact shadow digest and cursor, current
revision and generation, authorized Effective Principal and capability, and an
explicit receipt reference. Native events advance a retained high-water
cursor. Rollback requires exact reconciliation through that cursor. The
vendored boundary does not activate the writer switch; installed journey and
policy/tooling gates remain required.

## Strict bug candidate ingress

The generated profile includes `strict_bug.candidate.read` and
`strict_bug.candidate.execute`. The Rust supervisor validates each generated
request and result and refuses a receipt that reports a GitHub write. A strict
public GitHub bug enters the OpenAgents ledger as an untrusted, pending
candidate. Only a separate owner-local triage command can reject it or link it
to canonical Work. That link does not grant Work command authority.

Omega does not verify GitHub webhook signatures. The production transport must
verify the delivery first and supply the public-safe verification evidence ref
to the OpenAgents authority. The vendored client boundary is not a claim that
the production webhook or candidate inbox is installed.

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
account registry does not own Organization membership. The production account
poll now queries the separate Effect-owned ledger through
`organization.membership.read`, fenced by the exact account reference,
generation, and Effective Principal. The authority starts empty and only an
explicit owner-local provisioned row can enable Organization-dependent
controls. Fixture controls remain simulated.

After delegation, **Link session** admits the exact active local Omega Thread
only when it has a real ACP session and its Direct Agent is the admitted Agent
Delegate. The command creates separate OpenAgents Thread, Session, Agent
Session, and Run references at the active grant generation; it does not start a
provider process. **Record handoff** records only the exact link fact as
portable progress when no admissible provider event is available. If the exact
retained ACP session has an agent/tool projection, the control becomes **Record
provider event** and attaches its stable session-digested entry/revision ref.
The portable kind and summary contain only generic event/status facts; reasoning
and user/system events are excluded. Omega does not copy the provider-native
payload into Work and records
`loss:omega:provider-native-payload-not-projected` explicitly. Neither path
supplies an Effect. Real execution-effect receipts and the installed owner
journey remain incomplete.

The snapshot also carries additive full Session and portable Agent Activity
projections beside the compatibility ref arrays. A Session binds its Thread,
Agent Session, Run, Delegation Grant, Host, generation, and exact
active/paused/stopped/revoked lifecycle. An activity binds its Session, Run,
generation, portable kind and summary, provider event, explicit projection
losses, and nullable Effect. Omega rejects duplicate, zero-generation,
cross-ref, or activity-to-session mismatches before displaying the result.
This makes restart and revocation state inspectable without treating a provider
event or Effect reference as a Receipt, verification, or owner acceptance.

**Revoke delegate** uses that durable state as the authority fence. When the
latest active or paused Session names the exact retained local Thread, grant,
and generation, Omega first asks the existing ACP path to cancel that Thread if
it is still generating. A missing, closed, stale, or unparsable local runtime
does not block the canonical revoke command: the Session becomes revoked and
late commands at its generation remain fenced. Process cancellation and
durable revocation are not one atomic Effect, and Omega does not claim an
Effect Receipt for either operation.

When the exact retained Thread is generating, **Stop agent** first calls
Omega's existing local ACP cancel path and only then records the generated
Session `stop` transition at the active grant generation. If the Thread is not
open or not generating, no Work command is sent. A crash between cancellation
and command persistence can leave a visible stopped-agent/pending-record gap;
refresh and idempotent retry reconcile the command, and the UI does not claim an
atomic external Effect Receipt.

After portable activity exists, **Needs changes** records a human Owner
Disposition separately from the Agent Delegate, Session state, activity, and
any verification refs already attached to the Work. It never converts agent
completion or the stop record into human acceptance.

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
