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
| Runtime release | `omega-effectd-v0.1.0-rc.2` |
| macOS arm64 archive SHA-256 | `f9b4bb93216d4a09dea97a3a94ab1679e1a8d9c13c68a93affd5818d08554cd8` |
| Runtime source commit | `c164bc25c42dab369d645d77c99f86b083323540` |
| Runtime source tree | `26378837c60dbcd4253918cb1a07ce8c36f0c5a8` |

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
- Manual provider handoff is legal only while the run is paused. The service
  rechecks target-lane readiness through the native host, durably rebinds the
  run and dispatch profiles, and records the transition before Resume can
  dispatch on the new lane.
- Diagnostics are redacted. Objective and transcript stay out of list
  projections.
- Do not let a GPUI entity rewrite a durable run after restart.

## Verification

- `cargo test -p omega_effectd`
- `./script/check-licenses`

The Omega RC packager downloads the immutable runtime release, verifies its
archive and component manifest, installs the fixed Node runtime at the
component path above, signs the nested Node executable before the application,
and records the source and packaged component digests in the release record.
The nested runtime receives only the two V8 execution entitlements required by
hardened Node (`allow-jit` and `allow-unsigned-executable-memory`); it does not
inherit Omega's device, personal-data, automation, or filesystem entitlements.
Installed proof launches both the mounted and installed copies through framed
`initialize` and `health` requests. The packaged runtime also advertises and
passes the framed `handoff` contract.

Omega registers the reverse-host handler when it creates the shared supervisor.
The handler stays on GPUI's foreground executor and delegates to the active
`Workspace`, `AgentPanel`, and `AcpThread` authorities. It resolves exactly one
open local workspace and creates either a native Agent thread for
`codex-local` or a registered `claude-acp` external-agent thread for
`claude-local`. Codex admission requires a default model. Claude admission
requires the registered external agent and, once a Claude thread exists, a
connected ACP authority. Both lanes dispatch through `AcpThread::send`, project
bounded assistant evidence from real thread entries, and interrupt through
`AcpThread::cancel`. Unknown lanes, ambiguous workspaces, closed threads,
concurrent turns, cross-lane dispatch, and unavailable profile overrides fail
closed.

The host bridge retains only correlation data between effectd's leased turn
references and ACP entry boundaries; it is not a second run engine and does
not mutate durable Full Auto state. Omega journals those content-free refs and
entry indexes under its channel data directory before acknowledging thread or
turn creation. After a full app-process restart, the bridge reloads the
journal, reopens the matching Agent thread through `ThreadMetadataStore`, and
continues evidence refresh against the same entry boundaries.

`append_system_note` remains typed `unavailable`. The current `AcpThread`
entry model contains user messages, assistant messages, tool calls,
elicitations, completed plans, and compaction records, but no durable
owner-visible entry excluded from subsequent model context. Sending a hidden
command would not create an owner-visible entry; sending a user or assistant
message would misstate authorship and enter model context. Until ACP gains an
explicit non-model notice entry, the strongest safe owner-visible path is the
durable Full Auto run monitor backed by effectd, not a fabricated chat bubble.

## Next packets

1. [#22](https://github.com/OpenAgentsInc/omega/issues/22) GPUI launcher
2. [#23](https://github.com/OpenAgentsInc/omega/issues/23) routing and
   liveness
3. [#24](https://github.com/OpenAgentsInc/omega/issues/24) reports, Sync,
   mobile
4. [#25](https://github.com/OpenAgentsInc/omega/issues/25) native project
   join
5. [#26](https://github.com/OpenAgentsInc/omega/issues/26) proof
