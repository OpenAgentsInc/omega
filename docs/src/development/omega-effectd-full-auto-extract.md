# Omega-effectd Full Auto extract pin

Omega pins the released Full Auto engine artifact from OpenAgents. This page
is the consumer lock. It does not redefine the run lifecycle.

## Authority

- Extract receipt:
  [`docs/omega/2026-07-24-omega-effectd-extract.md`](https://github.com/OpenAgentsInc/openagents/blob/main/docs/omega/2026-07-24-omega-effectd-extract.md)
- Freeze:
  [`docs/omega/2026-07-24-full-auto-contract-freeze.md`](https://github.com/OpenAgentsInc/openagents/blob/main/docs/omega/2026-07-24-full-auto-contract-freeze.md)
- Package README:
  [`packages/omega-effectd/README.md`](https://github.com/OpenAgentsInc/openagents/blob/main/packages/omega-effectd/README.md)
- Issue: [#20](https://github.com/OpenAgentsInc/omega/issues/20) (`OMEGA-FA-01`)
- OpenAgents land: `f795e357c5e797b1cb74d37e12ea5e9b7c45fd9b`

## Pin

| Field | Value |
| --- | --- |
| Package | `@openagentsinc/omega-effectd` |
| Version | `0.1.0` |
| Pack SHA-256 | `d4d8ea7035c37d9d03ef6b90ce129a71118f55280d6af08a2a4cee5d3aa5d93b` |

## Consumption rules

- Pin by version and pack digest.
- Do not import through a relative monorepo path.
- Do not use an unpublished `workspace:*` edge to run Full Auto.
- Inject the Omega data root with `OPENAGENTS_OMEGA_EFFECTD_DATA_ROOT`.
- Durable files stay under `{dataRoot}/full-auto/`.
- `full-auto-run-actions` remains the only mutation API.

## Falsifier

Omega needs a relative monorepo path or an unpublished workspace edge to build
or run Full Auto.

## Next packets

1. [#23](https://github.com/OpenAgentsInc/omega/issues/23) routing and
   liveness (FA-03: [omega-full-auto-gpui-launcher.md](./omega-full-auto-gpui-launcher.md))
2. [#24](https://github.com/OpenAgentsInc/omega/issues/24) reports, Sync,
   mobile
3. [#25](https://github.com/OpenAgentsInc/omega/issues/25) native project
   join
4. [#26](https://github.com/OpenAgentsInc/omega/issues/26) proof
