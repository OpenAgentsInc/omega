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
Effect service is available. The index stays read-only. The Work detail surface
can submit a typed Intent only when the named source supplies a matching
mutation capability. The source remains the authority.

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
stable selection, keyboard navigation, and accessible row labels. Press Enter
to open the source. Press Space or I, or select **Details**, to inspect the Work
without changing source identity. The surface shows loading, partial, offline,
error, conflict, and empty states explicitly. A source failure is never
rendered as successful emptiness.

Opening a Thread row returns to its durable Thread route. Opening a Forensics
row selects its repository-bound Forensics Work route. The detail surface
shows the exact authority, source reference, revision, cursor, freshness,
completeness, visibility, and accountability. See [Omega Work and Issue
detail](./omega-work-detail.md) for its mutation boundary.

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

## OAW-004 installed receipt {#oaw-004-installed-receipt}

The installed development build was produced from source commit
`17cb3268fb6def1f28c1c3920eae47baf3b831f5`, whose parent is
`1ec05dde41dbebb6edc5d10fa3230db4fee33ff1`. The release-fast build embeds the
same source commit in its `omega` executable.

- Application: `/Applications/Omega Dev.app`
- Bundle identifier: `com.openagents.omega.dev`
- Bundle version: `20260803.010149`
- Architecture: arm64
- Installed and bundled `omega` SHA-256:
  `ce3eb34e72e3526501f9bda43d4d67f30a4236b0d9af7491263665df35255c0d`
- DMG SHA-256:
  `7675e268aa0c768786847f39de4d8555783c44988db9250b7bd53f9c65adf5d9`
- CLI receipt: `Omega 0.2.0 – /Applications/Omega Dev.app`
- Signature receipt: ad hoc arm64 signature; `codesign --verify --deep
--strict` passed before and after installation.
- Recoverable previous development build:
  `/private/tmp/Omega-Dev-before-oaw004-20260802.app`

The exact source passed 9 Work Index model tests, both focused GPUI route
tests, all 314 delta checks, and release all-target lint for the affected
packages. The deterministic model and GPUI tests cover offline restoration,
reconnect, conditional navigation, keyboard operation, and route identity.
Installed-artifact verification used hashes, metadata, CLI output, embedded
source provenance, and code-sign checks only. It did not launch or control the
installed UI. `/Applications/Omega.app` was not changed.
