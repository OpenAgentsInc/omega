# Omega Full Auto routing and liveness (FA-04)

- Date: 2026-07-24
- Packet: `OMEGA-FA-04`
- Issue: [#23](https://github.com/OpenAgentsInc/omega/issues/23)
- Protocol pin: OpenAgents pack SHA-256
  `2dec2474e2cb64acb88291beb3d5efdeef4cbd8004dfe26c0492d1f3757174a9`

## Result

Omega's Full Auto panel and `omega_effectd` supervisor now consume FA-04
protocol methods:

- `get_capacity` — per-lane capacity ledger and non-overridable guardrail list

## Provider account roster

The Full Auto panel treats provider accounts and capacity lanes as distinct
records. When `get_capacity` includes bounded public-safe `accounts`, Omega
shows each account's provider, device-local label, readiness, quota state, and
lane mapping. An absent account list renders as unavailable rather than
inventing one account per lane. Provider connection and reauthentication stay
in Omega's native Agent authentication flow; the roster never reads, copies, or
prints credentials from a Codex home.
- `decide_attention` — redacted stall/retry attention decision
- `get_run` stall cause + recovery action rendered in the run monitor

A missing host thread settles to a typed stall with `stop_only`. Capacity
lanes show on the launcher and run monitor. GPUI stays presentation-only.

The panel checks attention only for typed `retrying` and `stalled` snapshots.
It passes the previous service dedup key back to `decide_attention`, records
denied decisions so a disabled notification preference cannot create a poll
storm, and suppresses overlapping checks. An allowed decision produces an
Omega workspace notification and requests window attention. Visible copy is
selected from an exact state allowlist and is built locally from fixed strings;
the service-provided run title/body, objective, paths, prompts, transcript,
credentials, dedup key, and raw errors never enter the notification.

## Verification

- `cargo test -p full_auto_ui -p omega_effectd --lib`
- `./script/clippy -p full_auto_ui -p omega_effectd`
- `cargo check -p zed -p agent_ui -p full_auto_ui`
