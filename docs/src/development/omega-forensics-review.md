# Omega Forensics review

Omega issue [#194](https://github.com/OpenAgentsInc/omega/issues/194) adds a
native review projection to the Forensics work surface. It mirrors the checked-in
OpenAgents forensic contracts instead of interpreting model prose or rebuilding
truth from logs.

## Review contract

`ForensicsReviewProjection` binds a review to one repository, 40-character
commit, run, placement, sandbox, and resource generation. It contains bounded
finding, hypothesis, metric, lifecycle, receipt, and review-decision records.
The projection is rejected if source citations drift from the pinned commit,
paths escape the repository, causal links are unordered or unevidenced,
executed evidence lacks a successful artifact receipt, cleanup state disagrees
with its receipt, or collection bounds are exceeded.

Findings and hypotheses are different types and different visual lanes. A
hypothesis always names its missing evidence and next check. A finding shows
claim state, severity, duplicate group, causal links, source citations,
evidence tier, PoC/test artifact ref, evidence receipts, and verifier verdict.
The five contract tiers remain explicit: hypothesis, source observed, artifact
observed, executed, and independently verified.

## Metrics and terminal truth

Metric values retain `exact`, `estimated`, `upper_bound`, or `unavailable`.
Unavailable values have no numeric value and require a reason ref, so missing
tokens or spend cannot appear as zero. The surface can render time, tokens,
cost, causal-link coverage, reference validity, reviewer burden, or any later
frozen metric without changing this truth model. It also shows the budget state
and the complete request-to-cleanup lifecycle waterfall.

Completed and completed-incomplete are distinct. Cancelled, missed, failed,
censored, and cleanup-failed runs remain valid review outcomes rather than
disappearing from the UI. Zero-residue cleanup is displayed only with its
receipt; cleanup failure remains visible beside placement and generation.

## Source navigation and decisions

Selecting a citation asks the host—not the renderer—to resolve the file. The
host requires the current worktree HEAD to equal the citation commit,
canonicalizes both repository and file paths, refuses symlink escape or missing
files, then opens the exact line. Success or the precise resolution failure is
projected back into the review surface.

Accept, Correct, and Reject append ordered decision records. Each decision
names the immutable finding, reviewer, reason, and timestamp. The original
finding event is never edited, so a correction remains review history rather
than retroactive model output.

The renderer receives only this bounded review projection. Provider clients,
session credentials, source bytes, artifact bytes, hidden model reasoning, and
private runtime logs remain outside it. The structured claim, causal, receipt,
metric, and lifecycle map is sufficient for review.
