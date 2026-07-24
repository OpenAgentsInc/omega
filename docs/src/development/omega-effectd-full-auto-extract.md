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
- OpenAgents land: `b330615d62a328a348a890d830102013f85b5010`

## Pin

| Field | Value |
| --- | --- |
| Package | `@openagentsinc/omega-effectd` |
| Version | `0.1.0` |
| Pack SHA-256 | `0717aa2fef95734773257ec702848e3d226631b7e298cc128cfc12e8de0d0847` |

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

1. [#21](https://github.com/OpenAgentsInc/omega/issues/21) Rust supervisor
2. [#22](https://github.com/OpenAgentsInc/omega/issues/22) GPUI launcher
3. [#23](https://github.com/OpenAgentsInc/omega/issues/23) routing and
   liveness
4. [#24](https://github.com/OpenAgentsInc/omega/issues/24) reports, Sync,
   mobile
5. [#25](https://github.com/OpenAgentsInc/omega/issues/25) native project
   join
6. [#26](https://github.com/OpenAgentsInc/omega/issues/26) proof
