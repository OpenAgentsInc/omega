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
`crates/zed/test_fixtures/visual_tests` and are copied into the output root
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
`crates/zed/test_fixtures/visual_tests`. The default policy requires at least
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

## Sharding and CI {#ci}

Select one deterministic catalog shard with both arguments:

```sh
script/omega-workbench-proof \
  --pixel-only \
  --shard-index 0 \
  --shard-count 4
```

The catalog order defines the shard assignment. The harness rejects an empty or
out-of-range shard.

`.github/workflows/omega_workbench_proof.yml` runs on pull requests, merge
groups, pushes to `main`, and manual dispatches. Its jobs are:

- **Portable semantics** on GitHub-hosted `ubuntu-22.04`. It runs the
  `omega_workbench_harness` tests, the production Agent UI scene adapter, a
  16-iteration GPUI `debug_render_snapshot` seed sweep starting at seed `0`,
  and the deliberate pending-task and retained-entity failure probes.
- **Metal pixels** on GitHub-hosted `macos-15`, the pinned Apple Silicon runner
  image for these baselines. A two-shard matrix verifies that the runner is
  `arm64`, then runs the pixel lane at seed `0`. The output for shard `<n>` is
  `target/omega-workbench-proof/shard-<n>`.
- **Required** on GitHub-hosted `ubuntu-24.04`. It fails unless both the portable
  and Metal jobs succeeded, including when an upstream job was cancelled or
  skipped.

The workflow disables Cargo debug information for its dev, test, and release
profiles. The proof does not inspect debug symbols, and omitting them keeps the
independent Rust build products inside the standard hosted runners' disk
budget. Local developer profiles are unchanged.

The Metal job attempts to upload each failed shard's output folder as
`omega-workbench-proof-shard-<n>` for 14 days. Receipts, current images, and
diff images produced before the failure remain together in that artifact. A
failure before any evidence is written produces an artifact warning instead of
hiding the original test failure.

Use these focused checks while developing the harness:

```sh
cargo test -p omega_workbench_harness
cargo test -p omega_workbench_harness --features gpui-support
```

CI must not use `--update`. Missing scenes, duplicate scene names, zero semantic
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
