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
therefore return no rows until the owned read supplies the required state; the
reducer does not infer attention from a nearby reference or label.

## Development surface

The v0.2.0 dogfood Project exposes Overview, List, Board, Table, Timeline,
Roadmap, Issue, Session, and Review scenes behind the existing debug/mock gate.
List and Board now use the same reducer as the three added views. Saved View,
filter, group, and sort controls update one persisted query state. Older saved
state defaults to All Work. Table provides a dense keyboard-addressable field
view. Timeline shows grouped Work tracks with non-text completion and blocker
cues. Roadmap shows group progress without turning a portfolio Project or
milestone into the root Work object.

The input may be the checked fixture, the complete owned planning read, or the
explicit last-known-good offline projection. Its provenance and loss state
remain visible. No view gains a renderer-specific mutation path. Existing
claim controls continue through the generated repository-claim processor, and
the planning views remain read-only.

## Scope boundary

This is a substantive OAW-008 slice, not its close proof. Production portfolio
navigation remains absent. Native Organization switching, saved-View editing,
bulk actions, inline mutations, drag/drop Intent admission, large-dataset
performance, installed accessibility, and complete two-domain workflows remain
required before omega#215 can close. Test execution is deferred to the single
final omega#208 build gate by owner direction.
