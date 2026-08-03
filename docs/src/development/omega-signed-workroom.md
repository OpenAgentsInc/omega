# Omega signed Workroom projection

Omega consumes the generated signed Workroom boundary from OpenAgents commit
`c82cbe7394f68615e3a3a189b07037c1204f2d83`. The contract definition SHA-256
is `e504e3084007e8bddab99e9703f1f62c6bf62e1aa7a9f612de75fb522848b628`.

The Rust supervisor negotiates `workroom.activity.read` and
`workroom.activity.enqueue`. The development Work screen reads the selected
Work identity and renders signed history in its Session scene. It shows actor,
signer, audience, generation, causal-parent count, and event time. Empty and
unavailable states remain explicit.

Signature proves signer and bytes only. Audience/privacy use a closed profile.
The canonical record enters a durable pending outbox before relay publish.
Relay acceptance is transport evidence, not command admission or effect
completion. Evidence and verification references do not become verification,
owner acceptance, merge, or release authority. Supersession and revocation
advance a generation and retain prior history.

The screen remains behind the dogfood development gate. No production Workroom
navigation is exposed. Enqueue has no UI control until identity enrollment,
capability inspection, signature verification, outage state, and installed
acceptance are complete. Test execution and the installed two-client journey
remain deferred to the final omega#208 gate.
