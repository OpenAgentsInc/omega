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
does not currently own Organization membership, so Omega leaves those buttons
disabled with an explicit explanation. It does not substitute the bootstrap
organization principal or turn a fixture selection into authority.

This packet is deterministic UI/model coverage. It cannot close omega#214.
The shared command/admission processor now exists, but the real close journey
still requires verified Organization membership, live bounded delegation and
revocation, independently inspectable Session, Agent Session, Activity, and
Run, and a separate human Owner Disposition in an installed Omega candidate.
