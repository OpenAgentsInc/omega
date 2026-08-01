# Omega Forensics preflight

The Forensics work surface lets you inspect a repository target and prepare a
bounded forensic run without leaving the active Omega conversation. It is a
preflight surface. Preparing a run creates a typed launch intent; it does not
provision or start a worker.

This page documents the contract introduced by
[issue 192](https://github.com/OpenAgentsInc/omega/issues/192).

## Open the surface {#open-the-surface}

Open a thread with a Git repository, then select **Forensics** from the
workbench activity rail. The surface retains the current repository binding,
remote URL, exact commit, and clean or dirty source state. Selecting another
worktree or thread gives the new binding its own retained host.

Forensics is unavailable for a projectless thread or a worktree without a Git
repository. Other workbench surfaces keep their existing availability rules;
Plan remains available without a repository binding.

## Select the Coldcard benchmark {#select-the-coldcard-benchmark}

The benchmark selector exposes four explicit arms:

- **Vulnerable** uses the known pre-fix Coldcard commit and complete scan
  profile.
- **Incomplete** uses the vulnerable commit with a deliberately incomplete
  scan profile.
- **Fixed** uses the known fixed commit.
- **Clean** uses the fixed commit as a clean control.

Changing an arm clears the previous coverage result and returns the preflight
to `Coverage pending`. You do not edit serialized state or repository files to
change an arm.

## Verify managed execution {#verify-managed-execution}

The surface accepts only a validated OpenAgents-managed placement. The public
projection identifies Google Cloud, GCE VM isolation, the admitted GCE adapter,
region and custody refs, pinned image and profile digests, a broker-only
network policy, a lease ref, and public capability refs.

The renderer does not receive provider clients, cloud project or instance IDs,
topology, control tokens, credentials, or a shell. Local, fake, bring-your-own,
Box-owned, Pylon, generic remote Linux, foreign-cloud, and fallback target
classes are not variants in the typed contract.

Until OpenAgents supplies an admitted projection for the exact repository
binding, the surface displays `Awaiting OpenAgents managed profile` and keeps
**Prepare run** disabled.

## Verify bounds and coverage {#verify-bounds-and-coverage}

An admitted profile displays model, effort, concurrency, time, token, cost,
artifact, and network caps. Every cap required to perform work must be
positive. The projection rejects an omitted or zero execution budget before it
can create a launch intent.

Coverage reports these source classes:

- present;
- missing;
- excluded;
- generated;
- oversized; and
- dependency-owned.

Pending coverage cannot create a launch intent. Complete coverage cannot retain
missing or oversized entries. An incomplete manifest remains visibly
`Incomplete research`; you must acknowledge that state before preparing the
run, and the resulting intent retains `incomplete: true`. Denied coverage
remains denied.

## Prepare a run {#prepare-a-run}

Review the target, managed placement, bounds, and terminal coverage, then press
**Prepare run**. This explicit action creates a launch intent bound to the
preflight ref, coverage state, incomplete state, and admitted budgets. It does
not start analysis. Worker admission and launch are separate follow-up work.

The boundary prevents surface selection, benchmark selection, coverage updates,
or profile delivery from starting a worker as a side effect.

## Verify the implementation {#verify-the-implementation}

Run the focused contracts with:

```sh
cargo test -p omega_forensics
cargo test -p agent_ui forensics_workbench --features test-support
cargo test -p omega_workbench_state -p omega_workbench_conformance
cargo test -p omega_deltas drawn_activity_rail_controls_are_admitted_and_loaded
```

The domain tests cover placement refusal, renderer-safe serialization, budget
admission, coverage terminality, persistent incomplete state, and explicit
operator action. The GPUI tests cover repository binding and all four benchmark
selectors. The independent workbench checker covers the seventh surface's
selection, persistence, focus, and stale-completion behavior.
