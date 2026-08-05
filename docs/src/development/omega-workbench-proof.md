# Deterministic Omega workbench proofs

The workbench proof harness lets you verify Omega's desktop UI without
controlling another application or synthesizing operating-system input. It
drives GPUI directly, inspects the rendered frame, and can compare the Metal
output with a committed baseline.

Use the harness for changes to agent threads, work surfaces, repository and
worktree context, messages, tool calls, plans, artifacts, or workbench chrome.
Do not use a screenshot as the only assertion for application state.

## Quick start {#quick-start}

List the registered scenes:

```sh
script/omega-workbench-proof --list
```

Use `--json` when another tool needs to consume the catalog:

```sh
script/omega-workbench-proof --list --json
```

Run one scene's semantic checks without capturing a PNG:

```sh
script/omega-workbench-proof \
  --scene omega_front_door_no_project \
  --semantic-only
```

Run its semantic preflight and Metal comparison:

```sh
script/omega-workbench-proof \
  --scene omega_front_door_no_project \
  --pixel-only
```

The pixel lane is the default. The compatibility command
`script/omega-visual-proof` forwards its arguments to
`script/omega-workbench-proof`.

The proof command, including `--list`, builds the release visual runner unless
you pass `--no-build`. Use `--no-build` only after running this command for the
same source revision:

```sh
cargo build --release \
  -p zed \
  --bin zed_visual_test_runner \
  --features visual-tests
```

## What a proof establishes {#proof-layers}

A workbench proof has separate state, semantic, interaction, persistence, and
pixel responsibilities:

1. A typed scene defines the input world.
2. State assertions confirm that the intended thread, repository, worktree, and
   surface are active.
3. The semantic probe confirms that the intended controls were rendered once,
   inside the expected layout, with accessible identities.
4. GPUI dispatches actions, keyboard input, pointer input, focus changes,
   resizes, and fake-time changes directly to the test window.
5. A restart scene reads production persistence in a second process.
6. The pixel lane captures the GPUI render texture and compares it with the
   scene's baseline.

Each layer catches a different failure. A plausible screenshot can still show
the wrong thread or worktree. A correct state reducer can still render a
clipped control. Keep both assertions.

The logical workbench reducer also has a bounded TLA+ model and an independent
runtime trace checker. See
[Workbench projection consistency](./workbench-consistency.md) for the exact
claims, bounds, trace schema, and reducer-to-model mapping. Those checks prove
neither GPUI semantics nor pixels; this harness attaches those remaining proof
layers. See
[Omega desktop workbench shell](./omega-desktop-workbench-shell.md) for the
rail, dock, focus, lifecycle, and native-adapter contracts those layers must
exercise.

## Typed scenes {#typed-scenes}

The shared scene types and proof records live in
`crates/omega_workbench_harness`. `WorkbenchScene` describes:

- viewport size and scale;
- dark or light theme and fake time;
- connectivity state: online, offline, reconnecting, or stale;
- content state: empty, loading, ready, or an error with a message;
- threads and the active thread;
- one optional project with zero or more repositories;
- each repository's worktrees, optional branch, dirty-file and conflict counts,
  and ahead/behind counts;
- user, assistant, and system messages in complete, streaming, or error state;
- pending, running, completed, or failed tool calls;
- pending, in-progress, completed, or blocked plan steps;
- file, diff, command, plan, and URL artifacts, optionally tied to a worktree;
- revisioned message, tool-call, artifact, repository, connectivity,
  persistence, route-decision, and executor-disclosure events;
- each work surface's availability and optional badge, the active surface, and
  whether the dock is open; and
- each Review session's thread, ACP session, repository, worktree, action-log
  checkpoint and generation, lifecycle, ordered files and hunks, statuses and
  ranges, selection, focus, pending work, rejected stale completions, and
  observed mutations with resulting temporary-repository contents;
- each Git snapshot's exact thread, repository, worktree, native repository
  entity and generation, lifecycle, branch and tracking state, ordered status
  entries, staging and conflict counts, selection, focus, pending operation,
  requested mutation results, badge agreement, and rejected stale refresh
  count; and
- the requested surface, dock state, revision, and mutations persisted across a
  cold restart.

Use logical fixture IDs such as `thread-a`, `repository-a`, and `worktree-a`.
Do not use random entity IDs, temporary folder names, timestamps, or values
obtained from a developer's data folder.

`WorkbenchScene::validate` rejects invalid fixtures before GPUI starts. It
checks duplicate and empty IDs, including worktree IDs across repositories;
references from messages, tool calls, plan steps, artifacts, and events to
missing threads; unavailable active surfaces; and an open dock without an
active surface. A repository requires the scene's project and at least one
worktree. Every repository must belong to that project. Each thread either has
no project context or identifies an existing project, repository, and worktree
together.

`PersistedSceneFixture::mutations_before_restart` supports:

- changing the active thread, active surface, dock-open state, or connectivity;
- completing one existing message or tool call; and
- advancing the persisted revision.

Mutation validation rejects references to missing threads, messages, or tool
calls and unavailable surfaces.

`WorkbenchScene::digest` serializes the validated fixture and records its
SHA-256 digest in the receipt. If two runs claim to use the same fixture, their
fixture digests must match.

The production Agent Panel adapter materializes repository fixtures in
`FakeFs`, waits for real Project and GitStore scans to complete, and projects
the resulting repository/worktree/head/status/tracking data through the same
thread-identity code used by the desktop header. It does not inject rendered
labels. Fixtures can create linked worktrees with one common repository,
folders without Git, unborn branches, named branches, and detached heads.
Repository- and worktree-picker tests open the retained native `ContextMenu`
and apply its production selection and confirmation actions. Branch tests open
the existing Git branch picker, inject a backend checkout failure, retain a
menu across a newly active turn, and prove a successful checkout advances the
binding generation so an earlier Git load is stale. A pending-session fixture
proves restored desired work directories select the correct root before an
`AcpThread` exists. Removal tests retain the last-known missing label and
recover through the rendered repository picker. Failed recovery remains
formally unbound and cannot revive the removed candidate.

Selection tests also inject a connection-level cwd rejection and prove that
binding, generation, metadata eligibility, and an old-target load are
unchanged. Busy and connection-phase tests prove both button and keyboard
actions are disabled, while the native terminal test proves `.` resolves to
the selected worktree and explicit project roots or absolute paths can select
another command directory. A real
multi-session partial-retarget test makes one session accept, another reject,
and the first reject rollback; it then proves the rendered projection is
`Inconsistent` and repository-bound actions stop. A recovery test reselects
the target, forces all sessions to reconcile, clears that phase, and advances
the content epoch.

Branch checkout tests introduce deterministic FakeGit latency. Across seeded
scheduler runs they prove that the pending checkout disables target controls
and repository-bound surfaces, makes the composer read-only without discarding
its text, and permits that held prompt only after checkout completes.

Four registered Metal scenes cover clean identity, dirty/conflicted and
ahead/behind identity, long labels in a 909-pixel window, and an offline
identity. Each captures the whole window plus a named `thread-identity` region
derived from `omega.workbench.thread-identity`. The semantic preflight requires
distinct interactive repository, worktree, and branch controls when
applicable, exact picker labels and candidate order, containment within the
toolbar, the offline status node, and full untruncated accessibility text in
the narrow scene. The clean identity scene also opens and closes the repository
picker before capture and requires the repository control to retain focus.

### Native Files scenes {#native-files-scenes}

Seven registered scenes exercise the native Project Panel after it becomes the
canonical Files work-surface entity:

| Scene                                               | Fixture and proof boundary                                                                                                                                                                                                                                                                                                                     |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `omega_workbench_files_wide`                        | A ready worktree at 1200 pixels. Proves the scoped native tree, selected path, accessible rows, and tree-to-surface containment.                                                                                                                                                                                                               |
| `omega_workbench_files_narrow`                      | The same ready tree at the exact 910-pixel minimum allocation. Proves the 240-pixel Files dock without changing semantic ownership.                                                                                                                                                                                                            |
| `omega_workbench_files_empty`                       | An empty active worktree. Proves an accessible scoped-empty status, no rendered file rows, and no fallback to another project root.                                                                                                                                                                                                            |
| `omega_workbench_files_loading`                     | Begins a generation-bound Files load after activation. Proves the active scope remains authoritative while the non-ready host hides the native tree.                                                                                                                                                                                           |
| `omega_workbench_files_error`                       | Completes that typed load with an error. Proves the error transition applies to the active host and does not expose stale native rows.                                                                                                                                                                                                         |
| `omega_workbench_files_multi_root`                  | Materializes alpha and beta roots while binding the thread to beta. Proves every native row belongs to beta and excludes alpha-only content.                                                                                                                                                                                                   |
| `omega_workbench_files_stale_filesystem_completion` | Selects distinct alpha and beta production bindings through the rendered identity pickers. It starts an alpha derivation, advances exactly one scheduler task while that derivation remains pending, then selects beta. Proves a newer beta binding generation and scope revision supersede alpha and that every alpha row selector is absent. |

The runner creates each fixture in an isolated `TempDir`, retains that owner
through the capture, adds its folders through the real Project worktree scan,
and loads one Workspace-created `ProjectPanel` before `AgentPanel`. Before any
pixel capture, it verifies that only that entity is registered for the fixture,
that the projected `WorktreeId` matches the active thread target, and that
every materialized row carries that scope. After the capture it removes the
fixture worktrees and drains their tasks before dropping the `TempDir`, so the
proof neither leaks fixture directories nor invalidates paths still owned by a
live worktree.

Ready scenes require `omega.project-panel.tree` to be an accessible `Tree`
inside `omega.workbench.surface.files`. Every rendered
`omega.project-panel.row.<worktree>.<entry>` must be a visible `TreeItem` with a
non-empty label, lie inside that tree, and contribute to exactly one selected
row. The empty scene instead requires
`omega.project-panel.scope.empty` as an accessible `Status`. Loading and error
scenes require the non-ready Files host to hide the native tree and its row
selectors.

Each scene captures the whole window and a named `files-surface` region derived
from `omega.workbench.surface.files`. The visual stale-completion scene uses
the production identity controls to establish distinct alpha and beta binding
epochs. It deliberately leaves the alpha Project Panel derivation in
`Loading`, records its revision and visible row selectors, and then selects
beta. Before capture it requires the beta projection generation and scope
revision to be newer, the beta-only path to be present, the alpha-only path to
be absent, and every recorded alpha selector to be absent from the rendered
tree.

That Metal scene proves deterministic derivation supersession; the portable
Agent UI test separately exercises a late filesystem notification. It pauses
`FakeFs` events for worktree A, rebinds to worktree B, flushes the delayed A
watcher event, and proves neither the old row selectors nor the late A path can
repopulate the native tree. Portable ownership tests also compare entity IDs
across first activation, collapse, reopen, and rebind so a plausible Files
screenshot cannot hide a second Project Panel. They also exercise the
Workspace's nonvisual rehome registry: project reveal, external activation,
toggle, close-dock, and exact-path File History must route to the one embedded
entity without reopening the legacy dock.

The portable suite treats hidden behavior as a semantic failure even when the
pixels look plausible. Preview open must keep native tree focus, permanent open
must reveal and focus the Workspace editor beside the retained transcript, and
File History must produce a visible focused graph. Loading, error, offline,
inconsistent, unbound, and missing-root states dispatch native mutating and
activation commands and require filesystem, selection, focus, and outer Agent
Panel ownership to remain unchanged. Cross-worktree tests record undo state
under A, rebind to B, and prove B cannot replay it; same-binding recovery
retains compatible tree state.

### Native Search scenes {#native-search-scenes}

Eight registered scenes exercise the search crate's native Project Search
entities inside the Search work surface:

| Scene                                   | Fixture and proof boundary                                                                                                                                 |
| --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `omega_workbench_search_empty`          | Opens the scoped native view with an empty query. Proves the query toolbar, landing state, focus target, and active beta binding.                          |
| `omega_workbench_search_populated`      | Searches known cross-file matches. Proves exact count and order, typed beta ownership, native query and options, and exclusion of ignored and alpha files. |
| `omega_workbench_search_no_results`     | Runs a valid query with no fixture match. Proves completed no-results state instead of stale rows or a fallback worktree.                                  |
| `omega_workbench_search_invalid_regex`  | Enables the native regex option and enters an invalid expression. Proves the accessible native query error and no result publication.                      |
| `omega_workbench_search_loading`        | Starts a generation-bound Search load. Proves the visible loading status hides the native toolbar and result content.                                      |
| `omega_workbench_search_narrow`         | Renders populated Search at the 910-pixel minimum allocation. Proves bounded toolbar/content layout without transcript or composer overlap.                |
| `omega_workbench_search_focused_result` | Selects a known native match through Search actions. Proves selected path and range, result focus, and existing editor-open navigation.                    |
| `omega_workbench_search_error`          | Completes the generation-bound load with an error. Proves the accessible error host and absence of interactive stale Search content.                       |

The disk fixture creates alpha and beta worktrees in an isolated `TempDir`.
Both contain deliberately conflicting search text. Beta also contains
cross-file matches, Unicode, a long line, and a Git-ignored match. The active
thread is bound to beta through the rendered identity controls before Search
opens. Semantic checks compare the production projection binding, native
Search `WorktreeId`, request generation, query and options, match paths and
ranges, selected match, and lifecycle state. A result from alpha is a failure
even when its text would make the screenshot look plausible.

The populated proof controls one pending alpha request, advances the
deterministic scheduler enough to keep that request in flight, then selects
beta and runs the final query. The runner requires the beta generation to
supersede alpha at the production projection boundary before releasing the old
completion. It then proves the native result snapshot contains only beta paths
and that the alpha host cannot replace or change the completed beta host. The
same native snapshot controls cover filters, replacement, option-triggered
reruns, cancellation, and recovery without screen-coordinate automation.

Ready scenes require accessible `omega.workbench.search.toolbar` and
`omega.workbench.search.content` targets inside
`omega.workbench.surface.search`. The invalid-regex scene additionally requires
`omega.workbench.search.query-error` as an accessible alert. Loading and error
scenes require the non-ready Search host to hide the toolbar, content, and query
error. Narrow scenes assert that the Search surface remains inside the
work-surface dock and disjoint from the transcript and composer.

Each Search scene captures the whole workbench and a named `search-surface`
region derived from `omega.workbench.surface.search`. A capture is recorded
only after semantic ownership, lifecycle, focus, accessibility, action, and
bounds checks pass.

### Native Review scenes {#native-review-scenes}

Nine registered scenes exercise the native Agent Diff pane and toolbar inside
the thread-bound Review work surface:

| Scene                                      | Fixture and proof boundary                                                                                                                                               |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `omega_workbench_review_empty`             | A valid checkpoint with no changed buffers. Proves the native empty lifecycle, no file/hunk projection, and no fallback to the foreign thread.                           |
| `omega_workbench_review_multi_file`        | Modified and added files with three ordered hunks. Proves exact file/hunk counts, statuses and ranges under the active beta checkpoint.                                  |
| `omega_workbench_review_selected_hunk`     | Selects the second hunk in a multi-file review. Proves native selection, diff focus, and editor-open routing without changing Review ownership.                           |
| `omega_workbench_review_streaming_update`  | Adds an incoming hunk while the agent edit stream is pending. Proves the surviving selected hunk remains selected and the lifecycle stays `Streaming`.                   |
| `omega_workbench_review_rename_delete`     | Projects one renamed file with its old path and one deleted file. Proves that non-modified statuses and zero-sided ranges survive the native snapshot.                   |
| `omega_workbench_review_conflict`          | Projects a conflicted file and conflict hunk. Proves conflict status, disabled unsafe mutations, and exact active-worktree ownership.                                    |
| `omega_workbench_review_all_reviewed`      | Drives keep/reject through native actions until no pending hunks remain. Proves `AllReviewed`, mutation counts, and resulting working-tree contents.                    |
| `omega_workbench_review_narrow`            | Renders the multi-file review at the 910-pixel minimum allocation. Proves the native toolbar and diff remain contained without overlapping transcript or composer.       |
| `omega_workbench_review_error`             | Publishes a generation-bound checkpoint error. Proves an accessible alert, no interactive stale diff, and retention of the typed binding needed for deterministic retry. |

Every Review fixture contains two logical threads, ACP sessions, worktrees, and
action-log checkpoints. Alpha contains `src/foreign_thread_only.rs`; beta owns
the visible scene. The active session's `ReviewBindingFixture` must match the
production `AgentDiffBinding` field for field, including checkpoint entity ID
and generation. The proof then compares lifecycle, ordered file and hunk
projection, selected path and hunk, focus owner, native mutation records,
pending-operation count, and ignored-stale-completion count. It separately
rejects any foreign-only path even when other counts happen to match.

The runner never derives expected Review state from visible text. It creates two
temporary Git repositories with committed base files, translates the production
Agent Diff test snapshot into the shared typed fixture, and calls
`prove_review_surface` before pixel capture. Fixture edits use a test-only
awaitable entry into the same action-log and `BufferDiff` path as production,
so the scheduler cannot turn an unobserved detached edit into an apparent
empty-state success. Keep/reject assertions read the repository working tree
after dispatching the native action, so incrementing a counter without applying
the authoritative mutation cannot pass.

The stale-completion proof starts an alpha generation, switches the active
thread/worktree/checkpoint to beta, and only then releases alpha. It requires
the alpha completion to be rejected by its checkpoint generation, the beta
entity and selection to remain unchanged, and alpha's path to be absent from
both native state and the rendered selector set. Seeded GPUI scheduler
iterations cover different completion, render, and focus task orderings. A
worktree-invalidation case removes the bound worktree and requires
`Invalidated`, zero actionable hunks, no mutation, and no retained focus inside
hidden native content.

Ready Review scenes require accessible
`omega.workbench.review.toolbar` and
`omega.workbench.review.content` targets inside
`omega.workbench.surface.review`. File and hunk selectors must be unique,
ordered, labeled, and expose selected/disabled state through accessibility
properties. Empty, error, offline, unavailable-checkpoint, and invalidated
states expose one accessible status or alert and hide stale interactive diff
and mutation controls. Narrow captures additionally require the Review surface
to remain inside the dock and disjoint from transcript and composer.

Each scene captures the whole workbench and a named `review-surface` region
derived from `omega.workbench.surface.review`. Baselines are generated only
after native identity, lifecycle, mutation, focus, accessibility, leak, stale
completion, and teardown assertions pass. Teardown invalidates the generation,
clears tracked buffers, removes the GPUI window and project worktrees, drains
scheduled work, and only then releases the temporary repositories.

### Native Git scenes {#native-git-scenes}

Twelve registered scenes exercise the retained native Git Panel under an exact
thread repository/worktree scope:

| Scene                                         | Fixture and proof boundary                                                                                                                        |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `omega_workbench_git_clean`                   | A clean named branch. Proves exact native repository entity, zero status rows, no badge, and retained panel ownership.                            |
| `omega_workbench_git_dirty`                   | Modified and untracked files with ahead/behind tracking. Proves ordered native rows, counts, header indicators, and rail badge agreement.          |
| `omega_workbench_git_staged`                  | A staged change. Proves native section/staging state, selection, commit-button validation, and exact mutation target.                             |
| `omega_workbench_git_conflict`                | A conflicted entry plus a cancelled destructive request. Proves conflict semantics and that cancellation leaves repository state unchanged.       |
| `omega_workbench_git_detached`                | A detached HEAD. Proves head classification without inventing a branch or losing the scoped repository.                                           |
| `omega_workbench_git_unborn`                  | A repository before its first commit. Proves the native unborn state and commit validation boundary.                                              |
| `omega_workbench_git_pending`                 | A held safe operation. Proves pending lifecycle, retained entity/state across collapse and reopen, and one recorded operation.                    |
| `omega_workbench_git_multi_repository`        | Workspace-global alpha with the thread scoped to beta. Proves every read, mutation, row, badge, and branch belongs to beta.                        |
| `omega_workbench_git_repository_removed`      | Removes the scoped repository after handoff. Proves the scope remains fail-closed, native rows disappear, and no neighboring repository is chosen. |
| `omega_workbench_git_offline`                 | Makes remote Git work unavailable while local status remains scoped. Proves the lifecycle hides native actions without treating it as project loss. |
| `omega_workbench_git_reconnect`               | Rejects an older lifecycle generation while remote Git reconnects. Proves stale completion cannot overwrite local rows or tracking state.           |
| `omega_workbench_git_error`                   | Surfaces a typed refresh failure. Proves an accessible alert, retained last-known branch, and deterministic retry identity.                         |

The runner creates isolated temporary Git repositories and loads the real
Workspace `GitPanel` before Agent Panel. It selects the target through the
production identity and `SelectGit` paths, then compares the panel's
`GitPanelStateSnapshot` with the scene's `GitSnapshotFixture`. Scope
repository, worktree, generation, resolved repository entity, head state,
ordered status rows and sections, counts, selection, pending operation, commit
validation, focus, and mutation records must agree before a frame can be
captured.

Multi-repository fixtures deliberately put recognizable foreign-only paths in
alpha while the thread owns beta. The proof rejects any foreign path before it
compares aggregate counts, so equal dirty-file totals cannot conceal an
incorrect global-repository fallback. It separately requires the thread
header, Git rail badge, native scope, and normalized panel state to identify
the same logical repository/worktree and binding generation.

Portable front-door tests dispatch native stage, unstage, open-diff, commit,
branch, and discard actions against fake Git backends and inspect recorded
calls. Destructive fixtures prove both cancellation and backend failure leave
status, selection, and files unchanged. The portable open-diff proof checks
the resulting Workspace item's exact project path and worktree ID. The visual
proof additionally requires the diff item's synchronous repository ID to equal
the thread-scoped repository and rejects any materialized active path from the
foreign worktree, then restores the retained Git surface before capture.
Together these checks catch same-named paths being resolved through a foreign
Workspace-global repository. Collapse/reopen compares the panel, surface,
commit editor, and selected-entry identity rather than relying on a similar
screenshot.

The stale-refresh proof holds alpha's debounced status or repository task,
switches to beta, advances the scope generation, and then releases alpha.
Acceptance requires beta's entity and selected row to remain unchanged,
alpha's completion to increment the rejected-stale count, and all alpha-only
selectors to remain absent. Repository removal uses the explicit unavailable
scope; `None` is never interpreted as permission to follow the
Workspace-global active repository after handoff. Remote-offline and
reconnecting lifecycles instead preserve the available local scope and its
last status/tracking projection while rejecting unsafe remote work.

Actionable scenes require the native Git content inside
`omega.workbench.surface.git`, with unique ordered status-row semantics and
the expected focus owner. Offline, reconnecting, removed, and error scenes
require `omega.workbench.git.lifecycle` and no interactive stale panel.
Every scene captures the whole workbench plus the named `git-surface` region
from `WORKBENCH_GIT_REGIONS`; baselines are admitted only after typed identity,
mutation, focus, accessibility, safety, no-leak, and teardown assertions pass.

### Native Terminal scenes {#native-terminal-scenes}

The typed catalog defines exactly nineteen Terminal scene contracts:

| Scene | Contract |
| --- | --- |
| `omega_workbench_terminal_empty` | The one Workspace-owned panel is mounted with no tabs, pending creation, process, or badge. Selecting Terminal performs no implicit spawn. |
| `omega_workbench_terminal_starting` | One explicit creation is pending with the immutable owner captured by its request and contributes to the running badge. |
| `omega_workbench_terminal_running` | A native terminal has a stable item/view/process identity, exact initial cwd and immutable creation owner, with the badge derived from native running state. |
| `omega_workbench_terminal_typed_input` | The focused native `TerminalView` receives the exact deterministic input bytes; transcript focus prevents subsequent bytes from reaching it. |
| `omega_workbench_terminal_multiple_tabs` | Multiple terminals share one native pane with stable order, unique IDs, one active tab, and immutable owner records. |
| `omega_workbench_terminal_split` | Native pane splitting preserves exact pane membership, active pane/tab identity, and each terminal's owner and content. |
| `omega_workbench_terminal_exited` | An exited terminal retains its output and identity but no longer contributes to the running count. |
| `omega_workbench_terminal_failed_to_spawn` | A failed explicit creation has a non-empty error, balances the pending count, creates no running process, and remains attributable to its request binding. |
| `omega_workbench_terminal_hidden_running` | A running terminal remains alive and counted while the retained Terminal dock is collapsed. |
| `omega_workbench_terminal_collapse_reopen` | Collapse and reopen preserve the panel, host, pane, terminal, owner, selection, and output identities without spawning or killing. |
| `omega_workbench_terminal_focus_return` | Opening focuses native terminal content; the explicit workbench return action restores transcript/composer keyboard ownership. |
| `omega_workbench_terminal_worktree_removed` | Removing the target disables new creation while retaining existing output and the original terminal owner; no neighboring worktree is selected. |
| `omega_workbench_terminal_offline` | Offline project state retains local terminal entities/output and blocks new creation instead of treating the process as disconnected state. |
| `omega_workbench_terminal_reconnecting` | Reconnecting retains the native panel and owners, blocks new creation, and rejects an older lifecycle generation. |
| `omega_workbench_terminal_thread_switch` | The next-creation binding follows a worktree identity switch while already-created terminals retain their original thread/worktree owners. |
| `omega_workbench_terminal_stale_spawn` | A completion captured under an older binding is counted as stale and cannot relabel or replace current terminal state. |
| `omega_workbench_terminal_foreign_spawn_rejected` | A request for a binding other than the authoritative creation target is rejected rather than redirected to a convenient cwd. |
| `omega_workbench_terminal_error` | A typed owner error is accessible, retains existing output and owners, disables creation, and does not expose stale commands as successful. |
| `omega_workbench_terminal_narrow` | At a 910×720 viewport, the native Terminal surface and New control remain fully visible and disjoint from the transcript and composer. |

The model fixture distinguishes the current creation binding from every
process's creation owner. `prove_terminal_surface` compares panel identity,
lifecycle, pane layout, ordered tab membership and selection, process/item
identities, cwd, process lifecycle, exact input bytes, spawn results, pending
and running counts, badge agreement, focus owner, stale/foreign rejection
counts, and owner immutability. Fixture validation rejects an implicit spawn,
owner relabel, pane leak, or badge mismatch before GPUI rendering is involved.

The rendered lane deliberately uses display-only `Terminal` entities so it can
exercise the real `TerminalPanel`, terminal emulator, panes, tabs, focus, and
surface ownership without launching a shell. Consequently, its
running/exited/failed text is a controlled projection of the typed fixture
rather than an observation of an operating-system process. The portable
front-door lane drives deterministic running and exited lifecycle overrides
through the production `TerminalPanel` snapshot and proves that Agent Panel's
derived badge clears. It also dispatches focus return, lifecycle transitions,
disabled New/Split callbacks, and removed-worktree host retention. The
independent typed checker covers pending/running/exited/failed lifecycle,
stale-generation rejection, and foreign-spawn rejection. A scene receipt
should be read as the combination of those layers, not as evidence that its
display-only terminal acquired a PID.

Portable front-door tests then mount the production Agent Panel and retained
`TerminalPanel`. They dispatch the production Terminal selection, collapse,
focus-return, explicit-create, and native pane paths. A TerminalPanel-owned
test factory makes that same explicit-create path record the requested cwd and
return a display-only terminal, proving the production action passes the
canonical active worktree and records immutable ownership without starting a
shell. It can hold completion across a worktree switch and then
deterministically succeed or fail, proving pending-badge balance, synchronous
removal of stale completions, and UI-visible failure propagation. Display-only
terminals receive output through the real terminal emulator and are inserted
through native tab/split APIs. This makes output retention, selection, focus,
and byte-level input assertions deterministic. The seam must not be described
as proof that an operating-system process started.

Run the portable model and front-door layers with:

```sh
cargo test -p omega_workbench_harness terminal_
cargo test -p agent_ui --features test-support native_terminal_
cargo test -p terminal_view --features test-support \
  test_display_terminal_insertion_and_split_are_deterministic
```

Inspect registration and run all registered semantic scenes with:

```sh
script/omega-workbench-proof --list
script/omega-workbench-proof --semantic-only
```

To isolate a Terminal scene, pass its exact catalog name:

```sh
script/omega-workbench-proof \
  --scene omega_workbench_terminal_collapse_reopen \
  --semantic-only
```

The nineteen catalog entries define contracts; they do not by themselves
prove that semantic execution succeeded or that reviewed pixel baselines are
committed. Confirm a scene's receipt before making either claim. Run its pixel
lane only when a baseline has been reviewed and admitted:

```sh
script/omega-workbench-proof \
  --scene omega_workbench_terminal_collapse_reopen \
  --pixel-only
```

Application restart is deliberately outside the live-process retention claim.
Native persistence may recreate eligible terminal layout and cwd by starting a
new shell; it cannot reconnect to the pre-restart operating-system process,
and task terminals are not serialized.

### Native Plan scenes {#native-plan-scenes}

The Plan lane is a typed projection proof, not a screenshot of markdown that
resembles a checklist. Every scene feeds `acp::SessionUpdate::Plan` through the
active `AcpThread`, then reads the retained `NativePlanSurface` snapshot before
the semantic or pixel boundary.

| Scene | Contract |
| --- | --- |
| `omega_workbench_plan_empty` | A mounted Plan surface at revision zero exposes an accessible no-plan state and no step rows. |
| `omega_workbench_plan_active` | Ordered completed, in-progress, and pending entries retain their typed statuses and priorities; the in-progress entry is the active step. |
| `omega_workbench_plan_replacement` | A full ACP replacement preserves existing positional step identities, updates labels/status, and assigns only appended entries a new identity. |
| `omega_workbench_plan_all_complete` | An all-completed current plan has an explicit all-complete summary rather than being mistaken for empty or historical state. |
| `omega_workbench_plan_historical` | Snapshotting a completed plan clears the live projection, retains typed historical entries, and gives every historical step a transcript source. |
| `omega_workbench_plan_interrupted` | An interrupted agent run retains the last good steps under an accessible alert. |
| `omega_workbench_plan_stale` | Offline/stale projection state retains the last good steps, selection, and revision while warning that the data may be stale. |
| `omega_workbench_plan_reconnecting` | Reconnect state retains the same projection and rejects an older lifecycle generation. |
| `omega_workbench_plan_malformed` | A provider update containing a blank step is rejected, surfaced as an accessible alert, and cannot replace the last good plan. |
| `omega_workbench_plan_no_source_navigation` | Selecting a live step records the explicit no-source result instead of inventing a transcript target. |
| `omega_workbench_plan_collapse_reopen` | Collapse and reopen preserve the exact Plan surface entity, revision, ordered step identities, and selection. |
| `omega_workbench_plan_narrow_foreign_binding` | At the narrow viewport, the active thread's Plan remains fully visible and disjoint from transcript/composer while a foreign binding is rejected and cannot leak steps into the visible projection. |

The portable seeded tests are authoritative for scheduler-sensitive update
ordering, stable IDs, thread switching, stale/foreign rejection, lifecycle
retention, and collapse/reopen identity. They run the production ACP session
update path and compare the surface's binding, revision, active step, ordered
current and historical entries, source indices, selection, navigation status,
and rejected-update count. A provider revision is not present in the ACP Plan
payload, so ordering within one live ACP session is the order accepted by
`AcpThread`; the client can reject an older local projection revision or a
foreign retained-thread event, but must not claim a server-supplied Plan
revision that the protocol does not carry.

The visual runner adds a second boundary. Before capture it proves the typed
snapshot against `PlanSnapshotFixture`, then requires unique Plan selectors,
the native content inside `omega.workbench.surface.plan`, an accessible summary
and list, accessible lifecycle or navigation status where applicable, and no
empty-state/step-row contradiction. The narrow scene also proves the Plan
surface is fully visible and does not overlap the transcript or composer.
Every scene captures the full workbench and the selector-derived
`plan-surface` region from `WORKBENCH_PLAN_REGIONS`.

Run the portable model and production front-door layers with:

```sh
cargo test -p omega_workbench_harness plan_
cargo test -p agent_ui --features test-support native_plan_
```

Inspect registration and run all Plan scenes semantically with:

```sh
script/omega-workbench-proof --list | grep omega_workbench_plan_
for scene in $(script/omega-workbench-proof --list --no-build | rg '^omega_workbench_plan_' | cut -f1); do
  script/omega-workbench-proof --scene "$scene" --semantic-only
done
```

To reproduce one scheduler seed or inspect one reviewed pixel baseline:

```sh
script/omega-workbench-proof \
  --scene omega_workbench_plan_replacement \
  --semantic-only \
  --seed 37

script/omega-workbench-proof \
  --scene omega_workbench_plan_replacement \
  --pixel-only
```

The catalog and region contract do not prove that a run passed. Require the
scene receipt, non-empty semantic checks, and a reviewed matching baseline
before treating the pixel lane as evidence.

> Note: the native artifact and event outline lane (`ThreadOutline`) and its
> fourteen `omega_workbench_outline_*` scenes were removed at owner direction
> 2026-07-30 (delta OMEGA-DELTA-0188). None of those scenes had committed
> pixel baselines.

### Registering a scene {#registering-a-scene}

Add every named scene to `HERMETIC_SCENES`. A `SceneSpec` defines:

- its unique name;
- whether it runs in the recording or restart process;
- its viewport and fixture version;
- its pixel threshold, channel tolerance, and rationale; and
- any named pixel regions.

Keep scene names stable after committing a baseline. The name is part of the
command-line interface, artifact path, receipt path, and baseline filename.

The catalog rejects duplicate names, invalid pixel policies, and invalid base
fixtures. Scene selection also rejects unknown names, incomplete shard
arguments, out-of-range shard indexes, and empty shards.

When you add a feature scene:

1. Build its typed fixture from fake or isolated services.
2. Assert the application state that makes the rendering meaningful.
3. Add stable GPUI selectors and accessibility metadata to the relevant
   controls.
4. Add semantic layout and interaction assertions.
5. Register the screenshot only after the semantic preflight passes.
6. Add a restart scene if the behavior crosses a process boundary.

Do not assemble a parallel fixture from source-text searches. The fixture must
drive the same entities and persistence edges as the product.

## GPUI semantic targets {#semantic-targets}

Attach a debug selector to the interactive element that owns the target:

```rust
div()
    .id("omega.workbench.control.git")
    .debug_selector(|| "omega.workbench.control.git".to_string())
    .role(accesskit::Role::Button)
    .aria_label("Git")
```

For selected, expanded, or toggled controls, also set the corresponding ARIA
state. The element ID, selector, role, and label should identify the same
control. Name interactive controls `omega.workbench.control.<name>` so the
Metal preflight can require that identity. Use other `omega.workbench.*`
selectors for non-interactive layout targets.

Selectors are enabled by GPUI's `test-support` feature and are no-ops in normal
release builds. Use names that describe the workbench role, not an icon or its
screen position. A generic selector such as `ICON-Plus` is not stable when a
screen contains more than one plus icon.

Include the fixture's stable logical ID when ownership matters. For example,
use separate targets for `workbench-thread:thread-a` and
`workbench-thread:thread-b`, then compare the rendered target with the scene's
active thread, repository, worktree, or surface. Do not infer ownership from a
row index or screen position.

`DebugRenderSnapshot` records every occurrence of a selector. Each occurrence
includes:

- full and visible bounds;
- whether it is visible, partially clipped, fully clipped, or transparent;
- whether it is hit-testable;
- whether it is focusable;
- whether it owns focus; and
- whether it contains the focused descendant.

Duplicate selectors are failures. Do not select the last matching bounds.

Use `SemanticProbe` for common assertions:

- `require_unique` confirms that one target rendered;
- `require_absent` confirms that a target did not render;
- `require_visible` rejects fully clipped or transparent targets;
- `require_interactive` also requires a hit-testable, focusable target;
- `require_focus` checks whether the target owns focus;
- `require_inside` catches overflow and offscreen layout;
- `require_disjoint` catches overlap; and
- `require_accessible` checks a unique element ID, role, and label in the
  accessibility tree.

Call `set_debug_accessibility_active(true)` before reading accessibility
semantics. GPUI refreshes the frame and builds the same AccessKit tree used by
assistive technology.

Accessibility JSON contains diagnostic frame metadata and ephemeral AccessKit
IDs. Do not put the raw tree in a deterministic receipt.
`normalized_accessibility_nodes` keeps stable element IDs and ARIA properties
and rejects duplicate accessible IDs. Assert selected, expanded, checked, and
disabled state from those normalized ARIA properties when the control exposes
that state.

## Deterministic interaction {#deterministic-interaction}

Use `VisualTestContext` for portable GPUI interaction tests and
`VisualTestAppContext` when the test also needs the Metal renderer. Both paths
dispatch input to the in-process GPUI window.

Use these APIs instead of calling a view's implementation method:

- `dispatch_action` for a GPUI action;
- `simulate_keystrokes` and `simulate_input` for focused input;
- `simulate_mouse_move`, `simulate_mouse_down`, and `simulate_mouse_up` for
  pointer behavior;
- `simulate_click_selector` for a unique, visible, hit-testable target;
- `simulate_resize` or a fixed-size test window for responsive layout; and
- `set_debug_accessibility_active` for accessibility assertions.

Use direct entity updates to construct a fixture, not to stand in for the
interaction being tested. After an interaction, assert both application state
and rendered semantics.

`simulate_click_selector` refuses missing, duplicate, fully clipped,
transparent, or non-hit-testable targets. The Metal context uses
result-returning input helpers so a closed test window cannot turn an
interaction into a silent no-op.

`WorkbenchInteractionDriver` provides the shared helpers for selecting a rail
item, opening or collapsing the dock, switching threads or worktrees, focusing
a surface, and requesting a restart. Its portable and Metal GPUI backends click
stable selectors and run the deterministic scheduler to quiescence. Feature
tests must then wait for their typed state predicate and assert rendered
semantics. The GPUI backends deliberately reject `restart()`:
`script/omega-workbench-proof` supplies the restart backend by launching a
second process, because an in-process helper cannot prove a cold launch.

### Fake time and quiescence {#fake-time}

Run asynchronous work on GPUI's foreground and background executors. Use a GPUI
executor timer:

```rust
cx.background_executor().timer(duration).await;
```

Do not use `smol::Timer`, `std::thread::sleep`, or a wall-clock timeout in a
deterministic scene. Advance fake time explicitly, then run the scheduler until
the documented state predicate is true or the test reaches its scheduler-step
budget.

Use `run_until_parked` only when every service in the scene is fake or isolated
and is expected to park. A real child process or permanently runnable transport
needs a bounded state wait.

## Seeds and iteration sweeps {#seeds}

The seed controls GPUI scheduler interleavings. The default seed is `0`.

Run one known seed:

```sh
script/omega-workbench-proof \
  --scene omega_front_door_typing \
  --semantic-only \
  --seed 37
```

Run consecutive seeds:

```sh
script/omega-workbench-proof \
  --scene omega_front_door_typing \
  --semantic-only \
  --seed 0 \
  --iterations 20
```

With multiple iterations, each seed writes under
`target/omega-workbench-proof/seed-<seed>`.

For ordinary `#[gpui::test]` tests, you can also use `SEED` and `ITERATIONS`:

```sh
SEED=37 cargo test -p agent_ui test_name -- --nocapture
ITERATIONS=100 cargo test -p agent_ui test_name -- --nocapture
```

When a sweep fails, rerun only the reported seed before changing the test.

## Semantic and Metal lanes {#proof-lanes}

| Lane              | Renderer                 | Primary assertions                                     | Platform        |
| ----------------- | ------------------------ | ------------------------------------------------------ | --------------- |
| `#[gpui::test]`   | GPUI test platform       | State, focus, actions, layout, scheduler behavior      | Portable        |
| `--semantic-only` | Workbench visual runner  | Typed preflight and registered semantic checks         | Currently macOS |
| `--pixel-only`    | Real GPUI Metal renderer | Semantic preflight plus whole-window and region pixels | macOS           |

The semantic-only command skips PNG capture and comparison. It does not permit
an empty semantic result. A selected scene that never reaches its semantic
boundary produces a failed receipt.

The pixel lane uses `VisualTestAppContext` with real assets and fonts. It
captures `Window::render_to_image`, not the desktop or system window chrome. It
does not require Screen Recording permission and does not move the pointer or
type into the foreground application.

## Receipts and artifacts {#receipts}

The default output root is `target/omega-workbench-proof`. Override it with:

```sh
script/omega-workbench-proof --output target/my-proof
```

Each selected scene writes `receipt.json`. A pixel run can also produce this
evidence:

```text
target/omega-workbench-proof/
└── scenes/
    └── <scene>/
        ├── receipt.json
        ├── baseline.png                # committed comparison source copied here
        ├── current.png                 # after pixel capture
        ├── diff.png                    # failed whole-window comparison
        └── regions/
            ├── <region>_baseline.png   # committed region source copied here
            ├── <region>.png            # configured named region
            └── <region>_diff.png       # failed region comparison
```

The semantic lane writes `"pixel": null` in the receipt and no PNG artifacts.
Pixel artifacts are written when the scene reaches capture. `diff.png` and
`<region>_diff.png` exist only for comparisons that fail their configured
threshold.

The receipt schema is `openagents.omega.workbench-proof.v1`. It records:

- scene and fixture digest;
- scheduler seed;
- lane and viewport;
- every named semantic check;
- pixel policy, match percentage, and changed/total pixel counts;
- baseline, current, optional diff, and named-region results and paths; and
- the final outcome.

Artifact paths are always relative and cannot contain parent traversal.
All receipt artifact paths are relative to the command's output root. The
committed comparison sources remain under
`crates/omega/test_fixtures/visual_tests` and are copied into the output root
when they are available. A pixel receipt without a pixel result, a receipt with
zero semantic checks, or an outcome that disagrees with its semantic,
whole-window, or named-region checks is invalid.

## Cold restart {#cold-restart}

`script/omega-workbench-proof` starts with a recording process for each seed:

1. The recording process renders recording scenes and writes any
   production-format persistence needed by restart scenes into an isolated data
   folder.
2. The command then starts a second process with empty process state. It reuses
   only that isolated data folder and renders restart scenes. This phase still
   runs when the recording process fails so selected restart scenes emit
   failure receipts instead of disappearing from the proof artifact.

The script creates a new data folder for each seed and removes it afterward. It
never uses your Omega data folder.

Do not replace the second process with another `TestAppContext` or another
window in the first process. That would retain process globals, static caches,
entities, and tasks and would not prove deserialization after launch.

When a scene does not depend on restart, the second process has no matching
restart scene and performs no capture for that scene. Restart scene receipts
come only from the second phase.

Scene filtering also bounds the recording journey. The runner performs only
the setup and interactions needed to reach the latest selected recording
scene, then tears its window down. Selecting a restart scene keeps the whole
recording prerequisite journey, writes the production-format handoff, and
launches the restart process. Assertions owned by an unrelated later scene
cannot fail a filtered proof.

## Pixel baselines {#pixel-baselines}

Omega's workbench baselines are committed in
`crates/omega/test_fixtures/visual_tests`. The default policy requires at least
99% matching pixels and permits a per-channel difference of `2`. Each scene can
declare another policy, but it must include a rationale.

The authoritative baselines use the Apple Silicon Metal renderer. A different
GPU, operating-system font rasterizer, scale, or font set can produce a
different image. Do not update a baseline to make an unexplained failure green.

When a UI change intentionally changes a scene:

1. Run the semantic lane and review its receipt.
2. Run the pixel lane and inspect `current.png` and `diff.png`.
3. Confirm that the fixture, viewport, fonts, theme, and renderer are expected.
4. Update only the affected baseline:

   ```sh
   script/omega-workbench-proof \
     --scene omega_front_door_no_project \
     --pixel-only \
     --update
   ```

5. Review the committed PNG and rerun without `--update`.

Baseline updates are disabled when `CI` or `GITHUB_ACTIONS` is set. `--update`
also rejects semantic-only and multi-iteration runs.

## Sharding and local gates {#ci}

Select one deterministic catalog shard with both arguments:

```sh
script/omega-workbench-proof \
  --pixel-only \
  --shard-index 0 \
  --shard-count 4
```

The catalog order defines the shard assignment. The harness rejects an empty or
out-of-range shard.

Omega contains no GitHub Actions. The former gate lanes are preserved by the
repository-owned runner:

```sh
script/omega-workbench-checks
```

Pass `model`, `portable`, or `metal` to run one lane. The model lane exhausts
the bounded model and runs reducer conformance. The portable lane runs the
harness, production Agent UI adapter, deterministic GPUI seed sweep, and
pending-task and retained-entity probes. The Metal lane requires Apple Silicon
and runs both pixel shards at seed `0`, writing evidence under
`target/omega-workbench-proof/shard-<n>`. External infrastructure may invoke
this script, but repository automation must not live in `.github/workflows`.

The broader local gate is `script/omega-checks`. It preserves formatting,
clippy, workspace-nextest, and workbench entry points without relying on a
specific automation host.

Use these focused checks while developing the harness:

```sh
script/omega-workbench-model
cargo test -p omega_workbench_harness
cargo test -p omega_workbench_harness --features gpui-support
```

Automated gates must not use `--update`. Missing scenes, duplicate scene names, zero semantic
assertions, skipped captures, and invalid receipts are failures rather than
successful skips.

## Debugging failures {#debugging}

The proof command prints an exact single-seed reproduction command when a run
fails. It uses the failing seed, omits `--iterations`, points `--output` at that
seed's evidence folder, and preserves the selected scene, lane, shard, and
baseline-update mode.

Use a separate output folder to retain one investigation:

```sh
script/omega-workbench-proof \
  --scene omega_front_door_typing \
  --seed 37 \
  --output target/workbench-seed-37
```

For a scheduler that reports pending work:

```sh
SEED=37 PENDING_TRACES=1 \
  cargo test -p agent_ui test_name -- --nocapture
```

Use `DEBUG_SCHEDULER=1` for fake-clock and scheduler diagnostics. Use
`LEAK_BACKTRACE=1` when an entity handle survives scene teardown.

Common failure categories:

- **Unknown scene:** Run `--list` and use the registered name.
- **Duplicate selector:** Give each rendered target a stable semantic ID.
- **Not visibly hit-testable:** Check clipping, opacity, overlays, and whether
  the selector is on the interactive element.
- **Missing accessibility node:** Add an element ID, role, and label, then
  activate accessibility before taking the snapshot.
- **Fixture digest changed:** Review the typed input state before accepting new
  evidence.
- **Pixel dimensions differ:** Check the registered viewport and scale.
- **Pixel mismatch:** Inspect the current and diff images before considering a
  baseline update.
- **Parking forbidden or leaked entity:** Check detached tasks, unclosed
  windows, retained entity handles, and timers outside the GPUI executor.

Computer Use can be a final packaged-application smoke test. It is not a
substitute for these deterministic checks.

## Next steps {#next-steps}

See [Building Zed for macOS](./macos.md#visual-regression-tests) for Metal
requirements. Use the workbench scene catalog as the shared proof surface when
adding desktop workbench features.
