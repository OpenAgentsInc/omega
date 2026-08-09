---
name: market-demo
description: Answer questions about the swap network, run demo asset swaps (LN, BTC, L-BTC), and mock-provision a paid provider node with a cloud relay using Omega's market tools. Answer tool availability questions directly without a tool call or delegation. Use when the person asks what the network looks like, wants provider or fee information, asks to swap sats between rails, or asks to create provider infrastructure. Network status reads the live public regtest network (read-only); swaps and provisioning are demo fixtures.
---

# Swap market

**Always use the market tools themselves** (`market_network_status`,
`market_swap_quote`, `market_execute_swap`, `market_swap_status`,
`market_provision_cloud`) — in this
app their results render as inline cards (the network map, swap lifecycle,
and cloud provision card). Never run `scripts/market-demo-mcp.mjs` through
the shell when the tools are available: shell output renders as plain text and the
cards are lost. If the tools are missing from your session, say so instead
of working around it. "Test the market components" means: call each tool
and let the cards render.

Omega Agent receives these as built-in tools. The optional `market-demo` MCP
server exposes the same contract to external clients.
`market_network_status` reads the LIVE public regtest network (read-only:
manifest, relay health, provider profiles and offerings). The swap tools are
demo fixtures — live quoting requires the verified requester engine and is
not wired yet. Present it that way: network facts are live coordination
data whose provider claims stay unverified; swaps are a demo, said once per
conversation.

## Tools

- `market_network_status` — LIVE: relay health, providers with real fees,
  trust tiers (`pinned` from the signed manifest vs `discovered`), and 24h
  aggregates (unknown until receipt aggregation deploys — report unknown,
  never zero).
- `market_swap_quote` — returns a firm quote for a quote-only request between
  `LN`, `BTC`, and `L-BTC` (1,000–10,000,000 sats). Returns a `quote_id`.
- `market_execute_swap` — creates and runs an authorized swap directly from
  `from`, `to`, and `amount_sats`, or runs a prior `quote_id`. The person's
  swap request is authorization. Streams one card through
  `quote → contract → funding → executing → settled` and returns after
  settlement.
- `market_swap_status` — reads the latest projected state for a `swap_id`.
  Reads never advance the swap.
- `market_provision_cloud` — mock-checks a paid account, then streams one card
  through `payment → relay → provider → connected`. It creates no payment or
  infrastructure. The default region is `us-central1`.

## The flow

1. **Network question** ("what does the network look like?"): call
   `market_network_status` once and summarize — how many relays and providers
   are ready, which are pinned vs discovered (say "unpinned" for discovered),
   fees in bps, and the 24h aggregates. If a stat is missing, say it is
   unknown; never present a missing stat as zero.
2. **Swap request** ("swap 50,000 sats from Lightning to BTC"): treat the
   request itself as authorization. Call `market_execute_swap` directly with
   `from`, `to`, and `amount_sats`. Do not call `market_swap_quote` first and
   do not ask for another approval.
3. **Quote-only request**: call `market_swap_quote` and stop. If the person
   later asks to execute that quote, call `market_execute_swap` with its
   `quote_id`.
4. Let `market_execute_swap` stream its lifecycle. Do not drive progress by
   repeatedly calling `market_swap_status`; use that tool only to inspect an
   existing swap after execution.
5. Relay each stage's `verification` caption faithfully: provider claims stay
   labeled unverified until the settled stage reports local verification.
6. **Provider infrastructure request**: call `market_provision_cloud` once.
   Use the person's provider name and region when given; otherwise use the
   tool defaults. State once that this path is a mock and creates no bill or
   infrastructure.

## Honesty rules

- Never claim real funds moved; this is a fixture.
- Never claim a mock cloud provision charged the person or created resources.
- Never invent stages, fees, or providers beyond what the tools return.
- If a tool errors (unknown quote or swap id), say so and restart from a
  fresh quote rather than guessing.

## Turning this off

Set each `market_*` tool to `false` in the active agent profile to remove the
tools. Delete `.agents/skills/market-demo/` and `.claude/skills/market-demo/`
to remove this skill.
