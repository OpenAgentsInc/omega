# Workbench projection consistency

Omega has a single logical reducer for deciding which desktop work surface is
visible, which thread and worktree it belongs to, which surface owns actions,
and what survives reconnect or restart. A bounded TLA+ model, an independent
Rust checker, and deterministic harness traces keep that reducer honest before
the result reaches GPUI.

Run the complete logical proof lane with:

```sh
script/omega-workbench-model
```

The command downloads TLA+ Tools 1.7.4 from the official release, verifies its
SHA-256 digest, then runs the exhaustive bounded model, reachability probes,
and intentional red mutations. It does not read an Omega data directory.

Run the Rust side with:

```sh
cargo test \
  -p omega_workbench_state \
  -p omega_workbench_conformance \
  -p omega_workbench_harness
```

## Four different claims

The workbench proof system deliberately separates four claims:

| Layer                     | Establishes                                                                                                                       | Does not establish                                         |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| Bounded TLA+ model        | Every reachable modeled state within the checked finite bounds satisfies the safety invariants and bounded liveness property      | Unbounded correctness, Rust conformance, layout, or pixels |
| Runtime trace conformance | A tested production reducer execution has the same effects and logical states as an independently implemented transition relation | Executions that were not traced, GPUI layout, or pixels    |
| GPUI semantic assertions  | The expected controls, ownership, focus, accessibility, and layout relationships reached the render tree                          | Exact raster output                                        |
| Metal pixel comparison    | The final Apple Silicon render matches its reviewed baseline within the declared tolerance                                        | Hidden logical ownership on its own                        |

A green layer cannot substitute for another one. In particular, the checker is
not a formal proof, and a screenshot cannot prove that a command was routed to
the current worktree.

## Runtime boundary

`omega_workbench_state::WorkbenchProjection::apply` is the production semantic
boundary. It applies a transition transactionally: rejected transitions leave
the state unchanged. The state contains only the logical values needed to
derive a work surface:

- active and known logical thread IDs;
- repository/worktree binding and generation per thread;
- available, requested, and effective surfaces;
- dock visibility and the single focus/action owner;
- artifact and event outline revisions;
- pending loads with their captured binding and generation;
- connection and projection revision;
- persisted selection, revision, and cold-restore status.

`visible_projection()` is the only projection a renderer or command router
should consume. A hidden thread retains its own requested surface, but cannot
own global focus or receive a visible-surface command. Going offline clears
global focus ownership without discarding the thread's selection. Plan is the
one offline interaction exception: because it is thread-local and does not
address repository state, it may be requested and its retained dock may be
collapsed or expanded offline. Repository-bound requests and every surface
command remain online-only.

Generation is the binding content epoch, not merely a counter for path
changes. A deliberate `ChangeBinding` to the current repository/worktree is a
refresh action: branch checkout uses it to invalidate loads that captured the
same paths under the previous Git state.

The fixed surface fallback order is Files, Search, Review, Git, Terminal, then
Plan. A missing request remains closed; fallback is used only when a requested
surface became unavailable.

### Transition mapping

The checked model lives in `docs/spec/workbench_projection`. The independent
wire relation lives in `omega_workbench_conformance`. This table is the
reviewable source-to-model map:

| Model action                                                        | Production transition                                                | Trace action                                                             |
| ------------------------------------------------------------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `OpenThread`, `CloseThread`, `SwitchThread`                         | `OpenThread`, `CloseThread`, `SwitchThread`                          | `open_thread`, `close_thread`, `switch_thread`                           |
| `RequestSurface`, close, collapse, expand                           | `RequestSurface`, `CloseSurface`, `CollapseDock`, `ExpandDock`       | the corresponding surface/dock action                                    |
| `BindRepository`, `ChangeWorktree`, `ChangeBinding`, remove binding | `BindRepository`, `ChangeWorktree`, `ChangeBinding`, `RemoveBinding` | `bind_repository`, `change_worktree`, `change_binding`, `remove_binding` |
| `BeginLoad`, `CompleteLoad`, `FailLoad`                             | `BeginSurfaceLoad`, `CompleteSurfaceLoad`, `FailSurfaceLoad`         | the corresponding surface-load action                                    |
| `Disconnect`, `BeginReconnect`, `ReceiveSnapshot`                   | `Disconnect`, `Reconnect`, `ReceiveProjectionSnapshot`               | the corresponding connection action                                      |
| `PersistSelection`, `ColdStart`, `Restore`                          | `PersistSelection`, `ColdStart`, `RestoreSelection`                  | the corresponding persistence action                                     |
| `InvalidateCapability`                                              | `InvalidateCapability`                                               | `invalidate_capability`                                                  |
| `RouteCommand`                                                      | `DispatchSurfaceCommand`                                             | `dispatch_surface_command`                                               |

Search and Review are represented by Files in the TLA+ state space because all
three have the same binding rules for the checked properties. The production
reducer and independent checker retain all six distinct surfaces.

Opening a thread activates it only when no active thread exists. Closing an
active thread deterministically selects the first remaining logical thread, if
one exists, and cancels every pending load for the closed thread.

## Trace contract

The wire schema is
`openagents.omega.workbench-conformance.v1`. Every critical transition records:

- a contiguous sequence number;
- a closed transition kind and its logical identity/revision fields;
- the observed closed effect; and
- the complete observed logical projection after the attempt.

Effects are:

- `applied`;
- `stale_completion_ignored`;
- `older_revision_ignored`;
- `deterministic_fallback`; or
- `rejected` with a closed reason code.

Recording rejected attempts matters. Without the effect, a checker could not
distinguish an intentionally ignored stale completion from an implementation
that accidentally applied it. A rejected attempt must also record the same
state as the preceding step.

The schema permits only bounded ASCII logical identifiers containing letters,
numbers, `.`, `_`, or `-`. It has no fields for paths, thread titles, messages,
tool output, terminal contents, source, credentials, error strings, or GPUI
entity IDs. Unknown fields and unknown critical actions fail closed.

`omega_workbench_harness::WorkbenchTransitionRecorder` records one step for
every production reducer attempt and rejects a coverage-count mismatch. The
independent checker never imports or calls the production reducer. Its tests
also reject an empty trace or a trace that omits any declared required action.

## Deterministic implementation traces

The harness generates six named reducer/model conformance scenarios:

| Scenario                   | Required observation                                                                                                     |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `thread_switch`            | Switch A/Git to B/Terminal; only B owns binding, selection, focus, artifact outline, and event outline                   |
| `worktree_change`          | Change A from worktree A/generation 0 to worktree B/generation 1                                                         |
| `stale_completion`         | Ignore the old worktree-A completion, then accept a worktree-B completion and advance both outline revisions once        |
| `reconnect`                | Ignore an equal/older snapshot while staying non-online, then accept revision 1 and converge online                      |
| `valid_restore`            | Persist A/Review, cold start, and restore the exact binding, surface, dock, and focus                                    |
| `invalid_restore_fallback` | Persist A/Git on worktree A, rehydrate worktree B without Git, choose Files, and rewrite the persisted logical selection |

A separate adapter test drives every one of the 22 critical action kinds
through the production reducer and the independent checker. Another test proves
that a rejected production action is recorded and leaves state unchanged.

These are reducer-level traces emitted by the GPUI workbench harness crate.
They are not yet claims about rendered Files, Search, Review, Git, Terminal, or
Plan controls. Each feature must attach the same recorder to its production
entity and add semantic GPUI assertions as the corresponding surface lands.

## Checked model

The base TLC configuration exhausts:

- 2 threads, 2 repositories, and 2 worktrees;
- 4 representative surfaces;
- generation and revision values `0..1`;
- at most 1 pending load; and
- at most 3 non-stuttering transitions.

The current base graph contains 12,400 generated states and 5,153 distinct
states at depth 4. TLC reports no queued states and no property violation. A
six-step action-scoped configuration checks the longer older-snapshot
sequence. These are exact finite bounds, not an unbounded proof.

The model checks:

- binding safety;
- selection validity and deterministic fallback;
- one visible focus/action owner;
- stale-completion immunity;
- thread and outline isolation;
- persistence/projection revision monotonicity;
- valid and invalid restore behavior; and
- weakly fair convergence after inputs quiesce.

Every property has an intentional mutation that must fail with its designated
invariant or temporal-property message. Separate reachability configurations
produce witnesses for cold restore, reconnect, invalid fallback, stale
completion, and a current hidden-thread completion that updates only its
owner's outlines. Disabling stale completion makes only that reachability probe
pass, which detects a vacuous green invariant.

See `docs/spec/workbench_projection/README.md` for the complete variable,
action, property, mutation, and bound definitions.

## CI

`.github/workflows/omega_workbench_proof.yml` has four responsibilities:

- **Projection model and conformance** installs a pinned Java runtime, verifies
  the pinned TLA+ Tools download, runs every model/probe/mutation, and tests the
  reducer, checker, and harness traces.
- **Portable semantics** exercises GPUI semantic probes and deterministic
  scheduler behavior.
- **Metal pixels** runs the sharded Apple Silicon comparisons.
- **Required** fails unless every preceding lane succeeded, including cancelled
  or skipped jobs.

Keep model checking in the logical lane. Do not add Java or TLC to a Metal
pixel job.

## Extending the workbench

When adding a transition or a new identity that can affect ownership:

1. Add it to the production reducer.
2. Add the payload-minimized trace action and independent transition.
3. Update the TLA+ action or document why the existing abstraction covers it.
4. Add it to the adapter's all-actions coverage test.
5. Add a hand-built or generated trace that distinguishes the safe and unsafe
   behavior.
6. Add a red mutation when it introduces a new invariant.
7. Attach semantic GPUI assertions to the rendered control.

Do not derive fallback, restore validity, command ownership, or stale-request
acceptance again in a view. Renderers should consume the reducer's visible
projection so the model, trace, and UI all describe the same decision.

The production activity rail and retained host boundary are described in
[Omega desktop workbench shell](./omega-desktop-workbench-shell.md). That shell
tracks actual GPUI focus separately from the projection's logical action owner
and must prove both at the semantic layer.

## Limits

The model abstracts native entity lifetimes, filesystem and Git behavior,
server payload delivery, layout, and content. The checker covers only recorded
executions. The six current traces exercise the logical reducer before the full
native surface UI exists. The retained generic shell proves the common rail,
dock, focus-transfer, responsive-layout, and host-lifecycle boundary. The GPUI
and Metal layers remain responsible for proving that each later native entity
actually dispatches through this seam and renders the projected state.
