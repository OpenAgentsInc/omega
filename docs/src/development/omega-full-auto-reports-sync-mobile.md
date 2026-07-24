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

## Verification

- `cargo test -p full_auto_ui -p omega_effectd --lib`
- `cargo check -p zed -p agent_ui -p full_auto_ui`
