# OAW-007 Agent Session simulation

The v0.2.0 dogfood Session screen includes a deterministic Packet C simulator
only when the existing development/mock fixture gate admits the whole planning
surface. It cannot appear in normal, candidate, or release builds.

The simulator provides Pending, Active, Input, Error, Stale, Complete, Diff,
and Review scenes. Each scene derives stable simulated references from the
selected Work identity. It displays Human Assignee, Agent Delegate, Delegation
Grant, Repository Work Claim, Lease, Thread, Session, Agent Session, Run, Host,
generation, plan, activity, and effect as separate facts.

Every projection is marked `simulation: true` and `ephemeral: true`. Claim and
Lease references end in `not-live`; Receipt and Owner Disposition are always
absent. An effect reference means only that the selected scene depicts an
ephemeral effect. It is not command admission, evidence, verification, a
release fact, or an external action.

The Session screen also retains the signed Workroom reader. Signed activity and
the simulation are separate: a signature authenticates signer and bytes, while
the simulator grants nothing.

The separate Issue inspector now reads the durable canonical command snapshot
through `work.snapshot.read`. It uses the returned Work revision for generated
assign and unassign requests. Live buttons require the displayed enrolled
Effective Principal and a verified Organization reference. The account registry
does not own Organization membership. The production poll reads the separate
Effect-owned membership ledger for the exact account generation and principal;
the empty-by-default authority leaves these buttons disabled until an explicit
verified row exists. Omega does not substitute the bootstrap organization
principal or turn a fixture selection into authority.

For a linked real ACP session, the Issue inspector can project the latest
admissible agent/tool event into canonical Agent Activity. The event reference
is bound to the exact session digest, stable entry identity, and projection
revision. Reasoning, user, and system entries never become Work activity. Only
generic kind/status facts cross the boundary, with an explicit loss ref for the
provider-native payload that Work does not copy. This projection grants no
Effect, Receipt, verification, completion, or Owner Disposition authority.

This packet is deterministic UI/model coverage. It cannot close omega#214.
The shared command/admission processor now exists, but the real close journey
still requires an explicitly provisioned Organization membership, live bounded
delegation and revocation, independently inspectable Session, Agent Session,
Activity, and Run, real Effect evidence, and a separate human Owner Disposition
in an installed Omega candidate.
