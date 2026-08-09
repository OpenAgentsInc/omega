---
name: market-demo
description: Answer questions about the swap network, run demo or regtest asset swaps, and mock-provision a paid provider node with a cloud relay using Omega's market tools. Answer tool availability questions directly without a tool call or delegation. Use when the person asks what the network looks like, wants provider or fee information, asks to swap sats between rails, or asks to create provider infrastructure. Every swap tool requires demo, regtest, or mainnet. Mainnet is blocked.
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
Every swap tool call must specify one `network` value:

- `demo` returns deterministic representative fixtures. It moves no funds.
- `regtest` uses live shared providers and valueless Bitcoin regtest funds.
  The requester verifies both settlement rails. The public service currently
  supports only `LN → BTC` and `BTC → LN` from 100,000 through 1,000,000 sats.
- `mainnet` is blocked. The tools return a warning and send no mainnet request.

Use the network the person names. If they do not name one, use `demo` and say
that the result is a fixture. Do not silently treat `regtest` as `demo`.

## Tools

- `market_network_status` — requires `network`. Demo returns a representative
  fixture. Regtest reads live relay health, provider offerings, and trust
  tiers from the public regtest network. Mainnet returns a blocked warning.
- `market_swap_quote` — requires `network`. Demo returns a firm fixture quote
  for `LN`, `BTC`, or `L-BTC` from 1,000 through 10,000,000 sats. Regtest
  returns an indicative live route for the two supported directions. The
  signed provider quotes are obtained during execution.
- `market_execute_swap` — creates and runs an authorized swap directly from
  `network`, `from`, `to`, and `amount_sats`, or runs a prior `quote_id` with
  its matching `network`. The person's swap request is authorization. Demo
  streams one fixture card through
  `quote → contract → funding → executing → settled` and returns after
  settlement. Regtest returns only after requester-verified Bitcoin and
  Lightning evidence exists.
- `market_swap_status` — requires `network` and reads the latest recorded state
  for a `swap_id`. Reads never advance the swap.
- `market_provision_cloud` — mock-checks a paid account, then streams one card
  through `payment → relay → provider → connected`. It creates no payment or
  infrastructure. The default region is `us-central1`.

## LN Markets

LN Markets is separate from the OpenAgents swap market. Use the built-in
`lnmarkets_*` tools directly. Do not delegate an LN Markets question and do not
substitute a `market_*` fixture.

- `lnmarkets_account` reads the configured Signet or Mainnet account.
- `lnmarkets_market_data` has four views:
  - `snapshot` reads the current ticker, index, funding, liquidity price tiers,
    and synthetic USD quote. This is the default when `request` is omitted.
  - `history` reads paginated OHLCV candles and funding settlements. Supply an
    ISO 8601 `from` time, a candle resolution, and a bounded limit.
  - `portfolio` reads the account, cross position and orders, isolated trades,
    funding fees, transfers, wallet deposit and withdrawal history,
    notifications, and synthetic USD swap history. It uses the saved credential
    and the requested network must match that credential.
  - `live` opens a bounded LN Markets WebSocket subscription and returns the
    observed events. Public market topics need no credential. Private
    position, order, trade, deposit, and withdrawal topics authenticate with
    the saved credential and require its network and permissions.
- `lnmarkets_swap` converts between BTC and synthetic USD on the selected LN
  Markets account. This is separate from futures order placement.
- `lnmarkets_strategy` can run a collected-data backtest, read durable reports,
  or start, adjust, halt, and inspect an automated strategy. Run a backtest for
  the exact configuration before attempting to start or adjust it. The cost
  model must name its local measurement source and match the configuration's
  measured round-trip cost.

Use REST history for analysis over a time range and the bounded WebSocket view
for current changes. Never claim a stream is still active after the tool
returns. Report any per-section `ok: false` result from `portfolio`; it usually
means the API key lacks that read scope.

### Scheduled portfolio reviews

When a strategy starts, its thread becomes the portfolio-review thread. A
scheduled or event-triggered review already contains local feature, ledger,
mandate, strategy-state, backtest-report, opportunity, and limit-headroom data. Do not fetch remote market or account data during that turn, and do not repeat the local
feature or ledger reads. Rank the supplied opportunity inventory. Use
`lnmarkets_strategy` at most once to start, adjust, or halt a supported
strategy inside the active mandate. Never use `lnmarkets_swap` or another raw
order path from a review turn. The strategy engine owns execution and enforces
the mandate. End with one short reasoning note and the supplied daily profit,
fees, funding, drawdown, and headroom figures.

## The flow

1. **Network question** ("what does the network look like?"): call
   `market_network_status` once with the chosen network and summarize — how many relays and providers
   are ready, which are pinned vs discovered (say "unpinned" for discovered),
   fees in bps, and the 24h aggregates. If a stat is missing, say it is
   unknown; never present a missing stat as zero.
2. **Swap request** ("swap 50,000 sats from Lightning to BTC"): treat the
   request itself as authorization. Call `market_execute_swap` directly with
   `network`, `from`, `to`, and `amount_sats`. Do not call
   `market_swap_quote` first and do not ask for another approval.
3. **Quote-only request**: call `market_swap_quote` and stop. If the person
   later asks to execute that quote, call `market_execute_swap` with its
   `network` and `quote_id`.
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

- For demo, never claim funds moved; it is a fixture.
- For regtest, call the funds valueless regtest funds. Do not call them demo
  fixtures or mainnet funds.
- For mainnet, preserve the blocked warning. Do not retry on another network.
- Never claim a mock cloud provision charged the person or created resources.
- Never invent stages, fees, or providers beyond what the tools return.
- If a tool errors (unknown quote or swap id), say so and restart from a
  fresh quote rather than guessing.

## Turning this off

Set each `market_*` tool to `false` in the active agent profile to remove the
tools. Delete `.agents/skills/market-demo/` and `.claude/skills/market-demo/`
to remove this skill.
