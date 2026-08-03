# Omega Work Index

Omega projects source-owned Work into two read-only views: **Inbox** and
**My Work**. The index does not become a writable Work authority.

## Production admission {#production-admission}

Omega omits both navigation items until at least two independent adapters have
qualified, nonempty rows. An empty adapter, a loading lane, and a synthetic
fixture do not satisfy this gate.

The first native adapters are:

- `omega.thread-metadata.v1` for durable Threads;
- `omega.forensics-workbench.v1` for repository-bound Forensics cases and
  runs.

`openagents.omega-effectd.v2` adds generated All Work summaries when the
Effect service is available. The source authority remains writable only at the
source. Omega's projection and detail surface are read-only.

## Preserved source facts {#preserved-source-facts}

Every row uses the generated `WorkSummary` type. Omega preserves the exact
Work reference, source reference, source authority, adapter version, revision,
cursor, freshness, completeness, gap references, and visibility metadata.
The index rejects invalid generated values and quarantines conflicting Work
identities.

The native adapters map only observed source state. They do not invent a
completed item, assignment, delegation, or priority. Inbox groups questions,
recoverable work, blockers, failures, and stale work. My Work distinguishes
local ownership, assignment, participation, and bounded agent delegation.

The identity rules are explicit. A Thread maps to
`work:omega:thread:<thread-ref>`. A Forensics case maps to
`work:omega:forensics:<case-ref>`, and its run maps to
`work:omega:forensics-run:<run-ref>`. Effect-backed rows must repeat their
generated Work reference in the adapter envelope. A source/entity mismatch is
rejected before it enters the index.

## Refresh and offline behavior {#refresh-and-offline-behavior}

Each adapter refreshes in a staging lane. A paginated result replaces the
qualified lane only after its final page passes validation. A failed adapter
does not erase the last qualified rows or rows from another adapter.

Omega stores the qualified snapshot under its application data directory in
`work-index-v1/snapshot.json`. Restart restoration marks cached lanes as
offline. It preserves the last selection and resume cursor without changing
the source summary's freshness claim. Reconnect resumes from the stored cursor
and rejects gaps, non-advancing cursors, and conflicting identities.

## Interface {#interface}

Inbox and My Work have search, attention and lifecycle filters, grouped rows,
stable selection, keyboard navigation, and accessible row labels. The surface
shows loading, partial, offline, error, conflict, and empty states explicitly.
A source failure is never rendered as successful emptiness.

Opening a Thread row returns to its durable Thread route. Opening a Forensics
row selects its repository-bound Forensics Work route. Other admitted Work
opens a read-only detail with its authority, source reference, revision,
cursor, freshness, completeness, visibility, and accountability.

## Verification {#verification}

Run:

```sh
cargo test -p omega_work_index
cargo test -p agent_ui omega_entity_routes_render_and_navigate_without_thread_identity_leaks --lib
./script/clippy -p omega_work_index -p omega_workbench_state -p agent_ui
```

The model suite covers two-authority admission, adapter-failure isolation,
cursor and gap rejection, identity-conflict quarantine, offline restoration,
selection restoration, repeated-page deduplication, and 10,000-row query
performance. The GPUI test proves conditional navigation and both production
views without controlling an installed application UI.
