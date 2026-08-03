# Omega Work Index

Omega projects source-owned Work into two read-only views: **Inbox** and
**My Work**. The index does not become a writable Work authority.

## Production admission {#production-admission}

Omega omits both navigation items until at least two independent adapters have
qualified, nonempty rows. An empty adapter, a loading lane, and a synthetic
fixture do not satisfy this gate.

The first native adapters are:

- `omega.thread-metadata.v1` for durable Threads;
- `omega.forensics-workbench.v1` for repository-bound Security cases and
  source-owned runs.

`openagents.omega-effectd.v2` adds generated All Work summaries when the
Effect service is available. The index stays read-only. The Work detail surface
can submit a typed Intent only when the named source supplies a matching
mutation capability. The source remains the authority.

## v0.2.0 development fixture boundary {#v020-development-fixture-boundary}

The typed `openagents.omega.dogfood-fixture.v1` preview graph contains the
source-pinned v0.2.0 dogfood snapshot. It preserves the Organization, Team,
Initiative, two Projects, five Project Milestones, Cycle, Release Pipeline,
Release Stages, Release Planning Record, workflow states, labels, Documents,
Project Updates, saved Views, 28 same-identity Work/Issue rows, and their typed
blocking relations. Ten open rows belong to the dogfood Project, 12 open rows
belong to adjacent Security Work, and six closed rows explain the completed
foundation.

The preview is not a Work Index adapter lane. It cannot satisfy production
admission, persist a production route, or admit a command. It loads only when
debug assertions are active and `OMEGA_UI_MOCKS=1`. A release or release-fast
binary ignores the environment request and receives no fixture. The normalized
graph has a checked SHA-256 digest and fixed source time. Every issue retains
its repository, number, URL, and source revision. All assignee, delegate,
claim, lease, Thread, Session, Run, Receipt, Evidence, Verification, and owner
disposition fields remain absent.

The fixture uses Linear-shaped planning names, but `releases[]` decodes only to
a Release Planning Record. It has no canonical release authority. The
OpenAgents extension envelope marks the complete graph as simulated, lists the
lost authority, and grants no commands. Screen composition over this one graph
is owned by OAW-016; the fixture module itself does not add navigation.

When the development fixture gate is active, Omega opens the `Omega v0.2.0`
Project with omega#214 selected. A **DEV MOCKS** section in the sidebar contains
the dogfood Project and the adjacent Security Work Project. The section, its
routes, counts, tab, and selection do not exist when the gate is inactive.
Restoring either fixture Project route without the gate normalizes to My Work
when production Work is admitted, or to the normal Thread destination when it
is not.

One retained surface reads the fixture graph for Overview, List, Board, Issue,
Session, and Review scenes. It does not make screen-specific copies. Project
switching, Issue selection, typed blockers, milestones, progress, release-stage
planning, source coordinates, and Inspector identity all derive from that same
graph. Session and Review show explicit empty projections because the accepted
snapshot has no public delegate, session, diff, pull request, or review. The
screen repeats the development-mock boundary and fixture provenance instead of
creating a plausible live state.

That same surface now accepts the generated canonical `PlanningGraph` through
the owned `omega-effectd.v2` client. Fixture and live input decode into one
view model. The header distinguishes **DEV MOCKS**, **OWNED READ**, stale,
offline, partial, and cursor-gap states. A partial or failed refresh changes
only freshness and loss metadata; the rows stay on the last complete revision.
The last complete live projection is restored visibly as an offline cache, not
as fresh data. This client refresh remains behind the development fixture gate.

The development surface stores only its Project, Issue, and scene selection.
It validates those IDs against the checked fixture before restoration and
defaults to omega#214 when no valid state exists. A build without the fixture
gate does not read or render that development-only selection.

The Issue Inspector also projects the canonical Repository Work Claim for the
selected GitHub Work identity. It shows state, holder, generation, last
evidence, and declared collision scope. Claim, Status, Heartbeat, Block,
Release, and Refresh controls go through the generated supervised client; the
planning fixture and offline planning cache never become claim stores. The
Inspector states that claim state is separate from assignment, delegation,
lease, verification, merge, and release authority. See
[Omega repository work claims](./omega-repository-work-claims.md).

List, Board, Table, Timeline, and Roadmap now share one organization-scoped
planning reducer. They preserve the same Work references, source revision, and
event cursor while applying one filter/group/sort query. The three additional
scenes remain development-only with the dogfood route; they do not admit
production portfolio navigation. See [Omega planning views](./omega-planning-views.md).

The development Session scene reads the selected Work's generated signed
Workroom projection. It renders exact signer, actor, audience, generation, and
causal facts or an explicit empty/offline error. It does not infer a Session,
admitted effect, verification, or owner disposition from a signed event or
relay result. See [Omega signed Workroom projection](./omega-signed-workroom.md).

Use Up/Down or J/K to change the selected Issue and Enter to open its Issue
scene. Keys 1 through 6 open Overview, List, Board, Table, Timeline, and
Roadmap. Keys 7 and 8 open the empty Session and Review scenes. The same controls are available as standard
focusable pointer buttons with accessible labels. Blocked rows use an icon and
text; completed rows use a check icon and progress dots, so color is not the
only cue.

## Preserved source facts {#preserved-source-facts}

Every row uses the generated `WorkSummary` type. Omega preserves the exact
Work reference, source reference, source authority, adapter version, revision,
cursor, freshness, completeness, gap references, and visibility metadata.
The index rejects invalid generated values and quarantines conflicting Work
identities.

The Security adapter emits one separate case for each Git repository candidate,
not only the selected repository. Current managed runs, entropy runs, campaign
project runs, and model-matrix runs become child Work rows with their exact run
refs. The native adapters map only observed source state. They do not invent a
completed item, assignment, delegation, or priority. Inbox groups questions,
recoverable work, blockers, failures, and stale work. My Work distinguishes
local ownership, assignment, participation, and bounded agent delegation.

The identity rules are explicit. A Thread maps to
`work:omega:thread:<thread-ref>`. A Forensics case maps to
`work:omega:forensics:<case-ref>`, and its run maps to
`work:omega:forensics-run:<run-ref>`. Effect-backed rows must repeat their
generated Work reference in the adapter envelope. A source/entity mismatch is
rejected before it enters the index.

For Omega-native Thread Work, the adapter can also project the validated local
participation journal. It preserves the human assignee and the one active agent
delegate with its grant and generation. Revocation removes the active delegate
from the row but does not delete grant history. The adapter ignores a journal
with a different Work or source identity, an invalid record, or a revision
older than the current Thread metadata.

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

The sidebar shows the Work section only after production admission succeeds.
It places that conditional section above Threads. An Experimental section
follows the thread list and contains the source Forensics workbench; the
shared Security cases and runs remain addressable through Work.

Inbox and My Work have search, attention and lifecycle filters, grouped rows,
stable selection, keyboard navigation, and accessible row labels. Press Enter,
Space, or I, or select **Details**, to inspect the Work without changing source
identity. The detail surface provides the explicit source-opening action. The
surface shows loading, partial, offline,
error, conflict, and empty states explicitly. A source failure is never
rendered as successful emptiness.

Opening a Thread row returns to its durable Thread route. Opening a Security
case or run row selects its same-identity shared Work route. The detail surface
shows the exact authority, source reference, revision, cursor, freshness,
completeness, visibility, and accountability. See [Omega Work and Issue
detail](./omega-work-detail.md) for its mutation boundary.
See [Omega Security Work](./omega-security-work.md) for its typed Blocks,
parent/child relations, and fail-closed publication boundary.

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

## Experimental sidebar installed receipt {#experimental-sidebar-installed-receipt}

The installed development build was produced from source commit
`57a74da3fc8e48e34f98131c9539f8c4b61bb684`, whose parent is
`39f5c2c4442323c3f247b71c896824aa2513877b`. The release-fast build embeds the
same source commit in its `omega` executable.

- Application: `/Applications/Omega Dev.app`
- Bundle identifier: `com.openagents.omega.dev`
- Bundle version: `20260803.034541`
- Short version: `0.2.0`
- Architecture: arm64
- Installed and bundled `omega` SHA-256:
  `c3584ab69df882c7e1cbee297d39e53dca39b417a179a008c32dbc96c618f125`
- DMG SHA-256:
  `4d42b48f5aca1594f72b44c1252bf506724db4887398617bf1b6594244e09c65`
- CLI receipt: `Omega 0.2.0 – /Applications/Omega Dev.app`
- Signature receipt: ad hoc arm64 signature; `codesign --verify --deep
--strict` passed before and after installation.
- Recoverable pre-build development app:
  `/private/tmp/Omega-Dev-before-experimental-sidebar-20260802.app`
- Recoverable replaced development app:
  `/private/tmp/Omega-Dev-replaced-experimental-sidebar-20260803.app`

The exact source passed the focused GPUI sidebar and route test, all 11 Work
Index model tests, release all-target lint for `agent_ui`, formatting, and the
documentation build. Installed-artifact verification used hashes, metadata,
CLI output, embedded source provenance, and code-sign checks only. It did not
launch or control the installed UI. The production app binary remained at
SHA-256
`0475b4f52bd0c79b53e761f8b720e4e18d6533eee223c60f0fd81b1ca77de2c`.
