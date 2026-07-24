# HE-02 harness-environment release (Omega pin)

- Packet: `HE-02`
- OpenAgents issue: https://github.com/OpenAgentsInc/openagents/issues/9210
- OpenAgents receipt:
  [`docs/omega/2026-07-24-he02-harness-environment-release.md`](https://github.com/OpenAgentsInc/openagents/blob/main/docs/omega/2026-07-24-he02-harness-environment-release.md)

Omega consumes `@openagentsinc/agent-harness-environment@0.1.0-rc.1` from
npm. Tarball SHA-256:
`9ed2d1c2439dfd33f736b2d3f63795144f7ffb9ad0ce8965f49cc78cd44334fd`.

Omega must not depend on a relative openagents monorepo path for this package.
`omega-effectd` pins the released version.
