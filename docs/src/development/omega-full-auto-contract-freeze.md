# Omega Full Auto contract freeze

Omega Full Auto implementation follows the freeze recorded in the OpenAgents
monorepo. This page is the Omega consumer pointer. It does not redefine the
lifecycle.

## Authority

- Freeze:
  [`docs/omega/2026-07-24-full-auto-contract-freeze.md`](https://github.com/OpenAgentsInc/openagents/blob/main/docs/omega/2026-07-24-full-auto-contract-freeze.md)
- Omega host ProductSpec:
  [`specs/omega/full-auto.product-spec.md`](https://github.com/OpenAgentsInc/openagents/blob/main/specs/omega/full-auto.product-spec.md)
  (rev 1)
- Desktop lifecycle ProductSpec:
  [`specs/desktop/full-auto.product-spec.md`](https://github.com/OpenAgentsInc/openagents/blob/main/specs/desktop/full-auto.product-spec.md)
  (rev 14)
- Port audit:
  [`docs/omega/2026-07-24-full-auto-port-audit.md`](https://github.com/OpenAgentsInc/openagents/blob/main/docs/omega/2026-07-24-full-auto-port-audit.md)
- Issue: [#19](https://github.com/OpenAgentsInc/omega/issues/19) (`OMEGA-FA-00`)

## Laws Omega must keep

- Full Auto is a dedicated run, never a composer toggle.
- Active run limit is 8. One active lease per thread.
- Ten-state lifecycle and legal transitions stay identical to Desktop.
- Non-overridable guardrails: `workspace_binding`, `own_capacity_only`,
  `no_rate_limit_reset_triggering`.
- Durable mutation lives in supervised `omega-effectd` through
  `full-auto-run-actions` (or its released successor).
- GPUI is launcher and monitor only. It is not run authority.
- Public receipts use schema `openagents.desktop.full_auto_run_receipt.v1`.
- MemoHarness and initiative stay deferred for `OMEGA-FA-01` through
  `OMEGA-FA-07` unless a later freeze admits them.

## Falsifier

A GPUI view, ACP panel, or ordinary chat path becomes Full Auto run authority.

## Next packets

1. [#23](https://github.com/OpenAgentsInc/omega/issues/23) routing and
   liveness (FA-03 GPUI launcher pinned in
   [omega-full-auto-gpui-launcher.md](./omega-full-auto-gpui-launcher.md))
2. [#24](https://github.com/OpenAgentsInc/omega/issues/24) reports, Sync,
   mobile
3. [#25](https://github.com/OpenAgentsInc/omega/issues/25) native project
   join
4. [#26](https://github.com/OpenAgentsInc/omega/issues/26) proof
