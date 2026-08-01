# Omega Forensics run matrices

Omega compares forensic prompt, model, and scan-profile candidates through
matched run matrices. A matrix arm binds the prompt and model digests, effort,
scope, dependency policy, random seed, tool surface, static/build mode, worker
image/profile, and immutable source bundle. Every arm also has distinct writable
disk, provider session, auth home, environment, and hidden worker-state refs.
The projector rejects shared state before comparison.

## Truthful populations and denominators

The projection follows the OpenAgents forensic scorecard eligibility rules.
Only complete vulnerable or structural-variant development/holdout runs enter
the qualified-identification denominator. Incomplete, fixed-control, and clean-
control runs stay visible in their own populations and gates; they are never
pooled as peers.

Eligible misses retain their total spent tokens/cost and a nonzero declared
right-censor boundary. They appear as `None` in identification observations,
remain in hit-rate denominators, and never become zero-duration successes.
Failed and cancelled runs remain rows with failure, event, and receipt refs.

Each matrix row displays:

- hit/miss/sample/censor counts and hit-rate confidence bounds;
- every identification-time and token observation;
- p50 and tail identification time;
- total tokens and cost;
- causal-link coverage and false positives;
- active reviewer seconds and cleanup count; and
- exact contributing run, event, and receipt refs.

Token and cost aggregates retain `exact`, `estimated`, `upper_bound`, or
`unavailable` truth. If any contributing value is unavailable, the aggregate is
unavailable rather than numeric zero.

When a group has fewer than its pre-registered sample count, estimates are
`provisional`. Tail latency is `not_estimable` until the sample requirement is
met without censoring. This keeps a three-run smoke test useful without
presenting it as a defensible p95.

## Promotion and Pareto safety

Promotion is lexicographic, not a weighted score. It requires all input,
isolation, clean-control, evidence-quality, budget, cleanup, and hit-rate gates;
observed zero false positives; complete causal evidence for hits; budget
compliance; cleanup for every run; and a `dominates` or `non_dominated` Pareto
result. A faster candidate that misses more, weakens evidence, produces a clean-
control false positive, exceeds budget, or fails cleanup remains blocked even if
its supplied gate summary claims success.

The workbench renders a compact row per arm and keeps aggregate-to-source
drill-down counts visible. Raw events, receipts, findings, and private evidence
remain in the host-owned OpenAgents Cloud boundary.

## Verification

```sh
cargo test -p omega_forensics
cargo test -p agent_ui forensics_workbench --lib
./script/clippy -p omega_forensics -p agent_ui
```
