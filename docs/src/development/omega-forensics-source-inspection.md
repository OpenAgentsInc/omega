# Omega Forensics source inspection

Omega opens a visible catalog Forensics task as soon as the top-level pinned
checkout is usable. Recursive dependency inspection starts after the thread is
visible. It does not make task creation wait for submodule network delivery.

The mechanical inspector emits
`openagents.omega.entropy-source-inspection.v1`. Each generation binds the exact
top-level commit and Git tree, manifest ref and digest, every declared recursive
dependency path, expected and observed dependency revisions, availability, and
the exact Git error when delivery fails. A failed `git submodule status
--recursive` is expanded across the declared `.gitmodules` paths; it cannot
collapse them into one silent omission.

The projection keeps these facts separate:

- focal, contextual, reached, and not-reached paths;
- dependency paths and their status rows;
- declared required and missing generated inputs;
- policy-excluded and required-excluded paths;
- oversized paths; and
- dirty working-tree paths excluded from an immutable target.

The workbench shows the inspection generation, tree, manifest, dependency rows,
path counts, incomplete reasons, and qualified-miss eligibility. While an
attached task materializes dependencies, the inspector re-evaluates the source
for at most two minutes and installs monotonic generations. A top-level commit
or tree change marks the prior projection stale instead of relabeling it.
The latest bounded projection is persisted with the Forensics workspace. A
restored projection is always advanced to `stale`; restart never reuses prior
completeness as current source truth.

## Completion and qualified misses

Available source remains useful when inspection is incomplete. Missing or
wrong-revision dependencies, unavailable source, oversized required source,
missing generated inputs, required exclusions, dirty excluded bytes, and stale
generations all block a qualified miss. The analysis can continue with an
incomplete result; absence of a candidate is not evidence that missing source
is safe.

An ordinary unsupported non-source file is recorded as a policy exclusion but
does not by itself make a dependency-complete source bundle incomplete. A path
that the target profile declares required does. The distinction prevents a
README from blocking every run while still failing closed for an excluded build
or source input.

Managed and benchmark runs still require the stronger immutable OpenAgents
source-bundle authority owned by OpenAgents issue 9290. A local inspection does
not replace that bundle, admit a managed worker, or grant verification authority.

## Verification

```sh
cargo test -p omega_forensics
./script/clippy -p omega_forensics
cargo test -p agent_ui forensics_workbench --lib
```

The contract fixtures cover complete and missing dependencies, path-class
separation, dirty exclusions, missing generated inputs, required exclusions,
stale generations, and qualified-miss refusal.
