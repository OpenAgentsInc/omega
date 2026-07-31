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
- omega#171 (accessibility tree): CLOSED not-planned by owner; revisit before beta launch.

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
