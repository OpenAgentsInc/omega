# Windows Build Findings (QA)

**Status:** QA findings only — no code changes in this PR. Prepared from a real Windows build attempt of the Omega client so the maintainers can implement the fixes in their own style.

**Environment:** Windows 11 (build 26200), rustc/cargo 1.95.0, target `x86_64-pc-windows-msvc`, MSVC 14.44, Windows SDK 10.0.26100.

**Branches/commits referenced:** `main` at `3beb79c88e` (current upstream main at time of writing).

---

## Finding 1 — Orphaned macOS cfg gate breaks Windows and Linux builds

**File:** `crates/omega/src/zed.rs` (lines 6–7)

**Problem:** The line `#[cfg(target_os = "macos")]` sits directly above `mod open_listener;`, gating cross-platform machinery — open-url handling, CLI handoff, and single-instance forwarding — to macOS. On Windows (and Linux/freebsd) this produces unresolved-import errors (E0432):

- `crates/omega/src/zed.rs:57` — `pub use open_listener::*;`
- `crates/omega/src/main.rs:70` — `use zed::{OpenListener, OpenRequest, ...}` (six items)
- `crates/omega/src/main.rs:75` — `use crate::zed::OpenRequestKind`
- `crates/omega/src/zed/open_url_modal.rs:10` — `use super::{OpenListener, RawOpenRequest}`
- `crates/omega/src/zed/windows_only_instance.rs:26` — `use crate::zed::{OpenListener, RawOpenRequest}`

**Root cause:** The cfg line was orphaned. Upstream zed#54719 (commit `602cf8f6c7e`) added `#[cfg(target_os = "macos")] pub(crate) mod move_to_applications;` directly above `mod open_listener;`. Omega fork commit `6ebe8fd159` ("Delete the hosted-plan, trial, and ambient-nag surfaces", 2026-07-25) deleted the `move_to_applications` module but left the cfg line behind, which re-attached to `mod open_listener;` and gated the whole module to macOS. `open_listener.rs` is cross-platform by design (it contains `#[cfg(target_os = "windows")]` branches and a Linux-gated `listen_for_cli_connections`).

**Suggested fix (one line):** delete the stray `#[cfg(target_os = "macos")]` line above `mod open_listener;`.

**Why not gate the import sites instead:** the imported items are used on Windows in ~18 non-gated code paths (e.g., `main.rs:407` `OpenListener::new()`, `:421` `handle_single_instance(...)`, `:506-508`, `:571`, `:973-982`, `:992-995`, `:1038-1047`; `zed.rs:1143-1148` inside un-gated `register_actions`; `windows_only_instance.rs:41-53`). Gating the imports would only produce a second wave of errors and would remove intended Windows single-instance/open-url behavior. Deleting the cfg line is the minimal, behavior-preserving fix.

---

## Finding 2 — `windows_only_instance.rs` references removed `Args` fields

**File:** `crates/omega/src/zed/windows_only_instance.rs` (lines ~143–152 and ~164; `let mut diff_paths` at ~124)

**Problem:** The Windows-only single-instance forwarding code references fields that no longer exist on `Args` (`crates/omega/src/main.rs`, `Args` struct ~line 2062) or on `CliRequest::Open` (`crates/cli/src/cli.rs:57-76`):

- Line 143 — `for path in args.diff.chunks(2) {` → E0609: no field `diff` on `&Args`
- Line 164 — `dev_container: args.dev_container,` → E0559 (CliRequest::Open) and E0609 (Args)

**Root cause:** upstream removed the flags in two commits and never re-checked this file:

- `60165d1d97` "Remove the full-editor mode split" (omega#161, 2026-07-30) — deleted `--full-editor`, `--diff`, `--dev-container`, `--demo-workroom` from the omega binary's argument parser.
- `b7e1c3e64c` (omega#162, 2026-07-30) — deleted dev-container end to end, including the `CliRequest::Open.dev_container` IPC field.

The file is gated `#[cfg(target_os = "windows")]` (`crates/omega/src/zed.rs:20-21`), and upstream CI compiles Linux/macOS only, so the drift never surfaced. The file is byte-identical to `main` — **this is a defect on main itself**, not introduced by any branch.

**Suggested fix:**

1. Delete the `args.diff` accumulation loop (lines ~143–152). It is unreachable dead code: the omega binary exits on its startup guard (`main.rs:211-223`) before `Args::parse()` for these removed flags, so the block can never run. Diff mode survives only through the `cli` crate via `zed-cli://` IPC to `handle_cli_connection` (`open_listener.rs:591`). Deleting it loses zero reachable behavior.
2. Delete the `dev_container` line (line ~164) — dev-container support was deleted end to end (omega#161 + omega#162); there is no successor field.
3. Drop `mut` on `let mut diff_paths` (line ~124) to avoid a new `unused_mut` warning.

**Do not touch** `crates/cli/src/main.rs` — its `diff: Vec<String>` handling is live and canonical.

---

## Finding 3 — `omega_deltas`: 34 of 250 checks fail on Windows

Separate issue with the full classified inventory (all 34 failures, verbatim output, repro, and suggested fix directions): **OpenAgentsInc/omega#312**.

Summary: `cargo test -p omega_deltas` at `3d94f38271` → `216 passed; 34 failed` (exit 101). Six observable classes: (1) 2 path-separator mismatches in the harness itself, (2) 13 fail-closed "parsed 0 / not found" vacuous-check panics (harness read/parse failures vs. genuine tree gaps — not determinable from output; a Linux baseline run would split the classes), (3) 13 brand/content gate findings (mix of real tree state — e.g., `assets/icons/ai_zed.svg`, Zed strings in `crates/app_identity`, two SKILL.md files without YAML frontmatter — and scan contamination where the gate flags its own config and the harness file), (4) 2 behavioral/tree-state assertions, (5) 3 harness self-matches, (6) 1 harness execution-portability failure (uninstall check cannot run its script on Windows). No baseline Windows run exists, so pre-existence is unproven; none of the 34 reference the two findings above.

---

## Verification performed

Both findings were reproduced locally on this Windows machine:

- Without fixes: `cargo build --profile release-fast` fails — first with the Finding-1 E0432 errors, then (with Finding 1 patched) with the Finding-2 errors.
- With both findings patched locally (for verification only — those edits are intentionally **not** part of this PR): `cargo check -p omega` passes with zero errors, `cargo build --profile release-fast` completes (`Finished ... in 3m 29s`), and the application launches (window opens, process stable).

**Suggested next step for maintainers:** implement Finding 1 (one-line cfg deletion) and Finding 2 (three small deletions) on a fix branch, re-run `cargo check -p omega` / a full Windows build, and triage issue #312 with a Linux baseline run of `omega_deltas` to separate harness portability from tree-state findings.
