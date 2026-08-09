use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use lnmarkets_client::{Asset, LnMarketsClient, Network, NewSwapResult};
use lnmarkets_data::FeatureSnapshot;
use serde::{Deserialize, Serialize};
use serde_json::json;
use strategy_engine::{
    BacktestExecutionModel, BacktestTick, OrderIntent, OrderKind, OrderQuantity, OrderSide,
    QuantityUnit, SimulatedTrade, StrategyProgram, StrategyStep, StrategyTick, VenueExecution,
    VenueExecutor, VenueRiskSnapshot,
};

use crate::SyntheticUsdExecutor;

pub const THRESHOLD_SWING_SCHEMA: &str = "openagents.omega.lnmarkets.threshold_swing.v1";
const STRATEGY_ID: &str = "threshold_swing";
const STRATEGY_VERSION: &str = "1";
const SYNTHETIC_USD_INSTRUMENT: &str = "lnmarkets.synthetic_usd";
const FILL_SCHEMA: &str = "omega.lnmarkets.threshold_swing_fill.v1";
const BASIS_POINTS_DENOMINATOR: f64 = 10_000.0;
const SATOSHIS_PER_BITCOIN: f64 = 100_000_000.0;
const VOLATILITY_UNITS_SCALE: f64 = 1_000.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdSwingWindow {
    OneHour,
    SixHours,
    OneDay,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ThresholdSwingConfig {
    pub network: Network,
    pub window: ThresholdSwingWindow,
    pub threshold_volatility_units_milli: u32,
    pub measured_round_trip_cost_bps: u32,
    pub cost_margin_bps: u32,
    pub maximum_spread_bps: u32,
    pub maximum_position_usd_cents: u64,
    pub liquidity_utilization_bps: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThresholdSwingPosition {
    Flat,
    LongBitcoin {
        entry_index_price: f64,
        amount_sats: u64,
        input_usd_cents: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThresholdSwingAction {
    WaitingForIndex,
    WaitingForVolatility,
    WaitingForAccountData,
    WaitingForLiquidity,
    SpreadTooWide,
    SignalBelowThreshold,
    CostHurdleNotMet,
    OrderBelowVenueMinimum,
    BuyBitcoin {
        amount_usd_cents: u64,
        expected_amount_sats: u64,
    },
    HoldLong,
    SellBitcoin {
        amount_sats: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThresholdSwingState {
    pub schema: String,
    pub sequence: u64,
    pub window: ThresholdSwingWindow,
    pub index_price: Option<f64>,
    pub index_move_bps: Option<i32>,
    pub volatility_bps: Option<u32>,
    pub signal_volatility_units_milli: Option<u32>,
    pub spread_bps: Option<u32>,
    pub cost_hurdle_bps: u32,
    pub position: ThresholdSwingPosition,
    pub last_action: ThresholdSwingAction,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ThresholdSwingProgram;

impl StrategyProgram for ThresholdSwingProgram {
    type Config = ThresholdSwingConfig;
    type State = ThresholdSwingState;
    type Features = FeatureSnapshot;

    fn strategy_id(&self) -> &'static str {
        STRATEGY_ID
    }

    fn strategy_version(&self) -> &'static str {
        STRATEGY_VERSION
    }

    fn validate_config(&self, config: &Self::Config) -> Result<()> {
        if config.network != Network::Signet {
            bail!("threshold_swing is restricted to signet");
        }
        if config.threshold_volatility_units_milli == 0
            || config.threshold_volatility_units_milli > 100_000
        {
            bail!("threshold swing threshold must be from 1 through 100000 milli-units");
        }
        let cost_hurdle_bps = config
            .measured_round_trip_cost_bps
            .checked_add(config.cost_margin_bps)
            .context("threshold swing cost hurdle overflowed")?;
        if cost_hurdle_bps > 10_000 {
            bail!("threshold swing cost hurdle must not exceed 10000 basis points");
        }
        if config.maximum_spread_bps == 0 || config.maximum_spread_bps > 10_000 {
            bail!("threshold swing maximum spread must be from 1 through 10000 basis points");
        }
        if config.maximum_position_usd_cents == 0 {
            bail!("threshold swing maximum position must be greater than zero");
        }
        if !(1..=10_000).contains(&config.liquidity_utilization_bps) {
            bail!(
                "threshold swing liquidity utilization must be from 1 through 10000 basis points"
            );
        }
        Ok(())
    }

    fn initial_state(&self, config: &Self::Config) -> Result<Self::State> {
        self.validate_config(config)?;
        Ok(ThresholdSwingState {
            schema: THRESHOLD_SWING_SCHEMA.into(),
            sequence: 0,
            window: config.window,
            index_price: None,
            index_move_bps: None,
            volatility_bps: None,
            signal_volatility_units_milli: None,
            spread_bps: None,
            cost_hurdle_bps: cost_hurdle_bps(config)?,
            position: ThresholdSwingPosition::Flat,
            last_action: ThresholdSwingAction::WaitingForIndex,
        })
    }

    fn on_tick(
        &self,
        config: &Self::Config,
        state: &Self::State,
        tick: &StrategyTick<Self::Features>,
    ) -> Result<StrategyStep<Self::State>> {
        self.validate_config(config)?;
        let sequence = state
            .sequence
            .checked_add(1)
            .context("threshold swing sequence overflowed")?;
        let cost_hurdle_bps = cost_hurdle_bps(config)?;
        let Some(index_price) = tick.features.index.current_price else {
            return Ok(step_without_order(
                config,
                state,
                sequence,
                None,
                None,
                None,
                None,
                cost_hurdle_bps,
                ThresholdSwingAction::WaitingForIndex,
            ));
        };
        validate_positive_finite(index_price, "threshold swing index price")?;
        let (volatility, index_move) = window_features(config.window, &tick.features);
        let Some(volatility) = volatility else {
            return Ok(step_without_order(
                config,
                state,
                sequence,
                Some(index_price),
                None,
                None,
                None,
                cost_hurdle_bps,
                ThresholdSwingAction::WaitingForVolatility,
            ));
        };
        let Some(index_move) = index_move else {
            return Ok(step_without_order(
                config,
                state,
                sequence,
                Some(index_price),
                None,
                Some(ratio_to_basis_points(volatility.abs())?),
                None,
                cost_hurdle_bps,
                ThresholdSwingAction::WaitingForIndex,
            ));
        };
        validate_positive_finite(volatility, "threshold swing volatility")?;
        if !index_move.is_finite() {
            bail!("threshold swing index move must be finite");
        }
        let index_move_bps = signed_ratio_to_basis_points(index_move)?;
        let volatility_bps = ratio_to_basis_points(volatility)?;
        let signal_volatility_units_milli = ratio_to_milli_units(index_move.abs(), volatility)?;
        let spread_bps = finite_non_negative(tick.features.liquidity.spread_bps)
            .map(ratio_value_to_u32)
            .transpose()?;
        let Some(spread_bps) = spread_bps else {
            return Ok(step_without_order(
                config,
                state,
                sequence,
                Some(index_price),
                Some(index_move_bps),
                Some(volatility_bps),
                Some(signal_volatility_units_milli),
                cost_hurdle_bps,
                ThresholdSwingAction::WaitingForLiquidity,
            ));
        };
        if spread_bps > config.maximum_spread_bps {
            return Ok(step_without_order_with_spread(
                config,
                state,
                sequence,
                index_price,
                index_move_bps,
                volatility_bps,
                signal_volatility_units_milli,
                spread_bps,
                cost_hurdle_bps,
                ThresholdSwingAction::SpreadTooWide,
            ));
        }
        if signal_volatility_units_milli < config.threshold_volatility_units_milli {
            let action = if matches!(state.position, ThresholdSwingPosition::Flat) {
                ThresholdSwingAction::SignalBelowThreshold
            } else {
                ThresholdSwingAction::HoldLong
            };
            return Ok(step_without_order_with_spread(
                config,
                state,
                sequence,
                index_price,
                index_move_bps,
                volatility_bps,
                signal_volatility_units_milli,
                spread_bps,
                cost_hurdle_bps,
                action,
            ));
        }
        if index_move_bps.unsigned_abs() <= cost_hurdle_bps {
            return Ok(step_without_order_with_spread(
                config,
                state,
                sequence,
                index_price,
                index_move_bps,
                volatility_bps,
                signal_volatility_units_milli,
                spread_bps,
                cost_hurdle_bps,
                ThresholdSwingAction::CostHurdleNotMet,
            ));
        }

        match &state.position {
            ThresholdSwingPosition::Flat if index_move_bps < 0 => enter_long(
                config,
                sequence,
                index_price,
                index_move_bps,
                volatility_bps,
                signal_volatility_units_milli,
                spread_bps,
                cost_hurdle_bps,
                tick,
            ),
            ThresholdSwingPosition::LongBitcoin {
                entry_index_price,
                amount_sats,
                input_usd_cents,
            } if index_move_bps > 0 => exit_long(
                config,
                sequence,
                index_price,
                index_move_bps,
                volatility_bps,
                signal_volatility_units_milli,
                spread_bps,
                cost_hurdle_bps,
                *entry_index_price,
                *amount_sats,
                *input_usd_cents,
                tick,
            ),
            ThresholdSwingPosition::Flat => Ok(step_without_order_with_spread(
                config,
                state,
                sequence,
                index_price,
                index_move_bps,
                volatility_bps,
                signal_volatility_units_milli,
                spread_bps,
                cost_hurdle_bps,
                ThresholdSwingAction::SignalBelowThreshold,
            )),
            ThresholdSwingPosition::LongBitcoin { .. } => Ok(step_without_order_with_spread(
                config,
                state,
                sequence,
                index_price,
                index_move_bps,
                volatility_bps,
                signal_volatility_units_milli,
                spread_bps,
                cost_hurdle_bps,
                ThresholdSwingAction::HoldLong,
            )),
        }
    }

    fn on_execution(
        &self,
        config: &Self::Config,
        state: &Self::State,
        intent: &OrderIntent,
        execution: &VenueExecution,
    ) -> Result<Self::State> {
        self.validate_config(config)?;
        let fill = execution
            .ledger_entries
            .iter()
            .find(|entry| {
                entry
                    .metadata
                    .get("schema")
                    .and_then(serde_json::Value::as_str)
                    == Some(FILL_SCHEMA)
            })
            .context("threshold swing execution has no fill record")?;
        let result: NewSwapResult = serde_json::from_value(
            fill.metadata
                .get("swap")
                .context("threshold swing fill has no swap result")?
                .clone(),
        )?;
        let mut next_state = state.clone();
        match intent.side {
            OrderSide::Buy => {
                if result.in_asset != Asset::USD || result.out_asset != Asset::BTC {
                    bail!("threshold swing buy returned the wrong assets");
                }
                let input_usd =
                    decimal_positive_f64(&result.in_amount, "threshold swing USD input")?;
                let expected_input_usd = intent.quantity.amount as f64 / 100.0;
                if (input_usd - expected_input_usd).abs() > 0.000_000_01 {
                    bail!("threshold swing buy input does not match the admitted intent");
                }
                let amount_sats = exact_positive_u64(
                    decimal_positive_f64(&result.out_amount, "threshold swing BTC output")?,
                    "threshold swing BTC output",
                )?;
                let ThresholdSwingPosition::LongBitcoin {
                    entry_index_price,
                    input_usd_cents,
                    ..
                } = next_state.position
                else {
                    bail!("threshold swing buy did not produce a long position state");
                };
                next_state.position = ThresholdSwingPosition::LongBitcoin {
                    entry_index_price,
                    amount_sats,
                    input_usd_cents,
                };
            }
            OrderSide::Sell => {
                if result.in_asset != Asset::BTC || result.out_asset != Asset::USD {
                    bail!("threshold swing sell returned the wrong assets");
                }
                let input_sats = exact_positive_u64(
                    decimal_positive_f64(&result.in_amount, "threshold swing BTC input")?,
                    "threshold swing BTC input",
                )?;
                if input_sats != intent.quantity.amount {
                    bail!("threshold swing sell input does not match the admitted intent");
                }
                decimal_positive_f64(&result.out_amount, "threshold swing USD output")?;
                if !matches!(next_state.position, ThresholdSwingPosition::Flat) {
                    bail!("threshold swing sell did not produce a flat position state");
                }
            }
        }
        Ok(next_state)
    }
}

fn enter_long(
    config: &ThresholdSwingConfig,
    sequence: u64,
    index_price: f64,
    index_move_bps: i32,
    volatility_bps: u32,
    signal_volatility_units_milli: u32,
    spread_bps: u32,
    cost_hurdle_bps: u32,
    tick: &StrategyTick<FeatureSnapshot>,
) -> Result<StrategyStep<ThresholdSwingState>> {
    let Some(account) = tick.features.account_drift.as_ref() else {
        return Ok(flat_step(
            config,
            sequence,
            index_price,
            index_move_bps,
            volatility_bps,
            signal_volatility_units_milli,
            spread_bps,
            cost_hurdle_bps,
            ThresholdSwingAction::WaitingForAccountData,
        ));
    };
    if !account.synthetic_usd.is_finite() || account.synthetic_usd < 0.0 {
        bail!("threshold swing synthetic USD balance must be non-negative and finite");
    }
    let available_usd_cents = floor_u64(account.synthetic_usd * 100.0)?;
    let ladder_usd_cents = tick
        .features
        .liquidity
        .ask_depth
        .filter(|depth| depth.is_finite() && *depth >= 0.0)
        .map(|depth| {
            floor_u64(
                depth * 100.0 * f64::from(config.liquidity_utilization_bps)
                    / BASIS_POINTS_DENOMINATOR,
            )
        })
        .transpose()?;
    let amount_usd_cents = config
        .maximum_position_usd_cents
        .min(available_usd_cents)
        .min(ladder_usd_cents.unwrap_or(0));
    if amount_usd_cents == 0 {
        return Ok(flat_step(
            config,
            sequence,
            index_price,
            index_move_bps,
            volatility_bps,
            signal_volatility_units_milli,
            spread_bps,
            cost_hurdle_bps,
            ThresholdSwingAction::WaitingForLiquidity,
        ));
    }
    let expected_amount_sats =
        floor_u64(amount_usd_cents as f64 / 100.0 / index_price * SATOSHIS_PER_BITCOIN)?;
    if expected_amount_sats < 1_000 {
        return Ok(flat_step(
            config,
            sequence,
            index_price,
            index_move_bps,
            volatility_bps,
            signal_volatility_units_milli,
            spread_bps,
            cost_hurdle_bps,
            ThresholdSwingAction::OrderBelowVenueMinimum,
        ));
    }
    let action = ThresholdSwingAction::BuyBitcoin {
        amount_usd_cents,
        expected_amount_sats,
    };
    let next_state = state(
        config,
        sequence,
        Some(index_price),
        Some(index_move_bps),
        Some(volatility_bps),
        Some(signal_volatility_units_milli),
        Some(spread_bps),
        cost_hurdle_bps,
        ThresholdSwingPosition::LongBitcoin {
            entry_index_price: index_price,
            amount_sats: expected_amount_sats,
            input_usd_cents: amount_usd_cents,
        },
        action,
    );
    let intent = intent(
        sequence,
        tick.occurred_at_ms,
        OrderSide::Buy,
        OrderQuantity {
            amount: amount_usd_cents,
            unit: QuantityUnit::UsdCents,
        },
        false,
        &next_state,
    )?;
    Ok(StrategyStep {
        cancels: Vec::new(),
        next_state,
        intents: vec![intent],
    })
}

fn exit_long(
    config: &ThresholdSwingConfig,
    sequence: u64,
    index_price: f64,
    index_move_bps: i32,
    volatility_bps: u32,
    signal_volatility_units_milli: u32,
    spread_bps: u32,
    cost_hurdle_bps: u32,
    entry_index_price: f64,
    amount_sats: u64,
    input_usd_cents: u64,
    tick: &StrategyTick<FeatureSnapshot>,
) -> Result<StrategyStep<ThresholdSwingState>> {
    let position_value_usd = amount_sats as f64 / SATOSHIS_PER_BITCOIN * index_price;
    let available_bid_depth = tick
        .features
        .liquidity
        .bid_depth
        .filter(|depth| depth.is_finite() && *depth >= 0.0)
        .map(|depth| {
            depth * f64::from(config.liquidity_utilization_bps) / BASIS_POINTS_DENOMINATOR
        });
    if !available_bid_depth.is_some_and(|depth| depth >= position_value_usd) {
        return Ok(state_step(
            config,
            sequence,
            index_price,
            index_move_bps,
            volatility_bps,
            signal_volatility_units_milli,
            spread_bps,
            cost_hurdle_bps,
            ThresholdSwingPosition::LongBitcoin {
                entry_index_price,
                amount_sats,
                input_usd_cents,
            },
            ThresholdSwingAction::WaitingForLiquidity,
        ));
    }
    let next_state = state(
        config,
        sequence,
        Some(index_price),
        Some(index_move_bps),
        Some(volatility_bps),
        Some(signal_volatility_units_milli),
        Some(spread_bps),
        cost_hurdle_bps,
        ThresholdSwingPosition::Flat,
        ThresholdSwingAction::SellBitcoin { amount_sats },
    );
    let intent = intent(
        sequence,
        tick.occurred_at_ms,
        OrderSide::Sell,
        OrderQuantity {
            amount: amount_sats,
            unit: QuantityUnit::Sats,
        },
        true,
        &next_state,
    )?;
    Ok(StrategyStep {
        cancels: Vec::new(),
        next_state,
        intents: vec![intent],
    })
}

fn intent(
    sequence: u64,
    occurred_at_ms: i64,
    side: OrderSide,
    quantity: OrderQuantity,
    reduce_only: bool,
    state: &ThresholdSwingState,
) -> Result<OrderIntent> {
    let intent = OrderIntent {
        intent_id: format!("{STRATEGY_ID}:{occurred_at_ms}:{sequence}"),
        instrument: SYNTHETIC_USD_INSTRUMENT.into(),
        side,
        kind: OrderKind::Market,
        quantity,
        limit_price: None,
        reduce_only,
        protection: None,
        metadata: json!({
            "schema": THRESHOLD_SWING_SCHEMA,
            "occurred_at_ms": occurred_at_ms,
            "window": state.window,
            "index_price": state.index_price,
            "index_move_bps": state.index_move_bps,
            "volatility_bps": state.volatility_bps,
            "signal_volatility_units_milli": state.signal_volatility_units_milli,
            "spread_bps": state.spread_bps,
            "cost_hurdle_bps": state.cost_hurdle_bps,
        }),
    };
    intent.validate()?;
    Ok(intent)
}

fn window_features(
    window: ThresholdSwingWindow,
    features: &FeatureSnapshot,
) -> (Option<f64>, Option<f64>) {
    match window {
        ThresholdSwingWindow::OneHour => {
            (features.volatility.one_hour, features.index.one_hour_move)
        }
        ThresholdSwingWindow::SixHours => {
            (features.volatility.six_hours, features.index.six_hours_move)
        }
        ThresholdSwingWindow::OneDay => (features.volatility.one_day, features.index.one_day_move),
    }
}

fn cost_hurdle_bps(config: &ThresholdSwingConfig) -> Result<u32> {
    config
        .measured_round_trip_cost_bps
        .checked_add(config.cost_margin_bps)
        .context("threshold swing cost hurdle overflowed")
}

fn step_without_order(
    config: &ThresholdSwingConfig,
    previous: &ThresholdSwingState,
    sequence: u64,
    index_price: Option<f64>,
    index_move_bps: Option<i32>,
    volatility_bps: Option<u32>,
    signal_volatility_units_milli: Option<u32>,
    cost_hurdle_bps: u32,
    action: ThresholdSwingAction,
) -> StrategyStep<ThresholdSwingState> {
    StrategyStep {
        cancels: Vec::new(),
        next_state: state(
            config,
            sequence,
            index_price,
            index_move_bps,
            volatility_bps,
            signal_volatility_units_milli,
            None,
            cost_hurdle_bps,
            previous.position.clone(),
            action,
        ),
        intents: Vec::new(),
    }
}

fn step_without_order_with_spread(
    config: &ThresholdSwingConfig,
    previous: &ThresholdSwingState,
    sequence: u64,
    index_price: f64,
    index_move_bps: i32,
    volatility_bps: u32,
    signal_volatility_units_milli: u32,
    spread_bps: u32,
    cost_hurdle_bps: u32,
    action: ThresholdSwingAction,
) -> StrategyStep<ThresholdSwingState> {
    state_step(
        config,
        sequence,
        index_price,
        index_move_bps,
        volatility_bps,
        signal_volatility_units_milli,
        spread_bps,
        cost_hurdle_bps,
        previous.position.clone(),
        action,
    )
}

#[allow(clippy::too_many_arguments)]
fn flat_step(
    config: &ThresholdSwingConfig,
    sequence: u64,
    index_price: f64,
    index_move_bps: i32,
    volatility_bps: u32,
    signal_volatility_units_milli: u32,
    spread_bps: u32,
    cost_hurdle_bps: u32,
    action: ThresholdSwingAction,
) -> StrategyStep<ThresholdSwingState> {
    state_step(
        config,
        sequence,
        index_price,
        index_move_bps,
        volatility_bps,
        signal_volatility_units_milli,
        spread_bps,
        cost_hurdle_bps,
        ThresholdSwingPosition::Flat,
        action,
    )
}

#[allow(clippy::too_many_arguments)]
fn state_step(
    config: &ThresholdSwingConfig,
    sequence: u64,
    index_price: f64,
    index_move_bps: i32,
    volatility_bps: u32,
    signal_volatility_units_milli: u32,
    spread_bps: u32,
    cost_hurdle_bps: u32,
    position: ThresholdSwingPosition,
    action: ThresholdSwingAction,
) -> StrategyStep<ThresholdSwingState> {
    StrategyStep {
        cancels: Vec::new(),
        next_state: state(
            config,
            sequence,
            Some(index_price),
            Some(index_move_bps),
            Some(volatility_bps),
            Some(signal_volatility_units_milli),
            Some(spread_bps),
            cost_hurdle_bps,
            position,
            action,
        ),
        intents: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn state(
    config: &ThresholdSwingConfig,
    sequence: u64,
    index_price: Option<f64>,
    index_move_bps: Option<i32>,
    volatility_bps: Option<u32>,
    signal_volatility_units_milli: Option<u32>,
    spread_bps: Option<u32>,
    cost_hurdle_bps: u32,
    position: ThresholdSwingPosition,
    last_action: ThresholdSwingAction,
) -> ThresholdSwingState {
    ThresholdSwingState {
        schema: THRESHOLD_SWING_SCHEMA.into(),
        sequence,
        window: config.window,
        index_price,
        index_move_bps,
        volatility_bps,
        signal_volatility_units_milli,
        spread_bps,
        cost_hurdle_bps,
        position,
        last_action,
    }
}

fn finite_non_negative(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}

fn validate_positive_finite(value: f64, label: &str) -> Result<()> {
    if !value.is_finite() || value <= 0.0 {
        bail!("{label} must be a positive finite number");
    }
    Ok(())
}

fn ratio_to_basis_points(value: f64) -> Result<u32> {
    ratio_value_to_u32(value * BASIS_POINTS_DENOMINATOR)
}

fn signed_ratio_to_basis_points(value: f64) -> Result<i32> {
    let value = value * BASIS_POINTS_DENOMINATOR;
    if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
        bail!("threshold swing signed basis points are outside the supported range");
    }
    Ok(value.round() as i32)
}

fn ratio_to_milli_units(numerator: f64, denominator: f64) -> Result<u32> {
    ratio_value_to_u32(numerator / denominator * VOLATILITY_UNITS_SCALE)
}

fn ratio_value_to_u32(value: f64) -> Result<u32> {
    if !value.is_finite() || value < 0.0 || value > u32::MAX as f64 {
        bail!("threshold swing ratio is outside the supported range");
    }
    Ok(value.round() as u32)
}

fn floor_u64(value: f64) -> Result<u64> {
    if !value.is_finite() || value < 0.0 || value > u64::MAX as f64 {
        bail!("threshold swing amount is outside the supported range");
    }
    Ok(value.floor() as u64)
}

fn exact_positive_u64(value: f64, label: &str) -> Result<u64> {
    if value <= 0.0 || value > u64::MAX as f64 || value.fract().abs() > f64::EPSILON {
        bail!("{label} must be a positive whole number");
    }
    Ok(value as u64)
}

fn decimal_positive_f64(value: &lnmarkets_client::DecimalAmount, label: &str) -> Result<f64> {
    let value = value
        .as_number()
        .as_f64()
        .with_context(|| format!("{label} is outside the supported numeric range"))?;
    if !value.is_finite() || value <= 0.0 {
        bail!("{label} must be a positive finite number");
    }
    Ok(value)
}

#[derive(Clone)]
pub struct ThresholdSwingExecutor {
    inner: SyntheticUsdExecutor,
}

impl ThresholdSwingExecutor {
    pub fn new(client: LnMarketsClient) -> Result<Self> {
        Ok(Self {
            inner: SyntheticUsdExecutor::for_strategy(
                client,
                Network::Signet,
                STRATEGY_ID,
                FILL_SCHEMA,
                "omega.lnmarkets.threshold_swing_cost.v1",
                STRATEGY_ID,
            )?,
        })
    }
}

#[async_trait]
impl VenueExecutor for ThresholdSwingExecutor {
    async fn preview(&self, intent: &OrderIntent) -> Result<VenueRiskSnapshot> {
        self.inner.preview(intent).await
    }

    async fn execute_once(&self, intent: &OrderIntent) -> Result<VenueExecution> {
        self.inner.execute_once(intent).await
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct BacktestEntry {
    index_price: f64,
    amount_sats: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThresholdSwingBacktestModel {
    entry: Option<BacktestEntry>,
}

impl BacktestExecutionModel<FeatureSnapshot> for ThresholdSwingBacktestModel {
    fn execute(
        &mut self,
        intent: &OrderIntent,
        tick: &BacktestTick<FeatureSnapshot>,
    ) -> Result<SimulatedTrade> {
        let index_price = tick
            .features
            .index
            .current_price
            .context("threshold swing backtest has no index price")?;
        validate_positive_finite(index_price, "threshold swing backtest index price")?;
        match (intent.side, intent.quantity.unit) {
            (OrderSide::Buy, QuantityUnit::UsdCents) => {
                if self.entry.is_some() {
                    bail!("threshold swing backtest cannot enter twice");
                }
                let amount_sats = floor_u64(
                    intent.quantity.amount as f64 / 100.0 / index_price * SATOSHIS_PER_BITCOIN,
                )?;
                self.entry = Some(BacktestEntry {
                    index_price,
                    amount_sats,
                });
                Ok(SimulatedTrade {
                    gross_profit_sats: 0,
                    notional_sats: amount_sats,
                    funding_sats: 0,
                    counts_as_trade: true,
                })
            }
            (OrderSide::Sell, QuantityUnit::Sats) => {
                let entry = self
                    .entry
                    .take()
                    .context("threshold swing backtest cannot exit without an entry")?;
                if intent.quantity.amount > entry.amount_sats {
                    bail!("threshold swing backtest exit exceeds its recorded position");
                }
                let amount_bitcoin = intent.quantity.amount as f64 / SATOSHIS_PER_BITCOIN;
                let gross_profit_usd = amount_bitcoin * (index_price - entry.index_price);
                let gross_profit_sats =
                    signed_floor_i64(gross_profit_usd / index_price * SATOSHIS_PER_BITCOIN)?;
                Ok(SimulatedTrade {
                    gross_profit_sats,
                    notional_sats: intent.quantity.amount,
                    funding_sats: 0,
                    counts_as_trade: true,
                })
            }
            _ => bail!("threshold swing backtest received an unsupported intent"),
        }
    }
}

fn signed_floor_i64(value: f64) -> Result<i64> {
    if !value.is_finite() || value < i64::MIN as f64 || value > i64::MAX as f64 {
        bail!("threshold swing profit is outside the supported range");
    }
    Ok(value.floor() as i64)
}

#[cfg(test)]
mod tests {
    use lnmarkets_data::{
        AccountDriftFeatures, FundingFeatures, FundingSign, IndexFeatures, LiquidityFeatures,
        VolatilityFeatures,
    };
    use strategy_engine::{BacktestCostModel, BacktestOutcome, BacktestPolicy, run_backtest};
    use trading_mandate::TradingNetwork;

    use super::*;

    fn config() -> ThresholdSwingConfig {
        ThresholdSwingConfig {
            network: Network::Signet,
            window: ThresholdSwingWindow::OneHour,
            threshold_volatility_units_milli: 1_000,
            measured_round_trip_cost_bps: 10,
            cost_margin_bps: 5,
            maximum_spread_bps: 20,
            maximum_position_usd_cents: 10_000,
            liquidity_utilization_bps: 5_000,
        }
    }

    fn features(index_price: f64, index_move: f64, volatility: f64) -> FeatureSnapshot {
        FeatureSnapshot {
            schema: "omega.lnmarkets.features.v1".into(),
            as_of_ms: Some(100),
            index: IndexFeatures {
                current_price: Some(index_price),
                one_hour_move: Some(index_move),
                six_hours_move: Some(index_move),
                one_day_move: Some(index_move),
                price_points: 20,
            },
            volatility: VolatilityFeatures {
                one_hour: Some(volatility),
                six_hours: Some(volatility),
                one_day: Some(volatility),
                price_points: 20,
            },
            funding: FundingFeatures {
                current_rate: None,
                ema: None,
                sign: FundingSign::Neutral,
                sign_flipped_at_ms: None,
                samples: 0,
            },
            liquidity: LiquidityFeatures {
                best_bid: Some(index_price - 0.5),
                best_ask: Some(index_price + 0.5),
                spread: Some(1.0),
                spread_bps: Some(1.0),
                bid_depth: Some(1_000_000.0),
                ask_depth: Some(1_000_000.0),
                tier_count: 1,
            },
            account_drift: Some(AccountDriftFeatures {
                btc_value_usd: 10_000.0,
                synthetic_usd: 10_000.0,
                current_btc_weight: 0.5,
                target_btc_weight: 0.5,
                drift: 0.0,
            }),
        }
    }

    fn tick(index_price: f64, index_move: f64, volatility: f64) -> StrategyTick<FeatureSnapshot> {
        StrategyTick {
            occurred_at_ms: 100,
            features: features(index_price, index_move, volatility),
        }
    }

    #[test]
    fn drop_buys_bounded_bitcoin_and_symmetric_rise_exits() {
        let config = config();
        let initial = ThresholdSwingProgram.initial_state(&config).expect("state");
        let entry = ThresholdSwingProgram
            .on_tick(&config, &initial, &tick(90.0, -0.03, 0.02))
            .expect("entry");
        assert!(matches!(
            entry.intents.as_slice(),
            [OrderIntent {
                side: OrderSide::Buy,
                quantity: OrderQuantity {
                    amount: 10_000,
                    unit: QuantityUnit::UsdCents,
                },
                ..
            }]
        ));
        let exit = ThresholdSwingProgram
            .on_tick(&config, &entry.next_state, &tick(100.0, 0.03, 0.02))
            .expect("exit");
        assert!(matches!(
            exit.intents.as_slice(),
            [OrderIntent {
                side: OrderSide::Sell,
                reduce_only: true,
                ..
            }]
        ));
        assert_eq!(exit.next_state.position, ThresholdSwingPosition::Flat);
    }

    #[test]
    fn venue_fill_replaces_estimated_position_with_delivered_satoshis() {
        let config = config();
        let initial = ThresholdSwingProgram.initial_state(&config).expect("state");
        let entry = ThresholdSwingProgram
            .on_tick(&config, &initial, &tick(90.0, -0.03, 0.02))
            .expect("entry");
        let intent = entry.intents.first().expect("buy intent");
        let execution = VenueExecution {
            venue_order_id: "lnmarkets-swap:test".into(),
            ledger_entries: vec![trading_ledger::LedgerEntryDraft {
                event_id: "lnmarkets-swap-fill:test".into(),
                occurred_at_ms: 100,
                strategy_id: STRATEGY_ID.into(),
                kind: trading_ledger::LedgerEntryKind::Fill,
                postings: Vec::new(),
                metadata: json!({
                    "schema": FILL_SCHEMA,
                    "swap": {
                        "inAmount": 100.00,
                        "inAsset": "USD",
                        "outAmount": 1234,
                        "outAsset": "BTC"
                    }
                }),
            }],
        };
        let reconciled = ThresholdSwingProgram
            .on_execution(&config, &entry.next_state, intent, &execution)
            .expect("reconciled fill");
        assert!(matches!(
            reconciled.position,
            ThresholdSwingPosition::LongBitcoin {
                amount_sats: 1_234,
                input_usd_cents: 10_000,
                ..
            }
        ));

        let exit = ThresholdSwingProgram
            .on_tick(&config, &reconciled, &tick(100.0, 0.03, 0.02))
            .expect("exit");
        assert_eq!(exit.intents[0].quantity.amount, 1_234);
    }

    #[test]
    fn spread_signal_and_cost_hurdles_prevent_orders() {
        let config = config();
        let initial = ThresholdSwingProgram.initial_state(&config).expect("state");
        let weak = ThresholdSwingProgram
            .on_tick(&config, &initial, &tick(99.9, -0.001, 0.02))
            .expect("weak");
        assert!(weak.intents.is_empty());
        assert_eq!(
            weak.next_state.last_action,
            ThresholdSwingAction::SignalBelowThreshold
        );

        let mut expensive = config.clone();
        expensive.measured_round_trip_cost_bps = 400;
        let expensive_state = ThresholdSwingProgram
            .initial_state(&expensive)
            .expect("expensive state");
        let cost_blocked = ThresholdSwingProgram
            .on_tick(&expensive, &expensive_state, &tick(99.0, -0.03, 0.02))
            .expect("cost blocked");
        assert!(cost_blocked.intents.is_empty());
        assert_eq!(
            cost_blocked.next_state.last_action,
            ThresholdSwingAction::CostHurdleNotMet
        );

        let mut wide_features = features(90.0, -0.03, 0.02);
        wide_features.liquidity.spread_bps = Some(21.0);
        let wide = ThresholdSwingProgram
            .on_tick(
                &config,
                &initial,
                &StrategyTick {
                    occurred_at_ms: 100,
                    features: wide_features,
                },
            )
            .expect("wide spread");
        assert!(wide.intents.is_empty());
        assert_eq!(
            wide.next_state.last_action,
            ThresholdSwingAction::SpreadTooWide
        );
    }

    #[test]
    fn collected_feature_path_backtests_positive_expectancy_after_costs() {
        let config = config();
        let ticks = vec![
            BacktestTick {
                occurred_at_ms: 100,
                features: features(90.0, -0.03, 0.02),
            },
            BacktestTick {
                occurred_at_ms: 200,
                features: features(100.0, 0.03, 0.02),
            },
        ];
        let report = run_backtest(
            &ThresholdSwingProgram,
            &config,
            &ticks,
            &mut ThresholdSwingBacktestModel::default(),
            TradingNetwork::Signet,
            BacktestCostModel {
                taker_fee_bps: 1,
                observed_round_trip_cost_bps: 1,
                measurement_source: "signet ledger".into(),
                measured_at_ms: 1,
            },
            BacktestPolicy {
                minimum_trade_count: 2,
                minimum_expectancy_millisats: 1,
                maximum_drawdown_sats: 100_000,
            },
            300,
        )
        .expect("backtest");
        assert!(report.expectancy_millisats > 0);
        assert_eq!(report.outcome, BacktestOutcome::Passed);
    }

    #[test]
    fn mainnet_and_unbounded_parameters_are_rejected() {
        let mut config = config();
        config.network = Network::Mainnet;
        assert!(ThresholdSwingProgram.validate_config(&config).is_err());
        config.network = Network::Signet;
        config.maximum_position_usd_cents = 0;
        assert!(ThresholdSwingProgram.validate_config(&config).is_err());
    }
}
