use std::{str::FromStr as _, sync::Arc};

use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use chrono::DateTime;
use lnmarkets_client::{
    Asset, DecimalAmount, FuturesIsolatedAmountRequest, FuturesIsolatedNewTradeRequest,
    FuturesIsolatedStoplossUpdate, FuturesIsolatedTrade, FuturesIsolatedTradeReference,
    FuturesIsolatedTradeSize, FuturesLeverage, FuturesTradeId, FuturesTradeSide, LnMarketsClient,
    Network, NewSwapRequest, Pagination,
};
use lnmarkets_data::{FeatureSnapshot, MarketDataStore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use strategy_engine::{
    BacktestExecutionModel, BacktestTick, IntentPrediction, OrderIntent, OrderKind, OrderQuantity,
    OrderSide, QuantityUnit, SimulatedSettlement, SimulatedTrade, StrategyProgram, StrategyStep,
    StrategyTick, VenueExecution, VenueExecutor, VenueProtection, VenueRiskSnapshot,
};
use trading_ledger::{
    AssetId, LedgerAccount, LedgerEntryDraft, LedgerEntryKind, LedgerPosting, LedgerStore,
};
use trading_mandate::TradingNetwork;

pub const FUNDING_CARRY_SCHEMA: &str = "omega.lnmarkets.funding_carry.v1";
const STRATEGY_ID: &str = "funding_carry";
const STRATEGY_VERSION: &str = "1";
const ISOLATED_INSTRUMENT: &str = "lnmarkets.futures.isolated";
const SYNTHETIC_USD_INSTRUMENT: &str = "lnmarkets.synthetic_usd";
const ISOLATED_STREAM_TOPIC: &str = "futures/inverse/btc_usd/isolated/trades";
const SATOSHIS_PER_BITCOIN: f64 = 100_000_000.0;
const BASIS_POINTS_DENOMINATOR: u64 = 10_000;
const PARTS_PER_MILLION_PER_BASIS_POINT: u64 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingCarryInstrument {
    SyntheticUsd,
    IsolatedFuture,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FundingCarryConfig {
    pub network: Network,
    pub instrument: FundingCarryInstrument,
    pub entry_funding_rate_ppm: i32,
    pub exit_funding_rate_ppm: i32,
    pub expected_settlements: u32,
    pub measured_round_trip_cost_bps: u32,
    pub cost_margin_bps: u32,
    pub maximum_hedge_notional_usd: u64,
    pub notional_per_funding_ppm_usd: u64,
    pub leverage: u8,
    pub stop_loss_distance_bps: u32,
    pub liquidation_buffer_floor_bps: u32,
    pub margin_top_up_sats: u64,
    pub profit_sweep_threshold_sats: u64,
    pub profit_sweep_sats: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FundingCarryPosition {
    SyntheticUsd {
        notional_usd_cents: u64,
    },
    IsolatedFuture {
        trade_id: String,
        side: String,
        notional_usd: u64,
        margin_sats: u64,
        leverage: u8,
        liquidation_price: f64,
        liquidation_buffer_bps: u32,
        unrealized_profit_sats: i64,
        stop_loss_price: Option<f64>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FundingCarryFeatures {
    pub market: FeatureSnapshot,
    pub position: Option<FundingCarryPosition>,
    pub settled_funding_sats: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FundingCarryAction {
    WaitingForFunding,
    FundingBelowEntry,
    CostHurdleNotMet,
    WaitingForAccountData,
    WaitingForPrice,
    Holding,
    OpenShort { notional_usd: u64 },
    IncreaseSyntheticUsd { amount_sats: u64 },
    CloseShort { trade_id: String },
    ReduceSyntheticUsd { amount_usd_cents: u64 },
    InstallVenueStop { trade_id: String },
    AddMargin { trade_id: String, amount_sats: u64 },
    CashInProfit { trade_id: String, amount_sats: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FundingCarryState {
    pub schema: String,
    pub sequence: u64,
    pub instrument: FundingCarryInstrument,
    pub funding_rate_ppm: Option<i32>,
    pub funding_ema_ppm: Option<i32>,
    pub target_notional_usd: u64,
    pub position_notional_usd: u64,
    pub liquidation_buffer_bps: Option<u32>,
    pub venue_stop_active: bool,
    pub funding_attributed_sats: i64,
    pub last_action: FundingCarryAction,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FundingCarryProgram;

impl StrategyProgram for FundingCarryProgram {
    type Config = FundingCarryConfig;
    type State = FundingCarryState;
    type Features = FundingCarryFeatures;

    fn strategy_id(&self) -> &'static str {
        STRATEGY_ID
    }

    fn strategy_version(&self) -> &'static str {
        STRATEGY_VERSION
    }

    fn validate_config(&self, config: &Self::Config) -> Result<()> {
        if config.network != Network::Signet {
            bail!("funding_carry is restricted to signet");
        }
        if config.entry_funding_rate_ppm <= 0 {
            bail!("funding carry entry rate must be positive");
        }
        if config.exit_funding_rate_ppm >= config.entry_funding_rate_ppm {
            bail!("funding carry exit rate must be below its entry rate");
        }
        if config.expected_settlements == 0 {
            bail!("funding carry expected settlements must be greater than zero");
        }
        let cost_hurdle_bps = config
            .measured_round_trip_cost_bps
            .checked_add(config.cost_margin_bps)
            .context("funding carry cost hurdle overflowed")?;
        if cost_hurdle_bps > 10_000 {
            bail!("funding carry cost hurdle must not exceed 10000 basis points");
        }
        if config.maximum_hedge_notional_usd == 0 || config.notional_per_funding_ppm_usd == 0 {
            bail!("funding carry sizing values must be greater than zero");
        }
        if !(1..=100).contains(&config.leverage) {
            bail!("funding carry leverage must be from 1 through 100");
        }
        if !(1..10_000).contains(&config.stop_loss_distance_bps) {
            bail!("funding carry stop-loss distance must be from 1 through 9999 basis points");
        }
        if config.liquidation_buffer_floor_bps == 0 || config.liquidation_buffer_floor_bps > 10_000
        {
            bail!(
                "funding carry liquidation buffer floor must be from 1 through 10000 basis points"
            );
        }
        if config.margin_top_up_sats == 0 {
            bail!("funding carry margin top-up must be greater than zero");
        }
        if config.profit_sweep_sats == 0
            || config.profit_sweep_sats > config.profit_sweep_threshold_sats
        {
            bail!("funding carry profit sweep must be positive and not exceed its threshold");
        }
        Ok(())
    }

    fn initial_state(&self, config: &Self::Config) -> Result<Self::State> {
        self.validate_config(config)?;
        Ok(FundingCarryState {
            schema: FUNDING_CARRY_SCHEMA.into(),
            sequence: 0,
            instrument: config.instrument,
            funding_rate_ppm: None,
            funding_ema_ppm: None,
            target_notional_usd: 0,
            position_notional_usd: 0,
            liquidation_buffer_bps: None,
            venue_stop_active: config.instrument == FundingCarryInstrument::SyntheticUsd,
            funding_attributed_sats: 0,
            last_action: FundingCarryAction::WaitingForFunding,
        })
    }

    fn on_tick(
        &self,
        config: &Self::Config,
        state: &Self::State,
        tick: &StrategyTick<Self::Features>,
    ) -> Result<StrategyStep<Self::State>> {
        self.validate_config(config)?;
        validate_position_instrument(config.instrument, tick.features.position.as_ref())?;
        let sequence = state
            .sequence
            .checked_add(1)
            .context("funding carry sequence overflowed")?;
        let funding_attributed_sats = state
            .funding_attributed_sats
            .checked_add(tick.features.settled_funding_sats)
            .context("funding carry attributed funding overflowed")?;
        let current_rate_ppm = tick
            .features
            .market
            .funding
            .current_rate
            .map(rate_to_ppm)
            .transpose()?;
        let ema_rate_ppm = tick
            .features
            .market
            .funding
            .ema
            .map(rate_to_ppm)
            .transpose()?;
        let signal_ppm = ema_rate_ppm.or(current_rate_ppm);
        let Some(signal_ppm) = signal_ppm else {
            return Ok(no_intent_step(
                config,
                sequence,
                current_rate_ppm,
                ema_rate_ppm,
                0,
                tick.features.position.as_ref(),
                funding_attributed_sats,
                FundingCarryAction::WaitingForFunding,
            ));
        };
        let target_notional_usd = target_notional(config, signal_ppm)?;

        if let Some(position) = tick.features.position.as_ref() {
            if signal_ppm <= config.exit_funding_rate_ppm {
                return close_position_step(
                    config,
                    sequence,
                    current_rate_ppm,
                    ema_rate_ppm,
                    target_notional_usd,
                    position,
                    funding_attributed_sats,
                    tick.occurred_at_ms,
                );
            }
            if let FundingCarryPosition::IsolatedFuture {
                trade_id,
                stop_loss_price,
                liquidation_buffer_bps,
                unrealized_profit_sats,
                ..
            } = position
            {
                if stop_loss_price.is_none() {
                    return install_stop_step(
                        config,
                        sequence,
                        current_rate_ppm,
                        ema_rate_ppm,
                        target_notional_usd,
                        position,
                        funding_attributed_sats,
                        trade_id,
                        tick,
                    );
                }
                if *liquidation_buffer_bps < config.liquidation_buffer_floor_bps {
                    return management_step(
                        config,
                        sequence,
                        current_rate_ppm,
                        ema_rate_ppm,
                        target_notional_usd,
                        position,
                        funding_attributed_sats,
                        FundingCarryAction::AddMargin {
                            trade_id: trade_id.clone(),
                            amount_sats: config.margin_top_up_sats,
                        },
                        "add_margin",
                        trade_id,
                        config.margin_top_up_sats,
                        tick.occurred_at_ms,
                    );
                }
                if *unrealized_profit_sats
                    >= i64::try_from(config.profit_sweep_threshold_sats)
                        .context("funding carry sweep threshold exceeded signed range")?
                {
                    return management_step(
                        config,
                        sequence,
                        current_rate_ppm,
                        ema_rate_ppm,
                        target_notional_usd,
                        position,
                        funding_attributed_sats,
                        FundingCarryAction::CashInProfit {
                            trade_id: trade_id.clone(),
                            amount_sats: config.profit_sweep_sats,
                        },
                        "cash_in",
                        trade_id,
                        config.profit_sweep_sats,
                        tick.occurred_at_ms,
                    );
                }
            }
            return Ok(no_intent_step(
                config,
                sequence,
                current_rate_ppm,
                ema_rate_ppm,
                target_notional_usd,
                Some(position),
                funding_attributed_sats,
                FundingCarryAction::Holding,
            ));
        }

        if signal_ppm < config.entry_funding_rate_ppm {
            return Ok(no_intent_step(
                config,
                sequence,
                current_rate_ppm,
                ema_rate_ppm,
                target_notional_usd,
                None,
                funding_attributed_sats,
                FundingCarryAction::FundingBelowEntry,
            ));
        }
        if !clears_cost_hurdle(config, signal_ppm)? {
            return Ok(no_intent_step(
                config,
                sequence,
                current_rate_ppm,
                ema_rate_ppm,
                target_notional_usd,
                None,
                funding_attributed_sats,
                FundingCarryAction::CostHurdleNotMet,
            ));
        }
        open_position_step(
            config,
            sequence,
            current_rate_ppm,
            ema_rate_ppm,
            target_notional_usd,
            funding_attributed_sats,
            tick,
        )
    }
}

fn open_position_step(
    config: &FundingCarryConfig,
    sequence: u64,
    current_rate_ppm: Option<i32>,
    ema_rate_ppm: Option<i32>,
    target_notional_usd: u64,
    funding_attributed_sats: i64,
    tick: &StrategyTick<FundingCarryFeatures>,
) -> Result<StrategyStep<FundingCarryState>> {
    let (intent, action) = match config.instrument {
        FundingCarryInstrument::SyntheticUsd => {
            let Some(bid_price) = tick.features.market.liquidity.best_bid else {
                return Ok(no_intent_step(
                    config,
                    sequence,
                    current_rate_ppm,
                    ema_rate_ppm,
                    target_notional_usd,
                    None,
                    funding_attributed_sats,
                    FundingCarryAction::WaitingForPrice,
                ));
            };
            let Some(account) = tick.features.market.account_drift.as_ref() else {
                return Ok(no_intent_step(
                    config,
                    sequence,
                    current_rate_ppm,
                    ema_rate_ppm,
                    target_notional_usd,
                    None,
                    funding_attributed_sats,
                    FundingCarryAction::WaitingForAccountData,
                ));
            };
            validate_price(bid_price, "funding carry bid price")?;
            let amount_sats = floor_u64(
                f64::min(account.btc_value_usd, target_notional_usd as f64) / bid_price
                    * SATOSHIS_PER_BITCOIN,
            )?;
            if amount_sats == 0 {
                return Ok(no_intent_step(
                    config,
                    sequence,
                    current_rate_ppm,
                    ema_rate_ppm,
                    target_notional_usd,
                    None,
                    funding_attributed_sats,
                    FundingCarryAction::WaitingForAccountData,
                ));
            }
            (
                strategy_intent(
                    sequence,
                    tick.occurred_at_ms,
                    SYNTHETIC_USD_INSTRUMENT,
                    OrderSide::Sell,
                    OrderQuantity {
                        amount: amount_sats,
                        unit: QuantityUnit::Sats,
                    },
                    false,
                    None,
                    json!({"operation": "increase_synthetic_usd"}),
                )?,
                FundingCarryAction::IncreaseSyntheticUsd { amount_sats },
            )
        }
        FundingCarryInstrument::IsolatedFuture => {
            let price = tick
                .features
                .market
                .liquidity
                .best_ask
                .or(tick.features.market.liquidity.best_bid)
                .context("funding carry has no current price for its venue stop")?;
            validate_price(price, "funding carry current price")?;
            let stop_loss_price =
                price * (1.0 + f64::from(config.stop_loss_distance_bps) / 10_000.0);
            let protection = VenueProtection {
                stop_loss_price,
                take_profit_price: None,
            };
            (
                strategy_intent(
                    sequence,
                    tick.occurred_at_ms,
                    ISOLATED_INSTRUMENT,
                    OrderSide::Sell,
                    OrderQuantity {
                        amount: target_notional_usd,
                        unit: QuantityUnit::UsdNotional,
                    },
                    false,
                    Some(protection),
                    json!({
                        "operation": "open_short",
                        "leverage": config.leverage,
                        "projected_liquidation_buffer_bps": config.liquidation_buffer_floor_bps,
                    }),
                )?,
                FundingCarryAction::OpenShort {
                    notional_usd: target_notional_usd,
                },
            )
        }
    };
    Ok(StrategyStep {
        cancels: Vec::new(),
        next_state: next_state(
            config,
            sequence,
            current_rate_ppm,
            ema_rate_ppm,
            target_notional_usd,
            None,
            funding_attributed_sats,
            action,
        ),
        intents: vec![intent],
    })
}

fn close_position_step(
    config: &FundingCarryConfig,
    sequence: u64,
    current_rate_ppm: Option<i32>,
    ema_rate_ppm: Option<i32>,
    target_notional_usd: u64,
    position: &FundingCarryPosition,
    funding_attributed_sats: i64,
    occurred_at_ms: i64,
) -> Result<StrategyStep<FundingCarryState>> {
    let (intent, action) = match position {
        FundingCarryPosition::SyntheticUsd { notional_usd_cents } => (
            strategy_intent(
                sequence,
                occurred_at_ms,
                SYNTHETIC_USD_INSTRUMENT,
                OrderSide::Buy,
                OrderQuantity {
                    amount: *notional_usd_cents,
                    unit: QuantityUnit::UsdCents,
                },
                true,
                None,
                json!({"operation": "reduce_synthetic_usd"}),
            )?,
            FundingCarryAction::ReduceSyntheticUsd {
                amount_usd_cents: *notional_usd_cents,
            },
        ),
        FundingCarryPosition::IsolatedFuture {
            trade_id,
            notional_usd,
            ..
        } => (
            strategy_intent(
                sequence,
                occurred_at_ms,
                ISOLATED_INSTRUMENT,
                OrderSide::Buy,
                OrderQuantity {
                    amount: *notional_usd,
                    unit: QuantityUnit::UsdNotional,
                },
                true,
                None,
                json!({"operation": "close_short", "trade_id": trade_id}),
            )?,
            FundingCarryAction::CloseShort {
                trade_id: trade_id.clone(),
            },
        ),
    };
    Ok(StrategyStep {
        cancels: Vec::new(),
        next_state: next_state(
            config,
            sequence,
            current_rate_ppm,
            ema_rate_ppm,
            target_notional_usd,
            Some(position),
            funding_attributed_sats,
            action,
        ),
        intents: vec![intent],
    })
}

fn install_stop_step(
    config: &FundingCarryConfig,
    sequence: u64,
    current_rate_ppm: Option<i32>,
    ema_rate_ppm: Option<i32>,
    target_notional_usd: u64,
    position: &FundingCarryPosition,
    funding_attributed_sats: i64,
    trade_id: &str,
    tick: &StrategyTick<FundingCarryFeatures>,
) -> Result<StrategyStep<FundingCarryState>> {
    let price = tick
        .features
        .market
        .liquidity
        .best_ask
        .or(tick.features.market.liquidity.best_bid)
        .context("funding carry has no current price for its venue stop")?;
    validate_price(price, "funding carry current price")?;
    let stop_loss_price = price * (1.0 + f64::from(config.stop_loss_distance_bps) / 10_000.0);
    let action = FundingCarryAction::InstallVenueStop {
        trade_id: trade_id.to_owned(),
    };
    let intent = strategy_intent(
        sequence,
        tick.occurred_at_ms,
        ISOLATED_INSTRUMENT,
        OrderSide::Buy,
        OrderQuantity {
            amount: 1,
            unit: QuantityUnit::Sats,
        },
        true,
        Some(VenueProtection {
            stop_loss_price,
            take_profit_price: None,
        }),
        json!({"operation": "install_stop", "trade_id": trade_id}),
    )?;
    Ok(StrategyStep {
        cancels: Vec::new(),
        next_state: next_state(
            config,
            sequence,
            current_rate_ppm,
            ema_rate_ppm,
            target_notional_usd,
            Some(position),
            funding_attributed_sats,
            action,
        ),
        intents: vec![intent],
    })
}

#[allow(clippy::too_many_arguments)]
fn management_step(
    config: &FundingCarryConfig,
    sequence: u64,
    current_rate_ppm: Option<i32>,
    ema_rate_ppm: Option<i32>,
    target_notional_usd: u64,
    position: &FundingCarryPosition,
    funding_attributed_sats: i64,
    action: FundingCarryAction,
    operation: &str,
    trade_id: &str,
    amount_sats: u64,
    occurred_at_ms: i64,
) -> Result<StrategyStep<FundingCarryState>> {
    let intent = strategy_intent(
        sequence,
        occurred_at_ms,
        ISOLATED_INSTRUMENT,
        OrderSide::Buy,
        OrderQuantity {
            amount: amount_sats,
            unit: QuantityUnit::Sats,
        },
        true,
        None,
        json!({"operation": operation, "trade_id": trade_id}),
    )?;
    Ok(StrategyStep {
        cancels: Vec::new(),
        next_state: next_state(
            config,
            sequence,
            current_rate_ppm,
            ema_rate_ppm,
            target_notional_usd,
            Some(position),
            funding_attributed_sats,
            action,
        ),
        intents: vec![intent],
    })
}

#[allow(clippy::too_many_arguments)]
fn strategy_intent(
    sequence: u64,
    occurred_at_ms: i64,
    instrument: &str,
    side: OrderSide,
    quantity: OrderQuantity,
    reduce_only: bool,
    protection: Option<VenueProtection>,
    operation: Value,
) -> Result<OrderIntent> {
    let operation = operation
        .as_object()
        .context("funding carry operation metadata must be an object")?;
    let mut metadata = serde_json::Map::new();
    metadata.insert("schema".into(), Value::String(FUNDING_CARRY_SCHEMA.into()));
    metadata.insert("occurred_at_ms".into(), Value::from(occurred_at_ms));
    metadata.extend(operation.clone());
    let intent = OrderIntent {
        intent_id: format!("{STRATEGY_ID}:{occurred_at_ms}:{sequence}"),
        instrument: instrument.into(),
        side,
        kind: OrderKind::Market,
        quantity,
        limit_price: None,
        reduce_only,
        protection,
        prediction: (!reduce_only).then(|| IntentPrediction {
            confidence_micros: 600_000,
            horizon_ms: 60 * 60 * 1_000,
            resolution_source: "lnmarkets:stored_last_price".into(),
            flat_tolerance_bps: 10,
            observation_refs: vec![format!("lnmarkets:features:{occurred_at_ms}")],
            private_payload_ref: None,
        }),
        metadata: Value::Object(metadata),
    };
    intent.validate()?;
    Ok(intent)
}

#[allow(clippy::too_many_arguments)]
fn no_intent_step(
    config: &FundingCarryConfig,
    sequence: u64,
    current_rate_ppm: Option<i32>,
    ema_rate_ppm: Option<i32>,
    target_notional_usd: u64,
    position: Option<&FundingCarryPosition>,
    funding_attributed_sats: i64,
    action: FundingCarryAction,
) -> StrategyStep<FundingCarryState> {
    StrategyStep {
        cancels: Vec::new(),
        next_state: next_state(
            config,
            sequence,
            current_rate_ppm,
            ema_rate_ppm,
            target_notional_usd,
            position,
            funding_attributed_sats,
            action,
        ),
        intents: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn next_state(
    config: &FundingCarryConfig,
    sequence: u64,
    current_rate_ppm: Option<i32>,
    ema_rate_ppm: Option<i32>,
    target_notional_usd: u64,
    position: Option<&FundingCarryPosition>,
    funding_attributed_sats: i64,
    action: FundingCarryAction,
) -> FundingCarryState {
    let (position_notional_usd, liquidation_buffer_bps, venue_stop_active) = match position {
        Some(FundingCarryPosition::SyntheticUsd { notional_usd_cents }) => {
            (notional_usd_cents / 100, None, true)
        }
        Some(FundingCarryPosition::IsolatedFuture {
            notional_usd,
            liquidation_buffer_bps,
            stop_loss_price,
            ..
        }) => (
            *notional_usd,
            Some(*liquidation_buffer_bps),
            stop_loss_price.is_some(),
        ),
        None => (
            0,
            None,
            config.instrument == FundingCarryInstrument::SyntheticUsd,
        ),
    };
    FundingCarryState {
        schema: FUNDING_CARRY_SCHEMA.into(),
        sequence,
        instrument: config.instrument,
        funding_rate_ppm: current_rate_ppm,
        funding_ema_ppm: ema_rate_ppm,
        target_notional_usd,
        position_notional_usd,
        liquidation_buffer_bps,
        venue_stop_active,
        funding_attributed_sats,
        last_action: action,
    }
}

fn target_notional(config: &FundingCarryConfig, rate_ppm: i32) -> Result<u64> {
    if rate_ppm <= 0 {
        return Ok(0);
    }
    u64::try_from(rate_ppm)
        .context("funding carry rate exceeded unsigned range")?
        .checked_mul(config.notional_per_funding_ppm_usd)
        .map(|notional| notional.min(config.maximum_hedge_notional_usd))
        .context("funding carry target notional overflowed")
}

fn clears_cost_hurdle(config: &FundingCarryConfig, rate_ppm: i32) -> Result<bool> {
    if rate_ppm <= 0 {
        return Ok(false);
    }
    let expected_carry_ppm = u64::try_from(rate_ppm)
        .context("funding carry rate exceeded unsigned range")?
        .checked_mul(u64::from(config.expected_settlements))
        .context("funding carry expected return overflowed")?;
    let hurdle_bps = config
        .measured_round_trip_cost_bps
        .checked_add(config.cost_margin_bps)
        .context("funding carry cost hurdle overflowed")?;
    let hurdle_ppm = u64::from(hurdle_bps)
        .checked_mul(PARTS_PER_MILLION_PER_BASIS_POINT)
        .context("funding carry cost conversion overflowed")?;
    Ok(expected_carry_ppm > hurdle_ppm)
}

fn validate_position_instrument(
    instrument: FundingCarryInstrument,
    position: Option<&FundingCarryPosition>,
) -> Result<()> {
    match (instrument, position) {
        (
            FundingCarryInstrument::SyntheticUsd,
            Some(FundingCarryPosition::IsolatedFuture { .. }),
        )
        | (
            FundingCarryInstrument::IsolatedFuture,
            Some(FundingCarryPosition::SyntheticUsd { .. }),
        ) => bail!("funding carry position does not match the configured instrument"),
        (
            FundingCarryInstrument::IsolatedFuture,
            Some(FundingCarryPosition::IsolatedFuture { side, .. }),
        ) if side != "sell" => bail!("funding carry refuses to manage a non-short futures trade"),
        _ => Ok(()),
    }
}

fn rate_to_ppm(rate: f64) -> Result<i32> {
    if !rate.is_finite() || !(-1.0..=1.0).contains(&rate) {
        bail!("funding rate must be a finite value between negative one and one");
    }
    let ppm = (rate * 1_000_000.0).round();
    i32::try_from(ppm as i64).context("funding rate exceeded supported range")
}

pub fn funding_carry_features(
    store: &MarketDataStore,
    network: Network,
    instrument: FundingCarryInstrument,
) -> Result<FundingCarryFeatures> {
    let market = store
        .feature_snapshot(network)?
        .context("LN Markets collector has no feature snapshot")?;
    let position = match instrument {
        FundingCarryInstrument::SyntheticUsd => market
            .account_drift
            .as_ref()
            .filter(|account| account.synthetic_usd > 0.0)
            .map(|account| -> Result<FundingCarryPosition> {
                Ok(FundingCarryPosition::SyntheticUsd {
                    notional_usd_cents: round_u64(account.synthetic_usd * 100.0)?,
                })
            })
            .transpose()?,
        FundingCarryInstrument::IsolatedFuture => store
            .recent(network, ISOLATED_STREAM_TOPIC, 50)?
            .into_iter()
            .find_map(|event| position_from_stream(&event.payload, &market).transpose())
            .transpose()?,
    };
    Ok(FundingCarryFeatures {
        market,
        position,
        settled_funding_sats: 0,
    })
}

fn position_from_stream(
    value: &Value,
    market: &FeatureSnapshot,
) -> Result<Option<FundingCarryPosition>> {
    if let Some(values) = value.as_array() {
        for value in values {
            if let Some(position) = position_from_stream(value, market)? {
                return Ok(Some(position));
            }
        }
        return Ok(None);
    }
    let candidate = value.get("trade").unwrap_or(value);
    let Some(object) = candidate.as_object() else {
        return Ok(None);
    };
    if object.get("running").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    let trade_id = object
        .get("id")
        .and_then(Value::as_str)
        .context("running isolated trade has no ID")?;
    let side = object
        .get("side")
        .and_then(Value::as_str)
        .context("running isolated trade has no side")?;
    let notional_usd = value_u64(object.get("quantity"), "isolated trade quantity")?;
    let margin_sats = value_u64(object.get("margin"), "isolated trade margin")?;
    let leverage = value_u8(object.get("leverage"), "isolated trade leverage")?;
    let liquidation_price = value_f64(
        object.get("liquidation"),
        "isolated trade liquidation price",
    )?;
    let current_price = market
        .liquidity
        .best_ask
        .or(market.liquidity.best_bid)
        .context("position stream has no corresponding market price")?;
    validate_price(current_price, "position stream market price")?;
    let liquidation_buffer_bps = if side == "sell" {
        ratio_basis_points_f64(liquidation_price - current_price, current_price)?
    } else {
        ratio_basis_points_f64(current_price - liquidation_price, current_price)?
    };
    let unrealized_profit_sats = value_i64(object.get("pl"), "isolated trade profit")?;
    let stop_loss_price = object
        .get("stoploss")
        .and_then(Value::as_f64)
        .filter(|price| *price > 0.0);
    Ok(Some(FundingCarryPosition::IsolatedFuture {
        trade_id: trade_id.into(),
        side: side.into(),
        notional_usd,
        margin_sats,
        leverage,
        liquidation_price,
        liquidation_buffer_bps,
        unrealized_profit_sats,
        stop_loss_price,
    }))
}

#[derive(Clone)]
pub struct FundingCarryExecutor {
    client: Arc<LnMarketsClient>,
}

impl FundingCarryExecutor {
    pub fn new(client: LnMarketsClient) -> Result<Self> {
        if client.network() != Network::Signet {
            bail!("funding carry execution is restricted to a signet client");
        }
        Ok(Self {
            client: Arc::new(client),
        })
    }

    fn operation<'a>(&self, intent: &'a OrderIntent) -> Result<&'a str> {
        if self.client.network() != Network::Signet {
            bail!("funding carry execution is restricted to signet");
        }
        intent.validate()?;
        if !matches!(
            intent.instrument.as_str(),
            ISOLATED_INSTRUMENT | SYNTHETIC_USD_INSTRUMENT
        ) {
            bail!("funding carry executor received an unsupported instrument");
        }
        intent
            .metadata
            .get("operation")
            .and_then(Value::as_str)
            .context("funding carry intent has no operation")
    }
}

#[async_trait]
impl VenueExecutor for FundingCarryExecutor {
    async fn preview(&self, intent: &OrderIntent) -> Result<VenueRiskSnapshot> {
        let operation = self.operation(intent)?;
        let account = self.client.account().await?;
        let venue_balance_after_sats = decimal_u64(&account.balance, "account balance")?;
        let (position_notional_before_usd, position_notional_after_usd, leverage, buffer) =
            if intent.instrument == ISOLATED_INSTRUMENT {
                let running = self.client.isolated_running_trades().await?;
                let current_notional = running
                    .iter()
                    .map(|trade| decimal_u64(&trade.quantity, "isolated quantity"))
                    .try_fold(0_u64, |sum, value| {
                        sum.checked_add(value?)
                            .context("isolated position notional overflowed")
                    })?;
                let after = match operation {
                    "open_short" => current_notional
                        .checked_add(intent.quantity.amount)
                        .context("isolated projected notional overflowed")?,
                    "close_short" => current_notional.saturating_sub(intent.quantity.amount),
                    _ => current_notional,
                };
                let leverage = intent
                    .metadata
                    .get("leverage")
                    .and_then(Value::as_u64)
                    .map(u8::try_from)
                    .transpose()
                    .context("funding carry leverage exceeded supported range")?
                    .or_else(|| {
                        running
                            .first()
                            .and_then(|trade| decimal_u8(&trade.leverage).ok())
                    })
                    .unwrap_or(1);
                let buffer = intent
                    .metadata
                    .get("projected_liquidation_buffer_bps")
                    .and_then(Value::as_u64)
                    .map(u32::try_from)
                    .transpose()
                    .context("funding carry liquidation buffer exceeded supported range")?
                    .unwrap_or(10_000);
                (current_notional, after, leverage, buffer)
            } else {
                (0, 0, 1, 10_000)
            };
        Ok(VenueRiskSnapshot {
            network: TradingNetwork::Signet,
            venue: "lnmarkets".into(),
            collateral_asset: AssetId::sats(),
            venue_balance_after: venue_balance_after_sats,
            position_notional_before_usd,
            position_notional_after_usd,
            leverage,
            liquidation_buffer_bps: buffer,
        })
    }

    async fn execute_once(&self, intent: &OrderIntent) -> Result<VenueExecution> {
        let operation = self.operation(intent)?;
        let occurred_at_ms = intent
            .metadata
            .get("occurred_at_ms")
            .and_then(Value::as_i64)
            .context("funding carry intent has no occurrence timestamp")?;
        let result = match operation {
            "open_short" => {
                let leverage = metadata_u8(intent, "leverage")?;
                let stop_loss = intent
                    .protection
                    .as_ref()
                    .context("funding carry short has no venue protection")?
                    .stop_loss_price;
                let trade = FuturesIsolatedNewTradeRequest::market(
                    FuturesLeverage::new(leverage)?,
                    FuturesTradeSide::Sell,
                    FuturesIsolatedTradeSize::quantity_usd(intent.quantity.amount)?,
                )
                .with_stoploss(decimal_from_f64(stop_loss)?)?
                .with_client_id(intent.intent_id.clone());
                ExecutionResult::Trade(
                    self.client
                        .isolated_new_trade(Network::Signet, &trade)
                        .await?,
                )
            }
            "close_short" => {
                let reference = trade_reference(intent)?;
                ExecutionResult::Trade(
                    self.client
                        .isolated_close_trade(Network::Signet, &reference)
                        .await?,
                )
            }
            "install_stop" => {
                let trade_id = trade_id(intent)?;
                let stop_loss = intent
                    .protection
                    .as_ref()
                    .context("funding carry stop update has no stop price")?
                    .stop_loss_price;
                let update =
                    FuturesIsolatedStoplossUpdate::fixed(trade_id, decimal_from_f64(stop_loss)?)?;
                ExecutionResult::Trade(
                    self.client
                        .isolated_update_stoploss(Network::Signet, &update)
                        .await?,
                )
            }
            "add_margin" => {
                let request =
                    FuturesIsolatedAmountRequest::new(trade_id(intent)?, intent.quantity.amount)?;
                ExecutionResult::Trade(
                    self.client
                        .isolated_add_margin(Network::Signet, &request)
                        .await?,
                )
            }
            "cash_in" => {
                let request =
                    FuturesIsolatedAmountRequest::new(trade_id(intent)?, intent.quantity.amount)?;
                ExecutionResult::Trade(
                    self.client
                        .isolated_cash_in(Network::Signet, &request)
                        .await?,
                )
            }
            "increase_synthetic_usd" => {
                let request = NewSwapRequest::bitcoin_to_synthetic_usd(intent.quantity.amount)?;
                ExecutionResult::Swap(self.client.new_swap(&request).await?)
            }
            "reduce_synthetic_usd" => {
                let request = NewSwapRequest::synthetic_usd_to_bitcoin(intent.quantity.amount)?;
                ExecutionResult::Swap(self.client.new_swap(&request).await?)
            }
            _ => bail!("unsupported funding carry operation {operation:?}"),
        };
        result.into_execution(intent, occurred_at_ms)
    }
}

enum ExecutionResult {
    Trade(FuturesIsolatedTrade),
    Swap(lnmarkets_client::NewSwapResult),
}

impl ExecutionResult {
    fn into_execution(self, intent: &OrderIntent, occurred_at_ms: i64) -> Result<VenueExecution> {
        let (venue_order_id, result, fee_sats) = match self {
            Self::Trade(trade) => {
                if intent.metadata.get("operation").and_then(Value::as_str) == Some("open_short")
                    && decimal_optional_f64(&trade.stoploss)? <= 0.0
                {
                    bail!("LN Markets opened a futures trade without the required venue stop");
                }
                let fees = decimal_signed_i64(&trade.opening_fee, "opening fee")?
                    .checked_add(decimal_signed_i64(&trade.closing_fee, "closing fee")?)
                    .context("funding carry trade fees overflowed")?;
                (
                    format!("lnmarkets-trade:{}", trade.id),
                    serde_json::to_value(&trade)?,
                    fees,
                )
            }
            Self::Swap(swap) => {
                match intent.side {
                    OrderSide::Sell
                        if swap.in_asset != Asset::BTC || swap.out_asset != Asset::USD =>
                    {
                        bail!("LN Markets returned the wrong assets for the carry swap")
                    }
                    OrderSide::Buy
                        if swap.in_asset != Asset::USD || swap.out_asset != Asset::BTC =>
                    {
                        bail!("LN Markets returned the wrong assets for the carry swap")
                    }
                    _ => {}
                }
                let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&swap)?));
                (
                    format!("lnmarkets-swap:{digest}"),
                    serde_json::to_value(&swap)?,
                    0,
                )
            }
        };
        let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&result)?));
        let mut ledger_entries = vec![LedgerEntryDraft {
            event_id: format!("lnmarkets-funding-carry-fill:{digest}"),
            occurred_at_ms,
            strategy_id: STRATEGY_ID.into(),
            kind: LedgerEntryKind::Fill,
            postings: Vec::new(),
            metadata: json!({
                "schema": "omega.lnmarkets.funding_carry_fill.v1",
                "intent_id": intent.intent_id,
                "operation": intent.metadata.get("operation"),
                "result": result,
            }),
        }];
        if fee_sats > 0 {
            ledger_entries.push(fee_entry(
                format!("lnmarkets-funding-carry-fee:{digest}"),
                occurred_at_ms,
                fee_sats,
                json!({"intent_id": intent.intent_id}),
            )?);
        }
        Ok(VenueExecution {
            venue_order_id,
            ledger_entries,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FundingFeeSyncReport {
    pub queried: usize,
    pub appended: usize,
    pub net_funding_sats: i64,
}

pub async fn sync_funding_fees(
    client: &LnMarketsClient,
    ledger: &LedgerStore,
    network: Network,
    trade_id: Option<&str>,
    from: Option<String>,
) -> Result<FundingFeeSyncReport> {
    if network != Network::Signet || client.network() != Network::Signet {
        bail!("funding fee sync is restricted to signet");
    }
    let mut pagination = Pagination::default().with_limit(1_000)?;
    if let Some(from) = from {
        pagination = pagination.with_time_range(from, None);
    }
    let page = client.isolated_funding_fees(&pagination, trade_id).await?;
    let mut report = FundingFeeSyncReport {
        queried: page.data.len(),
        ..FundingFeeSyncReport::default()
    };
    for fee in page.data {
        let fee_sats = decimal_signed_i64(&fee.fee, "funding fee")?;
        if fee_sats == 0 {
            continue;
        }
        let occurred_at_ms = DateTime::parse_from_rfc3339(&fee.time)
            .with_context(|| format!("invalid funding fee timestamp {:?}", fee.time))?
            .timestamp_millis();
        let entry = funding_entry(
            format!("lnmarkets-funding:{}", fee.settlement_id),
            occurred_at_ms,
            fee_sats,
            json!({
                "schema": "omega.lnmarkets.funding_settlement.v1",
                "settlement_id": fee.settlement_id,
                "trade_id": fee.trade_id,
                "venue_time": fee.time,
            }),
        )?;
        let sequence_before = ledger.entries(&Default::default())?.len();
        ledger.append(entry)?;
        let sequence_after = ledger.entries(&Default::default())?.len();
        if sequence_after > sequence_before {
            report.appended = report.appended.saturating_add(1);
            report.net_funding_sats = report
                .net_funding_sats
                .checked_add(fee_sats)
                .context("funding sync total overflowed")?;
        }
    }
    Ok(report)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FundingCarryBacktestModel {
    position: Option<FundingCarryPosition>,
    entry_price: Option<f64>,
    position_sats: u64,
    last_funding_samples: usize,
    uses_explicit_settlements: bool,
}

impl FundingCarryBacktestModel {
    fn price(features: &FundingCarryFeatures) -> Result<f64> {
        let price = features
            .market
            .index
            .current_price
            .or(features.market.liquidity.best_ask)
            .or(features.market.liquidity.best_bid)
            .context("funding carry backtest has no price")?;
        validate_price(price, "funding carry backtest price")?;
        Ok(price)
    }

    fn projected_funding_sats(&self, features: &FundingCarryFeatures) -> Result<i64> {
        if self.position.is_none() || features.market.funding.samples <= self.last_funding_samples {
            return Ok(0);
        }
        let rate = features.market.funding.current_rate.unwrap_or_default();
        if !rate.is_finite() {
            bail!("funding carry backtest rate must be finite");
        }
        signed_floor_i64(self.position_sats as f64 * rate)
    }
}

impl BacktestExecutionModel<FundingCarryFeatures> for FundingCarryBacktestModel {
    fn prepare_tick(
        &mut self,
        tick: &BacktestTick<FundingCarryFeatures>,
    ) -> Result<BacktestTick<FundingCarryFeatures>> {
        let mut tick = tick.clone();
        tick.features.position = self.position.clone();
        self.uses_explicit_settlements |= tick.features.settled_funding_sats != 0;
        if !self.uses_explicit_settlements {
            tick.features.settled_funding_sats = self.projected_funding_sats(&tick.features)?;
        }
        self.last_funding_samples = tick.features.market.funding.samples;
        Ok(tick)
    }

    fn settle_tick(
        &mut self,
        tick: &BacktestTick<FundingCarryFeatures>,
    ) -> Result<SimulatedSettlement> {
        Ok(SimulatedSettlement {
            gross_profit_sats: 0,
            funding_sats: tick.features.settled_funding_sats,
        })
    }

    fn execute(
        &mut self,
        intent: &OrderIntent,
        tick: &BacktestTick<FundingCarryFeatures>,
    ) -> Result<SimulatedTrade> {
        let price = Self::price(&tick.features)?;
        let notional_sats = match intent.quantity.unit {
            QuantityUnit::Sats => intent.quantity.amount,
            QuantityUnit::UsdCents => {
                floor_u64(intent.quantity.amount as f64 / 100.0 / price * SATOSHIS_PER_BITCOIN)?
            }
            QuantityUnit::UsdNotional => {
                floor_u64(intent.quantity.amount as f64 / price * SATOSHIS_PER_BITCOIN)?
            }
        };
        let operation = intent
            .metadata
            .get("operation")
            .and_then(Value::as_str)
            .context("funding carry backtest intent has no operation")?;
        let mut gross_profit_sats = 0;
        let counts_as_trade = match operation {
            "increase_synthetic_usd" => {
                self.entry_price = Some(price);
                self.position_sats = intent.quantity.amount;
                self.position = Some(FundingCarryPosition::SyntheticUsd {
                    notional_usd_cents: floor_u64(
                        intent.quantity.amount as f64 / SATOSHIS_PER_BITCOIN * price * 100.0,
                    )?,
                });
                true
            }
            "reduce_synthetic_usd" => {
                let entry_price = self
                    .entry_price
                    .take()
                    .context("funding carry backtest synthetic USD exit has no entry")?;
                gross_profit_sats =
                    signed_floor_i64(self.position_sats as f64 * (entry_price - price) / price)?;
                self.position = None;
                self.position_sats = 0;
                true
            }
            "open_short" => {
                let leverage = intent
                    .metadata
                    .get("leverage")
                    .and_then(Value::as_u64)
                    .and_then(|value| u8::try_from(value).ok())
                    .context("funding carry backtest short has no leverage")?;
                let notional_usd = intent.quantity.amount;
                self.entry_price = Some(price);
                self.position_sats = notional_sats;
                self.position = Some(FundingCarryPosition::IsolatedFuture {
                    trade_id: format!("backtest:{}", intent.intent_id),
                    side: "sell".into(),
                    notional_usd,
                    margin_sats: notional_sats / u64::from(leverage),
                    leverage,
                    liquidation_price: price * (1.0 + 1.0 / f64::from(leverage)),
                    liquidation_buffer_bps: 10_000 / u32::from(leverage),
                    unrealized_profit_sats: 0,
                    stop_loss_price: None,
                });
                true
            }
            "close_short" => {
                let entry_price = self
                    .entry_price
                    .take()
                    .context("funding carry backtest short exit has no entry")?;
                gross_profit_sats =
                    signed_floor_i64(self.position_sats as f64 * (entry_price - price) / price)?;
                self.position = None;
                self.position_sats = 0;
                true
            }
            "install_stop" => {
                if let Some(FundingCarryPosition::IsolatedFuture {
                    stop_loss_price, ..
                }) = self.position.as_mut()
                {
                    *stop_loss_price = intent
                        .protection
                        .as_ref()
                        .map(|protection| protection.stop_loss_price);
                }
                false
            }
            "add_margin" => {
                if let Some(FundingCarryPosition::IsolatedFuture { margin_sats, .. }) =
                    self.position.as_mut()
                {
                    *margin_sats = margin_sats
                        .checked_add(intent.quantity.amount)
                        .context("funding carry backtest margin overflowed")?;
                }
                false
            }
            "cash_in" => false,
            _ => bail!("funding carry backtest received unsupported operation {operation:?}"),
        };
        Ok(SimulatedTrade {
            gross_profit_sats,
            notional_sats,
            funding_sats: 0,
            counts_as_trade,
        })
    }
}

fn signed_floor_i64(value: f64) -> Result<i64> {
    if !value.is_finite() || value < i64::MIN as f64 || value > i64::MAX as f64 {
        bail!("funding carry backtest result is outside the supported range");
    }
    Ok(value.floor() as i64)
}

fn funding_entry(
    event_id: String,
    occurred_at_ms: i64,
    fee_sats: i64,
    metadata: Value,
) -> Result<LedgerEntryDraft> {
    let venue_amount = fee_sats;
    let income_amount = fee_sats
        .checked_neg()
        .context("funding posting overflowed")?;
    Ok(LedgerEntryDraft {
        event_id,
        occurred_at_ms,
        strategy_id: STRATEGY_ID.into(),
        kind: LedgerEntryKind::FundingSettlement,
        postings: vec![
            LedgerPosting::sats(
                LedgerAccount::VenueBalance {
                    venue: "lnmarkets".into(),
                },
                venue_amount,
            ),
            LedgerPosting::sats(LedgerAccount::FundingIncome, income_amount),
        ],
        metadata,
    })
}

fn fee_entry(
    event_id: String,
    occurred_at_ms: i64,
    fee_sats: i64,
    metadata: Value,
) -> Result<LedgerEntryDraft> {
    let venue_amount = fee_sats.checked_neg().context("fee posting overflowed")?;
    Ok(LedgerEntryDraft {
        event_id,
        occurred_at_ms,
        strategy_id: STRATEGY_ID.into(),
        kind: LedgerEntryKind::Fee,
        postings: vec![
            LedgerPosting::sats(
                LedgerAccount::VenueBalance {
                    venue: "lnmarkets".into(),
                },
                venue_amount,
            ),
            LedgerPosting::sats(LedgerAccount::FeeExpense, fee_sats),
        ],
        metadata,
    })
}

fn trade_id(intent: &OrderIntent) -> Result<FuturesTradeId> {
    intent
        .metadata
        .get("trade_id")
        .and_then(Value::as_str)
        .context("funding carry intent has no trade ID")?
        .parse()
        .context("funding carry intent has an invalid trade ID")
}

fn trade_reference(intent: &OrderIntent) -> Result<FuturesIsolatedTradeReference> {
    Ok(FuturesIsolatedTradeReference::new(trade_id(intent)?))
}

fn metadata_u8(intent: &OrderIntent, key: &str) -> Result<u8> {
    u8::try_from(
        intent
            .metadata
            .get(key)
            .and_then(Value::as_u64)
            .with_context(|| format!("funding carry intent has no {key}"))?,
    )
    .with_context(|| format!("funding carry {key} exceeded supported range"))
}

fn decimal_from_f64(value: f64) -> Result<DecimalAmount> {
    validate_price(value, "funding carry decimal")?;
    DecimalAmount::from_str(&format!("{value:.8}"))
        .context("funding carry decimal could not be represented")
}

fn decimal_optional_f64(value: &DecimalAmount) -> Result<f64> {
    value
        .as_number()
        .as_f64()
        .context("LN Markets decimal exceeded supported range")
}

fn decimal_u64(value: &DecimalAmount, label: &str) -> Result<u64> {
    round_u64(
        value
            .as_number()
            .as_f64()
            .with_context(|| format!("{label} exceeded supported range"))?,
    )
}

fn decimal_u8(value: &DecimalAmount) -> Result<u8> {
    u8::try_from(decimal_u64(value, "leverage")?).context("leverage exceeded supported range")
}

fn decimal_signed_i64(value: &DecimalAmount, label: &str) -> Result<i64> {
    let value = value
        .as_number()
        .as_f64()
        .with_context(|| format!("{label} exceeded supported range"))?;
    if !value.is_finite() || value < i64::MIN as f64 || value > i64::MAX as f64 {
        bail!("{label} exceeded supported range");
    }
    Ok(value.round() as i64)
}

fn value_f64(value: Option<&Value>, label: &str) -> Result<f64> {
    let value = value
        .and_then(Value::as_f64)
        .with_context(|| format!("{label} is missing or invalid"))?;
    if !value.is_finite() {
        bail!("{label} must be finite");
    }
    Ok(value)
}

fn value_u64(value: Option<&Value>, label: &str) -> Result<u64> {
    round_u64(value_f64(value, label)?)
}

fn value_u8(value: Option<&Value>, label: &str) -> Result<u8> {
    u8::try_from(value_u64(value, label)?).with_context(|| format!("{label} exceeded range"))
}

fn value_i64(value: Option<&Value>, label: &str) -> Result<i64> {
    let value = value_f64(value, label)?;
    if value < i64::MIN as f64 || value > i64::MAX as f64 {
        bail!("{label} exceeded supported range");
    }
    Ok(value.round() as i64)
}

fn validate_price(value: f64, label: &str) -> Result<()> {
    if !value.is_finite() || value <= 0.0 {
        bail!("{label} must be a positive finite number");
    }
    Ok(())
}

fn round_u64(value: f64) -> Result<u64> {
    if !value.is_finite() || value < 0.0 || value > u64::MAX as f64 {
        bail!("funding carry amount exceeded supported range");
    }
    Ok(value.round() as u64)
}

fn floor_u64(value: f64) -> Result<u64> {
    if !value.is_finite() || value < 0.0 || value > u64::MAX as f64 {
        bail!("funding carry amount exceeded supported range");
    }
    Ok(value.floor() as u64)
}

fn ratio_basis_points_f64(numerator: f64, denominator: f64) -> Result<u32> {
    if !numerator.is_finite() || numerator < 0.0 {
        return Ok(0);
    }
    validate_price(denominator, "funding carry ratio denominator")?;
    let value = numerator / denominator * BASIS_POINTS_DENOMINATOR as f64;
    if !value.is_finite() || value > f64::from(u32::MAX) {
        bail!("funding carry ratio exceeded supported range");
    }
    Ok(value.floor() as u32)
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use futures::{FutureExt as _, executor::block_on};
    use http::{Request, Response, StatusCode};
    use lnmarkets_client::{Credentials, HttpTransport, TransportFailure};
    use lnmarkets_data::{
        AccountDriftFeatures, FundingFeatures, FundingSign, IndexFeatures, LiquidityFeatures,
        VolatilityFeatures,
    };
    use strategy_engine::{BacktestCostModel, BacktestOutcome, BacktestPolicy, run_backtest};

    use super::*;

    fn config(instrument: FundingCarryInstrument) -> FundingCarryConfig {
        FundingCarryConfig {
            network: Network::Signet,
            instrument,
            entry_funding_rate_ppm: 100,
            exit_funding_rate_ppm: -10,
            expected_settlements: 100,
            measured_round_trip_cost_bps: 50,
            cost_margin_bps: 10,
            maximum_hedge_notional_usd: 1_000,
            notional_per_funding_ppm_usd: 2,
            leverage: 2,
            stop_loss_distance_bps: 500,
            liquidation_buffer_floor_bps: 1_000,
            margin_top_up_sats: 10_000,
            profit_sweep_threshold_sats: 20_000,
            profit_sweep_sats: 10_000,
        }
    }

    fn market(rate: f64) -> FeatureSnapshot {
        FeatureSnapshot {
            schema: "omega.lnmarkets.features.v1".into(),
            as_of_ms: Some(100),
            index: IndexFeatures {
                current_price: Some(50_005.0),
                one_hour_move: None,
                six_hours_move: None,
                one_day_move: None,
                price_points: 1,
            },
            volatility: VolatilityFeatures {
                one_hour: None,
                six_hours: None,
                one_day: None,
                price_points: 1,
            },
            funding: FundingFeatures {
                current_rate: Some(rate),
                ema: Some(rate),
                sign: if rate > 0.0 {
                    FundingSign::Positive
                } else {
                    FundingSign::Negative
                },
                sign_flipped_at_ms: None,
                measurement_started_at_ms: Some(1),
                measurement_ended_at_ms: Some(10),
                samples: 10,
            },
            liquidity: LiquidityFeatures {
                best_bid: Some(50_000.0),
                best_ask: Some(50_010.0),
                spread: Some(10.0),
                spread_bps: Some(2.0),
                bid_depth: Some(10_000.0),
                ask_depth: Some(10_000.0),
                tier_count: 1,
            },
            account_drift: Some(AccountDriftFeatures {
                btc_value_usd: 2_000.0,
                synthetic_usd: 0.0,
                current_btc_weight: 1.0,
                target_btc_weight: 0.5,
                drift: 0.5,
            }),
        }
    }

    fn tick(
        rate: f64,
        position: Option<FundingCarryPosition>,
    ) -> StrategyTick<FundingCarryFeatures> {
        StrategyTick {
            occurred_at_ms: 100,
            features: FundingCarryFeatures {
                market: market(rate),
                position,
                settled_funding_sats: 0,
            },
        }
    }

    fn isolated_position(
        stop_loss_price: Option<f64>,
        liquidation_buffer_bps: u32,
        profit_sats: i64,
    ) -> FundingCarryPosition {
        FundingCarryPosition::IsolatedFuture {
            trade_id: "77ad8f20-afa4-4844-915b-af2557af9758".into(),
            side: "sell".into(),
            notional_usd: 200,
            margin_sats: 100_000,
            leverage: 2,
            liquidation_price: 60_000.0,
            liquidation_buffer_bps,
            unrealized_profit_sats: profit_sats,
            stop_loss_price,
        }
    }

    #[test]
    fn positive_funding_opens_protected_short_after_cost_hurdle() {
        let config = config(FundingCarryInstrument::IsolatedFuture);
        let state = FundingCarryProgram.initial_state(&config).expect("state");
        let step = FundingCarryProgram
            .on_tick(&config, &state, &tick(0.001, None))
            .expect("step");
        assert_eq!(step.intents.len(), 1);
        assert_eq!(step.intents[0].side, OrderSide::Sell);
        assert!(step.intents[0].protection.is_some());
        assert_eq!(
            step.next_state.last_action,
            FundingCarryAction::OpenShort {
                notional_usd: 1_000
            }
        );
    }

    #[test]
    fn sign_flip_closes_and_hysteresis_holds() {
        let config = config(FundingCarryInstrument::IsolatedFuture);
        let state = FundingCarryProgram.initial_state(&config).expect("state");
        let position = isolated_position(Some(52_500.0), 2_000, 0);
        let hold = FundingCarryProgram
            .on_tick(&config, &state, &tick(0.00005, Some(position.clone())))
            .expect("hold");
        assert!(hold.intents.is_empty());
        assert_eq!(hold.next_state.last_action, FundingCarryAction::Holding);
        let close = FundingCarryProgram
            .on_tick(&config, &state, &tick(-0.00002, Some(position)))
            .expect("close");
        assert_eq!(close.intents.len(), 1);
        assert!(close.intents[0].reduce_only);
        assert!(matches!(
            close.next_state.last_action,
            FundingCarryAction::CloseShort { .. }
        ));
    }

    #[test]
    fn position_management_installs_stop_then_tops_up_then_sweeps() {
        let config = config(FundingCarryInstrument::IsolatedFuture);
        let state = FundingCarryProgram.initial_state(&config).expect("state");
        let missing_stop = FundingCarryProgram
            .on_tick(
                &config,
                &state,
                &tick(0.001, Some(isolated_position(None, 500, 30_000))),
            )
            .expect("stop");
        assert!(matches!(
            missing_stop.next_state.last_action,
            FundingCarryAction::InstallVenueStop { .. }
        ));
        let margin = FundingCarryProgram
            .on_tick(
                &config,
                &state,
                &tick(0.001, Some(isolated_position(Some(52_500.0), 500, 30_000))),
            )
            .expect("margin");
        assert!(matches!(
            margin.next_state.last_action,
            FundingCarryAction::AddMargin { .. }
        ));
        let sweep = FundingCarryProgram
            .on_tick(
                &config,
                &state,
                &tick(
                    0.001,
                    Some(isolated_position(Some(52_500.0), 2_000, 30_000)),
                ),
            )
            .expect("sweep");
        assert!(matches!(
            sweep.next_state.last_action,
            FundingCarryAction::CashInProfit { .. }
        ));
    }

    #[test]
    fn synthetic_usd_path_opens_and_reduces_without_leverage() {
        let config = config(FundingCarryInstrument::SyntheticUsd);
        let state = FundingCarryProgram.initial_state(&config).expect("state");
        let open = FundingCarryProgram
            .on_tick(&config, &state, &tick(0.001, None))
            .expect("open");
        assert_eq!(open.intents[0].instrument, SYNTHETIC_USD_INSTRUMENT);
        assert!(open.intents[0].protection.is_none());
        let close = FundingCarryProgram
            .on_tick(
                &config,
                &state,
                &tick(
                    -0.00002,
                    Some(FundingCarryPosition::SyntheticUsd {
                        notional_usd_cents: 10_000,
                    }),
                ),
            )
            .expect("close");
        assert_eq!(close.intents[0].quantity.unit, QuantityUnit::UsdCents);
        assert!(close.intents[0].reduce_only);
    }

    #[test]
    fn funding_history_backtest_accounts_for_settlements() {
        let config = config(FundingCarryInstrument::SyntheticUsd);
        let mut model = FundingCarryBacktestModel::default();
        let ticks = [
            (100, 0.001, 500),
            (200, -0.00002, 0),
            (300, 0.001, 500),
            (400, -0.00002, 0),
        ]
        .into_iter()
        .map(
            |(occurred_at_ms, rate, settled_funding_sats)| BacktestTick {
                occurred_at_ms,
                features: FundingCarryFeatures {
                    market: market(rate),
                    position: None,
                    settled_funding_sats,
                },
            },
        )
        .collect::<Vec<_>>();
        let report = run_backtest(
            &FundingCarryProgram,
            &config,
            &ticks,
            &mut model,
            TradingNetwork::Signet,
            BacktestCostModel {
                taker_fee_bps: 0,
                observed_round_trip_cost_bps: 0,
                measurement_source: "signet ledger".into(),
                measured_at_ms: 1,
            },
            BacktestPolicy {
                minimum_trade_count: 2,
                minimum_expectancy_millisats: 0,
                maximum_drawdown_sats: 10_000,
            },
            500,
        )
        .expect("backtest");
        assert_eq!(report.funding_sats, 1_000);
        assert_eq!(report.outcome, BacktestOutcome::Passed);
    }

    struct FakeTransport {
        responses: Mutex<VecDeque<Response<Vec<u8>>>>,
        requests: Mutex<Vec<Request<Vec<u8>>>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<Response<Vec<u8>>>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl HttpTransport for FakeTransport {
        fn send(
            &self,
            request: Request<Vec<u8>>,
        ) -> futures::future::BoxFuture<'static, Result<Response<Vec<u8>>, TransportFailure>>
        {
            self.requests.lock().expect("requests").push(request);
            let response = self
                .responses
                .lock()
                .expect("responses")
                .pop_front()
                .expect("fake response");
            async move { Ok(response) }.boxed()
        }
    }

    fn response(status: StatusCode, body: &str) -> Response<Vec<u8>> {
        Response::builder()
            .status(status)
            .body(body.as_bytes().to_vec())
            .expect("response")
    }

    fn client(transport: Arc<FakeTransport>) -> LnMarketsClient {
        LnMarketsClient::authenticated(
            transport,
            Network::Signet,
            Credentials::new("key", "secret", "passphrase").expect("credentials"),
        )
    }

    #[test]
    fn executor_opens_short_once_with_stop_and_syncs_funding_idempotently() {
        block_on(async {
            let trade_id = "77ad8f20-afa4-4844-915b-af2557af9758";
            let transport = Arc::new(FakeTransport::new(vec![
                response(
                    StatusCode::OK,
                    r#"{"balance":1000000,"email":null,"feeTier":0,"id":"id","linkingPublicKey":null,"syntheticUsdBalance":0,"username":"user"}"#,
                ),
                response(StatusCode::OK, "[]"),
                response(
                    StatusCode::OK,
                    &format!(
                        r#"{{"id":"{trade_id}","type":"market","side":"sell","openingFee":2,"closingFee":0,"maintenanceMargin":100,"quantity":200,"margin":1000,"leverage":2,"price":50000,"liquidation":60000,"stoploss":52500,"stoplossTrailingDistance":0.1,"takeprofit":0,"exitPrice":null,"pl":0,"createdAt":"2026-01-01T00:00:00.000Z","filledAt":"2026-01-01T00:00:01.000Z","closedAt":null,"entryPrice":50000,"open":false,"running":true,"canceled":false,"closed":false,"sumFundingFees":0,"sumCashInPl":0,"sumCashInMargin":0,"clientId":"carry"}}"#
                    ),
                ),
                response(
                    StatusCode::OK,
                    &format!(
                        r#"{{"data":[{{"fee":25,"settlementId":"settlement-1","time":"2026-08-09T08:00:00.000Z","tradeId":"{trade_id}"}}],"nextCursor":null}}"#
                    ),
                ),
                response(
                    StatusCode::OK,
                    &format!(
                        r#"{{"data":[{{"fee":25,"settlementId":"settlement-1","time":"2026-08-09T08:00:00.000Z","tradeId":"{trade_id}"}}],"nextCursor":null}}"#
                    ),
                ),
            ]));
            let executor = FundingCarryExecutor::new(client(transport.clone())).expect("executor");
            let config = config(FundingCarryInstrument::IsolatedFuture);
            let state = FundingCarryProgram.initial_state(&config).expect("state");
            let intent = FundingCarryProgram
                .on_tick(&config, &state, &tick(0.001, None))
                .expect("step")
                .intents
                .remove(0);
            executor.preview(&intent).await.expect("preview");
            let execution = executor.execute_once(&intent).await.expect("execute");
            assert_eq!(execution.ledger_entries.len(), 2);
            {
                let requests = transport.requests.lock().expect("requests");
                assert_eq!(
                    requests
                        .iter()
                        .filter(|request| request.uri().path() == "/v3/futures/isolated/trade")
                        .count(),
                    1
                );
                assert!(
                    String::from_utf8_lossy(requests.last().expect("request").body())
                        .contains("\"stoploss\"")
                );
            }

            let ledger = LedgerStore::in_memory().expect("ledger");
            let client = client(transport);
            let first = sync_funding_fees(&client, &ledger, Network::Signet, Some(trade_id), None)
                .await
                .expect("first sync");
            let second = sync_funding_fees(&client, &ledger, Network::Signet, Some(trade_id), None)
                .await
                .expect("second sync");
            assert_eq!(first.appended, 1);
            assert_eq!(second.appended, 0);
            assert_eq!(
                ledger
                    .profit_report(&Default::default())
                    .expect("profit")
                    .total_funding_collected_sats,
                25
            );
        });
    }

    #[test]
    fn mainnet_is_rejected_before_transport() {
        let transport = Arc::new(FakeTransport::new(Vec::new()));
        let client = LnMarketsClient::authenticated(
            transport.clone(),
            Network::Mainnet,
            Credentials::new("key", "secret", "passphrase").expect("credentials"),
        );
        assert!(FundingCarryExecutor::new(client).is_err());
        assert!(transport.requests.lock().expect("requests").is_empty());
        let mut config = config(FundingCarryInstrument::SyntheticUsd);
        config.network = Network::Mainnet;
        assert!(FundingCarryProgram.validate_config(&config).is_err());
    }
}
