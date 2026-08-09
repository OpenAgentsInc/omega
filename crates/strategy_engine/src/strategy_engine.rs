use std::{collections::VecDeque, sync::Arc};

use agent_wakeup::WakeupSource;
use anyhow::{Context as _, Result, anyhow, bail};
use async_trait::async_trait;
use futures::{
    StreamExt as _,
    channel::{mpsc, oneshot},
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use trading_ledger::{LedgerEntryDraft, LedgerEntryKind, LedgerQuery, LedgerStore};
use trading_mandate::{
    MandateDecision, MandateRefusal, MandateStore, TradingInstruction, TradingNetwork,
};

mod backtest;

pub use backtest::{
    BACKTEST_SCHEMA, BacktestApproval, BacktestCostModel, BacktestExecutionModel, BacktestGate,
    BacktestOutcome, BacktestPolicy, BacktestReport, BacktestStore, BacktestTick,
    SimulatedSettlement, SimulatedTrade, parameter_digest, run_backtest,
};

const ONE_HOUR_MS: i64 = 60 * 60 * 1_000;
const ONE_DAY_MS: i64 = 24 * ONE_HOUR_MS;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderKind {
    Market,
    Limit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuantityUnit {
    Sats,
    UsdCents,
    UsdNotional,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OrderQuantity {
    pub amount: u64,
    pub unit: QuantityUnit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VenueProtection {
    pub stop_loss_price: f64,
    pub take_profit_price: Option<f64>,
}

impl VenueProtection {
    fn validate(&self) -> Result<()> {
        if !self.stop_loss_price.is_finite() || self.stop_loss_price <= 0.0 {
            bail!("venue stop-loss price must be a positive finite number");
        }
        if let Some(take_profit_price) = self.take_profit_price
            && (!take_profit_price.is_finite() || take_profit_price <= 0.0)
        {
            bail!("venue take-profit price must be a positive finite number");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderIntent {
    pub intent_id: String,
    pub instrument: String,
    pub side: OrderSide,
    pub kind: OrderKind,
    pub quantity: OrderQuantity,
    pub limit_price: Option<f64>,
    pub reduce_only: bool,
    pub protection: Option<VenueProtection>,
    pub metadata: Value,
}

impl OrderIntent {
    pub fn validate(&self) -> Result<()> {
        validate_identifier("order intent ID", &self.intent_id)?;
        validate_identifier("instrument", &self.instrument)?;
        if self.quantity.amount == 0 {
            bail!("order quantity must be greater than zero");
        }
        match (self.kind, self.limit_price) {
            (OrderKind::Market, Some(_)) => bail!("a market order must not carry a limit price"),
            (OrderKind::Limit, None) => bail!("a limit order requires a limit price"),
            (_, Some(price)) if !price.is_finite() || price <= 0.0 => {
                bail!("order limit price must be a positive finite number")
            }
            _ => {}
        }
        if let Some(protection) = &self.protection {
            protection.validate()?;
        }
        if !self.metadata.is_object() {
            bail!("order metadata must be a JSON object");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VenueRiskSnapshot {
    pub network: TradingNetwork,
    pub venue: String,
    pub venue_balance_after_sats: u64,
    pub position_notional_before_usd: u64,
    pub position_notional_after_usd: u64,
    pub leverage: u8,
    pub liquidation_buffer_bps: u32,
}

impl VenueRiskSnapshot {
    fn validate(&self) -> Result<()> {
        validate_identifier("venue", &self.venue)?;
        if !(1..=100).contains(&self.leverage) {
            bail!("venue leverage must be from 1 through 100");
        }
        if self.liquidation_buffer_bps > 10_000 {
            bail!("liquidation buffer must not exceed 10000 basis points");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StrategyTick<Features> {
    pub occurred_at_ms: i64,
    pub features: Features,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StrategyStep<State> {
    pub next_state: State,
    pub intents: Vec<OrderIntent>,
}

pub trait StrategyProgram: Send + Sync + 'static {
    type Config: Clone + Send + Sync + Serialize + DeserializeOwned + 'static;
    type State: Clone + Send + Sync + Serialize + DeserializeOwned + 'static;
    type Features: Clone + Send + Sync + 'static;

    fn strategy_id(&self) -> &'static str;
    fn strategy_version(&self) -> &'static str;
    fn validate_config(&self, config: &Self::Config) -> Result<()>;
    fn initial_state(&self, config: &Self::Config) -> Result<Self::State>;
    fn on_tick(
        &self,
        config: &Self::Config,
        state: &Self::State,
        tick: &StrategyTick<Self::Features>,
    ) -> Result<StrategyStep<Self::State>>;

    fn on_execution(
        &self,
        _config: &Self::Config,
        state: &Self::State,
        _intent: &OrderIntent,
        _execution: &VenueExecution,
    ) -> Result<Self::State> {
        Ok(state.clone())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VenueExecution {
    pub venue_order_id: String,
    pub ledger_entries: Vec<LedgerEntryDraft>,
}

#[async_trait]
pub trait VenueExecutor: Send + Sync + 'static {
    async fn preview(&self, intent: &OrderIntent) -> Result<VenueRiskSnapshot>;

    /// This method is called once for each admitted intent. Implementations
    /// must not retry an ambiguous mutation.
    async fn execute_once(&self, intent: &OrderIntent) -> Result<VenueExecution>;
}

pub trait MandateAuthority: Send + Sync + 'static {
    fn authorize(&self, instruction: &TradingInstruction, now_ms: i64) -> Result<MandateDecision>;
}

impl MandateAuthority for MandateStore {
    fn authorize(&self, instruction: &TradingInstruction, now_ms: i64) -> Result<MandateDecision> {
        MandateStore::authorize(self, instruction, now_ms)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StrategyHaltReason {
    Manual { reason: String },
    ProgramError { message: String },
    MandateError { message: String },
    RiskLimit { refusal: MandateRefusal },
    MissingVenueProtection { intent_id: String },
    VenueError { message: String },
    LedgerError { message: String },
}

impl StrategyHaltReason {
    fn summary(&self) -> String {
        match self {
            Self::Manual { reason } => reason.clone(),
            Self::ProgramError { message } => format!("strategy program failed: {message}"),
            Self::MandateError { message } => format!("mandate check failed: {message}"),
            Self::RiskLimit { refusal } => format!("mandate refused the next order: {refusal:?}"),
            Self::MissingVenueProtection { intent_id } => {
                format!("leveraged risk increase {intent_id} had no venue-side stop loss")
            }
            Self::VenueError { message } => format!("venue operation failed: {message}"),
            Self::LedgerError { message } => format!("ledger operation failed: {message}"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum StrategyStatus {
    Idle,
    Running {
        started_at_ms: i64,
    },
    Halted {
        halted_at_ms: i64,
        reason: StrategyHaltReason,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StrategyLifecycleEvent {
    Started {
        strategy_id: String,
        at_ms: i64,
    },
    TickProcessed {
        strategy_id: String,
        at_ms: i64,
        intent_count: usize,
    },
    OrderAuthorized {
        strategy_id: String,
        intent_id: String,
        mandate_revision: u64,
    },
    OrderSubmitted {
        strategy_id: String,
        intent_id: String,
        venue_order_id: String,
    },
    BacktestApproved {
        strategy_id: String,
        report_digest: String,
    },
    StateUpdated {
        strategy_id: String,
        at_ms: i64,
        state: Value,
    },
    LedgerEntryAppended {
        strategy_id: String,
        event_id: String,
        sequence: u64,
    },
    Halted {
        strategy_id: String,
        at_ms: i64,
        reason: StrategyHaltReason,
    },
}

pub trait LifecycleSink: Send + Sync + 'static {
    fn publish(&self, event: StrategyLifecycleEvent);
}

pub trait WakeupSink: Send + Sync + 'static {
    fn publish(&self, source: WakeupSource, instruction: String);
}

#[derive(Clone, Default)]
pub struct MemoryLifecycleSink {
    events: Arc<Mutex<Vec<StrategyLifecycleEvent>>>,
}

impl MemoryLifecycleSink {
    pub fn events(&self) -> Vec<StrategyLifecycleEvent> {
        self.events.lock().clone()
    }
}

impl LifecycleSink for MemoryLifecycleSink {
    fn publish(&self, event: StrategyLifecycleEvent) {
        self.events.lock().push(event);
    }
}

#[derive(Clone, Default)]
pub struct MemoryWakeupSink {
    wakeups: Arc<Mutex<VecDeque<(WakeupSource, String)>>>,
}

impl MemoryWakeupSink {
    pub fn wakeups(&self) -> Vec<(WakeupSource, String)> {
        self.wakeups.lock().iter().cloned().collect()
    }

    pub fn pending(&self) -> Option<(WakeupSource, String)> {
        self.wakeups.lock().front().cloned()
    }

    pub fn acknowledge(&self, source: &WakeupSource, instruction: &str) -> bool {
        let mut wakeups = self.wakeups.lock();
        if wakeups
            .front()
            .is_some_and(|pending| &pending.0 == source && pending.1 == instruction)
        {
            wakeups.pop_front();
            true
        } else {
            false
        }
    }
}

impl WakeupSink for MemoryWakeupSink {
    fn publish(&self, source: WakeupSource, instruction: String) {
        self.wakeups.lock().push_back((source, instruction));
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TickReport {
    pub intent_count: usize,
    pub submitted_count: usize,
}

pub struct StrategyEngine<Program, Executor>
where
    Program: StrategyProgram,
    Executor: VenueExecutor,
{
    program: Program,
    executor: Executor,
    network: TradingNetwork,
    mandate: Arc<dyn MandateAuthority>,
    backtests: Arc<dyn BacktestGate>,
    ledger: LedgerStore,
    lifecycle: Arc<dyn LifecycleSink>,
    wakeups: Arc<dyn WakeupSink>,
    status: StrategyStatus,
    config: Option<Program::Config>,
    state: Option<Program::State>,
}

impl<Program, Executor> StrategyEngine<Program, Executor>
where
    Program: StrategyProgram,
    Executor: VenueExecutor,
{
    pub fn new(
        program: Program,
        executor: Executor,
        network: TradingNetwork,
        mandate: impl MandateAuthority,
        backtests: impl BacktestGate,
        ledger: LedgerStore,
        lifecycle: Arc<dyn LifecycleSink>,
        wakeups: Arc<dyn WakeupSink>,
    ) -> Self {
        Self {
            program,
            executor,
            network,
            mandate: Arc::new(mandate),
            backtests: Arc::new(backtests),
            ledger,
            lifecycle,
            wakeups,
            status: StrategyStatus::Idle,
            config: None,
            state: None,
        }
    }

    pub fn status(&self) -> &StrategyStatus {
        &self.status
    }

    pub fn state(&self) -> Option<&Program::State> {
        self.state.as_ref()
    }

    pub fn start(&mut self, config: Program::Config, at_ms: i64) -> Result<()> {
        validate_timestamp(at_ms)?;
        if matches!(self.status, StrategyStatus::Running { .. }) {
            bail!("strategy is already running");
        }
        self.program.validate_config(&config)?;
        let approval = self.require_backtest(&config)?;
        let state = self.program.initial_state(&config)?;
        let state_value = serde_json::to_value(&state)?;
        self.config = Some(config);
        self.state = Some(state);
        self.status = StrategyStatus::Running {
            started_at_ms: at_ms,
        };
        self.lifecycle
            .publish(StrategyLifecycleEvent::BacktestApproved {
                strategy_id: self.program.strategy_id().to_string(),
                report_digest: approval.report_digest,
            });
        self.lifecycle.publish(StrategyLifecycleEvent::Started {
            strategy_id: self.program.strategy_id().to_string(),
            at_ms,
        });
        self.lifecycle
            .publish(StrategyLifecycleEvent::StateUpdated {
                strategy_id: self.program.strategy_id().to_string(),
                at_ms,
                state: state_value,
            });
        Ok(())
    }

    pub fn adjust(&mut self, config: Program::Config) -> Result<()> {
        if !matches!(self.status, StrategyStatus::Running { .. }) {
            bail!("strategy must be running before its configuration can be adjusted");
        }
        self.program.validate_config(&config)?;
        let approval = self.require_backtest(&config)?;
        self.config = Some(config);
        self.lifecycle
            .publish(StrategyLifecycleEvent::BacktestApproved {
                strategy_id: self.program.strategy_id().to_string(),
                report_digest: approval.report_digest,
            });
        Ok(())
    }

    pub fn halt(&mut self, at_ms: i64, reason: StrategyHaltReason) -> Result<()> {
        validate_timestamp(at_ms)?;
        self.halt_inner(at_ms, reason);
        Ok(())
    }

    pub async fn handle_tick(
        &mut self,
        tick: StrategyTick<Program::Features>,
    ) -> Result<TickReport> {
        validate_timestamp(tick.occurred_at_ms)?;
        if !matches!(self.status, StrategyStatus::Running { .. }) {
            bail!("strategy is not running");
        }
        let config = self
            .config
            .as_ref()
            .context("running strategy has no configuration")?
            .clone();
        let state = self
            .state
            .as_ref()
            .context("running strategy has no state")?;
        let step = match self.program.on_tick(&config, state, &tick) {
            Ok(step) => step,
            Err(error) => {
                let reason = StrategyHaltReason::ProgramError {
                    message: format!("{error:#}"),
                };
                self.halt_inner(tick.occurred_at_ms, reason.clone());
                return Err(anyhow!(reason.summary()));
            }
        };
        for intent in &step.intents {
            if let Err(error) = intent.validate() {
                let reason = StrategyHaltReason::ProgramError {
                    message: format!("invalid order intent: {error:#}"),
                };
                self.halt_inner(tick.occurred_at_ms, reason.clone());
                return Err(anyhow!(reason.summary()));
            }
        }

        let mut submitted_count = 0;
        let mut next_state = step.next_state;
        for intent in &step.intents {
            let execution = self.execute_intent(intent, tick.occurred_at_ms).await?;
            next_state = match self
                .program
                .on_execution(&config, &next_state, intent, &execution)
            {
                Ok(next_state) => next_state,
                Err(error) => {
                    let reason = StrategyHaltReason::ProgramError {
                        message: format!("could not reconcile venue execution: {error:#}"),
                    };
                    self.halt_inner(tick.occurred_at_ms, reason.clone());
                    return Err(anyhow!(reason.summary()));
                }
            };
            submitted_count += 1;
        }
        let state_value = serde_json::to_value(&next_state)?;
        self.state = Some(next_state);
        self.lifecycle
            .publish(StrategyLifecycleEvent::StateUpdated {
                strategy_id: self.program.strategy_id().to_string(),
                at_ms: tick.occurred_at_ms,
                state: state_value,
            });
        self.lifecycle
            .publish(StrategyLifecycleEvent::TickProcessed {
                strategy_id: self.program.strategy_id().to_string(),
                at_ms: tick.occurred_at_ms,
                intent_count: step.intents.len(),
            });
        Ok(TickReport {
            intent_count: step.intents.len(),
            submitted_count,
        })
    }

    async fn execute_intent(
        &mut self,
        intent: &OrderIntent,
        now_ms: i64,
    ) -> Result<VenueExecution> {
        let preview = match self.executor.preview(intent).await {
            Ok(preview) => preview,
            Err(error) => {
                return self.fail(
                    now_ms,
                    StrategyHaltReason::VenueError {
                        message: format!("risk preview failed: {error:#}"),
                    },
                );
            }
        };
        if let Err(error) = preview.validate() {
            return self.fail(
                now_ms,
                StrategyHaltReason::VenueError {
                    message: format!("invalid risk preview: {error:#}"),
                },
            );
        }
        if preview.network != self.network {
            return self.fail(
                now_ms,
                StrategyHaltReason::VenueError {
                    message: format!(
                        "risk preview used {:?} while the strategy runs on {:?}",
                        preview.network, self.network
                    ),
                },
            );
        }
        if preview.leverage > 1
            && preview.position_notional_after_usd > preview.position_notional_before_usd
            && intent.protection.is_none()
        {
            return self.fail(
                now_ms,
                StrategyHaltReason::MissingVenueProtection {
                    intent_id: intent.intent_id.clone(),
                },
            );
        }

        let (order_count_last_hour, daily_realized_loss_sats) =
            match self.ledger_risk_totals(now_ms) {
                Ok(totals) => totals,
                Err(error) => {
                    return self.fail(
                        now_ms,
                        StrategyHaltReason::LedgerError {
                            message: format!("could not calculate risk totals: {error:#}"),
                        },
                    );
                }
            };
        let instruction = TradingInstruction {
            network: preview.network,
            strategy_id: self.program.strategy_id().to_string(),
            venue_balance_after_sats: preview.venue_balance_after_sats,
            position_notional_usd: preview.position_notional_after_usd,
            leverage: preview.leverage,
            daily_realized_loss_sats,
            orders_last_hour: order_count_last_hour,
            liquidation_buffer_bps: preview.liquidation_buffer_bps,
        };
        let decision = match self.mandate.authorize(&instruction, now_ms) {
            Ok(decision) => decision,
            Err(error) => {
                return self.fail(
                    now_ms,
                    StrategyHaltReason::MandateError {
                        message: format!("could not authorize order: {error:#}"),
                    },
                );
            }
        };
        let mandate_revision = match decision {
            MandateDecision::Authorized { revision } => revision,
            MandateDecision::Refused { reason, .. } => {
                return self.fail(now_ms, StrategyHaltReason::RiskLimit { refusal: reason });
            }
        };
        self.lifecycle
            .publish(StrategyLifecycleEvent::OrderAuthorized {
                strategy_id: self.program.strategy_id().to_string(),
                intent_id: intent.intent_id.clone(),
                mandate_revision,
            });

        let order_entry = LedgerEntryDraft {
            event_id: format!("strategy-order:{}", intent.intent_id),
            occurred_at_ms: now_ms,
            strategy_id: self.program.strategy_id().to_string(),
            kind: LedgerEntryKind::Order,
            postings: Vec::new(),
            metadata: json!({
                "intent": intent,
                "venue": preview.venue,
                "mandate_revision": mandate_revision,
            }),
        };
        if let Err(error) = self.append_ledger(order_entry) {
            return self.fail(
                now_ms,
                StrategyHaltReason::LedgerError {
                    message: format!("could not record authorized order: {error:#}"),
                },
            );
        }

        let execution = match self.executor.execute_once(intent).await {
            Ok(execution) => execution,
            Err(error) => {
                return self.fail(
                    now_ms,
                    StrategyHaltReason::VenueError {
                        message: format!("single-attempt order failed: {error:#}"),
                    },
                );
            }
        };
        if let Err(error) = validate_identifier("venue order ID", &execution.venue_order_id) {
            return self.fail(
                now_ms,
                StrategyHaltReason::VenueError {
                    message: format!("invalid execution result: {error:#}"),
                },
            );
        }
        for entry in &execution.ledger_entries {
            if entry.strategy_id != self.program.strategy_id() {
                return self.fail(
                    now_ms,
                    StrategyHaltReason::LedgerError {
                        message: format!(
                            "venue ledger event {} names strategy {} instead of {}",
                            entry.event_id,
                            entry.strategy_id,
                            self.program.strategy_id()
                        ),
                    },
                );
            }
            if matches!(entry.kind, LedgerEntryKind::Order) {
                return self.fail(
                    now_ms,
                    StrategyHaltReason::LedgerError {
                        message: format!(
                            "venue ledger event {} duplicated the engine-owned order event",
                            entry.event_id
                        ),
                    },
                );
            }
            if let Err(error) = self.append_ledger(entry.clone()) {
                return self.fail(
                    now_ms,
                    StrategyHaltReason::LedgerError {
                        message: format!("could not record venue execution: {error:#}"),
                    },
                );
            }
        }
        self.lifecycle
            .publish(StrategyLifecycleEvent::OrderSubmitted {
                strategy_id: self.program.strategy_id().to_string(),
                intent_id: intent.intent_id.clone(),
                venue_order_id: execution.venue_order_id.clone(),
            });
        Ok(execution)
    }

    fn ledger_risk_totals(&self, now_ms: i64) -> Result<(u32, u64)> {
        let strategy_id = self.program.strategy_id().to_string();
        let hourly_entries = self.ledger.entries(&LedgerQuery {
            from_ms: Some(now_ms.saturating_sub(ONE_HOUR_MS).max(0)),
            to_ms: Some(now_ms),
            strategy_id: Some(strategy_id.clone()),
        })?;
        let order_count = hourly_entries
            .iter()
            .filter(|entry| matches!(entry.kind, LedgerEntryKind::Order))
            .count();
        let order_count = u32::try_from(order_count).unwrap_or(u32::MAX);
        let report = self.ledger.profit_report(&LedgerQuery {
            from_ms: Some(now_ms.saturating_sub(ONE_DAY_MS).max(0)),
            to_ms: Some(now_ms),
            strategy_id: Some(strategy_id),
        })?;
        let daily_realized_loss_sats = if report.total_profit_sats < 0 {
            report
                .total_profit_sats
                .checked_neg()
                .and_then(|loss| u64::try_from(loss).ok())
                .unwrap_or(u64::MAX)
        } else {
            0
        };
        Ok((order_count, daily_realized_loss_sats))
    }

    fn append_ledger(&self, draft: LedgerEntryDraft) -> Result<()> {
        let entry = self.ledger.append(draft)?;
        self.lifecycle
            .publish(StrategyLifecycleEvent::LedgerEntryAppended {
                strategy_id: self.program.strategy_id().to_string(),
                event_id: entry.event_id,
                sequence: entry.sequence,
            });
        Ok(())
    }

    fn require_backtest(&self, config: &Program::Config) -> Result<BacktestApproval> {
        let config = serde_json::to_value(config)?;
        self.backtests.require_passing(
            self.program.strategy_id(),
            self.program.strategy_version(),
            self.network,
            &config,
        )
    }

    fn fail<T>(&mut self, at_ms: i64, reason: StrategyHaltReason) -> Result<T> {
        let message = reason.summary();
        self.halt_inner(at_ms, reason);
        Err(anyhow!(message))
    }

    fn halt_inner(&mut self, at_ms: i64, reason: StrategyHaltReason) {
        self.status = StrategyStatus::Halted {
            halted_at_ms: at_ms,
            reason: reason.clone(),
        };
        let strategy_id = self.program.strategy_id().to_string();
        self.lifecycle.publish(StrategyLifecycleEvent::Halted {
            strategy_id: strategy_id.clone(),
            at_ms,
            reason: reason.clone(),
        });
        let summary = reason.summary();
        self.wakeups.publish(
            WakeupSource::StrategyHalt {
                strategy: strategy_id.clone(),
                reason: summary.clone(),
            },
            format!(
                "Strategy {strategy_id} halted: {summary}. Review the ledger and current risk before changing its state."
            ),
        );
    }
}

pub enum StrategyCommand<Config, Features> {
    Start { config: Config, at_ms: i64 },
    Adjust { config: Config },
    Tick(StrategyTick<Features>),
    Halt { at_ms: i64, reason: String },
    Shutdown,
}

#[derive(Clone)]
pub struct StrategyServiceHandle<Config, Features> {
    sender: mpsc::UnboundedSender<StrategyServiceMessage<Config, Features>>,
}

struct StrategyServiceMessage<Config, Features> {
    command: StrategyCommand<Config, Features>,
    response: Option<oneshot::Sender<Result<(), String>>>,
}

impl<Config, Features> StrategyServiceHandle<Config, Features> {
    pub fn send(&mut self, command: StrategyCommand<Config, Features>) -> Result<()> {
        self.sender
            .start_send(StrategyServiceMessage {
                command,
                response: None,
            })
            .map_err(|error| anyhow!("strategy service is unavailable: {error}"))
    }

    pub async fn request(&mut self, command: StrategyCommand<Config, Features>) -> Result<()> {
        let (sender, receiver) = oneshot::channel();
        self.sender
            .start_send(StrategyServiceMessage {
                command,
                response: Some(sender),
            })
            .map_err(|error| anyhow!("strategy service is unavailable: {error}"))?;
        receiver
            .await
            .map_err(|_| anyhow!("strategy service stopped before acknowledging the command"))?
            .map_err(anyhow::Error::msg)
    }
}

pub fn background_service<Program, Executor>(
    mut engine: StrategyEngine<Program, Executor>,
) -> (
    StrategyServiceHandle<Program::Config, Program::Features>,
    impl std::future::Future<Output = Result<StrategyEngine<Program, Executor>>>,
)
where
    Program: StrategyProgram,
    Executor: VenueExecutor,
{
    let (sender, mut receiver) = mpsc::unbounded();
    let handle = StrategyServiceHandle { sender };
    let service = async move {
        while let Some(message) = receiver.next().await {
            let should_shutdown = matches!(message.command, StrategyCommand::Shutdown);
            let result = match message.command {
                StrategyCommand::Start { config, at_ms } => engine.start(config, at_ms),
                StrategyCommand::Adjust { config } => engine.adjust(config),
                StrategyCommand::Tick(tick) => engine.handle_tick(tick).await.map(|_| ()),
                StrategyCommand::Halt { at_ms, reason } => {
                    engine.halt(at_ms, StrategyHaltReason::Manual { reason })
                }
                StrategyCommand::Shutdown => Ok(()),
            };
            if let Some(response) = message.response {
                let response_result = match &result {
                    Ok(()) => Ok(()),
                    Err(error) => Err(error.to_string()),
                };
                if response.send(response_result).is_err() {
                    log::debug!("strategy command requester stopped before receiving its response");
                }
            } else {
                result?;
            }
            if should_shutdown {
                break;
            }
        }
        Ok(engine)
    };
    (handle, service)
}

fn validate_identifier(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{label} must not be empty");
    }
    if value.len() > 200 {
        bail!("{label} must not exceed 200 bytes");
    }
    Ok(())
}

fn validate_timestamp(timestamp_ms: i64) -> Result<()> {
    if timestamp_ms < 0 {
        bail!("strategy timestamp must not be negative");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use futures::executor::block_on;
    use serde_json::json;
    use trading_ledger::{LedgerAccount, LedgerPosting};

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct TestConfig {
        protected: bool,
    }

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    struct TestState {
        ticks: u64,
        executions: u64,
    }

    #[derive(Clone, Debug, PartialEq)]
    struct TestFeatures {
        should_trade: bool,
    }

    struct TestProgram;

    #[test]
    fn memory_wakeup_events_remain_pending_until_exact_acknowledgement() {
        let wakeups = MemoryWakeupSink::default();
        let first_source = WakeupSource::StrategyHalt {
            strategy: "funding_carry".into(),
            reason: "funding flipped".into(),
        };
        let second_source = WakeupSource::VolatilityRegimeChange {
            previous: "quiet".into(),
            current: "active".into(),
        };
        wakeups.publish(first_source.clone(), "review funding".into());
        wakeups.publish(second_source.clone(), "review volatility".into());

        assert_eq!(
            wakeups.pending(),
            Some((first_source.clone(), "review funding".into()))
        );
        assert!(!wakeups.acknowledge(&second_source, "review volatility"));
        assert_eq!(wakeups.wakeups().len(), 2);
        assert!(wakeups.acknowledge(&first_source, "review funding"));
        assert_eq!(
            wakeups.pending(),
            Some((second_source, "review volatility".into()))
        );
    }

    impl StrategyProgram for TestProgram {
        type Config = TestConfig;
        type State = TestState;
        type Features = TestFeatures;

        fn strategy_id(&self) -> &'static str {
            "test_strategy"
        }

        fn strategy_version(&self) -> &'static str {
            "1"
        }

        fn validate_config(&self, _config: &Self::Config) -> Result<()> {
            Ok(())
        }

        fn initial_state(&self, _config: &Self::Config) -> Result<Self::State> {
            Ok(TestState::default())
        }

        fn on_tick(
            &self,
            config: &Self::Config,
            state: &Self::State,
            tick: &StrategyTick<Self::Features>,
        ) -> Result<StrategyStep<Self::State>> {
            let mut next_state = state.clone();
            next_state.ticks = next_state
                .ticks
                .checked_add(1)
                .context("test state overflowed")?;
            let intents = if tick.features.should_trade {
                vec![OrderIntent {
                    intent_id: format!("intent-{}", tick.occurred_at_ms),
                    instrument: "btc_usd".into(),
                    side: OrderSide::Sell,
                    kind: OrderKind::Market,
                    quantity: OrderQuantity {
                        amount: 100,
                        unit: QuantityUnit::UsdNotional,
                    },
                    limit_price: None,
                    reduce_only: false,
                    protection: config.protected.then_some(VenueProtection {
                        stop_loss_price: 70_000.0,
                        take_profit_price: Some(60_000.0),
                    }),
                    metadata: json!({"source": "test"}),
                }]
            } else {
                Vec::new()
            };
            Ok(StrategyStep {
                next_state,
                intents,
            })
        }

        fn on_execution(
            &self,
            _config: &Self::Config,
            state: &Self::State,
            _intent: &OrderIntent,
            _execution: &VenueExecution,
        ) -> Result<Self::State> {
            let mut next_state = state.clone();
            next_state.executions = next_state
                .executions
                .checked_add(1)
                .context("test execution count overflowed")?;
            Ok(next_state)
        }
    }

    #[derive(Clone)]
    struct TestExecutor {
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl TestExecutor {
        fn successful() -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
                fail: false,
            }
        }

        fn ambiguous_failure() -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
                fail: true,
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl VenueExecutor for TestExecutor {
        async fn preview(&self, _intent: &OrderIntent) -> Result<VenueRiskSnapshot> {
            Ok(VenueRiskSnapshot {
                network: TradingNetwork::Signet,
                venue: "test_venue".into(),
                venue_balance_after_sats: 10_000,
                position_notional_before_usd: 0,
                position_notional_after_usd: 100,
                leverage: 2,
                liquidation_buffer_bps: 2_500,
            })
        }

        async fn execute_once(&self, intent: &OrderIntent) -> Result<VenueExecution> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                bail!("ambiguous venue failure");
            }
            let venue = LedgerAccount::VenueBalance {
                venue: "test_venue".into(),
            };
            let entry = |suffix: &str, kind: LedgerEntryKind, postings: Vec<LedgerPosting>| {
                LedgerEntryDraft {
                    event_id: format!("{suffix}-{}", intent.intent_id),
                    occurred_at_ms: 100,
                    strategy_id: "test_strategy".into(),
                    kind,
                    postings,
                    metadata: json!({"intent_id": intent.intent_id}),
                }
            };
            Ok(VenueExecution {
                venue_order_id: format!("venue-{}", intent.intent_id),
                ledger_entries: vec![
                    entry(
                        "fill",
                        LedgerEntryKind::Fill,
                        vec![
                            LedgerPosting {
                                account: venue.clone(),
                                amount_sats: 10,
                            },
                            LedgerPosting {
                                account: LedgerAccount::TradingProfit,
                                amount_sats: -10,
                            },
                        ],
                    ),
                    entry(
                        "fee",
                        LedgerEntryKind::Fee,
                        vec![
                            LedgerPosting {
                                account: venue.clone(),
                                amount_sats: -2,
                            },
                            LedgerPosting {
                                account: LedgerAccount::FeeExpense,
                                amount_sats: 2,
                            },
                        ],
                    ),
                    entry(
                        "funding",
                        LedgerEntryKind::FundingSettlement,
                        vec![
                            LedgerPosting {
                                account: venue,
                                amount_sats: 1,
                            },
                            LedgerPosting {
                                account: LedgerAccount::FundingIncome,
                                amount_sats: -1,
                            },
                        ],
                    ),
                ],
            })
        }
    }

    struct TestMandateAuthority {
        max_orders_per_hour: u32,
    }

    struct TestBacktestGate {
        passing: bool,
    }

    impl BacktestGate for TestBacktestGate {
        fn require_passing(
            &self,
            _strategy_id: &str,
            _strategy_version: &str,
            _network: TradingNetwork,
            _config: &Value,
        ) -> Result<BacktestApproval> {
            if !self.passing {
                bail!("no passing backtest artifact for this parameter set");
            }
            Ok(BacktestApproval {
                report_digest: "0".repeat(64),
                created_at_ms: 1,
            })
        }
    }

    impl MandateAuthority for TestMandateAuthority {
        fn authorize(
            &self,
            instruction: &TradingInstruction,
            _now_ms: i64,
        ) -> Result<MandateDecision> {
            if instruction.orders_last_hour >= self.max_orders_per_hour {
                Ok(MandateDecision::Refused {
                    reason: MandateRefusal::HourlyOrderLimit {
                        limit: self.max_orders_per_hour,
                        orders_last_hour: instruction.orders_last_hour,
                    },
                    required_posture: trading_mandate::RequiredRiskPosture::FlatRisk,
                })
            } else {
                Ok(MandateDecision::Authorized { revision: 1 })
            }
        }
    }

    fn tick(at_ms: i64) -> StrategyTick<TestFeatures> {
        StrategyTick {
            occurred_at_ms: at_ms,
            features: TestFeatures { should_trade: true },
        }
    }

    fn engine(
        max_orders_per_hour: u32,
        executor: TestExecutor,
        ledger: LedgerStore,
        lifecycle: MemoryLifecycleSink,
        wakeups: MemoryWakeupSink,
    ) -> StrategyEngine<TestProgram, TestExecutor> {
        StrategyEngine::new(
            TestProgram,
            executor,
            TradingNetwork::Signet,
            TestMandateAuthority {
                max_orders_per_hour,
            },
            TestBacktestGate { passing: true },
            ledger,
            Arc::new(lifecycle),
            Arc::new(wakeups),
        )
    }

    #[test]
    fn strategy_program_is_deterministic_and_model_free() {
        let program = TestProgram;
        let config = TestConfig { protected: true };
        let state = TestState::default();
        let tick = tick(100);
        assert_eq!(
            program
                .on_tick(&config, &state, &tick)
                .expect("first result"),
            program
                .on_tick(&config, &state, &tick)
                .expect("second result")
        );
    }

    #[test]
    fn strategy_cannot_start_without_passing_backtest_for_its_config() {
        let executor = TestExecutor::successful();
        let mut engine = StrategyEngine::new(
            TestProgram,
            executor.clone(),
            TradingNetwork::Signet,
            TestMandateAuthority {
                max_orders_per_hour: 2,
            },
            TestBacktestGate { passing: false },
            LedgerStore::in_memory().expect("ledger"),
            Arc::new(MemoryLifecycleSink::default()),
            Arc::new(MemoryWakeupSink::default()),
        );

        let error = engine
            .start(TestConfig { protected: true }, 1)
            .expect_err("missing backtest");
        assert!(error.to_string().contains("no passing backtest"));
        assert!(matches!(engine.status(), StrategyStatus::Idle));
        assert_eq!(executor.calls(), 0);
    }

    #[test]
    fn admitted_order_executes_once_and_records_every_venue_event() {
        block_on(async {
            let executor = TestExecutor::successful();
            let ledger = LedgerStore::in_memory().expect("ledger");
            let lifecycle = MemoryLifecycleSink::default();
            let wakeups = MemoryWakeupSink::default();
            let mut engine = engine(
                2,
                executor.clone(),
                ledger.clone(),
                lifecycle.clone(),
                wakeups.clone(),
            );
            engine
                .start(TestConfig { protected: true }, 1)
                .expect("start");
            let report = engine.handle_tick(tick(100)).await.expect("tick");
            assert_eq!(report.submitted_count, 1);
            assert_eq!(executor.calls(), 1);
            assert_eq!(engine.state().expect("state").executions, 1);
            assert_eq!(
                ledger
                    .entries(&LedgerQuery::default())
                    .expect("entries")
                    .iter()
                    .map(|entry| entry.kind.clone())
                    .collect::<Vec<_>>(),
                vec![
                    LedgerEntryKind::Order,
                    LedgerEntryKind::Fill,
                    LedgerEntryKind::Fee,
                    LedgerEntryKind::FundingSettlement,
                ]
            );
            assert!(matches!(engine.status(), StrategyStatus::Running { .. }));
            assert!(wakeups.wakeups().is_empty());
            assert!(
                lifecycle
                    .events()
                    .iter()
                    .any(|event| matches!(event, StrategyLifecycleEvent::OrderSubmitted { .. }))
            );
            assert!(lifecycle.events().iter().any(|event| matches!(
                event,
                StrategyLifecycleEvent::StateUpdated { at_ms: 100, state, .. }
                    if state["ticks"] == 1
            )));
        });
    }

    #[test]
    fn hard_limit_halts_and_emits_typed_agent_wakeup() {
        block_on(async {
            let executor = TestExecutor::successful();
            let ledger = LedgerStore::in_memory().expect("ledger");
            let lifecycle = MemoryLifecycleSink::default();
            let wakeups = MemoryWakeupSink::default();
            let mut engine = engine(1, executor.clone(), ledger, lifecycle, wakeups.clone());
            engine
                .start(TestConfig { protected: true }, 1)
                .expect("start");
            engine.handle_tick(tick(100)).await.expect("first order");
            let error = engine
                .handle_tick(tick(200))
                .await
                .expect_err("hourly limit");
            assert!(error.to_string().contains("HourlyOrderLimit"));
            assert_eq!(executor.calls(), 1);
            assert!(matches!(engine.status(), StrategyStatus::Halted { .. }));
            assert!(matches!(
                wakeups.wakeups().as_slice(),
                [(WakeupSource::StrategyHalt { strategy, .. }, _)] if strategy == "test_strategy"
            ));
        });
    }

    #[test]
    fn ambiguous_mutation_failure_is_not_retried_and_halts() {
        block_on(async {
            let executor = TestExecutor::ambiguous_failure();
            let ledger = LedgerStore::in_memory().expect("ledger");
            let wakeups = MemoryWakeupSink::default();
            let mut engine = engine(
                2,
                executor.clone(),
                ledger.clone(),
                MemoryLifecycleSink::default(),
                wakeups.clone(),
            );
            engine
                .start(TestConfig { protected: true }, 1)
                .expect("start");
            let error = engine
                .handle_tick(tick(100))
                .await
                .expect_err("ambiguous failure");

            assert!(error.to_string().contains("single-attempt order failed"));
            assert_eq!(executor.calls(), 1);
            assert!(matches!(engine.status(), StrategyStatus::Halted { .. }));
            assert_eq!(
                ledger
                    .entries(&LedgerQuery::default())
                    .expect("entries")
                    .iter()
                    .map(|entry| entry.kind.clone())
                    .collect::<Vec<_>>(),
                vec![LedgerEntryKind::Order]
            );
            assert_eq!(wakeups.wakeups().len(), 1);
        });
    }

    #[test]
    fn leveraged_risk_without_venue_stop_halts_before_execution() {
        block_on(async {
            let executor = TestExecutor::successful();
            let mut engine = engine(
                2,
                executor.clone(),
                LedgerStore::in_memory().expect("ledger"),
                MemoryLifecycleSink::default(),
                MemoryWakeupSink::default(),
            );
            engine
                .start(TestConfig { protected: false }, 1)
                .expect("start");
            let error = engine
                .handle_tick(tick(100))
                .await
                .expect_err("missing stop");
            assert!(error.to_string().contains("no venue-side stop loss"));
            assert_eq!(executor.calls(), 0);
        });
    }

    #[test]
    fn background_service_processes_commands_in_order() {
        block_on(async {
            let executor = TestExecutor::successful();
            let mut handle;
            let service;
            (handle, service) = background_service(engine(
                2,
                executor.clone(),
                LedgerStore::in_memory().expect("ledger"),
                MemoryLifecycleSink::default(),
                MemoryWakeupSink::default(),
            ));
            handle
                .send(StrategyCommand::Start {
                    config: TestConfig { protected: true },
                    at_ms: 1,
                })
                .expect("send start");
            handle
                .send(StrategyCommand::Tick(tick(100)))
                .expect("send tick");
            handle
                .send(StrategyCommand::Shutdown)
                .expect("send shutdown");
            let engine = service.await.expect("service");
            assert_eq!(engine.state().expect("state").ticks, 1);
            assert_eq!(executor.calls(), 1);
        });
    }

    #[test]
    fn requested_commands_are_acknowledged_and_rejections_do_not_stop_the_service() {
        block_on(async {
            let executor = TestExecutor::successful();
            let (mut handle, service) = background_service(engine(
                2,
                executor,
                LedgerStore::in_memory().expect("ledger"),
                MemoryLifecycleSink::default(),
                MemoryWakeupSink::default(),
            ));
            let commands = async move {
                let error = handle
                    .request(StrategyCommand::Start {
                        config: TestConfig { protected: true },
                        at_ms: -1,
                    })
                    .await
                    .expect_err("negative timestamp");
                assert!(error.to_string().contains("must not be negative"));
                handle
                    .request(StrategyCommand::Start {
                        config: TestConfig { protected: true },
                        at_ms: 1,
                    })
                    .await
                    .expect("valid start");
                handle
                    .request(StrategyCommand::Shutdown)
                    .await
                    .expect("shutdown");
            };
            let ((), engine) = futures::join!(commands, service);
            assert!(matches!(
                engine.expect("service").status(),
                StrategyStatus::Running { started_at_ms: 1 }
            ));
        });
    }
}
