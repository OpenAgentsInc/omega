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
./script/clippy -p omega_forensics -p agent_ui
```
