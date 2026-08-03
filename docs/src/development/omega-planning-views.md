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
shared query applies organization, Project, open/blocked/completed state,
grouping, sorting, and search before rendering. A different Organization ID
fails closed with no rows or groups. This prevents counts or cached view state
from crossing the active organization boundary.

## Development surface

The v0.2.0 dogfood Project exposes Overview, List, Board, Table, Timeline,
Roadmap, Issue, Session, and Review scenes behind the existing debug/mock gate.
List and Board now use the same reducer as the three added views. Filter,
group, and sort controls update one persisted query state. Table provides a
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
navigation remains absent. Native organization switching, saved-view editing,
bulk actions, inline mutations, drag/drop Intent admission, large-dataset
performance, installed accessibility, and complete two-domain workflows remain
required before omega#215 can close. Test execution is deferred to the single
final omega#208 build gate by owner direction.
