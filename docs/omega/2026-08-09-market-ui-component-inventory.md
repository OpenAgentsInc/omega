# Market UI component inventory

- Status: Planning inventory, not a commitment to build order
- Date: 2026-08-09
- Scope: every UI component the full market vision needs — trading, swaps,
  portfolio, agent supervision, marketplace — in GPUI, with an honest
  have/partial/build status against the Zed-inherited `crates/ui` set and
  the viz primitives from #247
- Rendering law (from the #247 work): market graphics are GPUI canvas +
  `PathBuilder`/lyon, no SVG; render only what is visible (a screen that
  instantiates every element at once was the lag bug that forced this
  rule); explicit flattening for dashed curves (the lyon dash/NaN panic).
  High-frequency surfaces throttle notify and batch per frame.

Legend: ✅ have (Zed-inherited or already built) · 🟡 partial (base exists,
market variant needed) · 🔨 build.

## 1. Charting and data visualization

| Component | Notes | Status |
|---|---|---|
| Canvas plot base (axes, layout, hit-testing, crosshair plumbing) | Generalize from `viz_geometry`/`viz_panorama`; one shared plot kernel so every chart below is a config, not a fork | 🟡 |
| Candlestick/OHLC chart | Intervals, zoom/pan, autoscale, price+time axes, hover crosshair with OHLC readout, live last-candle updates | 🔨 |
| Line/area chart | Price, equity, funding series; `viz_edge`/`viz_progress_rail` give path styling | 🟡 |
| Volume bars (under-chart histogram) | Paired with candles, shared time axis | 🔨 |
| Depth chart | Cumulative bid/ask area from L2 book, mid marker, hover size readout | 🔨 |
| Sparkline | Tiny inline series for cards, watchlists, transcript cards | 🟡 |
| Funding-rate history chart | Bar/line with sign coloring, settlement markers, EMA overlay | 🔨 |
| Equity curve + drawdown chart | Ledger-driven; drawdown as shaded underwater plot | 🔨 |
| Returns histogram | Strategy evaluation views | 🔨 |
| Calibration chart | Predicted confidence vs realized frequency, per agent/strategy | 🔨 |
| Heatmap | Correlation matrices, liquidation clusters, hour-of-day activity | 🔨 |
| Gauge / limit meter | Mandate headroom (balance, leverage, loss stop, order rate), rate-budget usage; `progress` exists, needs threshold zones + semantic colors | 🟡 |
| Network panorama / node-edge map | Live swap-network view | ✅ (`viz_panorama`, `viz_node`, `viz_edge`, `viz_zone`, `viz_port`) |
| Time-axis + numeric-axis utilities | Tick generation, ms→label, log/linear price scales, tabular-figure alignment | 🔨 (shared by everything above) |
| Drawing tools (trendlines, fibs, annotations) | Deliberately later; not needed for the agent-first terminal | 🔨 (deferred) |

## 2. Market data displays

| Component | Notes | Status |
|---|---|---|
| Price ticker readout | Last/mark/index/oracle, change %, up/down flash animation on tick | 🔨 |
| Market stats strip | 24h volume, OI, funding + countdown, spread bps | 🔨 |
| Watchlist table | Sortable, sparkline column, live updates; `data_table` exists, needs high-frequency update discipline | 🟡 |
| Order-book ladder (DOM) | Grouped price levels, size bars, cumulative column, mid/spread row, flash on change; the hardest high-frequency surface — budget for a dedicated virtualized implementation | 🔨 |
| Time & sales tape | Virtualized streaming trade list, side coloring, size buckets | 🔨 |
| Instrument selector | Fuzzy search over the instrument catalog with venue/kind facets; `picker` pattern exists | 🟡 |
| Funding countdown | Per-venue cadence-aware (hourly vs 3×/day) | 🔨 |
| Outcome/probability bar | Prediction-market prices as probability bars with implied-odds readout | 🔨 |
| Oracle/attestation readout | Announced vs attested price, oracle identity, verification state | 🔨 |
| Venue status badge | Connected/degraded/halted per venue; capability/account-mode badge (e.g. unified account) | 🟡 (`indicator`, `chip` bases) |

## 3. Order entry and management

| Component | Notes | Status |
|---|---|---|
| Order ticket | Market/limit/trigger tabs; size in units or notional with a slider vs available margin; leverage selector; reduce-only; TIF; TP/SL attachment; live margin + liquidation-price preview; per-venue tick/lot validation feedback | 🔨 |
| Order confirm dialog | The `confirm: true` gate rendered honestly: exact order, cost estimate, mandate headroom consumed, counterparty, network label (testnet/mainnet) | 🟡 (`modal` base) |
| Open orders table | Cancel/modify per row, batch cancel, status chips, cloid/oid | 🟡 (`data_table` base) |
| Positions panel | Entry/mark/liq prices, liquidation-distance meter, uPnL with color semantics, close/reduce controls | 🔨 |
| Order lifecycle chips/toasts | placed → resting → partially filled → filled/cancelled; `notification` base plus streaming-card updates (pattern proven by the swap card) | 🟡 |
| TWAP configurator | Slices, duration, randomization band, progress view | 🔨 (deferred until TWAP ships) |
| Dead-man's-switch indicator | scheduleCancel armed/expiry countdown | 🔨 |

## 4. Swap and network (the live lane)

| Component | Notes | Status |
|---|---|---|
| Swap lifecycle card | quote → contract → funding → executing → settled streaming card | ✅ (`viz_swap`) |
| Network card / panorama card | Inline network status | ✅ (`viz_panorama` + market tool cards) |
| RFQ quote comparison | Multi-provider quote table: output, fee, spread, provider reputation chip, expiry countdown, best-quote highlight | 🔨 |
| Rail selector | Onchain/LN/Liquid/Ark with fee/latency hints | 🔨 |
| Lightning invoice display | BOLT11/taproot-asset invoice with QR code render, copy, paid-state polling | 🔨 (QR generation needed) |
| Address/invoice verification row | Truncated-with-checksum display, reveal, copy; never-ambiguous asset+network labeling | 🔨 |
| Swap/transfer history table | Receipts-linked | 🟡 (`data_table`) |
| Provision progress card | Cloud provision demo flow | ✅ (`viz_cloud_provision`) |

## 5. Portfolio and accounting

| Component | Notes | Status |
|---|---|---|
| Command-center header | Portfolio value, today/30d PnL, max drawdown, active theses count — the top-of-screen readout | 🔨 |
| Balances table | Per (venue, asset): balance, unrealized, in-flight, counterparty exposure (from the derived metric), usable margin (mode-aware) | 🟡 (`data_table`) |
| Ledger browser | Double-entry drill-down: entry → postings, per-strategy attribution filters, sequence/hash verification state | 🔨 |
| Receipt viewer | Typed receipt rendering with verification status; links to venue records | 🔨 |
| Fee/funding breakdown | Per strategy/venue/day; feeds the cost-floor reports | 🔨 |
| Reconciliation status | Ledger-vs-venue diff (must be zero), last reconcile time, per (venue, asset) | 🔨 |
| Deposit/withdraw flow | Venue-specific: invoice-based (LN), bridge-aware copy for EVM-bridged venues | 🔨 |

## 6. Agent and mandate supervision (the differentiator)

| Component | Notes | Status |
|---|---|---|
| Mandate editor + approval dialog | Typed limit fields with units, widening-vs-narrowing diff view (widen = approval ceremony), expiry, allowed strategies | 🔨 (approval flow exists for LN Markets settings; generalize) |
| Mandate status card | Live headroom meters per limit, revoke control, scope (venue, network, collateral) | 🔨 |
| Strategy card | Lifecycle streaming: state machine phase, position, last action, halt reason surfacing | 🟡 (lnmarkets operator panel has the v0) |
| Backtest report card | Gate outcome, trade count, fees vs gross, expectancy, drawdown | 🟡 (report exists; card polish) |
| Agent activity feed | The timestamped "LATEST" stream: observations, decisions, orders, halts; filterable, links into threads | 🔨 |
| Review-turn card | Scheduled-turn summary in transcript: what was read, prediction emitted, action taken/none, token cost | 🔨 |
| Prediction card | Instrument, direction/distribution, confidence, horizon, resolution rule; later joined outcome + score | 🔨 |
| Wakeup/schedule timeline | Upcoming cadence + recent event-triggered wakeups with budgets | 🔨 |
| Halt banner | Strategy/venue halts with typed reason and resume path | 🟡 (`banner` base) |
| Approval queue | Pending approvals (mandate widening, orders awaiting confirm) in one place | 🔨 |
| Agent roster | The chat1 mock: per-agent status (researching/watching/monitoring/thinking) | 🔨 |

## 7. Marketplace and reputation (later phases)

| Component | Notes | Status |
|---|---|---|
| Strategy listing card | Name, tier badge (OPEN/VERIFIED/PRIVATE/OPENAGENTS), live-since, capital, net return, Sharpe, drawdown, calibration, receipts-verified mark | 🔨 |
| Track-record detail view | Equity curve + attribution + receipts drill-down | 🔨 |
| Provider reputation card | Fill rate, quote competitiveness, settlement latency, receipt history | 🔨 |
| Tier/verification badges | Consistent iconography for verification states | 🟡 (`chip`/`count_badge` bases) |
| Leaderboard table | Sortable, time-window selector | 🟡 (`data_table`) |
| Allocation composer | Percent sliders across strategies with constraint feedback | 🔨 (deferred to allocator phase) |

## 8. Operator and infrastructure

| Component | Notes | Status |
|---|---|---|
| Collector health row | Status, per-stream last-event age, backfill progress, reconnect count | ✅ (lnmarkets/hyperliquid panels have v0) |
| Rate-budget meter | Live weighted usage vs venue allowance, cancel-reserve state | 🔨 |
| Key/credential status | Agent-wallet approval + expiry, nonce high-water state, rotation prompts | 🟡 (settings page base) |
| Event/log stream | Filterable typed events (not raw logs) for the market lane | 🔨 |
| Capability probe panel | Observed venue modes with probed-at timestamps, unknown-mode alerts | 🔨 |

## 9. Chat/inline surface (pattern proven, extend per schema)

| Component | Notes | Status |
|---|---|---|
| Schema-dispatched tool cards | Versioned schema string → card renderer | ✅ (market tool cards) |
| Streaming card updates | One tool call updating a single card through a lifecycle | ✅ (swap card pattern) |
| Inline chart cards | Sparkline/candle-lite inside transcript cards | 🟡 (needs §1 kernel) |
| Market chat demo harness | Scripted demo conversations for the component library | ✅ (`viz_market_chat_demo`) |

## 10. Shell, theming, and cross-cutting primitives

| Component | Notes | Status |
|---|---|---|
| Panels/docks/splits, tabs, command palette, pickers, settings pages, notifications, keybindings, scrollbars, virtualized lists, markdown, modals, context menus | The Zed inheritance — this is why GPUI is the right substrate | ✅ |
| Component library screen | Gated catalog with live previews (render-only-selected discipline) | ✅ (#247) |
| Financial theme tokens | Up/down/neutral semantic colors (colorblind-safe pair, not raw red/green), PnL sign styling, testnet/mainnet environment tinting, monospace tabular numerals everywhere numbers align | 🔨 |
| Number/unit formatting kit | Sats↔BTC, USD cents, per-asset decimals, compact notation (1.2M), signed coloring, flash-on-change; one crate so every surface agrees | 🔨 |
| High-frequency update discipline | Batched per-frame updates, notify throttling, damage-region-friendly list rows — a documented pattern + helpers, or the book/tape/watchlist will each invent one | 🔨 |
| QR code renderer | Canvas-drawn QR for invoices/addresses | 🔨 |
| Countdown/relative-time primitives | Funding, expiries, dead-man timers | 🔨 |

## Build-order note

The leverage order is bottom-up: §10 formatting kit + financial tokens and
the §1 plot kernel unblock nearly everything; the order-book ladder and
candlestick chart are the two components that deserve dedicated
performance work; §6 is where Omega stops looking like an exchange
front-end and starts looking like what it actually is. Marketplace (§7)
and drawing tools wait for their phases. Everything lands through the
component library screen first, with demo data, before it touches live
surfaces — the pattern that already caught the lag and rendering bugs
once.
