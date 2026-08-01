# Omega Forensics cloud control

Omega issue [#193](https://github.com/OpenAgentsInc/omega/issues/193) adds the
host-owned control path for one disposable OpenAgents Cloud forensic worker.
The native route contract is pinned by OpenAgents commit `684220be7b` and lives
at `POST /api/forensics/workers`.

## Authority boundary

The Forensics renderer emits typed launch, refresh, cancel, and cleanup
commands. The Agent Panel host resolves the already verified OpenAgents
session and passes its bearer only to `ForensicsCloudClient`. The client accepts
only `https://openagents.com`, calls only the native forensic route, bounds the
response body, and never stores the credential. The renderer receives worker
refs, generations, receipt refs, event kinds, sequence numbers, timestamps,
and typed failure state. The control projection never receives source bytes,
prompts, runtime text, credentials, findings, or private evidence. The separate
[Forensics review](omega-forensics-review.md) projection may display bounded
owner-authorized finding text and evidence metadata after contract validation;
it still carries no credential or provider authority.

This path does not call the generic cloud-GCP lane and does not use the Box v1
facade. OpenAgents remains the placement, budget, runtime, and cleanup
authority.

## Lifecycle and retry

An explicit **Launch worker** action creates one stable run ref and one stable
admit command/idempotency pair. Replaying the same command may return the same
placement, but the Omega reducer rejects a different sandbox, attachment
generation, or resource generation as a duplicate worker.

**Refresh events** sends `Observe` with the last accepted native sequence. The
route synchronizes the turn and returns a contiguous, generation-matched page.
Omega rejects gaps, foreign generations, cursor rewinds, and any response that
claims silence is terminal. An empty page leaves the current phase unchanged.

For a running turn, **Cancel and clean up** performs the native interrupt and
inspect sequence, requires structural settlement, observes the cancellation
events, then deletes the worker and requires the cleaned placement with both
deletion and cleanup receipts. An idle ready worker exposes **Delete and verify
cleanup** so a pre-dispatch worker cannot be stranded.

The public projection records separate timestamps for admission request,
worker readiness, run start, cancel request, interrupt observation, structural
settlement, deletion request, and cleanup observation.

Broker outages and invalid response truth become `recovery_required`. Budget
or admission conflicts become typed refusals. Runtime failures remain failed.
These states never collapse into a generic network error or a false completed
state.

## Coldcard binding

The vulnerable and incomplete benchmark arms use the same authoritative source
commit as OpenAgents OFR-003:
`7abc9a4c680b5623fc8a64f70555dd2d3802e488`. That keeps Omega preflight,
materialization, worker admission, and later benchmark receipts on one exact
source graph.
