# NEEDS_OWNER

Owner-only steps that agents deliberately leave incomplete. Public-safe; no secrets.

## Relay: create `omega-alpha-feedback` NIP-29 group (rc28 review batch 3 / item 12)

**Status:** client hardcodes the destination; live group creation was not performed from this worktree (no interactive admin credentials or live runtime-credential probes).

**What shipped client-side**

- Bundled registry `crates/agent_ui/fixtures/tester-channel-registry.v1.json` (`omega.alpha-feedback.2`) lists:
  - **Alpha feedback · #alpha-feedback** → group `omega-alpha-feedback`
  - **Agent Chat · #agent-chat** → group `openagents-public`
- Sidebar shows both. Remote Agent Chat manifest refresh validates only the `openagents-public` operational fields.

**Owner one-step**

1. Using the owned relay admin key (see openagents repo `docs/ops/2026-07-24-owned-nostr-relay-deploy.md`), create the NIP-29 group `omega-alpha-feedback` on `wss://relay.openagents.com` with the same kind/limit policy as `openagents-public` (kinds 5/7/9/1337/1984, group state 39000/39001/39003/39005, moderation 9002/9005/9010), or confirm unmanaged groups auto-create on first signed kind-9 with `h=omega-alpha-feedback`.
2. Confirm a throwaway identity can post to `omega-alpha-feedback` and a second account can read it (feeds omega#156 / channels-send-receive).

Until step 1–2 succeed, first-launch **send** to Alpha feedback may authenticate via NIP-42 and then be refused by the relay if the group does not exist or is admission-gated.

## Sarah LiveKit: the three-desktop room journey (omega#186 / #187) — NO owner step remains

**Both admissions this entry used to ask for are done, and neither needed the owner.**
All three journey identities hold an active `sarah_voice_cohort:alpha_v1` row, and all
three are admitted to NIP-29 group `openagents-public`. Membership resolution is proven:
`POST /api/sarah/livekit/room/join` answers `community_room_not_active` (404) for each
identity rather than `community_membership_required` (403).

The community admin key was never missing. `openagents-nostr-relay-private-key` in
Secret Manager derives to `e841147f…2046ed`, which is exactly the pubkey the deployed
`SARAH_LIVEKIT_COMMUNITY_AUTHORITY_JSON` already lists as group admin — the same value
committed beside it as `PUBLIC_NOSTR_CHAT_RELAY_SELF_PUBKEY`. Kind-9000 admission works
with it once the connection completes NIP-42 AUTH; without AUTH the relay answers
`auth-required`, which reads like a missing permission and is what an earlier lane
concluded from. A second admin key we hold outright is now stored as
`oa-livekit-community-admin-nostr-key`, granted relay-side `put-user`, and added
**beside** the existing pubkey (nothing replaced) in Cloud Run revision
`openagents-monolith-00374-h4p`. Admission is repeatable from a reviewed tool:
`apps/openagents.com/workers/api/scripts/admit-sarah-livekit-community-npub.ts`.

**What still blocks the row is not a credential.** rc29 exposes the five controls this
journey needs — Join room, Summon Sarah, Talk to Sarah, Stop — only as GPUI `on_click`
closures in `crates/agent_ui/src/omega_public_channel_view.rs`. They are not registered
GPUI actions, no keymap reaches them, the `omega` CLI flags target the agent-thread
composer, the `zed://` URL scheme has no route for them, `omega-effectd` is a private
stdio pipe between the app and its own child process, and `omega-device-bridge` is a
read-only mirror that refuses every command frame by design. So the journey currently
needs **a human clicking through three packaged instances**, and that is the only thing
an owner could still supply.

The closest unattended path found is macOS Accessibility automation after opting each
instance into `ZED_EXPERIMENTAL_A11Y=1`, which is undocumented, off by default, and named
experimental. A run that needs it must disclose it as an environment deviation. The
durable fix is to give these controls a real dispatch surface.

Two things that no longer need re-deriving: offline identity pre-seed is proven on rc29
(write the raw 32-byte `identity.secret` plus a consistent `identity.json` /
`identity.complete.json` under `<root>/Library/Application Support/Omega RC/identity/`),
so the three identities need not be whatever the app generates; and `member_removed` /
`membership_changed` now have their first production emitter in openagents `de475872af`,
which the serving image predates — a normal source deploy, not an owner action.

Current receipt: `docs/ops/receipts/livekit/gate/2026-07-31-rc29-room-admission-cleared.json`
in openagents, digest `sha256:006af7ee…42e5bc`; all nine observations remain
`not_observed`.

## Other open owner steps (unchanged)

- Reduce Motion toggle for the strict reduced-motion cell
- Throwaway hosted identity / scratch `GEMINI_API_KEY` for paid-path cells
