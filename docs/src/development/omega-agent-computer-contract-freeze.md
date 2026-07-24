# Omega Agent Computer contract freeze

Omega Agent Computer work follows the freeze recorded in the OpenAgents
monorepo. This page is the Omega consumer pointer. It does not redefine
control-plane ownership.

## Authority

- Freeze:
  [`docs/omega/2026-07-24-omega-agent-computer-contract-freeze.md`](https://github.com/OpenAgentsInc/openagents/blob/main/docs/omega/2026-07-24-omega-agent-computer-contract-freeze.md)
- Plan:
  [`docs/omega/2026-07-24-agent-computer-omega-completion-plan.md`](https://github.com/OpenAgentsInc/openagents/blob/main/docs/omega/2026-07-24-agent-computer-omega-completion-plan.md)
- Roadmap home: `OMEGA-OA-08` portable execution
- Issue: [#27](https://github.com/OpenAgentsInc/omega/issues/27) (`OMEGA-AC-00`)

## Laws Omega must keep

- Agent Computer is `HarnessEnvironment.openagents_cloud`.
- Omega never owns Firecracker, GCE, or placement APIs for this path.
- `omega-effectd` is the only Omega mutation path to cloud coding sessions.
- Rust supervises process and protocol health only.
- GPUI is projection and command entry only. It is not receipt authority.
- Live-capacity probes and runtime-only credentials are mandatory.
- Full Auto may add a cloud lane only after `OMEGA-AC-03` and a Full Auto
  freeze revision.

## Falsifier

A GPUI view or Rust crate becomes Agent Computer receipt authority.

## Next packets

1. `OMEGA-AC-01` — `omega-effectd` Agent Computer runner (#28)
2. `OMEGA-AC-02` — minimal launch surface (#29)
3. `OMEGA-AC-03` — live Omega proof (#30)
