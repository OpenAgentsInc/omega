use std::{collections::BTreeMap, rc::Rc};

use command_center_ui::{
    PredictionCard, PredictionCardData, ReviewTurnCard, ReviewTurnDecision, ReviewTurnValue,
};
use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, Window};
use nautilus_sidecar::{CommandOutcome, CommandReceipt, NautilusMarketSnapshot, StreamEvent};
use plugin_api::CardRendererRegistration;
use serde::Deserialize;
use serde_json::{Value, json};
use ui::prelude::*;
use ui::{
    AttestationState, Candle, CandleSeries, CandlestickChart, ChecksumState, FundingCadence,
    MarketDirection, MarketStats, MarketStatsSource, MarketStatsStrip, MarketTokens,
    OracleAttestation, OracleAttestationReadout, OracleAttestationSource, OrderLifecycle,
    OrderLifecycleSource, OrderLifecycleStage, OrderLifecycleToast, Position, PositionsPanel,
    PositionsSource, PriceTick, PriceTicker, PriceTickerSource, Sparkline, VerificationRow,
    VerificationSource, VerificationTargetKind, VerificationValue,
};

pub const CARD_SCHEMAS: [&str; 8] = [
    "omega.market.candle-lite.v1",
    "omega.market.sparkline.v1",
    "omega.market.stats.v1",
    "omega.market.positions.v1",
    "omega.market.order-lifecycle.v1",
    "omega.market.prediction.v1",
    "omega.market.review-turn.v1",
    "omega.market.oracle-attestation.v1",
];

pub const VERIFICATION_SCHEMA: &str = "omega.market.verification.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveCardKind {
    CandleLite,
    Sparkline,
    Positions,
}

pub fn live_card_payload(snapshot: &NautilusMarketSnapshot, kind: LiveCardKind) -> Option<Value> {
    match kind {
        LiveCardKind::CandleLite => live_candle_payload(snapshot),
        LiveCardKind::Sparkline => live_sparkline_payload(snapshot),
        LiveCardKind::Positions => Some(live_positions_payload(snapshot)),
    }
}

#[derive(Clone, Copy)]
struct LiveCandle {
    time_ms: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

fn live_trades(snapshot: &NautilusMarketSnapshot) -> Vec<(i64, f64, f64)> {
    snapshot
        .recent_trades
        .iter()
        .filter_map(|event| {
            let StreamEvent::Trade {
                price,
                size,
                ts_event,
                ..
            } = event
            else {
                return None;
            };
            let time_ms = i64::try_from(ts_event / 1_000_000).ok()?;
            Some((time_ms, price.parse().ok()?, size.parse().ok()?))
        })
        .filter(|(_, price, size): &(i64, f64, f64)| {
            price.is_finite() && *price > 0.0 && size.is_finite() && *size >= 0.0
        })
        .collect()
}

fn live_candle_payload(snapshot: &NautilusMarketSnapshot) -> Option<Value> {
    let mut candles = BTreeMap::<i64, LiveCandle>::new();
    for (time_ms, price, size) in live_trades(snapshot) {
        let bucket_ms = time_ms.div_euclid(60_000).saturating_mul(60_000);
        candles
            .entry(bucket_ms)
            .and_modify(|candle| {
                candle.high = candle.high.max(price);
                candle.low = candle.low.min(price);
                candle.close = price;
                candle.volume += size;
            })
            .or_insert(LiveCandle {
                time_ms: bucket_ms,
                open: price,
                high: price,
                low: price,
                close: price,
                volume: size,
            });
    }
    if candles.is_empty() {
        return None;
    }
    Some(json!({
        "schema": CARD_SCHEMAS[0],
        "instrument": "BTC-USD-PERP.HYPERLIQUID",
        "price_decimals": 2,
        "candles": candles.into_values().map(|candle| json!({
            "time_ms": candle.time_ms,
            "open": candle.open,
            "high": candle.high,
            "low": candle.low,
            "close": candle.close,
            "volume": candle.volume,
        })).collect::<Vec<_>>(),
        "stream_frame": snapshot.frame_count,
    }))
}

fn live_sparkline_payload(snapshot: &NautilusMarketSnapshot) -> Option<Value> {
    let values = live_trades(snapshot)
        .into_iter()
        .map(|(_, price, _)| price)
        .collect::<Vec<_>>();
    if values.len() < 2 {
        return None;
    }
    Some(json!({
        "schema": CARD_SCHEMAS[1],
        "instrument": "BTC-USD-PERP.HYPERLIQUID",
        "decimals": 2,
        "values": values,
        "stream_frame": snapshot.frame_count,
    }))
}

fn live_positions_payload(snapshot: &NautilusMarketSnapshot) -> Value {
    let positions = snapshot
        .positions
        .iter()
        .filter_map(|position| {
            let position_id = super::find_string(
                position,
                &["position_id", "id", "instrument_id", "instrument"],
            )?;
            let instrument = super::find_string(position, &["instrument_id", "instrument"])?;
            let units = super::find_decimal(position, &["quantity", "signed_qty", "size"])?;
            let units = units.parse::<f64>().ok()?;
            let side =
                super::find_string(position, &["side", "position_side"]).unwrap_or_else(|| {
                    if units < 0.0 {
                        "short".into()
                    } else {
                        "long".into()
                    }
                });
            let entry_price = super::find_decimal(
                position,
                &["avg_px_open", "entry_price", "average_open_price"],
            )?
            .parse::<f64>()
            .ok()?;
            let mark_price = super::find_decimal(position, &["mark_price", "mark_px"])?
                .parse::<f64>()
                .ok()?;
            let liquidation_price =
                super::find_decimal(position, &["liquidation_price", "liquidation_px"])?
                    .parse::<f64>()
                    .ok()?;
            let unrealized_pnl =
                super::find_decimal(position, &["unrealized_pnl", "unrealized_pnl_usd", "pnl"])
                    .and_then(|value| value.parse::<f64>().ok())
                    .unwrap_or(0.0);
            Some(json!({
                "position_id": position_id,
                "instrument": instrument,
                "direction": side,
                "units": units.abs(),
                "entry_price": entry_price,
                "mark_price": mark_price,
                "liquidation_price": liquidation_price,
                "unrealized_pnl": unrealized_pnl,
            }))
        })
        .collect::<Vec<_>>();
    json!({
        "schema": CARD_SCHEMAS[3],
        "network": "testnet",
        "positions": positions,
        "stream_frame": snapshot.frame_count,
    })
}

pub fn order_receipt_payload(receipt: &CommandReceipt, total_units: f64) -> Option<Value> {
    let (order_id, stage) = match &receipt.outcome {
        CommandOutcome::OrderAccepted {
            client_order_id, ..
        } => (client_order_id, "resting"),
        CommandOutcome::OrderCanceled {
            client_order_id, ..
        } => (client_order_id, "cancelled"),
        _ => return None,
    };
    Some(json!({
        "schema": CARD_SCHEMAS[4],
        "network": "testnet",
        "order_id": order_id,
        "stage": stage,
        "filled_units": 0.0,
        "total_units": total_units,
        "receipt": receipt,
    }))
}

type CardRenderer = fn(&Value, &App) -> Option<AnyElement>;

pub fn card_renderer_registrations() -> Vec<CardRendererRegistration> {
    [
        (CARD_SCHEMAS[0], render_candle as CardRenderer),
        (CARD_SCHEMAS[1], render_sparkline as CardRenderer),
        (CARD_SCHEMAS[2], render_market_stats as CardRenderer),
        (CARD_SCHEMAS[3], render_positions as CardRenderer),
        (CARD_SCHEMAS[4], render_order_lifecycle as CardRenderer),
        (CARD_SCHEMAS[5], render_prediction as CardRenderer),
        (CARD_SCHEMAS[6], render_review_turn as CardRenderer),
        (CARD_SCHEMAS[7], render_oracle as CardRenderer),
        (VERIFICATION_SCHEMA, render_verification as CardRenderer),
    ]
    .into_iter()
    .map(|(schema, render)| CardRendererRegistration {
        plugin_id: super::MANIFEST.id,
        schema,
        render: Rc::new(render),
    })
    .collect()
}

#[derive(Deserialize)]
struct CandlePayload {
    #[serde(default = "default_price_decimals")]
    price_decimals: usize,
    candles: Vec<CandleValue>,
}

#[derive(Deserialize)]
struct CandleValue {
    time_ms: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    #[serde(default)]
    volume: f64,
}

fn default_price_decimals() -> usize {
    2
}

fn candle_element(payload: &Value, tokens: Option<MarketTokens>) -> Option<AnyElement> {
    let payload: CandlePayload = serde_json::from_value(payload.clone()).ok()?;
    let candles = payload
        .candles
        .into_iter()
        .filter(|candle| {
            [
                candle.open,
                candle.high,
                candle.low,
                candle.close,
                candle.volume,
            ]
            .into_iter()
            .all(f64::is_finite)
                && candle.high >= candle.low
        })
        .map(|candle| Candle {
            time_ms: candle.time_ms,
            open: candle.open,
            high: candle.high,
            low: candle.low,
            close: candle.close,
            volume: candle.volume,
        })
        .collect::<Vec<_>>();
    if candles.is_empty() {
        return None;
    }
    let mut chart = CandlestickChart::new(CandleSeries::new(candles, payload.price_decimals))
        .size(560.0, 184.0)
        .volume(false);
    if let Some(tokens) = tokens {
        chart = chart.tokens(tokens);
    }
    Some(chart.into_any_element())
}

fn render_candle(payload: &Value, _cx: &App) -> Option<AnyElement> {
    candle_element(payload, None)
}

#[derive(Deserialize)]
struct SparklinePayload {
    values: Vec<f64>,
    #[serde(default = "default_price_decimals")]
    decimals: usize,
}

fn sparkline_element(payload: &Value, tokens: Option<MarketTokens>) -> Option<AnyElement> {
    let payload: SparklinePayload = serde_json::from_value(payload.clone()).ok()?;
    if payload
        .values
        .iter()
        .filter(|value| value.is_finite())
        .count()
        < 2
    {
        return None;
    }
    let mut sparkline = Sparkline::new(payload.values)
        .size(300.0, 48.0)
        .decimals(payload.decimals);
    if let Some(tokens) = tokens {
        sparkline = sparkline.tokens(tokens);
    }
    Some(sparkline.into_any_element())
}

fn render_sparkline(payload: &Value, _cx: &App) -> Option<AnyElement> {
    sparkline_element(payload, None)
}

#[derive(Clone, Deserialize)]
struct TickerValue {
    instrument: String,
    last: f64,
    mark: f64,
    index: f64,
    oracle: f64,
    change_fraction: f64,
    sequence: u64,
}

impl PriceTickerSource for TickerValue {
    fn price_ticks(&self) -> Vec<PriceTick> {
        vec![PriceTick {
            instrument: self.instrument.clone().into(),
            last: self.last,
            mark: self.mark,
            index: self.index,
            oracle: self.oracle,
            change_fraction: self.change_fraction,
            sequence: self.sequence,
        }]
    }
}

#[derive(Clone, Deserialize)]
struct StatsValue {
    venue: String,
    volume_24h: f64,
    open_interest: f64,
    funding_fraction: f64,
    spread_bps: f64,
    funding_anchor_ms: i64,
    now_ms: i64,
    #[serde(default = "default_funding_hours")]
    funding_interval_hours: u8,
}

fn default_funding_hours() -> u8 {
    1
}

impl MarketStatsSource for StatsValue {
    fn market_stats(&self) -> MarketStats {
        MarketStats {
            venue: self.venue.clone().into(),
            volume_24h: self.volume_24h,
            open_interest: self.open_interest,
            funding_fraction: self.funding_fraction,
            spread_bps: self.spread_bps,
            funding_anchor_ms: self.funding_anchor_ms,
            now_ms: self.now_ms,
            funding_cadence: match self.funding_interval_hours {
                1 => FundingCadence::Hourly,
                8 => FundingCadence::ThreeTimesDaily,
                hours => FundingCadence::EveryHours(hours),
            },
        }
    }
}

#[derive(Deserialize)]
struct MarketStatsPayload {
    ticker: TickerValue,
    stats: Option<StatsValue>,
}

fn market_stats_element(payload: &Value, tokens: Option<MarketTokens>) -> Option<AnyElement> {
    let payload: MarketStatsPayload = serde_json::from_value(payload.clone()).ok()?;
    let mut ticker = PriceTicker::from_source(&payload.ticker);
    if let Some(tokens) = tokens {
        ticker = ticker.tokens(tokens);
    }
    let stats = payload.stats.map(|stats| {
        let mut strip = MarketStatsStrip::from_source(&stats);
        if let Some(tokens) = tokens {
            strip = strip.tokens(tokens);
        }
        strip.into_any_element()
    });
    Some(
        v_flex()
            .debug_selector(|| "market.inline_stats".into())
            .w_full()
            .max_w(px(640.0))
            .gap_2()
            .child(ticker)
            .when_some(stats, |this, stats| this.child(stats))
            .into_any_element(),
    )
}

fn render_market_stats(payload: &Value, _cx: &App) -> Option<AnyElement> {
    market_stats_element(payload, None)
}

#[derive(Clone, Deserialize)]
struct PositionValue {
    position_id: String,
    instrument: String,
    direction: String,
    units: f64,
    entry_price: f64,
    mark_price: f64,
    liquidation_price: f64,
    unrealized_pnl: f64,
}

#[derive(Deserialize)]
struct PositionsPayload {
    positions: Vec<PositionValue>,
}

struct InlinePositions(Vec<PositionValue>);

impl PositionsSource for InlinePositions {
    fn positions(&self) -> Vec<Position> {
        self.0
            .iter()
            .filter_map(|position| {
                let direction = match position.direction.as_str() {
                    "up" | "long" | "buy" => MarketDirection::Up,
                    "down" | "short" | "sell" => MarketDirection::Down,
                    "flat" => MarketDirection::Flat,
                    _ => return None,
                };
                Some(Position {
                    position_id: position.position_id.clone().into(),
                    instrument: position.instrument.clone().into(),
                    direction,
                    units: position.units,
                    entry_price: position.entry_price,
                    mark_price: position.mark_price,
                    liquidation_price: position.liquidation_price,
                    unrealized_pnl: position.unrealized_pnl,
                })
            })
            .collect()
    }
}

fn positions_element(payload: &Value, tokens: Option<MarketTokens>) -> Option<AnyElement> {
    let payload: PositionsPayload = serde_json::from_value(payload.clone()).ok()?;
    let source = InlinePositions(payload.positions);
    let mut panel = PositionsPanel::from_source(&source);
    if let Some(tokens) = tokens {
        panel = panel.tokens(tokens);
    }
    Some(panel.into_any_element())
}

fn render_positions(payload: &Value, _cx: &App) -> Option<AnyElement> {
    positions_element(payload, None)
}

#[derive(Clone, Deserialize)]
struct OrderPayload {
    order_id: String,
    stage: String,
    filled_units: f64,
    total_units: f64,
}

impl OrderLifecycleSource for OrderPayload {
    fn order_lifecycle(&self) -> OrderLifecycle {
        let stage = match self.stage.as_str() {
            "placed" => OrderLifecycleStage::Placed,
            "resting" | "accepted" => OrderLifecycleStage::Resting,
            "partially_filled" | "partial" => OrderLifecycleStage::PartiallyFilled,
            "filled" => OrderLifecycleStage::Filled,
            "cancelled" | "canceled" => OrderLifecycleStage::Cancelled,
            _ => OrderLifecycleStage::Placed,
        };
        OrderLifecycle {
            order_id: self.order_id.clone().into(),
            stage,
            filled_units: self.filled_units,
            total_units: self.total_units,
        }
    }
}

fn order_element(payload: &Value, tokens: Option<MarketTokens>) -> Option<AnyElement> {
    let payload: OrderPayload = serde_json::from_value(payload.clone()).ok()?;
    let mut toast = OrderLifecycleToast::from_source(&payload);
    if let Some(tokens) = tokens {
        toast = toast.tokens(tokens);
    }
    Some(toast.into_any_element())
}

fn render_order_lifecycle(payload: &Value, _cx: &App) -> Option<AnyElement> {
    order_element(payload, None)
}

fn render_prediction(payload: &Value, _cx: &App) -> Option<AnyElement> {
    let event: prediction_events::PredictionEvent =
        serde_json::from_value(payload.get("prediction")?.clone()).ok()?;
    Some(
        PredictionCard::new(
            PredictionCardData::from_event(&event, None),
            command_center_ui::unix_now_ms(),
        )
        .into_any_element(),
    )
}

#[derive(Deserialize)]
struct ReviewPayload {
    at_ms: i64,
    trigger: String,
    #[serde(default)]
    read_sources: Vec<String>,
    prediction: Option<String>,
    decision: ReviewDecisionPayload,
    model_id: String,
    input_tokens: u64,
    output_tokens: u64,
    token_cost_microusd: Option<u64>,
    wall_clock_ms: u64,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ReviewDecisionPayload {
    Action { summary: String },
    NoChange,
    Failed { reason: String },
}

fn review_element(payload: &Value, tokens: Option<MarketTokens>) -> Option<AnyElement> {
    let payload: ReviewPayload = serde_json::from_value(payload.clone()).ok()?;
    let decision = match payload.decision {
        ReviewDecisionPayload::Action { summary } => ReviewTurnDecision::Action {
            summary: summary.into(),
        },
        ReviewDecisionPayload::NoChange => ReviewTurnDecision::NoChange,
        ReviewDecisionPayload::Failed { reason } => ReviewTurnDecision::Failed {
            reason: reason.into(),
        },
    };
    let mut card = ReviewTurnCard::new(ReviewTurnValue {
        at_ms: payload.at_ms,
        trigger: payload.trigger.into(),
        read_sources: payload.read_sources.into_iter().map(Into::into).collect(),
        prediction: payload.prediction.map(Into::into),
        decision,
        model_id: payload.model_id.into(),
        input_tokens: payload.input_tokens,
        output_tokens: payload.output_tokens,
        token_cost_microusd: payload.token_cost_microusd,
        wall_clock_ms: payload.wall_clock_ms,
    });
    if let Some(tokens) = tokens {
        card = card.tokens(tokens);
    }
    Some(card.into_any_element())
}

fn render_review_turn(payload: &Value, _cx: &App) -> Option<AnyElement> {
    review_element(payload, None)
}

#[derive(Clone, Deserialize)]
struct OraclePayload {
    oracle: String,
    announced_price: f64,
    attested_price: Option<f64>,
    state: String,
    attested_at_ms: Option<i64>,
}

impl OracleAttestationSource for OraclePayload {
    fn oracle_attestation(&self) -> OracleAttestation {
        OracleAttestation {
            oracle: self.oracle.clone().into(),
            announced_price: self.announced_price,
            attested_price: self.attested_price,
            state: match self.state.as_str() {
                "verified" => AttestationState::Verified,
                "invalid" => AttestationState::Invalid,
                _ => AttestationState::Pending,
            },
            attested_at_ms: self.attested_at_ms,
        }
    }
}

fn oracle_element(payload: &Value, tokens: Option<MarketTokens>) -> Option<AnyElement> {
    let payload: OraclePayload = serde_json::from_value(payload.clone()).ok()?;
    let mut readout = OracleAttestationReadout::from_source(&payload);
    if let Some(tokens) = tokens {
        readout = readout.tokens(tokens);
    }
    Some(readout.into_any_element())
}

fn render_oracle(payload: &Value, _cx: &App) -> Option<AnyElement> {
    oracle_element(payload, None)
}

#[derive(Clone, Deserialize)]
struct VerificationPayload {
    kind: String,
    value: String,
    asset: String,
    network: String,
    checksum: String,
    #[serde(default)]
    revealed: bool,
}

impl VerificationSource for VerificationPayload {
    fn verification(&self) -> VerificationValue {
        VerificationValue {
            kind: if self.kind == "invoice" {
                VerificationTargetKind::Invoice
            } else {
                VerificationTargetKind::Address
            },
            value: self.value.clone().into(),
            asset: self.asset.clone().into(),
            network: self.network.clone().into(),
            checksum: match self.checksum.as_str() {
                "verified" => ChecksumState::Verified,
                "invalid" => ChecksumState::Invalid,
                _ => ChecksumState::NotAvailable,
            },
        }
    }
}

fn verification_element(payload: &Value, tokens: Option<MarketTokens>) -> Option<AnyElement> {
    let payload: VerificationPayload = serde_json::from_value(payload.clone()).ok()?;
    let mut row = VerificationRow::from_source(&payload).revealed(payload.revealed);
    if let Some(tokens) = tokens {
        row = row.tokens(tokens);
    }
    Some(row.into_any_element())
}

fn render_verification(payload: &Value, _cx: &App) -> Option<AnyElement> {
    verification_element(payload, None)
}

fn demo_payloads() -> Vec<Value> {
    vec![
        json!({
            "schema": CARD_SCHEMAS[0],
            "price_decimals": 2,
            "candles": (0..24).map(|index| {
                let open = 116_000.0 + f64::from(index) * 18.0;
                let close = open + (f64::from(index) / 2.0).sin() * 42.0;
                json!({
                    "time_ms": 1_786_276_800_000_i64 + i64::from(index) * 60_000,
                    "open": open,
                    "high": open.max(close) + 22.0,
                    "low": open.min(close) - 18.0,
                    "close": close,
                    "volume": 12.0 + f64::from(index),
                })
            }).collect::<Vec<_>>(),
        }),
        json!({
            "schema": CARD_SCHEMAS[1],
            "decimals": 2,
            "values": (0..48).map(|index| 116_000.0 + f64::from(index) * 5.0 + (f64::from(index) / 4.0).sin() * 38.0).collect::<Vec<_>>(),
        }),
        json!({
            "schema": CARD_SCHEMAS[2],
            "ticker": {
                "instrument": "BTC-PERP", "last": 116_420.0, "mark": 116_418.5,
                "index": 116_401.2, "oracle": 116_407.8, "change_fraction": 0.0184,
                "sequence": 42,
            },
            "stats": {
                "venue": "Hyperliquid", "volume_24h": 4_830_000_000.0,
                "open_interest": 1_240_000_000.0, "funding_fraction": 0.000117,
                "spread_bps": 0.8, "funding_anchor_ms": 0,
                "now_ms": 1_786_276_800_000_i64, "funding_interval_hours": 1,
            },
        }),
        json!({
            "schema": CARD_SCHEMAS[3],
            "positions": [{
                "position_id": "position-btc-1", "instrument": "BTC-PERP",
                "direction": "long", "units": 0.08, "entry_price": 114_200.0,
                "mark_price": 116_420.0, "liquidation_price": 82_800.0,
                "unrealized_pnl": 177.60,
            }],
        }),
        json!({
            "schema": CARD_SCHEMAS[4], "order_id": "O-OMEGA-302-1",
            "stage": "partially_filled", "filled_units": 0.05, "total_units": 0.08,
        }),
        json!({
            "schema": CARD_SCHEMAS[5],
            "prediction": {
                "sequence": 12, "prediction_id": "pred-demo", "schema_version": 1,
                "emitted_at_ms": 1_786_276_800_000_i64,
                "actor": { "type": "agent", "agent_id": "trading-session" },
                "mandate_scope": { "venue": "hyperliquid", "network": "testnet" },
                "instrument": "BTC-PERP",
                "forecast": { "type": "directional", "direction": "up", "probability_micros": 720000 },
                "confidence_micros": 720000, "horizon_ms": 3_600_000_u64,
                "resolution_rule": {
                    "source": "nautilus.testnet.quote.v1", "baseline_at_ms": 1_786_276_800_000_i64,
                    "resolve_at_ms": 1_786_280_400_000_i64, "flat_tolerance_bps": 10,
                },
                "scoring_rule": "brier", "observation_refs": [],
                "private_payload_ref": Value::Null, "subsequent_decision_id": "decision-demo",
            },
        }),
        json!({
            "schema": CARD_SCHEMAS[6], "at_ms": 1_786_276_800_000_i64,
            "trigger": "funding sign flip", "read_sources": ["features", "ledger", "mandate"],
            "prediction": "BTC-PERP down · 64% · 8h",
            "decision": { "type": "action", "summary": "reduce carry target" },
            "model_id": "claude-sonnet-4", "input_tokens": 704, "output_tokens": 126,
            "token_cost_microusd": 5912, "wall_clock_ms": 2840,
        }),
        json!({
            "schema": CARD_SCHEMAS[7], "oracle": "Pyth BTC/USD",
            "announced_price": 116420.50, "attested_price": 116419.92,
            "state": "verified", "attested_at_ms": 1_786_276_800_000_i64,
        }),
        json!({
            "schema": VERIFICATION_SCHEMA, "kind": "invoice",
            "value": "lntb250u1pomega302pp5qexampleinvoiceforinlineverification",
            "asset": "BTC", "network": "testnet", "checksum": "verified",
        }),
    ]
}

fn renderer_for(schema: &str) -> Option<CardRendererRegistration> {
    card_renderer_registrations()
        .into_iter()
        .find(|renderer| renderer.schema == schema)
}

#[derive(RegisterComponent)]
pub struct InlineMarketCardsPreview;

impl Component for InlineMarketCardsPreview {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        "Schema-dispatched market cards rendered in the transcript with live-component shapes."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let normal = demo_payloads()
            .into_iter()
            .filter_map(|payload| {
                let schema = payload.get("schema")?.as_str()?;
                let renderer = renderer_for(schema)?;
                (renderer.render)(&payload, cx)
            })
            .collect::<Vec<_>>();
        let grayscale_tokens = MarketTokens::from_theme(cx).grayscale();
        let grayscale = demo_payloads()
            .into_iter()
            .filter_map(|payload| {
                let schema = payload.get("schema")?.as_str()?;
                match schema {
                    "omega.market.candle-lite.v1" => {
                        candle_element(&payload, Some(grayscale_tokens))
                    }
                    "omega.market.sparkline.v1" => {
                        sparkline_element(&payload, Some(grayscale_tokens))
                    }
                    "omega.market.stats.v1" => {
                        market_stats_element(&payload, Some(grayscale_tokens))
                    }
                    "omega.market.positions.v1" => {
                        positions_element(&payload, Some(grayscale_tokens))
                    }
                    "omega.market.order-lifecycle.v1" => {
                        order_element(&payload, Some(grayscale_tokens))
                    }
                    "omega.market.review-turn.v1" => {
                        review_element(&payload, Some(grayscale_tokens))
                    }
                    "omega.market.oracle-attestation.v1" => {
                        oracle_element(&payload, Some(grayscale_tokens))
                    }
                    "omega.market.verification.v1" => {
                        verification_element(&payload, Some(grayscale_tokens))
                    }
                    "omega.market.prediction.v1" => render_prediction(&payload, cx),
                    _ => None,
                }
            })
            .collect::<Vec<_>>();
        v_flex()
            .gap_6()
            .child(example_group_with_title(
                "Inline market transcript",
                vec![single_example(
                    "Every schema through the MarketChatDemo harness",
                    ui::market_chat_inline_demo(normal, cx),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Signs, glyphs, geometry, and labels preserve state",
                    ui::market_chat_inline_demo(grayscale, cx),
                )],
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gpui::{Context, IntoElement, Render, TestAppContext};
    use nautilus_sidecar::{CommandType, Network};

    use super::*;

    #[test]
    fn every_demo_payload_has_one_exact_renderer() {
        let registrations = card_renderer_registrations();
        let schemas = registrations
            .iter()
            .map(|registration| registration.schema)
            .collect::<BTreeSet<_>>();
        assert_eq!(schemas.len(), registrations.len());
        for payload in demo_payloads() {
            let schema = payload
                .get("schema")
                .and_then(Value::as_str)
                .expect("demo schema");
            assert!(schemas.contains(schema), "missing renderer for {schema}");
        }
    }

    #[test]
    fn malformed_payloads_fail_closed() {
        assert!(candle_element(&json!({"candles": []}), None).is_none());
        assert!(sparkline_element(&json!({"values": [1.0]}), None).is_none());
    }

    #[test]
    fn live_stream_snapshot_produces_versioned_cards() {
        let mut snapshot = NautilusMarketSnapshot::default();
        snapshot.frame_count = 9;
        snapshot.recent_trades = vec![
            StreamEvent::Trade {
                schema: "omega.nautilus.stream.v1".into(),
                generation: 1,
                sequence: 1,
                network: Network::Testnet,
                instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
                price: "116400".into(),
                size: "0.01".into(),
                aggressor_side: "buyer".into(),
                trade_id: "trade-1".into(),
                ts_event: 1_786_276_800_000_000_000,
                ts_init: 1_786_276_800_000_000_000,
            },
            StreamEvent::Trade {
                schema: "omega.nautilus.stream.v1".into(),
                generation: 1,
                sequence: 2,
                network: Network::Testnet,
                instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
                price: "116420".into(),
                size: "0.02".into(),
                aggressor_side: "seller".into(),
                trade_id: "trade-2".into(),
                ts_event: 1_786_276_860_000_000_000,
                ts_init: 1_786_276_860_000_000_000,
            },
        ];
        for (kind, schema) in [
            (LiveCardKind::CandleLite, CARD_SCHEMAS[0]),
            (LiveCardKind::Sparkline, CARD_SCHEMAS[1]),
            (LiveCardKind::Positions, CARD_SCHEMAS[3]),
        ] {
            let payload = live_card_payload(&snapshot, kind).expect("live card payload");
            assert_eq!(payload["schema"], schema);
            assert_eq!(payload["stream_frame"], 9);
        }
    }

    #[test]
    fn accepted_and_cancelled_receipts_map_to_lifecycle_stages() {
        let accepted = CommandReceipt {
            command_id: "place-1".into(),
            command_type: CommandType::PlaceOrder,
            acknowledged: true,
            sent: true,
            outcome: CommandOutcome::OrderAccepted {
                client_order_id: "client-1".into(),
                venue_order_id: "venue-1".into(),
            },
        };
        assert_eq!(
            order_receipt_payload(&accepted, 0.08)
                .and_then(|payload| payload.get("stage").cloned()),
            Some(json!("resting"))
        );
        let cancelled = CommandReceipt {
            command_id: "cancel-1".into(),
            command_type: CommandType::CancelOrder,
            acknowledged: true,
            sent: true,
            outcome: CommandOutcome::OrderCanceled {
                client_order_id: "client-1".into(),
                venue_order_id: "venue-1".into(),
            },
        };
        assert_eq!(
            order_receipt_payload(&cancelled, 0.0)
                .and_then(|payload| payload.get("stage").cloned()),
            Some(json!("cancelled"))
        );
    }

    struct InlineCardsTestView;

    impl Render for InlineCardsTestView {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            InlineMarketCardsPreview::preview(window, cx)
        }
    }

    #[gpui::test]
    async fn every_inline_card_preview_paints(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
        let (_view, cx) = cx.add_window_view(|_, _| InlineCardsTestView);
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();

        let rendered = cx.debug_render_snapshot();
        for selector in [
            "market.sparkline",
            "market.price_ticker",
            "market.stats_strip",
            "market.positions",
            "market.order_lifecycle",
            "command_center.prediction_card",
            "command_center.review_turn",
            "market.oracle_attestation",
            "market.verification_row",
        ] {
            assert!(
                !rendered.occurrences(selector).is_empty(),
                "inline market card preview did not paint {selector}"
            );
        }
    }
}
