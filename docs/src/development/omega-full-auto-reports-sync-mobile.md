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
Connect, Reconnect, and Disconnect actions. Connect uses the system browser,
the public `openagents-desktop` client, GitHub authorization code, PKCE S256,
and only an RFC 8252 loopback redirect at
`http://127.0.0.1:{ephemeral-port}/auth/callback`.

Access and refresh tokens remain in Omega's release-namespaced sovereign
system-keychain provider. They are never written to settings, the Full Auto
registry, host correlation journal, logs, UI state, transcripts, or child
process environment. Every `resolve_sync_session` reverse-host request
re-verifies the credential through the existing OpenAgents native auth-session
API and writes any server rotation back to Keychain before returning the one
runtime-only `{ baseUrl, accessToken }` response to omega-effectd. Missing,
denied, malformed, or transiently unverifiable credentials resolve as
`{ available: false }`.

Disconnect requires server proof that both access and refresh credentials were
revoked before Omega deletes the local Keychain record. An incomplete or
unavailable revoke retains local custody and presents a typed retryable state.

## Verification

- `cargo test -p full_auto_ui -p omega_effectd --lib`
- `cargo check -p zed -p agent_ui -p full_auto_ui`
- `./script/clippy -p omega_effectd -p full_auto_ui -p agent_ui`
