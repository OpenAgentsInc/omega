use std::collections::BTreeSet;

use command_center_ui::{
    ActivityEvent, ActivityEventKind, AgentActivityState, AgentRosterEntry, BalanceMode,
    BalanceRow, MandateUsage, PendingApproval, PendingApprovalKind, PortfolioSummary,
    demo_activity_events, demo_roster_entries,
};
use gpui::SharedString;
use market_ui::{
    NautilusBookSource, NautilusCandleSource, NautilusLiveSnapshot, NautilusOrderIntent,
};
use nautilus_sidecar::{
    AgentApprovalStatus, AgentWalletSummary, NautilusCredentialSnapshot, NautilusMarketSnapshot,
    Network, StreamEvent,
};
use trading_ledger::{Counterparty, LedgerStore};
use trading_mandate::{AssetId, ReviewCadence, TradingMandate, TradingNetwork};
use ui::{
    AnalyticsLine, AnalyticsPoint, BookSource, CandleFill, CandleSeries, CandleSource,
    DemoBookSource, DemoCandleSource, DemoFillsOnCandlesSource, DemoIndicatorOverlaySource,
    DemoInstrumentCatalogSource, DemoOpenOrdersSource, DemoOscillatorPaneSource,
    DemoPositionsSource, DemoStatisticGridSource, DemoTearsheetSource, DemoTradeLogSource,
    EquityPoint, EquitySeries, FillEffect, FillSide, FillsOnCandlesData, FillsOnCandlesSource,
    IndicatorOverlayData, IndicatorOverlaySource, Instrument, InstrumentCatalogSource,
    InstrumentKind, MarketDirection, OpenOrder, OpenOrderStatus, OpenOrdersSource, OrderBook,
    OscillatorPaneData, OscillatorPaneSource, Position, PositionsSource, ReturnSeries,
    StatisticGridSource, StatisticKind, StatisticUnit, StatisticValue, TearsheetData,
    TearsheetSource, TradeLogRow, TradeLogSource,
};

#[derive(Clone)]
pub struct MandateCardData {
    pub mandate: TradingMandate,
    pub usage: MandateUsage,
    pub revision: u64,
    pub now_ms: i64,
}

#[derive(Clone)]
pub struct PortfolioPanelData {
    pub summary: PortfolioSummary,
    pub balances: Vec<BalanceRow>,
    pub positions: Vec<Position>,
    pub roster: Vec<AgentRosterEntry>,
    pub activity: Vec<ActivityEvent>,
    pub mandate: Option<MandateCardData>,
    pub approvals: Vec<PendingApproval>,
    pub credential: NautilusCredentialSnapshot,
    pub error: Option<SharedString>,
}

#[derive(Clone)]
pub struct TradingPanelData {
    pub instruments: Vec<Instrument>,
    pub candles: CandleSeries,
    pub book: OrderBook,
    pub order_intent: Option<NautilusOrderIntent>,
    pub open_orders: Vec<OpenOrder>,
}

#[derive(Clone)]
pub struct AnalyticsPanelData {
    pub overlays: IndicatorOverlayData,
    pub oscillators: Vec<OscillatorPaneData>,
    pub statistics: Vec<StatisticValue>,
    pub tearsheet: TearsheetData,
    pub fills: FillsOnCandlesData,
    pub trades: Vec<TradeLogRow>,
}

impl PortfolioPanelData {
    pub fn demo() -> Self {
        let mandate = TradingMandate {
            venue: "hyperliquid".to_owned(),
            network: TradingNetwork::Testnet,
            collateral_asset: AssetId::usdc(),
            objective: "Bounded testnet execution".to_owned(),
            max_venue_balance: 2_500_000,
            max_position_usd: 10_000,
            max_leverage: 3,
            daily_loss_stop: 25_000,
            max_orders_per_hour: 20,
            min_liquidation_buffer_bps: 1_500,
            allowed_strategies: BTreeSet::from(["funding_carry".to_owned()]),
            review_cadence: ReviewCadence::Interval { seconds: 3_600 },
            expires_at_ms: 1_757_000_000_000,
        };
        Self {
            summary: PortfolioSummary {
                portfolio_value_sats: 4_812_930,
                pnl_today_sats: 12_480,
                pnl_30d_sats: 291_552,
                max_drawdown_sats: -48_211,
                active_strategy_count: 2,
            },
            balances: vec![BalanceRow {
                venue: "hyperliquid".into(),
                asset: "USDC".into(),
                balance: 1_250_000,
                unrealized: 42_500,
                in_flight: 0,
                counterparty_exposure: 1_292_500,
                usable_margin: Some(912_000),
                mode: BalanceMode::Unified,
            }],
            positions: DemoPositionsSource.positions(),
            roster: demo_roster_entries(),
            activity: demo_activity_events(),
            mandate: Some(MandateCardData {
                mandate,
                usage: MandateUsage {
                    venue_balance: 1_250_000,
                    position_notional_usd: 4_800,
                    leverage_hundredths: 160,
                    daily_loss: 2_200,
                    orders_this_hour: 4,
                    liquidation_distance_bps: Some(4_200),
                },
                revision: 7,
                now_ms: 1_756_900_000_000,
            }),
            approvals: vec![
                PendingApproval {
                    approval_id: "mandate-8".into(),
                    requested_at_ms: 1_756_899_900_000,
                    summary: "Raise position limit to $15,000".into(),
                    kind: PendingApprovalKind::MandateWidening {
                        venue: "hyperliquid".into(),
                        digest: "sha256:8f3c".into(),
                    },
                },
                PendingApproval {
                    approval_id: "order-probe-42".into(),
                    requested_at_ms: 1_756_899_980_000,
                    summary: "Buy 0.001 BTC-PERP".into(),
                    kind: PendingApprovalKind::OrderConfirmation {
                        instrument: "BTC-PERP".into(),
                        request_id: "omega-ui-place-42".into(),
                    },
                },
            ],
            credential: NautilusCredentialSnapshot {
                selected_network: Network::Testnet,
                testnet: Some(AgentWalletSummary {
                    network: Network::Testnet,
                    owner_address: "0x71c4…8e12".to_owned(),
                    agent_address: "0xa021…0f73".to_owned(),
                    agent_name: "omega-testnet".to_owned(),
                    approval: AgentApprovalStatus::Approved {
                        valid_until_ms: 1_781_000_000_000,
                    },
                }),
                mainnet: None,
                halt: None,
                wakeup: None,
                error: None,
                loading: false,
            },
            error: None,
        }
    }

    pub fn live(
        market: &NautilusMarketSnapshot,
        ledger: Option<&LedgerStore>,
        mandate: Option<(TradingMandate, u64)>,
        now_ms: i64,
        store_error: Option<&str>,
        credential: NautilusCredentialSnapshot,
    ) -> Self {
        let live = NautilusLiveSnapshot::new(market.clone());
        let account = live.account_summary();
        let summary = ledger
            .and_then(|ledger| {
                PortfolioSummary::from_ledger(ledger, &["hyperliquid"], 0, now_ms).ok()
            })
            .unwrap_or_default();
        let balances = ledger
            .and_then(|ledger| ledger.latest_counterparty_exposures().ok())
            .unwrap_or_default()
            .into_iter()
            .map(|exposure| {
                let usable = matches!(
                    &exposure.counterparty,
                    Counterparty::Venue { venue } if venue == "hyperliquid"
                )
                .then_some(account.available_margin_cents)
                .flatten();
                BalanceRow::from_exposure(&exposure, usable, BalanceMode::Unified)
            })
            .collect();
        let positions = positions_from_market(market);
        let roster = vec![AgentRosterEntry {
            name: "Nautilus".into(),
            state: AgentActivityState::Monitoring,
            detail: Some(format!("frame {}", account.frame_count).into()),
        }];
        let activity = market
            .recent_fills
            .iter()
            .rev()
            .take(40)
            .filter_map(fill_activity)
            .collect();
        let usage = MandateUsage {
            venue_balance: account
                .available_margin_cents
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or_default(),
            position_notional_usd: positions
                .iter()
                .map(|position| (position.mark_price * position.units).max(0.0) as u64)
                .sum(),
            leverage_hundredths: 0,
            daily_loss: 0,
            orders_this_hour: u32::try_from(account.order_count).unwrap_or(u32::MAX),
            liquidation_distance_bps: positions.iter().filter_map(liquidation_distance_bps).min(),
        };
        Self {
            summary,
            balances,
            positions,
            roster,
            activity,
            mandate: mandate.map(|(mandate, revision)| MandateCardData {
                mandate,
                usage,
                revision,
                now_ms,
            }),
            approvals: Vec::new(),
            credential,
            error: store_error.map(Into::into),
        }
    }
}

impl TradingPanelData {
    pub fn demo() -> Self {
        Self {
            instruments: DemoInstrumentCatalogSource.instruments(),
            candles: DemoCandleSource::default().series(),
            book: DemoBookSource.snapshot(),
            order_intent: NautilusOrderIntent::testnet_probe(116_000.0, 1_500_000, 42),
            open_orders: DemoOpenOrdersSource.open_orders(),
        }
    }

    pub fn live(market: &NautilusMarketSnapshot) -> Self {
        let live = NautilusLiveSnapshot::new(market.clone());
        let order_intent = live
            .latest_quote()
            .map(|(bid, _)| bid)
            .zip(live.account_summary().available_margin_cents)
            .and_then(|(bid, margin)| {
                NautilusOrderIntent::testnet_probe(bid, margin, market.frame_count)
            });
        Self {
            instruments: vec![Instrument {
                symbol: "BTC-PERP".into(),
                name: "Bitcoin perpetual".into(),
                venue: "Hyperliquid".into(),
                kind: InstrumentKind::Perpetual,
            }],
            candles: NautilusCandleSource::new(market.clone()).series(),
            book: NautilusBookSource::new(market.clone()).snapshot(),
            order_intent,
            open_orders: open_orders_from_market(market),
        }
    }
}

impl AnalyticsPanelData {
    pub fn demo() -> Self {
        Self {
            overlays: DemoIndicatorOverlaySource.indicator_overlays(),
            oscillators: DemoOscillatorPaneSource.oscillator_panes(),
            statistics: DemoStatisticGridSource.statistics(),
            tearsheet: DemoTearsheetSource.tearsheet(),
            fills: DemoFillsOnCandlesSource.fills_on_candles(),
            trades: DemoTradeLogSource.trade_log(),
        }
    }

    pub fn live(market: &NautilusMarketSnapshot, now_ms: i64) -> Self {
        let candles = NautilusCandleSource::new(market.clone()).series();
        let points = candles
            .candles()
            .iter()
            .map(|candle| AnalyticsPoint {
                time_ms: candle.time_ms,
                value: candle.close,
            })
            .collect::<Vec<_>>();
        let returns = points
            .windows(2)
            .filter_map(|pair| {
                let previous = pair.first()?.value;
                let current = pair.get(1)?.value;
                (previous > 0.0).then_some(current / previous - 1.0)
            })
            .collect::<Vec<_>>();
        let moving_average = moving_average(&points, 20);
        let overlays = IndicatorOverlayData {
            price: AnalyticsLine {
                label: "Price".into(),
                points: points.clone(),
            },
            moving_averages: vec![AnalyticsLine {
                label: "SMA 20".into(),
                points: moving_average,
            }],
            bollinger: None,
            keltner: None,
            donchian: None,
            ichimoku: None,
            vwap: None,
            decimals: 2,
        };
        let oscillator_points = returns
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                let time_ms = points.get(index.saturating_add(1))?.time_ms;
                Some(AnalyticsPoint {
                    time_ms,
                    value: (50.0 + value * 2_000.0).clamp(0.0, 100.0),
                })
            })
            .collect();
        let oscillators = vec![OscillatorPaneData {
            kind: ui::OscillatorKind::Rsi,
            domain: (0.0, 100.0),
            reference_lines: vec![30.0, 70.0],
            series: vec![AnalyticsLine {
                label: "RSI".into(),
                points: oscillator_points,
            }],
        }];
        let maximum_drawdown = maximum_drawdown(&returns);
        let win_rate = if returns.is_empty() {
            0.0
        } else {
            returns.iter().filter(|value| **value > 0.0).count() as f64 / returns.len() as f64
        };
        let statistics = vec![
            StatisticValue {
                kind: StatisticKind::MaxDrawdown,
                value: maximum_drawdown,
                unit: StatisticUnit::Percent,
                favorable_when_positive: false,
            },
            StatisticValue {
                kind: StatisticKind::WinRate,
                value: win_rate,
                unit: StatisticUnit::Percent,
                favorable_when_positive: true,
            },
        ];
        let base_equity = NautilusLiveSnapshot::new(market.clone())
            .account_summary()
            .available_margin_cents
            .unwrap_or_default();
        let equity = equity_from_returns(&points, &returns, base_equity, now_ms);
        let rolling_sharpe = AnalyticsLine {
            label: "Rolling Sharpe".into(),
            points: returns
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    Some(AnalyticsPoint {
                        time_ms: points.get(index.saturating_add(1))?.time_ms,
                        value: *value,
                    })
                })
                .collect(),
        };
        let tearsheet = TearsheetData {
            title: "BTC testnet · live".into(),
            generated_at_ms: now_ms,
            equity,
            rolling_sharpe,
            pnl_distribution: ReturnSeries::new(returns),
            statistics: statistics.clone(),
        };
        let fills = FillsOnCandlesData {
            candles,
            fills: fills_from_market(market),
        };
        let trades = trade_log_from_market(market);
        Self {
            overlays,
            oscillators,
            statistics,
            tearsheet,
            fills,
            trades,
        }
    }
}

fn moving_average(points: &[AnalyticsPoint], window: usize) -> Vec<AnalyticsPoint> {
    points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let start = index.saturating_add(1).saturating_sub(window);
            let values = points.get(start..=index).unwrap_or_default();
            let total = values.iter().map(|point| point.value).sum::<f64>();
            AnalyticsPoint {
                time_ms: point.time_ms,
                value: total / values.len().max(1) as f64,
            }
        })
        .collect()
}

fn maximum_drawdown(returns: &[f64]) -> f64 {
    let mut equity = 1.0f64;
    let mut peak = 1.0f64;
    let mut drawdown = 0.0f64;
    for value in returns {
        equity *= 1.0 + value;
        peak = peak.max(equity);
        drawdown = drawdown.max(1.0 - equity / peak.max(f64::EPSILON));
    }
    drawdown
}

fn equity_from_returns(
    points: &[AnalyticsPoint],
    returns: &[f64],
    base_equity: i64,
    now_ms: i64,
) -> EquitySeries {
    if points.len() < 2 {
        return EquitySeries::new(vec![
            EquityPoint {
                time_ms: now_ms.saturating_sub(1),
                equity_cents: base_equity,
            },
            EquityPoint {
                time_ms: now_ms,
                equity_cents: base_equity,
            },
        ]);
    }
    let mut equity = base_equity;
    let mut values = vec![EquityPoint {
        time_ms: points
            .first()
            .map_or(now_ms.saturating_sub(1), |point| point.time_ms),
        equity_cents: equity,
    }];
    for (index, value) in returns.iter().enumerate() {
        equity = ((equity as f64) * (1.0 + value)).round() as i64;
        if let Some(point) = points.get(index.saturating_add(1)) {
            values.push(EquityPoint {
                time_ms: point.time_ms,
                equity_cents: equity,
            });
        }
    }
    EquitySeries::new(values)
}

fn value_string(value: &serde_json::Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| value.get(*name))
        .and_then(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .or_else(|| value.as_f64().map(|number| number.to_string()))
        })
}

fn value_f64(value: &serde_json::Value, names: &[&str]) -> Option<f64> {
    names
        .iter()
        .find_map(|name| value.get(*name))
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .filter(|value| value.is_finite())
}

fn positions_from_market(market: &NautilusMarketSnapshot) -> Vec<Position> {
    market
        .positions
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            let units = value_f64(value, &["quantity", "size", "signed_qty"])?;
            let entry_price = value_f64(value, &["avg_px_open", "entry_price", "avg_price"])?;
            let mark_price = value_f64(value, &["mark_price", "last_price"]).unwrap_or(entry_price);
            let liquidation_price =
                value_f64(value, &["liquidation_price", "liq_price"]).unwrap_or_default();
            Some(Position {
                position_id: value_string(value, &["position_id", "id"])
                    .unwrap_or_else(|| format!("position-{index}"))
                    .into(),
                instrument: value_string(value, &["instrument_id", "symbol"])
                    .unwrap_or_else(|| "BTC-PERP".to_owned())
                    .into(),
                direction: if units < 0.0 {
                    MarketDirection::Down
                } else {
                    MarketDirection::Up
                },
                units: units.abs(),
                entry_price,
                mark_price,
                liquidation_price,
                unrealized_pnl: value_f64(value, &["unrealized_pnl", "unrealized_pnl_usd"])
                    .unwrap_or_default(),
            })
        })
        .collect()
}

fn liquidation_distance_bps(position: &Position) -> Option<u32> {
    (position.mark_price > 0.0 && position.liquidation_price > 0.0).then(|| {
        (((position.mark_price - position.liquidation_price).abs() / position.mark_price)
            * 10_000.0)
            .round()
            .clamp(0.0, u32::MAX as f64) as u32
    })
}

fn open_orders_from_market(market: &NautilusMarketSnapshot) -> Vec<OpenOrder> {
    market
        .orders
        .iter()
        .filter_map(|value| {
            let remaining = value_f64(value, &["leaves_qty", "remaining", "quantity"])?;
            let side = value_string(value, &["side"]).unwrap_or_default();
            Some(OpenOrder {
                client_order_id: value_string(value, &["client_order_id", "cloid"])?.into(),
                venue_order_id: value_string(value, &["venue_order_id", "order_id"])
                    .unwrap_or_else(|| "—".to_owned())
                    .into(),
                instrument: value_string(value, &["instrument_id", "symbol"])
                    .unwrap_or_else(|| "BTC-PERP".to_owned())
                    .into(),
                direction: if side.eq_ignore_ascii_case("sell") {
                    MarketDirection::Down
                } else {
                    MarketDirection::Up
                },
                price: value_f64(value, &["price", "limit_price"]).unwrap_or_default(),
                remaining,
                status: if value_f64(value, &["filled_qty"]).unwrap_or_default() > 0.0 {
                    OpenOrderStatus::PartiallyFilled
                } else {
                    OpenOrderStatus::Resting
                },
            })
        })
        .collect()
}

fn fill_state(event: &StreamEvent) -> Option<&serde_json::Map<String, serde_json::Value>> {
    match event {
        StreamEvent::Fill { state, .. } => Some(state),
        _ => None,
    }
}

fn fill_activity(event: &StreamEvent) -> Option<ActivityEvent> {
    let state = fill_state(event)?;
    let value = serde_json::Value::Object(state.clone());
    Some(ActivityEvent {
        at_ms: value_f64(&value, &["ts_event", "timestamp"])
            .map(|value| (value / 1_000_000.0) as i64)
            .unwrap_or_default(),
        kind: ActivityEventKind::Order,
        title: value_string(&value, &["instrument_id", "symbol"])
            .unwrap_or_else(|| "Fill".to_owned())
            .into(),
        detail: value_string(&value, &["trade_id", "fill_id"]).map(Into::into),
    })
}

fn fills_from_market(market: &NautilusMarketSnapshot) -> Vec<CandleFill> {
    market
        .recent_fills
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            let state = fill_state(event)?;
            let value = serde_json::Value::Object(state.clone());
            let side = value_string(&value, &["side"]).unwrap_or_default();
            Some(CandleFill {
                fill_id: value_string(&value, &["trade_id", "fill_id"])
                    .unwrap_or_else(|| format!("fill-{index}"))
                    .into(),
                ledger_entry_id: value_string(&value, &["ledger_entry_id"])
                    .unwrap_or_else(|| format!("nautilus-fill-{index}"))
                    .into(),
                time_ms: value_f64(&value, &["ts_event", "timestamp"])
                    .map(|value| (value / 1_000_000.0) as i64)
                    .unwrap_or_default(),
                price: value_f64(&value, &["price"]).unwrap_or_default(),
                quantity: value_f64(&value, &["quantity", "size"]).unwrap_or_default(),
                side: if side.eq_ignore_ascii_case("sell") {
                    FillSide::Sell
                } else {
                    FillSide::Buy
                },
                effect: FillEffect::Entry,
            })
        })
        .collect()
}

fn trade_log_from_market(market: &NautilusMarketSnapshot) -> Vec<TradeLogRow> {
    fills_from_market(market)
        .into_iter()
        .map(|fill| TradeLogRow {
            position_id: fill.fill_id.clone(),
            instrument: "BTC-USD-PERP.HYPERLIQUID".into(),
            side: fill.side,
            realized_pnl_cents: 0,
            duration_ms: 0,
            costs_cents: 0,
            quantity: fill.quantity,
            ledger: ui::LedgerEntryLink {
                ledger_entry_id: fill.ledger_entry_id,
            },
        })
        .collect()
}
