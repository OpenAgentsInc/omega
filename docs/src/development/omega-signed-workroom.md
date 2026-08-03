# Omega signed Workroom projection

Omega consumes the generated signed Workroom boundary from OpenAgents commit
`2630b1d38393df7b5cd9a10cd12584b604b49351`. The contract definition SHA-256
is `012a5962bc2bea70792014f5b375d78b7afef2bee15408489a729278c641508b`.

The Rust supervisor negotiates `workroom.activity.read`,
`workroom.activity.prepare`, `workroom.activity.commit`, the legacy internal
`workroom.activity.enqueue`, the transport-only `workroom.activity.deliver`
reducer, and the authority-owned `workroom.activity.publish` operation. The
development Work screen reads the
selected Work identity and renders signed history in its Session scene. It
shows actor, signer, signed projection profile, direct or purpose-bound actor
grant and grant generation, audience, generation, causal-parent count, event time,
pending/partial/failed/accepted delivery state, the exact accepted relay count,
and the newest bounded delivery attempts. Empty and unavailable states remain
explicit.

The OpenAgents authority prepares the exact canonical unsigned NIP-01 JSON,
event identity, next revision and generation, five-minute expiry, and a digest
of its configured relay policy. Omega cannot supply relay targets. It verifies
that the prepared direct actor and signer equal the selected enrolled account,
authorizes a durable `workroom_activity` identity action, and signs through the
existing local-custody or NIP-46 route. Remote custody must explicitly allow
the prepared signed Workroom kind (32150 through 32163); an older capability
that does not declare that kind fails closed. Omega rechecks the selected
account, identity action, signature, and all prepared event fields after
custody returns.

Commit sends the unchanged preparation and signed JSON back to OpenAgents. The
authority recomputes the preparation reference, current relay-policy digest,
event ID, and signature before it writes the canonical activity and pending
outbox row. The UI accepts the commit result only when it names the prepared
event, reports persistence before publication, and denies relay authority and
admitted effect. A relay failure after commit keeps and displays the durable
outbox row for retry.

The OpenAgents authority also recomputes the deterministic NIP-01 event ID,
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
navigation is exposed. Its **Sign checkpoint** control creates only a direct
actor `thread` checkpoint for the selected v0.2.0 Work item, with a public-safe
digest and the newest activity as its causal parent. It does not accept
undisclosed note content, infer completion, create an agent grant, verify Work,
merge, release, or record Owner Disposition. The separate publish/retry control
still cannot create or sign an event, and accepted, superseded, and revoked rows
have no retry action.
Test execution and the installed two-client journey remain deferred to the
final omega#208 gate.
