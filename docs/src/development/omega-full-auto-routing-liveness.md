# Omega Full Auto routing and liveness (FA-04)

- Date: 2026-07-24
- Packet: `OMEGA-FA-04`
- Issue: [#23](https://github.com/OpenAgentsInc/omega/issues/23)
- Protocol pin: OpenAgents pack SHA-256
  `d4d8ea7035c37d9d03ef6b90ce129a71118f55280d6af08a2a4cee5d3aa5d93b`

## Result

Omega's Full Auto panel and `omega_effectd` supervisor now consume FA-04
protocol methods:

- `get_capacity` — per-lane capacity ledger and non-overridable guardrail list
- `decide_attention` — redacted stall/retry attention decision
- `get_run` stall cause + recovery action rendered in the run monitor

A missing host thread settles to a typed stall with `stop_only`. Capacity
lanes show on the launcher and run monitor. GPUI stays presentation-only.

## Verification

- `cargo test -p full_auto_ui -p omega_effectd --lib`
- `cargo check -p zed -p agent_ui -p full_auto_ui`
