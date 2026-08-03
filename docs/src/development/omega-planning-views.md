# Omega planning views

Omega projects one source-owned planning graph into multiple read views. The
renderer does not own Work identity, revision, filtering, grouping, sorting,
or commands.

## Shared projection

`omega_work_index::project_planning_view` accepts the complete planning view
model plus one organization-scoped query. It returns canonical Work rows,
groups, the source revision, and the event cursor. List, Board, Table,
Timeline, and Roadmap consume that projection.

Every row preserves:

- the canonical GitHub-backed Work reference for the imported source row;
- the same-identity Issue projection and provider URL;
- repository and source revision;
- Project, Project Milestone, Cycle, and Release Planning Record links;
- lifecycle, priority, completion, and blocker facts.

Changing renderer kind does not change the row set or source revision. The
shared query applies Organization, Project, saved View, open/blocked/completed
state, grouping, sorting, and search before rendering. A different Organization
ID fails closed with no rows or groups. This prevents counts or cached view
state from crossing the active Organization boundary.

The saved View selector has typed entries for All Work, critical path,
unassigned, blocked, agent active, needs owner, in review, and verification.
Critical path uses Release Planning Record scope or an exact outgoing Blocks
relation. Unassigned, blocked, in-review, and verification use their exact
projection facts. An Agent Session reference alone does not prove active, and
an Owner Disposition reference alone does not prove needs-owner. Those Views
therefore do not infer attention from a nearby reference or label. The current
detail reader can contribute exact attention for its selected Work: an active
canonical Session supplies **Agent active**, and a latest canonical Question on
an active or paused Session supplies **Needs owner**. Every renderer consumes
that same attention overlay through the shared reducer. Because Omega has only
one selected Work snapshot, the result is visibly marked partial and never
claims portfolio-wide completeness. A missing snapshot returns no inferred
attention row.

## Development surface

The v0.2.0 dogfood Project exposes Overview, List, Board, Table, Timeline,
Roadmap, Issue, Session, and Review scenes behind the existing debug/mock gate.
List and Board now use the same reducer as the three added views. Saved View,
filter, group, and sort controls update one persisted query state. Older saved
state defaults to All Work. The local View editor stores up to eight ordered,
stable-ID, user-named queries. Each captures that exact typed Saved View,
filter, group, and sort tuple and can be selected, updated, renamed, or removed
without introducing a renderer-owned copy. Names are trimmed, bounded,
duplicate-free, control-free, and reject private-key-shaped text. Changing any
shared control clears the active View until a saved query is reapplied or
updated. The former single **My view** record migrates to
`view:omega-local:1` without losing its query. These Views are owner-local;
their existence does not claim Organization sharing or Sync. Table provides a
dense keyboard-addressable field view. Timeline shows grouped Work tracks with
non-text completion and blocker cues. Roadmap shows group progress without
turning a portfolio Project or milestone into the root Work object.

The input may be the checked fixture, the complete owned planning read, or the
explicit last-known-good offline projection. Its provenance and loss state
remain visible. No view gains a renderer-specific mutation path. Existing
claim controls continue through the generated repository-claim processor, and
the planning views remain read-only.

## Scope boundary

This is a substantive OAW-008 slice, not its close proof. Production portfolio
navigation remains absent. Native Organization switching, Organization-shared
Views, complete portfolio attention reads, Inbox/triage interaction, bulk actions,
inline mutations, drag/drop Intent admission, large-dataset performance,
installed accessibility, and complete two-domain workflows remain required
before omega#215 can close. Test execution is deferred to the single final
omega#208 build gate by owner direction.
