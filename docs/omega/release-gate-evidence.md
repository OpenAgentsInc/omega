The owner-evidence manifest uses
`openagents.omega.release-gate-owner-evidence.v1` and the checked-in schema at
`script/fixtures/omega-release-gate/owner-evidence.schema.json`. Every supplied
row is bound to this candidate's package digest and source revision, names its
observer and timezone-qualified observation time, and carries repository-local
public-safe evidence with an exact SHA-256. The independent reviewer must
differ from the candidate producer. The distribution row additionally refuses
to pass unless the Episode 263 transcript, website, release notes, installed
binary, and announcement copy agree claim for claim.

This checked-in report is the v0.2.0-rc29 receipt, not a 0.2.0 release
verdict. Its owner-assisted and blocked rows are the exact known limits. A
later candidate must replace the candidate metadata and evidence; source tests
or closed implementation issues cannot promote this receipt.

As of 2026-07-31 the cut remains **no-go for a 0.2.0 release**, but rc29 is a
valid capture and review candidate: it is Developer ID signed, notarized, and
stapled on both `Omega.app` and the disk image, and it is built from
`c7ecf769e8`, which carries every LiveKit desktop commit and the whole owner
review cycle that landed after rc28. Omega issues
[#185](https://github.com/OpenAgentsInc/omega/issues/185),
[#186](https://github.com/OpenAgentsInc/omega/issues/186), and
[#187](https://github.com/OpenAgentsInc/omega/issues/187) still own the
unfinished LiveKit desktop and installed-evidence journeys. Issue
[#189](https://github.com/OpenAgentsInc/omega/issues/189) may now capture
against this candidate digest instead of rc28. Every row shown below as
`owner-assisted-pending` still needs observation on this exact package, and the
distribution and independent-review rows need the strict owner manifest
described above.

The six Sarah LiveKit rows are no longer blocked on a missing manifest.
[`2026-07-31-omega-v0.2.0-rc29.sarah-livekit-evidence.json`](release-gate/2026-07-31-omega-v0.2.0-rc29.sarah-livekit-evidence.json)
binds them to this exact package digest, source commit, LiveKit server and
Sarah worker image digests, infrastructure and configuration revisions, and the
pricing and model inputs in force in production; binding SHA-256
`e1ed7f02ae9f298c976e64ec2d87cc7ba45fcffca5e5099274a9db96c4ab4f52`. The rows
now carry their true state, and none of them is green.

The only substantive live artifact behind them is the passing two-room
acceptance receipt on OpenAgents `main`, preserved here as
[`openagents-2026-07-31-ep263lk-community-join-acceptance.json`](release-gate/sarah-livekit/openagents-2026-07-31-ep263lk-community-join-acceptance.json)
(`sha256:fd4381da…`). It proves two concurrent Sarah sessions, one private and
one community, with real audio and transcription, 67 ms and 110 ms interrupt
acknowledgements, one accounted cancelled response each, exact provider usage
reconciled to distinct settled charges of 74368 and 19520 msat, and audible
fanout to two subscribers. It is a **headless Node harness** run, not the
packaged candidate, and two headless subscribers are **not** three
authenticated desktops. On that strength `sarah-livekit-private` and
`sarah-livekit-isolation` are `inconclusive`; `sarah-livekit-room`,
`sarah-livekit-connectivity`, `sarah-livekit-failure`, and
`sarah-livekit-independent-review` stay `blocked`.

What each row is still missing is named exactly in its receipt under
`docs/omega/release-gate/sarah-livekit/`, together with a `refused_claims` list
recording what was deliberately not promoted. In summary: no packaged-desktop
private journey (admission terms before capture, bounded command, agent thread,
reconnect without a second provider generation); no three-desktop community
journey (floor transfer, shared answer, moderator stop, and the non-floor,
stale-grant, replay, and removed-member refusals); zero of the three forced
UDP / TCP / TURN-TLS transport captures, and an ICE path type the harness never
records at all; no preserved cross-room provider-generation comparison and no
live community refusal of a private editor fact or a privileged tool; none of
the bounded failure drills, whose production route is gated by
`SARAH_LIVEKIT_PROVIDER_DISCONNECT_ACCEPTANCE_ENABLED`, currently `false`; no
eight-scope privacy scan, which both acceptance receipts name as a stated
limitation; no media-key rekey proof; and no reviewer distinct from the
candidate producer. Two binding fields are deliberately unversioned because
Sarah has no model catalog revision and no price catalog revision, only pinned
constants and raw production environment values.

Source-readiness note, 2026-08-01: the community-room controls after rc29 are
registered `community_sarah::*` GPUI actions, individually admitted by zero
base, handled by the selected channel, and bound in all three default keymaps.
Pointer, keyboard, and direct dispatch therefore reach the same governed
control path without experimental Accessibility automation. This removes the
client-driver prerequisite for a later candidate; it does not change any rc29
row or claim that the live three-desktop journey has occurred.

Harness hermeticity note, 2026-07-31, now fixed: the first `control-crawl` run
against this candidate reported `automated-fail` because a cached test binary in
the shared `target/` directory had a deleted worktree's `CARGO_MANIFEST_DIR`
compiled into it, so `CrawlRegistry::load_from_repository` read a path that no
longer existed. The row could therefore report a build-cache state rather than a
source state, which was a defect in the gate and not in the candidate. The gate
now runs `cargo clean -p omega_control_crawl` before the test, records
`forced_rebuild` and the clean exit code in the row facts, and the row above is
that forced rebuild. The whole gate run still completes in about a minute, so
the fix costs a known rebuild instead of an unknown cache.

Assemble that manifest with
`script/assemble-omega-sarah-livekit-evidence`. It takes the DMG and release
record, the exact OpenAgents/LiveKit/worker/model/pricing revisions, and six
`--row ID=STATUS:PATH` arguments. Each path must be a repository-contained JSON
receipt that repeats the exact candidate/infrastructure `binding`, its
`row_id`, `status`, `facts`, and `public_safe: true`. The assembler computes
the package and receipt digests; the gate independently recomputes them.
Passing rows also require their row-specific observed facts. For example, the
room row requires three authenticated desktops, a completed floor transfer,
shared answer, moderator stop, and non-floor/removed-member refusals. The
failure row requires all eight drills, eight privacy scopes, non-overlap, and
exact settlement. A public-safe label without that shape cannot promote a
row.

The private and connectivity rows additionally require the packaged client's
`openagents.omega.sarah-livekit-transport-evidence.v1` objects in
`facts.transport_observations`. Omega writes them to
`<Omega data dir>/voice/livekit-transport-evidence.jsonl`. The gate recomputes
`direct_udp`, `tcp_fallback`, and `turn_tls` from both candidates' type,
protocol, and relay protocol; the three booleans cannot pass by themselves.
For the private reconnect, one `connected` and one `reconnected` object must
preserve identical session, room, dispatch, and provider-generation digests.
Addresses, ports, URLs, grants, media, and transcripts are deliberately absent.
OpenAgents must provide the opaque `providerGenerationRef` in the authenticated
private `session_ready` frame; Omega cannot infer it from the LiveKit room.

```sh
script/assemble-omega-sarah-livekit-evidence \
  --dmg target/omega-rc/Omega-v0.2.0-rcNN-macos-arm64.dmg \
  --release-record target/omega-rc/omega-v0.2.0-rcNN-macos-arm64.release.json \
  --output docs/omega/release-gate/<date>-rcNN.sarah-livekit.json \
  --openagents-source-revision <git-sha> \
  --livekit-infrastructure-revision <revision> \
  --livekit-config-revision <revision> \
  --livekit-image-digest sha256:<digest> \
  --sarah-worker-revision <git-sha> \
  --sarah-worker-image-digest sha256:<digest> \
  --admitted-model-revision <revision> \
  --price-catalog-revision <revision> \
  --row sarah-livekit-private=owner-assisted-pass:<receipt.json> \
  --row sarah-livekit-room=owner-assisted-pass:<receipt.json> \
  --row sarah-livekit-connectivity=automated-pass:<receipt.json> \
  --row sarah-livekit-isolation=automated-pass:<receipt.json> \
  --row sarah-livekit-failure=automated-pass:<receipt.json> \
  --row sarah-livekit-independent-review=owner-assisted-pass:<receipt.json>
```
