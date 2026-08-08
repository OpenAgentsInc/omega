---
name: market-demo
description: Answer questions about the swap network and run demo asset swaps (LN, BTC, L-BTC) using Omega's market tools. Use when the person asks what the network looks like, wants provider or fee information, or asks to swap sats between rails. Network status reads the live public regtest network (read-only); swaps are demo only — no real funds.
---

# Swap market

**Always use the market tools themselves** (`market_network_status`,
`market_swap_quote`, `market_execute_swap`, `market_swap_status`) — in this
app their results render as inline cards (the network map and the swap
lifecycle card). Never run `scripts/market-demo-mcp.mjs` through the shell
when the tools are available: shell output renders as plain text and the
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

Set each `market_*` tool to `false` in the active agent profile to remove the
tools. Delete `.agents/skills/market-demo/` and `.claude/skills/market-demo/`
to remove this skill.
