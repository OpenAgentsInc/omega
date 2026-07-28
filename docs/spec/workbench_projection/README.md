# Workbench projection model

This directory contains a bounded TLA+ model of the state machine that decides
which desktop work surface is visible, which repository/worktree it is bound
to, which thread owns its outlines and actions, and what survives persistence,
cold restore, and reconnect.

The model is intentionally below the GPUI layer. It does not model pixels,
layout, message contents, paths, terminal output, or individual Files/Search/
Review implementations. It models the identity and revision values that those
views must consume. `Files`, `Git`, and `Terminal` stand in for surfaces that
require a repository/worktree binding; `Plan` stands in for an unbound surface.
The checked-in Rust reducer has two additional binding-required surfaces,
`Search` and `Review`, which are equivalent to `Files` for these properties.

## State-machine contract

`WorkbenchProjection.tla` makes the following behavior explicit:

- Opening a thread preserves the current active thread. When there is no active
  thread, the newly opened thread becomes active and projects its saved
  selection.
- Closing a thread cancels every pending load owned by that thread. Closing the
  active thread deterministically activates and projects the first remaining
  logical thread; if none remains, it clears the active projection and owners.
- A visible binding is the tuple `(thread, repository, worktree, surface,
generation)`. A command is legal only when its action binding equals that
  visible tuple.
- Binding-required surfaces are unavailable unless both a repository and
  worktree exist. An unavailable request falls back in the fixed order `Files`,
  `Git`, `Terminal`, `Plan` for the modeled subset of
  `WorkSurface::FALLBACK_ORDER`.
- Repository/worktree and capability changes advance the owning thread's
  generation before recomputing its projection.
- A load captures its thread, repository, worktree, surface, and generation.
  Completing a current load advances only that thread's artifact and event
  revisions. Completing a stale load removes it from the pending set without
  changing a visible projection or either outline revision. Switching to
  another thread does not itself stale the hidden thread's load: currentness is
  derived from the owning thread's generation, binding, available surfaces,
  and selected surface.
- Persisted selection includes thread, requested surface, dock state,
  repository, worktree, generation, and persistence revision. Older or repeated
  persistence and reconnect revisions are legal inputs but are ignored. The
  requested surface is persisted even when a different effective surface is
  currently rendering as its deterministic fallback.
- A repeated or older reconnect snapshot leaves the connection stale. A newer
  snapshot is accepted from either reconnecting or stale state. Disconnect is
  legal from online or stale state.
- Cold start retains known per-thread projections and durable state while
  clearing the active thread, dock, focus, action binding, and outline owners.
  It records the prior active thread as the only restore target. The model's
  explicit `Cold` connection phase is an abstraction boundary: production
  represents this condition with `restore_pending`, not another connection
  enum variant.
- Restore uses the durable selection only when its thread, binding, generation,
  and capability remain valid. Otherwise it computes the same deterministic
  fallback as a live transition.
- Only the visible active thread can own selection, artifact outline, event
  outline, focus, and surface actions.
- Once external input quiesces, weak fairness on `Settle` requires the derived
  projection and durable selection to converge.

The model keeps `requestedSurface` separate from `effectiveSurface`. That
distinction is necessary to preserve user intent while making an invalid
request render a deterministic, currently available fallback.

## Actions

The full scenario covers thread open/close/switch, surface selection, dock
collapse/expand, repository and worktree replacement/removal, capability
invalidation, load begin/complete/fail, disconnect/reconnect/snapshot, persist,
cold start/restore, command routing, quiesce, and settle. Scenario-specific
configurations restrict the action alphabet only to make a longer witness cheap
and deterministic; they use the same actions and invariants as the full model.

`Scenario = "Full"` with `MaxSteps = 3` reaches every action class. Three
transitions are also sufficient for the longest required reachability witness:
begin load, change its worktree generation, then complete the stale load. The
hidden-current witness also uses three transitions: begin on one thread, switch
threads, and complete the original thread's still-current load. The
older-snapshot mutation uses the restricted `Persistence` scenario with six
transitions: receive revision 1 and later receive revision 0.

## Checked properties

| Property                      | Kind     | Meaning                                                                     | Discriminating mutation                                            |
| ----------------------------- | -------- | --------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `Inv_BindingSafety`           | Safety   | Rendered and actionable tuples match the active projection                  | Apply a stale completion to the visible binding                    |
| `Inv_SelectionValidity`       | Safety   | The effective surface is available and is the fixed fallback                | Keep Git after worktree removal; choose the reverse fallback order |
| `Inv_SingleOwner`             | Safety   | At most one visible surface owns focus/actions; hidden surfaces own neither | Keep focus/action ownership after dock collapse                    |
| `Inv_StaleCompletionImmunity` | Safety   | No stale async completion is accepted                                       | Record a stale completion as accepted                              |
| `Inv_ThreadIsolation`         | Safety   | Selection, outlines, rendering, and actions belong to the active thread     | Restore selection ownership from the previous thread               |
| `Inv_PersistenceMonotonicity` | Safety   | Applied and maximum-seen durable revisions never move backward              | Apply revision 0 after revision 1                                  |
| `Inv_RestoreFidelity`         | Safety   | Restore uses its recorded target and deterministic validity rules           | Restore data from the previous thread                              |
| `EventualConvergence`         | Liveness | After quiescence, derived and durable projection state settles              | Disable `Settle`                                                   |

Each file under `mutations/` enables exactly one faulty behavior and checks the
single property it is expected to violate. The two files using
`MutRestorePreviousThread` check thread isolation and restore fidelity
separately. The two stale-completion files separately establish that the same
bad transition is observable at the binding boundary and by the explicit stale
acceptance invariant.

## Reachability and non-vacuity

The five `Probe_*_Unreached` operators are intentionally false invariants: a
successful probe run is a TLC invariant violation with a concrete witness.
They establish reachability of:

- cold start followed by restore;
- disconnect, reconnect, and snapshot application;
- invalid requested-surface fallback;
- stale async completion;
- successful completion for a load whose owning thread is hidden.

`reachability/StaleCompletionDisabled.cfg` disables only stale completion.
Unlike the ordinary stale probe, it passes. This deliberate contrast proves
that the probe distinguishes a live transition from a safety property that
could otherwise pass because the dangerous action was unreachable.

## Bounds and observed result

The required base configuration is exhaustive within these finite bounds:

- 2 threads, 2 repositories, and 2 worktrees;
- 4 representative surfaces;
- generations and revisions in `0..1`;
- at most 1 pending load, enforced by the `BeginLoad` guard;
- at most 3 non-stuttering transitions in the full scenario.

With TLC 2.19, four workers, and the checked-in base configuration, the full
graph completed in about one second:

```text
10,044 states generated, 4,657 distinct states found, 0 states left on queue.
The depth of the complete state graph search is 4.
Model checking completed. No error has been found.
```

This is bounded model checking, not an unbounded proof. Increasing
`MaxGeneration`, `MaxRevision`, or `MaxSteps` expands the checked graph.
The six-step persistence mutation is action-scoped so that the required lane
does not pay for unrelated six-transition interleavings.

## Running the suite

From the repository root, run:

```sh
script/omega-workbench-model
```

The command downloads the pinned TLA+ Tools release, verifies its SHA-256
digest, and invokes the model runner. Use `TLA2TOOLS_JAR=/path/to/tla2tools.jar`
to supply the same pinned jar yourself. The lower-level runner also accepts
`TLC=/path/to/tlc`. Use `TLC_WORKERS=1` to override the default four workers.
The runner puts all TLC metadata and output in a uniquely-created temporary
directory, verifies the exact expected failure for every probe and mutation,
and deletes the directory when it exits. Generated `states/` directories and
common TLC output files are also ignored beside the model.

## Runtime integration map

The intended executable counterpart is
`crates/omega_workbench_state/src/omega_workbench_state.rs`:

- `WorkbenchProjection` corresponds to the per-thread, active, persisted,
  connection, restore, and pending-load variables.
- `ProjectionTransition` corresponds to the model actions.
- `ProjectionTransition::OpenThread` and `OpenThread` both preserve an existing
  active thread and activate the new thread only when no thread is active.
- `ProjectionTransition::CloseThread` and `CloseThread` both cancel owned loads
  and select the first remaining logical thread when the active thread closes.
- `CompleteSurfaceLoad` and `IsCurrentLoad` both derive currentness from the
  load's owning thread, so active-thread switching is not an invalidation.
- `PersistSelection` stores requested surface and dock state in both
  implementations, independently of the currently effective fallback.
- `ReceiveProjectionSnapshot` maps to the `Reconnecting`/`Stale` acceptance
  paths; the model's `Cold` phase maps to production `restore_pending`.
- `visible_projection()` corresponds to `ProjectWith` and its visible binding.
- `WorkSurface::FALLBACK_ORDER` corresponds to `Rank` and
  `FallbackForState`.
- `TransitionEffect::StaleCompletionIgnored`,
  `OlderRevisionIgnored`, and `DeterministicFallback` correspond to the
  explicit ignored/fallback paths.
- `validate()` is the runtime safety checker for states the model represents.

The independent relation is implemented in
`crates/omega_workbench_conformance/src/omega_workbench_conformance.rs`. The
production-to-wire adapter and coverage recorder are implemented in
`crates/omega_workbench_harness/src/omega_workbench_harness.rs`. The adapter
translates reducer attempts and observed effects into payload-minimized trace
steps; it does not use the independent replay function to generate expected
states.

The `agent_ui` integration should own an `Entity<WorkbenchProjection>`,
dispatch all thread, binding, surface, reconnect, and restore inputs through
`apply`, and derive visible surface props, action routing, artifact outline,
and event outline from one `visible_projection()` snapshot.

GPUI tests should then use debug selectors only to assert that the semantic
projection reached the expected render boundary. Rendering code must not
recompute fallback, restore validity, or command ownership independently.
Keeping those decisions in the reducer is what lets TLC traces, generated Rust
tests, and deterministic GPUI render assertions describe the same contract.
