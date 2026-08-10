use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use agent::{AgentTool, ToolCallEventStream, ToolInput, ToolPermissionContext};
use agent_client_protocol::schema::v1 as acp;
use anyhow::{Context as _, Result, bail};
use gpui::{App, Task};
use language_model::LanguageModelToolResultContent;
use nautilus_sidecar::{
    CommandReceipt, CommandRequest, NautilusCommand, NautilusCommandChannel, NautilusStreamSource,
    OrderSide, OrderTimeInForce, StrategyParameters, StreamEvent, credential_state,
};
use parking_lot::Mutex;
use plugin_api::{
    CardSchemaRegistration, Maturity, ObservedVenueMode, PluginManifest, PluginRegistry,
    ProbedVenueAssumption, ReviewCadence as PluginReviewCadence, ReviewTurnEvidence,
    ReviewTurnOutcome, SessionReviewDriver, VenueAccountMode, VenueActionCapability,
    VenueActionClass, VenueActionStatus, VenueCapabilities, VenueCapabilityStore, VenueMarginMode,
};
use prediction_events::{
    MandateScope, PREDICTION_SCHEMA_VERSION, PredictedDirection, PredictionActor,
    PredictionEventDraft, PredictionForecast, PredictionStore, ResolutionRule, ScoringRule,
};
use review_accounting::{
    REVIEW_ACCOUNTING_SCHEMA_VERSION, ReviewAccountingStore, ReviewCostRecord, ReviewDisposition,
    ReviewInterventionKind,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use trading_ledger::{
    AssetId, LedgerAccount, LedgerEntryDraft, LedgerEntryKind, LedgerPosting, LedgerQuery,
    LedgerStore, ReconciliationOutcome,
};
use trading_mandate::{
    MandateDecision, MandateStore, ReviewCadence as MandateReviewCadence, TradingInstruction,
    TradingNetwork,
};
use ui::SharedString;

mod card_renderers;

const VENUE: &str = "hyperliquid";
const STRATEGY_ID: &str = "OMEGA-BOUNDED-QUOTE-001";
const CAPABILITY_MAX_AGE_MS: i64 = 30_000;
const USDC_SCALE: u32 = 6;
const BTC_SCALE: u32 = 8;
const GOVERNANCE_STATE_CAPACITY: usize = 2_048;
pub const COST_FLOOR_SCHEMA: &str = "omega.nautilus.cost_floor.v1";
pub const BASIS_POINT_MICROS: i64 = 1_000_000;

pub const MANIFEST: PluginManifest = PluginManifest {
    id: "nautilus_governance",
    name: "Nautilus Governance",
    version: env!("CARGO_PKG_VERSION"),
    maturity: Maturity::Testnet,
    // Connectivity is owned exclusively by the Nautilus sidecar.
    hosts: &[],
};

#[derive(Clone, Debug, Default, Serialize)]
struct GovernanceState {
    account: Option<Value>,
    orders: Vec<Value>,
    positions: Vec<Value>,
    strategy: Option<Value>,
    latest_review: Option<Value>,
    halted_reason: Option<String>,
    #[serde(skip)]
    processed_event_keys: HashSet<String>,
    #[serde(skip)]
    processed_event_order: VecDeque<String>,
    #[serde(skip)]
    processed_venue_fill_ids: HashSet<String>,
    #[serde(skip)]
    claimed_sessions: HashSet<String>,
    pending_wakeup: Option<String>,
    pending_credential_wakeup: Option<plugin_api::WakeupSource>,
}

#[derive(Clone)]
pub struct GovernanceRuntime {
    mandate: MandateStore,
    ledger: LedgerStore,
    predictions: PredictionStore,
    review_accounting: ReviewAccountingStore,
    capabilities: VenueCapabilityStore,
    state: Arc<Mutex<GovernanceState>>,
    command_counter: Arc<AtomicU64>,
}

impl GovernanceRuntime {
    pub fn open_default(capabilities: VenueCapabilityStore) -> Result<Self> {
        Ok(Self {
            mandate: MandateStore::open_default()?,
            ledger: LedgerStore::open_default()?,
            predictions: PredictionStore::open_default()?,
            review_accounting: ReviewAccountingStore::open_default()?,
            capabilities,
            state: Arc::new(Mutex::new(GovernanceState::default())),
            command_counter: Arc::new(AtomicU64::new(1)),
        })
    }

    #[cfg(test)]
    fn in_memory(capabilities: VenueCapabilityStore) -> Result<Self> {
        Ok(Self {
            mandate: MandateStore::in_memory()?,
            ledger: LedgerStore::in_memory()?,
            predictions: PredictionStore::in_memory()?,
            review_accounting: ReviewAccountingStore::in_memory()?,
            capabilities,
            state: Arc::new(Mutex::new(GovernanceState::default())),
            command_counter: Arc::new(AtomicU64::new(1)),
        })
    }

    fn ingest(&self, events: &[StreamEvent]) -> Result<()> {
        let mut account_to_reconcile = None;
        for event in events {
            let key = stream_event_key(event);
            if self.state.lock().processed_event_keys.contains(&key) {
                continue;
            }
            match event {
                StreamEvent::Account {
                    state,
                    generation,
                    sequence,
                    ..
                } => {
                    let value = Value::Object(state.clone());
                    self.observe_account_modes(&value)?;
                    self.state.lock().account = Some(value.clone());
                    account_to_reconcile = Some((*generation, *sequence, value));
                }
                StreamEvent::Order { state, .. } => {
                    let mut projection = self.state.lock();
                    if projection.orders.len() == GOVERNANCE_STATE_CAPACITY {
                        projection.orders.remove(0);
                    }
                    projection.orders.push(Value::Object(state.clone()));
                }
                StreamEvent::OrderState {
                    orders,
                    generation,
                    sequence,
                    ..
                } => {
                    self.state.lock().orders = bounded_latest(orders);
                    for order in orders {
                        let Some((venue_order_id, fill_state)) = reconciled_fill_state(order)
                        else {
                            continue;
                        };
                        if self
                            .state
                            .lock()
                            .processed_venue_fill_ids
                            .insert(venue_order_id)
                        {
                            self.record_fill(*generation, *sequence, &fill_state)?;
                        }
                    }
                }
                StreamEvent::Position { state, .. } => {
                    let mut projection = self.state.lock();
                    if projection.positions.len() == GOVERNANCE_STATE_CAPACITY {
                        projection.positions.remove(0);
                    }
                    projection.positions.push(Value::Object(state.clone()));
                }
                StreamEvent::PositionState { positions, .. } => {
                    self.state.lock().positions = bounded_latest(positions);
                }
                StreamEvent::Fill {
                    state,
                    generation,
                    sequence,
                    ..
                } => {
                    self.record_fill(*generation, *sequence, state)?;
                }
                StreamEvent::StrategyState { halted_reason, .. } => {
                    self.state.lock().strategy = Some(serde_json::to_value(event)?);
                    if let Some(reason) = halted_reason {
                        self.halt(format!("Nautilus strategy halted: {reason}"));
                    }
                }
                StreamEvent::Quote { .. }
                | StreamEvent::Trade { .. }
                | StreamEvent::Book { .. } => {}
            }
            let mut projection = self.state.lock();
            if projection.processed_event_order.len() == GOVERNANCE_STATE_CAPACITY
                && let Some(expired) = projection.processed_event_order.pop_front()
            {
                projection.processed_event_keys.remove(&expired);
            }
            projection.processed_event_order.push_back(key.clone());
            projection.processed_event_keys.insert(key);
        }
        if let Some((generation, sequence, account)) = account_to_reconcile {
            self.reconcile_account(generation, sequence, &account)?;
        }
        Ok(())
    }

    fn observe_account_modes(&self, account: &Value) -> Result<()> {
        self.observe_account_modes_at(account, unix_ms()?)
    }

    fn observe_account_modes_at(&self, account: &Value, observed_at_ms: i64) -> Result<()> {
        let raw_account = find_string(account, &["account_type", "account_mode"])
            .unwrap_or_else(|| "unknown".into());
        let account_mode = if matches!(
            raw_account.to_ascii_uppercase().as_str(),
            "MARGIN" | "UNIFIED"
        ) {
            ObservedVenueMode::known(VenueAccountMode::UnifiedAccount, raw_account)
        } else {
            self.halt(format!("unknown Hyperliquid account mode {raw_account:?}"));
            ObservedVenueMode::unknown(raw_account)
        };
        let raw_margin = find_string(account, &["margin_mode"]).unwrap_or_else(|| "cross".into());
        let margin_mode = if raw_margin.eq_ignore_ascii_case("cross") {
            ObservedVenueMode::known(VenueMarginMode::Cross, raw_margin)
        } else {
            self.halt(format!("unknown Hyperliquid margin mode {raw_margin:?}"));
            ObservedVenueMode::unknown(raw_margin)
        };
        self.capabilities.publish(VenueCapabilities {
            venue_id: VENUE.into(),
            account_mode: ProbedVenueAssumption::new(account_mode, observed_at_ms),
            margin_mode: ProbedVenueAssumption::new(margin_mode, observed_at_ms),
            actions: [
                VenueActionClass::StrategyExecution,
                VenueActionClass::OrderPlacement,
                VenueActionClass::OrderCancellation,
            ]
            .into_iter()
            .map(|action_class| {
                ProbedVenueAssumption::new(
                    VenueActionCapability {
                        action_class,
                        status: VenueActionStatus::Supported,
                    },
                    observed_at_ms,
                )
            })
            .collect(),
        })?;
        Ok(())
    }

    fn record_fill(
        &self,
        generation: u64,
        sequence: u64,
        state: &serde_json::Map<String, Value>,
    ) -> Result<()> {
        let value = Value::Object(state.clone());
        let instrument = find_string(&value, &["instrument_id", "instrument"])
            .context("fill has no instrument ID")?;
        let side = find_string(&value, &["order_side", "side"]).context("fill has no side")?;
        let quantity = find_string(&value, &["last_qty", "quantity", "size"])
            .context("fill has no quantity")?;
        let price = find_string(&value, &["last_px", "price"]).context("fill has no price")?;
        let base = instrument
            .split('-')
            .next()
            .unwrap_or("btc")
            .to_ascii_lowercase();
        let base_asset = AssetId::new(base)?;
        let base_amount = decimal_to_units(&quantity, BTC_SCALE)?;
        let quote_amount = decimal_product_to_units(&quantity, &price, USDC_SCALE)?;
        let buy_sign = if side.eq_ignore_ascii_case("buy") {
            1
        } else if side.eq_ignore_ascii_case("sell") {
            -1
        } else {
            bail!("fill has unknown side {side:?}");
        };
        // A perpetual fill changes the strategy's economic base/quote
        // position, not the venue's spot collateral balances. Fees and
        // funding are the postings that move VenueBalance and are therefore
        // what account reconciliation compares with the engine.
        let participant = LedgerAccount::MarketParticipant {
            role: "requester".into(),
            participant: STRATEGY_ID.into(),
        };
        let fill_identity = find_string(&value, &["trade_id", "venue_order_id", "client_order_id"])
            .unwrap_or_else(|| format!("{generation}-{sequence}"));
        let mut draft = LedgerEntryDraft::new(
            format!("nautilus-fill-{fill_identity}"),
            unix_ms()?,
            STRATEGY_ID,
            LedgerEntryKind::Fill,
        );
        draft.postings = vec![
            LedgerPosting::new(
                participant.clone(),
                buy_sign * base_amount,
                base_asset.clone(),
            ),
            LedgerPosting::new(LedgerAccount::External, -buy_sign * base_amount, base_asset),
            LedgerPosting::new(participant, -buy_sign * quote_amount, AssetId::usdc()),
            LedgerPosting::new(
                LedgerAccount::External,
                buy_sign * quote_amount,
                AssetId::usdc(),
            ),
        ];
        draft.metadata = json!({"schema":"omega.nautilus.ledger.fill.v1","generation":generation,"stream_sequence":sequence,"fill":value});
        self.ledger.append(draft)?;
        if let Some(commission) = find_string(&Value::Object(state.clone()), &["commission", "fee"])
        {
            let mut parts = commission.split_whitespace();
            let amount = parts.next().context("fill commission has no amount")?;
            let asset = parts
                .next()
                .map(str::to_ascii_lowercase)
                .map(AssetId::new)
                .transpose()?
                .unwrap_or_else(AssetId::usdc);
            let scale = if asset == AssetId::usdc() {
                USDC_SCALE
            } else {
                BTC_SCALE
            };
            let fee = decimal_to_units(amount, scale)?.unsigned_abs();
            if fee != 0 {
                let fee = i64::try_from(fee).context("fill commission exceeds ledger range")?;
                let mut fee_draft = LedgerEntryDraft::new(
                    format!("nautilus-fee-{fill_identity}"),
                    unix_ms()?,
                    STRATEGY_ID,
                    LedgerEntryKind::Fee,
                );
                fee_draft.postings = vec![
                    LedgerPosting::new(LedgerAccount::FeeExpense, fee, asset.clone()),
                    LedgerPosting::new(
                        LedgerAccount::VenueBalance {
                            venue: VENUE.into(),
                        },
                        -fee,
                        asset,
                    ),
                ];
                fee_draft.metadata = json!({"schema":"omega.nautilus.ledger.fee.v1","generation":generation,"stream_sequence":sequence});
                self.ledger.append(fee_draft)?;
            }
        }
        Ok(())
    }

    fn halt(&self, reason: String) {
        let mut state = self.state.lock();
        state.pending_wakeup = Some(reason.clone());
        state.halted_reason = Some(reason);
    }

    fn set_credential_wakeup(&self, wakeup: Option<plugin_api::WakeupSource>) {
        self.state.lock().pending_credential_wakeup = wakeup;
    }

    fn reconcile_account(&self, generation: u64, sequence: u64, account: &Value) -> Result<()> {
        let observed = decimal_to_units(
            &find_account_balance(account)
                .context("account state has no recognized USDC balance")?,
            USDC_SCALE,
        )?;
        if observed < 0 {
            bail!("account state reports a negative venue balance");
        }
        let entries = self.ledger.entries(&LedgerQuery::default())?;
        let initialized = entries.iter().any(|entry| {
            entry.kind == LedgerEntryKind::BalanceAdjustment
                && entry.metadata.get("projection")
                    == Some(&Value::String("opening_balance".into()))
        });
        if !initialized {
            let expected = self.ledger.venue_asset_balance(VENUE, &AssetId::usdc())?;
            let difference = observed
                .checked_sub(expected)
                .context("opening balance difference overflowed")?;
            if difference != 0 {
                let mut draft = LedgerEntryDraft::new(
                    format!("nautilus-opening-balance-{generation}-{sequence}"),
                    unix_ms()?,
                    STRATEGY_ID,
                    LedgerEntryKind::BalanceAdjustment,
                );
                draft.postings = vec![
                    LedgerPosting::new(
                        LedgerAccount::VenueBalance {
                            venue: VENUE.into(),
                        },
                        difference,
                        AssetId::usdc(),
                    ),
                    LedgerPosting::new(
                        LedgerAccount::BalanceAdjustment,
                        -difference,
                        AssetId::usdc(),
                    ),
                ];
                draft.metadata = json!({"schema":"omega.nautilus.reconciliation.v1","projection":"opening_balance","generation":generation,"stream_sequence":sequence});
                self.ledger.append(draft)?;
            }
            return Ok(());
        }
        match self.ledger.reconcile_asset(
            format!("nautilus-reconcile-{generation}-{sequence}"),
            unix_ms()?,
            STRATEGY_ID,
            VENUE,
            AssetId::usdc(),
            observed,
        )? {
            ReconciliationOutcome::Matched { .. } => Ok(()),
            ReconciliationOutcome::Mismatch { alert } => {
                self.halt(format!(
                    "ledger reconciliation gap recorded at entry {}",
                    alert.sequence
                ));
                Ok(())
            }
        }
    }

    fn claim_session(&self, session_id: String) {
        self.state.lock().claimed_sessions.insert(session_id);
    }

    fn refresh_from_source(&self, cx: &App) -> Result<()> {
        let source = NautilusStreamSource::try_global(cx)
            .context("Nautilus stream source is unavailable")?;
        self.ingest(&source.read(cx).state_snapshot())
    }

    fn risk_snapshot(&self, allow_halted: bool) -> Result<RiskSnapshot> {
        let state = self.state.lock();
        if !allow_halted && let Some(reason) = &state.halted_reason {
            bail!("Nautilus governance is halted: {reason}");
        }
        let account = state
            .account
            .as_ref()
            .context("no Hyperliquid account state has been observed")?;
        let venue_balance =
            find_account_balance(account).context("account state has no recognized balance")?;
        let venue_balance_micros = decimal_to_units(&venue_balance, USDC_SCALE)?;
        let position_notional_usd = state.positions.iter().try_fold(0_u64, |total, position| {
            let quantity = find_decimal(position, &["quantity", "signed_qty", "size"])
                .unwrap_or_else(|| "0".into());
            let price = find_decimal(position, &["avg_px_open", "entry_price", "mark_price"])
                .unwrap_or_else(|| "0".into());
            let units = decimal_product_to_units(&quantity, &price, 0)?.unsigned_abs();
            total
                .checked_add(units)
                .context("position notional overflowed")
        })?;
        Ok(RiskSnapshot {
            venue_balance_micros: venue_balance_micros.unsigned_abs(),
            position_notional_usd,
        })
    }

    fn authorize(
        &self,
        action: VenueActionClass,
        extra_notional_usd: u64,
        emergency: bool,
    ) -> Result<u64> {
        let now = unix_ms()?;
        self.capabilities
            .guard(VENUE, action, CAPABILITY_MAX_AGE_MS)
            .require_effectful(now)?;
        let risk = self.risk_snapshot(emergency)?;
        let instruction = TradingInstruction {
            venue: VENUE.into(),
            network: TradingNetwork::Testnet,
            strategy_id: STRATEGY_ID.into(),
            collateral_asset: AssetId::usdc(),
            venue_balance_after: risk.venue_balance_micros,
            position_notional_usd: risk
                .position_notional_usd
                .saturating_add(extra_notional_usd),
            leverage: 1,
            daily_realized_loss: 0,
            orders_last_hour: self.orders_last_hour(now)?,
            liquidation_buffer_bps: 10_000,
        };
        match self.mandate.authorize(&instruction, now)? {
            MandateDecision::Authorized { revision } => Ok(revision),
            MandateDecision::Refused { reason, .. } => {
                bail!("trading mandate refused action: {reason:?}")
            }
        }
    }

    fn require_prediction(&self, prediction_id: &str, decision_id: &str) -> Result<()> {
        let prediction = self
            .predictions
            .events()?
            .into_iter()
            .find(|event| event.prediction_id == prediction_id)
            .context("governance action has no matching stored prediction")?;
        if prediction.draft.subsequent_decision_id != decision_id {
            bail!("prediction is linked to a different governance decision");
        }
        Ok(())
    }

    fn orders_last_hour(&self, now: i64) -> Result<u32> {
        let cutoff = now.saturating_sub(3_600_000);
        let count = self
            .ledger
            .entries(&LedgerQuery::default())?
            .into_iter()
            .filter(|entry| {
                entry.occurred_at_ms >= cutoff
                    && entry.kind == LedgerEntryKind::Order
                    && entry.metadata["receipt"]["command_type"] == "place_order"
            })
            .count();
        u32::try_from(count).context("hourly order count overflowed")
    }

    fn bounded_strategy_parameters(
        &self,
        mandate_revision: u64,
        min_reprice_interval_ms: u64,
        quote_offset_bps: u32,
    ) -> Result<StrategyParameters> {
        let snapshot = self.mandate.snapshot()?;
        if snapshot.revision != mandate_revision {
            bail!("trading mandate changed before strategy parameters were sent");
        }
        let mandate = snapshot
            .mandate_for(VENUE, TradingNetwork::Testnet)
            .context("testnet strategy has no active mandate")?;
        let risk = self.risk_snapshot(false)?;
        let position_headroom_usd = mandate
            .max_position_usd
            .checked_sub(risk.position_notional_usd)
            .context("testnet strategy mandate has no position headroom")?;
        let orders_used = self.orders_last_hour(unix_ms()?)?;
        let order_budget = mandate
            .max_orders_per_hour
            .checked_sub(orders_used)
            .context("testnet strategy mandate has no order-rate headroom")?;
        Ok(StrategyParameters {
            min_reprice_interval_ms,
            quote_offset_bps,
            order_quantity: "0.001".into(),
            position_headroom_usd,
            order_budget,
            mandate_revision,
        })
    }

    fn record_receipt(
        &self,
        receipt: &CommandReceipt,
        mandate_revision: u64,
        prediction_id: Option<&str>,
    ) -> Result<()> {
        let kind = match receipt.command_type {
            nautilus_sidecar::CommandType::PlaceOrder => LedgerEntryKind::Order,
            nautilus_sidecar::CommandType::CancelOrder => LedgerEntryKind::Cancel,
            _ => LedgerEntryKind::Order,
        };
        let mut draft = LedgerEntryDraft::new(
            format!("nautilus-command-{}", receipt.command_id),
            unix_ms()?,
            STRATEGY_ID,
            kind,
        );
        draft.metadata = json!({
            "schema":"omega.nautilus.command_receipt.v1",
            "mandate_revision":mandate_revision,
            "prediction_id":prediction_id,
            "receipt":receipt,
        });
        self.ledger.append(draft)?;
        Ok(())
    }

    fn next_command_id(&self, prefix: &str) -> String {
        let sequence = self.command_counter.fetch_add(1, Ordering::Relaxed);
        format!("omega-{prefix}-{sequence}")
    }

    fn snapshot(&self) -> Result<Value> {
        let state = self.state.lock().clone();
        let ledger = self.ledger.entries(&LedgerQuery::default())?;
        let mandate = self.mandate.snapshot()?;
        Ok(json!({
            "schema":"omega.nautilus.governance.v1",
            "network":"testnet",
            "state":state,
            "mandate":mandate,
            "ledger":ledger,
            "capabilities":self.capabilities.report(VENUE, unix_ms()?, CAPABILITY_MAX_AGE_MS),
        }))
    }
}

#[derive(Clone, Copy)]
struct RiskSnapshot {
    venue_balance_micros: u64,
    position_notional_usd: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostPath {
    TakerTaker,
    MakerTaker,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FillLiquidity {
    Maker,
    Taker,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CostLegObservation {
    pub quote_generation: u64,
    pub quote_sequence: u64,
    pub fill_generation: u64,
    pub fill_sequence: u64,
    pub client_order_id: String,
    pub side: OrderSide,
    pub liquidity: FillLiquidity,
    pub quantity: String,
    pub pre_trade_mid: String,
    pub fill_price: String,
    pub fee_usd: String,
    pub signed_adverse_slippage_micros_usd: i64,
    pub reference_notional_micros_usd: u64,
}

impl CostLegObservation {
    pub fn from_events(quote: &StreamEvent, fill: &StreamEvent) -> Result<Self> {
        let StreamEvent::Quote {
            generation: quote_generation,
            sequence: quote_sequence,
            bid_price,
            ask_price,
            ..
        } = quote
        else {
            bail!("cost observation requires a typed quote event");
        };
        let StreamEvent::Fill {
            generation: fill_generation,
            sequence: fill_sequence,
            state,
            ..
        } = fill
        else {
            bail!("cost observation requires a typed fill event");
        };
        let fill = Value::Object(state.clone());
        let client_order_id =
            find_string(&fill, &["client_order_id"]).context("fill has no client order ID")?;
        let side = match find_string(&fill, &["order_side", "side"])
            .context("fill has no side")?
            .to_ascii_lowercase()
            .as_str()
        {
            "buy" => OrderSide::Buy,
            "sell" => OrderSide::Sell,
            side => bail!("fill has unsupported side {side:?}"),
        };
        let liquidity = match find_string(&fill, &["liquidity_side"])
            .context("fill has no liquidity side")?
            .to_ascii_lowercase()
            .as_str()
        {
            "maker" => FillLiquidity::Maker,
            "taker" => FillLiquidity::Taker,
            liquidity => bail!("fill has unsupported liquidity {liquidity:?}"),
        };
        let quantity = find_string(&fill, &["last_qty", "quantity", "size"])
            .context("fill has no quantity")?;
        let fill_price = find_string(&fill, &["last_px", "price"]).context("fill has no price")?;
        let fee = find_string(&fill, &["commission", "fee"]).unwrap_or_else(|| "0 USDC".into());
        let fee_usd = fee.split_whitespace().next().context("fill fee is empty")?;
        let bid_units = i128::from(decimal_to_units(bid_price, 8)?);
        let ask_units = i128::from(decimal_to_units(ask_price, 8)?);
        let mid_units = bid_units
            .checked_add(ask_units)
            .context("quote mid overflowed")?
            / 2;
        let fill_units = i128::from(decimal_to_units(&fill_price, 8)?);
        let quantity_units = i128::from(decimal_to_units(&quantity, 8)?);
        let signed_price_delta = match side {
            OrderSide::Buy => fill_units.checked_sub(mid_units),
            OrderSide::Sell => mid_units.checked_sub(fill_units),
        }
        .context("slippage price delta overflowed")?;
        let signed_adverse_slippage_micros_usd = signed_price_delta
            .checked_mul(quantity_units)
            .context("slippage notional overflowed")?
            / 10_i128.pow(10);
        let reference_notional_micros_usd = mid_units
            .checked_mul(quantity_units)
            .context("reference notional overflowed")?
            / 10_i128.pow(10);
        Ok(Self {
            quote_generation: *quote_generation,
            quote_sequence: *quote_sequence,
            fill_generation: *fill_generation,
            fill_sequence: *fill_sequence,
            client_order_id,
            side,
            liquidity,
            quantity,
            pre_trade_mid: format_decimal_units(mid_units, 8)?,
            fill_price,
            fee_usd: fee_usd.into(),
            signed_adverse_slippage_micros_usd: i64::try_from(signed_adverse_slippage_micros_usd)?,
            reference_notional_micros_usd: u64::try_from(reference_notional_micros_usd)?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CostFloorSample {
    pub schema: String,
    pub network: TradingNetwork,
    pub path: CostPath,
    pub requested_notional_usd: u64,
    pub entry: CostLegObservation,
    pub exit: CostLegObservation,
    pub total_fee_micros_usd: i64,
    pub total_signed_adverse_slippage_micros_usd: i64,
    pub round_trip_cost_micros_bps: i64,
}

impl CostFloorSample {
    pub fn from_legs(
        requested_notional_usd: u64,
        entry: CostLegObservation,
        exit: CostLegObservation,
    ) -> Result<Self> {
        if entry.side == exit.side
            || decimal_to_units(&entry.quantity, 8)? != decimal_to_units(&exit.quantity, 8)?
        {
            bail!("cost sample requires opposite equal-sized legs");
        }
        if exit.liquidity != FillLiquidity::Taker {
            bail!("cost sample exit must be taker");
        }
        let path = match entry.liquidity {
            FillLiquidity::Maker => CostPath::MakerTaker,
            FillLiquidity::Taker => CostPath::TakerTaker,
        };
        let total_fee_micros_usd = decimal_to_units(&entry.fee_usd, USDC_SCALE)?
            .checked_add(decimal_to_units(&exit.fee_usd, USDC_SCALE)?)
            .context("cost sample fees overflowed")?;
        let total_signed_adverse_slippage_micros_usd = entry
            .signed_adverse_slippage_micros_usd
            .checked_add(exit.signed_adverse_slippage_micros_usd)
            .context("cost sample slippage overflowed")?;
        let total_cost = total_fee_micros_usd
            .checked_add(total_signed_adverse_slippage_micros_usd)
            .context("cost sample total overflowed")?;
        let reference = entry
            .reference_notional_micros_usd
            .checked_add(exit.reference_notional_micros_usd)
            .context("cost sample reference overflowed")?
            / 2;
        if reference == 0 {
            bail!("cost sample reference notional is zero");
        }
        let round_trip_cost_micros_bps = i128::from(total_cost)
            .checked_mul(i128::from(10_000 * BASIS_POINT_MICROS))
            .context("cost basis points overflowed")?
            / i128::from(reference);
        Ok(Self {
            schema: COST_FLOOR_SCHEMA.into(),
            network: TradingNetwork::Testnet,
            path,
            requested_notional_usd,
            entry,
            exit,
            total_fee_micros_usd,
            total_signed_adverse_slippage_micros_usd,
            round_trip_cost_micros_bps: i64::try_from(round_trip_cost_micros_bps)?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CostFloorSummary {
    pub schema: String,
    pub network: TradingNetwork,
    pub path: CostPath,
    pub requested_notional_usd: u64,
    pub sample_count: usize,
    pub median_cost_micros_bps: i64,
    pub margin_bps: u32,
    pub admission_floor_bps: u32,
}

impl CostFloorSummary {
    pub fn stable(samples: &[CostFloorSample], margin_bps: u32) -> Result<Self> {
        let first = samples.first().context("cost floor has no samples")?;
        if samples.len() < 5 {
            bail!("stable cost floor requires at least five completed samples");
        }
        if samples.iter().any(|sample| {
            sample.network != TradingNetwork::Testnet
                || sample.path != first.path
                || sample.requested_notional_usd != first.requested_notional_usd
        }) {
            bail!("cost floor samples do not describe one testnet configuration");
        }
        let mut costs = samples
            .iter()
            .map(|sample| sample.round_trip_cost_micros_bps)
            .collect::<Vec<_>>();
        costs.sort_unstable();
        let median_cost_micros_bps = costs[costs.len() / 2];
        let measured_ceiling_bps = median_cost_micros_bps
            .max(0)
            .checked_add(BASIS_POINT_MICROS - 1)
            .context("cost floor ceiling overflowed")?
            / BASIS_POINT_MICROS;
        let admission_floor_bps = u32::try_from(measured_ceiling_bps)?
            .checked_add(margin_bps)
            .context("admission floor overflowed")?;
        Ok(Self {
            schema: COST_FLOOR_SCHEMA.into(),
            network: TradingNetwork::Testnet,
            path: first.path,
            requested_notional_usd: first.requested_notional_usd,
            sample_count: samples.len(),
            median_cost_micros_bps,
            margin_bps,
            admission_floor_bps,
        })
    }
}

pub struct NautilusGovernancePlugin;

impl NautilusGovernancePlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NautilusGovernancePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl plugin_api::OmegaPlugin for NautilusGovernancePlugin {
    fn manifest(&self) -> &'static PluginManifest {
        &MANIFEST
    }

    fn register(&self, registry: &mut PluginRegistry, cx: &mut App) {
        registry.add_settings_page(trading_workspace_ui::settings_page_registration());
        let runtime = match GovernanceRuntime::open_default(registry.venue_capabilities()) {
            Ok(runtime) => runtime,
            Err(error) => {
                log_error("initialize Nautilus governance", &error);
                return;
            }
        };
        if let Some(source) = NautilusStreamSource::try_global(cx) {
            let observed_runtime = runtime.clone();
            cx.observe(&source, move |source, cx| {
                let events = source.read(cx).state_snapshot();
                if let Err(error) = observed_runtime.ingest(&events) {
                    observed_runtime.halt(format!("stream ingestion failed: {error:#}"));
                }
            })
            .detach();
        }
        if let Some(state) = credential_state(cx) {
            let observed_runtime = runtime.clone();
            observed_runtime.set_credential_wakeup(state.read(cx).snapshot().wakeup);
            cx.observe(&state, move |state, cx| {
                observed_runtime.set_credential_wakeup(state.read(cx).snapshot().wakeup);
            })
            .detach();
        }
        registry.add_review_driver(std::rc::Rc::new(GovernanceReviewDriver {
            runtime: runtime.clone(),
        }));
        registry.add_extension(agent_tools_registration(runtime));
        for schema in [
            "omega.nautilus.governance.v1",
            "omega.nautilus.command_receipt.v1",
            "omega.nautilus.ledger.fill.v1",
        ]
        .into_iter()
        .chain(card_renderers::CARD_SCHEMAS)
        .chain(std::iter::once(card_renderers::VERIFICATION_SCHEMA))
        {
            registry.add_card_schema(CardSchemaRegistration {
                plugin_id: MANIFEST.id,
                schema,
            });
        }
        for renderer in card_renderers::card_renderer_registrations() {
            if let Err(error) = registry.add_card_renderer(renderer) {
                log::error!("could not register a Nautilus governance card renderer: {error}");
            }
        }
    }
}

fn agent_tools_registration(runtime: GovernanceRuntime) -> agent::PluginAgentTools {
    agent::PluginAgentTools {
        plugin_id: MANIFEST.id,
        tool_names: &[
            AccountTool::NAME,
            PredictionTool::NAME,
            StrategyTool::NAME,
            OrderTool::NAME,
            RiskTool::NAME,
        ],
        default_enabled_profiles: &["basic", "editor"],
        build: std::rc::Rc::new(move |context, cx| {
            runtime.claim_session(context.session_id.clone());
            let channel = nautilus_sidecar::command_channel(cx);
            vec![
                AccountTool {
                    runtime: runtime.clone(),
                }
                .erase(),
                PredictionTool {
                    runtime: runtime.clone(),
                    session_id: context.session_id.clone(),
                }
                .erase(),
                StrategyTool {
                    runtime: runtime.clone(),
                    channel: channel.clone(),
                }
                .erase(),
                OrderTool {
                    runtime: runtime.clone(),
                    channel: channel.clone(),
                }
                .erase(),
                RiskTool {
                    runtime: runtime.clone(),
                    channel,
                }
                .erase(),
            ]
        }),
    }
}

struct GovernanceReviewDriver {
    runtime: GovernanceRuntime,
}

impl SessionReviewDriver for GovernanceReviewDriver {
    fn review_cadence(
        &self,
        session_id: &str,
        _cx: &App,
    ) -> Result<Option<PluginReviewCadence>, String> {
        if !self
            .runtime
            .state
            .lock()
            .claimed_sessions
            .contains(session_id)
        {
            return Ok(None);
        }
        let snapshot = self
            .runtime
            .mandate
            .snapshot()
            .map_err(|error| error.to_string())?;
        let cadence = snapshot.mandate_for(VENUE, TradingNetwork::Testnet).map_or(
            PluginReviewCadence::EventDriven,
            |mandate| match mandate.review_cadence {
                MandateReviewCadence::FundingSettlement => PluginReviewCadence::EventDriven,
                MandateReviewCadence::Interval { seconds } => {
                    PluginReviewCadence::Interval { seconds }
                }
            },
        );
        Ok(Some(cadence))
    }

    fn review_token_budget(&self) -> u64 {
        4_096
    }

    fn pending_wakeup(
        &self,
        session_id: &str,
        _cx: &App,
    ) -> Option<(plugin_api::WakeupSource, String)> {
        let state = self.runtime.state.lock();
        if !state.claimed_sessions.contains(session_id) {
            return None;
        }
        if let Some(source) = state.pending_credential_wakeup.clone() {
            return Some((
                source,
                "Hyperliquid agent-wallet authority halted fail-closed. Inspect the named network, extraAgents approval, and validUntil before resuming; do not increase risk."
                    .to_owned(),
            ));
        }
        state.pending_wakeup.as_ref().map(|reason| (
            plugin_api::WakeupSource::StrategyHalt { strategy: STRATEGY_ID.into(), reason: reason.clone() },
            format!("Nautilus governance halted fail-closed: {reason}. Read account, ledger, and engine state; do not increase risk."),
        ))
    }

    fn review_instruction(
        &self,
        session_id: &str,
        _now_ms: i64,
        trigger: &str,
        _cx: &App,
    ) -> Result<Option<String>, String> {
        if !self
            .runtime
            .state
            .lock()
            .claimed_sessions
            .contains(session_id)
        {
            return Ok(None);
        }
        Ok(Some(format!(
            "Review Hyperliquid TESTNET governance after {trigger}. Read nautilus_account first. Record exactly one nautilus_prediction before any start, parameter, or risk-increasing order decision; a no-change review still records a flat prediction. Emergency stop/cancel/flatten may proceed without a prediction. Never use MCP in the Nautilus tick loop."
        )))
    }

    fn acknowledge_wakeup(
        &self,
        session_id: &str,
        source: &plugin_api::WakeupSource,
        instruction: &str,
        _cx: &App,
    ) -> bool {
        let mut state = self.runtime.state.lock();
        if !state.claimed_sessions.contains(session_id) {
            return false;
        }
        if state.pending_credential_wakeup.as_ref() == Some(source)
            && instruction.contains("agent-wallet authority halted")
        {
            state.pending_credential_wakeup = None;
            return true;
        }
        if state.pending_wakeup.is_none() {
            return false;
        }
        if !instruction.contains("Nautilus governance halted") {
            return false;
        }
        state.pending_wakeup = None;
        true
    }

    fn record_review_turn(
        &self,
        session_id: &str,
        _at_ms: i64,
        _source: plugin_api::WakeupSource,
        _outcome: ReviewTurnOutcome,
        _cx: &App,
    ) -> bool {
        self.runtime
            .state
            .lock()
            .claimed_sessions
            .contains(session_id)
    }

    fn evidence_tool_names(&self) -> &'static [&'static str] {
        &[
            AccountTool::NAME,
            PredictionTool::NAME,
            StrategyTool::NAME,
            OrderTool::NAME,
            RiskTool::NAME,
        ]
    }

    fn record_review_evidence(
        &self,
        session_id: &str,
        evidence: ReviewTurnEvidence,
        _cx: &App,
    ) -> bool {
        if !valid_review_sequence(&evidence) {
            return false;
        }
        let mut kinds = Vec::new();
        for call in &evidence.tool_calls {
            let input = call.input.get("value").unwrap_or(&call.input);
            let action = input.get("action").and_then(Value::as_str);
            match (call.name.as_str(), action) {
                ("nautilus_strategy", Some("set_parameters")) => {
                    kinds.push(ReviewInterventionKind::ParameterChange)
                }
                ("nautilus_strategy", Some("start")) | ("nautilus_order", Some("place")) => {
                    kinds.push(ReviewInterventionKind::Intent)
                }
                ("nautilus_strategy", Some("stop"))
                | ("nautilus_order", Some("cancel"))
                | ("nautilus_risk", Some("flatten" | "reduce")) => {
                    kinds.push(ReviewInterventionKind::HaltResponse)
                }
                _ => {}
            }
        }
        kinds.sort();
        kinds.dedup();
        let disposition = if kinds.is_empty() {
            ReviewDisposition::NoChange
        } else {
            ReviewDisposition::Intervention { kinds }
        };
        let decision = match &disposition {
            ReviewDisposition::NoChange => json!({"type":"no_change"}),
            ReviewDisposition::Intervention { .. } => {
                json!({"type":"action","summary":"bounded_intervention"})
            }
        };
        let review_payload = json!({
            "schema":"omega.market.review-turn.v1",
            "at_ms":evidence.at_ms,
            "trigger":evidence.source.transcript_label(),
            "read_sources":evidence.tool_calls.iter().map(|call| call.name.as_str()).collect::<Vec<_>>(),
            "prediction":evidence.tool_calls.iter().any(|call| call.name == PredictionTool::NAME).then_some("recorded"),
            "decision":decision,
            "model_id":evidence.model_id,
            "input_tokens":evidence.token_usage.input_total(),
            "output_tokens":evidence.token_usage.output_tokens,
            "token_cost_microusd":Value::Null,
            "wall_clock_ms":evidence.wall_clock_ms,
        });
        let record = ReviewCostRecord {
            schema_version: REVIEW_ACCOUNTING_SCHEMA_VERSION,
            turn_id: format!("{session_id}:{}", evidence.at_ms),
            session_id: session_id.into(),
            started_at_ms: evidence.at_ms,
            completed_at_ms: evidence.completed_at_ms,
            wall_clock_ms: evidence.wall_clock_ms,
            model_id: evidence.model_id,
            token_usage: evidence.token_usage,
            tool_calls: evidence.tool_calls,
            disposition,
            source: evidence.source,
            venues: vec![VENUE.into()],
            strategies: vec![STRATEGY_ID.into()],
        };
        if self.runtime.review_accounting.append(record).is_err() {
            return false;
        }
        self.runtime.state.lock().latest_review = Some(review_payload);
        true
    }
}

fn valid_review_sequence(evidence: &ReviewTurnEvidence) -> bool {
    let prediction_index = evidence
        .tool_calls
        .iter()
        .position(|call| call.name == PredictionTool::NAME);
    if matches!(
        evidence.source,
        plugin_api::WakeupSource::ScheduledReview { .. }
    ) && prediction_index.is_none()
    {
        return false;
    }
    for (index, call) in evidence.tool_calls.iter().enumerate() {
        let input = call.input.get("value").unwrap_or(&call.input);
        let requires_prediction = matches!(
            (
                call.name.as_str(),
                input.get("action").and_then(Value::as_str)
            ),
            ("nautilus_strategy", Some("start" | "set_parameters"))
                | ("nautilus_order", Some("place"))
        );
        if requires_prediction
            && !prediction_index.is_some_and(|prediction_index| prediction_index < index)
        {
            return false;
        }
    }
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolOutput(Value);

impl ToolOutput {
    fn ok(value: Value) -> Self {
        Self(value)
    }
    fn error(error: impl Into<String>) -> Self {
        Self(json!({"error":error.into()}))
    }
}

impl From<ToolOutput> for LanguageModelToolResultContent {
    fn from(output: ToolOutput) -> Self {
        serde_json::to_string_pretty(&output.0)
            .unwrap_or_else(|error| {
                format!("could not serialize Nautilus governance output: {error}")
            })
            .into()
    }
}

fn emit_card_update(events: &ToolCallEventStream, payload: &Value, title: &'static str) {
    let content = serde_json::to_string_pretty(payload)
        .unwrap_or_else(|error| format!("could not serialize inline market card: {error}"));
    events.update_fields(acp::ToolCallUpdateFields::new().title(title).content(vec![
        acp::ToolCallContent::Content(acp::Content::new(content)),
    ]));
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AccountInput {
    Account,
    Positions,
    Exposure,
    Ledger,
    CandleLite,
    Sparkline,
    Review,
}

struct AccountTool {
    runtime: GovernanceRuntime,
}

impl AgentTool for AccountTool {
    type Input = AccountInput;
    type Output = ToolOutput;
    const NAME: &'static str = "nautilus_account";
    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }
    fn initial_title(&self, _input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        "Read Nautilus testnet governance state".into()
    }
    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _events: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let refresh = self
            .runtime
            .refresh_from_source(cx)
            .map_err(|error| ToolOutput::error(error.to_string()));
        let market_snapshot =
            NautilusStreamSource::try_global(cx).map(|source| source.read(cx).market_snapshot());
        cx.spawn(async move |_cx| {
            refresh?;
            let input = input
                .recv()
                .await
                .map_err(|error| ToolOutput::error(error.to_string()))?;
            let card_kind = match input {
                AccountInput::Positions => Some(card_renderers::LiveCardKind::Positions),
                AccountInput::CandleLite => Some(card_renderers::LiveCardKind::CandleLite),
                AccountInput::Sparkline => Some(card_renderers::LiveCardKind::Sparkline),
                AccountInput::Account | AccountInput::Exposure | AccountInput::Ledger => None,
                AccountInput::Review => {
                    return self
                        .runtime
                        .state
                        .lock()
                        .latest_review
                        .clone()
                        .map(ToolOutput::ok)
                        .ok_or_else(|| ToolOutput::error("no Nautilus review turn has completed"));
                }
            };
            if let Some(kind) = card_kind {
                let snapshot = market_snapshot
                    .as_ref()
                    .ok_or_else(|| ToolOutput::error("Nautilus stream source is unavailable"))?;
                return card_renderers::live_card_payload(snapshot, kind)
                    .map(ToolOutput::ok)
                    .ok_or_else(|| ToolOutput::error("Nautilus stream has no card data yet"));
            }
            self.runtime
                .snapshot()
                .map(ToolOutput::ok)
                .map_err(|error| ToolOutput::error(error.to_string()))
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Up,
    Down,
    Flat,
}

impl From<Direction> for PredictedDirection {
    fn from(value: Direction) -> Self {
        match value {
            Direction::Up => Self::Up,
            Direction::Down => Self::Down,
            Direction::Flat => Self::Flat,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PredictionInput {
    instrument: String,
    direction: Direction,
    confidence_micros: u32,
    horizon_ms: u64,
    decision_id: String,
    #[serde(default)]
    observation_refs: Vec<String>,
}

struct PredictionTool {
    runtime: GovernanceRuntime,
    session_id: String,
}

impl AgentTool for PredictionTool {
    type Input = PredictionInput;
    type Output = ToolOutput;
    const NAME: &'static str = "nautilus_prediction";
    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }
    fn initial_title(&self, _input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        "Record Nautilus governance prediction".into()
    }
    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _events: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |_cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| ToolOutput::error(error.to_string()))?;
            let now = unix_ms().map_err(|error| ToolOutput::error(error.to_string()))?;
            let resolve_at_ms = now
                .checked_add(
                    i64::try_from(input.horizon_ms)
                        .map_err(|error| ToolOutput::error(error.to_string()))?,
                )
                .ok_or_else(|| ToolOutput::error("prediction resolution time overflowed"))?;
            let event = self
                .runtime
                .predictions
                .append(PredictionEventDraft {
                    schema_version: PREDICTION_SCHEMA_VERSION,
                    emitted_at_ms: now,
                    actor: PredictionActor::Agent {
                        agent_id: self.session_id.clone(),
                    },
                    mandate_scope: MandateScope {
                        venue: VENUE.into(),
                        network: TradingNetwork::Testnet,
                    },
                    instrument: input.instrument,
                    forecast: PredictionForecast::Directional {
                        direction: input.direction.into(),
                        probability_micros: input.confidence_micros,
                    },
                    confidence_micros: input.confidence_micros,
                    horizon_ms: input.horizon_ms,
                    resolution_rule: ResolutionRule {
                        source: "nautilus.testnet.quote.v1".into(),
                        baseline_at_ms: now,
                        resolve_at_ms,
                        flat_tolerance_bps: 10,
                    },
                    scoring_rule: ScoringRule::Brier,
                    observation_refs: input.observation_refs,
                    private_payload_ref: None,
                    subsequent_decision_id: input.decision_id,
                })
                .map_err(|error| ToolOutput::error(error.to_string()))?;
            Ok(ToolOutput::ok(
                json!({"schema":"omega.market.prediction.v1","prediction":event}),
            ))
        })
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
enum StrategyInput {
    Start {
        prediction_id: String,
        decision_id: String,
    },
    Stop,
    SetParameters {
        min_reprice_interval_ms: u64,
        quote_offset_bps: u32,
        prediction_id: String,
        decision_id: String,
    },
}

struct StrategyTool {
    runtime: GovernanceRuntime,
    channel: Option<NautilusCommandChannel>,
}

impl AgentTool for StrategyTool {
    type Input = StrategyInput;
    type Output = ToolOutput;
    const NAME: &'static str = "nautilus_strategy";
    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }
    fn initial_title(&self, _input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        "Govern Nautilus testnet strategy".into()
    }
    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _events: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let refresh = self
            .runtime
            .refresh_from_source(cx)
            .map_err(|error| ToolOutput::error(error.to_string()));
        cx.spawn(async move |_cx| {
            refresh?;
            let input = input.recv().await.map_err(|error| ToolOutput::error(error.to_string()))?;
            let (prediction_id, emergency) = match &input {
                StrategyInput::Start { prediction_id, decision_id } => {
                    self.runtime.require_prediction(&prediction_id, &decision_id).map_err(|error| ToolOutput::error(error.to_string()))?;
                    (Some(prediction_id.clone()), false)
                }
                StrategyInput::Stop => (None, true),
                StrategyInput::SetParameters { prediction_id, decision_id, .. } => {
                    self.runtime.require_prediction(&prediction_id, &decision_id).map_err(|error| ToolOutput::error(error.to_string()))?;
                    (Some(prediction_id.clone()), false)
                }
            };
            let revision = self.runtime.authorize(VenueActionClass::StrategyExecution, 0, emergency).map_err(|error| ToolOutput::error(error.to_string()))?;
            let command = match input {
                StrategyInput::Start { .. } => NautilusCommand::StartStrategy { strategy_id: STRATEGY_ID.into() },
                StrategyInput::Stop => NautilusCommand::StopStrategy { strategy_id: STRATEGY_ID.into() },
                StrategyInput::SetParameters { min_reprice_interval_ms, quote_offset_bps, .. } => {
                    let parameters = self.runtime.bounded_strategy_parameters(revision, min_reprice_interval_ms, quote_offset_bps).map_err(|error| ToolOutput::error(error.to_string()))?;
                    NautilusCommand::SetStrategyParameters { strategy_id: STRATEGY_ID.into(), parameters }
                }
            };
            let receipt = send_once(self.channel.as_ref(), CommandRequest { command_id: self.runtime.next_command_id("strategy"), command }).await?;
            self.runtime.record_receipt(&receipt, revision, prediction_id.as_deref()).map_err(|error| ToolOutput::error(error.to_string()))?;
            Ok(ToolOutput::ok(json!({"schema":"omega.nautilus.strategy.v1","mandate_revision":revision,"receipt":receipt})))
        })
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
enum OrderInput {
    Place {
        client_order_id: String,
        instrument_id: String,
        side: ToolSide,
        quantity: String,
        price: String,
        post_only: bool,
        reduce_only: bool,
        prediction_id: String,
        decision_id: String,
    },
    Cancel {
        client_order_id: String,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ToolSide {
    Buy,
    Sell,
}

impl From<ToolSide> for OrderSide {
    fn from(side: ToolSide) -> Self {
        match side {
            ToolSide::Buy => Self::Buy,
            ToolSide::Sell => Self::Sell,
        }
    }
}

struct OrderTool {
    runtime: GovernanceRuntime,
    channel: Option<NautilusCommandChannel>,
}

impl AgentTool for OrderTool {
    type Input = OrderInput;
    type Output = ToolOutput;
    const NAME: &'static str = "nautilus_order";
    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }
    fn initial_title(&self, _input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        "Confirm Nautilus testnet order".into()
    }
    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        events: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let refresh = self
            .runtime
            .refresh_from_source(cx)
            .map_err(|error| ToolOutput::error(error.to_string()));
        cx.spawn(async move |cx| {
            refresh?;
            let input = input.recv().await.map_err(|error| ToolOutput::error(error.to_string()))?;
            let summary = serde_json::to_string(&input).map_err(|error| ToolOutput::error(error.to_string()))?;
            let authorize = cx.update(|cx| events.authorize_always_prompt(
                "Confirm this discrete Hyperliquid TESTNET order",
                ToolPermissionContext::new(Self::NAME, vec![summary]), cx,
            ));
            authorize.await.map_err(|error| ToolOutput::error(error.to_string()))?;
            let (command, action, extra_notional, prediction_id, emergency, pending_card, total_units) = match input {
                OrderInput::Place { client_order_id, instrument_id, side, quantity, price, post_only, reduce_only, prediction_id, decision_id } => {
                    self.runtime.require_prediction(&prediction_id, &decision_id).map_err(|error| ToolOutput::error(error.to_string()))?;
                    let extra = if reduce_only { 0 } else { decimal_product_to_units(&quantity, &price, 0).map_err(|error| ToolOutput::error(error.to_string()))?.unsigned_abs() };
                    let total_units = quantity.parse::<f64>().ok().filter(|value| value.is_finite() && *value >= 0.0).unwrap_or(0.0);
                    let pending_card = json!({
                        "schema":"omega.market.order-lifecycle.v1",
                        "network":"testnet",
                        "order_id":client_order_id,
                        "stage":"placed",
                        "filled_units":0.0,
                        "total_units":total_units,
                    });
                    (NautilusCommand::PlaceOrder { client_order_id, instrument_id, side: side.into(), quantity, price, time_in_force: OrderTimeInForce::Gtc, post_only, reduce_only }, VenueActionClass::OrderPlacement, extra, Some(prediction_id), reduce_only, Some(pending_card), total_units)
                }
                OrderInput::Cancel { client_order_id } => (NautilusCommand::CancelOrder { client_order_id }, VenueActionClass::OrderCancellation, 0, None, true, None, 0.0),
            };
            let revision = self.runtime.authorize(action, extra_notional, emergency).map_err(|error| ToolOutput::error(error.to_string()))?;
            if let Some(pending_card) = pending_card.as_ref() {
                emit_card_update(&events, pending_card, "Nautilus testnet order: placed");
            }
            let receipt = send_once(self.channel.as_ref(), CommandRequest { command_id: self.runtime.next_command_id("order"), command }).await?;
            self.runtime.record_receipt(&receipt, revision, prediction_id.as_deref()).map_err(|error| ToolOutput::error(error.to_string()))?;
            let mut output = card_renderers::order_receipt_payload(&receipt, total_units)
                .unwrap_or_else(|| json!({"schema":"omega.nautilus.order.v1","network":"testnet","receipt":receipt}));
            if let Some(output) = output.as_object_mut() {
                output.insert("mandate_revision".into(), revision.into());
                output.insert("single_attempt".into(), true.into());
                output.insert("prediction_id".into(), prediction_id.into());
            }
            Ok(ToolOutput::ok(output))
        })
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
enum RiskInput {
    Reduce {
        client_order_id: String,
        instrument_id: String,
        side: ToolSide,
        quantity: String,
        limit_price: String,
    },
    Flatten {
        client_order_id: String,
        instrument_id: String,
        side: ToolSide,
        quantity: String,
        limit_price: String,
    },
}

struct RiskTool {
    runtime: GovernanceRuntime,
    channel: Option<NautilusCommandChannel>,
}

impl AgentTool for RiskTool {
    type Input = RiskInput;
    type Output = ToolOutput;
    const NAME: &'static str = "nautilus_risk";
    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }
    fn initial_title(&self, _input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        "Reduce Nautilus testnet risk".into()
    }
    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _events: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let refresh = self
            .runtime
            .refresh_from_source(cx)
            .map_err(|error| ToolOutput::error(error.to_string()));
        cx.spawn(async move |_cx| {
            refresh?;
            let input = input.recv().await.map_err(|error| ToolOutput::error(error.to_string()))?;
            let (client_order_id, instrument_id, side, quantity, price, action) = match input {
                RiskInput::Reduce { client_order_id, instrument_id, side, quantity, limit_price } => (client_order_id, instrument_id, side, quantity, limit_price, "reduce"),
                RiskInput::Flatten { client_order_id, instrument_id, side, quantity, limit_price } => (client_order_id, instrument_id, side, quantity, limit_price, "flatten"),
            };
            let revision = self.runtime.authorize(VenueActionClass::OrderPlacement, 0, true).map_err(|error| ToolOutput::error(error.to_string()))?;
            let receipt = send_once(self.channel.as_ref(), CommandRequest {
                command_id: self.runtime.next_command_id(action),
                command: NautilusCommand::PlaceOrder { client_order_id, instrument_id, side: side.into(), quantity, price, time_in_force: OrderTimeInForce::Ioc, post_only: false, reduce_only: true },
            }).await?;
            self.runtime.record_receipt(&receipt, revision, None).map_err(|error| ToolOutput::error(error.to_string()))?;
            Ok(ToolOutput::ok(json!({"schema":"omega.nautilus.risk.v1","action":action,"emergency_prediction_exemption":true,"mandate_revision":revision,"single_attempt":true,"receipt":receipt})))
        })
    }
}

async fn send_once(
    channel: Option<&NautilusCommandChannel>,
    request: CommandRequest,
) -> Result<CommandReceipt, ToolOutput> {
    let channel =
        channel.ok_or_else(|| ToolOutput::error("Nautilus command channel is unavailable"))?;
    channel.send(request).await.map_err(|error| {
        ToolOutput::error(format!(
            "command outcome is unknown; do not retry: {error:#}"
        ))
    })
}

fn stream_event_key(event: &StreamEvent) -> String {
    match event {
        StreamEvent::Quote {
            generation,
            sequence,
            ..
        } => format!("quote-{generation}-{sequence}"),
        StreamEvent::Trade {
            generation,
            sequence,
            ..
        } => format!("trade-{generation}-{sequence}"),
        StreamEvent::Book {
            generation,
            sequence,
            ..
        } => format!("book-{generation}-{sequence}"),
        StreamEvent::Account {
            generation,
            sequence,
            ..
        } => format!("account-{generation}-{sequence}"),
        StreamEvent::Order {
            generation,
            sequence,
            ..
        } => format!("order-{generation}-{sequence}"),
        StreamEvent::OrderState {
            generation,
            sequence,
            ..
        } => format!("order-state-{generation}-{sequence}"),
        StreamEvent::Position {
            generation,
            sequence,
            ..
        } => format!("position-{generation}-{sequence}"),
        StreamEvent::PositionState {
            generation,
            sequence,
            ..
        } => format!("position-state-{generation}-{sequence}"),
        StreamEvent::Fill {
            generation,
            sequence,
            ..
        } => format!("fill-{generation}-{sequence}"),
        StreamEvent::StrategyState {
            generation,
            sequence,
            ..
        } => format!("strategy-state-{generation}-{sequence}"),
    }
}

fn bounded_latest(values: &[Value]) -> Vec<Value> {
    let start = values.len().saturating_sub(GOVERNANCE_STATE_CAPACITY);
    values[start..].to_vec()
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key) {
                    match found {
                        Value::String(value) => return Some(value.clone()),
                        Value::Number(value) => return Some(value.to_string()),
                        _ => {}
                    }
                }
            }
            map.values().find_map(|child| find_string(child, keys))
        }
        Value::Array(values) => values.iter().find_map(|child| find_string(child, keys)),
        _ => None,
    }
}

fn find_decimal(value: &Value, keys: &[&str]) -> Option<String> {
    find_string(value, keys)
}

fn reconciled_fill_state(order: &Value) -> Option<(String, serde_json::Map<String, Value>)> {
    if !find_string(order, &["status"])?.eq_ignore_ascii_case("FILLED") {
        return None;
    }
    let venue_order_id = find_string(order, &["venue_order_id"])?;
    let commission = order
        .get("commissions")
        .and_then(Value::as_array)
        .and_then(|commissions| commissions.first())
        .and_then(Value::as_str)
        .unwrap_or("0 USDC");
    let mut fill = serde_json::Map::new();
    for (target, sources) in [
        ("instrument_id", &["instrument_id"][..]),
        ("client_order_id", &["client_order_id"][..]),
        ("order_side", &["side", "order_side"][..]),
        ("last_qty", &["filled_qty", "quantity"][..]),
        ("last_px", &["avg_px", "price"][..]),
        ("liquidity_side", &["liquidity_side"][..]),
    ] {
        fill.insert(target.into(), Value::String(find_string(order, sources)?));
    }
    fill.insert(
        "venue_order_id".into(),
        Value::String(venue_order_id.clone()),
    );
    fill.insert("commission".into(), Value::String(commission.into()));
    Some((venue_order_id, fill))
}

fn find_account_balance(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    for key in ["account_value", "equity", "total_balance"] {
        if let Some(value) = object.get(key) {
            match value {
                Value::String(value) => return Some(value.clone()),
                Value::Number(value) => return Some(value.to_string()),
                _ => {}
            }
        }
    }
    find_usdc_balance(value).or_else(|| find_decimal(value, &["total", "balance"]))
}

fn find_usdc_balance(value: &Value) -> Option<String> {
    match value {
        Value::Object(object) => {
            let currency = object
                .get("currency")
                .or_else(|| object.get("asset"))
                .and_then(Value::as_str);
            if currency.is_some_and(|currency| currency.eq_ignore_ascii_case("usdc")) {
                for key in ["total", "balance", "free"] {
                    if let Some(value) = object.get(key) {
                        match value {
                            Value::String(value) => return Some(value.clone()),
                            Value::Number(value) => return Some(value.to_string()),
                            _ => {}
                        }
                    }
                }
            }
            for value in object.values() {
                if let Value::String(value) = value
                    && value
                        .split_whitespace()
                        .nth(1)
                        .is_some_and(|currency| currency.eq_ignore_ascii_case("usdc"))
                {
                    return Some(value.clone());
                }
                if let Some(balance) = find_usdc_balance(value) {
                    return Some(balance);
                }
            }
            None
        }
        Value::Array(values) => values.iter().find_map(find_usdc_balance),
        _ => None,
    }
}

fn decimal_to_units(value: &str, scale: u32) -> Result<i64> {
    let value = value
        .split_whitespace()
        .next()
        .context("decimal is empty")?;
    let (negative, unsigned) = value
        .strip_prefix('-')
        .map_or((false, value), |value| (true, value));
    let mut parts = unsigned.split('.');
    let whole = parts.next().context("decimal has no whole component")?;
    let fraction = parts.next().unwrap_or("");
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        bail!("invalid decimal {value:?}");
    }
    let factor = 10_i128
        .checked_pow(scale)
        .context("decimal scale overflowed")?;
    let whole = whole
        .parse::<i128>()
        .context("decimal whole component overflowed")?;
    let scale_usize = usize::try_from(scale).context("decimal scale exceeds usize")?;
    let kept = fraction.chars().take(scale_usize).collect::<String>();
    let padded = format!("{kept:0<width$}", width = scale_usize);
    let fractional = if padded.is_empty() {
        0
    } else {
        padded
            .parse::<i128>()
            .context("decimal fraction overflowed")?
    };
    let units = whole
        .checked_mul(factor)
        .and_then(|whole| whole.checked_add(fractional))
        .context("decimal units overflowed")?;
    let signed = if negative { -units } else { units };
    i64::try_from(signed).context("decimal exceeds ledger integer range")
}

fn format_decimal_units(value: i128, scale: u32) -> Result<String> {
    let factor = 10_i128
        .checked_pow(scale)
        .context("decimal format scale overflowed")?;
    let sign = if value < 0 { "-" } else { "" };
    let value = value.abs();
    let whole = value / factor;
    let fraction = value % factor;
    let width = usize::try_from(scale).context("decimal format scale exceeds usize")?;
    Ok(format!("{sign}{whole}.{fraction:0>width$}"))
}

fn decimal_product_to_units(left: &str, right: &str, scale: u32) -> Result<i64> {
    let left_units = decimal_to_units(left, 8)? as i128;
    let right_units = decimal_to_units(right, 8)? as i128;
    let product = left_units
        .checked_mul(right_units)
        .context("decimal product overflowed")?;
    let divisor_scale = 16_u32
        .checked_sub(scale)
        .context("requested product scale exceeds parser precision")?;
    let divisor = 10_i128
        .checked_pow(divisor_scale)
        .context("decimal product divisor overflowed")?;
    i64::try_from(product / divisor).context("decimal product exceeds ledger integer range")
}

fn unix_ms() -> Result<i64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_millis();
    i64::try_from(millis).context("Unix timestamp exceeds i64")
}

fn log_error(context: &str, error: &anyhow::Error) {
    log::error!("{context}: {error:#}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_sidecar::{OfficialVenueState, evaluate_official_venue_state};
    use std::{collections::BTreeSet, path::PathBuf, time::Duration};
    use trading_mandate::TradingMandate;

    fn cost_quote(sequence: u64, bid: &str, ask: &str) -> StreamEvent {
        StreamEvent::Quote {
            schema: "omega.nautilus.stream.v1".into(),
            generation: 1,
            sequence,
            network: nautilus_sidecar::Network::Testnet,
            instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
            bid_price: bid.into(),
            ask_price: ask.into(),
            bid_size: "1".into(),
            ask_size: "1".into(),
            ts_event: sequence,
            ts_init: sequence,
        }
    }

    fn cost_fill(
        sequence: u64,
        client_order_id: &str,
        side: &str,
        liquidity: &str,
        price: &str,
        fee: &str,
    ) -> StreamEvent {
        StreamEvent::Fill {
            schema: "omega.nautilus.stream.v1".into(),
            generation: 1,
            sequence,
            network: nautilus_sidecar::Network::Testnet,
            state: serde_json::from_value(json!({
                "instrument_id":"BTC-USD-PERP.HYPERLIQUID",
                "client_order_id":client_order_id,
                "order_side":side,
                "liquidity_side":liquidity,
                "last_qty":"1",
                "last_px":price,
                "commission":format!("{fee} USDC"),
            }))
            .expect("valid fill state"),
        }
    }

    fn next_testnet_quote(
        supervisor: &nautilus_sidecar::NautilusSupervisor,
        timeout: Duration,
    ) -> Result<StreamEvent> {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            let frame = supervisor.take_stream_frame()?;
            if let Some(quote) = frame.quote {
                return Ok(quote);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        bail!("no typed testnet quote arrived before the measurement deadline")
    }

    fn official_testnet_state(owner_address: &str) -> Result<OfficialVenueState> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .context("build Hyperliquid testnet safety client")?;
        let open_orders = client
            .post("https://api.hyperliquid-testnet.xyz/info")
            .json(&json!({"type":"openOrders","user":owner_address}))
            .send()
            .context("query Hyperliquid testnet openOrders")?
            .error_for_status()
            .context("Hyperliquid testnet openOrders status")?
            .bytes()
            .context("read Hyperliquid testnet openOrders")?;
        let clearinghouse = client
            .post("https://api.hyperliquid-testnet.xyz/info")
            .json(&json!({"type":"clearinghouseState","user":owner_address}))
            .send()
            .context("query Hyperliquid testnet clearinghouseState")?
            .error_for_status()
            .context("Hyperliquid testnet clearinghouseState status")?
            .bytes()
            .context("read Hyperliquid testnet clearinghouseState")?;
        evaluate_official_venue_state(&open_orders, &clearinghouse)
    }

    fn engine_zero_observations(
        frame: &nautilus_sidecar::StreamFrame,
    ) -> (Option<bool>, Option<bool>) {
        let mut zero_positions = None;
        let mut zero_orders = None;
        for event in &frame.state {
            if let StreamEvent::PositionState { positions, .. } = event {
                zero_positions = Some(positions.is_empty());
            }
            if let StreamEvent::OrderState { orders, .. } = event {
                zero_orders = Some(!orders.iter().any(|order| {
                    find_string(order, &["status"]).is_some_and(|status| {
                        matches!(
                            status.to_ascii_uppercase().as_str(),
                            "INITIALIZED"
                                | "SUBMITTED"
                                | "ACCEPTED"
                                | "OPEN"
                                | "TRIGGERED"
                                | "PENDING_UPDATE"
                                | "PENDING_CANCEL"
                                | "PARTIALLY_FILLED"
                        )
                    })
                }));
            }
        }
        (zero_positions, zero_orders)
    }

    fn require_official_and_engine_zero(
        supervisor: &nautilus_sidecar::NautilusSupervisor,
        runtime: &GovernanceRuntime,
        owner_address: &str,
    ) -> Result<()> {
        let official_deadline = std::time::Instant::now() + Duration::from_secs(30);
        let official_before = loop {
            let state = official_testnet_state(owner_address)?;
            if state.is_zero_exposure() || std::time::Instant::now() >= official_deadline {
                break state;
            }
            std::thread::sleep(Duration::from_millis(250));
        };
        if !official_before.is_zero_exposure() {
            bail!(
                "official testnet safety gate refused: open_order_ids={:?} positions={:?}",
                official_before
                    .open_orders
                    .iter()
                    .map(|order| order.oid)
                    .collect::<Vec<_>>(),
                official_before.positions,
            );
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut zero_positions = None;
        let mut zero_orders = None;
        let mut last_positions = Vec::new();
        let mut last_live_orders = Vec::new();
        while std::time::Instant::now() < deadline {
            let frame = supervisor.take_stream_frame()?;
            runtime.ingest(&frame.state)?;
            for event in &frame.state {
                if let StreamEvent::PositionState { positions, .. } = event {
                    last_positions.clone_from(positions);
                }
                if let StreamEvent::OrderState { orders, .. } = event {
                    last_live_orders = orders
                        .iter()
                        .filter(|order| {
                            find_string(order, &["status"]).is_some_and(|status| {
                                matches!(
                                    status.to_ascii_uppercase().as_str(),
                                    "INITIALIZED"
                                        | "SUBMITTED"
                                        | "ACCEPTED"
                                        | "OPEN"
                                        | "TRIGGERED"
                                        | "PENDING_UPDATE"
                                        | "PENDING_CANCEL"
                                        | "PARTIALLY_FILLED"
                                )
                            })
                        })
                        .cloned()
                        .collect();
                }
            }
            let (positions, orders) = engine_zero_observations(&frame);
            if positions.is_some() {
                zero_positions = positions;
            }
            if orders.is_some() {
                zero_orders = orders;
            }
            if zero_positions == Some(true) && zero_orders == Some(true) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if zero_positions != Some(true) || zero_orders != Some(true) {
            bail!(
                "Nautilus safety gate did not report zero positions and zero orders: zero_positions={zero_positions:?} zero_orders={zero_orders:?} positions={last_positions:?} live_orders={last_live_orders:?}"
            );
        }
        let official_after = official_testnet_state(owner_address)?;
        if !official_after.is_zero_exposure() {
            bail!("official testnet state changed during the Nautilus safety gate");
        }
        println!(
            "cost_floor_safety_gate=official_open_orders:0 official_positions:0 nautilus_orders:0 nautilus_positions:0"
        );
        Ok(())
    }

    fn quote_price(quote: &StreamEvent, side: OrderSide, taker: bool) -> Result<String> {
        let StreamEvent::Quote {
            bid_price,
            ask_price,
            ..
        } = quote
        else {
            bail!("measurement price requires a quote event");
        };
        let (raw, numerator) = match (side, taker) {
            (OrderSide::Sell, true) => (bid_price, 100_i128),
            (OrderSide::Buy, true) => (ask_price, 100_i128),
            (OrderSide::Sell, false) => (ask_price, 100_i128),
            (OrderSide::Buy, false) => (bid_price, 100_i128),
        };
        let scaled = i128::from(decimal_to_units(raw, 8)?)
            .checked_mul(numerator)
            .context("measurement limit overflowed")?
            / 100;
        let tick_units = 10_000_000_i128;
        let ticked = match side {
            OrderSide::Sell => scaled / tick_units * tick_units,
            OrderSide::Buy => (scaled + tick_units - 1) / tick_units * tick_units,
        };
        format_decimal_units(ticked / tick_units, 1)
    }

    fn wait_for_measurement_fill(
        supervisor: &nautilus_sidecar::NautilusSupervisor,
        runtime: &GovernanceRuntime,
        client_order_id: &str,
        requested_quantity: &str,
        timeout: Duration,
    ) -> Result<(Option<StreamEvent>, Option<StreamEvent>)> {
        let deadline = std::time::Instant::now() + timeout;
        let mut latest_quote = None;
        let target_quantity_units = i128::from(decimal_to_units(requested_quantity, 8)?);
        let mut quantity_units = 0_i128;
        let mut quote_value_units = 0_i128;
        let mut fee_micros = 0_i128;
        let mut aggregate = None;
        while std::time::Instant::now() < deadline {
            let frame = supervisor.take_stream_frame()?;
            if frame.quote.is_some() {
                latest_quote = frame.quote;
            }
            let matching = frame.state.iter().filter(|event| {
                if let StreamEvent::Fill { state, .. } = event {
                    find_string(&Value::Object(state.clone()), &["client_order_id"]).as_deref()
                        == Some(client_order_id)
                } else {
                    false
                }
            });
            let fills = frame
                .state
                .iter()
                .filter(|event| matches!(event, StreamEvent::Fill { .. }))
                .cloned()
                .collect::<Vec<_>>();
            runtime.ingest(&fills)?;
            for fill in matching {
                let StreamEvent::Fill { state, .. } = fill else {
                    continue;
                };
                let state_value = Value::Object(state.clone());
                let fill_quantity = find_string(&state_value, &["last_qty", "quantity", "size"])
                    .context("measurement fill has no quantity")?;
                let fill_price = find_string(&state_value, &["last_px", "price"])
                    .context("measurement fill has no price")?;
                let commission = find_string(&state_value, &["commission", "fee"])
                    .unwrap_or_else(|| "0 USDC".into());
                let fill_quantity_units = i128::from(decimal_to_units(&fill_quantity, 8)?);
                quantity_units = quantity_units
                    .checked_add(fill_quantity_units)
                    .context("aggregate fill quantity overflowed")?;
                quote_value_units = quote_value_units
                    .checked_add(
                        fill_quantity_units
                            .checked_mul(i128::from(decimal_to_units(&fill_price, 8)?))
                            .context("aggregate fill value overflowed")?,
                    )
                    .context("aggregate fill value overflowed")?;
                fee_micros = fee_micros
                    .checked_add(i128::from(decimal_to_units(
                        commission
                            .split_whitespace()
                            .next()
                            .context("measurement commission is empty")?,
                        USDC_SCALE,
                    )?))
                    .context("aggregate fill commission overflowed")?;
                let mut combined = fill.clone();
                if let StreamEvent::Fill { state, .. } = &mut combined {
                    state.insert(
                        "last_qty".into(),
                        Value::String(format_decimal_units(quantity_units, 8)?),
                    );
                    state.insert(
                        "last_px".into(),
                        Value::String(format_decimal_units(quote_value_units / quantity_units, 8)?),
                    );
                    state.insert(
                        "commission".into(),
                        Value::String(format!(
                            "{} USDC",
                            format_decimal_units(fee_micros, USDC_SCALE)?
                        )),
                    );
                }
                aggregate = Some(combined);
            }
            if quantity_units >= target_quantity_units {
                return Ok((aggregate, latest_quote));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok((aggregate, latest_quote))
    }

    fn submit_measurement_order(
        supervisor: &mut nautilus_sidecar::NautilusSupervisor,
        client_order_id: String,
        side: OrderSide,
        quantity: &str,
        price: String,
        post_only: bool,
        reduce_only: bool,
    ) -> Result<()> {
        let receipt = supervisor.send_command(CommandRequest {
            command_id: format!("cost-floor-command-{}", unix_ms()?),
            command: NautilusCommand::PlaceOrder {
                client_order_id,
                instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
                side,
                quantity: quantity.into(),
                price,
                time_in_force: if post_only {
                    OrderTimeInForce::Gtc
                } else {
                    OrderTimeInForce::Ioc
                },
                post_only,
                reduce_only,
            },
        })?;
        if !matches!(
            receipt.outcome,
            nautilus_sidecar::CommandOutcome::OrderAccepted { .. }
        ) {
            bail!("measurement order was not accepted: {:?}", receipt.outcome);
        }
        Ok(())
    }

    #[test]
    fn cost_floor_uses_signed_mid_slippage_fees_and_a_stable_median() -> Result<()> {
        let entry = CostLegObservation::from_events(
            &cost_quote(1, "99", "101"),
            &cost_fill(2, "O-COST-ENTRY", "SELL", "MAKER", "100.1", "0.015"),
        )?;
        let mut exit = CostLegObservation::from_events(
            &cost_quote(3, "99", "101"),
            &cost_fill(4, "O-COST-EXIT", "BUY", "TAKER", "100.1", "0.045"),
        )?;
        exit.quantity = "1.00000000".into();
        assert!(entry.signed_adverse_slippage_micros_usd < 0);
        assert!(exit.signed_adverse_slippage_micros_usd > 0);
        let sample = CostFloorSample::from_legs(100, entry, exit)?;
        assert_eq!(sample.path, CostPath::MakerTaker);
        assert_eq!(sample.round_trip_cost_micros_bps, 6 * BASIS_POINT_MICROS);
        let summary = CostFloorSummary::stable(&vec![sample; 5], 3)?;
        assert_eq!(summary.sample_count, 5);
        assert_eq!(summary.admission_floor_bps, 9);
        assert_eq!(summary.schema, COST_FLOOR_SCHEMA);
        Ok(())
    }

    #[test]
    fn cost_floor_refuses_an_unstable_or_mixed_sample_set() -> Result<()> {
        let entry = CostLegObservation::from_events(
            &cost_quote(1, "99", "101"),
            &cost_fill(2, "O-COST-ENTRY", "BUY", "TAKER", "100.1", "0.045"),
        )?;
        let exit = CostLegObservation::from_events(
            &cost_quote(3, "99", "101"),
            &cost_fill(4, "O-COST-EXIT", "SELL", "TAKER", "99.9", "0.045"),
        )?;
        let sample = CostFloorSample::from_legs(100, entry, exit)?;
        assert!(CostFloorSummary::stable(&vec![sample.clone(); 4], 3).is_err());
        let mut mixed = vec![sample; 5];
        mixed[4].requested_notional_usd = 10;
        assert!(CostFloorSummary::stable(&mixed, 3).is_err());
        Ok(())
    }

    #[test]
    fn reconciled_filled_orders_enter_the_ledger_once() -> Result<()> {
        let runtime = GovernanceRuntime::in_memory(VenueCapabilityStore::default())?;
        let orders = vec![json!({
            "instrument_id":"BTC-USD-PERP.HYPERLIQUID",
            "client_order_id":"O-RECONCILED-1",
            "venue_order_id":"12345",
            "side":"BUY",
            "status":"FILLED",
            "filled_qty":"0.001",
            "avg_px":"65000",
            "liquidity_side":"MAKER",
            "commissions":["0.00975 USDC"]
        })];
        for sequence in [7, 8] {
            runtime.ingest(&[StreamEvent::OrderState {
                schema: "omega.nautilus.stream.v1".into(),
                generation: 1,
                sequence,
                network: nautilus_sidecar::Network::Testnet,
                venue: "HYPERLIQUID".into(),
                orders: orders.clone(),
                ts_init: sequence,
            }])?;
        }
        let entries = runtime.ledger.entries(&LedgerQuery::default())?;
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.kind == LedgerEntryKind::Fill)
                .count(),
            1
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.kind == LedgerEntryKind::Fee)
                .count(),
            1
        );
        Ok(())
    }

    #[test]
    fn balanced_fill_posts_both_assets_once() -> Result<()> {
        let runtime = GovernanceRuntime::in_memory(VenueCapabilityStore::default())?;
        let state = serde_json::from_value(json!({
            "instrument_id":"BTC-USDC-PERP.HYPERLIQUID",
            "order_side":"BUY",
            "last_qty":"0.001",
            "last_px":"60000"
        }))?;
        runtime.record_fill(1, 7, &state)?;
        let entries = runtime.ledger.entries(&LedgerQuery::default())?;
        let entry = entries.first().context("fill ledger entry missing")?;
        assert_eq!(entry.kind, LedgerEntryKind::Fill);
        assert_eq!(entry.postings.len(), 4);
        for asset in [AssetId::new("btc")?, AssetId::usdc()] {
            assert_eq!(
                entry
                    .postings
                    .iter()
                    .filter(|posting| posting.asset == asset)
                    .map(|posting| posting.amount)
                    .sum::<i64>(),
                0
            );
        }
        Ok(())
    }

    #[test]
    fn decimal_conversion_never_uses_floating_point() -> Result<()> {
        assert_eq!(decimal_to_units("0.001", 8)?, 100_000);
        assert_eq!(decimal_product_to_units("0.001", "60000", 6)?, 60_000_000);
        Ok(())
    }

    #[gpui::test]
    async fn order_lifecycle_update_keeps_one_typed_card(_cx: &mut gpui::TestAppContext) {
        let (stream, mut events) = ToolCallEventStream::test();
        emit_card_update(
            &stream,
            &json!({
                "schema":"omega.market.order-lifecycle.v1",
                "order_id":"O-OMEGA-302-1",
                "stage":"placed",
                "filled_units":0.0,
                "total_units":0.08,
            }),
            "Nautilus testnet order: placed",
        );
        let update = events.expect_update_fields().await;
        assert_eq!(
            update.title.as_deref(),
            Some("Nautilus testnet order: placed")
        );
        let content = update.content.expect("typed card content");
        assert_eq!(content.len(), 1);
        assert!(format!("{content:?}").contains("omega.market.order-lifecycle.v1"));
    }

    #[test]
    fn in_engine_strategy_breach_halts_and_wakes_governance() -> Result<()> {
        let runtime = GovernanceRuntime::in_memory(VenueCapabilityStore::default())?;
        let event = serde_json::from_value(json!({
            "type": "strategy_state",
            "schema": "omega.nautilus.stream.v1",
            "generation": 1,
            "sequence": 9,
            "network": "testnet",
            "strategy_id": STRATEGY_ID,
            "phase": "halted",
            "running": true,
            "halted_reason": "position_limit",
            "mandate_revision": 2,
            "quote_ticks": 40,
            "trade_ticks": 7,
            "book_ticks": 31,
            "action_count": 3,
            "active_client_order_id": null,
            "ts_init": 99
        }))?;
        runtime.ingest(&[event])?;
        let state = runtime.state.lock();
        assert_eq!(
            state.halted_reason.as_deref(),
            Some("Nautilus strategy halted: position_limit")
        );
        assert!(state.pending_wakeup.is_some());
        assert!(state.strategy.is_some());
        Ok(())
    }

    #[test]
    fn unknown_account_mode_halts_governance() -> Result<()> {
        let runtime = GovernanceRuntime::in_memory(VenueCapabilityStore::default())?;
        runtime.observe_account_modes(&json!({"account_type":"ALIEN","margin_mode":"cross"}))?;
        assert!(runtime.state.lock().halted_reason.is_some());
        Ok(())
    }

    #[test]
    fn a_new_account_snapshot_refreshes_capability_evidence() -> Result<()> {
        let capabilities = VenueCapabilityStore::default();
        let runtime = GovernanceRuntime::in_memory(capabilities.clone())?;
        let account = json!({"account_type":"MARGIN","margin_mode":"cross"});
        runtime.observe_account_modes_at(&account, 1_000)?;
        assert!(
            capabilities
                .report(VENUE, 31_001, CAPABILITY_MAX_AGE_MS)
                .verification
                .stale
        );

        runtime.observe_account_modes_at(&account, 30_000)?;
        let report = capabilities.report(VENUE, 31_001, CAPABILITY_MAX_AGE_MS);
        assert_eq!(
            report.verification.status,
            plugin_api::VenueCapabilityVerificationStatus::Verified
        );
        assert!(!report.verification.stale);
        Ok(())
    }

    #[test]
    fn reconciliation_gap_halts_and_queues_wakeup() -> Result<()> {
        let runtime = GovernanceRuntime::in_memory(VenueCapabilityStore::default())?;
        runtime.reconcile_account(1, 1, &json!({"account_value":"100"}))?;
        assert!(runtime.state.lock().halted_reason.is_none());
        runtime.reconcile_account(1, 2, &json!({"account_value":"99"}))?;
        let state = runtime.state.lock();
        assert!(state.halted_reason.is_some());
        assert!(state.pending_wakeup.is_some());
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit confirmation, testnet key, safe price, and public testnet access"]
    fn confirmed_testnet_order_is_mandated_and_ledger_recorded() -> Result<()> {
        if std::env::var("OMEGA_NAUTILUS_TEST_CONFIRMED").as_deref() != Ok("YES") {
            bail!(
                "set OMEGA_NAUTILUS_TEST_CONFIRMED=YES after explicitly confirming the test order"
            );
        }
        let private_key = std::env::var("HYPERLIQUID_TESTNET_PRIVATE_KEY")
            .context("HYPERLIQUID_TESTNET_PRIVATE_KEY must be configured")?;
        let price = std::env::var("OMEGA_NAUTILUS_TEST_PRICE")
            .context("OMEGA_NAUTILUS_TEST_PRICE must be configured")?;
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .context("repository root")?
            .to_path_buf();
        let config = nautilus_sidecar::NautilusConfig {
            network: nautilus_sidecar::Network::Testnet,
            python: repository_root.join("sidecar/nautilus/.venv/bin/python"),
            engine: repository_root.join("sidecar/nautilus/engine.py"),
            reconciliation_lookback_minutes: 60,
            health_timeout: Duration::from_secs(40),
        };
        let mut supervisor = nautilus_sidecar::NautilusSupervisor::new(
            config,
            nautilus_sidecar::PrivateKey::new(private_key.into_bytes())?,
        )?;
        supervisor.start()?;

        let runtime = GovernanceRuntime::in_memory(VenueCapabilityStore::default())?;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            let frame = supervisor.take_stream_frame()?;
            runtime.ingest(&frame.state)?;
            if runtime.state.lock().account.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let risk = runtime.risk_snapshot(false)?;
        let now = unix_ms()?;
        let mandate = TradingMandate {
            venue: VENUE.into(),
            network: TradingNetwork::Testnet,
            collateral_asset: AssetId::usdc(),
            objective: "Explicitly confirmed Hyperliquid testnet governance proof".into(),
            max_venue_balance: risk.venue_balance_micros.saturating_add(1_000_000_000),
            max_position_usd: risk.position_notional_usd.saturating_add(1_000),
            max_leverage: 2,
            daily_loss_stop: 100_000_000,
            max_orders_per_hour: 6,
            min_liquidation_buffer_bps: 1_000,
            allowed_strategies: BTreeSet::from([STRATEGY_ID.into()]),
            review_cadence: MandateReviewCadence::Interval { seconds: 3_600 },
            expires_at_ms: now.saturating_add(3_600_000),
        };
        let proposal = runtime.mandate.propose(mandate)?;
        trading_mandate::MandateStore::apply_ui_approved(&runtime.mandate, proposal, now)?;
        let prediction = runtime.predictions.append(PredictionEventDraft {
            schema_version: PREDICTION_SCHEMA_VERSION,
            emitted_at_ms: now,
            actor: PredictionActor::Agent {
                agent_id: "live-test".into(),
            },
            mandate_scope: MandateScope {
                venue: VENUE.into(),
                network: TradingNetwork::Testnet,
            },
            instrument: "BTC-USD-PERP.HYPERLIQUID".into(),
            forecast: PredictionForecast::Directional {
                direction: PredictedDirection::Flat,
                probability_micros: 500_000,
            },
            confidence_micros: 500_000,
            horizon_ms: 60_000,
            resolution_rule: ResolutionRule {
                source: "nautilus.testnet.quote.v1".into(),
                baseline_at_ms: now,
                resolve_at_ms: now.saturating_add(60_000),
                flat_tolerance_bps: 10,
            },
            scoring_rule: ScoringRule::Brier,
            observation_refs: vec!["live-account-read".into()],
            private_payload_ref: None,
            subsequent_decision_id: "live-order-288".into(),
        })?;
        runtime.require_prediction(&prediction.prediction_id, "live-order-288")?;
        let notional = decimal_product_to_units("0.001", &price, 0)?.unsigned_abs();
        let mandate_revision =
            runtime.authorize(VenueActionClass::OrderPlacement, notional, false)?;
        let nonce = u64::try_from(now).context("live test time is negative")?;
        let client_order_id = format!("O-288-{nonce}");
        let place = supervisor.send_command(CommandRequest {
            command_id: format!("testnet-place-288-{nonce}"),
            command: NautilusCommand::PlaceOrder {
                client_order_id: client_order_id.clone(),
                instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
                side: OrderSide::Buy,
                quantity: "0.001".into(),
                price,
                time_in_force: OrderTimeInForce::Gtc,
                post_only: true,
                reduce_only: false,
            },
        })?;
        runtime.record_receipt(&place, mandate_revision, Some(&prediction.prediction_id))?;
        if !matches!(
            place.outcome,
            nautilus_sidecar::CommandOutcome::OrderAccepted { .. }
        ) {
            bail!("testnet placement was not accepted: {:?}", place.outcome);
        }
        let cancel_revision = runtime.authorize(VenueActionClass::OrderCancellation, 0, true)?;
        let cancel = supervisor.send_command(CommandRequest {
            command_id: format!("testnet-cancel-288-{nonce}"),
            command: NautilusCommand::CancelOrder { client_order_id },
        })?;
        runtime.record_receipt(&cancel, cancel_revision, None)?;
        if !matches!(
            cancel.outcome,
            nautilus_sidecar::CommandOutcome::OrderCanceled { .. }
        ) {
            bail!(
                "testnet cancellation was not confirmed: {:?}",
                cancel.outcome
            );
        }
        let entries = runtime.ledger.entries(&LedgerQuery::default())?;
        assert!(
            entries
                .iter()
                .any(|entry| entry.kind == LedgerEntryKind::Order
                    && entry.metadata["mandate_revision"] == mandate_revision)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.kind == LedgerEntryKind::Cancel
                    && entry.metadata["mandate_revision"] == cancel_revision)
        );
        println!(
            "testnet governance evidence: account_read=true mandate_revision={mandate_revision} place_ack={} place_sent={} cancel_ack={} cancel_sent={} ledger_entries={}",
            place.acknowledged,
            place.sent,
            cancel.acknowledged,
            cancel.sent,
            entries.len(),
        );
        supervisor.stop()?;
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit confirmation, testnet key, and a safe reduce-only sell price"]
    fn confirmed_testnet_known_long_is_flattened_once_and_recorded() -> Result<()> {
        if std::env::var("OMEGA_NAUTILUS_TEST_CONFIRMED").as_deref() != Ok("YES") {
            bail!("set OMEGA_NAUTILUS_TEST_CONFIRMED=YES after confirming the risk reduction");
        }
        let private_key = std::env::var("HYPERLIQUID_TESTNET_PRIVATE_KEY")
            .context("HYPERLIQUID_TESTNET_PRIVATE_KEY must be configured")?;
        let price = std::env::var("OMEGA_NAUTILUS_TEST_PRICE")
            .context("OMEGA_NAUTILUS_TEST_PRICE must be configured")?;
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .context("repository root")?
            .to_path_buf();
        let config = nautilus_sidecar::NautilusConfig {
            network: nautilus_sidecar::Network::Testnet,
            python: repository_root.join("sidecar/nautilus/.venv/bin/python"),
            engine: repository_root.join("sidecar/nautilus/engine.py"),
            reconciliation_lookback_minutes: 60,
            health_timeout: Duration::from_secs(40),
        };
        let mut supervisor = nautilus_sidecar::NautilusSupervisor::new(
            config,
            nautilus_sidecar::PrivateKey::new(private_key.into_bytes())?,
        )?;
        supervisor.start()?;
        let client_order_id = format!("O-305-RISK-REDUCE-{}", unix_ms()?);
        let receipt = supervisor.send_command(CommandRequest {
            command_id: "testnet-risk-reduce-305".into(),
            command: NautilusCommand::PlaceOrder {
                client_order_id: client_order_id.clone(),
                instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
                side: OrderSide::Sell,
                quantity: "0.001".into(),
                price,
                time_in_force: OrderTimeInForce::Ioc,
                post_only: false,
                reduce_only: true,
            },
        })?;
        if !matches!(
            receipt.outcome,
            nautilus_sidecar::CommandOutcome::OrderAccepted { .. }
        ) {
            bail!("risk reduction was not accepted: {:?}", receipt.outcome);
        }
        let runtime = GovernanceRuntime::in_memory(VenueCapabilityStore::default())?;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut fill_key = None;
        while std::time::Instant::now() < deadline {
            let frame = supervisor.take_stream_frame()?;
            for event in &frame.state {
                if let StreamEvent::Fill {
                    generation,
                    sequence,
                    state,
                    ..
                } = event
                    && find_string(&Value::Object(state.clone()), &["client_order_id"]).as_deref()
                        == Some(client_order_id.as_str())
                {
                    fill_key = Some((*generation, *sequence));
                }
            }
            let fills = frame
                .state
                .iter()
                .filter(|event| matches!(event, StreamEvent::Fill { .. }))
                .cloned()
                .collect::<Vec<_>>();
            runtime.ingest(&fills)?;
            if fill_key.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let (generation, sequence) = fill_key.context("risk-reducing fill did not arrive")?;
        let entries = runtime.ledger.entries(&LedgerQuery::default())?;
        let entry = entries
            .iter()
            .find(|entry| {
                entry.kind == LedgerEntryKind::Fill
                    && entry.metadata["generation"] == generation
                    && entry.metadata["stream_sequence"] == sequence
            })
            .context("risk-reducing fill did not reach the ledger")?;
        println!(
            "testnet risk reduction evidence: generation={generation} stream_sequence={sequence} ledger_event_id={}",
            entry.event_id
        );
        supervisor.stop()?;
        Ok(())
    }

    #[test]
    #[ignore = "requires the testnet key and public read-only reconciliation access"]
    fn confirmed_testnet_flat_reconciliation_records_known_fills() -> Result<()> {
        let private_key = std::env::var("HYPERLIQUID_TESTNET_PRIVATE_KEY")
            .context("HYPERLIQUID_TESTNET_PRIVATE_KEY must be configured")?;
        let private_key = nautilus_sidecar::PrivateKey::new(private_key.into_bytes())?;
        let owner_address = private_key.ethereum_address()?;
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .context("repository root")?
            .to_path_buf();
        let config = nautilus_sidecar::NautilusConfig {
            network: nautilus_sidecar::Network::Testnet,
            python: repository_root.join("sidecar/nautilus/.venv/bin/python"),
            engine: repository_root.join("sidecar/nautilus/engine.py"),
            reconciliation_lookback_minutes: 60,
            health_timeout: Duration::from_secs(40),
        };
        let mut supervisor = nautilus_sidecar::NautilusSupervisor::new(config, private_key)?;
        supervisor.start()?;
        let runtime = GovernanceRuntime::in_memory(VenueCapabilityStore::default())?;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut flat_account = false;
        let mut open_orders = None;
        while std::time::Instant::now() < deadline {
            let frame = supervisor.take_stream_frame()?;
            for event in &frame.state {
                if let StreamEvent::PositionState { positions, .. } = event {
                    flat_account = positions.is_empty();
                }
                if let StreamEvent::Account { state, .. } = event {
                    flat_account = state
                        .get("margins")
                        .and_then(Value::as_array)
                        .is_some_and(Vec::is_empty);
                }
                if let StreamEvent::OrderState { orders, .. } = event {
                    open_orders = Some(
                        orders
                            .iter()
                            .filter(|order| {
                                find_string(order, &["status"]).is_some_and(|status| {
                                    matches!(
                                        status.to_ascii_uppercase().as_str(),
                                        "INITIALIZED"
                                            | "SUBMITTED"
                                            | "ACCEPTED"
                                            | "TRIGGERED"
                                            | "PENDING_UPDATE"
                                            | "PENDING_CANCEL"
                                            | "PARTIALLY_FILLED"
                                    )
                                })
                            })
                            .filter_map(|order| find_string(order, &["client_order_id"]))
                            .collect::<Vec<_>>(),
                    );
                }
            }
            runtime.ingest(&frame.state)?;
            if flat_account
                && open_orders.as_ref().is_some_and(Vec::is_empty)
                && runtime
                    .ledger
                    .entries(&LedgerQuery::default())?
                    .iter()
                    .filter(|entry| entry.kind == LedgerEntryKind::Fill)
                    .count()
                    >= 2
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let entries = runtime.ledger.entries(&LedgerQuery::default())?;
        let fills = entries
            .iter()
            .filter(|entry| entry.kind == LedgerEntryKind::Fill)
            .collect::<Vec<_>>();
        if !flat_account {
            bail!("testnet reconciliation still reports an open position");
        }
        let open_orders =
            open_orders.context("testnet reconciliation did not report order state")?;
        if !open_orders.is_empty() {
            bail!("testnet reconciliation reports open orders: {open_orders:?}");
        }
        if fills.len() < 2 {
            bail!("testnet reconciliation did not record both known fills");
        }
        let official = official_testnet_state(&owner_address)?;
        if !official.is_zero_exposure() {
            bail!("official testnet state is not flat after reconciliation: {official:?}");
        }
        let known_venue_order_ids = ["57672332979", "57672471851"];
        let known_fill_entries = known_venue_order_ids
            .iter()
            .map(|venue_order_id| {
                fills
                    .iter()
                    .find(|entry| {
                        entry.metadata["fill"]["venue_order_id"].as_str() == Some(*venue_order_id)
                    })
                    .map(|entry| (*venue_order_id, entry.event_id.clone()))
                    .with_context(|| {
                        format!("known venue fill {venue_order_id} did not reach the ledger")
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        println!(
            "testnet flat reconciliation evidence: official_positions=0 official_open_orders=0 nautilus_positions=0 nautilus_open_orders=0 known_fill_entries={known_fill_entries:?} fill_entries={} fee_entries={}",
            fills.len(),
            entries
                .iter()
                .filter(|entry| entry.kind == LedgerEntryKind::Fee)
                .count()
        );
        supervisor.stop()?;
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit testnet-only cost-floor confirmation, key, and public venue access"]
    fn confirmed_testnet_nautilus_cost_floor_samples_are_stable_and_ledger_backed() -> Result<()> {
        if std::env::var("OMEGA_NAUTILUS_COST_FLOOR_CONFIRM").as_deref()
            != Ok("execute-testnet-only")
        {
            bail!(
                "set OMEGA_NAUTILUS_COST_FLOOR_CONFIRM=execute-testnet-only to authorize bounded testnet samples"
            );
        }
        let private_key = std::env::var("HYPERLIQUID_TESTNET_PRIVATE_KEY")
            .context("HYPERLIQUID_TESTNET_PRIVATE_KEY must be configured")?;
        let clip = std::env::var("OMEGA_NAUTILUS_COST_CLIP")
            .context("OMEGA_NAUTILUS_COST_CLIP must be 65, 325, or 650")?;
        let (requested_notional_usd, quantity) = match clip.as_str() {
            "65" => (65_u64, "0.001"),
            "325" => (325, "0.005"),
            "650" => (650, "0.01"),
            _ => bail!("OMEGA_NAUTILUS_COST_CLIP must be 65, 325, or 650"),
        };
        let path = match std::env::var("OMEGA_NAUTILUS_COST_PATH").as_deref() {
            Ok("taker_taker") => CostPath::TakerTaker,
            Ok("maker_taker") => CostPath::MakerTaker,
            _ => bail!("OMEGA_NAUTILUS_COST_PATH must be taker_taker or maker_taker"),
        };
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .context("repository root")?
            .to_path_buf();
        let config = nautilus_sidecar::NautilusConfig {
            network: nautilus_sidecar::Network::Testnet,
            python: repository_root.join("sidecar/nautilus/.venv/bin/python"),
            engine: repository_root.join("sidecar/nautilus/engine.py"),
            reconciliation_lookback_minutes: 60,
            health_timeout: Duration::from_secs(40),
        };
        let private_key = nautilus_sidecar::PrivateKey::new(private_key.into_bytes())?;
        let owner_address = private_key.ethereum_address()?;
        let mut supervisor = nautilus_sidecar::NautilusSupervisor::new(config, private_key)?;
        supervisor.start()?;
        let runtime = GovernanceRuntime::in_memory(VenueCapabilityStore::default())?;
        let mut incomplete_maker_attempts = Vec::new();
        let mut samples = Vec::new();
        let max_attempts = if path == CostPath::MakerTaker { 20 } else { 10 };
        for attempt in 1..=max_attempts {
            if samples.len() == 5 {
                break;
            }
            require_official_and_engine_zero(&supervisor, &runtime, &owner_address)?;
            let entry_quote = next_testnet_quote(&supervisor, Duration::from_secs(60))?;
            let entry_id = format!(
                "O-305-{}-{requested_notional_usd}-{attempt}-{}",
                match path {
                    CostPath::MakerTaker => "MAKER",
                    CostPath::TakerTaker => "TAKER",
                },
                unix_ms()?
            );
            submit_measurement_order(
                &mut supervisor,
                entry_id.clone(),
                OrderSide::Buy,
                quantity,
                quote_price(&entry_quote, OrderSide::Buy, path == CostPath::TakerTaker)?,
                path == CostPath::MakerTaker,
                false,
            )?;
            let (entry_fill, latest_quote) = wait_for_measurement_fill(
                &supervisor,
                &runtime,
                &entry_id,
                quantity,
                if path == CostPath::MakerTaker {
                    Duration::from_secs(45)
                } else {
                    Duration::from_secs(20)
                },
            )?;
            let Some(entry_fill) = entry_fill else {
                if path == CostPath::MakerTaker {
                    let cancel = supervisor.send_command(CommandRequest {
                        command_id: format!("cost-floor-cancel-{}", unix_ms()?),
                        command: NautilusCommand::CancelOrder {
                            client_order_id: entry_id,
                        },
                    })?;
                    if !matches!(
                        cancel.outcome,
                        nautilus_sidecar::CommandOutcome::OrderCanceled { .. }
                    ) {
                        bail!(
                            "unfilled maker cancel was not confirmed: {:?}",
                            cancel.outcome
                        );
                    }
                }
                incomplete_maker_attempts.push((requested_notional_usd, attempt));
                require_official_and_engine_zero(&supervisor, &runtime, &owner_address)?;
                continue;
            };
            let observed_quantity = if let StreamEvent::Fill { state, .. } = &entry_fill {
                find_string(&Value::Object(state.clone()), &["last_qty"])
                    .context("entry fill has no quantity")?
            } else {
                bail!("entry fill has the wrong event type");
            };
            let complete_size =
                decimal_to_units(&observed_quantity, 8)? == decimal_to_units(quantity, 8)?;
            if !complete_size && path == CostPath::MakerTaker {
                let cancel = supervisor.send_command(CommandRequest {
                    command_id: format!("cost-floor-partial-cancel-{}", unix_ms()?),
                    command: NautilusCommand::CancelOrder {
                        client_order_id: entry_id,
                    },
                })?;
                if !matches!(
                    cancel.outcome,
                    nautilus_sidecar::CommandOutcome::OrderCanceled { .. }
                ) {
                    bail!(
                        "partial maker cancel was not confirmed: {:?}",
                        cancel.outcome
                    );
                }
                incomplete_maker_attempts.push((requested_notional_usd, attempt));
            }
            let exit_quote = if let Some(quote) = latest_quote {
                quote
            } else {
                next_testnet_quote(&supervisor, Duration::from_secs(60))?
            };
            let exit_id = format!(
                "O-305-EXIT-{requested_notional_usd}-{attempt}-{}",
                unix_ms()?
            );
            submit_measurement_order(
                &mut supervisor,
                exit_id.clone(),
                OrderSide::Sell,
                &observed_quantity,
                quote_price(&exit_quote, OrderSide::Sell, true)?,
                false,
                true,
            )?;
            let (exit_fill, _) = wait_for_measurement_fill(
                &supervisor,
                &runtime,
                &exit_id,
                &observed_quantity,
                Duration::from_secs(20),
            )?;
            let exit_fill = exit_fill.context("reduce-only testnet exit fill did not arrive")?;
            require_official_and_engine_zero(&supervisor, &runtime, &owner_address)?;
            if complete_size {
                let sample = CostFloorSample::from_legs(
                    requested_notional_usd,
                    CostLegObservation::from_events(&entry_quote, &entry_fill)?,
                    CostLegObservation::from_events(&exit_quote, &exit_fill)?,
                )?;
                println!("cost_floor_sample={}", serde_json::to_string(&sample)?);
                samples.push(sample);
            }
        }
        let summary = CostFloorSummary::stable(&samples, 3)?;
        let entries = runtime.ledger.entries(&LedgerQuery::default())?;
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema":COST_FLOOR_SCHEMA,
                "network":"testnet",
                "summary":summary,
                "incomplete_maker_attempts":incomplete_maker_attempts,
                "ledger_fill_entries":entries.iter().filter(|entry| entry.kind == LedgerEntryKind::Fill).count(),
                "ending_posture":"flat_after_every_completed_sample",
                "mainnet":"hard_refused"
            }))?
        );
        supervisor.stop()?;
        Ok(())
    }

    #[test]
    #[ignore = "requires the testnet key and public read-only venue access"]
    fn confirmed_testnet_cost_floor_safety_gate_is_zero() -> Result<()> {
        let private_key = std::env::var("HYPERLIQUID_TESTNET_PRIVATE_KEY")
            .context("HYPERLIQUID_TESTNET_PRIVATE_KEY must be configured")?;
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .context("repository root")?
            .to_path_buf();
        let config = nautilus_sidecar::NautilusConfig {
            network: nautilus_sidecar::Network::Testnet,
            python: repository_root.join("sidecar/nautilus/.venv/bin/python"),
            engine: repository_root.join("sidecar/nautilus/engine.py"),
            reconciliation_lookback_minutes: 60,
            health_timeout: Duration::from_secs(40),
        };
        let private_key = nautilus_sidecar::PrivateKey::new(private_key.into_bytes())?;
        let owner_address = private_key.ethereum_address()?;
        let mut supervisor = nautilus_sidecar::NautilusSupervisor::new(config, private_key)?;
        supervisor.start()?;
        let runtime = GovernanceRuntime::in_memory(VenueCapabilityStore::default())?;
        require_official_and_engine_zero(&supervisor, &runtime, &owner_address)?;
        supervisor.stop()?;
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit authorization to close the observed testnet-only 0.0002 BTC exposure"]
    fn confirmed_testnet_cost_floor_closes_exact_observed_exposure_once() -> Result<()> {
        if std::env::var("OMEGA_NAUTILUS_COST_CLEANUP_CONFIRM").as_deref()
            != Ok("close-exact-observed-0.0002")
        {
            bail!(
                "set OMEGA_NAUTILUS_COST_CLEANUP_CONFIRM=close-exact-observed-0.0002 after explicit authorization"
            );
        }
        let private_key = std::env::var("HYPERLIQUID_TESTNET_PRIVATE_KEY")
            .context("HYPERLIQUID_TESTNET_PRIVATE_KEY must be configured")?;
        let private_key = nautilus_sidecar::PrivateKey::new(private_key.into_bytes())?;
        let owner_address = private_key.ethereum_address()?;
        let official_before = official_testnet_state(&owner_address)?;
        if !official_before.open_orders.is_empty()
            || official_before.positions
                != vec![nautilus_sidecar::OfficialPosition {
                    coin: "BTC".into(),
                    size: "0.0002".into(),
                }]
        {
            bail!(
                "cleanup refused: expected no open orders and exact BTC 0.0002 exposure, got open_order_ids={:?} positions={:?}",
                official_before
                    .open_orders
                    .iter()
                    .map(|order| order.oid)
                    .collect::<Vec<_>>(),
                official_before.positions,
            );
        }
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .context("repository root")?
            .to_path_buf();
        let config = nautilus_sidecar::NautilusConfig {
            network: nautilus_sidecar::Network::Testnet,
            python: repository_root.join("sidecar/nautilus/.venv/bin/python"),
            engine: repository_root.join("sidecar/nautilus/engine.py"),
            reconciliation_lookback_minutes: 60,
            health_timeout: Duration::from_secs(40),
        };
        let mut supervisor = nautilus_sidecar::NautilusSupervisor::new(config, private_key)?;
        supervisor.start()?;
        let runtime = GovernanceRuntime::in_memory(VenueCapabilityStore::default())?;
        let quote = next_testnet_quote(&supervisor, Duration::from_secs(60))?;
        let client_order_id = format!("O-305-EXACT-CLEANUP-{}", unix_ms()?);
        let receipt = supervisor.send_command(CommandRequest {
            command_id: "cost-floor-exact-cleanup-305".into(),
            command: NautilusCommand::PlaceOrder {
                client_order_id: client_order_id.clone(),
                instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
                side: OrderSide::Sell,
                quantity: "0.0002".into(),
                price: quote_price(&quote, OrderSide::Sell, true)?,
                time_in_force: OrderTimeInForce::Ioc,
                post_only: false,
                reduce_only: true,
            },
        })?;
        let deadline = std::time::Instant::now() + Duration::from_secs(45);
        let mut fill_key = None;
        let mut zero_positions = None;
        let mut zero_orders = None;
        let mut official_after = official_testnet_state(&owner_address)?;
        while std::time::Instant::now() < deadline {
            let frame = supervisor.take_stream_frame()?;
            runtime.ingest(&frame.state)?;
            for event in &frame.state {
                if let StreamEvent::Fill {
                    generation,
                    sequence,
                    state,
                    ..
                } = event
                    && find_string(&Value::Object(state.clone()), &["client_order_id"]).as_deref()
                        == Some(client_order_id.as_str())
                {
                    fill_key = Some((*generation, *sequence));
                }
            }
            let (positions, orders) = engine_zero_observations(&frame);
            if positions.is_some() {
                zero_positions = positions;
            }
            if orders.is_some() {
                zero_orders = orders;
            }
            official_after = official_testnet_state(&owner_address)?;
            if fill_key.is_some()
                && official_after.is_zero_exposure()
                && zero_positions == Some(true)
                && zero_orders == Some(true)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let (generation, sequence) = fill_key.context("exact cleanup fill did not arrive")?;
        if !official_after.is_zero_exposure() {
            bail!("official venue was not flat after exact cleanup: {official_after:?}");
        }
        if zero_positions != Some(true) || zero_orders != Some(true) {
            bail!("Nautilus was not flat with zero live orders after exact cleanup");
        }
        let entries = runtime.ledger.entries(&LedgerQuery::default())?;
        let entry = entries
            .iter()
            .find(|entry| {
                entry.kind == LedgerEntryKind::Fill
                    && entry.metadata["generation"] == generation
                    && entry.metadata["stream_sequence"] == sequence
            })
            .context("exact cleanup fill did not reach the ledger")?;
        println!(
            "cost_floor_cleanup=official_open_orders:0 official_positions:0 nautilus_orders:0 nautilus_positions:0 generation={generation} sequence={sequence} ledger_event_id={} receipt={:?}",
            entry.event_id, receipt.outcome
        );
        supervisor.stop()?;
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit confirmation, testnet key, safe flatten price, and a bounded live fill window"]
    fn confirmed_testnet_tick_strategy_fill_reaches_the_ledger() -> Result<()> {
        if std::env::var("OMEGA_NAUTILUS_TEST_CONFIRMED").as_deref() != Ok("YES") {
            bail!(
                "set OMEGA_NAUTILUS_TEST_CONFIRMED=YES after explicitly confirming the testnet strategy run"
            );
        }
        let private_key = std::env::var("HYPERLIQUID_TESTNET_PRIVATE_KEY")
            .context("HYPERLIQUID_TESTNET_PRIVATE_KEY must be configured")?;
        let flatten_price = std::env::var("OMEGA_NAUTILUS_TEST_PRICE")
            .context("OMEGA_NAUTILUS_TEST_PRICE must be configured")?;
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .context("repository root")?
            .to_path_buf();
        let config = nautilus_sidecar::NautilusConfig {
            network: nautilus_sidecar::Network::Testnet,
            python: repository_root.join("sidecar/nautilus/.venv/bin/python"),
            engine: repository_root.join("sidecar/nautilus/engine.py"),
            reconciliation_lookback_minutes: 60,
            health_timeout: Duration::from_secs(40),
        };
        let mut supervisor = nautilus_sidecar::NautilusSupervisor::new(
            config,
            nautilus_sidecar::PrivateKey::new(private_key.into_bytes())?,
        )?;
        supervisor.start()?;

        let runtime = GovernanceRuntime::in_memory(VenueCapabilityStore::default())?;
        let account_deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < account_deadline {
            let frame = supervisor.take_stream_frame()?;
            runtime.ingest(&frame.state)?;
            if runtime.state.lock().account.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let risk = runtime.risk_snapshot(false)?;
        let now = unix_ms()?;
        let mandate = TradingMandate {
            venue: VENUE.into(),
            network: TradingNetwork::Testnet,
            collateral_asset: AssetId::usdc(),
            objective: "Bounded in-engine BTC testnet quote and fill proof".into(),
            max_venue_balance: risk.venue_balance_micros.saturating_add(1_000_000_000),
            max_position_usd: risk.position_notional_usd.saturating_add(1_000),
            max_leverage: 2,
            daily_loss_stop: 100_000_000,
            max_orders_per_hour: 30,
            min_liquidation_buffer_bps: 1_000,
            allowed_strategies: BTreeSet::from([STRATEGY_ID.into()]),
            review_cadence: MandateReviewCadence::Interval { seconds: 3_600 },
            expires_at_ms: now.saturating_add(600_000),
        };
        let proposal = runtime.mandate.propose(mandate)?;
        let approved =
            trading_mandate::MandateStore::apply_ui_approved(&runtime.mandate, proposal, now)?;
        let parameters = runtime.bounded_strategy_parameters(approved.revision, 30_000, 0)?;
        let parameter_receipt = supervisor.send_command(CommandRequest {
            command_id: "testnet-strategy-parameters-290".into(),
            command: NautilusCommand::SetStrategyParameters {
                strategy_id: STRATEGY_ID.into(),
                parameters,
            },
        })?;
        if !matches!(
            parameter_receipt.outcome,
            nautilus_sidecar::CommandOutcome::StrategyParametersApplied { .. }
        ) {
            bail!(
                "testnet strategy parameters were not applied: {:?}",
                parameter_receipt.outcome
            );
        }
        let start_receipt = supervisor.send_command(CommandRequest {
            command_id: "testnet-strategy-start-290".into(),
            command: NautilusCommand::StartStrategy {
                strategy_id: STRATEGY_ID.into(),
            },
        })?;
        if !matches!(
            start_receipt.outcome,
            nautilus_sidecar::CommandOutcome::StrategyStarted { running: true }
        ) {
            bail!(
                "testnet strategy did not start: {:?}",
                start_receipt.outcome
            );
        }

        let fill_deadline = std::time::Instant::now() + Duration::from_secs(300);
        let mut strategy_fill = None;
        while std::time::Instant::now() < fill_deadline {
            let frame = supervisor.take_stream_frame()?;
            for event in &frame.state {
                if let StreamEvent::Fill {
                    generation,
                    sequence,
                    state,
                    ..
                } = event
                    && find_string(&Value::Object(state.clone()), &["client_order_id"])
                        .is_some_and(|id| id.starts_with("O-290-"))
                {
                    strategy_fill = Some((*generation, *sequence));
                }
            }
            runtime.ingest(&frame.state)?;
            if strategy_fill.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let stop_receipt = supervisor.send_command(CommandRequest {
            command_id: "testnet-strategy-stop-290".into(),
            command: NautilusCommand::StopStrategy {
                strategy_id: STRATEGY_ID.into(),
            },
        })?;
        if !matches!(
            stop_receipt.outcome,
            nautilus_sidecar::CommandOutcome::StrategyStopped { running: false }
        ) {
            bail!(
                "testnet strategy did not stop cleanly: {:?}",
                stop_receipt.outcome
            );
        }
        let (fill_generation, fill_sequence) =
            strategy_fill.context("bounded testnet strategy produced no fill in five minutes")?;
        let flatten_receipt = supervisor.send_command(CommandRequest {
            command_id: "testnet-strategy-flatten-290".into(),
            command: NautilusCommand::PlaceOrder {
                client_order_id: format!("O-305-FLATTEN-{}", unix_ms()?),
                instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
                side: OrderSide::Buy,
                quantity: "0.001".into(),
                price: flatten_price,
                time_in_force: OrderTimeInForce::Ioc,
                post_only: false,
                reduce_only: true,
            },
        })?;
        if !matches!(
            flatten_receipt.outcome,
            nautilus_sidecar::CommandOutcome::OrderAccepted { .. }
        ) {
            bail!(
                "testnet strategy flatten was not accepted: {:?}",
                flatten_receipt.outcome
            );
        }
        let flatten_deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut flatten_fill = None;
        while std::time::Instant::now() < flatten_deadline {
            let frame = supervisor.take_stream_frame()?;
            for event in &frame.state {
                if let StreamEvent::Fill {
                    generation,
                    sequence,
                    state,
                    ..
                } = event
                    && find_string(&Value::Object(state.clone()), &["client_order_id"])
                        .is_some_and(|id| id.starts_with("O-305-FLATTEN-"))
                {
                    flatten_fill = Some((*generation, *sequence));
                }
            }
            runtime.ingest(&frame.state)?;
            if flatten_fill.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let (flatten_generation, flatten_sequence) =
            flatten_fill.context("strategy fill flatten did not reach the stream")?;
        let entries = runtime.ledger.entries(&LedgerQuery::default())?;
        let fill_entry = entries
            .iter()
            .find(|entry| {
                entry.kind == LedgerEntryKind::Fill
                    && entry.metadata["generation"] == fill_generation
                    && entry.metadata["stream_sequence"] == fill_sequence
            })
            .context("strategy fill did not reach the trading ledger")?;
        let flatten_entry = entries
            .iter()
            .find(|entry| {
                entry.kind == LedgerEntryKind::Fill
                    && entry.metadata["generation"] == flatten_generation
                    && entry.metadata["stream_sequence"] == flatten_sequence
            })
            .context("strategy flatten fill did not reach the trading ledger")?;
        println!(
            "testnet strategy fill evidence: generation={fill_generation} stream_sequence={fill_sequence} ledger_entry_id={} postings={} flatten_ledger_entry_id={}",
            fill_entry.event_id,
            fill_entry.postings.len(),
            flatten_entry.event_id,
        );
        supervisor.stop()?;
        Ok(())
    }
}
