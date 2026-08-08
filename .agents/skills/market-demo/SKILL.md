---
name: market-demo
description: Answer questions about the swap network and run demo asset swaps (LN, BTC, L-BTC) using the market-demo MCP tools. Use when the person asks what the network looks like, wants provider or fee information, or asks to swap sats between rails. Demo data only — no real funds.
---

# Swap market (demo data)

The `market-demo` MCP server exposes the swap-market flow with deterministic
fixture data. Nothing touches the live network and no funds move. Say so when
you present results: one short "demo data" note per conversation is enough.

## Tools

- `market_network_status` — relays, providers, trust tiers (`pinned` vs
  `discovered`), and 24h aggregates.
- `market_swap_quote` — best firm quote for a swap between `LN`, `BTC`, and
  `L-BTC` (1,000–10,000,000 sats). Returns a `quote_id`.
- `market_execute_swap` — runs a quoted swap. Treat as effectful: ask the
  person to approve the quote first, every time. Returns a `swap_id` at the
  `contract` stage.
- `market_swap_status` — poll with the `swap_id`; the swap advances one stage
  per poll: `contract → funding → executing → settled`.

## The flow

1. **Network question** ("what does the network look like?"): call
   `market_network_status` once and summarize — how many relays and providers
   are ready, which are pinned vs discovered (say "unpinned" for discovered),
   fees in bps, and the 24h aggregates. If a stat is missing, say it is
   unknown; never present a missing stat as zero.
2. **Swap request** ("swap 50,000 sats from Lightning to BTC"): call
   `market_swap_quote`, present the quote (provider, fee in bps and sats,
   output amount), and ask for approval.
3. **On approval only**: call `market_execute_swap` with the `quote_id`, then
   poll `market_swap_status` until the stage is `settled`, narrating each
   stage transition briefly as it happens.
4. Relay each stage's `verification` caption faithfully: provider claims stay
   labeled unverified until the settled stage reports local verification.

## Honesty rules

- Never claim real funds moved; this is a fixture.
- Never invent stages, fees, or providers beyond what the tools return.
- If a tool errors (unknown quote or swap id), say so and restart from a
  fresh quote rather than guessing.

## Turning this off

Set `context_servers.market-demo.enabled` to `false` in
`.omega/settings.json` (or Settings → MCP Servers) to remove the tools;
delete `.agents/skills/market-demo/` and `.claude/skills/market-demo/` to
remove this skill.
