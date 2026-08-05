# Omega desktop workbench shell

The Omega desktop workbench keeps the active thread transcript in the center
while a compact activity rail opens a retained work-surface dock beside it.
The shell owns selection, focus transfer, responsive allocation, and host
lifecycle. It does not create another workspace or replace the conversation
view.

This page is the implementation contract for the shell introduced by
[issue 128](https://github.com/OpenAgentsInc/omega/issues/128). The logical
state and proof contracts are documented separately in
[Workbench projection consistency](./workbench-consistency.md) and
[Deterministic Omega workbench proofs](./omega-workbench-proof.md).

## Scope {#scope}

Issue 128 installed the production rail and dock boundary, normal GPUI actions,
per-thread reducer integration, retained generic hosts, typed badge plumbing,
accessibility semantics, and deterministic tests. Issue 133 added the
authoritative repository, worktree, branch, and Git-state projection described
below. Issue 129 rehomes the Workspace-created native Project Panel into Files,
issue 134 mounts native project Search, and issue 136 mounts native agent
change review. Issue 132 rehomes the native Git Panel under an exact
repository/worktree scope. Issue 137 rehomes the Workspace-owned native
Terminal Panel and gives explicit terminal creation the active thread's exact
worktree path. Issue 130 mounts the active thread's typed ACP plan in a retained
native Plan surface. Issue 192 adds a repository-bound Forensics preflight
surface. Its target, placement, coverage, and operator-action contract is
documented in [Omega Forensics preflight](./omega-forensics-preflight.md).

The remaining work is deliberately split so each native adapter can prove its
own identity, behavior, and lifecycle:

| Follow-up                                                      | Responsibility                                                |
| -------------------------------------------------------------- | ------------------------------------------------------------- |
| [Issue 129](https://github.com/OpenAgentsInc/omega/issues/129) | Mounted the existing Project Panel as Files                   |
| [Issue 130](https://github.com/OpenAgentsInc/omega/issues/130) | Present the thread's typed plan                               |
| [Issue 131](https://github.com/OpenAgentsInc/omega/issues/131) | Persist and cold-restore each thread's selection              |
| [Issue 132](https://github.com/OpenAgentsInc/omega/issues/132) | Mounted the existing Git Panel                                |
| [Issue 134](https://github.com/OpenAgentsInc/omega/issues/134) | Mounted an embedded project-search entity                     |
| [Issue 136](https://github.com/OpenAgentsInc/omega/issues/136) | Mounted a thread-bound native review entity                   |
| [Issue 137](https://github.com/OpenAgentsInc/omega/issues/137) | Mounted the existing Terminal Panel without implicit spawning |

Files, Search, Review, Forensics, Git, Terminal, and Plan are production
content.

## Composition boundary {#composition-boundary}

The Agent Panel builds the toolbar, transcript, composer, drag target, and
legacy terminal content once. The shell wraps that completed content in one
horizontal allocation:

1. The 40-pixel workbench activity rail (sidebar toggle at top, Settings at
   bottom; work-surface icons between).
2. The threads sidebar when expanded (no separate column when collapsed).
3. The work-surface dock when it is logically open and fits.
4. The existing transcript column with flexible width.

The transcript entity is not recreated when a surface opens, closes, or
changes. Surface selection therefore cannot reset message state, scroll state,
streaming state, or composer contents by rebuilding the conversation.

The work-surface dock is not a `workspace::Dock`. It is a host inside the
Agent Panel's zero-base surface. Calling a Workspace panel toggle from this
host can unzoom the Agent Panel or reveal the editor, so native adapters must
render or delegate to retained native entities without routing through the
ordinary Workspace open-panel path.

Files, Git, and Terminal share one initialization boundary. Production startup
and `AgentWorkbenchFrontDoor::mount` both call
`agent_ui::initialize_workbench_panels`, which fully loads and registers the
native `ProjectPanel`, `GitPanel`, and `TerminalPanel` before either path may
construct `AgentPanel`. This ordering is required because `AgentPanel::new`
captures those exact workspace entities for its adapters. The shared helper
verifies all three registrations before it returns; a partial load fails the
front door instead of leaving an enabled rail control permanently waiting for a
panel that cannot arrive.

User-triggered Workspace center opens share a separate presentation boundary.
Every default-surface handler calls
`Workspace::reveal_zero_base_center_for_user_open` before it opens or activates
a center item. This includes Files, Git, Search, Review excerpts, transcript
file and skill links, tool-call locations, rule controls, debug JSON,
the ACP registry, and file-like URL fallthrough. The low-level Workspace open
APIs do not reveal implicitly: background restoration and preview machinery
retain their existing presentation semantics, HTTP and HTTPS links stay in the
system browser, and `AgentDiff::review_in_active_editor` may advance within the
editor that is already visible. Intentional read-only file and Markdown peeks
remain sheets. Closing the final revealed tab restores the agent-only surface.

Vim is part of both sides of that presentation boundary. The zero-base action
gate admits the exact non-Helix editor action set derived from the shipped Vim
keymap, plus `workspace::ToggleVimMode` and `workspace::Save`. Helix-flavored
actions, pane and tab management, Project Panel actions, the rest of the
Workspace namespace, and `workspace::ToggleHelixMode` remain refused. One
`vim::ModeIndicator` is created during Workspace initialization, before center
editors or conversations, and shared with Agent Panel. Loading and connected
composer bars render that same entity at bottom left; only the active connected
thread hosts it, so its mode follows focus without producing duplicate readouts.

### Native Files adapter {#native-files-adapter}

The shared initializer creates the native `ProjectPanel` and initially
registers it with the legacy Workspace dock before Agent Panel construction.
Agent Panel captures that exact `Entity<ProjectPanel>`. Merely constructing or
rendering Agent Panel does not scope, clear, or detach the legacy Project Panel.
It remains unscoped and retains its existing state outside the first Files
handoff transaction.

The first successful Files activation establishes the canonical ownership
boundary. Agent Panel resolves and applies the typed worktree scope, asks the
shell to create or find the retained Files host, and only then calls
`Workspace::rehome_panel`. Rehoming removes the entity from every ordinary Dock
render list while retaining the exact entity in a generic nonvisual Workspace
panel registry. The Files host is its sole visual parent from that point
forward, while `workspace.panel::<ProjectPanel>()` and the native
open/focus/toggle/close routes still resolve through `PanelEvent::Activate` and
`PanelEvent::Close`. Later collapse, reopen, thread switch, and worktree switch
operations reuse the cached entity instead of registering another Project
Panel or moving it back to a Workspace dock.

The handoff is transactional. Agent Panel snapshots the previous scope before
preparing Files. If host creation or detaching fails, it restores that scope,
rolls back the surface projection, keeps or returns the entity to its previous
visual owner, returns focus to visible thread content, and reports the error.
The first-failure case therefore leaves the legacy panel genuinely unscoped
and registered with its previous owner. Compatible state survives scope
reconciliation; incompatible state is deliberately filtered rather than
resurrected by rollback. This prevents two visible parents, an ownerless panel,
and a partially applied first activation.

The adapter subscribes to the rehomed entity's panel events before changing
ownership. Project-driven reveal, `ActivateProjectPanel`, command-palette
actions, File History, and external Workspace panel callers therefore activate
the Files host rather than an absent legacy dock. `ToggleFocus` and
`CloseActiveDock` are captured at the embedded boundary: they collapse Files
and focus the visible transcript without closing the outer Agent Panel. An
unavailable host refuses activation instead of focusing a hidden native tree.

The adapter does not construct a second project tree, filesystem watcher, Git
projection, selection model, or editor-opening path.

Before the host becomes visible, Agent Panel resolves the selected identity
candidate's absolute worktree path back to the Project's typed `WorktreeId`.
Project Panel applies that identifier as a fail-closed scope and derives rows
from only that visible worktree. Scope reconciliation preserves selection,
marked entries, edit state, clipboard entries, and scroll state only when they
are compatible with the new scope. It filters incompatible state and clears
transient drag, hover, context-menu, and row-derivation state. Expansion
remains keyed by worktree, so returning to a target restores that worktree's
native expansion state without leaking rows from another target.

Authority and compatibility are distinct. Loading, content error, offline,
inconsistent identity, and other transient failures put Project Panel in an
explicit `Unavailable` interaction state. With the same resolved worktree, the
adapter retains compatible selection, expansion, edit, clipboard, scroll, and
undo state while publishing no rows and allowing no action. An unbound,
missing, or incompatible target clears that state. Undo history is filtered by
typed `WorktreeId`, so an operation recorded under worktree A cannot execute
after rebinding to worktree B.

The scoped root is the structural authority and is kept expanded when it is
rendered. It is not counted as file content. `Empty` means the scoped worktree
has no visible non-root entry after the Project Panel's existing hidden-file
and Git-ignore filters. `Missing` means the typed root no longer exists.
Neither state can publish rows from another visible workspace root.

Project Panel captures a monotonic scope revision with each asynchronous row
derivation and checks it before publishing. A filesystem update started for
worktree A therefore cannot overwrite the visible tree after the thread moves
to worktree B. The shell's existing `(thread, binding, surface)` host key and
generation-gated completion contract remain the outer consistency boundary.

When content is `Ready`, the Files host delegates its `Focusable`
implementation to Project Panel. Loading and error content hides the native
tree and focuses the visible host instead. Returning to `Ready` transfers focus
back into Project Panel. Keyboard traversal, open, rename, context menus,
reveal, drag-and-drop, diagnostics, and Git decoration are consequently the
existing native behavior, not workbench-specific replicas. Stable tree and
path-row semantics make the same production rendering observable to
deterministic GPUI tests.

Native actions that produce Workspace center items explicitly call
`Workspace::reveal_zero_base_center_for_user_open` after revalidating the
current scope.
Open, permanent open, split, File History, Markdown preview, compare, and search
results therefore become visible beside the Agent Panel instead of succeeding
into a hidden zero-base pane. Preview open preserves Project Panel focus;
permanent open focuses the editor. Closing the final center item restores the
agent-only presentation.

### Native Search adapter {#native-search-adapter}

The Search work surface uses the search crate's `ProjectSearchView`,
`ProjectSearchBar`, project search task, result multibuffer, and editor
navigation. The workbench adapter supplies a visual host and typed binding; it
does not run a parallel grep service or translate native results into a second
row model.

Each native search request captures a monotonic request generation and the
active `WorktreeId`. Running, completed, cancelled, and failed lifecycle states
retain that request identity. A result publication is accepted only while both
values still match. Changing the typed scope within one native view cancels the
prior request, clears incompatible results, applies the new scope, and advances
the generation before another result can render. When the shell changes to a
different binding-keyed host, the prior host may finish independently but is no
longer eligible to project. A late completion from worktree A therefore cannot
flash into worktree B.

Query text, include and exclude filters, case-sensitive, whole-word, regex, and
include-ignored options remain native Search state. The native selected match
owns its project path and range. The workbench host retains that compatible
state across collapse and reopen, but does not restore it into another typed
binding. Empty query, no results, invalid regex, loading, cancellation,
disconnection, and removed-worktree failures are explicit native or host
states.

The Search toolbar and result content live inside
`omega.workbench.surface.search`. Non-ready host content hides both instead of
leaving stale results interactive. Returning to `Ready` restores focus to the
native query or result target. Opening a result uses the existing Search editor
navigation path and reveals the Workspace center beside the retained
transcript.

### Native Review adapter {#native-review-adapter}

Review embeds the existing `AgentDiffPane`, `AgentDiffToolbar`, split diff
editor, hunk controls, and keep/reject commands in a retained
`NativeReviewSurface`. The adapter contributes the workbench host, binding, and
lifecycle boundary. It does not create another diff engine, patch
representation, filesystem writer, or keep/reject implementation.

Every mounted pane has an `AgentDiffBinding` containing the logical Omega
thread ID, ACP session ID, `RepositoryBinding`, typed `WorktreeId`, and an
`AgentDiffCheckpoint`. The checkpoint is the source action-log entity ID plus
the workbench binding generation. The host key retains the outer
`(thread, repository/worktree, Review)` identity; the checkpoint additionally
prevents a pane or completion from being reused after an action log or
generation changes inside that host epoch.

Before a pane accepts a binding it verifies that the ACP session, action-log
entity, and visible worktree still match. Changed buffers are filtered by the
bound `WorktreeId` before they become multibuffer excerpts. File ordering,
diff-hunk ranges and statuses, editor decorations, keyboard navigation,
selection, scrolling, and open-in-editor behavior remain native Agent Diff
state. The workbench never falls back to the most recently active global diff.

Each generation has an explicit lifecycle:

- `Unbound` before a typed identity is installed;
- `Loading` while a generation-bound review is being prepared;
- `Empty`, `Ready`, `Streaming`, or `AllReviewed` for valid native content;
- `Offline` while the project connection cannot authorize current content;
- `UnavailableCheckpoint` when the source action log cannot be resolved;
- `UnsupportedBinary` when a changed binary cannot be represented safely;
- `Invalidated` when the bound worktree or identity disappears; and
- `Error` for a surfaced failure with user-readable detail.

Only `Ready` and `Streaming` permit keep/reject mutations. The existing
`Keep`, `Reject`, `KeepAll`, and `RejectAll` handlers remain the sole mutation
authority and revalidate the binding before writing. Review lifecycle code
does not add a second patch-application path, bypass native confirmation, or
convert an invalid action into apparent success.

An incremental action-log update rebuilds native excerpts while first
capturing the selected path and hunk range. If that hunk still exists, its
selection and editor position survive. If it disappeared, selection falls
forward to the next hunk in the same file and then to the first remaining
hunk. An empty rebuild focuses the pane-level empty state. This rule is
deterministic and does not depend on excerpt entity allocation or render order.

Completion and lifecycle callbacks carry the checkpoint generation. A callback
whose generation no longer matches returns without publishing content or
status. Worktree removal clears the old excerpts and invalidates the pane;
disconnect marks it offline. Switching threads or worktrees projects a
different retained host, so a late completion may settle privately in its old
host but cannot replace or mutate the visible one.

The embedded toolbar is rendered as the accessible
`omega.workbench.review.toolbar` (`Toolbar`, “Review controls”) and the native
diff below it as `omega.workbench.review.content` (`Group`, “Review changes”),
both inside `omega.workbench.surface.review`. Opening excerpts uses the
existing editor action, then reveals the Workspace zero-base center beside the
transcript. Collapse and reopen retain the pane and toolbar entities; an editor
round trip does not transfer Review ownership to the global Workspace item
selection.

Deterministic tests inspect a native snapshot rather than rendered labels. That
snapshot includes the full typed binding and checkpoint, lifecycle and
pending/error state, ordered file and hunk projection with statuses and ranges,
stable selection, retained entity IDs, focus owner, and observed keep/reject
counts. Tests use two threads with different worktrees and action logs, then
exercise navigation, incremental updates, mutation, collapse/reopen, thread
switch, invalidation, and a released stale completion. A passing screenshot is
therefore impossible unless the underlying identity, selection, mutation,
focus, and no-leak assertions have already passed.

### Native Git adapter {#native-git-adapter}

Git embeds the Workspace-created `GitPanel` and its existing repository
entities, status subscriptions, commit editor, history view, diff navigation,
branch operations, credential flow, hooks, signing, confirmation prompts, and
error reporting. Agent Panel contributes only an ownership handoff, an exact
scope, and the workbench lifecycle boundary. It does not create a parallel Git
client or execute shell-string Git commands.

The first successful Git activation resolves the selected
`ThreadIdentityCandidate` to one typed `RepositoryId` and `WorktreeId`. The
adapter verifies that `GitStore::repository_ids_for_worktree` contains that
pair, applies a `GitPanelRepositoryScope`, creates or finds the retained host,
and then calls `Workspace::rehome_panel` for the exact existing panel entity.
The handoff is transactional: host or rehome failure restores the prior scope
and collapses the attempted host. Later collapse, reopen, and thread switches
reuse the same `GitPanel`; they do not recreate its editor, selection, scroll,
history, or a safe operation already in progress.

`GitPanelRepositoryScope` contains the repository ID, worktree ID, and
workbench binding generation. While it is present, workspace-global
`ActiveRepositoryChanged` events cannot retarget the panel. Repository updates
are accepted only for the scoped repository, and every action continues
through the existing panel's repository entity. A multi-root workspace may
therefore have repository A selected globally while an Omega thread safely
reads and mutates repository B.

An unavailable scope is distinct from an unscoped legacy panel. After handoff,
offline, reconnecting, inconsistent, removed, or unresolved thread identity
retains the last typed scope but resolves no actionable repository. It never
falls back to the first or globally active repository. Repository removal
keeps that fail-closed scope so a late action cannot target a neighbor; a
subsequent authoritative observation can bind a newly materialized runtime
repository entity for the same logical thread target.

The panel increments an internal transition generation whenever its scope or
availability changes. Debounced row derivation, commit-buffer restore, Git
access discovery, history loading, and follow-up work capture that generation
and verify it before publishing into panel state. Repository operations that
were already safely submitted may finish against their original repository,
but their completion cannot replace the selection, rows, draft, access state,
or history of a newer scope.

The workbench header and rail derive repository, worktree, branch, dirty,
conflict, and ahead/behind identity from the selected typed observation. The
native surface binding carries the same logical repository/worktree,
`RepositoryId`, `WorktreeId`, and workbench generation. Its deterministic
snapshot then records the exact scope, resolved repository, head state,
ordered native status entries and sections, counts, selection, commit-button
validation, and pending operations. Proofs reject a frame unless the header
binding, rail badge, native scope, repository entity, and generation agree.

Clean, dirty, conflicted, detached, unborn, pending, offline, reconnecting,
removed, and error are explicit native-surface lifecycles. Only actionable
states render and focus `GitPanel`; non-actionable states hide it behind an
accessible status or alert so stale rows cannot receive commands. Native
`Toggle`, `ToggleFocus`, `Close`, `CloseActiveDock`, and `PanelEvent` routes
are captured after rehome and open, focus, or collapse the embedded surface
without unzooming Agent Panel or reopening a legacy Workspace dock.

Stage, unstage, commit, discard, open-diff, checkout, pull, and push retain the
native safety boundaries. In particular, discard still passes through the
existing confirmation prompt, commit still uses the panel's validation,
credential and signing failures remain visible, and opening a diff uses the
existing Workspace item path with the scoped repository entity passed
explicitly to `ProjectDiff`, `StagedDiff`, or `UnstagedDiff`. It cannot resolve
a same-named path against the Workspace-global repository. The adapter never
converts a cancelled, rejected, stale, or failed operation into success.

Portable tests drive the production `SelectGit` action against fake Git
backends and inspect the retained panel snapshot. They prove exact entity
retention, global-A/scoped-B isolation, stage and unstage targeting, discard
cancel/error invariance, commit validation, diff opening, repository and
thread switching, removal, offline/reconnect, and rejection of a held stale
refresh. Real-pixel scenes run only after the same typed snapshot, badge,
focus, mutation log, and foreign-path exclusion checks pass.

### Native Terminal adapter {#native-terminal-adapter}

Workspace creates one `TerminalPanel`. On the first successful Terminal
activation, Agent Panel resolves that exact entity, creates a retained
`NativeTerminalSurface`, and rehomes the panel out of the ordinary Workspace
dock render list. Workspace lookup and native panel events continue to resolve
the same entity. The workbench does not build a second terminal emulator,
pane tree, tab model, PTY path, task integration, or action set.

The panel remains Workspace-owned. The shell's active thread binding is the
creation target for the next explicit terminal, not permission to rewrite
existing terminals. `NativeTerminalBinding` records the logical thread,
repository/worktree binding, typed `WorktreeId`, canonical absolute worktree
path, and binding generation. When the user explicitly invokes New Terminal
or Split from the visible embedded surface, Agent Panel passes that exact
absolute path to `TerminalPanel::create_terminal_at_working_directory`. It
neither uses the process-wide current directory nor clones a terminal owned
by another thread. The native Files surface switches its `Open in Terminal`
command to the typed `project_panel::OpenInThreadTerminal` action while
embedded. Agent Panel accepts only a path inside that same worktree;
outside-worktree requests fail closed instead of reaching the legacy dock.
The global `workspace::OpenTerminal` action remains refused in zero base, so
a revealed center pane cannot bypass the thread-bound route.
After creation succeeds, the surface associates the terminal entity ID with
the binding that created it. That owner is immutable even when the active
thread, repository, worktree, generation, or next-creation target changes.

Selecting Terminal, focusing it, collapsing it, reopening it, switching
threads, or rebinding a thread never creates or kills a process. Process
creation is confined to explicit thread-bound New, Split, and Files Open in
Terminal controls. The embedded path
uses `RevealStrategy::Never`, so creation cannot reopen a legacy Workspace
dock or unzoom Agent Panel. Spawn failures surface through the workbench error
path and the panel's pending count is balanced. Existing tabs, pointer-driven
tab selection and close controls, zoom, copy, paste, search, scroll, task
actions, and terminal input remain native `TerminalPanel`, `Pane`,
`TerminalView`, and `Terminal` behavior. Workbench New and Split controls call
scoped handlers directly because the process-wide zero-base gate cannot safely
admit Workspace or Pane creation actions.

Focus follows the visible native tree. Opening Terminal transfers focus to the
panel's activation focus handle. Native keystrokes reach only the focused
`TerminalView`; returning focus to the transcript prevents later text from
being written to the terminal. The normal terminal keymap remains in force.
The narrower `WorkbenchTerminal > Terminal` context adds explicit workbench
escape routes: Collapse Work Surface Dock and Focus Thread Transcript, plus
thread-bound new-terminal, tab-next, tab-previous, tab-close, and native search
bindings. The tab wrappers call the embedded panel directly; generic pane
mutations are not globally admitted by zero base. A panel Activate event opens
or focuses the embedded surface; Close collapses only the inner
work-surface dock and returns focus to the transcript.

The panel entity and all terminal entities stay retained while the dock is
collapsed, while another surface is selected, and across thread switches.
Output, selection, pane geometry, tabs, splits, process identity, and the
per-terminal creation owner therefore remain attached to the same entities.
The rail badge is derived from the native panel snapshot: terminals with a
process ID that have not exited, running task terminals, and pending explicit
creations contribute to the running count. Terminal exit notifications
invalidate that snapshot, and owner records are pruned when their terminal
items close.

Remote project availability and terminal-process lifetime are separate state.
`Ready`, `Offline`, `Reconnecting`, `WorktreeRemoved`, and `Error` are explicit
owner states. Offline, reconnecting, removed, and error states retain existing
terminal entities and output but disable the workbench New control and the
native TerminalPanel New and Split menus. Removal does not rebind
or kill a process; the active terminal continues to disclose its original
owner while the header discloses the current creation target. Reconnect may
restore creation only after current binding authority is ready. A stale spawn
completion cannot be registered as if it belonged to a newer binding: the
just-created native item is synchronously removed before it can become owned
or retained under the newer scope. A foreign requested binding is rejected
rather than redirected.

The retention boundary ends when the native terminal item, panel, window, or
application is actually closed. In particular, app restart does not preserve a
live operating-system process. Native panel persistence can restore eligible
layout and working-directory metadata, but deserialization creates a new
terminal shell; task terminals are not serialized. Tests and product copy must
not describe restart restoration as process continuation.

Test support constructs display-only native terminals inside `TerminalPanel`,
injects output through the terminal emulator, inserts tabs or splits through
the real pane APIs, activates the real `TerminalView`, and exposes exact input
bytes and normalized panel snapshots. This gives the front-door and visual
runner deterministic terminal content without starting a shell or importing
the terminal implementation directly into the runner. The same panel-owned
factory can defer an explicit production-path creation, then deterministically
succeed or fail it, so tests observe pending badges, failure propagation, and
late completion after a worktree switch without launching a process.

### Native Plan adapter {#native-plan-adapter}

Plan is a retained `NativePlanSurface` bound to the exact active
`Entity<AcpThread>`. It reads `acp_thread::Plan` and `PlanEntry` directly; it
does not recover steps, status, or priority from message text or Markdown.
The dock and inline transcript plan use the shared `plan_presentation` mapping
for pending, in-progress, completed, and forward-compatible unknown status
icons, colors, labels, and completed-step text styling.

Each accepted session Plan update advances the thread's monotonic local plan
revision. Exact content matches retain
their IDs through insertion, removal, and reorder. A same-shape replacement
also retains positional IDs for edited content; a structural replacement gives
unmatched steps new IDs rather than transferring selection to another logical
step. ACP currently supplies neither a provider revision nor a provider step
ID, so this remains client projection identity, not a claim that Omega can
recognize reordered packets within one still-current ACP session. The surface
additionally binds the logical thread
ID, workbench binding generation, and exact observed thread entity. Older
generations, a different logical thread at any generation, an event from a
replaced entity, and a revision below the surface's accepted revision cannot
publish into it.

Malformed updates, including blank steps, advance the observed revision but
retain the last good plan and expose an accessible error lifecycle. Empty,
active, all-complete, historical-only, interrupted, stale, reconnecting, and
malformed states are explicit. Stale, reconnecting, interrupted, and malformed
states keep the last good steps, selection, and counts visible while a separate
lifecycle banner discloses that the projection is not current.

Completed plans are copied into `AgentThreadEntry::CompletedPlan` only when
every step has the explicit completed status. Those historical steps carry the
source transcript entry index. Selecting one invokes the existing thread-view
scroll path after revalidating the active thread and source entry. Current ACP
plan steps have no stable source event; selecting one keeps the selection and
explicitly reports that no transcript event exists instead of inventing a
navigation target. Collapse and reopen retain the surface entity and ID-based
selection. Thread switches retain independent hosts, subscriptions, revisions,
and step sets.

Deterministic tests feed typed `SessionUpdate::Plan` values through
`AcpThread::handle_session_update`. Seed sweeps assert revisions, stable IDs,
order, status, priority, active step, malformed retention, completion history,
source navigation, lifecycle retention, collapse/reopen identity, and late
inactive-thread isolation. Visual scenes must prove the same typed snapshot and
accessibility contract before their pixels are captured.

> Note: the typed thread outline (issue 135, the retained `ThreadOutline`
> sidebar beside the transcript) was removed at owner direction 2026-07-30
> (delta OMEGA-DELTA-0188).

## State ownership {#state-ownership}

`omega_workbench_state::WorkbenchProjection` remains the semantic source of
truth for:

- the active logical thread;
- each thread's repository and worktree binding;
- requested and effective surface;
- logical dock-open state;
- connection phase;
- availability and invalidation;
- pending generation-scoped loads; and
- logical command ownership.

The shell consumes `visible_projection()` for rendering and command routing.
It separately tracks actual GPUI focus as one of:

- the transcript or composer;
- one activity-rail item; or
- the visible work-surface host.

Do not infer selection from icon color, the rendered child, or GPUI focus.
Focus can temporarily move while the selected surface remains unchanged.
Likewise, the reducer's logical owner is not proof that an element owns actual
GPUI focus. Semantic tests must assert both.

Direct requests for unavailable surfaces fail without changing the
projection. Deterministic fallback is reserved for a previously valid request
that becomes unavailable because its binding, capability, restore input, or
reconnect snapshot changed.

### Thread identity projection {#thread-identity-projection}

`ThreadIdentityProjection` is the presentation-side catalog and selection
authority paired with `WorkbenchProjection`. It resolves the active ACP
thread's working directories against the Project's visible worktrees and
`GitStore::repository_ids_for_worktree`. It never chooses
`visible_worktrees().next()` independently in the header.

Each candidate contains distinct typed fields for project, repository,
worktree, full worktree path, branch/head state, Git summary, source revision,
and the exact `RepositoryBinding` used by the reducer. Branch/head state is one
of named branch, detached HEAD, unborn repository, or no Git. Git summary
contains changed-file, conflict, ahead, and behind counts from the same
repository snapshot that feeds the Git rail badge.

The active header, work-surface host key, capability registry, and command
routing all consume the selected candidate's binding. A repository/worktree
change applies `ChangeBinding` once with the expected generation; it never
performs an observable remove-then-bind pair. The transition replaces both
identifiers, advances generation once, recomputes deterministic fallback, and
makes completions captured under the old binding stale.

Picker callbacks capture the source thread, binding, and formal generation.
They revalidate that epoch and resolve the target from the current candidate
catalog before changing anything. This prevents a picker opened for one thread,
or before an away-and-back binding change, from retargeting newer state.
The ConversationView's desired work directories are the preference authority
while a session is loading, before an `AcpThread` exists, so a restored
multi-root thread cannot temporarily bind to the lexicographically first root.
The selected working directory is persisted as its exact main-worktree/folder
pair; later title, message, Git, and project events do not expand it back to
every project root.

Live target changes are capability-gated at the agent connection and invoke
its concrete `update_work_dirs` operation for every idle thread in the
conversation before one presentation transition is committed. Each thread's
prior directories are captured independently. A rejected operation attempts
every required rollback, leaves the reducer binding and generation unchanged,
and preserves loads already issued under the old binding. Omega's native
connection and deterministic stub update the connection-owned `AcpThread`.
If one session accepts the new target, another rejects it, and a rollback also
fails, the connection reports a typed partial-retarget failure. The identity
enters `Inconsistent`: repository-bound surfaces, branch changes, and prompt
submission stop because no single path is authoritative. The repository and
worktree pickers remain available as recovery controls. Reselecting a target
forces that directory update through every session, even when it matches the
last projected path, and clears the inconsistent state only after every
session accepts it. That successful reconciliation advances the binding
content epoch so pre-recovery completions cannot publish.
The native terminal resolver reads those thread directories and uses the single
selected worktree for `.`. An explicit project root or absolute path can run a
command outside that selected worktree. ACP servers whose cwd is fixed when the
session starts leave the repository/worktree controls disabled and explain that
a new thread is required. Updating a client-side presentation field alone is
not treated as proof that an agent session accepted the new target.

Target selection is unavailable while the session-open request is loading,
while any affected thread is generating or waiting for a permission or
elicitation response, and while repository identity is stale, offline, or
reconnecting. This prevents one turn or one fixed-cwd session-open request from
spanning two targets. Button disabled state, tooltips, picker actions, and the
mutation entry point all enforce the same predicate. Branch selection uses the
same session and connection gate at render, keyboard-action, and picker-confirm
time. A successful checkout applies a deliberate same-binding `ChangeBinding`
to advance the content generation, so Git or Review loads captured before
checkout complete as stale. While checkout is pending, the identity projects
`Loading`, all repository/worktree/branch mutations and repository-bound
surfaces are disabled, and the composer is read-only. Text already in the
composer is retained and can be submitted after checkout commits.

Observations are monotonic. An older Git observation cannot replace a newer
selection. A removed selected worktree becomes `Missing` and preserves its
last-known accessible labels while removing the actionable binding; it does
not silently select another worktree. Its repository picker remains available
as an explicit recovery action and validates the formal `None` source binding
before committing the replacement target. A failed recovery remains formally
unbound and cannot revive the removed candidate merely because its last-known
label is still rendered. Loading, stale, offline, reconnecting, missing,
operation-error, and inconsistent-session are explicit phases. A thread switch
changes the active identity state before rendering, so a new thread cannot
display the previous thread's labels while its own observation loads.

No-Git worktrees retain a real thread target. Files, Search, Terminal, and Plan
remain available, while Review and Git explain that the selected worktree has
no Git repository.

## Surface registry and identity {#surface-registry}

The registry contains exactly seven stable surface identities in this order:
Files, Search, Review, Forensics, Git, Terminal, and Plan. Files, Search,
Review, Forensics, Git, and
Terminal require a repository/worktree binding. Plan requires an active thread
but no repository or live connection. Plan can therefore open, collapse, and
reopen while offline; repository-bound surfaces and surface commands remain
connection-gated.

Each entry supplies:

- a stable label, icon, action, and semantic ID;
- availability and a human-readable unavailable reason;
- an optional typed count or attention badge; and
- a retained host keyed by thread, binding, and surface.

Badge inputs are typed data. Adapters must not parse counts, conflict state, or
attention state out of a child entity's rendered label.

The current host key is:

```text
(logical thread ID, optional repository/worktree binding, surface)
```

This prevents a host from being shared across incompatible threads or
worktrees. Generation is still required on asynchronous load completion:
capture the thread, binding, and generation when work starts, then let the
reducer reject stale completion after a thread or binding change.

## Interaction and focus {#interaction-and-focus}

Selecting an inactive available item creates or finds its host, applies the
surface request, opens the dock, and focuses the host. Creating the host happens
before changing the projection. A creation failure therefore leaves the
selection and dock unchanged.

Selecting the already active item collapses the dock and focuses the active
thread transcript. The host remains in the shell's retained map. Selecting the
item again reopens the same entity.

The rail uses normal GPUI actions:

| Action                                              | Result                                     |
| --------------------------------------------------- | ------------------------------------------ |
| `omega_workbench::FocusActivityRail`                | Focus the selected or last-focused item    |
| `omega_workbench::FocusPreviousSurface`             | Move one item up without wrapping          |
| `omega_workbench::FocusNextSurface`                 | Move one item down without wrapping        |
| `omega_workbench::FocusFirstSurface`                | Focus Files                                |
| `omega_workbench::FocusLastSurface`                 | Focus Plan                                 |
| `omega_workbench::ActivateFocusedSurface`           | Open or collapse the focused surface       |
| `omega_workbench::CollapseWorkSurfaceDock`          | Collapse the dock and focus the transcript |
| `omega_workbench::FocusThreadTranscript`            | Return focus to the transcript or composer |
| `omega_workbench::SelectFiles` through `SelectPlan` | Select one stable surface directly         |

The default rail key context maps Up and Down to adjacent items, Home and End
to the endpoints, Enter and Space to activation, and Escape back to the
transcript. Pointer and keyboard activation dispatch the same surface action.

Every rail item is an accessible toggle button inside a vertical toolbar. Its
accessible state includes its name, expanded state, shortcut, and unavailable
description. A disabled item remains visible so the reason can be inspected.
The selected, focused, hovered, unavailable, badge, and attention states must
not depend on color alone.

Hidden hosts must not retain focus or receive workbench actions. Transfer focus
before detaching a host from presentation. Files applies this to the actual
focus chain, not only the reducer's logical focus target: loading and inline
error states focus the rendered Files host, while root removal, offline
invalidation, and dock collapse focus the active transcript or composer. The
unrendered Project Panel cannot remain the keyboard-action owner.

## Responsive allocation {#responsive-allocation}

One allocator decides both the existing threads sidebar and the work-surface
dock. The current dimensions are:

| Allocation                 | Size       |
| -------------------------- | ---------- |
| Threads sidebar, expanded  | 280 pixels |
| Threads sidebar, collapsed | 0 pixels   |
| Workbench activity rail    | 40 pixels  |
| Transcript reservation     | 600 pixels |
| Work-surface dock minimum  | 240 pixels |
| Work-surface dock default  | 320 pixels |
| Work-surface dock maximum  | 480 pixels |

When the threads sidebar is collapsed, it does not draw a separate vertical
rail. Expand/collapse sits at the top of the activity rail, and Settings sits
at the bottom of that same rail.

At the default dock width, an expanded threads sidebar, activity rail, dock,
and reserved transcript fit at 1,240 pixels. Below that width, the threads
sidebar collapses before the work-surface dock. The dock can shrink to 240
pixels and remains present through 880 pixels:

```text
40 activity rail + 240 dock + 600 transcript = 880
```

Below 880 pixels, the shell applies a real dock-collapse transition and returns
focus to the transcript. It does not merely hide a logically open dock. When
the window widens again, the dock stays collapsed until the person explicitly
reopens it. The retained host is then reused.

At widths too small for the 600-pixel transcript reservation, the activity rail
remains visible, the dock remains closed, and the transcript receives the
remaining width. The reservation controls when optional columns collapse; it is
not an application minimum width. The composer must remain reachable and
columns must not overlap.

The dock's right edge is an accessible vertical splitter. Drag it to resize the
dock, or double-click it to return to the 320-pixel default. The splitter and
the host model both use the shared allocator, which clamps the result to 240
through 480 pixels without taking the transcript below its 600-pixel
reservation.

## Thread and binding lifecycle {#thread-and-binding-lifecycle}

Switching threads uses the Agent Panel's existing base-view switch boundary.
The shell opens a reducer thread on first use or switches to its existing
projection. Each thread retains its requested surface and dock state in memory.

A repository or worktree change reconciles through the atomic `ChangeBinding`
reducer transition. If the
previous effective surface is no longer valid while its dock is open, the shell
collapses the dock and focuses the transcript. It does not guess another
repository or silently open another surface.

Closing a thread:

1. Applies `CloseThread`.
2. Removes every retained host keyed to that thread.
3. Cancels or generation-gates pending work.
4. Returns focus to the transcript when no active thread remains.

Closing the window drops Agent Panel, the shell, its retained hosts, and the
Workspace's nonvisual rehome registration for Project Panel. The Project and
its filesystem and Git stores still follow the existing Workspace lifecycle;
the Project Panel's canonical visual ownership follows the Files host after the
first successful handoff. The shell must not create a second project tree,
repository, terminal, search, diff, or plan store.

Issue 128 retains state only for the lifetime of the Agent Panel. Cold restart
persistence and restoration belong to
[issue 131](https://github.com/OpenAgentsInc/omega/issues/131).

## Availability, loading, errors, and offline state {#failure-states}

Capability availability and content state are separate:

- **Unavailable:** The rail item is disabled and explains the missing thread,
  workspace, binding, or capability. Direct action dispatch is rejected
  atomically instead of opening a fallback.
- **Loading:** The dock remains associated with the requested host and presents
  a loading state. Completion must match the captured thread, binding, and
  generation.
- **Error:** A host can present an inline error. If host construction itself
  fails, the dock does not open and the rail exposes a warning status.
- **Offline, reconnecting, or stale:** The rail cannot open a new work surface,
  actual focus returns to the transcript, and logical per-thread selection is
  retained for reconciliation. A reconnect snapshot must pass through the
  reducer before a host becomes actionable again.

The host defines these content states for adapters and tests. Files replaces
the ready placeholder with the native Project Panel only after a typed
worktree scope is available. Missing or removed roots fail closed through the
identity capability instead of falling back to all workspace roots. The active
thread's production `Error` identity phase feeds the visible Files host's inline
alert directly. Returning to a healthy identity restores `Ready`; this path
does not depend on a test-only load transition. `Inconsistent` disables
repository-bound surfaces and closes the dock through the capability
projection. Every non-ready state also disables the rehomed Project Panel's
action and reveal paths even though native Workspace lookup still resolves its
entity. Compatible same-binding state remains available for recovery; no
hidden action can rename, duplicate, delete, undo, or open history while
authority is absent. Other native adapter issues must add their production
feeds instead of using the rail as an inferred connection indicator.

## Deterministic test contract {#deterministic-test-contract}

The shell is tested through the same public action and semantic paths used by
the product. Tests must not call the selection implementation directly to stand
in for a click or keybinding.

Stable selectors include:

| Target           | Selector                                      |
| ---------------- | --------------------------------------------- |
| Workbench root   | `omega.workbench.root`                        |
| Transcript       | `omega.workbench.transcript`                  |
| Activity rail    | `omega.workbench.activity-rail`               |
| Rail item        | `omega.workbench.control.rail.<surface>`      |
| Dock             | `omega.workbench.dock`                        |
| Collapse control | `omega.workbench.control.dock.collapse`       |
| Resize splitter  | `omega.workbench.control.dock.resize`         |
| Hosted surface   | `omega.workbench.surface.<surface>`           |
| Typed badge      | `omega.workbench.badge.<surface>`             |
| Identity strip   | `omega.workbench.thread-identity`             |
| Repository       | `omega.workbench.control.identity.repository` |
| Worktree         | `omega.workbench.control.identity.worktree`   |
| Branch           | `omega.workbench.control.identity.branch`     |
| Repository row   | `omega.workbench.control.repository.<id>`     |
| Worktree row     | `omega.workbench.control.worktree.<id>`       |
| Identity status  | `omega.workbench.identity.status`             |
| Git indicator    | `omega.workbench.identity.indicator.<kind>`   |
| Files tree       | `omega.project-panel.tree`                    |
| Files row        | `omega.project-panel.row.<worktree>.<entry>`  |
| Files scope      | `omega.project-panel.scope.<state>`           |
| Terminal content | `omega.workbench.terminal.content`            |
| Terminal create  | `omega.workbench.terminal.new`                |
| Terminal owner   | `omega.workbench.terminal.owner`              |
| Terminal state   | `omega.workbench.terminal.owner-state`        |
| Plan content     | `omega.workbench.plan.content`                |
| Plan summary     | `omega.workbench.plan.summary`                |
| Plan lifecycle   | `omega.workbench.plan.lifecycle`              |
| Plan history     | `omega.workbench.plan.history`                |
| Plan step        | `omega.workbench.plan.step.<stable-id>`       |
| Plan navigation  | `omega.workbench.plan.navigation-status`      |

For every interaction, assert logical state and rendered semantics:

- the active thread, binding, requested surface, effective surface, and dock;
- exact GPUI focus ownership;
- a unique accessible ID, role, label, state, and unavailable description;
- containment, disjoint columns, clipping, and composer reachability;
- transcript entity identity before and after surface changes;
- host entity identity across collapse and reopen;
- native Project Panel identity before and after the first Workspace-to-Files
  handoff;
- an unscoped, unmodified legacy Project Panel before handoff and restoration of
  its prior scope and ownership after a failed first activation;
- per-thread Files host and dock isolation across thread switches, with
  worktree-keyed native expansion state;
- native keyboard row traversal and file opening through the focused
  Project Panel action path;
- Workspace rehome lookup and panel-event routing for reveal, toggle, close,
  external focus, and exact-path File History;
- preview-open focus preservation, permanent-open editor focus, and visible
  zero-base center presentation without replacing the transcript;
- unavailable-state no-ops for global rename, duplicate, delete, undo, reveal,
  and activation actions;
- cross-worktree undo filtering and same-binding transient state recovery;
- actual focus ownership across loading, identity error, recovery, root
  removal, offline invalidation, collapse, and reopen;
- a native asynchronous rename failure surfaced through the Workspace
  notification path without creating the requested destination;
- stale completion rejection after thread or binding changes; and
- weak-entity release for both the Files host and rehomed Project Panel after
  window teardown.

Terminal tests additionally assert the single Workspace-owned panel identity,
zero implicit creations on selection/collapse/reopen, exact creation cwd,
stable terminal/view/pane IDs, tab and split membership, active selection,
exact input bytes, focus return, running-badge agreement, immutable creation
owners, retained output through lifecycle changes, and explicit rejection or
isolation of stale and foreign spawn epochs. The typed harness defines nineteen
Terminal visual scene contracts; the proof page lists their exact names and
commands. Registration is not evidence that reviewed pixel baselines exist.

Plan tests additionally assert the exact bound thread and generation, accepted
revision, stable step IDs and order, status and priority mapping, active and
selected IDs, historical source indices, no-source disclosure, lifecycle
retention, independent per-thread host identity, and absence of cross-thread
steps after late inactive-thread updates.

Identity tests additionally assert exact picker candidates, one-generation
binding replacement, full tooltip/accessibility values under visual
truncation, typed detached/unborn/no-Git states, Git-summary equality with the
rail badge, connection/error phases, removal-to-missing behavior, and
branch-picker failures. They also invoke a retained picker after switching
threads or starting a turn, recover a removed selection through the rendered
picker, hold session creation pending while checking desired-worktree
selection, prove branch checkout stales old-generation loads, preserve exact
linked-worktree metadata through later events, and check long labels at both
compact and desktop widths. Real FakeFs repositories exercise branch, status,
conflict, and upstream tracking. The Metal suite
captures clean, dirty/conflicted, long/narrow, and offline identity strips as
both whole-window and selector-derived region baselines.

Use `VisualTestContext` actions and selector clicks, GPUI executor timers, fake
time, and scheduler-seed sweeps. Run the semantic assertions before recording
Metal pixels. Computer Use may be a final packaged-application smoke test, but
it is not evidence for these contracts.

The common shell scene matrix must include:

- a wide window that selects every available item;
- the exact 910-pixel allocation boundary and one pixel below it;
- an active thread with no workspace binding;
- pointer and keyboard activation;
- collapse and reopen with stable transcript and host entity IDs;
- two threads with independent requested surfaces and Files hosts;
- an active surface invalidated after it opens;
- a stale completion after switching surface, thread, or worktree;
- transactional Files handoff rollback from the unscoped legacy panel;
- host construction failure and offline, loading, content-error, and production
  identity-error states;
- native reveal, toggle, File History, editor-open, and `CloseActiveDock`
  routing without closing the outer Agent Panel or focusing hidden content;
- transient same-binding recovery and incompatible cross-worktree undo
  isolation;
- a native asynchronous filesystem-action error; and
- thread and window teardown with Files host and Project Panel weak-entity leak
  probes.

Record whole-workbench and rail/dock region baselines for the default, active,
focus-visible, badge, unavailable, narrow, and collapsed states. Loading and
error behavior must pass semantic state-transition tests before pixel capture.
Files adds seven registered whole-window and `files-surface` scenes for wide,
910-pixel narrow, empty, loading, error, multi-root, and stale-filesystem
completion states. See
[Native Files scenes](./omega-workbench-proof.md#native-files-scenes) for their
state, semantic, race, and region contracts. A baseline is added only after its
semantic scene passes.

## Native adapter checklist {#native-adapter-checklist}

For each follow-up surface:

1. Resolve the authoritative thread and repository/worktree binding.
2. Reuse the existing native entity or typed thread store.
3. Add an embedded constructor when the existing public path navigates the
   Workspace center.
4. Keep selection free of side effects. In particular, selecting Terminal must
   not spawn a process.
5. Expose typed availability, badge, loading, and error state.
6. Route commands only to the visible active host.
7. Capture thread, binding, and generation for asynchronous work.
8. Prove pointer, keyboard, focus, narrow layout, stale completion, and cleanup
   behavior with deterministic GPUI tests.
9. Add reviewed whole-workbench and rail/dock pixel baselines after semantic
   checks pass.

## Project-gated thread build receipt {#project-gated-thread-build-receipt}

On 2026-08-03, the project-gated new-thread change was built from source
commit `ab74fe4e0f0c46864665bb6758e8ecc2b47bf5af`, whose parent was
`989b5fe73078aa850ef64e466bcb15b100ee83e2`.

- Application: `/Applications/Omega Dev.app`
- Bundle identifier: `com.openagents.omega.dev`
- Bundle version: `20260803.042233`
- Short version: `0.2.0`
- Architecture: `arm64`
- Installed and bundled `omega` binary SHA-256:
  `48c45bc6017699ae3cfbc3b3c5abd181833417b92262fc13a62d59ce88d98756`
- DMG SHA-256:
  `f7b9c450120183f81e989cde9e42b6a0ac204f77adcf9eb4d25ee8110a6bd9ee`
- CLI receipt: `Omega 0.2.0 – /Applications/Omega Dev.app`
- Signature: ad hoc; deep strict verification passed.
- Previous development app backup:
  `/private/tmp/Omega-Dev-before-project-gated-thread-20260803-ab74fe4e.app`
- Unchanged production binary SHA-256:
  `0475b4f52bd0c79b53a9b4dfafd83a9ed081b7ee8858ba48966ead53ae5a5f73`

The focused projectless-startup, Command-N focus, delayed-front-door, and
Forensics-to-composer tests passed. `./script/clippy -p agent_ui -p ui`, Rust
formatting, the documentation build, and diff checks also passed. The app was
not launched, and no GUI automation was used for this receipt.

## Working-folder and Forensics-target build receipt {#working-folder-forensics-build-receipt}

On 2026-08-03, the working-folder navigation and selected Forensics-target
change was built from source commit
`2369dfb7b135196ba6b8a777b2bd3e0476859fb4`, whose parent was
`b08497a2de1bc752df1c7273fb2a7ad00deee84d`.

- Application: `/Applications/Omega Dev.app`
- Bundle identifier: `com.openagents.omega.dev`
- Bundle version: `20260803.045948`
- Short version: `0.2.0`
- Architecture: `arm64`
- Installed and bundled `omega` binary SHA-256:
  `2d87039ad9960b7a7749f91e336c8a7b5078773889f015e587512fb7c5880e33`
- DMG SHA-256:
  `cce8c1eb9c3f76806f12ec38ff71720447a2f5e3e59e36c8771af996fa97f6c0`
- CLI receipt: `Omega 0.2.0 – /Applications/Omega Dev.app`
- Signature: ad hoc; deep strict verification passed.
- Previous development app backups:
  `/private/tmp/Omega-Dev-before-working-folder-forensics-20260803-ab74fe4e.app`
  and `/private/tmp/Omega-Dev-replaced-working-folder-forensics-20260803.app`
- Unchanged production binary SHA-256:
  `0475b4f52bd0c79b53a9b4dfafd83a9ed081b7ee8858ba48966ead53ae5a5f73`

The complete `agent_ui` library suite, all 316 `omega_deltas` tests, the
selected-catalog-project and explicit-working-directory regressions, and the
working-folder focus and switching regressions passed. Clippy for `agent_ui`,
`ui`, and `omega_deltas`, Rust formatting, the documentation build, and diff
checks also passed. The app was not launched, and no GUI automation was used
for this receipt.

## Command-N Forensics release-fast receipt {#command-n-forensics-release-fast-receipt}

On 2026-08-03, the New Thread navigation fix was built directly from source
commit `e578d9e8a93e86a407308ea302dc35907ed962e4` with:

```sh
OMEGA_PRIMARY_INTERFACE_BUILD=1 \
  CARGO_TARGET_DIR=/Users/christopherdavid/work/omega/target \
  cargo build -p omega --profile release-fast
```

- Executable: `/Users/christopherdavid/work/omega/target/release-fast/omega`
- Architecture: `arm64`
- Executable SHA-256:
  `69e907d07221c631d0f88ccd8c7346d20b554c2e65d62087851ca2961e942a66`
- Embedded source commit:
  `e578d9e8a93e86a407308ea302dc35907ed962e4`
- Embedded interface contract: `Omega embedded primary-interface build is active`
- Signature: link-produced ad hoc signature; strict verification passed.
- Previous release-fast executable backup:
  `/private/tmp/omega-release-fast-before-command-n-20260803`
- Previous executable SHA-256:
  `c1bac8585f3cf58529f9c15b01cea867d5fac9966069e9a7d8f2d98a36455c4b`

The deferred-Forensics-restore regression and the window-global primary New
Thread keymap regression passed. All 317 `omega_deltas` tests and clippy for
`agent_ui` and `omega_deltas` passed. The complete `agent_ui` suite recorded
898 passes, one ignored test, and nine unrelated existing failures in executor
naming, routed-model copy, legacy front-door selectors, and WSL migration. The
release-fast executable was not launched, and no app bundle or installed
application was replaced.
