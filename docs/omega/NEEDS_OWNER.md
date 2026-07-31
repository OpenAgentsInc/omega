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

## Sarah LiveKit: admit three identities so the three-desktop room journey can run (omega#186 / #187)

**Status:** the client side is proven; admission is the only blocker. Three packaged
`v0.2.0-rc29` desktops (package `116ae5ae…c34`) ran concurrently on one Mac, each with
an isolated profile and its own self-provisioned Nostr identity. None could join a
community room, so all nine `sarah-livekit-room` observations are `not_observed`
(receipt `docs/ops/receipts/livekit/gate/2026-07-31-rc29-room.json` in openagents,
digest `sha256:343e7cd9…a8cf`).

Three Macs and three people are **not** required — one host runs three real packaged
instances. What is required is two owner-only admissions, per identity.

**Owner steps (repeat for each of the three identities)**

1. **Alpha cohort.** Run the owner-gated script in openagents against production
   Postgres to write the `sarah_voice_cohort:alpha_v1` row:
   `apps/openagents.com/workers/api/scripts/admit-sarah-voice-npub.ts`
   (gate `I_APPROVE_BOUNDED_SARAH_VOICE_CREDIT`). Without this row every Sarah voice
   session is rejected before a room exists. There is no HTTP surface for it.
2. **NIP-29 group admission.** Using the community admin key, publish a kind-9000
   put-user for each identity into the configured community/channel
   (`openagents-public`, channel `agent-chat` or `alpha-feedback`) on
   `wss://relay.openagents.com`. Make **one** of the three a group admin so the
   moderator-stop observation is reachable.

The three npubs to admit are printed by each instance at
`<profile>/identity/identity.json`; regenerate them at run time rather than pinning
throwaway keys here.

**Not an owner step — an implementation gap.** Two of the four required refusals
cannot return their expected codes today. `removeSarahLiveKitRoomMember` is the only
producer of `member_removed` and `membership_changed` and has no non-test caller;
production `resolveMember` returns undefined for a removed member, which the routes
map to `403 room_membership_required`. Either give those codes a production emitter or
amend the row's expected codes. No credential fixes this.

## Other open owner steps (unchanged)

- Reduce Motion toggle for the strict reduced-motion cell
- Throwaway hosted identity / scratch `GEMINI_API_KEY` for paid-path cells
