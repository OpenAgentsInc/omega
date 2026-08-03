# Omega signed Workroom projection

Omega consumes the generated signed Workroom boundary from OpenAgents commit
`1a7d624c5ee87810e2ed6c70249878bad2ddbfa2`. The contract definition SHA-256
is `2797ea64aba30c5ccc19cf2f4771d47955b126a3d91576f24409ad2d23f47a03`.

The Rust supervisor negotiates `workroom.activity.read` and
`workroom.activity.enqueue`, plus the transport-only
`workroom.activity.deliver` reducer. The development Work screen reads the
selected Work identity and renders signed history in its Session scene. It
shows actor, signer, audience, generation, causal-parent count, event time,
pending/partial/failed/accepted delivery state, the exact accepted relay count,
and the newest bounded delivery attempts. Empty and unavailable states remain
explicit.

The OpenAgents authority now recomputes the deterministic NIP-01 event ID,
verifies the BIP-340 Schnorr signature, and requires a direct actor to equal
`principal:nostr:<signer-pubkey>`. It revalidates canonical and outbox records
when durable state loads and before replay or append. Schema-valid signature
substitution therefore cannot enter the Omega read.

Signature proves signer and serialized projection bytes only, including the
payload digest. It does not prove undisclosed payload content, membership, or a
grant. Audience/privacy use a closed profile. The canonical record enters a
durable pending outbox before relay publish. Relay acceptance is transport
evidence, not command admission or effect completion. A typed delivery request
can record accepted, rejected, or unreachable facts only for configured relay
targets. It retains exact attempt history and supports optimistic idempotent
retry, but it is not a network publisher. Evidence and verification references
do not become verification, owner acceptance, merge, or release authority.
Supersession and revocation advance a generation and retain prior history.

The screen remains behind the dogfood development gate. No production Workroom
navigation is exposed. Enqueue and delivery have no direct UI control until
identity enrollment, capability inspection, purpose-bound non-human actor
grants, a real publisher, outage state, and installed acceptance are complete.
Test execution and the installed two-client journey remain deferred to the
final omega#208 gate.
