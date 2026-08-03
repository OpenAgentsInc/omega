# Omega signed Workroom projection

Omega consumes the generated signed Workroom boundary from OpenAgents commit
`f84fc2ea958169db59109c71c43d4c526aae4d1b`. The contract definition SHA-256
is `818f7960246a0811d8193523afd83197183f9defccf507e31991624f65f7d1e8`.

The Rust supervisor negotiates `workroom.activity.read` and
`workroom.activity.enqueue`, plus the transport-only
`workroom.activity.deliver` reducer. The development Work screen reads the
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
rechecks the current grant before opening a socket. It revalidates canonical and outbox records
when durable state loads and before replay or append. Schema-valid signature
substitution therefore cannot enter the Omega read.

Signature proves signer and serialized projection bytes only, including the
payload digest. It does not prove undisclosed payload content, membership, or a
grant. Audience/privacy use a closed profile. The canonical record enters a
durable pending outbox before relay publish. Relay acceptance is transport
evidence, not command admission or effect completion. A typed delivery request
can record accepted, rejected, or unreachable facts only for configured relay
targets. It retains exact attempt history and supports optimistic idempotent
retry. The OpenAgents Effect publisher sends the exact persisted signed event
only to those WSS targets and accepts only a matching relay `OK` frame. Omega
does not expose an enqueue or publish control until that publisher has a
generated supervised host method; it never fabricates a delivery attempt from
the UI. Evidence and verification references
do not become verification, owner acceptance, merge, or release authority.
Supersession and revocation advance a generation and retain prior history.

The screen remains behind the dogfood development gate. No production Workroom
navigation is exposed. Enqueue and delivery have no direct UI control until
identity enrollment, authoritative grant provisioning, a supervised publisher
method, outage state, and installed acceptance are complete.
Test execution and the installed two-client journey remain deferred to the
final omega#208 gate.
