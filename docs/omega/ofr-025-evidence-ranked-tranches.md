# OFR-025: Evidence-ranked forensic tranches

Status: code-complete contract and fixture/runtime integration. Installed aggregate proof remains deferred to the final Forensics build.

## Outcome

Omega schedules the entropy-and-secret-randomness investigation by deterministic boundary evidence instead of alphabetic path order. The manifest keeps its dense path order as immutable source identity. A separate `openagents.omega.evidence-ranked-schedule.v1` artifact owns analysis order.

The rank is triage evidence. It is not a vulnerability finding, severity, confidence score, or publication claim. Every eligible focal unit receives a dense rank and a tranche, including the zero-marker fallback tail, so ranking cannot silently remove eventual coverage.

## Boundary map

The versioned boundary map covers:

- entropy and randomness;
- provider selection and compile-time guards;
- key derivation;
- secret sinks;
- authentication and authorization;
- parsers;
- unsafe and FFI boundaries;
- external inputs;
- dependencies and build configuration;
- domain invariants.

Before a model session starts, bounded scanners inspect the immutable source bytes. Each scanner receipt binds the tool version, configuration digest, source digest, matched boundary classes, matched feature references, and a canonical digest. Scanner matches affect scheduling only. They do not enter the finding projection.

## Scheduling and context

Each model session receives one focal path, its evidence rank, tranche, boundary classes, and human-readable rationale. The prompt states that the session can read the complete admitted source graph as context and must cite contextual source exactly. Focal use and contextual reads remain separate accounting dimensions.

The schedule serializes:

- ranking and scanner versions;
- the threat-model reference and immutable manifest digest;
- scanner receipts and feature references;
- dense eligible rank, tranche boundary, rationale, and evidence score;
- independent queued, focal, contextual, completed, excluded, skipped, oversized, unreachable, and never-reached dispositions;
- exact exclusion reasons.

Score ties use immutable content digest and then manifest sequence. Path spelling is never the hidden ranking policy.

## Budget and lifecycle

`TrancheBudgetLedger` separates admitted budget from consumed focal sessions. Pause, resume, budget extension, cancellation, and restart append typed control events. These controls do not rebuild or reorder the schedule. A restored schedule resumes from the lowest still-queued dense rank, which prevents duplicate work.

Budget exhaustion, operator cancellation, explicit exclusion, unsupported language, oversized source, source failure, and unreachable work remain distinct terminal facts. A run cannot describe pending eligible units as covered.

## Evaluation and promotion

Ranked scheduling uses the same matched-run matrix introduced by OpenAgents #9292 and the existing Omega matrix projection. The comparison keeps dataset, metric, evaluator, source bundle, provider session, environment, and worker state isolated per arm. Recall-over-time and recall-over-token curves can measure earlier discovery, but speed alone cannot promote the ranked arm. Cleanup, budget compliance, false-positive, causal-link, sample-size, and Pareto hard gates continue to fail closed.

## Verification

Focused contract verification:

```sh
cargo test -p omega_forensics
./script/clippy -p omega_forensics
```

The tests prove that strong deterministic boundary evidence outranks an alphabetically earlier ordinary file, every eligible file remains ranked, and serialization/resume does not reorder or repeat settled work. The final installed-app batch must additionally capture a visible Workbench receipt for tranche order, source accounting, lifecycle controls, and aggregate cleanup.
