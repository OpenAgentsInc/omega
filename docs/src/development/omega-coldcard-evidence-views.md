# Omega Coldcard case reader

Omega issue [#197](https://github.com/OpenAgentsInc/omega/issues/197) added the
typed Coldcard evidence projection. Omega issue
[#203](https://github.com/OpenAgentsInc/omega/issues/203) makes the bundled,
synthetic projection available in the production Forensics workbench as a
read-only case reader. The reader consumes a validated projection; it does not
recompute evidence from prose or grant a renderer access to worker
credentials, private keys, or source artifacts.

## Comet reader

The reader is one bounded list/detail workspace. Its left list contains a case
overview and all nine ordered proof rungs. Its right detail pane shows the
selected overview or rung. This replaces the former vertical pile of evidence
cards while preserving the surrounding Comet sidebar, tabs, composer, and
thread history.

Users can open Forensics from the project sidebar or from **View → Workbench →
Forensics**. Both paths select the same native work surface.

The workbench navigation separates the entropy dashboard, Coldcard case, and
preflight lifecycle. The selected view and lifecycle row are presentation
state. Omega restores them for the same repository binding without copying a
preflight, run, review, or evidence projection into UI-owned state.

The overview identifies the Coldcard target, public clone URL, pinned
vulnerable commit, synthetic fixture state, private/non-reportable boundary,
highest supported rung, evidence completeness, and provenance health. It also
labels the live-run control as unavailable. The fixture is a reader input, not
a live receipt.

Every list row is reachable by Tab and activatable with Enter or Space. The
list, rows, selected state, status/error scenes, detail group, and read-only
boundary have explicit accessibility roles and labels. Reader selection is
presentation state only. Selecting a row does not prepare or launch a run,
promote a claim, publish a result, or create another authority store.

## Preflight and lifecycle

The lifecycle view is a bounded list/detail workspace. Its rows separate the
summary, target, coverage, tool profile, runtime, and cleanup. The detail pane
always names the projected state, exact blocker, and next admitted action.

The presentation distinguishes awaiting-profile, awaiting-coverage, complete,
incomplete, denied, incompatible-tool, running, cancelled, recovery-required,
cleaned, and stale scenes. A commit mismatch is stale only when the selected
project and preflight name the same repository. A profile is incompatible when
it does not prove the required read-only forensic source capability. Cancelled
work remains cleanup-blocked until the run projection proves deletion and zero
residue.

Live prepare and launch controls are visible but disabled. The synthetic
fixture and existing local projections do not satisfy the live worker and
source-delivery acceptance gates tracked by OpenAgents issues #9289 and #9290.
The UI cannot acknowledge those gates or convert a ready preflight into launch
authority.

## Evidence queue and claim inspector

The **Evidence** bench view is a bounded list/detail workspace. Its queue keeps
five claim classes separate: findings, hypotheses, limitations, disputes, and
reconciliation. Selecting a row changes presentation state only. The validated
Coldcard workspace and forensic review projections remain the authorities.

The inspector exposes the evidence ladder, generator trace, historical scan,
graph health, append-only corrections, exact evidence and rule references,
missing rungs, non-implications, and the next mechanical check. An unavailable
value stays unavailable. A missing rung is not inferred from evidence on either
side of it. The bundled scan is marked private and non-reportable.

Source locations are interactive only when a typed `ForensicSourceCitation`
exists in the review projection. The button emits the existing `OpenSource`
command. The evidence UI does not resolve or open arbitrary strings as paths.

Deterministic evidence scenes reuse the validated case projection states:
loading, empty, invalid, stale, and complete. Keyboard users can focus every
bench and queue row and activate it with Enter or Space. Selected rows publish
their selected accessibility state and the list/detail regions have explicit
labels.

## Model panel and run matrix

The **Models** bench view shows a bounded run list and one scorecard. Each run
keeps its model family, forensic role, eligibility, typed outcome, right-censor
boundary, prompt and model digests, token exactness, cost exactness, qualified
findings, and arm-specific disagreement. An unavailable token or cost value is
shown as unavailable, never as zero.

The matched-comparison block exposes the frozen dataset, metric-definition, and
evaluator digests plus common findings. Agreement is not truth, and a majority
vote cannot promote a forensic claim. The projection's hard gates and promotion
state remain authoritative.

Before an observed matrix exists, Omega displays a validated synthetic
Coldcard matrix with an explicit **Synthetic fixture** badge. It contains an
eligible hit, a right-censored miss, and an ineligible clean control so that the
important scenes stay inspectable without implying a live run. Model-run
selection is restored presentation state only.

## Evidence ladder

The ladder always contains nine ordered rungs: source flaw, artifact reality,
generator behavior, exploitability, owned-fixture recovery, program
fingerprint, entity grouping, unauthorized movement, and identity attribution.
A missing rung stays visible with unavailable time and token truth. Downstream
evidence cannot backfill it. Every non-missing rung shows its exactness,
evidence refs, assumptions, verifier state, and at least one non-implication.

The linked trace keeps source, compiled artifact, and generator steps separate.
Each step has a dense sequence, evidence ref, derivation rule, and verifier
state. The entropy explorer always shows UID, timer, call-trace, firmware, and
hardware assumptions, with baseline-to-selected diffs and bounded resulting
entropy intervals.

## Private scan console

The historical-chain view is admitted only under
`boundary.omega.private-forensic-run.v1` with `reportable = false`. It shows
block ranges, completed heights, checkpoint refs, restart state, throughput
exactness, positive and negative controls, rate strata, the candidate funnel,
and missing-data failures. Funnel counts must be monotonically non-increasing.
Public transaction IDs and candidate cluster refs may be retained for private
review, but the UI displays only their counts and the non-reportable boundary.

## Graph, reconciliation, and corrections

Graph health is green only when a claim or projection has at least one source,
at least one derivation rule, and no missing-provenance refs. Reconciliation
uses `MATCH`, `DRIFT`, and `UNAVAILABLE` while preserving the independently
derived and published originals. Unavailable comparisons cannot carry invented
numeric values.

Corrections are dense, append-only records. They retain prior and corrected
display values, the reason, appended evidence, and every affected projection.
No correction mutates an earlier claim event.

## Render and export safety

The workbench validates the complete projection before storing or rendering
it. Secret-shaped text—including extended private keys and explicitly labelled
private-key, node-cookie, RPC-credential, password, or seed-phrase payloads—is
rejected. The fixture at
`crates/omega_forensics/fixtures/coldcard-evidence-workspace.v1.json` contains
only synthetic refs and values and exercises missing rungs, private IDs,
controls, assumptions, provenance gaps, reconciliation, and correction history.

Run the contract and GPUI coverage with:

```sh
cargo test -p omega_forensics coldcard_workspace
cargo test -p agent_ui forensics_workbench::tests::coldcard --lib
cargo test -p agent_ui forensics_workbench::tests::lifecycle --lib
cargo test -p agent_ui forensics_workbench::tests::evidence --lib
cargo test -p agent_ui forensics_workbench::tests::model --lib
./script/clippy -p omega_forensics -p agent_ui
```
