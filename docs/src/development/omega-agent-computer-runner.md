# Omega Agent Computer runner (AC-01)

- Packet: `OMEGA-AC-01`
- Omega issue: https://github.com/OpenAgentsInc/omega/issues/28
- OpenAgents receipt:
  [`docs/omega/2026-07-24-omega-agent-computer-runner.md`](https://github.com/OpenAgentsInc/openagents/blob/main/docs/omega/2026-07-24-omega-agent-computer-runner.md)

Omega supervises Agent Computer sessions only through framed
`omega-effectd` methods. Rust does not call placement or GCE APIs for this
path. Durable session rows keep public-safe fields only.
