# Omega Full Auto GPUI launcher (FA-03)

- Date: 2026-07-24
- Packet: `OMEGA-FA-03`
- Omega issue: [#22](https://github.com/OpenAgentsInc/omega/issues/22)
- Depends on: FA-00 freeze, FA-01 extract, FA-02 supervisor
- Protocol pin: OpenAgents pack SHA-256
  `d4d8ea7035c37d9d03ef6b90ce129a71118f55280d6af08a2a4cee5d3aa5d93b`
- OpenAgents land: `f795e357c5`

## Result

Omega ships a dedicated Full Auto dock panel.

- Entry: Agent panel **New** menu → **Full Auto**, plus
  `full_auto_panel::ToggleFocus` / `OpenLauncher`
- Launcher: one objective, collapsed Advanced (title, done condition, turn
  cap), Start / Cancel
- Monitor: up to eight active runs with typed states
- Run view: pause / resume / stop / retry, mission text, turn list
- No ordinary composer on this surface
- Mutations call supervised `omega_effectd` only

Crate: `crates/full_auto_ui`. Supervisor helpers: `start_run`, `get_run`,
`pause_run`, `resume_run`, `stop_run`, `retry_run`.

## Verification

- `cargo test -p full_auto_ui -p omega_effectd --lib`
- `cargo check -p zed -p agent_ui`

## Falsifier

A composer toggle or ordinary chat path starts Full Auto.

## Next

1. [#23](https://github.com/OpenAgentsInc/omega/issues/23) routing and
   liveness (FA-04)
2. [#24](https://github.com/OpenAgentsInc/omega/issues/24) reports, Sync,
   mobile (FA-05)
