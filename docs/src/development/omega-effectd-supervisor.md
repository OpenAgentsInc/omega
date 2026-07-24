# Omega-effectd Rust supervisor

Omega supervises packaged `omega-effectd` through crate `omega_effectd`.
Durable Full Auto run truth stays on disk under the injected data root.
GPUI is not run authority.

## Authority

- Supervisor receipt:
  [`docs/omega/2026-07-24-omega-effectd-supervisor.md`](https://github.com/OpenAgentsInc/openagents/blob/main/docs/omega/2026-07-24-omega-effectd-supervisor.md)
- Extract pin: [omega-effectd-full-auto-extract.md](./omega-effectd-full-auto-extract.md)
- Issue: [#21](https://github.com/OpenAgentsInc/omega/issues/21) (`OMEGA-FA-02`)
- OpenAgents land: `cf5a2085b1b422bc6a41483ea0e43820a331a5a0`

## Pin

| Field | Value |
| --- | --- |
| Package | `@openagentsinc/omega-effectd` |
| Version | `0.1.0` |
| Pack SHA-256 | `4cc1cb2e5d71ff8af6f730248871ee779488a991f21a848880e885331ef31831` |
| Protocol | `openagents.omega.effectd.v1` |
| Omega crate | `crates/omega_effectd` |

## Supervisor laws

- Rust owns process life, health, restart, and generation fencing.
- Node owns registries, leases, reconcile, and run-actions mutation.
- One application-scoped supervisor is shared by Full Auto and Agent Computer.
- Production resolves only the component installed at
  `Contents/Resources/omega-effectd/bin/omega-effectd`. An explicit
  `OPENAGENTS_OMEGA_EFFECTD_BIN` override is available for controlled
  development runs.
- A missing component fails closed. Production never searches a sibling
  OpenAgents checkout and never substitutes the test fixture.
- Request, response, and reverse-host frames are limited to 64 KiB. A
  malformed, oversized, timed-out, or prematurely closed response tears down
  the child before a later request may restart it.
- While awaiting a service response, the supervisor multiplexes typed
  `host_request` frames and emits ID- and generation-matched `host_response`
  frames. Unknown methods, stale generations, missing authorities, authority
  timeouts, and oversized host results fail closed.
- Reverse host handlers have a 30-second deadline. The outer request budget is
  180 seconds because one Full Auto operation can chain several reverse calls.
- Restart re-reads `{dataRoot}/full-auto/` from disk.
- Diagnostics are redacted. Objective and transcript stay out of list
  projections.
- Do not let a GPUI entity rewrite a durable run after restart.

## Verification

- `cargo test -p omega_effectd`
- `./script/check-licenses`

The Omega RC packager still needs to install the pinned service and fixed
Node runtime at the component path above. Until then the panels report the
component as unavailable rather than presenting fixture behavior.

Omega registers the reverse-host handler when it creates the shared supervisor.
The handler stays on GPUI's foreground executor and delegates to the active
`Workspace`, `AgentPanel`, and `AcpThread` authorities. It resolves exactly one
open local workspace, creates a native Agent thread, admits the `codex-local`
lane only when a default model is available, dispatches through
`AcpThread::send`, projects bounded assistant evidence from real thread entries,
and interrupts through `AcpThread::cancel`. Unknown lanes, ambiguous
workspaces, closed threads, concurrent turns, and unavailable profile overrides
fail closed.

The host bridge retains only correlation data between effectd's leased turn
references and live ACP entry boundaries; it is not a second run engine and
does not mutate durable Full Auto state. That correlation is currently scoped
to the app process, so owner-real restart recovery still needs a persisted
effectd-turn reference on the native Agent thread. `append_system_note` also
remains typed `unavailable`: `AcpThread` has no owner-visible, non-model system
entry authority, and Omega does not disguise control notes as user prompts.

## Next packets

1. [#22](https://github.com/OpenAgentsInc/omega/issues/22) GPUI launcher
2. [#23](https://github.com/OpenAgentsInc/omega/issues/23) routing and
   liveness
3. [#24](https://github.com/OpenAgentsInc/omega/issues/24) reports, Sync,
   mobile
4. [#25](https://github.com/OpenAgentsInc/omega/issues/25) native project
   join
5. [#26](https://github.com/OpenAgentsInc/omega/issues/26) proof
