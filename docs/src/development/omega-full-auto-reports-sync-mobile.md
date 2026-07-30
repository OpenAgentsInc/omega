# Omega Full Auto reports, Sync, and mobile control (FA-05)

- Date: 2026-07-24
- Packet: `OMEGA-FA-05`
- Issue: [#24](https://github.com/OpenAgentsInc/omega/issues/24)
- Protocol pin: OpenAgents pack SHA-256
  `2dec2474e2cb64acb88291beb3d5efdeef4cbd8004dfe26c0492d1f3757174a9`

## Result

Omega supervisor and fixture speak FA-05 methods:

- `get_report` / `get_receipt`
- `apply_control_intent` (mobile actor outcomes)
- `get_sync_status` / `publish_projection` (honest Sync stub)

The Full Auto panel shows the public receipt objective digest beside the
owner-local objective. Durable mutation stays in supervised omega-effectd.

## Native OpenAgents session

Omega's Full Auto panel exposes an **OpenAgents Sync** account row with
Connect, Reconnect, and Disconnect actions. Connect stays inside Omega. It
signs a fresh NIP-98 HTTP-auth event with the built-in Omega Nostr identity and
exchanges that proof directly with the OpenAgents session endpoint. The proof
is bound to the exact HTTPS URL, POST method, empty request payload, configured
owner public key, and a 60-second freshness window. It is single-use.

The server returns a short-lived opaque access token for the already configured
OpenAgents owner account. That token remains in Omega's release-namespaced
native credentials file. It is never written to settings, the Full
Auto registry, host correlation journal, logs, UI state, transcripts, or child
process environment. Every `resolve_sync_session` reverse-host request
re-verifies the credential through the existing OpenAgents native auth-session
API before returning the one runtime-only `{ baseUrl, accessToken }` response
to omega-effectd. Missing, denied, malformed, expired, or transiently
unverifiable credentials resolve as `{ available: false }`.

Legacy OAuth credentials remain readable during migration and can still rotate
through their refresh token. New background sessions carry no refresh token.
Disconnect requires server proof that every credential present in the stored
session was revoked before Omega deletes the local credentials-file record.

NIP-42 remains the relay-authentication protocol for Nostr WebSocket relays. It
does not mint an OpenAgents HTTP session; NIP-98 provides that separate,
request-bound proof.

## Verification

- `cargo test -p full_auto_ui -p omega_effectd --lib`
- `cargo check -p zed -p agent_ui -p full_auto_ui`
- `./script/clippy -p omega_effectd -p full_auto_ui -p agent_ui`
