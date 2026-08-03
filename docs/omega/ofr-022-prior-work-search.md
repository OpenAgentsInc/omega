# OFR-022 forensic prior-work search

Issue: [Omega #229](https://github.com/OpenAgentsInc/omega/issues/229)  
Parent: [Omega #208](https://github.com/OpenAgentsInc/omega/issues/208)  
Canonical service delivery: OpenAgents `b967618635`; durable cursor replay
follow-up `2fc1ca8257`

## Delivered boundary

OpenAgents owns durable forensic prior Work. Omega owns a typed client, a visible
case-workspace projection, and discovery continuation receipts. The UI does not
become a second history database.

The OpenAgents authority stores data under
`all-work/forensic-prior-work.v1.json` and exposes owner-local
`omega-effectd.v2` framed methods:

- `forensics.prior_work.query`
- `forensics.prior_work.submit`
- `forensics.prior_work.relate`
- `forensics.prior_work.dispose`

The daemon boundary is owner-local. Query principals and organization refs are
trusted only after the caller enters that local supervised boundary. The
authority filters records before matching or counting them. An unauthorized
query therefore returns no private content, existence signal, population count,
or match timing.

## Identity and history

An occurrence identity binds repository, revision, path, optional symbol, line
range, and source-window digest. A root-cause identity binds the normalized
causal mechanism, affected behavior, mechanism class, and security boundary.
Path and source-window similarity are signals, not root-cause authority.

All nine retained dispositions remain searchable: confirmed, dismissed,
rejected, inconclusive, expired, superseded, corrected, duplicate, and retained.
Relations and dispositions append events. They do not rewrite the original Work.
The relation vocabulary covers duplicate, related, supersedes, split-from, and
merged-into, with exact, probable, or possible confidence.

## Omega projection

`omega_forensics` mirrors the versioned wire models and validates identities,
audiences, queries, results, and completeness receipts. `omega_effectd` exposes
typed Rust supervisor methods for all four service operations. The Forensics
Case workspace has a **Prior forensic Work** region that searches the canonical
service and renders:

- authorized records searched and returned;
- complete or partial population state and explicit loss count;
- stable query receipt reference;
- causal mechanism, occurrence count, Work references, and match score.

The shared Work detail adapter carries the receipt, related Work refs, evidence
refs, root cause, occurrence count, disposition, completeness, and loss facts.

## Duplicate continuation

`continue_ranked_forensic_discovery` queries each ranked candidate and submits an
append-only record in both cases. A known root cause becomes a duplicate
disposition/relation in the canonical authority. The same task then advances to
the next candidate. Its receipt preserves candidate rank, exact query, returned
Work refs, query receipt, decision, resulting record, decision time, and time to
the next candidate.

This behavior prevents a known first-ranked defect from hiding a new
lower-ranked defect.

## Verification receipt

- OpenAgents All Work: 59 tests passed.
- OpenAgents `omega-effectd`: 269 tests passed, including durable framed-server
  restart and audience isolation.
- OpenAgents full pre-push policy gate: green.
- Omega `omega_forensics`: 53 tests passed, including cross-file/revision
  identity, all-disposition round trip, inconsistent audience rejection, and
  duplicate-first continuation.
- Omega `omega_effectd`: `cargo check` passed as part of the scoped check.
- The ignored real-process interop test passed against the OpenAgents source:
  Omega started the Node daemon, submitted prior Work, queried it by stable Work
  ref, and decoded its complete receipt.
- The full `agent_ui` check remains blocked by pre-existing errors introduced in
  `omega_dogfood_surface.rs` at Omega `414b285466` and one unrelated private
  `ThreadId` access in `agent_panel.rs`. The compiler reported no error in the
  OFR-022 Forensics files before reaching those baseline failures.

No installed application build is part of this issue. The parent sequence keeps
the requested aggregate build for the final forensic batch.
