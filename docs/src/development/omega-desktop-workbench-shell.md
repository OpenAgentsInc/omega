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

Issue 128 installs the production rail and dock boundary, normal GPUI actions,
per-thread reducer integration, retained generic hosts, typed badge plumbing,
accessibility semantics, and deterministic tests. The generic host currently
renders a loading, ready, error, or offline placeholder. It does not yet render
the workspace's native Files, Search, Review, Git, Terminal, or Plan entity.

The remaining work is deliberately split so each native adapter can prove its
own identity, behavior, and lifecycle:

| Follow-up                                                      | Responsibility                                        |
| -------------------------------------------------------------- | ----------------------------------------------------- |
| [Issue 129](https://github.com/OpenAgentsInc/omega/issues/129) | Mount the existing Project Panel as Files             |
| [Issue 130](https://github.com/OpenAgentsInc/omega/issues/130) | Present the thread's typed plan                       |
| [Issue 131](https://github.com/OpenAgentsInc/omega/issues/131) | Persist and cold-restore each thread's selection      |
| [Issue 132](https://github.com/OpenAgentsInc/omega/issues/132) | Mount the existing Git Panel                          |
| [Issue 133](https://github.com/OpenAgentsInc/omega/issues/133) | Supply authoritative repository and worktree identity |
| [Issue 134](https://github.com/OpenAgentsInc/omega/issues/134) | Mount an embedded project-search entity               |
| [Issue 136](https://github.com/OpenAgentsInc/omega/issues/136) | Mount a thread-bound review entity                    |
| [Issue 137](https://github.com/OpenAgentsInc/omega/issues/137) | Mount the existing Terminal Panel without spawning    |

Until those adapters land, the shell is a retained-host foundation rather than
a replacement for the existing native panels.

## Composition boundary {#composition-boundary}

The Agent Panel builds the toolbar, transcript, composer, drag target, and
legacy terminal content once. The shell wraps that completed content in one
horizontal allocation:

1. The existing threads sidebar or its collapsed rail.
2. The 40-pixel workbench activity rail.
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

## Surface registry and identity {#surface-registry}

The registry contains exactly six stable surface identities in this order:
Files, Search, Review, Git, Terminal, and Plan. Files, Search, Review, Git, and
Terminal require a repository/worktree binding. Plan requires an active thread
but no repository.

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
before detaching a host from presentation.

## Responsive allocation {#responsive-allocation}

One allocator decides both the existing threads sidebar and the work-surface
dock. The current dimensions are:

| Allocation                      | Size       |
| ------------------------------- | ---------- |
| Threads sidebar, expanded       | 280 pixels |
| Threads sidebar, collapsed rail | 30 pixels  |
| Workbench activity rail         | 40 pixels  |
| Transcript reservation          | 600 pixels |
| Work-surface dock minimum       | 240 pixels |
| Work-surface dock default       | 320 pixels |
| Work-surface dock maximum       | 480 pixels |

At the default dock width, an expanded threads sidebar, activity rail, dock,
and reserved transcript fit at 1,240 pixels. Below that width, the threads
sidebar collapses before the work-surface dock. The dock can shrink to 240
pixels and remains present through 910 pixels:

```text
30 thread rail + 40 activity rail + 240 dock + 600 transcript = 910
```

Below 910 pixels, the shell applies a real dock-collapse transition and returns
focus to the transcript. It does not merely hide a logically open dock. When
the window widens again, the dock stays collapsed until the person explicitly
reopens it. The retained host is then reused.

At widths too small for the 600-pixel transcript reservation, both rails remain
visible, the dock remains closed, and the transcript receives the remaining
width. The reservation controls when optional columns collapse; it is not an
application minimum width. The composer must remain reachable and columns must
not overlap.

The dock's right edge is an accessible vertical splitter. Drag it to resize the
dock, or double-click it to return to the 320-pixel default. The splitter and
the host model both use the shared allocator, which clamps the result to 240
through 480 pixels without taking the transcript below its 600-pixel
reservation.

## Thread and binding lifecycle {#thread-and-binding-lifecycle}

Switching threads uses the Agent Panel's existing base-view switch boundary.
The shell opens a reducer thread on first use or switches to its existing
projection. Each thread retains its requested surface and dock state in memory.

A repository or worktree change reconciles through reducer transitions. If the
previous effective surface is no longer valid while its dock is open, the shell
collapses the dock and focuses the transcript. It does not guess another
repository or silently open another surface.

Closing a thread:

1. Applies `CloseThread`.
2. Removes every retained host keyed to that thread.
3. Cancels or generation-gates pending work.
4. Returns focus to the transcript when no active thread remains.

Closing the window drops the shell and its retained generic hosts. Native
workspace entities added by later adapters remain owned by their existing
Workspace lifecycle; the shell must not create a second repository, terminal,
search, diff, or plan store.

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

The generic host defines these content states for adapters and tests. Issue 128
does not yet connect every desktop transport event or native surface status to
them. Native adapter issues must add those production feeds instead of using
the rail as an inferred connection indicator.

## Deterministic test contract {#deterministic-test-contract}

The shell is tested through the same public action and semantic paths used by
the product. Tests must not call the selection implementation directly to stand
in for a click or keybinding.

Stable selectors include:

| Target           | Selector                                 |
| ---------------- | ---------------------------------------- |
| Workbench root   | `omega.workbench.root`                   |
| Transcript       | `omega.workbench.transcript`             |
| Activity rail    | `omega.workbench.activity-rail`          |
| Rail item        | `omega.workbench.control.rail.<surface>` |
| Dock             | `omega.workbench.dock`                   |
| Collapse control | `omega.workbench.control.dock.collapse`  |
| Resize splitter  | `omega.workbench.control.dock.resize`    |
| Hosted surface   | `omega.workbench.surface.<surface>`      |
| Typed badge      | `omega.workbench.badge.<surface>`        |

For every interaction, assert logical state and rendered semantics:

- the active thread, binding, requested surface, effective surface, and dock;
- exact GPUI focus ownership;
- a unique accessible ID, role, label, state, and unavailable description;
- containment, disjoint columns, clipping, and composer reachability;
- transcript entity identity before and after surface changes;
- host entity identity across collapse and reopen;
- independent selections across thread switches;
- stale completion rejection after thread or binding changes; and
- host release after thread and window teardown.

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
- two threads with independent requested surfaces;
- an active surface invalidated after it opens;
- a stale completion after switching surface, thread, or worktree;
- host construction failure and offline, loading, and content-error states; and
- thread and window teardown with weak-entity leak probes.

Record whole-workbench and rail/dock region baselines for the default, active,
focus-visible, badge, unavailable, narrow, and collapsed states. Loading and
error behavior is covered by semantic state-transition tests. A baseline is
added only after its semantic scene passes.

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
