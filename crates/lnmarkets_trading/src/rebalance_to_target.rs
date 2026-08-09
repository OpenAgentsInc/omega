use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use lnmarkets_client::{Asset, LnMarketsClient, Network, NewSwapRequest, NewSwapResult};
use lnmarkets_data::{
    AccountAllocation, AccountDriftFeatures, FeatureSnapshot, account_drift_features,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use strategy_engine::{
    BacktestExecutionModel, BacktestTick, OrderIntent, OrderKind, OrderQuantity, OrderSide,
    QuantityUnit, SimulatedTrade, StrategyProgram, StrategyStep, StrategyTick, VenueExecution,
    VenueExecutor, VenueRiskSnapshot,
};
use trading_ledger::{
    LedgerAccount, LedgerEntryDraft, LedgerEntryKind, LedgerPosting, LedgerQuery, LedgerStore,
};
use trading_mandate::TradingNetwork;

pub const REBALANCE_TO_TARGET_SCHEMA: &str = "omega.lnmarkets.rebalance_to_target.v1";
const STRATEGY_ID: &str = "rebalance_to_target";
const STRATEGY_VERSION: &str = "1";
const SYNTHETIC_USD_INSTRUMENT: &str = "lnmarkets.synthetic_usd";
const BASIS_POINTS_DENOMINATOR: u64 = 10_000;
const SATOSHIS_PER_BITCOIN: f64 = 100_000_000.0;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RebalanceCostMeasurement {
    pub observed_round_trip_cost_bps: u32,
    pub traded_notional_sats: u64,
    pub realized_cost_sats: u64,
    pub sample_count: u64,
    pub measured_at_ms: i64,
    pub source: String,
}

impl RebalanceCostMeasurement {
    fn validate(&self) -> Result<()> {
        if self.observed_round_trip_cost_bps > 10_000 {
            bail!("observed round-trip cost must not exceed 10000 basis points");
        }
        if self.traded_notional_sats == 0 || self.sample_count == 0 {
            bail!("observed round-trip cost requires at least one traded ledger sample");
        }
        if self.realized_cost_sats > self.traded_notional_sats {
            bail!("observed round-trip cost must not exceed traded notional");
        }
        if self.measured_at_ms < 0 {
            bail!("observed round-trip cost timestamp must not be negative");
        }
        if self.source.trim().is_empty() || self.source.len() > 500 {
            bail!("observed round-trip cost source must contain from 1 through 500 bytes");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RebalanceToTargetConfig {
    pub network: Network,
    pub target_synthetic_usd_weight_bps: u32,
    pub drift_threshold_bps: u32,
    pub cost_margin_bps: u32,
    pub maximum_order_value_usd_cents: u64,
    pub liquidity_utilization_bps: u32,
    pub cost_measurement: RebalanceCostMeasurement,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RebalanceAction {
    WaitingForAccountData,
    WithinDriftThreshold,
    CostHurdleNotMet,
    WaitingForLiquidity,
    OrderBelowVenueMinimum,
    SellBitcoin { amount_sats: u64 },
    BuyBitcoin { amount_usd_cents: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RebalanceToTargetState {
    pub schema: String,
    pub sequence: u64,
    pub target_synthetic_usd_weight_bps: u32,
    pub current_synthetic_usd_weight_bps: Option<u32>,
    pub drift_bps: Option<i32>,
    pub expected_correction_value_usd_cents: u64,
    pub planned_order_value_usd_cents: u64,
    pub realized_cost_bps: u32,
    pub hurdle_bps: u32,
    pub last_action: RebalanceAction,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RebalanceToTargetProgram;

impl StrategyProgram for RebalanceToTargetProgram {
    type Config = RebalanceToTargetConfig;
    type State = RebalanceToTargetState;
    type Features = FeatureSnapshot;

    fn strategy_id(&self) -> &'static str {
        STRATEGY_ID
    }

    fn strategy_version(&self) -> &'static str {
        STRATEGY_VERSION
    }

    fn validate_config(&self, config: &Self::Config) -> Result<()> {
        if config.network != Network::Signet {
            bail!("rebalance_to_target is restricted to signet");
        }
        if !(1..10_000).contains(&config.target_synthetic_usd_weight_bps) {
            bail!("target synthetic USD weight must be from 1 through 9999 basis points");
        }
        if !(1..10_000).contains(&config.drift_threshold_bps) {
            bail!("rebalance drift threshold must be from 1 through 9999 basis points");
        }
        if config.cost_margin_bps > 10_000 {
            bail!("rebalance cost margin must not exceed 10000 basis points");
        }
        let hurdle_bps = config
            .cost_measurement
            .observed_round_trip_cost_bps
            .checked_add(config.cost_margin_bps)
            .context("rebalance cost hurdle overflowed")?;
        if hurdle_bps > 10_000 {
            bail!("rebalance cost hurdle must not exceed 10000 basis points");
        }
        if config.maximum_order_value_usd_cents == 0 {
            bail!("rebalance maximum order value must be greater than zero");
        }
        if !(1..=10_000).contains(&config.liquidity_utilization_bps) {
            bail!("rebalance liquidity utilization must be from 1 through 10000 basis points");
        }
        config.cost_measurement.validate()
    }

    fn initial_state(&self, config: &Self::Config) -> Result<Self::State> {
        self.validate_config(config)?;
        Ok(RebalanceToTargetState {
            schema: REBALANCE_TO_TARGET_SCHEMA.into(),
            sequence: 0,
            target_synthetic_usd_weight_bps: config.target_synthetic_usd_weight_bps,
            current_synthetic_usd_weight_bps: None,
            drift_bps: None,
            expected_correction_value_usd_cents: 0,
            planned_order_value_usd_cents: 0,
            realized_cost_bps: config.cost_measurement.observed_round_trip_cost_bps,
            hurdle_bps: config
                .cost_measurement
                .observed_round_trip_cost_bps
                .checked_add(config.cost_margin_bps)
                .context("rebalance cost hurdle overflowed")?,
            last_action: RebalanceAction::WaitingForAccountData,
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
            .context("rebalance sequence overflowed")?;
        let hurdle_bps = config
            .cost_measurement
            .observed_round_trip_cost_bps
            .checked_add(config.cost_margin_bps)
            .context("rebalance cost hurdle overflowed")?;
        let Some(account) = tick.features.account_drift.as_ref() else {
            return Ok(step_without_order(
                config,
                sequence,
                None,
                None,
                0,
                0,
                hurdle_bps,
                RebalanceAction::WaitingForAccountData,
            ));
        };
        validate_account(account)?;
        let total_value_usd = account.btc_value_usd + account.synthetic_usd;
        let current_synthetic_weight = account.synthetic_usd / total_value_usd;
        let target_synthetic_weight = f64::from(config.target_synthetic_usd_weight_bps) / 10_000.0;
        let signed_drift = current_synthetic_weight - target_synthetic_weight;
        let absolute_drift = signed_drift.abs();
        let current_weight_bps = rounded_basis_points(current_synthetic_weight)?;
        let drift_bps = signed_basis_points(signed_drift)?;
        let threshold = f64::from(config.drift_threshold_bps) / 10_000.0;
        if absolute_drift <= threshold {
            return Ok(step_without_order(
                config,
                sequence,
                Some(current_weight_bps),
                Some(drift_bps),
                0,
                0,
                hurdle_bps,
                RebalanceAction::WithinDriftThreshold,
            ));
        }

        let correction_value_usd = absolute_drift * total_value_usd;
        let maximum_order_usd = config.maximum_order_value_usd_cents as f64 / 100.0;
        let depth = if signed_drift > 0.0 {
            tick.features.liquidity.ask_depth
        } else {
            tick.features.liquidity.bid_depth
        };
        let ladder_limit_usd = depth.map(|depth| {
            depth * f64::from(config.liquidity_utilization_bps) / BASIS_POINTS_DENOMINATOR as f64
        });
        let planned_order_usd = correction_value_usd
            .min(maximum_order_usd)
            .min(ladder_limit_usd.unwrap_or(f64::MAX));
        if !planned_order_usd.is_finite() || planned_order_usd <= 0.0 {
            return Ok(step_without_order(
                config,
                sequence,
                Some(current_weight_bps),
                Some(drift_bps),
                0,
                0,
                hurdle_bps,
                RebalanceAction::WaitingForLiquidity,
            ));
        }
        let expected_correction_usd =
            ((absolute_drift - threshold) * total_value_usd).min(planned_order_usd);
        let hurdle_usd = planned_order_usd * f64::from(hurdle_bps) / 10_000.0;
        let expected_correction_value_usd_cents = floor_u64(expected_correction_usd * 100.0)?;
        let planned_order_value_usd_cents = floor_u64(planned_order_usd * 100.0)?;
        if expected_correction_usd <= hurdle_usd {
            return Ok(step_without_order(
                config,
                sequence,
                Some(current_weight_bps),
                Some(drift_bps),
                expected_correction_value_usd_cents,
                planned_order_value_usd_cents,
                hurdle_bps,
                RebalanceAction::CostHurdleNotMet,
            ));
        }

        let (side, quantity, action) = if signed_drift > 0.0 {
            let amount_usd_cents = planned_order_value_usd_cents;
            if amount_usd_cents == 0 {
                return Ok(step_without_order(
                    config,
                    sequence,
                    Some(current_weight_bps),
                    Some(drift_bps),
                    expected_correction_value_usd_cents,
                    planned_order_value_usd_cents,
                    hurdle_bps,
                    RebalanceAction::OrderBelowVenueMinimum,
                ));
            }
            (
                OrderSide::Buy,
                OrderQuantity {
                    amount: amount_usd_cents,
                    unit: QuantityUnit::UsdCents,
                },
                RebalanceAction::BuyBitcoin { amount_usd_cents },
            )
        } else {
            let Some(bid_price) = tick.features.liquidity.best_bid else {
                return Ok(step_without_order(
                    config,
                    sequence,
                    Some(current_weight_bps),
                    Some(drift_bps),
                    expected_correction_value_usd_cents,
                    planned_order_value_usd_cents,
                    hurdle_bps,
                    RebalanceAction::WaitingForLiquidity,
                ));
            };
            if !bid_price.is_finite() || bid_price <= 0.0 {
                bail!("rebalance bid price must be a positive finite number");
            }
            let amount_sats = floor_u64(planned_order_usd / bid_price * SATOSHIS_PER_BITCOIN)?;
            if amount_sats < 1_000 {
                return Ok(step_without_order(
                    config,
                    sequence,
                    Some(current_weight_bps),
                    Some(drift_bps),
                    expected_correction_value_usd_cents,
                    planned_order_value_usd_cents,
                    hurdle_bps,
                    RebalanceAction::OrderBelowVenueMinimum,
                ));
            }
            (
                OrderSide::Sell,
                OrderQuantity {
                    amount: amount_sats,
                    unit: QuantityUnit::Sats,
                },
                RebalanceAction::SellBitcoin { amount_sats },
            )
        };
        let next_state = RebalanceToTargetState {
            schema: REBALANCE_TO_TARGET_SCHEMA.into(),
            sequence,
            target_synthetic_usd_weight_bps: config.target_synthetic_usd_weight_bps,
            current_synthetic_usd_weight_bps: Some(current_weight_bps),
            drift_bps: Some(drift_bps),
            expected_correction_value_usd_cents,
            planned_order_value_usd_cents,
            realized_cost_bps: config.cost_measurement.observed_round_trip_cost_bps,
            hurdle_bps,
            last_action: action,
        };
        let intent = OrderIntent {
            intent_id: format!("{STRATEGY_ID}:{}:{sequence}", tick.occurred_at_ms),
            instrument: SYNTHETIC_USD_INSTRUMENT.into(),
            side,
            kind: OrderKind::Market,
            quantity,
            limit_price: None,
            reduce_only: false,
            protection: None,
            metadata: json!({
                "schema": REBALANCE_TO_TARGET_SCHEMA,
                "occurred_at_ms": tick.occurred_at_ms,
                "target_synthetic_usd_weight_bps": config.target_synthetic_usd_weight_bps,
                "current_synthetic_usd_weight_bps": current_weight_bps,
                "drift_bps": drift_bps,
                "expected_correction_value_usd_cents": expected_correction_value_usd_cents,
                "planned_order_value_usd_cents": planned_order_value_usd_cents,
                "realized_cost_bps": config.cost_measurement.observed_round_trip_cost_bps,
                "hurdle_bps": hurdle_bps,
                "cost_measurement_source": config.cost_measurement.source,
                "cost_measured_at_ms": config.cost_measurement.measured_at_ms,
            }),
        };
        intent.validate()?;
        Ok(StrategyStep {
            next_state,
            intents: vec![intent],
        })
    }
}

fn step_without_order(
    config: &RebalanceToTargetConfig,
    sequence: u64,
    current_synthetic_usd_weight_bps: Option<u32>,
    drift_bps: Option<i32>,
    expected_correction_value_usd_cents: u64,
    planned_order_value_usd_cents: u64,
    hurdle_bps: u32,
    last_action: RebalanceAction,
) -> StrategyStep<RebalanceToTargetState> {
    StrategyStep {
        next_state: RebalanceToTargetState {
            schema: REBALANCE_TO_TARGET_SCHEMA.into(),
            sequence,
            target_synthetic_usd_weight_bps: config.target_synthetic_usd_weight_bps,
            current_synthetic_usd_weight_bps,
            drift_bps,
            expected_correction_value_usd_cents,
            planned_order_value_usd_cents,
            realized_cost_bps: config.cost_measurement.observed_round_trip_cost_bps,
            hurdle_bps,
            last_action,
        },
        intents: Vec::new(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RebalanceBacktestModel {
    allocation: AccountAllocation,
}

impl RebalanceBacktestModel {
    pub fn new(allocation: AccountAllocation) -> Result<Self> {
        Ok(Self {
            allocation: allocation.validate()?,
        })
    }

    fn price(features: &FeatureSnapshot) -> Result<f64> {
        let price = features
            .index
            .current_price
            .or(features.liquidity.best_ask)
            .or(features.liquidity.best_bid)
            .context("rebalance backtest has no price")?;
        if !price.is_finite() || price <= 0.0 {
            bail!("rebalance backtest price must be positive and finite");
        }
        Ok(price)
    }
}

impl BacktestExecutionModel<FeatureSnapshot> for RebalanceBacktestModel {
    fn prepare_tick(
        &mut self,
        tick: &BacktestTick<FeatureSnapshot>,
    ) -> Result<BacktestTick<FeatureSnapshot>> {
        let mut tick = tick.clone();
        let price = Self::price(&tick.features)?;
        tick.features.account_drift = account_drift_features(self.allocation, price);
        Ok(tick)
    }

    fn execute(
        &mut self,
        intent: &OrderIntent,
        tick: &BacktestTick<FeatureSnapshot>,
    ) -> Result<SimulatedTrade> {
        let price = Self::price(&tick.features)?;
        let notional_sats = match (intent.side, intent.quantity.unit) {
            (OrderSide::Sell, QuantityUnit::Sats) => {
                if intent.quantity.amount as f64 > self.allocation.btc_sats {
                    bail!("rebalance backtest sell exceeds its simulated BTC balance");
                }
                self.allocation.btc_sats -= intent.quantity.amount as f64;
                self.allocation.synthetic_usd +=
                    intent.quantity.amount as f64 / SATOSHIS_PER_BITCOIN * price;
                intent.quantity.amount
            }
            (OrderSide::Buy, QuantityUnit::UsdCents) => {
                let amount_usd = intent.quantity.amount as f64 / 100.0;
                if amount_usd > self.allocation.synthetic_usd {
                    bail!("rebalance backtest buy exceeds its simulated synthetic USD balance");
                }
                self.allocation.synthetic_usd -= amount_usd;
                let amount_sats = floor_u64(amount_usd / price * SATOSHIS_PER_BITCOIN)?;
                self.allocation.btc_sats += amount_sats as f64;
                amount_sats
            }
            _ => bail!("rebalance backtest received an unsupported intent"),
        };
        Ok(SimulatedTrade {
            gross_profit_sats: 0,
            notional_sats,
            funding_sats: 0,
            counts_as_trade: true,
        })
    }
}

#[derive(Clone)]
pub struct SyntheticUsdExecutor {
    client: Arc<LnMarketsClient>,
    network: Network,
    strategy_id: &'static str,
    fill_schema: &'static str,
    cost_schema: &'static str,
    strategy_label: &'static str,
}

impl SyntheticUsdExecutor {
    pub fn new(client: LnMarketsClient, network: Network) -> Result<Self> {
        Self::for_strategy(
            client,
            network,
            STRATEGY_ID,
            "omega.lnmarkets.rebalance_fill.v1",
            "omega.lnmarkets.rebalance_cost.v1",
            STRATEGY_ID,
        )
    }

    pub(crate) fn for_strategy(
        client: LnMarketsClient,
        network: Network,
        strategy_id: &'static str,
        fill_schema: &'static str,
        cost_schema: &'static str,
        strategy_label: &'static str,
    ) -> Result<Self> {
        if network != Network::Signet || client.network() != Network::Signet {
            bail!("{strategy_label} execution is restricted to a signet client");
        }
        Ok(Self {
            client: Arc::new(client),
            network,
            strategy_id,
            fill_schema,
            cost_schema,
            strategy_label,
        })
    }

    fn validate_intent<'a>(&self, intent: &'a OrderIntent) -> Result<&'a OrderQuantity> {
        if self.network != Network::Signet || self.client.network() != Network::Signet {
            bail!("{} execution is restricted to signet", self.strategy_label);
        }
        intent.validate()?;
        if intent.instrument != SYNTHETIC_USD_INSTRUMENT || intent.kind != OrderKind::Market {
            bail!("rebalance executor accepts only LN Markets synthetic USD market swaps");
        }
        match (intent.side, intent.quantity.unit) {
            (OrderSide::Sell, QuantityUnit::Sats) | (OrderSide::Buy, QuantityUnit::UsdCents) => {
                Ok(&intent.quantity)
            }
            _ => bail!("rebalance swap side and quantity unit do not match"),
        }
    }
}

#[async_trait]
impl VenueExecutor for SyntheticUsdExecutor {
    async fn preview(&self, intent: &OrderIntent) -> Result<VenueRiskSnapshot> {
        self.validate_intent(intent)?;
        let account = self.client.account().await?;
        let prices = self.client.best_price().await?;
        let btc_balance_sats = decimal_non_negative_f64(&account.balance, "account BTC balance")?;
        let synthetic_usd =
            decimal_non_negative_f64(&account.synthetic_usd_balance, "synthetic USD balance")?;
        let bid_price = decimal_f64(&prices.bid_price, "synthetic USD bid price")?;
        let synthetic_value_sats = synthetic_usd / bid_price * SATOSHIS_PER_BITCOIN;
        let venue_balance_after_sats = ceil_u64(btc_balance_sats + synthetic_value_sats)?;
        Ok(VenueRiskSnapshot {
            network: TradingNetwork::Signet,
            venue: "lnmarkets".into(),
            venue_balance_after_sats,
            position_notional_before_usd: 0,
            position_notional_after_usd: 0,
            leverage: 1,
            liquidation_buffer_bps: 10_000,
        })
    }

    async fn execute_once(&self, intent: &OrderIntent) -> Result<VenueExecution> {
        let quantity = self.validate_intent(intent)?;
        let prices = self.client.best_price().await?;
        let request = match intent.side {
            OrderSide::Sell => NewSwapRequest::bitcoin_to_synthetic_usd(quantity.amount)?,
            OrderSide::Buy => NewSwapRequest::synthetic_usd_to_bitcoin(quantity.amount)?,
        };
        let result = self.client.new_swap(&request).await?;
        let measurement = execution_measurement(intent, &result, &prices)?;
        let result_json = serde_json::to_value(&result)?;
        let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&result)?));
        let occurred_at_ms = intent
            .metadata
            .get("occurred_at_ms")
            .and_then(Value::as_i64)
            .context("rebalance intent has no occurrence timestamp")?;
        let mut ledger_entries = vec![LedgerEntryDraft {
            event_id: format!("lnmarkets-swap-fill:{digest}"),
            occurred_at_ms,
            strategy_id: self.strategy_id.into(),
            kind: LedgerEntryKind::Fill,
            postings: Vec::new(),
            metadata: json!({
                "schema": self.fill_schema,
                "intent_id": intent.intent_id,
                "input_value_sats": measurement.input_value_sats,
                "output_value_sats": measurement.output_value_sats,
                "realized_cost_sats": measurement.realized_cost_sats,
                "realized_cost_bps": measurement.realized_cost_bps,
                "swap": result_json,
            }),
        }];
        if measurement.realized_cost_sats > 0 {
            let realized_cost_sats = i64::try_from(measurement.realized_cost_sats)
                .context("rebalance realized cost exceeded ledger range")?;
            ledger_entries.push(LedgerEntryDraft {
                event_id: format!("lnmarkets-swap-cost:{digest}"),
                occurred_at_ms,
                strategy_id: self.strategy_id.into(),
                kind: LedgerEntryKind::Fee,
                postings: vec![
                    LedgerPosting {
                        account: LedgerAccount::VenueBalance {
                            venue: "lnmarkets".into(),
                        },
                        amount_sats: -realized_cost_sats,
                    },
                    LedgerPosting {
                        account: LedgerAccount::FeeExpense,
                        amount_sats: realized_cost_sats,
                    },
                ],
                metadata: json!({
                    "schema": self.cost_schema,
                    "intent_id": intent.intent_id,
                    "input_value_sats": measurement.input_value_sats,
                    "realized_cost_sats": measurement.realized_cost_sats,
                    "realized_cost_bps": measurement.realized_cost_bps,
                }),
            });
        }
        Ok(VenueExecution {
            venue_order_id: format!("lnmarkets-swap:{digest}"),
            ledger_entries,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExecutionMeasurement {
    input_value_sats: u64,
    output_value_sats: u64,
    realized_cost_sats: u64,
    realized_cost_bps: u32,
}

fn execution_measurement(
    intent: &OrderIntent,
    result: &NewSwapResult,
    prices: &lnmarkets_client::BestPrice,
) -> Result<ExecutionMeasurement> {
    let bid_price = decimal_f64(&prices.bid_price, "synthetic USD bid price")?;
    let ask_price = decimal_f64(&prices.ask_price, "synthetic USD ask price")?;
    let (input_value_sats, output_value_sats) = match intent.side {
        OrderSide::Sell => {
            if result.in_asset != Asset::BTC || result.out_asset != Asset::USD {
                bail!("LN Markets returned the wrong assets for a BTC to USD swap");
            }
            let input_sats = decimal_f64(&result.in_amount, "swap BTC input")?;
            let output_usd = decimal_f64(&result.out_amount, "swap USD output")?;
            (
                floor_u64(input_sats)?,
                floor_u64(output_usd / bid_price * SATOSHIS_PER_BITCOIN)?,
            )
        }
        OrderSide::Buy => {
            if result.in_asset != Asset::USD || result.out_asset != Asset::BTC {
                bail!("LN Markets returned the wrong assets for a USD to BTC swap");
            }
            let input_usd = decimal_f64(&result.in_amount, "swap USD input")?;
            let output_sats = decimal_f64(&result.out_amount, "swap BTC output")?;
            (
                ceil_u64(input_usd / ask_price * SATOSHIS_PER_BITCOIN)?,
                floor_u64(output_sats)?,
            )
        }
    };
    let realized_cost_sats = input_value_sats.saturating_sub(output_value_sats);
    let realized_cost_bps = ratio_basis_points(realized_cost_sats, input_value_sats)?;
    Ok(ExecutionMeasurement {
        input_value_sats,
        output_value_sats,
        realized_cost_sats,
        realized_cost_bps,
    })
}

pub fn measure_rebalance_cost(
    ledger: &LedgerStore,
    from_ms: i64,
    to_ms: i64,
) -> Result<RebalanceCostMeasurement> {
    if from_ms < 0 || to_ms < from_ms {
        bail!("rebalance cost measurement range is invalid");
    }
    let entries = ledger.entries(&LedgerQuery {
        from_ms: Some(from_ms),
        to_ms: Some(to_ms),
        strategy_id: Some(STRATEGY_ID.into()),
    })?;
    let mut traded_notional_sats = 0_u64;
    let mut realized_cost_sats = 0_u64;
    let mut sample_count = 0_u64;
    let mut measured_at_ms = 0_i64;
    for entry in entries {
        if !matches!(entry.kind, LedgerEntryKind::Fill)
            || entry.metadata.get("schema").and_then(Value::as_str)
                != Some("omega.lnmarkets.rebalance_fill.v1")
        {
            continue;
        }
        let input_value_sats = entry
            .metadata
            .get("input_value_sats")
            .and_then(Value::as_u64)
            .context("rebalance fill is missing input value")?;
        let cost_sats = entry
            .metadata
            .get("realized_cost_sats")
            .and_then(Value::as_u64)
            .context("rebalance fill is missing realized cost")?;
        traded_notional_sats = traded_notional_sats
            .checked_add(input_value_sats)
            .context("rebalance measured notional overflowed")?;
        realized_cost_sats = realized_cost_sats
            .checked_add(cost_sats)
            .context("rebalance measured cost overflowed")?;
        sample_count = sample_count
            .checked_add(1)
            .context("rebalance cost sample count overflowed")?;
        measured_at_ms = measured_at_ms.max(entry.occurred_at_ms);
    }
    if sample_count == 0 || traded_notional_sats == 0 {
        bail!("rebalance cost measurement requires recorded swap fills");
    }
    let measurement = RebalanceCostMeasurement {
        observed_round_trip_cost_bps: ratio_basis_points(realized_cost_sats, traded_notional_sats)?,
        traded_notional_sats,
        realized_cost_sats,
        sample_count,
        measured_at_ms,
        source: "trading_ledger:rebalance_to_target".into(),
    };
    measurement.validate()?;
    Ok(measurement)
}

fn validate_account(account: &AccountDriftFeatures) -> Result<()> {
    if !account.btc_value_usd.is_finite()
        || account.btc_value_usd < 0.0
        || !account.synthetic_usd.is_finite()
        || account.synthetic_usd < 0.0
        || account.btc_value_usd + account.synthetic_usd <= 0.0
    {
        bail!("rebalance account values must be non-negative finite values with positive total");
    }
    Ok(())
}

fn rounded_basis_points(weight: f64) -> Result<u32> {
    if !weight.is_finite() || !(0.0..=1.0).contains(&weight) {
        bail!("rebalance account weight must be between zero and one");
    }
    let value = (weight * 10_000.0).round();
    u32::try_from(value as i64).context("rebalance account weight exceeded supported range")
}

fn signed_basis_points(weight: f64) -> Result<i32> {
    if !weight.is_finite() || !(-1.0..=1.0).contains(&weight) {
        bail!("rebalance drift must be between negative one and one");
    }
    let value = (weight * 10_000.0).round();
    i32::try_from(value as i64).context("rebalance drift exceeded supported range")
}

fn decimal_f64(value: &lnmarkets_client::DecimalAmount, label: &str) -> Result<f64> {
    let value = value
        .as_number()
        .as_f64()
        .with_context(|| format!("{label} is outside the supported numeric range"))?;
    if !value.is_finite() || value <= 0.0 {
        bail!("{label} must be a positive finite number");
    }
    Ok(value)
}

fn decimal_non_negative_f64(value: &lnmarkets_client::DecimalAmount, label: &str) -> Result<f64> {
    let value = value
        .as_number()
        .as_f64()
        .with_context(|| format!("{label} is outside the supported numeric range"))?;
    if !value.is_finite() || value < 0.0 {
        bail!("{label} must be a non-negative finite number");
    }
    Ok(value)
}

fn floor_u64(value: f64) -> Result<u64> {
    if !value.is_finite() || value < 0.0 || value > u64::MAX as f64 {
        bail!("rebalance amount is outside the supported range");
    }
    Ok(value.floor() as u64)
}

fn ceil_u64(value: f64) -> Result<u64> {
    if !value.is_finite() || value < 0.0 || value > u64::MAX as f64 {
        bail!("rebalance amount is outside the supported range");
    }
    Ok(value.ceil() as u64)
}

fn ratio_basis_points(numerator: u64, denominator: u64) -> Result<u32> {
    if denominator == 0 {
        bail!("rebalance cost ratio denominator must be greater than zero");
    }
    let basis_points = u128::from(numerator)
        .checked_mul(u128::from(BASIS_POINTS_DENOMINATOR))
        .and_then(|value| value.checked_add(u128::from(denominator) - 1))
        .map(|value| value / u128::from(denominator))
        .context("rebalance cost ratio overflowed")?;
    u32::try_from(basis_points).context("rebalance cost ratio exceeded supported range")
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use futures::{FutureExt as _, executor::block_on};
    use http::{Request, Response, StatusCode};
    use lnmarkets_client::{Credentials, HttpTransport, TransportFailure};
    use lnmarkets_data::{
        FundingFeatures, FundingSign, IndexFeatures, LiquidityFeatures, VolatilityFeatures,
    };

    use super::*;

    fn measurement(cost_bps: u32) -> RebalanceCostMeasurement {
        RebalanceCostMeasurement {
            observed_round_trip_cost_bps: cost_bps,
            traded_notional_sats: 300_000,
            realized_cost_sats: u64::from(cost_bps) * 30,
            sample_count: 3,
            measured_at_ms: 10,
            source: "signet ledger sample".into(),
        }
    }

    fn config(target_bps: u32) -> RebalanceToTargetConfig {
        RebalanceToTargetConfig {
            network: Network::Signet,
            target_synthetic_usd_weight_bps: target_bps,
            drift_threshold_bps: 100,
            cost_margin_bps: 50,
            maximum_order_value_usd_cents: 5_000,
            liquidity_utilization_bps: 10_000,
            cost_measurement: measurement(60),
        }
    }

    fn features(synthetic_usd: f64) -> FeatureSnapshot {
        let btc_value_usd = 100.0;
        let total = btc_value_usd + synthetic_usd;
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
                current_rate: None,
                ema: None,
                sign: FundingSign::Neutral,
                sign_flipped_at_ms: None,
                samples: 0,
            },
            liquidity: LiquidityFeatures {
                best_bid: Some(50_000.0),
                best_ask: Some(50_010.0),
                spread: Some(10.0),
                spread_bps: Some(2.0),
                bid_depth: Some(100.0),
                ask_depth: Some(100.0),
                tier_count: 1,
            },
            account_drift: Some(AccountDriftFeatures {
                btc_value_usd,
                synthetic_usd,
                current_btc_weight: btc_value_usd / total,
                target_btc_weight: 0.5,
                drift: btc_value_usd / total - 0.5,
            }),
        }
    }

    fn tick(features: FeatureSnapshot) -> StrategyTick<FeatureSnapshot> {
        StrategyTick {
            occurred_at_ms: 100,
            features,
        }
    }

    #[test]
    fn target_rebalance_sells_btc_only_after_drift_and_cost_hurdles() {
        let program = RebalanceToTargetProgram;
        let config = config(6_000);
        let state = program.initial_state(&config).expect("state");
        let step = program
            .on_tick(&config, &state, &tick(features(100.0)))
            .expect("step");
        assert!(matches!(
            step.next_state.last_action,
            RebalanceAction::SellBitcoin {
                amount_sats: 39_999
            }
        ));
        assert!(matches!(
            step.intents.as_slice(),
            [OrderIntent {
                side: OrderSide::Sell,
                quantity: OrderQuantity {
                    amount: 39_999,
                    unit: QuantityUnit::Sats
                },
                ..
            }]
        ));

        let mut expensive = config;
        expensive.target_synthetic_usd_weight_bps = 5_060;
        expensive.drift_threshold_bps = 50;
        expensive.cost_margin_bps = 0;
        expensive.cost_measurement = measurement(9_000);
        let state = program.initial_state(&expensive).expect("expensive state");
        let step = program
            .on_tick(&expensive, &state, &tick(features(100.0)))
            .expect("expensive step");
        assert!(step.intents.is_empty());
        assert_eq!(
            step.next_state.last_action,
            RebalanceAction::CostHurdleNotMet
        );
    }

    #[test]
    fn target_rebalance_buys_btc_and_caps_size_at_the_ladder() {
        let program = RebalanceToTargetProgram;
        let mut config = config(4_000);
        config.liquidity_utilization_bps = 5_000;
        let mut features = features(100.0);
        features.liquidity.ask_depth = Some(10.0);
        let state = program.initial_state(&config).expect("state");
        let step = program
            .on_tick(&config, &state, &tick(features))
            .expect("step");
        assert!(matches!(
            step.intents.as_slice(),
            [OrderIntent {
                side: OrderSide::Buy,
                quantity: OrderQuantity {
                    amount: 500,
                    unit: QuantityUnit::UsdCents
                },
                ..
            }]
        ));
        assert_eq!(step.next_state.planned_order_value_usd_cents, 500);
    }

    #[test]
    fn target_rebalance_waits_inside_the_drift_threshold() {
        let program = RebalanceToTargetProgram;
        let config = config(5_050);
        let state = program.initial_state(&config).expect("state");
        let step = program
            .on_tick(&config, &state, &tick(features(100.0)))
            .expect("step");
        assert!(step.intents.is_empty());
        assert_eq!(
            step.next_state.last_action,
            RebalanceAction::WithinDriftThreshold
        );
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

        fn post_count(&self) -> usize {
            self.requests
                .lock()
                .expect("requests")
                .iter()
                .filter(|request| request.method() == http::Method::POST)
                .count()
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

    fn response(body: &str) -> Response<Vec<u8>> {
        Response::builder()
            .status(StatusCode::OK)
            .body(body.as_bytes().to_vec())
            .expect("response")
    }

    fn authenticated_client(transport: Arc<FakeTransport>) -> LnMarketsClient {
        LnMarketsClient::authenticated(
            transport,
            Network::Signet,
            Credentials::new("key", "secret", "passphrase").expect("credentials"),
        )
    }

    fn sell_intent() -> OrderIntent {
        OrderIntent {
            intent_id: "rebalance:100:1".into(),
            instrument: SYNTHETIC_USD_INSTRUMENT.into(),
            side: OrderSide::Sell,
            kind: OrderKind::Market,
            quantity: OrderQuantity {
                amount: 1_000,
                unit: QuantityUnit::Sats,
            },
            limit_price: None,
            reduce_only: false,
            protection: None,
            metadata: json!({ "occurred_at_ms": 100 }),
        }
    }

    #[test]
    fn signet_executor_previews_then_submits_one_swap_and_records_cost() {
        block_on(async {
            let transport = Arc::new(FakeTransport::new(vec![
                response(
                    r#"{"balance":1000000,"email":null,"feeTier":0,"id":"id","linkingPublicKey":null,"syntheticUsdBalance":100,"username":"user"}"#,
                ),
                response(r#"{"askPrice":50010,"bidPrice":50000}"#),
                response(r#"{"askPrice":50010,"bidPrice":50000}"#),
                response(r#"{"inAmount":1000,"inAsset":"BTC","outAmount":0.49,"outAsset":"USD"}"#),
            ]));
            let executor =
                SyntheticUsdExecutor::new(authenticated_client(transport.clone()), Network::Signet)
                    .expect("executor");
            let intent = sell_intent();
            let preview = executor.preview(&intent).await.expect("preview");
            assert_eq!(preview.network, TradingNetwork::Signet);
            assert_eq!(preview.leverage, 1);
            let execution = executor.execute_once(&intent).await.expect("execute");
            assert_eq!(transport.post_count(), 1);
            assert_eq!(execution.ledger_entries.len(), 2);
            assert_eq!(
                execution.ledger_entries[0].metadata["realized_cost_sats"],
                21
            );
        });
    }

    #[test]
    fn shared_swap_executor_preserves_strategy_attribution_and_schema() {
        block_on(async {
            let transport = Arc::new(FakeTransport::new(vec![
                response(r#"{"askPrice":50010,"bidPrice":50000}"#),
                response(r#"{"inAmount":1000,"inAsset":"BTC","outAmount":0.49,"outAsset":"USD"}"#),
            ]));
            let executor = SyntheticUsdExecutor::for_strategy(
                authenticated_client(transport.clone()),
                Network::Signet,
                "threshold_swing",
                "omega.lnmarkets.threshold_swing_fill.v1",
                "omega.lnmarkets.threshold_swing_cost.v1",
                "threshold_swing",
            )
            .expect("threshold executor");
            let execution = executor
                .execute_once(&sell_intent())
                .await
                .expect("execution");

            assert_eq!(transport.post_count(), 1);
            assert!(execution.ledger_entries.iter().all(|entry| {
                entry.strategy_id == "threshold_swing"
                    && entry.metadata["schema"].as_str().is_some_and(|schema| {
                        schema.starts_with("omega.lnmarkets.threshold_swing_")
                    })
            }));
        });
    }

    #[test]
    fn executor_refuses_mainnet_before_any_request() {
        let transport = Arc::new(FakeTransport::new(Vec::new()));
        let client = LnMarketsClient::authenticated(
            transport.clone(),
            Network::Mainnet,
            Credentials::new("key", "secret", "passphrase").expect("credentials"),
        );
        assert!(SyntheticUsdExecutor::new(client, Network::Mainnet).is_err());
        assert!(transport.requests.lock().expect("requests").is_empty());

        let mut config = config(5_000);
        config.network = Network::Mainnet;
        assert!(RebalanceToTargetProgram.validate_config(&config).is_err());
    }

    #[test]
    fn realized_cost_is_remeasured_from_the_strategy_ledger() {
        let ledger = LedgerStore::in_memory().expect("ledger");
        for (event_id, occurred_at_ms, input, cost) in
            [("fill-1", 100, 100_000, 60), ("fill-2", 200, 200_000, 120)]
        {
            ledger
                .append(LedgerEntryDraft {
                    event_id: event_id.into(),
                    occurred_at_ms,
                    strategy_id: STRATEGY_ID.into(),
                    kind: LedgerEntryKind::Fill,
                    postings: Vec::new(),
                    metadata: json!({
                        "schema": "omega.lnmarkets.rebalance_fill.v1",
                        "input_value_sats": input,
                        "realized_cost_sats": cost,
                    }),
                })
                .expect("fill");
        }
        let measured = measure_rebalance_cost(&ledger, 0, 300).expect("measurement");
        assert_eq!(measured.observed_round_trip_cost_bps, 6);
        assert_eq!(measured.sample_count, 2);
        assert_eq!(measured.measured_at_ms, 200);
    }
}
