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
    OrderSide, StrategyParameters, StreamEvent,
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

const VENUE: &str = "hyperliquid";
const STRATEGY_ID: &str = "OMEGA-BOUNDED-QUOTE-001";
const CAPABILITY_MAX_AGE_MS: i64 = 30_000;
const USDC_SCALE: u32 = 6;
const BTC_SCALE: u32 = 8;
const GOVERNANCE_STATE_CAPACITY: usize = 2_048;

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
    halted_reason: Option<String>,
    #[serde(skip)]
    processed_event_keys: HashSet<String>,
    #[serde(skip)]
    processed_event_order: VecDeque<String>,
    #[serde(skip)]
    claimed_sessions: HashSet<String>,
    pending_wakeup: Option<String>,
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
                StreamEvent::OrderState { orders, .. } => {
                    self.state.lock().orders = bounded_latest(orders);
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
        let mut draft = LedgerEntryDraft::new(
            format!("nautilus-fill-{generation}-{sequence}"),
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
                    format!("nautilus-fee-{generation}-{sequence}"),
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
        registry.add_review_driver(std::rc::Rc::new(GovernanceReviewDriver {
            runtime: runtime.clone(),
        }));
        registry.add_extension(agent_tools_registration(runtime));
        for schema in [
            "omega.nautilus.governance.v1",
            "omega.nautilus.command_receipt.v1",
            "omega.nautilus.ledger.fill.v1",
        ] {
            registry.add_card_schema(CardSchemaRegistration {
                plugin_id: MANIFEST.id,
                schema,
            });
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
        _source: &plugin_api::WakeupSource,
        instruction: &str,
        _cx: &App,
    ) -> bool {
        let mut state = self.runtime.state.lock();
        if !state.claimed_sessions.contains(session_id) || state.pending_wakeup.is_none() {
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
        self.runtime.review_accounting.append(record).is_ok()
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

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AccountInput {
    Account,
    Positions,
    Exposure,
    Ledger,
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
        cx.spawn(async move |_cx| {
            refresh?;
            let _input = input
                .recv()
                .await
                .map_err(|error| ToolOutput::error(error.to_string()))?;
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
                json!({"schema":"omega.nautilus.prediction.v1","prediction":event}),
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
            let (command, action, extra_notional, prediction_id, emergency) = match input {
                OrderInput::Place { client_order_id, instrument_id, side, quantity, price, post_only, reduce_only, prediction_id, decision_id } => {
                    self.runtime.require_prediction(&prediction_id, &decision_id).map_err(|error| ToolOutput::error(error.to_string()))?;
                    let extra = if reduce_only { 0 } else { decimal_product_to_units(&quantity, &price, 0).map_err(|error| ToolOutput::error(error.to_string()))?.unsigned_abs() };
                    (NautilusCommand::PlaceOrder { client_order_id, instrument_id, side: side.into(), quantity, price, post_only, reduce_only }, VenueActionClass::OrderPlacement, extra, Some(prediction_id), reduce_only)
                }
                OrderInput::Cancel { client_order_id } => (NautilusCommand::CancelOrder { client_order_id }, VenueActionClass::OrderCancellation, 0, None, true),
            };
            let revision = self.runtime.authorize(action, extra_notional, emergency).map_err(|error| ToolOutput::error(error.to_string()))?;
            let receipt = send_once(self.channel.as_ref(), CommandRequest { command_id: self.runtime.next_command_id("order"), command }).await?;
            self.runtime.record_receipt(&receipt, revision, prediction_id.as_deref()).map_err(|error| ToolOutput::error(error.to_string()))?;
            Ok(ToolOutput::ok(json!({"schema":"omega.nautilus.order.v1","network":"testnet","mandate_revision":revision,"single_attempt":true,"receipt":receipt})))
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
                command: NautilusCommand::PlaceOrder { client_order_id, instrument_id, side: side.into(), quantity, price, post_only: false, reduce_only: true },
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
    use std::{collections::BTreeSet, path::PathBuf, time::Duration};
    use trading_mandate::TradingMandate;

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
    #[ignore = "requires explicit confirmation, testnet key, and a bounded live fill window"]
    fn confirmed_testnet_tick_strategy_fill_reaches_the_ledger() -> Result<()> {
        if std::env::var("OMEGA_NAUTILUS_TEST_CONFIRMED").as_deref() != Ok("YES") {
            bail!(
                "set OMEGA_NAUTILUS_TEST_CONFIRMED=YES after explicitly confirming the testnet strategy run"
            );
        }
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
        let entries = runtime.ledger.entries(&LedgerQuery::default())?;
        let fill_entry = entries
            .iter()
            .find(|entry| {
                entry.kind == LedgerEntryKind::Fill
                    && entry.metadata["generation"] == fill_generation
                    && entry.metadata["stream_sequence"] == fill_sequence
            })
            .context("strategy fill did not reach the trading ledger")?;
        println!(
            "testnet strategy fill evidence: generation={fill_generation} stream_sequence={fill_sequence} ledger_entry_id={} postings={}",
            fill_entry.event_id,
            fill_entry.postings.len(),
        );
        supervisor.stop()?;
        Ok(())
    }
}
