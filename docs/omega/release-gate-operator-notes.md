## Omega 0.2.0 Sarah LiveKit cutover plan

**Owner direction, 2026-07-30.** Fold the Sarah promises in
[`OpenAgents Episode 263`](https://github.com/OpenAgentsInc/openagents/blob/main/docs/transcripts/263.md)
into the self-hosted LiveKit design in the
[unified LiveKit teardown](https://github.com/OpenAgentsInc/openagents/blob/main/docs/teardowns/2026-07-30-livekit-armada-buzz-zed-teardown.md).
This section is an accepted release plan, not evidence that the rc29 candidate
above has been observed on the new transport. rc29 is the first candidate whose
source carries the `livekit_room_v1` desktop path. A candidate-bound evidence
manifest now exists for it, and it resolves the six Sarah LiveKit rows to
`inconclusive` or `blocked` rather than to a pass: nothing in this plan has yet
been observed on the packaged candidate. A later candidate must regenerate the
table and bind every added row to its own source and package digest.

### Release decision

Omega 0.2.0 will not treat the current custom PCM WebSocket as the completed
Sarah architecture. The release target is:

- the existing editor input-bar voice entry starts an owner-private LiveKit
  room and explicitly dispatches Sarah;
- Sarah joins as one named server participant backed by one
  `gpt-realtime-2.1` session for that exact room execution;
- an authenticated desktop community room can invite or summon the same
  `principal.sarah` and Nostr public identity;
- group members use a server-validated speaking floor so the agent listens to
  one attributed participant at a time and publishes one answer track that
  the room can hear;
- OpenAgents retains admission, membership, capability, credit hold, exact
  provider usage, proposal/confirmation, settlement, and Nostr-signing
  authority;
- self-hosted LiveKit, TURN, Redis, and Sarah Agents workers run on Google
  Cloud; OpenAI remains the inference provider; and
- `custom_wss_v1` remains a rollback/control cohort until the LiveKit path is
  independently green. An active conversation never changes transport
  silently.

The stable Sarah identity is shared; provider sessions, room context,
capability profiles, holds, usage, and settlement are not. Every private
conversation and community room receives a separate generation. Owner-private
editor memory never enters a community prompt.

### Episode 263 promise mapping

| Spoken promise | Omega 0.2.0 meaning |
| --- | --- |
| “Command me, Sarah, acting as your personal lieutenant.” | Sarah remains a first-class desktop executor with one durable identity. “Lieutenant” does not amplify authority: editor and agent-start effects still use the existing typed proposal, confirmation, and receipt boundaries. Community Sarah has no owner-private workspace or command capability. |
| “requiring voice commands” | Private editor voice and allowlisted desktop room speech use LiveKit media. In a group, “Talk to Sarah” creates a bounded floor lease and selects the exact participant; ambient mixed-room inference is outside 0.2.0. |
| “and expensive credits” | The UI shows exact price-catalog revision, hold, rate, duration/spend limit, remaining balance, and final charge. Exact OpenAI `response.done` usage drives settlement. Google Cloud and OpenAI budgets remain separate. |
| “tester channels … in the sidebar” | A supported desktop tester/community channel can create a live room epoch and visibly invite or summon Sarah. Text membership remains NIP-29/OpenAgents authority; LiveKit is media only. |

### In scope for 0.2.0

1. A versioned `livekit_room_v1` admission, participant-grant, explicit
   dispatch, generation, usage, and settlement contract.
2. A Google Cloud connectivity canary followed by the admitted self-hosted
   production candidate: direct UDP, TCP fallback, TURN/TLS, Redis, metrics,
   rollback, and bounded capacity.
3. A narrowly scoped `agents-js` Sarah worker using the LiveKit OpenAI
   Realtime plugin, the admitted model/voice/instructions, exactly one
   turn-detection owner, current proposal adapter, and exact usage forwarding.
4. A LiveKit Rust media adapter beneath Omega's existing Sarah controls. One
   audio layer owns capture, processing, playback, interruption, mute, and
   cleanup.
5. A desktop community-room Sarah path with explicit dispatch, visible
   presence/listening/speaking states, a server-validated speaking floor,
   signed public-safe Sarah presence/text, and fail-closed identity mapping.
6. Installed-candidate private and group journeys, cost and failure evidence,
   independent review, and a proven rollback to stop issuing new LiveKit
   sessions.

### Explicitly deferred beyond 0.2.0

- OpenAgents Mobile and the LiveKit React Native/Expo client.
- Cross-device desktop/mobile voice handoff.
- Untrusted public rooms. The OpenAI per-end-user safety-identifier treatment
  for one provider session with several speakers needs a documented answer
  before that expansion.
- Ambient room mixing, automatic active-speaker switching, resident Sarah in
  idle rooms, telephony, recording, egress, SIP, and video input.
- Moving command authority, transcripts, workspace state, raw media, or
  settlement into LiveKit data, metadata, or Redis.

### Required new release rows

`script/omega-release-gate` now emits these rows into every receipt and refuses
to promote them without a
`openagents.omega.sarah-livekit-evidence.v1` manifest. The manifest must bind
the exact Omega package/source, OpenAgents source, LiveKit
infrastructure/config/server image, Sarah worker source/image, admitted model,
and price catalog. Every evidence reference must exist in this repository,
remain repository-relative, match its SHA-256, and be marked public-safe.
Missing or mismatched evidence is `blocked`; an incomplete failure observation
is `inconclusive`. Until all six rows are green, `sarah-journey` remains
pending and omega#160 cannot close.

| Planned row | Required installed evidence |
| --- | --- |
| `sarah-livekit-private` | An eligible desktop user sees exact admission truth, receives `livekit_room_v1`, completes a voice turn, confirms one bounded command, starts an agent thread, interrupts Sarah, survives an allowed media reconnect without opening a second provider generation, ends, and sees the exact final charge. |
| `sarah-livekit-room` | At least three authenticated desktop participants join one tester/community room, summon Sarah, transfer the explicit floor between two members, hear Sarah's answer, see the same verified Sarah identity, and prove that a non-floor participant and a removed member cannot feed the model. |
| `sarah-livekit-connectivity` | The packaged client completes direct UDP, TCP fallback, and TURN/TLS journeys against the exact self-hosted Google Cloud candidate. |
| `sarah-livekit-isolation` | Two Sarah rooms run concurrently with distinct provider generations, context, holds, usage, and settlement; a private editor fact and privileged tool are unavailable in the community room. |
| `sarah-livekit-failure` | Worker crash, SFU loss, OpenAI disconnect, replayed grant, duplicate participant, membership revocation, and hold exhaustion produce bounded failure, no overlapping billable session, no raw-media retention, and deterministic settlement or an explicit inconclusive receipt. |
| `sarah-livekit-independent-review` | A reviewer other than the producer repeats the held-out private and group journeys against the exact package and infrastructure revisions. |

### Issue program

The master tracker and seven implementation packets use stable IDs so this
document remains legible before and after GitHub assigns issue numbers:

| Packet | Repository | Outcome |
| --- | --- | --- |
| [`EP263-LK-00`](https://github.com/OpenAgentsInc/openagents/issues/9282) | OpenAgents | Track the complete cross-repository Omega 0.2.0 Sarah LiveKit cutover and block omega#160 until every child and release row is green. |
| [`EP263-LK-01`](https://github.com/OpenAgentsInc/openagents/issues/9283) | OpenAgents | Add the LiveKit room admission, dispatch, generation, grant, usage, and settlement control plane. |
| [`EP263-LK-02`](https://github.com/OpenAgentsInc/openagents/issues/9284) | OpenAgents | Operate the self-hosted LiveKit Google Cloud canary and production candidate with TURN, Redis, observability, capacity, and rollback evidence. |
| [`EP263-LK-03`](https://github.com/OpenAgentsInc/openagents/issues/9285) | OpenAgents | Run Sarah as an `agents-js` worker over OpenAI Realtime while preserving current proposals, exact usage, reconnect, and privacy laws. |
| [`EP263-LK-04`](https://github.com/OpenAgentsInc/openagents/issues/9286) | OpenAgents | Bind Sarah's Nostr identity to room presence and implement the authenticated group speaking-floor service. |
| [`EP263-LK-05`](https://github.com/OpenAgentsInc/omega/issues/185) | Omega | Add the LiveKit Rust media transport beneath the existing Sarah voice controls. |
| [`EP263-LK-06`](https://github.com/OpenAgentsInc/omega/issues/186) | Omega | Let Sarah join authenticated desktop community rooms and talk through the explicit floor. |
| [`EP263-LK-07`](https://github.com/OpenAgentsInc/omega/issues/187) | Omega | Extend the installed release gate and prove the private, group, connectivity, isolation, failure, and independent-review rows. |

The #186 desktop slice is intentionally gated at the server boundary. Omega
has the compact tester-channel projection, verified epoch/participant model,
canonical floor request, and fail-closed test scenes. OpenAgents now exposes
authenticated snapshot, summon/remove, member/moderator floor routes and
worker enforcement, but its production provisioner still derives
`roomRef`/`sarahPresenceLeaseRef` from each private
`sessionRef:generation`. It does not expose a community/channel rendezvous
that lets three independently authenticated desktops discover and join one
stable room. Omega therefore cannot honestly enable production controls yet:
doing so would create one room per desktop rather than the #186 journey.
Installation alone is not evidence for any release row; OpenAgents must first
publish a stable community-room join contract, then Omega must consume it and
the installed three-desktop journey above must pass.


---

# Owner review ledger and agent handoff — 2026-07-30

The owner is reviewing rc28 live. This section is the complete, self-contained
ledger of every owner feedback item and in-flight process so a fresh agent can
pick up any lane cold if the current agents die (session limits / crashes).

**Coordinating-session state as of 2026-07-30 ~14:30 CDT (Grok cold-start
resume):** review batches 1–3 and the QA crawl scaffold are **LANDED** on
`origin/main`. ZEDREMOVE remains **OPEN** (owner-gated; not started this run).
Verify current truth against `gh issue list -R OpenAgentsInc/omega` and
`git log origin/main` before acting.

| Land | SHA | Delta / note |
| --- | --- | --- |
| Item 17 QA crawl scaffold | `9b10652f19` | OMEGA-DELTA-0187 |
| #162 preview/outline/repl batch | `9098f72404` | extends 0186 |
| Batch 1 items 1–3 | `4c9b485511` | OMEGA-DELTA-0188 |
| Batch 3 items 11–15 | `806ed31257` | amends 0182/0183 |
| Batch 2 items 4–10 | `9c3f4d0086` | OMEGA-DELTA-0189 |

## Standing owner laws (apply to ALL current and future UI work)

1. **Drawn implies working.** A visible control has an admitted action, a
   loaded dependency, and a visible result. Enabled-looking no-ops are
   defects. (Origin: single-experience plan §2.5; enforced by the growing
   crawl gate below.)
2. **No exposition in the UI, anywhere.** Never render explanatory sentences
   about internal mechanics ("The executor is free to change until…",
   "route selected when sent"). Controls are labeled, not narrated. One-word
   tooltips acceptable. (Owner, 2026-07-30, verbatim: "never include that
   level of exposition in the UI ANYWHERE and if there's anything like it,
   remove it.")
3. **Statuses are colors/icons, never words.** Sidebar lifecycle states show
   a colored dot/icon only (running/waiting/failed/completed/cancelled);
   a one-word tooltip is the maximum copy.
4. **Escape closes every modal/auxiliary window** (settings, pair phone, any
   future modal). The crawl gate asserts this per surface.
5. **No screen between `+` and a blinking cursor.** New thread lands in a
   focused composer; executor choice is the composer dropdown (omega#165).
6. **Version copy is short.** Footer: `v0.2.0` (stable), `v0.2.0 bNN`
   (preview; NN = the RC counter, one increment per build of a version),
   `v0.2.0 dev` (dev). Full sha/channel live only in the release record,
   About, and copy-system-specs.

## Feedback item ledger

State legend: LANDED (verified on main) · IN-FLIGHT (an agent holds it — verify
before duplicating) · OPEN (nobody holds it).

### Review batch 1 (agent: outline/thinking/version)
| # | Item | Spec | State |
|---|---|---|---|
| 1 | Delete the thread Outline right sidebar entirely | Remove `crates/agent_ui/src/thread_outline.rs` + all wiring (rail/menu/keymap/actions/scenes). Deletion discipline: keymap strips in the same commit (`keymaps_name_no_deleted_action` guards the startup panic), extend `REMOVED_FILES` + `FORBIDDEN_KEYMAP_NAMESPACES`, delta entry (owner direction 2026-07-30), delete its ~14 visual baselines (most of the documented pre-existing red scenes are outline scenes — the known-red list shrinks), update proof docs. Scope: the AGENT THREAD outline pane only — NOT the workbench Plan surface, NOT the buffer outline. | LANDED (OMEGA-DELTA-0188) |
| 2 | `Open Thread as Markdown` emits trailing empty `<thinking></thinking>` | Exporter must never emit empty thinking blocks. Regression test: exported markdown for a thread with empty/absent thinking contains no empty `<thinking>` tags. | LANDED |
| 3 | Footer version too long (`v0.2.0+preview.<fullsha>`) | Implement law 6 above. Reuse the RC counter in `script/bundle-omega-rc` as the build number (rc28 → `b28`); stamp via env→compiled constant like `ZED_COMMIT_SHA`. Update `.github/ISSUE_TEMPLATE/10_bug_report.yml` + `11_crash_report.yml` version-field hints, `docs/src/alpha-feedback.md`, and the release-gate `version-truth` OCR row (keep it binding version+build to the release record). | LANDED |

### Review batch 2 (agent: exposition/status sweep)
| # | Item | Spec | State |
|---|---|---|---|
| 4 | Record the no-exposition law | Product-contract/delta note, mechanically citable. | LANDED (`OMEGA-DELTA-0189`) |
| 5 | Kill the executor-dropdown tooltip essay | "This conversation will run on Omega Agent. The executor is free to change…" — delete, no replacement copy. | LANDED |
| 6 | Remove the routing-mode dropdown entirely | The second composer dropdown ("Run this new conversation on" → Automatic/Omega). Routing stays automatic (OMEGA-DELTA-0179 behavior unchanged); only the selector + its state/actions die. Amend 0184's row inventory if pinned. | LANDED |
| 7 | Remove "Omega router ready · route selected when sent" | Plus sweep the composer/empty states for any similar "ready — X when Y" status sentences. | LANDED |
| 8 | Sidebar statuses → color/icon only | Law 3. Amend the 0181 check if it pins label text. Keep existing color semantics. | LANDED |
| 9 | Remove "Owner unverified — legacy thread" sidebar annotation | The legacy-ambiguity fact stays internal (from omega#152 versioned owner metadata); no sidebar copy. | LANDED |
| 10 | Settings window closes on Escape | `crates/settings_ui`; test if harness allows. | LANDED |

### Review batch 3 (agent: channels/backup/pair-phone)
| # | Item | Spec | State |
|---|---|---|---|
| 11 | Pair phone closes on Escape | Same modal law; the sidebar-footer pairing surface. | LANDED (`806ed31257`) |
| 12 | Dedicated `omega-alpha-feedback` NIP-29 channel | Currently "Alpha feedback" wrongly adapts the `openagents-public` group. Create the new channel on the owned relay (check NIP-29 creation semantics; relay runbook: openagents repo `docs/ops/2026-07-24-owned-nostr-relay-deploy.md`; unmanaged groups may auto-create on first signed message — verify LIVE with a test identity before hardcoding). Hardcode it in `crates/agent_ui/fixtures/tester-channel-registry.v1.json` + the bundled pinned fallback (omega#156 work). KEEP `openagents-public` listed — sidebar shows BOTH. Amend the 0182 check. If creation truly needs an admin key, do everything else and record the one-step admin action in NEEDS_OWNER. | LANDED client hardcode (`806ed31257`); **relay group creation remains NEEDS_OWNER** (see `docs/omega/NEEDS_OWNER.md`) |
| 13 | Channel rows styled like the thread list | No blue link text; same color/weight/size/hover as recent threads. | LANDED (`806ed31257`) |
| 14 | Delete the stray "Live" text under Tester channels | | LANDED (`806ed31257`) |
| 15 | Backup-key notice click opens a real surface | The omega#164 nudge (OMEGA-DELTA-0183) is an enabled no-op. Minimal honest v1: modal revealing the nsec via the identity custody path, copy button, ONE short warning line, Dismiss. Regression test the click produces a surface. | LANDED (`806ed31257`; amends OMEGA-DELTA-0183) |

### ZEDREMOVE (its own agent, two phases)
| # | Item | Spec | State |
|---|---|---|---|
| 16a | Visible-Zed purge (phase 1) | The `/zed/` settings path shown in UI → Omega-branded path with startup MIGRATION (copy old→new, prefer new, never destroy old silently; delta records semantics). Sweep visible strings via `script/omega-brand-gate.json` classifications; clean zed-industries links + "Zed version" fields in the issue templates; add `OMEGA_`-prefixed env vars taking precedence over `ZED_*` (bundle scripts set both). | LANDED (omega#174 / OMEGA-DELTA-0190) |
| 16b | Crate rename (phase 2) | `crates/omega` → `crates/omega`, `zed_actions` → `omega_actions`, `script/zed-local` renamed. CRITICAL: action namespaces in keymaps must keep resolving (verify whether palette display is already `omega::` via the macro before assuming rename is internal-only); every omega_deltas source-literal check naming `crates/omega/...` paths updates in the same commit; Cargo workspace members/default-members/dependents/scripts/CI/docs all move together. COORDINATE: do not push while the #162 epic is mid-batch — land after #162 closes, or file the deferral. | OPEN (gated on #162 close + owner task) |

### QA process (its own agent)
| # | Item | Spec | State |
|---|---|---|---|
| 17 | Control-crawl gate | Hermetic-runner harness: enumerate every interactive element per scene via the semantic tree; activate each (pointer AND keyboard); FAIL on zero observable consequence unless a registered exemption names a reason. Menu entries individually activated (catches the display-only `ContextMenuEntry.action` trap structurally). Escape-dismissal asserted for every modal opened. Checked-in crawl registry; new surface without registration fails a delta check. Wire: full crawl into `script/omega-release-gate` as a new automated row; core-scene crawl into `cargo test`. Copy lint for multi-sentence tooltips/status strings (law 2) with an allowlist file. Process doc `docs/src/qa-process.md` (cadence, ownership, severity ladder from `docs/src/alpha-feedback.md`, the same-commit registration law). Mutation proof: a deliberate no-op control must fail the crawl. First run against main: fix or file everything it catches. | LANDED scaffold (OMEGA-DELTA-0187): process doc, registry, copy allowlist, `crates/omega_control_crawl` proving scene + mutation proof, release-gate `control-crawl` row. Hermetic GPUI scene expansion remains `pending-expansion` in the registry. |

## Landed earlier in this review cycle (context, do not redo)
- omega#165 composer executor dropdown replacing the full-screen chooser (`10637aa422`); title flush-left (`8371806042`); manifest version 0.2.0 (`a954971025`); workbench revision-spam reconcile (`7dcc27e1e8`); Sarah voice barge-in drop fix (`bc968334ac`); adapter-death honest failure (`fd9f21ff48`); #166 keyboard trap + #168 Sarah no-op (`04e629a77f`); #167/#169/#170 (`04df6e7674`); #161 mode-split removal (`5823eb686b`); #164 background identity (`562319fb4f`); rc28 notarized + published + on openagents.com/download (Cloud Run `00303-44b`).
- omega#217 / OAW-012 supersedes the earlier omega#71 and omega#171 alpha
  deferrals. Omega now enables GPUI's macOS AccessKit bridge by default, and a
  v0.2.0 candidate cannot waive the installed accessibility-tree observation.

## Open issues map (verify live before acting)
- Sarah LiveKit cutover: master
  [openagents#9282](https://github.com/OpenAgentsInc/openagents/issues/9282);
  backend/infra children
  [#9283](https://github.com/OpenAgentsInc/openagents/issues/9283),
  [#9284](https://github.com/OpenAgentsInc/openagents/issues/9284),
  [#9285](https://github.com/OpenAgentsInc/openagents/issues/9285), and
  [#9286](https://github.com/OpenAgentsInc/openagents/issues/9286); Omega
  desktop/proof children [#185](https://github.com/OpenAgentsInc/omega/issues/185),
  [#186](https://github.com/OpenAgentsInc/omega/issues/186), and
  [#187](https://github.com/OpenAgentsInc/omega/issues/187). Mobile is outside
  0.2.0.
- #151–#156, #160 — close on the owner-assisted rows above + owner verdict. Code review batches 1–3 + QA scaffold are landed; owner-assisted gate rows and live channel proof remain.
- #162 — crate-deletion epic continuing. Latest batch `9098f72404` deleted previews, diagnostics panel, buffer outline/outline_panel, repl, tab_switcher, auto_update_ui. **Still owner-kept:** file_finder, go_to_line/cursor position, language_tools LSP logs, acp_tools. Long tail remains (selectors, project_symbols, extensions_ui, etc.).
- #163 — proof inversion; dispatch AFTER #162 closes (gate-as-tripwire, refusal-log-empty proof rows, per-surface drawn-implies-working delta checks, docs de-moding).
- #172 — composer menu anchor; needs an installed repro via the landed `omega.composer.executor-menu.popup` position probe; queue behind #162 (build contention).
- #173 — PR that landed Batch 3 (merged).
- Server residuals (openagents repo): relay auto-admission of fresh background identities for tester rooms; Sarah gateway accepting first-seen identities (recorded on omega#164); **create/confirm NIP-29 group `omega-alpha-feedback`** on the owned relay.
- NEEDS_OWNER: Reduce Motion toggle for the strict reduced-motion cell; a throwaway hosted identity / scratch GEMINI_API_KEY for paid-path cells; live `omega-alpha-feedback` group creation (recorded in `docs/omega/NEEDS_OWNER.md`).
- B-roll capture for Episode 263 (coordinating-session task): shot list in the openagents session ledger; capture from the installed rc28+ build to `~/Desktop/Sarah/263/broll/`.
- ZEDREMOVE items 16a/16b — OPEN, owner-tasked separately.

## Handoff protocol for a fresh agent
1. `gh issue list -R OpenAgentsInc/omega --state open` + `git log origin/main -20` — establish what actually landed; IN-FLIGHT items above may be done or half-done. Search main's history for the item's keywords before starting.
2. Claim by commenting on omega#160 (the review thread) with the item numbers you take.
3. Fresh worktree off origin/main per item batch; never touch the primary checkout at `~/work/omega` (frequently dirty with another lane's live work); rebase over concurrent pushes and re-run `cargo test -p omega_deltas` + touched-crate tests after every rebase.
4. Delta discipline: entry + check + test change together; never weaken a check to pass. Keymap edits ride the same commit as any action/crate deletion.
5. Land = pushed to origin/main + issue comment (+ close where acceptance is met) + worktree removed.

## Recording the human-only Sarah LiveKit gate journeys

### Prepare one candidate-bound three-instance run

Use `script/omega-sarah-livekit-candidate-run` before the private, room, or
connectivity observations. Its private run directory must be outside this
repository. `prepare` recomputes the DMG digest, checks the release record and
full Sarah infrastructure binding, verifies the exact installed Omega binary
digest and code signature, and creates three mode-0700 homes and data roots.
`launch --apply` starts the installed binary three times with those isolated
`--user-data-dir` roots; it does not reuse the owner's Omega, Codex, Claude, or
Grok profile.

```sh
script/omega-sarah-livekit-candidate-run prepare \
  --dmg target/omega-rc/Omega-v0.2.0-rcNN-macos-arm64.dmg \
  --release-record target/omega-rc/omega-v0.2.0-rcNN-macos-arm64.release.json \
  --binding /private/path/rcNN-sarah-binding.json \
  --run-dir /private/path/rcNN-sarah-run

script/omega-sarah-livekit-candidate-run launch \
  --plan /private/path/rcNN-sarah-run/run-plan.json \
  --apply
```

Perform all three journeys in these exact instances. Fill a private
`openagents.omega.sarah-livekit-candidate-observations.v1` finding file with
the three profile-reference digests from the plan, the private reconnect
session digest, the room findings, and one acceptance-result/session digest
for each imposed connectivity constraint. Each cell names the corresponding
OpenAgents `openagents.sarah.livekit-acceptance.v4` receipt path; capture
recomputes its `resultDigest`, checks its deployed worker revision and declared
constraint, and emits only the digest. `prepare` writes the complete
fillable shape to `observations-template.json` in the private run directory;
copy it there under a run-specific name and fill it while observing the
journeys. Then preserve the public-safe capture:

```sh
script/omega-sarah-livekit-candidate-run capture \
  --plan /private/path/rcNN-sarah-run/run-plan.json \
  --runtime /private/path/rcNN-sarah-run/runtime.json \
  --observations /private/path/rcNN-observations.json \
  --output docs/omega/release-gate/sarah-livekit/rcNN-candidate-capture.json
```

Capture fails closed unless all three processes are still the exact installed
binary, all three profiles are authenticated packaged desktops for the bound
DMG, private `connected` and `reconnected` receipts preserve one provider
generation, and the three distinct transport sessions exclusively classify as
direct UDP, TCP fallback, and TURN/TLS under their declared constraints. It
reads the JSONL from each planned profile rather than accepting copied ICE
booleans. Addresses, ports, URLs, tokens, raw media, and transcripts are
rejected from the public capture.

Passing `sarah-livekit-private`, `sarah-livekit-room`, and
`sarah-livekit-connectivity` row receipts must repeat a
`facts.candidate_capture_ref` with the repository-relative path, SHA-256, and
`public_safe: true`. The release gate reopens that capture, recomputes its
digest and collector binding, and compares each row's facts to the captured
section. A hand-set pass boolean cannot substitute for this file.

**Added 2026-07-31.** The rc29 evidence manifest resolved all six Sarah LiveKit
rows to `blocked` or `inconclusive`, and named two of those blockers as
*harness incapability rather than missing human effort*: the acceptance receipt
had no field for floor, moderator, membership, or refusal, and ICE observation
was a bare boolean that never recorded candidate type or protocol. An operator
could have performed every journey perfectly and the results would have
evaporated.

That recorder now exists. This section is the operator runbook for the three
journeys that only a human can perform. Read it end to end before starting a
run; the ordering matters, and a journey performed without recording its
observations produces nothing.

**These instructions record evidence. They do not make any row green.** A
receipt with `"outcome": "observed_pass"` means the observations were preserved
against a named binding. Admitting a release-gate row remains the evidence
manifest's decision.

### Where the tools and receipts live

All commands run from a clean `openagents` checkout at current `origin/main`.

| Thing | Path |
| --- | --- |
| Journey recorder CLI | `pnpm --dir apps/sarah-livekit-agent gate-observation` |
| Acceptance run CLI | `pnpm --dir apps/sarah-livekit-agent acceptance` |
| Failure matrix CLI | `pnpm --dir apps/sarah-livekit-agent failure-matrix` |
| Journey receipts | `docs/ops/receipts/livekit/gate/<name>.json` |
| Acceptance receipts | `docs/ops/receipts/livekit/<name>.json` |
| Recorder contract | `apps/sarah-livekit-agent/src/gate-observation.ts` |

Every live run requires the cost gate in the environment:

```sh
export OA_LIVEKIT_OWNER_GATE=I_ACCEPT_EP263_LIVEKIT_GCP_COST
```

### The shape of every journey

The recorder is deliberately unforgiving in one direction: **an observation the
row requires may not simply be absent.** Every required key must be present,
either as a real finding or as an explicit `not_observed` with a reason. This is
what stops a partial run from quietly looking complete.

The three findings are:

- `satisfied` — the journey produced what the row requires.
- `contradicted` — the journey ran and produced the opposite. **Record this.** A
  refusal that did not refuse, a bound that was exceeded, or an isolation that
  did not hold is a finding, not a run to discard.
- `not_observed` — the journey did not reach this step. Requires a reason.

So every journey is the same three steps:

```sh
# 1. Get the complete list of what this row needs, as a fillable skeleton.
pnpm --dir apps/sarah-livekit-agent gate-observation -- \
  --row sarah-livekit-room \
  --template ~/sarah-gate/room-observations.json

# 2. Run the journey. Fill in the skeleton as you go, not from memory afterwards.

# 3. Record it.
pnpm --dir apps/sarah-livekit-agent gate-observation -- \
  --row sarah-livekit-room \
  --observations ~/sarah-gate/room-observations.json \
  --binding ~/sarah-gate/rc29-binding.json \
  --operator-ref "<your operator identity>" \
  --receipt docs/ops/receipts/livekit/gate/2026-XX-XX-rc29-room.json \
  --apply
```

Observation and binding inputs must live **outside** the repository, so a
half-filled draft cannot be committed. The operator ref is digested before
recording and never appears in the receipt. Receipts are written with `wx`, so a
second run against the same path fails rather than overwriting evidence.

To see what a row needs without writing anything, run it with `--row` alone.

### The binding file

Every journey binds to one exact candidate. Write this once per candidate and
reuse it for all journeys:

```json
{
  "omegaReleaseTag": "v0.2.0-rc29",
  "omegaPackageSha256": "<sha256 of the installed DMG, bare hex>",
  "openagentsSourceRevision": "<40-char openagents commit>",
  "livekitConfigRevision": "sha256:<configurationDigest of infra/livekit/production/livekit.yaml>",
  "sarahWorkerImageDigest": "sha256:<workerImage digest from infra/livekit/bundle.json>"
}
```

An unbound or malformed binding is refused before anything is written.

### Journey A — the three-desktop community room

**Row:** `sarah-livekit-room`. **Needs:** three people, three Macs, three
authenticated accounts, one installed candidate DMG on each.

A headless subscriber does not count and the recorder will refuse it: the
`authenticated_desktop_count` check requires at least three participants whose
`clientKind` is `packaged_omega_desktop` and whose `authenticated` is true. This
is the exact promotion the rc29 manifest refused, so it is enforced in code.

Run the journey in this order and record as you go:

1. **Join.** All three desktops join one community room. Record every
   participant under `authenticated_desktop_count` with their
   `participantRefDigest` (digest the ref, never record it raw), `clientKind`,
   and the candidate `packageSha256`.
2. **Summon and take the floor.** Member A summons Sarah and acquires the
   server-issued floor. Record `floor_acquired` with the floor lease's
   `issuance` and `toHolderDigest` (the lease's own `holderUserRefDigest`).
3. **Transfer the floor.** Move the floor from member A to member B. Record
   `floor_transfer_completed` with both `fromHolderDigest` and `toHolderDigest`
   and the new `issuance`. The recorder refuses a transfer whose two holders are
   the same member.
4. **Shared answer.** Sarah answers while B holds the floor. Confirm on each of
   the three machines that the answer was audible, then record
   `shared_answer_heard_by_all` with `audibleSarahOutputObserved: true` for every
   participant. If one machine did not hear it, that is `contradicted`.
5. **Moderator stop.** A moderator stops Sarah. Record
   `moderator_stop_completed` with `state: "stopped"` and `stopReason:
   "moderator_stop"`. Any other stop reason is refused for this key.
6. **The four refusals.** Each is a deliberate attempt that the server must
   refuse. Record the server's own refusal code and HTTP status, not a
   paraphrase:

   | Observation | Attempt | Expected code |
   | --- | --- | --- |
   | `non_floor_refused` | a member without the floor tries to drive Sarah | `not_floor_holder` |
   | `removed_member_refused` | a removed member tries to claim the floor | `member_removed` |
   | `stale_floor_grant_refused` | replay a grant from before a membership change | `membership_changed` |
   | `replayed_floor_grant_refused` | replay a previously used nonce | `nonce_replayed` |

   A 2xx here means the refusal did not happen: record `contradicted`. The
   recorder rejects a "refusal" filed with a success status.

### Journey B — the forced transport matrix

**Row:** `sarah-livekit-connectivity`. **Needs:** three private-journey
acceptance runs on the packaged candidate against the same infrastructure
binding, each under a different imposed network constraint.

ICE paths are now classified rather than counted. The acceptance run records the
selected candidate pair's candidate type, protocol, and whether it was relayed,
for both publisher and subscriber. Addresses, ports, and URLs are never
recorded.

The packaged Omega candidate itself preserves those observations under its
profile data directory at
`voice/livekit-transport-evidence.jsonl`. Copy the exact JSON objects into the
connectivity row's `facts.transport_observations`; do not transcribe only the
classification or set the three completion booleans by hand. The release gate
recomputes each classification from the local and remote candidate type,
protocol, and relay protocol and refuses a pass unless all three measured
classes survive. A row from the headless OpenAgents harness may corroborate the
same run, but it does not substitute for this packaged-client receipt.

Run the acceptance three times, declaring the constraint you actually imposed:

```sh
# Cell 1 — no constraint.
pnpm --dir apps/sarah-livekit-agent acceptance -- \
  --forced-transport unrestricted \
  --receipt docs/ops/receipts/livekit/2026-XX-XX-rc29-transport-udp.json --apply

# Cell 2 — block all UDP at the firewall first, then run.
pnpm --dir apps/sarah-livekit-agent acceptance -- \
  --forced-transport udp_blocked \
  --receipt docs/ops/receipts/livekit/2026-XX-XX-rc29-transport-tcp.json --apply

# Cell 3 — block UDP and plaintext TCP, leaving only the TLS relay.
pnpm --dir apps/sarah-livekit-agent acceptance -- \
  --forced-transport udp_and_plaintext_tcp_blocked \
  --receipt docs/ops/receipts/livekit/2026-XX-XX-rc29-transport-turn.json --apply
```

The declaration is checked against the observation. If you declare `udp_blocked`
and the capture rode direct UDP, the block did not take effect and the run
**fails closed** rather than being recorded as a passing cell. That is the
intended behaviour: a mis-imposed constraint is a run to repeat, not evidence.

Then record the three classified captures into the row, citing each acceptance
receipt's `resultDigest` as the `acceptanceResultDigest`. The expected
classifications are `direct_udp`, `tcp_fallback`, and `turn_tls` respectively; a
capture filed under the wrong cell is refused.

Note the one classification subtlety: a TURN relay negotiated over TLS still
reports `tcp` at the candidate-pair level, so the classifier reads the relay
protocol for relay candidates. `relayed_udp` is a distinct observed kind and
satisfies none of the three cells on its own.

### Journey C — the failure drills

**Row:** `sarah-livekit-failure`. Ten required observations: eight bounded
drills, the eight-scope privacy scan, and a media-key rekey proof.

Each executed drill records the fault injected, the millisecond bound the system
recovered within, and the settlement state and receipt digest — settlement must
stay deterministic under fault, which is the actual claim being tested.

**As of 2026-07-31, none of the ten can be run.** The blockers were verified
against the live production deployment, and they are not all the same kind of
blocker, so they are listed separately rather than as one wall.

**Every drill is blocked before its own first step.** A drill needs a live
Sarah session, and a live acceptance run needs the two 24 kHz mono s16 PCM
prompts. `--private-pcm` and `--community-pcm` are mandatory under `--apply`,
there is no synthesize-silence path, and the prompts are deliberately not in
Git. No prompt files exist on the current operator machine. Producing them
outside the repository is the single highest-leverage unblock: it is what
stands between the drills and any of the per-drill blockers below actually
mattering.

Then, per drill:

- **`sfu_loss_bounded`** — the scenario **is now defined** (openagents
  `af65458919`): fault `delete_exact_sfu_pod`, a 30000 ms bound measured from
  fault injection, admitted terminal reasons `worker_shutdown`,
  `participant_left`, and `worker_error`, with `session_expired` and `completed`
  refused. The executable procedure is in the openagents LiveKit runbook under
  **The SFU-loss drill**. It is still `not_observed`: an unexecuted defined
  scenario is worth exactly as much as an undefined one for this row.
- **`worker_crash_bounded`** and **`sfu_loss_bounded`** both need to delete a
  pod. `kubectl auth can-i delete pods -n livekit-system` is **no** for the
  automation identity, which is read-only on the cluster. Granting it is an
  authority decision, not a configuration detail.
- **`openai_disconnect_bounded`** — its production route is disabled. See the
  owner decision below.
- **`duplicate_participant_refused`** — **no production code path exists.**
  `recordSarahLiveKitParticipantJoin` is exported but called from nothing except
  its own test, and the live join route is an upsert that does not refuse a
  re-join. This one is not an access problem and no amount of credential will
  satisfy it; it is a gap between the gate vocabulary and the implementation.
- **`membership_revocation_bounded`** — needs a NIP-29 admin key to publish the
  revoking event, plus a second authenticated identity.
- **`replayed_grant_refused`** — needs no cluster mutation and no owner window,
  only a live community session with an active presence lease. It is the
  closest to reachable once prompts exist.
- **`privacy_scope_count`** — four independent blockers, enumerated per scope in
  openagents `docs/ops/2026-07-31-sarah-livekit-privacy-scan-executability.md`.
  Three are outside what any repository change can fix, and because the
  collector requires all eight scopes inside one two-hour window, clearing one
  blocker at a time produces nothing.
- **`media_key_rekey_proof`** — no proof exists. A checked-in static E2EE key
  revision is configuration, not a rekey.

The privacy scan requires at least eight distinct scopes, no residue, and a
same-window complete export. Seven scopes, or a scan that found residue, is
refused for that key rather than recorded as a partial pass.

### Journey D — the private journey and isolation

**Rows:** `sarah-livekit-private` and `sarah-livekit-isolation`. Both are run on
the packaged candidate, not the headless harness.

For `sarah-livekit-private`, the ordering of admission terms is the point:
`admission_terms_seen` requires the price, hold, rate, and limit to have been
displayed **before** capture began, and the recorder refuses terms whose
`displayedAtMs` is not strictly earlier than `firstCaptureAtMs`. Also required:
one confirmed bounded command, one started agent thread, and a media reconnect
whose provider generation is unchanged.

The reconnect comparison uses two exact packaged-client transport observations:
one `stage: connected` and one `stage: reconnected`. They must have identical
`sessionRefDigest`, `sessionGeneration`, `roomRefDigest`, `roomEpoch`,
`dispatchRefDigest`, and `providerGenerationRefDigest`. The gate rejects a
boolean-only `reconnect_same_generation` claim and rejects any changed binding.
OpenAgents supplies the raw opaque `providerGenerationRef` on the authenticated
`session_ready` frame; Omega hashes it before persistence. `dispatchRef`, room
epoch, and the Omega session generation are not substitutes for that server
field.

For `sarah-livekit-isolation`, the concurrent private and community generations
must be shown to differ. The acceptance receipt now preserves a per-receipt
salted comparable form of the identity digests, so the comparison survives into
the public receipt and an independent reviewer can re-verify it; the salt is
published nowhere, so the values leak nothing outside their own receipt. The
previously hard-coded `identityIsolationObserved` is now computed from those
digests, and identical cross-room generations fail the run.

The two capability refusals must be **live**: community Sarah asked for a known
private editor fact and asked for a privileged tool, and refusing both. The
tool-free capability profile is an architectural argument and is recorded, if at
all, with `sourceKind: "operator_attestation"` — never as a live observation.

### Owner decision required: arming the provider-disconnect drill

`openai_disconnect_bounded` cannot be run until an owner decides to arm it. The
facts, verified against the currently-serving revision:

- `SARAH_LIVEKIT_PROVIDER_DISCONNECT_ACCEPTANCE_ENABLED` is `"false"` in the
  serving revision, and **has been `"false"` in every revision that has ever
  carried the key**. The drill has never been armed in production.
- While false, the route returns `404` before authentication. That is
  deliberate, not a routing accident.
- The committed value is contract-locked to `"false"` by a deploy-bundle test.
  **Flipping the checked-in value is the wrong fix** and breaks the gate.
- Arming requires **no repository change**. It is one deploy carrying a
  per-deploy override, and the next ordinary deploy withdraws it automatically:

  ```sh
  SARAH_LIVEKIT_PROVIDER_DISCONNECT_ACCEPTANCE=on scripts/deploy-cloudrun.sh production
  ```

The decision the owner must make is **not** whether the code is right — it is
whether to open a bounded acceptance window. The runbook conditions arming on "a
separately approved, bounded acceptance window", and the drill itself closes a
live provider socket on a real connected session, so it is customer-affecting by
construction. Deploying also ships whatever else has landed on `main` since the
serving revision.

Arming is not evidence that the drill passed. Until the owner opens that window,
`openai_disconnect_bounded` is correctly recorded as `not_observed`.

## The `distribution` row is now measured, and it still needs you

`script/collect-omega-distribution-observations` performs the machine-checkable
half of this row against the live public download path. Run it for whatever
candidate is current:

```sh
script/collect-omega-distribution-observations \
  --expected-package-sha256 <the digest this receipt binds> \
  --expected-version 0.2.0-rcNN \
  --expected-team-id HQWSG26L43 \
  --out docs/omega/release-gate/<date>-<candidate>.distribution.json
```

It downloads the artifact from the public link and hashes what is actually
served, rather than reading the digest the page prints beside it. A page can
advertise the correct digest while serving different bytes, and the row exists
to catch exactly that. It then mounts the downloaded image and reads the
signature, the Gatekeeper assessment, the stapled notarization ticket, and the
bundle's own version from the artifact a user would receive — not from the
local build tree, which is a different file that happens to share a name.

For rc29 all eight agreements measured true, recorded in
[`2026-07-31-omega-v0.2.0-rc29.distribution.json`](release-gate/2026-07-31-omega-v0.2.0-rc29.distribution.json):
the served bytes, the page's published digest, and the release notes all name
`116ae5ae…`; the served app is Developer ID signed under team `HQWSG26L43`,
accepted by Gatekeeper as Notarized Developer ID, and carries a stapled ticket;
and the page declines to offer Intel, Windows, and Linux builds rather than
implying they exist.

**None of that promotes the row, and the collector says so in its own output.**
Two things remain, and neither is something a program can supply:

1. The row refuses to pass unless the episode transcript, website, release
   notes, installed binary, and announcement copy agree **claim for claim**.
   Comparing prose claims is a judgment.
2. `owner-assisted-pass` records a *preserved human observation*. The
   owner-evidence schema has no other status, so a program that submitted this
   row would be recording an observation nobody made. That is the one thing
   this report exists to prevent.

So the row stays `owner-assisted-pending`. What changed is that the human part
is now only the human part: the digest, signature, notarization, platform
honesty, and version agreement are already measured, digest-bound, and
re-runnable against the next candidate.

## `update-safety-lifecycle` was not attempted, and why

This row needs Omega dragged in beside the existing Zed and OpenAgents Desktop
installs, launched, then upgraded, rolled back, and uninstalled, with only
Omega paths changing. The filesystem half is automatable. The launch is not:
starting a GPUI application puts a window on whoever's display is attached, and
the previous session was stopped precisely because agents drove the owner's
desktop while he was working. It needs a separate display or host, or the
owner's own hands.

It was also the wrong candidate to spend a journey on. rc29 is superseded by
design — the plan is to cut rc30, notarize it, and bind every gate row to that
digest once rather than run the same journeys against two candidates. Observing
this row against rc29 would produce evidence that the next candidate discards.
