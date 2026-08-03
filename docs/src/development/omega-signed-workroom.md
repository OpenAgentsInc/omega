# Omega signed Workroom projection

Omega consumes the generated signed Workroom boundary from OpenAgents commit
`b0ff32b73ff61cccfca8107ac8984252371c9e5f`. The contract definition SHA-256
is `513432ed4d7deee1a8511f59b86cc1958d33cc32b76b314ebb0b901500dfb56d`.

The Rust supervisor negotiates `workroom.activity.read`,
`workroom.activity.enqueue`, the transport-only `workroom.activity.deliver`
reducer, and the authority-owned `workroom.activity.publish` operation. The
development Work screen reads the
selected Work identity and renders signed history in its Session scene. It
shows actor, signer, signed projection profile, direct or purpose-bound actor
grant and grant generation, audience, generation, causal-parent count, event time,
pending/partial/failed/accepted delivery state, the exact accepted relay count,
and the newest bounded delivery attempts. Empty and unavailable states remain
explicit.

The OpenAgents authority now recomputes the deterministic NIP-01 event ID,
verifies the BIP-340 Schnorr signature, and requires a direct actor to equal
`principal:nostr:<signer-pubkey>`. New writes use signed projection v2. Agent,
device, and organization actors require an authoritative active grant that
binds the signer, actor, exact Workroom/Work, allowed kinds, audience/privacy,
evidence, validity interval, and generation. Missing, expired, revoked,
superseded, stale, or scope-mismatched facts fail closed, and publication
rechecks the current grant before opening a socket. It revalidates canonical
and outbox records when durable state loads and before replay or append. Schema-valid signature
substitution therefore cannot enter the Omega read.

Signature proves signer and serialized projection bytes only, including the
payload digest. It does not prove undisclosed payload content, membership, or a
grant. Audience/privacy use a closed profile. The canonical record enters a
durable pending outbox before relay publish. Relay acceptance is transport
evidence, not command admission or effect completion. A typed internal
delivery request can record accepted, rejected, or unreachable facts only for
configured relay targets. It retains exact attempt history and supports optimistic idempotent
retry. The OpenAgents Effect publisher sends the exact persisted signed event
only to those WSS targets and accepts only a matching relay `OK` frame. Omega
exposes a bounded publish/retry control only for an existing pending, partial,
or failed outbox row. The request supplies the existing event identity and
Effective Principal; relay targets, attempts, and outcomes remain server-owned.
Omega validates that the receipt names the same event and asserts neither relay
authority nor an admitted effect. It never fabricates a delivery attempt from
the UI. Evidence and verification references
do not become verification, owner acceptance, merge, or release authority.
Supersession and revocation advance a generation and retain prior history.

The screen remains behind the dogfood development gate. No production Workroom
navigation is exposed. Enqueue has no direct UI control until identity
enrollment and authoritative grant provisioning are complete. The publish
control cannot create or sign an event, and accepted, superseded, and revoked
rows have no retry action.
Test execution and the installed two-client journey remain deferred to the
final omega#208 gate.
