use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use lnmarkets_client::{
    FuturesCrossNewOrderRequest, FuturesCrossOrderQuantity, FuturesCrossTransferRequest,
    FuturesLeverage, FuturesTradeSide, LnMarketsClient, Network, NewSwapRequest, Pagination,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use trading_ledger::{
    LedgerAccount, LedgerEntryDraft, LedgerEntryKind, LedgerPosting, LedgerQuery, LedgerStore,
    ProfitReport,
};

pub const CONFIG_SCHEMA: &str = "openagents.omega.lnmarkets-hedger-config.v1";
pub const CYCLE_SCHEMA: &str = "openagents.omega.lnmarkets-hedger-cycle.v1";
pub const EVALUATION_SCHEMA: &str = "openagents.omega.lnmarkets-hedger-evaluation.v1";
const STRATEGY_ID: &str = "provider_inventory_hedger";
const VENUE: &str = "lnmarkets_hedger";
const SATOSHIS_PER_BITCOIN: f64 = 100_000_000.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HedgeInstrument {
    CrossMargin,
    SyntheticUsd,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HedgerConfig {
    pub schema: String,
    pub network: Network,
    pub instrument: HedgeInstrument,
    pub target_inventory_sats: u64,
    pub hedge_ratio_bps: u16,
    pub minimum_adjustment_sats: u64,
    pub maximum_adjustment_sats: u64,
    pub cross_leverage: u8,
    pub minimum_liquidation_distance_bps: u32,
    pub margin_top_up_sats: u64,
    pub poll_interval_seconds: u64,
}

impl HedgerConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema != CONFIG_SCHEMA {
            bail!("the hedger configuration schema is not supported");
        }
        if self.network != Network::Signet {
            bail!("the incubating provider hedger is restricted to Signet");
        }
        if self.target_inventory_sats == 0 {
            bail!("the provider inventory target must be greater than zero");
        }
        if !(1..=10_000).contains(&self.hedge_ratio_bps) {
            bail!("the hedge ratio must be between 1 and 10000 basis points");
        }
        if self.minimum_adjustment_sats < 1_000 {
            bail!("the minimum hedge adjustment must be at least 1000 sats");
        }
        if self.maximum_adjustment_sats < self.minimum_adjustment_sats {
            bail!("the maximum hedge adjustment must not be below the minimum");
        }
        FuturesLeverage::new(self.cross_leverage)
            .context("the cross-margin leverage is not valid")?;
        if self.minimum_liquidation_distance_bps == 0 {
            bail!("the liquidation distance limit must be greater than zero");
        }
        if self.margin_top_up_sats == 0 {
            bail!("the margin top-up must be greater than zero");
        }
        if !(5..=3_600).contains(&self.poll_interval_seconds) {
            bail!("the polling interval must be between 5 and 3600 seconds");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VenueSnapshot {
    pub observed_at_ms: i64,
    pub wallet_balance_sats: i64,
    pub synthetic_usd_cents: i64,
    pub index_price_usd: f64,
    pub cross_quantity_usd: i64,
    pub cross_margin_sats: i64,
    pub cross_leverage: u8,
    pub liquidation_price_usd: Option<f64>,
    pub events: Vec<VenueLedgerEvent>,
}

impl VenueSnapshot {
    fn venue_equity_sats(&self) -> Result<i64> {
        if self.wallet_balance_sats < 0
            || self.synthetic_usd_cents < 0
            || self.cross_margin_sats < 0
            || !self.index_price_usd.is_finite()
            || self.index_price_usd <= 0.0
        {
            bail!("LN Markets returned an invalid account snapshot");
        }
        let synthetic_sats = (self.synthetic_usd_cents as f64 / 100.0 / self.index_price_usd
            * SATOSHIS_PER_BITCOIN)
            .round();
        let synthetic_sats = finite_i64(synthetic_sats, "synthetic USD value")?;
        self.wallet_balance_sats
            .checked_add(self.cross_margin_sats)
            .and_then(|value| value.checked_add(synthetic_sats))
            .context("venue equity overflowed")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VenueLedgerEventKind {
    Fill,
    Fee { amount_sats: i64 },
    Funding { amount_sats: i64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct VenueLedgerEvent {
    pub event_id: String,
    pub occurred_at_ms: i64,
    pub kind: VenueLedgerEventKind,
    pub metadata: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HedgeAction {
    SetCrossLeverage {
        leverage: u8,
    },
    TopUpCrossMargin {
        amount_sats: u64,
    },
    ChangeCrossPosition {
        side: FuturesTradeSide,
        quantity_usd: u64,
        client_id: String,
    },
    SwapBitcoinToSyntheticUsd {
        amount_sats: u64,
    },
    SwapSyntheticUsdToBitcoin {
        amount_cents: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VenueExecutionRecord {
    pub venue_id: String,
    pub metadata: Value,
}

#[async_trait]
pub trait HedgeVenue: Send + Sync {
    async fn snapshot(
        &self,
        instrument: HedgeInstrument,
        history_from_ms: i64,
    ) -> Result<VenueSnapshot>;

    async fn execute_once(&self, action: &HedgeAction) -> Result<VenueExecutionRecord>;
}

pub struct LnMarketsHedgeVenue {
    client: Arc<LnMarketsClient>,
}

impl LnMarketsHedgeVenue {
    pub fn new(client: LnMarketsClient) -> Result<Self> {
        if client.network() != Network::Signet {
            bail!("the incubating provider hedger is restricted to Signet");
        }
        Ok(Self {
            client: Arc::new(client),
        })
    }
}

#[async_trait]
impl HedgeVenue for LnMarketsHedgeVenue {
    async fn snapshot(
        &self,
        instrument: HedgeInstrument,
        history_from_ms: i64,
    ) -> Result<VenueSnapshot> {
        let account = self.client.account().await?;
        let ticker = self.client.ticker().await?;
        let index_price_usd = decimal_f64(&ticker.index, "index price")?;
        let history_from = DateTime::<Utc>::from_timestamp_millis(history_from_ms)
            .context("the ledger history cursor is outside the timestamp range")?
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let pagination = Pagination::default()
            .with_time_range(history_from, None)
            .with_limit(1_000)?;

        let mut events = Vec::new();
        let mut cross_quantity_usd = 0;
        let mut cross_margin_sats = 0;
        let mut cross_leverage = 1;
        let mut liquidation_price_usd = None;
        match instrument {
            HedgeInstrument::CrossMargin => {
                let position = self.client.cross_position().await?;
                cross_quantity_usd = decimal_i64(&position.quantity, "cross quantity")?;
                cross_margin_sats = decimal_i64(&position.margin, "cross margin")?;
                cross_leverage = decimal_u8(&position.leverage, "cross leverage")?;
                liquidation_price_usd = position
                    .liquidation
                    .as_ref()
                    .map(|value| decimal_f64(value, "liquidation price"))
                    .transpose()?;
                let orders = self.client.cross_filled_orders(&pagination).await?;
                for order in orders.data {
                    let occurred_at_ms =
                        parse_venue_time(order.filled_at.as_deref().unwrap_or(&order.created_at))?;
                    let metadata = serde_json::to_value(&order)?;
                    events.push(VenueLedgerEvent {
                        event_id: format!("lnmarkets-hedger:cross-fill:{}", order.id),
                        occurred_at_ms,
                        kind: VenueLedgerEventKind::Fill,
                        metadata: metadata.clone(),
                    });
                    let fee_sats = decimal_i64(&order.trading_fee, "cross trading fee")?
                        .checked_abs()
                        .context("cross trading fee overflowed")?;
                    if fee_sats != 0 {
                        events.push(VenueLedgerEvent {
                            event_id: format!("lnmarkets-hedger:cross-fee:{}", order.id),
                            occurred_at_ms,
                            kind: VenueLedgerEventKind::Fee {
                                amount_sats: fee_sats,
                            },
                            metadata,
                        });
                    }
                }
                let funding = self.client.cross_funding_fees(&pagination).await?;
                for payment in funding.data {
                    events.push(VenueLedgerEvent {
                        event_id: format!(
                            "lnmarkets-hedger:cross-funding:{}",
                            payment.settlement_id
                        ),
                        occurred_at_ms: parse_venue_time(&payment.time)?,
                        kind: VenueLedgerEventKind::Funding {
                            amount_sats: decimal_i64(&payment.fee, "cross funding fee")?,
                        },
                        metadata: serde_json::to_value(payment)?,
                    });
                }
            }
            HedgeInstrument::SyntheticUsd => {
                let swaps = self.client.swaps(&pagination).await?;
                for swap in swaps.data {
                    events.push(VenueLedgerEvent {
                        event_id: format!("lnmarkets-hedger:swap-fill:{}", swap.id),
                        occurred_at_ms: parse_venue_time(&swap.created_at)?,
                        kind: VenueLedgerEventKind::Fill,
                        metadata: serde_json::to_value(swap)?,
                    });
                }
            }
        }

        Ok(VenueSnapshot {
            observed_at_ms: Utc::now().timestamp_millis(),
            wallet_balance_sats: decimal_i64(&account.balance, "wallet balance")?,
            synthetic_usd_cents: decimal_usd_cents(
                &account.synthetic_usd_balance,
                "synthetic USD balance",
            )?,
            index_price_usd,
            cross_quantity_usd,
            cross_margin_sats,
            cross_leverage,
            liquidation_price_usd,
            events,
        })
    }

    async fn execute_once(&self, action: &HedgeAction) -> Result<VenueExecutionRecord> {
        let metadata = match action {
            HedgeAction::SetCrossLeverage { leverage } => serde_json::to_value(
                self.client
                    .cross_set_leverage(Network::Signet, FuturesLeverage::new(*leverage)?)
                    .await?,
            )?,
            HedgeAction::TopUpCrossMargin { amount_sats } => serde_json::to_value(
                self.client
                    .cross_deposit(
                        Network::Signet,
                        &FuturesCrossTransferRequest::new(*amount_sats)?,
                    )
                    .await?,
            )?,
            HedgeAction::ChangeCrossPosition {
                side,
                quantity_usd,
                client_id,
            } => serde_json::to_value(
                self.client
                    .cross_new_order(
                        Network::Signet,
                        &FuturesCrossNewOrderRequest::market(
                            *side,
                            FuturesCrossOrderQuantity::new(*quantity_usd)?,
                            client_id,
                        ),
                    )
                    .await?,
            )?,
            HedgeAction::SwapBitcoinToSyntheticUsd { amount_sats } => serde_json::to_value(
                self.client
                    .new_swap(&NewSwapRequest::bitcoin_to_synthetic_usd(*amount_sats)?)
                    .await?,
            )?,
            HedgeAction::SwapSyntheticUsdToBitcoin { amount_cents } => serde_json::to_value(
                self.client
                    .new_swap(&NewSwapRequest::synthetic_usd_to_bitcoin(*amount_cents)?)
                    .await?,
            )?,
        };
        let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&metadata)?));
        Ok(VenueExecutionRecord {
            venue_id: digest,
            metadata,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CycleReport {
    pub schema: &'static str,
    pub observed_at_ms: i64,
    pub instrument: HedgeInstrument,
    pub target_inventory_sats: u64,
    pub target_hedge_usd_cents: u64,
    pub current_hedge_usd_cents: i64,
    pub liquidation_distance_bps: Option<u32>,
    pub action: Option<HedgeAction>,
    pub execution: Option<VenueExecutionRecord>,
    pub ledger: ProfitReport,
}

pub struct Hedger<V> {
    config: HedgerConfig,
    venue: V,
    ledger: LedgerStore,
}

impl<V: HedgeVenue> Hedger<V> {
    pub fn new(config: HedgerConfig, venue: V, ledger: LedgerStore) -> Result<Self> {
        config.validate()?;
        ledger.verify()?;
        Ok(Self {
            config,
            venue,
            ledger,
        })
    }

    pub fn config(&self) -> &HedgerConfig {
        &self.config
    }

    pub async fn run_cycle(&self, now_ms: i64) -> Result<CycleReport> {
        if now_ms < 0 {
            bail!("the hedger cycle timestamp must not be negative");
        }
        let existing = self.ledger.entries(&LedgerQuery {
            strategy_id: Some(STRATEGY_ID.to_owned()),
            ..LedgerQuery::default()
        })?;
        let history_from_ms = existing
            .last()
            .map(|entry| entry.occurred_at_ms.saturating_sub(1))
            .unwrap_or(now_ms);
        let snapshot = self
            .venue
            .snapshot(self.config.instrument, history_from_ms)
            .await?;
        let observed_equity_sats = snapshot.venue_equity_sats()?;
        if existing.is_empty() {
            self.append_baseline(now_ms, observed_equity_sats)?;
        }
        self.append_venue_events(snapshot.events.clone())?;
        self.append_balance_residual(now_ms, observed_equity_sats)?;

        let target_hedge_usd_cents = self.target_hedge_usd_cents(&snapshot)?;
        let liquidation_distance_bps = liquidation_distance_bps(&snapshot)?;
        let action = self.select_action(
            now_ms,
            &snapshot,
            target_hedge_usd_cents,
            liquidation_distance_bps,
        )?;
        let execution = match &action {
            Some(action) => {
                let execution = self.venue.execute_once(action).await?;
                self.append_execution(now_ms, action, &execution)?;
                Some(execution)
            }
            None => None,
        };
        let current_hedge_usd_cents = match self.config.instrument {
            HedgeInstrument::CrossMargin => snapshot
                .cross_quantity_usd
                .checked_mul(100)
                .context("cross hedge value overflowed")?,
            HedgeInstrument::SyntheticUsd => snapshot.synthetic_usd_cents,
        };
        Ok(CycleReport {
            schema: CYCLE_SCHEMA,
            observed_at_ms: snapshot.observed_at_ms,
            instrument: self.config.instrument,
            target_inventory_sats: self.config.target_inventory_sats,
            target_hedge_usd_cents,
            current_hedge_usd_cents,
            liquidation_distance_bps,
            action,
            execution,
            ledger: self.ledger.profit_report(&LedgerQuery {
                strategy_id: Some(STRATEGY_ID.to_owned()),
                ..LedgerQuery::default()
            })?,
        })
    }

    fn target_hedge_usd_cents(&self, snapshot: &VenueSnapshot) -> Result<u64> {
        let target = self.config.target_inventory_sats as f64 / SATOSHIS_PER_BITCOIN
            * snapshot.index_price_usd
            * 100.0
            * f64::from(self.config.hedge_ratio_bps)
            / 10_000.0;
        finite_u64(target.round(), "target hedge value")
    }

    fn select_action(
        &self,
        now_ms: i64,
        snapshot: &VenueSnapshot,
        target_hedge_usd_cents: u64,
        liquidation_distance_bps: Option<u32>,
    ) -> Result<Option<HedgeAction>> {
        match self.config.instrument {
            HedgeInstrument::CrossMargin => {
                if snapshot.cross_quantity_usd != 0
                    && liquidation_distance_bps.is_some_and(|distance| {
                        distance < self.config.minimum_liquidation_distance_bps
                    })
                {
                    if snapshot.wallet_balance_sats
                        < i64::try_from(self.config.margin_top_up_sats)
                            .context("margin top-up exceeds the supported range")?
                    {
                        bail!("the wallet balance cannot fund the required margin top-up");
                    }
                    return Ok(Some(HedgeAction::TopUpCrossMargin {
                        amount_sats: self.config.margin_top_up_sats,
                    }));
                }
                if snapshot.cross_leverage != self.config.cross_leverage {
                    return Ok(Some(HedgeAction::SetCrossLeverage {
                        leverage: self.config.cross_leverage,
                    }));
                }
                let desired_quantity_usd = i64::try_from(target_hedge_usd_cents.div_ceil(100))
                    .context("target cross quantity exceeds the supported range")?
                    .checked_neg()
                    .context("target cross quantity overflowed")?;
                let difference = desired_quantity_usd
                    .checked_sub(snapshot.cross_quantity_usd)
                    .context("cross hedge difference overflowed")?;
                let minimum_usd = sats_to_usd(
                    self.config.minimum_adjustment_sats,
                    snapshot.index_price_usd,
                )?
                .ceil() as u64;
                let maximum_usd = sats_to_usd(
                    self.config.maximum_adjustment_sats,
                    snapshot.index_price_usd,
                )?
                .floor()
                .max(1.0) as u64;
                let magnitude = difference.unsigned_abs();
                if magnitude < minimum_usd.max(1) {
                    return Ok(None);
                }
                Ok(Some(HedgeAction::ChangeCrossPosition {
                    side: if difference < 0 {
                        FuturesTradeSide::Sell
                    } else {
                        FuturesTradeSide::Buy
                    },
                    quantity_usd: magnitude.min(maximum_usd),
                    client_id: format!("omega-provider-hedger-{now_ms}"),
                }))
            }
            HedgeInstrument::SyntheticUsd => {
                let target_cents = i64::try_from(target_hedge_usd_cents)
                    .context("target synthetic USD value exceeds the supported range")?;
                let difference_cents = target_cents
                    .checked_sub(snapshot.synthetic_usd_cents)
                    .context("synthetic USD hedge difference overflowed")?;
                let minimum_cents = (sats_to_usd(
                    self.config.minimum_adjustment_sats,
                    snapshot.index_price_usd,
                )? * 100.0)
                    .ceil() as i64;
                if difference_cents.unsigned_abs()
                    < u64::try_from(minimum_cents.max(1))
                        .context("minimum synthetic adjustment is invalid")?
                {
                    return Ok(None);
                }
                if difference_cents > 0 {
                    let required_sats =
                        (difference_cents as f64 / 100.0 / snapshot.index_price_usd
                            * SATOSHIS_PER_BITCOIN)
                            .ceil();
                    let amount_sats = finite_u64(required_sats, "synthetic USD swap input")?
                        .min(self.config.maximum_adjustment_sats);
                    if amount_sats > u64::try_from(snapshot.wallet_balance_sats.max(0))? {
                        bail!("the wallet balance cannot fund the synthetic USD hedge");
                    }
                    Ok(Some(HedgeAction::SwapBitcoinToSyntheticUsd { amount_sats }))
                } else {
                    let maximum_cents = (sats_to_usd(
                        self.config.maximum_adjustment_sats,
                        snapshot.index_price_usd,
                    )? * 100.0)
                        .floor();
                    Ok(Some(HedgeAction::SwapSyntheticUsdToBitcoin {
                        amount_cents: difference_cents
                            .unsigned_abs()
                            .min(finite_u64(maximum_cents, "maximum synthetic adjustment")?),
                    }))
                }
            }
        }
    }

    fn append_baseline(&self, now_ms: i64, balance_sats: i64) -> Result<()> {
        if balance_sats <= 0 {
            bail!("the initial venue equity must be greater than zero");
        }
        self.ledger.append(LedgerEntryDraft {
            event_id: "lnmarkets-hedger:baseline".to_owned(),
            occurred_at_ms: now_ms,
            strategy_id: STRATEGY_ID.to_owned(),
            kind: LedgerEntryKind::BalanceAdjustment,
            postings: vec![
                venue_posting(balance_sats),
                LedgerPosting {
                    account: LedgerAccount::External,
                    amount_sats: balance_sats
                        .checked_neg()
                        .context("baseline posting overflowed")?,
                },
            ],
            metadata: json!({"schema": "openagents.omega.lnmarkets-hedger-baseline.v1"}),
        })?;
        Ok(())
    }

    fn append_venue_events(&self, mut events: Vec<VenueLedgerEvent>) -> Result<()> {
        events.sort_by(|first, second| {
            (first.occurred_at_ms, &first.event_id).cmp(&(second.occurred_at_ms, &second.event_id))
        });
        for event in events {
            let (kind, postings) = match event.kind {
                VenueLedgerEventKind::Fill => (LedgerEntryKind::Fill, Vec::new()),
                VenueLedgerEventKind::Fee { amount_sats } => {
                    if amount_sats <= 0 {
                        bail!("a venue fee must be greater than zero");
                    }
                    (
                        LedgerEntryKind::Fee,
                        vec![
                            venue_posting(
                                amount_sats
                                    .checked_neg()
                                    .context("fee posting overflowed")?,
                            ),
                            LedgerPosting {
                                account: LedgerAccount::FeeExpense,
                                amount_sats,
                            },
                        ],
                    )
                }
                VenueLedgerEventKind::Funding { amount_sats } => {
                    if amount_sats == 0 {
                        continue;
                    }
                    (
                        LedgerEntryKind::FundingSettlement,
                        vec![
                            venue_posting(amount_sats),
                            LedgerPosting {
                                account: LedgerAccount::FundingIncome,
                                amount_sats: amount_sats
                                    .checked_neg()
                                    .context("funding posting overflowed")?,
                            },
                        ],
                    )
                }
            };
            self.ledger.append(LedgerEntryDraft {
                event_id: event.event_id,
                occurred_at_ms: event.occurred_at_ms,
                strategy_id: STRATEGY_ID.to_owned(),
                kind,
                postings,
                metadata: event.metadata,
            })?;
        }
        Ok(())
    }

    fn append_balance_residual(&self, now_ms: i64, observed_sats: i64) -> Result<()> {
        let expected_sats = self.ledger.venue_balance(VENUE)?;
        let difference_sats = observed_sats
            .checked_sub(expected_sats)
            .context("venue balance difference overflowed")?;
        if difference_sats == 0 {
            return Ok(());
        }
        let event_payload = format!("{now_ms}:{expected_sats}:{observed_sats}");
        self.ledger.append(LedgerEntryDraft {
            event_id: format!(
                "lnmarkets-hedger:balance:{}",
                hex_digest(event_payload.as_bytes())
            ),
            occurred_at_ms: now_ms,
            strategy_id: STRATEGY_ID.to_owned(),
            kind: LedgerEntryKind::BalanceAdjustment,
            postings: vec![
                venue_posting(difference_sats),
                LedgerPosting {
                    account: LedgerAccount::TradingProfit,
                    amount_sats: difference_sats
                        .checked_neg()
                        .context("profit posting overflowed")?,
                },
            ],
            metadata: json!({
                "schema": "openagents.omega.lnmarkets-hedger-balance.v1",
                "expected_sats": expected_sats,
                "observed_sats": observed_sats,
            }),
        })?;
        Ok(())
    }

    fn append_execution(
        &self,
        now_ms: i64,
        action: &HedgeAction,
        execution: &VenueExecutionRecord,
    ) -> Result<()> {
        self.ledger.append(LedgerEntryDraft {
            event_id: format!("lnmarkets-hedger:order:{}", execution.venue_id),
            occurred_at_ms: now_ms,
            strategy_id: STRATEGY_ID.to_owned(),
            kind: LedgerEntryKind::Order,
            postings: Vec::new(),
            metadata: json!({
                "schema": "openagents.omega.lnmarkets-hedger-order.v1",
                "action": action,
                "venue_result": execution.metadata,
            }),
        })?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HedgeEvaluation {
    pub schema: &'static str,
    pub sample_count: usize,
    pub hedged_profit_variance: f64,
    pub unhedged_profit_variance: f64,
    pub funding_sats: i64,
    pub fees_sats: i64,
    pub net_carry_sats: i64,
    pub lower_variance: bool,
    pub nonnegative_net_carry: bool,
    pub passes: bool,
}

pub fn evaluate_window(
    hedged_profit_samples_sats: &[i64],
    unhedged_profit_samples_sats: &[i64],
    funding_sats: i64,
    fees_sats: i64,
) -> Result<HedgeEvaluation> {
    if hedged_profit_samples_sats.len() != unhedged_profit_samples_sats.len()
        || hedged_profit_samples_sats.len() < 2
    {
        bail!("hedge evaluation needs two equally sized sample series");
    }
    if fees_sats < 0 {
        bail!("hedge evaluation fees must not be negative");
    }
    let hedged_profit_variance = variance(hedged_profit_samples_sats)?;
    let unhedged_profit_variance = variance(unhedged_profit_samples_sats)?;
    let net_carry_sats = funding_sats
        .checked_sub(fees_sats)
        .context("net carry overflowed")?;
    let lower_variance = hedged_profit_variance < unhedged_profit_variance;
    let nonnegative_net_carry = net_carry_sats >= 0;
    Ok(HedgeEvaluation {
        schema: EVALUATION_SCHEMA,
        sample_count: hedged_profit_samples_sats.len(),
        hedged_profit_variance,
        unhedged_profit_variance,
        funding_sats,
        fees_sats,
        net_carry_sats,
        lower_variance,
        nonnegative_net_carry,
        passes: lower_variance && nonnegative_net_carry,
    })
}

fn variance(samples: &[i64]) -> Result<f64> {
    let count = samples.len() as f64;
    let sum = samples.iter().try_fold(0_i128, |sum, sample| {
        sum.checked_add(i128::from(*sample))
            .context("hedge evaluation sum overflowed")
    })?;
    let mean = sum as f64 / count;
    let variance = samples
        .iter()
        .map(|sample| (*sample as f64 - mean).powi(2))
        .sum::<f64>()
        / count;
    if !variance.is_finite() {
        bail!("hedge evaluation variance is not finite");
    }
    Ok(variance)
}

fn liquidation_distance_bps(snapshot: &VenueSnapshot) -> Result<Option<u32>> {
    if snapshot.cross_quantity_usd == 0 {
        return Ok(None);
    }
    let Some(liquidation_price_usd) = snapshot.liquidation_price_usd else {
        bail!("the active cross position has no liquidation price");
    };
    if !liquidation_price_usd.is_finite() || liquidation_price_usd <= 0.0 {
        bail!("the active cross position has an invalid liquidation price");
    }
    let distance = ((liquidation_price_usd - snapshot.index_price_usd).abs()
        / snapshot.index_price_usd
        * 10_000.0)
        .floor();
    let distance = finite_u64(distance, "liquidation distance")?;
    let distance = u32::try_from(distance.min(u64::from(u32::MAX)))
        .context("liquidation distance conversion failed")?;
    Ok(Some(distance))
}

fn venue_posting(amount_sats: i64) -> LedgerPosting {
    LedgerPosting {
        account: LedgerAccount::VenueBalance {
            venue: VENUE.to_owned(),
        },
        amount_sats,
    }
}

fn decimal_f64(amount: &lnmarkets_client::DecimalAmount, label: &str) -> Result<f64> {
    let value = amount
        .as_number()
        .as_f64()
        .with_context(|| format!("LN Markets returned an invalid {label}"))?;
    if !value.is_finite() {
        bail!("LN Markets returned an invalid {label}");
    }
    Ok(value)
}

fn decimal_i64(amount: &lnmarkets_client::DecimalAmount, label: &str) -> Result<i64> {
    let value = decimal_f64(amount, label)?;
    if value.fract() != 0.0 {
        bail!("LN Markets returned a fractional {label}");
    }
    finite_i64(value, label)
}

fn decimal_u8(amount: &lnmarkets_client::DecimalAmount, label: &str) -> Result<u8> {
    u8::try_from(decimal_i64(amount, label)?)
        .with_context(|| format!("LN Markets returned an invalid {label}"))
}

fn decimal_usd_cents(amount: &lnmarkets_client::DecimalAmount, label: &str) -> Result<i64> {
    finite_i64((decimal_f64(amount, label)? * 100.0).round(), label)
}

fn parse_venue_time(value: &str) -> Result<i64> {
    Ok(DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("LN Markets returned an invalid timestamp {value:?}"))?
        .timestamp_millis())
}

fn sats_to_usd(sats: u64, index_price_usd: f64) -> Result<f64> {
    let value = sats as f64 / SATOSHIS_PER_BITCOIN * index_price_usd;
    if !value.is_finite() || value < 0.0 {
        bail!("the hedge value is outside the supported range");
    }
    Ok(value)
}

fn finite_i64(value: f64, label: &str) -> Result<i64> {
    if !value.is_finite() || value < i64::MIN as f64 || value > i64::MAX as f64 {
        bail!("{label} is outside the supported range");
    }
    Ok(value as i64)
}

fn finite_u64(value: f64, label: &str) -> Result<u64> {
    if !value.is_finite() || value < 0.0 || value > u64::MAX as f64 {
        bail!("{label} is outside the supported range");
    }
    Ok(value as u64)
}

fn hex_digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

pub fn redactable_summary(report: &CycleReport) -> BTreeMap<&'static str, Value> {
    BTreeMap::from([
        ("schema", json!(report.schema)),
        ("observed_at_ms", json!(report.observed_at_ms)),
        ("instrument", json!(report.instrument)),
        ("action", json!(report.action)),
        ("ledger", json!(report.ledger)),
    ])
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct FakeVenue {
        snapshots: Mutex<Vec<VenueSnapshot>>,
        actions: Mutex<Vec<HedgeAction>>,
    }

    impl FakeVenue {
        fn new(snapshots: Vec<VenueSnapshot>) -> Self {
            Self {
                snapshots: Mutex::new(snapshots.into_iter().rev().collect()),
                actions: Mutex::new(Vec::new()),
            }
        }

        fn actions(&self) -> Vec<HedgeAction> {
            self.actions.lock().expect("actions lock").clone()
        }
    }

    #[async_trait]
    impl HedgeVenue for Arc<FakeVenue> {
        async fn snapshot(
            &self,
            _instrument: HedgeInstrument,
            _history_from_ms: i64,
        ) -> Result<VenueSnapshot> {
            self.snapshots
                .lock()
                .expect("snapshot lock")
                .pop()
                .context("the fake venue has no snapshot")
        }

        async fn execute_once(&self, action: &HedgeAction) -> Result<VenueExecutionRecord> {
            self.actions
                .lock()
                .expect("actions lock")
                .push(action.clone());
            Ok(VenueExecutionRecord {
                venue_id: format!("execution-{}", self.actions().len()),
                metadata: json!({"status": "accepted"}),
            })
        }
    }

    fn config(instrument: HedgeInstrument) -> HedgerConfig {
        HedgerConfig {
            schema: CONFIG_SCHEMA.to_owned(),
            network: Network::Signet,
            instrument,
            target_inventory_sats: 100_000,
            hedge_ratio_bps: 10_000,
            minimum_adjustment_sats: 1_000,
            maximum_adjustment_sats: 50_000,
            cross_leverage: 2,
            minimum_liquidation_distance_bps: 1_000,
            margin_top_up_sats: 5_000,
            poll_interval_seconds: 60,
        }
    }

    fn snapshot() -> VenueSnapshot {
        VenueSnapshot {
            observed_at_ms: 1_000,
            wallet_balance_sats: 100_000,
            synthetic_usd_cents: 0,
            index_price_usd: 50_000.0,
            cross_quantity_usd: 0,
            cross_margin_sats: 0,
            cross_leverage: 2,
            liquidation_price_usd: None,
            events: Vec::new(),
        }
    }

    #[test]
    fn configuration_rejects_mainnet_and_unbounded_values() {
        let mut config = config(HedgeInstrument::CrossMargin);
        config.network = Network::Mainnet;
        assert!(config.validate().is_err());
        config.network = Network::Signet;
        config.hedge_ratio_bps = 10_001;
        assert!(config.validate().is_err());
        config.hedge_ratio_bps = 10_000;
        config.maximum_adjustment_sats = 999;
        assert!(config.validate().is_err());
    }

    #[test]
    fn cross_margin_cycle_opens_the_bounded_short_once() {
        smol::block_on(async {
            let venue = Arc::new(FakeVenue::new(vec![snapshot()]));
            let hedger = Hedger::new(
                config(HedgeInstrument::CrossMargin),
                venue.clone(),
                LedgerStore::in_memory().expect("ledger"),
            )
            .expect("hedger");
            let report = hedger.run_cycle(1_000).await.expect("cycle");
            assert_eq!(report.target_hedge_usd_cents, 5_000);
            assert_eq!(
                venue.actions(),
                vec![HedgeAction::ChangeCrossPosition {
                    side: FuturesTradeSide::Sell,
                    quantity_usd: 25,
                    client_id: "omega-provider-hedger-1000".to_owned(),
                }]
            );
            assert_eq!(report.ledger.total_profit_sats, 0);
        });
    }

    #[test]
    fn liquidation_protection_precedes_position_changes() {
        smol::block_on(async {
            let mut unsafe_snapshot = snapshot();
            unsafe_snapshot.cross_quantity_usd = -50;
            unsafe_snapshot.cross_margin_sats = 10_000;
            unsafe_snapshot.liquidation_price_usd = Some(54_000.0);
            let venue = Arc::new(FakeVenue::new(vec![unsafe_snapshot]));
            let hedger = Hedger::new(
                config(HedgeInstrument::CrossMargin),
                venue.clone(),
                LedgerStore::in_memory().expect("ledger"),
            )
            .expect("hedger");
            let report = hedger.run_cycle(1_000).await.expect("cycle");
            assert_eq!(report.liquidation_distance_bps, Some(800));
            assert_eq!(
                venue.actions(),
                vec![HedgeAction::TopUpCrossMargin { amount_sats: 5_000 }]
            );
        });
    }

    #[test]
    fn synthetic_cycle_uses_the_inventory_target() {
        smol::block_on(async {
            let venue = Arc::new(FakeVenue::new(vec![snapshot()]));
            let hedger = Hedger::new(
                config(HedgeInstrument::SyntheticUsd),
                venue.clone(),
                LedgerStore::in_memory().expect("ledger"),
            )
            .expect("hedger");
            hedger.run_cycle(1_000).await.expect("cycle");
            assert_eq!(
                venue.actions(),
                vec![HedgeAction::SwapBitcoinToSyntheticUsd {
                    amount_sats: 50_000,
                }]
            );
        });
    }

    #[test]
    fn venue_events_are_idempotent_and_profit_is_attributed() {
        smol::block_on(async {
            let mut first = snapshot();
            first.cross_quantity_usd = -1;
            first.liquidation_price_usd = Some(100_000.0);
            let mut second = snapshot();
            second.cross_quantity_usd = -1;
            second.liquidation_price_usd = Some(100_000.0);
            second.observed_at_ms = 2_000;
            second.wallet_balance_sats = 100_013;
            second.events = vec![
                VenueLedgerEvent {
                    event_id: "fee-1".to_owned(),
                    occurred_at_ms: 1_500,
                    kind: VenueLedgerEventKind::Fee { amount_sats: 2 },
                    metadata: json!({"source": "test"}),
                },
                VenueLedgerEvent {
                    event_id: "funding-1".to_owned(),
                    occurred_at_ms: 1_600,
                    kind: VenueLedgerEventKind::Funding { amount_sats: 5 },
                    metadata: json!({"source": "test"}),
                },
            ];
            let third = second.clone();
            let venue = Arc::new(FakeVenue::new(vec![first, second, third]));
            let mut config = config(HedgeInstrument::CrossMargin);
            config.target_inventory_sats = 1_000;
            let hedger = Hedger::new(config, venue, LedgerStore::in_memory().expect("ledger"))
                .expect("hedger");
            hedger.run_cycle(1_000).await.expect("first cycle");
            let report = hedger.run_cycle(2_000).await.expect("second cycle");
            assert_eq!(report.ledger.total_fees_paid_sats, 2);
            assert_eq!(report.ledger.total_funding_collected_sats, 5);
            assert_eq!(report.ledger.total_profit_sats, 13);
            let repeated = hedger.run_cycle(2_000).await.expect("repeated cycle");
            assert_eq!(repeated.ledger.total_profit_sats, 13);
            assert_eq!(repeated.ledger.strategies[0].entry_count, 4);
        });
    }

    #[test]
    fn evaluation_requires_lower_variance_and_nonnegative_carry() {
        let passing = evaluate_window(&[0, 1, -1, 0], &[0, 10, -10, 0], 5, 5).expect("evaluation");
        assert!(passing.lower_variance);
        assert!(passing.nonnegative_net_carry);
        assert!(passing.passes);

        let losing = evaluate_window(&[0, 1], &[0, 10], 1, 2).expect("evaluation");
        assert!(!losing.passes);
    }
}
