# NEEDS_OWNER

Owner-only steps that agents deliberately leave incomplete. Public-safe; no secrets.

## Relay: create `omega-alpha-feedback` NIP-29 group (rc28 review batch 3 / item 12)

**Status:** client hardcodes the destination; live group creation was not performed from this worktree (no interactive admin credentials; Keychain probes skipped by instruction).

**What shipped client-side**

- Bundled registry `crates/agent_ui/fixtures/tester-channel-registry.v1.json` (`omega.alpha-feedback.2`) lists:
  - **Alpha feedback · #alpha-feedback** → group `omega-alpha-feedback`
  - **Agent Chat · #agent-chat** → group `openagents-public`
- Sidebar shows both. Remote Agent Chat manifest refresh validates only the `openagents-public` operational fields.

**Owner one-step**

1. Using the owned relay admin key (see openagents repo `docs/ops/2026-07-24-owned-nostr-relay-deploy.md`), create the NIP-29 group `omega-alpha-feedback` on `wss://relay.openagents.com` with the same kind/limit policy as `openagents-public` (kinds 5/7/9/1337/1984, group state 39000/39001/39003/39005, moderation 9002/9005/9010), or confirm unmanaged groups auto-create on first signed kind-9 with `h=omega-alpha-feedback`.
2. Confirm a throwaway identity can post to `omega-alpha-feedback` and a second account can read it (feeds omega#156 / channels-send-receive).

Until step 1–2 succeed, first-launch **send** to Alpha feedback may authenticate via NIP-42 and then be refused by the relay if the group does not exist or is admission-gated.

## Other open owner steps (unchanged)

- Reduce Motion toggle for the strict reduced-motion cell
- Throwaway hosted identity / scratch `GEMINI_API_KEY` for paid-path cells
