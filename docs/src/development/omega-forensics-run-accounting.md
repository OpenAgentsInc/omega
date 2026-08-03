# Omega Forensics run accounting

Omega stores one canonical run summary with every entropy run. The summary is
rebuildable from the immutable source manifest, file-session states, typed tool
facts, retained outputs, limitations, usage facts, and cleanup receipt. A
restored run is invalid when its stored summary differs from those facts.

## Terminal outcomes

The accounting outcome is one of:

- `active` while required work is queued or running;
- `recovery_required` after outputs settle but before cleanup is observed;
- `completed` only when every eligible focal unit settles, every requested tool
  is available, no failure fact exists, and cleanup has an exact receipt;
- `completed_incomplete` when useful work settles with an incomplete source or
  tool denominator;
- `failed` when all attempted sessions fail without retained output;
- `failed_with_partial_output` when a later failure cannot erase an earlier
  finding or hypothesis; or
- `cancelled` when cancellation settles required work.

The UI cannot derive success from process exit, a completed model turn, or an
empty finding list. Before cleanup, the run remains recoverable and cannot be a
qualified miss.

## Canonical denominators

The summary records these facts together:

- eligible focal units and source paths used, reached, excluded, skipped,
  oversized, or never reached;
- dependency-tree totals and reached totals;
- sessions queued, attempted, settled, timed out, cancelled, refused, or
  failed;
- tools requested, available, unavailable, denied, timed out, or failed;
- findings, hypotheses, duplicates, limitations, and malformed submissions;
- exact, estimated, or unavailable time, token, cost, and network usage; and
- cleanup state, receipt, output refs, failure refs, and a canonical digest.

Unavailable usage has no numeric value. It is not numeric zero.

## Settlement and replay

File output and failure settlement is idempotent. An exact replay does not add
an event or change a denominator. A conflicting replay is rejected. Output is
persisted before cleanup, so a restart in that interval restores
`recovery_required` with the partial output intact. The cleanup transition then
persists the terminal outcome, canonical counts, output refs, failure refs, and
cleanup receipt in the same run projection.

The workbench, campaign rows, and shared Work projection read the stored
summary. They do not calculate independent terminal counts. Metric scorecards
can consume the same contract when OpenAgents issue 9292 lands its metric
authority.

## Qualified misses

A qualified miss additionally requires the matching complete source inspection
from [Omega Forensics source inspection](./omega-forensics-source-inspection.md).
It is blocked by an incomplete source, unsettled focal unit, missing or failed
tool, rejected output, unavailable cleanup receipt, finding, or hypothesis.
