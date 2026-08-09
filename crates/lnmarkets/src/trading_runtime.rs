use std::{collections::VecDeque, future::Future, sync::Arc};

use anyhow::{Context as _, Result, bail};
use gpui::{AppContext as _, AsyncApp, Task};
use lnmarkets_client::{LnMarketsClient, Network};
use lnmarkets_data::{CollectorHandle, FeatureSnapshot};
use lnmarkets_trading::{
    BacktestCostModel, BacktestPolicy, BacktestReport, BacktestStore, BacktestTick,
    FundingCarryBacktestModel, FundingCarryConfig, FundingCarryExecutor, FundingCarryFeatures,
    FundingCarryInstrument, FundingCarryProgram, MemoryLifecycleSink, MemoryWakeupSink,
    RebalanceBacktestModel, RebalanceToTargetConfig, RebalanceToTargetProgram, StrategyCommand,
    StrategyEngine, StrategyHaltReason, StrategyLifecycleEvent, StrategyServiceHandle,
    StrategyTick, SyntheticUsdExecutor, ThresholdSwingBacktestModel, ThresholdSwingConfig,
    ThresholdSwingExecutor, ThresholdSwingProgram, background_service, collected_backtest_replay,
    funding_carry_features, run_backtest,
};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use trading_ledger::{LedgerQuery, LedgerStore, ProfitReport};
use trading_mandate::{
    MandateRevision, MandateSnapshot, MandateStore, ReviewCadence, TradingNetwork,
};

use crate::review_turn::{PortfolioReview, hourly_start};
use crate::{SoakLimitBreach, SoakReviewTurn, SoakWindow};

const LNMARKETS_VENUE: &str = trading_mandate::LEGACY_VENUE;
const REBALANCE_STRATEGY_ID: &str = "rebalance_to_target";
const FUNDING_STRATEGY_ID: &str = "funding_carry";
const THRESHOLD_SWING_STRATEGY_ID: &str = "threshold_swing";

type RebalanceHandle = StrategyServiceHandle<RebalanceToTargetConfig, FeatureSnapshot>;
type FundingHandle = StrategyServiceHandle<FundingCarryConfig, FundingCarryFeatures>;
type ThresholdSwingHandle = StrategyServiceHandle<ThresholdSwingConfig, FeatureSnapshot>;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StrategyRuntimeSnapshot {
    pub strategy_id: String,
    pub status: String,
    pub started_at_ms: Option<i64>,
    pub halted_at_ms: Option<i64>,
    pub halt_reason: Option<Value>,
    pub state: Option<Value>,
    pub last_action: Option<String>,
    pub lifecycle_event_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecordedBacktest {
    pub report_digest: String,
    pub report: BacktestReport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewTurnOutcome {
    Completed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReviewTurnHistory {
    pub at_ms: i64,
    pub source: agent_wakeup::WakeupSource,
    pub outcome: ReviewTurnOutcome,
}

pub struct TradingRuntime {
    ledger: LedgerStore,
    mandate: MandateStore,
    backtests: BacktestStore,
    lifecycle: MemoryLifecycleSink,
    wakeups: MemoryWakeupSink,
    review_history: Mutex<VecDeque<ReviewTurnHistory>>,
    soak_review_turns: Mutex<VecDeque<SoakReviewTurn>>,
    review_session_id: Mutex<Option<String>>,
    rebalance: Mutex<Option<RebalanceHandle>>,
    funding: Mutex<Option<FundingHandle>>,
    funding_instrument: Mutex<Option<FundingCarryInstrument>>,
    threshold_swing: Mutex<Option<ThresholdSwingHandle>>,
}

impl TradingRuntime {
    pub fn open_default() -> Result<Self> {
        Ok(Self {
            ledger: LedgerStore::open_default().context("could not open the trading ledger")?,
            mandate: MandateStore::open_default().context("could not open the trading mandate")?,
            backtests: BacktestStore::open_default().context("could not open backtest reports")?,
            lifecycle: MemoryLifecycleSink::default(),
            wakeups: MemoryWakeupSink::default(),
            review_history: Mutex::new(VecDeque::new()),
            soak_review_turns: Mutex::new(VecDeque::new()),
            review_session_id: Mutex::new(None),
            rebalance: Mutex::new(None),
            funding: Mutex::new(None),
            funding_instrument: Mutex::new(None),
            threshold_swing: Mutex::new(None),
        })
    }

    pub fn profit_report(&self, query: &LedgerQuery) -> Result<ProfitReport> {
        self.ledger.profit_report(query)
    }

    pub fn ledger_entries(&self, query: &LedgerQuery) -> Result<Vec<trading_ledger::LedgerEntry>> {
        self.ledger.entries(query)
    }

    pub fn venue_balance(&self, venue: &str) -> Result<i64> {
        self.ledger.venue_balance(venue)
    }

    pub fn backtest_reports(
        &self,
        strategy_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<BacktestReport>> {
        self.backtests.reports(strategy_id, limit)
    }

    pub fn run_rebalance_backtest(
        &self,
        collector: &CollectorHandle,
        config: RebalanceToTargetConfig,
        from_ms: i64,
        to_ms: i64,
        cost_model: BacktestCostModel,
        policy: BacktestPolicy,
        created_at_ms: i64,
    ) -> Result<RecordedBacktest> {
        require_backtest_collector(collector)?;
        if cost_model.observed_round_trip_cost_bps
            != config.cost_measurement.observed_round_trip_cost_bps
            || cost_model.measurement_source != config.cost_measurement.source
            || cost_model.measured_at_ms != config.cost_measurement.measured_at_ms
        {
            bail!(
                "rebalance backtest cost model must match the configuration's ledger measurement"
            );
        }
        let replay = collected_backtest_replay(collector.store(), Network::Signet, from_ms, to_ms)?;
        let allocation = collector
            .store()
            .account_allocation(Network::Signet)?
            .context("rebalance backtest requires a collected account allocation")?;
        let mut model = RebalanceBacktestModel::new(allocation)?;
        let report = run_backtest(
            &RebalanceToTargetProgram,
            &config,
            &replay.ticks,
            &mut model,
            TradingNetwork::Signet,
            cost_model,
            policy,
            created_at_ms,
        )?;
        self.record_backtest(report)
    }

    pub fn run_funding_backtest(
        &self,
        collector: &CollectorHandle,
        config: FundingCarryConfig,
        from_ms: i64,
        to_ms: i64,
        cost_model: BacktestCostModel,
        policy: BacktestPolicy,
        created_at_ms: i64,
    ) -> Result<RecordedBacktest> {
        require_backtest_collector(collector)?;
        if cost_model.observed_round_trip_cost_bps != config.measured_round_trip_cost_bps {
            bail!("funding carry backtest cost must match the strategy configuration");
        }
        let replay = collected_backtest_replay(collector.store(), Network::Signet, from_ms, to_ms)?;
        let ticks = replay
            .ticks
            .into_iter()
            .map(|tick| BacktestTick {
                occurred_at_ms: tick.occurred_at_ms,
                features: FundingCarryFeatures {
                    market: tick.features,
                    position: None,
                    settled_funding_sats: 0,
                },
            })
            .collect::<Vec<_>>();
        let mut model = FundingCarryBacktestModel::default();
        let report = run_backtest(
            &FundingCarryProgram,
            &config,
            &ticks,
            &mut model,
            TradingNetwork::Signet,
            cost_model,
            policy,
            created_at_ms,
        )?;
        self.record_backtest(report)
    }

    pub fn run_threshold_swing_backtest(
        &self,
        collector: &CollectorHandle,
        config: ThresholdSwingConfig,
        from_ms: i64,
        to_ms: i64,
        cost_model: BacktestCostModel,
        policy: BacktestPolicy,
        created_at_ms: i64,
    ) -> Result<RecordedBacktest> {
        require_backtest_collector(collector)?;
        if cost_model.observed_round_trip_cost_bps != config.measured_round_trip_cost_bps {
            bail!("threshold swing backtest cost must match the strategy configuration");
        }
        let replay = collected_backtest_replay(collector.store(), Network::Signet, from_ms, to_ms)?;
        let mut model = ThresholdSwingBacktestModel::default();
        let report = run_backtest(
            &ThresholdSwingProgram,
            &config,
            &replay.ticks,
            &mut model,
            TradingNetwork::Signet,
            cost_model,
            policy,
            created_at_ms,
        )?;
        self.record_backtest(report)
    }

    fn record_backtest(&self, report: BacktestReport) -> Result<RecordedBacktest> {
        let report_digest = self.backtests.record(&report)?;
        Ok(RecordedBacktest {
            report_digest,
            report,
        })
    }

    pub fn signet_soak_ledger_summary(&self, window: &SoakWindow) -> Result<ProfitReport> {
        self.ledger.profit_report(&LedgerQuery {
            from_ms: Some(window.started_at_ms),
            to_ms: Some(window.ended_at_ms),
            strategy_id: None,
        })
    }

    pub fn signet_soak_limit_breaches(&self, window: &SoakWindow) -> Vec<SoakLimitBreach> {
        self.lifecycle
            .events()
            .into_iter()
            .filter_map(|event| {
                let StrategyLifecycleEvent::Halted {
                    strategy_id,
                    at_ms,
                    reason: StrategyHaltReason::RiskLimit { refusal },
                } = event
                else {
                    return None;
                };
                if at_ms < window.started_at_ms || at_ms > window.ended_at_ms {
                    return None;
                }
                let wakeup =
                    self.wakeups
                        .wakeups()
                        .into_iter()
                        .find_map(|(source, _instruction)| match &source {
                            agent_wakeup::WakeupSource::StrategyHalt { strategy, .. }
                                if strategy == &strategy_id =>
                            {
                                Some(source)
                            }
                            _ => None,
                        });
                Some(SoakLimitBreach {
                    at_ms,
                    strategy_id,
                    refusal,
                    strategy_halted: true,
                    wakeup: wakeup.unwrap_or_else(|| agent_wakeup::WakeupSource::External {
                        event_type: "missing_strategy_halt_wakeup".to_string(),
                        summary: "the risk halt had no matching typed wakeup".to_string(),
                    }),
                })
            })
            .collect()
    }

    pub fn mandate_snapshot(&self) -> Result<MandateSnapshot> {
        self.mandate.snapshot()
    }

    pub fn mandate_history(&self) -> Result<Vec<MandateRevision>> {
        self.mandate.history()
    }

    pub fn narrow_mandate(&self, changed_at_ms: i64) -> Result<MandateSnapshot> {
        let snapshot = self.mandate.snapshot()?;
        let mandate = snapshot
            .mandate_for(LNMARKETS_VENUE, TradingNetwork::Signet)
            .context("no active mandate is available to narrow")?
            .clone();
        let proposal = self.mandate.propose(narrowed_mandate(mandate))?;
        self.mandate.save_restriction(proposal, changed_at_ms)
    }

    pub fn revoke_mandate(&self, changed_at_ms: i64) -> Result<MandateSnapshot> {
        self.mandate
            .revoke(LNMARKETS_VENUE, TradingNetwork::Signet, changed_at_ms)
    }

    pub fn strategy_snapshots(&self) -> Vec<StrategyRuntimeSnapshot> {
        [
            REBALANCE_STRATEGY_ID,
            FUNDING_STRATEGY_ID,
            THRESHOLD_SWING_STRATEGY_ID,
        ]
        .into_iter()
        .map(|strategy_id| self.strategy_snapshot(strategy_id))
        .collect()
    }

    pub fn claim_review_session(&self, session_id: impl Into<String>) -> Result<()> {
        let session_id = session_id.into();
        if session_id.trim().is_empty() {
            bail!("portfolio review session ID must not be empty");
        }
        *self.review_session_id.lock() = Some(session_id);
        Ok(())
    }

    pub fn is_review_session(&self, session_id: &str) -> bool {
        self.review_session_id
            .lock()
            .as_deref()
            .is_some_and(|claimed| claimed == session_id)
    }

    pub fn review_cadence(&self, session_id: &str) -> Result<Option<ReviewCadence>> {
        if !self.is_review_session(session_id) {
            return Ok(None);
        }
        Ok(self
            .mandate
            .snapshot()?
            .mandate_for(LNMARKETS_VENUE, TradingNetwork::Signet)
            .map(|mandate| mandate.review_cadence.clone()))
    }

    pub fn pending_review_wakeup(
        &self,
        session_id: &str,
    ) -> Option<(agent_wakeup::WakeupSource, String)> {
        self.is_review_session(session_id)
            .then(|| self.wakeups.pending())
            .flatten()
    }

    pub fn acknowledge_review_wakeup(
        &self,
        session_id: &str,
        source: &agent_wakeup::WakeupSource,
        instruction: &str,
    ) -> bool {
        self.is_review_session(session_id) && self.wakeups.acknowledge(source, instruction)
    }

    pub fn record_review_turn(
        &self,
        session_id: &str,
        at_ms: i64,
        source: agent_wakeup::WakeupSource,
        outcome: ReviewTurnOutcome,
    ) -> bool {
        if !self.is_review_session(session_id) {
            return false;
        }
        let mut history = self.review_history.lock();
        if history.len() == 20 {
            history.pop_front();
        }
        history.push_back(ReviewTurnHistory {
            at_ms,
            source,
            outcome,
        });
        true
    }

    pub fn review_turn_history(&self) -> Vec<ReviewTurnHistory> {
        self.review_history.lock().iter().rev().cloned().collect()
    }

    pub fn record_soak_review_turn(&self, session_id: &str, turn: SoakReviewTurn) -> bool {
        if !self.is_review_session(session_id) {
            return false;
        }
        let mut turns = self.soak_review_turns.lock();
        if turns.len() == 1_000 {
            turns.pop_front();
        }
        turns.push_back(turn);
        true
    }

    pub fn soak_review_turns(&self) -> Vec<SoakReviewTurn> {
        self.soak_review_turns.lock().iter().cloned().collect()
    }

    pub fn pending_review_wakeup_count(&self) -> usize {
        self.wakeups.wakeups().len()
    }

    pub fn portfolio_review(
        &self,
        now_ms: i64,
        trigger: impl Into<String>,
        feature_status: impl Into<String>,
        features: Option<FeatureSnapshot>,
    ) -> Result<PortfolioReview> {
        let daily_query = LedgerQuery {
            from_ms: Some(now_ms.saturating_sub(24 * 60 * 60 * 1_000).max(0)),
            to_ms: Some(now_ms),
            strategy_id: None,
        };
        let hourly_query = LedgerQuery {
            from_ms: Some(hourly_start(now_ms)),
            to_ms: Some(now_ms),
            strategy_id: None,
        };
        Ok(PortfolioReview::build(
            now_ms,
            trigger,
            feature_status,
            features,
            self.ledger.profit_report(&daily_query)?,
            &self.ledger.entries(&hourly_query)?,
            self.mandate.snapshot()?,
            self.strategy_snapshots(),
            self.backtests.reports(None, 20)?,
        ))
    }

    pub async fn start_rebalance(
        &self,
        client: LnMarketsClient,
        config: RebalanceToTargetConfig,
        at_ms: i64,
        cx: &AsyncApp,
    ) -> Result<()> {
        require_signet(&client, config.network)?;
        self.require_strategy_mandate(
            REBALANCE_STRATEGY_ID,
            at_ms,
            config.maximum_order_value_usd_cents.div_ceil(100),
            1,
        )?;
        let mut handle = match self.rebalance.lock().clone() {
            Some(handle) => handle,
            None => {
                let executor = SyntheticUsdExecutor::new(client, Network::Signet)?;
                let engine = StrategyEngine::new(
                    RebalanceToTargetProgram,
                    executor,
                    TradingNetwork::Signet,
                    self.mandate.clone(),
                    self.backtests.clone(),
                    self.ledger.clone(),
                    Arc::new(self.lifecycle.clone()),
                    Arc::new(self.wakeups.clone()),
                );
                let (handle, service) = background_service(engine);
                spawn_service("rebalance_to_target", service, cx);
                *self.rebalance.lock() = Some(handle.clone());
                handle
            }
        };
        handle
            .request(StrategyCommand::Start { config, at_ms })
            .await
    }

    pub async fn adjust_rebalance(
        &self,
        config: RebalanceToTargetConfig,
        at_ms: i64,
    ) -> Result<()> {
        if config.network != Network::Signet {
            bail!("automated LN Markets strategies are restricted to signet");
        }
        self.require_strategy_mandate(
            REBALANCE_STRATEGY_ID,
            at_ms,
            config.maximum_order_value_usd_cents.div_ceil(100),
            1,
        )?;
        let mut handle = self
            .rebalance
            .lock()
            .clone()
            .context("rebalance_to_target has not been started")?;
        handle.request(StrategyCommand::Adjust { config }).await
    }

    pub async fn halt_rebalance(&self, at_ms: i64, reason: String) -> Result<()> {
        let mut handle = self
            .rebalance
            .lock()
            .clone()
            .context("rebalance_to_target has not been started")?;
        handle
            .request(StrategyCommand::Halt { at_ms, reason })
            .await
    }

    pub async fn start_funding(
        &self,
        client: LnMarketsClient,
        config: FundingCarryConfig,
        at_ms: i64,
        cx: &AsyncApp,
    ) -> Result<()> {
        require_signet(&client, config.network)?;
        self.require_strategy_mandate(
            FUNDING_STRATEGY_ID,
            at_ms,
            config.maximum_hedge_notional_usd,
            config.leverage,
        )?;
        let mut handle = match self.funding.lock().clone() {
            Some(handle) => handle,
            None => {
                let executor = FundingCarryExecutor::new(client)?;
                let engine = StrategyEngine::new(
                    FundingCarryProgram,
                    executor,
                    TradingNetwork::Signet,
                    self.mandate.clone(),
                    self.backtests.clone(),
                    self.ledger.clone(),
                    Arc::new(self.lifecycle.clone()),
                    Arc::new(self.wakeups.clone()),
                );
                let (handle, service) = background_service(engine);
                spawn_service("funding_carry", service, cx);
                *self.funding.lock() = Some(handle.clone());
                handle
            }
        };
        let instrument = config.instrument;
        handle
            .request(StrategyCommand::Start { config, at_ms })
            .await?;
        *self.funding_instrument.lock() = Some(instrument);
        Ok(())
    }

    pub async fn adjust_funding(&self, config: FundingCarryConfig, at_ms: i64) -> Result<()> {
        if config.network != Network::Signet {
            bail!("automated LN Markets strategies are restricted to signet");
        }
        self.require_strategy_mandate(
            FUNDING_STRATEGY_ID,
            at_ms,
            config.maximum_hedge_notional_usd,
            config.leverage,
        )?;
        let mut handle = self
            .funding
            .lock()
            .clone()
            .context("funding_carry has not been started")?;
        let instrument = config.instrument;
        handle.request(StrategyCommand::Adjust { config }).await?;
        *self.funding_instrument.lock() = Some(instrument);
        Ok(())
    }

    pub async fn halt_funding(&self, at_ms: i64, reason: String) -> Result<()> {
        let mut handle = self
            .funding
            .lock()
            .clone()
            .context("funding_carry has not been started")?;
        handle
            .request(StrategyCommand::Halt { at_ms, reason })
            .await
    }

    pub async fn start_threshold_swing(
        &self,
        client: LnMarketsClient,
        config: ThresholdSwingConfig,
        at_ms: i64,
        cx: &AsyncApp,
    ) -> Result<()> {
        require_signet(&client, config.network)?;
        self.require_strategy_mandate(
            THRESHOLD_SWING_STRATEGY_ID,
            at_ms,
            config.maximum_position_usd_cents.div_ceil(100),
            1,
        )?;
        let mut handle = match self.threshold_swing.lock().clone() {
            Some(handle) => handle,
            None => {
                let executor = ThresholdSwingExecutor::new(client)?;
                let engine = StrategyEngine::new(
                    ThresholdSwingProgram,
                    executor,
                    TradingNetwork::Signet,
                    self.mandate.clone(),
                    self.backtests.clone(),
                    self.ledger.clone(),
                    Arc::new(self.lifecycle.clone()),
                    Arc::new(self.wakeups.clone()),
                );
                let (handle, service) = background_service(engine);
                spawn_service(THRESHOLD_SWING_STRATEGY_ID, service, cx);
                *self.threshold_swing.lock() = Some(handle.clone());
                handle
            }
        };
        handle
            .request(StrategyCommand::Start { config, at_ms })
            .await
    }

    pub async fn adjust_threshold_swing(
        &self,
        config: ThresholdSwingConfig,
        at_ms: i64,
    ) -> Result<()> {
        if config.network != Network::Signet {
            bail!("automated LN Markets strategies are restricted to signet");
        }
        self.require_strategy_mandate(
            THRESHOLD_SWING_STRATEGY_ID,
            at_ms,
            config.maximum_position_usd_cents.div_ceil(100),
            1,
        )?;
        let mut handle = self
            .threshold_swing
            .lock()
            .clone()
            .context("threshold_swing has not been started")?;
        handle.request(StrategyCommand::Adjust { config }).await
    }

    pub async fn halt_threshold_swing(&self, at_ms: i64, reason: String) -> Result<()> {
        let mut handle = self
            .threshold_swing
            .lock()
            .clone()
            .context("threshold_swing has not been started")?;
        handle
            .request(StrategyCommand::Halt { at_ms, reason })
            .await
    }

    pub async fn process_collected_tick(
        &self,
        collector: &CollectorHandle,
        at_ms: i64,
    ) -> Result<()> {
        let rebalance_running = self.strategy_snapshot(REBALANCE_STRATEGY_ID).status == "running";
        let threshold_running =
            self.strategy_snapshot(THRESHOLD_SWING_STRATEGY_ID).status == "running";
        let funding_running = self.strategy_snapshot(FUNDING_STRATEGY_ID).status == "running";
        if !rebalance_running && !threshold_running && !funding_running {
            return Ok(());
        }
        let features = collector
            .features()?
            .context("LN Markets collector has no feature snapshot")?;
        let mut errors = Vec::new();

        let rebalance_handle = self.rebalance.lock().clone();
        if rebalance_running
            && let Some(mut handle) = rebalance_handle
            && let Err(error) = handle
                .request(StrategyCommand::Tick(StrategyTick {
                    occurred_at_ms: at_ms,
                    features: features.clone(),
                }))
                .await
        {
            errors.push(format!("rebalance_to_target: {error:#}"));
        }

        let threshold_handle = self.threshold_swing.lock().clone();
        if threshold_running
            && let Some(mut handle) = threshold_handle
            && let Err(error) = handle
                .request(StrategyCommand::Tick(StrategyTick {
                    occurred_at_ms: at_ms,
                    features: features.clone(),
                }))
                .await
        {
            errors.push(format!("threshold_swing: {error:#}"));
        }

        if funding_running {
            let instrument = *self
                .funding_instrument
                .lock()
                .as_ref()
                .context("running funding_carry has no configured instrument")?;
            let network = collector.health().network;
            match funding_carry_features(collector.store(), network, instrument) {
                Ok(features) => {
                    let funding_handle = self.funding.lock().clone();
                    if let Some(mut handle) = funding_handle
                        && let Err(error) = handle
                            .request(StrategyCommand::Tick(StrategyTick {
                                occurred_at_ms: at_ms,
                                features,
                            }))
                            .await
                    {
                        errors.push(format!("funding_carry: {error:#}"));
                    }
                }
                Err(error) => errors.push(format!("funding_carry features: {error:#}")),
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            bail!("LN Markets strategy tick failed: {}", errors.join("; "))
        }
    }

    fn strategy_snapshot(&self, strategy_id: &str) -> StrategyRuntimeSnapshot {
        let events = self
            .lifecycle
            .events()
            .into_iter()
            .filter(|event| lifecycle_strategy_id(event) == strategy_id)
            .collect::<Vec<_>>();
        let mut status = "idle".to_string();
        let mut started_at_ms = None;
        let mut halted_at_ms = None;
        let mut halt_reason = None;
        let mut state = None;
        for event in &events {
            match event {
                StrategyLifecycleEvent::Started { at_ms, .. } => {
                    status = "running".to_string();
                    started_at_ms = Some(at_ms);
                    halted_at_ms = None;
                    halt_reason = None;
                }
                StrategyLifecycleEvent::StateUpdated {
                    state: next_state, ..
                } => state = Some(next_state.clone()),
                StrategyLifecycleEvent::Halted { at_ms, reason, .. } => {
                    status = "halted".to_string();
                    halted_at_ms = Some(at_ms);
                    halt_reason = serde_json::to_value(reason).ok();
                }
                _ => {}
            }
        }
        StrategyRuntimeSnapshot {
            strategy_id: strategy_id.to_string(),
            status,
            started_at_ms: started_at_ms.copied(),
            halted_at_ms: halted_at_ms.copied(),
            halt_reason,
            state,
            last_action: events.last().map(lifecycle_summary),
            lifecycle_event_count: events.len(),
        }
    }

    fn require_strategy_mandate(
        &self,
        strategy_id: &str,
        at_ms: i64,
        maximum_position_usd: u64,
        leverage: u8,
    ) -> Result<()> {
        let snapshot = self.mandate.snapshot()?;
        let mandate = snapshot
            .mandate_for(LNMARKETS_VENUE, TradingNetwork::Signet)
            .context(
                "no trading mandate is active; approve one in Settings before starting a strategy",
            )?;
        if mandate.expires_at_ms <= at_ms {
            bail!("the active trading mandate has expired");
        }
        if mandate.network != TradingNetwork::Signet {
            bail!("automated LN Markets strategies require a signet mandate");
        }
        if !mandate.allowed_strategies.contains(strategy_id) {
            bail!("the active trading mandate does not allow {strategy_id}");
        }
        if maximum_position_usd > mandate.max_position_usd {
            bail!(
                "{strategy_id} can reach {maximum_position_usd} USD, above the mandate limit of {} USD",
                mandate.max_position_usd
            );
        }
        if leverage > mandate.max_leverage {
            bail!(
                "{strategy_id} leverage {leverage} exceeds the mandate limit of {}",
                mandate.max_leverage
            );
        }
        Ok(())
    }
}

fn lifecycle_strategy_id(event: &StrategyLifecycleEvent) -> &str {
    match event {
        StrategyLifecycleEvent::Started { strategy_id, .. }
        | StrategyLifecycleEvent::TickProcessed { strategy_id, .. }
        | StrategyLifecycleEvent::OrderAuthorized { strategy_id, .. }
        | StrategyLifecycleEvent::OrderSubmitted { strategy_id, .. }
        | StrategyLifecycleEvent::CancelResolved { strategy_id, .. }
        | StrategyLifecycleEvent::BacktestApproved { strategy_id, .. }
        | StrategyLifecycleEvent::StateUpdated { strategy_id, .. }
        | StrategyLifecycleEvent::LedgerEntryAppended { strategy_id, .. }
        | StrategyLifecycleEvent::Halted { strategy_id, .. } => strategy_id,
    }
}

fn lifecycle_summary(event: &StrategyLifecycleEvent) -> String {
    match event {
        StrategyLifecycleEvent::Started { at_ms, .. } => format!("started at {at_ms}"),
        StrategyLifecycleEvent::TickProcessed {
            at_ms,
            intent_count,
            ..
        } => format!("processed {intent_count} intents at {at_ms}"),
        StrategyLifecycleEvent::OrderAuthorized { intent_id, .. } => {
            format!("authorized order {intent_id}")
        }
        StrategyLifecycleEvent::OrderSubmitted { venue_order_id, .. } => {
            format!("submitted venue order {venue_order_id}")
        }
        StrategyLifecycleEvent::CancelResolved { venue_order_id, .. } => {
            format!("cancelled venue order {venue_order_id}")
        }
        StrategyLifecycleEvent::BacktestApproved { .. } => "backtest approved".to_string(),
        StrategyLifecycleEvent::StateUpdated { at_ms, .. } => {
            format!("updated state at {at_ms}")
        }
        StrategyLifecycleEvent::LedgerEntryAppended { sequence, .. } => {
            format!("recorded ledger entry {sequence}")
        }
        StrategyLifecycleEvent::Halted { at_ms, .. } => format!("halted at {at_ms}"),
    }
}

fn narrowed_mandate(
    mut mandate: trading_mandate::TradingMandate,
) -> trading_mandate::TradingMandate {
    mandate.max_venue_balance = (mandate.max_venue_balance / 2).max(1);
    mandate.max_position_usd = (mandate.max_position_usd / 2).max(1);
    mandate.max_leverage = (mandate.max_leverage / 2).max(1);
    mandate.daily_loss_stop = (mandate.daily_loss_stop / 2).max(1);
    mandate.max_orders_per_hour /= 2;
    mandate
}

fn require_signet(client: &LnMarketsClient, network: Network) -> Result<()> {
    if network != Network::Signet || client.network() != Network::Signet {
        bail!("automated LN Markets strategies are restricted to signet");
    }
    Ok(())
}

fn require_backtest_collector(collector: &CollectorHandle) -> Result<()> {
    let health = collector.health();
    if health.network != Network::Signet {
        bail!("automated LN Markets strategy backtests are restricted to signet data");
    }
    Ok(())
}

fn spawn_service<Engine>(
    strategy_id: &'static str,
    service: impl Future<Output = Result<Engine>> + Send + 'static,
    cx: &AsyncApp,
) where
    Engine: Send + 'static,
{
    let task: Task<()> = cx.background_spawn(async move {
        if let Err(error) = service.await {
            log::error!("LN Markets {strategy_id} service stopped: {error:#}");
        }
    });
    task.detach();
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, sync::Arc, time::Duration};

    use futures::{FutureExt as _, future::BoxFuture};
    use http::{Request, Response};
    use lnmarkets_client::{HttpTransport, TransportFailure};
    use lnmarkets_data::{
        AccountAllocation, CANDLE_TOPIC, Collector, CollectorConfig, EventSource,
        FUNDING_SETTLEMENT_TOPIC, MarketDataStore, ORACLE_INDEX_TOPIC, STREAM_BUCKETS_TOPIC,
    };
    use lnmarkets_trading::{BacktestGate as _, RebalanceCostMeasurement};
    use serde_json::json;
    use trading_mandate::{ReviewCadence, TradingMandate, TradingNetwork};

    use super::*;

    #[derive(Default)]
    struct NoopTransport;

    impl HttpTransport for NoopTransport {
        fn send(
            &self,
            _request: Request<Vec<u8>>,
        ) -> BoxFuture<'static, Result<Response<Vec<u8>>, TransportFailure>> {
            async move { Ok(Response::new(Vec::new())) }.boxed()
        }
    }

    fn test_runtime(backtests: BacktestStore) -> TradingRuntime {
        TradingRuntime {
            ledger: LedgerStore::in_memory().expect("ledger"),
            mandate: MandateStore::in_memory().expect("mandate"),
            backtests,
            lifecycle: MemoryLifecycleSink::default(),
            wakeups: MemoryWakeupSink::default(),
            review_history: Mutex::new(VecDeque::new()),
            soak_review_turns: Mutex::new(VecDeque::new()),
            review_session_id: Mutex::new(None),
            rebalance: Mutex::new(None),
            funding: Mutex::new(None),
            funding_instrument: Mutex::new(None),
            threshold_swing: Mutex::new(None),
        }
    }

    fn replay_collector() -> CollectorHandle {
        let store = MarketDataStore::in_memory(Duration::from_secs(60)).expect("market data");
        for (topic, events) in [
            (
                CANDLE_TOPIC,
                vec![
                    (100, json!({"close": 100.0})),
                    (300, json!({"close": 100.0})),
                ],
            ),
            (
                FUNDING_SETTLEMENT_TOPIC,
                vec![
                    (200, json!({"fundingRate": 0.0})),
                    (400, json!({"fundingRate": 0.0})),
                ],
            ),
            (
                ORACLE_INDEX_TOPIC,
                vec![
                    (100, json!({"index": 100.0})),
                    (300, json!({"index": 100.0})),
                ],
            ),
        ] {
            store
                .insert_backfill_batch(Network::Signet, topic, &events)
                .expect("backfill events");
        }
        store
            .insert(
                Network::Signet,
                STREAM_BUCKETS_TOPIC,
                100,
                EventSource::Stream,
                &json!({
                    "buckets": [{
                        "minSize": 0.0,
                        "maxSize": 1_000_000.0,
                        "bidPrice": 100.0,
                        "askPrice": 100.0
                    }]
                }),
            )
            .expect("liquidity");
        let collector = Collector::new(
            LnMarketsClient::public(Arc::new(NoopTransport), Network::Signet),
            store,
            CollectorConfig::public(Network::Signet),
        );
        let handle = collector.handle();
        handle
            .set_account_allocation(AccountAllocation {
                btc_sats: 100_000_000.0,
                synthetic_usd: 0.0,
                target_btc_weight: 0.5,
            })
            .expect("account allocation");
        handle
    }

    #[test]
    fn one_click_narrowing_only_reduces_mandate_authority() {
        let original = TradingMandate {
            venue: LNMARKETS_VENUE.into(),
            network: TradingNetwork::Signet,
            collateral_asset: trading_mandate::AssetId::sats(),
            objective: "Keep risk bounded".into(),
            max_venue_balance: 100_000,
            max_position_usd: 500,
            max_leverage: 5,
            daily_loss_stop: 5_000,
            max_orders_per_hour: 9,
            min_liquidation_buffer_bps: 1_500,
            allowed_strategies: BTreeSet::from(["funding_carry".into()]),
            review_cadence: ReviewCadence::Interval { seconds: 3_600 },
            expires_at_ms: 10_000,
        };
        let narrowed = narrowed_mandate(original.clone());

        assert_eq!(narrowed.max_venue_balance, 50_000);
        assert_eq!(narrowed.max_position_usd, 250);
        assert_eq!(narrowed.max_leverage, 2);
        assert_eq!(narrowed.daily_loss_stop, 2_500);
        assert_eq!(narrowed.max_orders_per_hour, 4);
        assert_eq!(
            narrowed.min_liquidation_buffer_bps,
            original.min_liquidation_buffer_bps
        );
        assert_eq!(narrowed.allowed_strategies, original.allowed_strategies);
        assert_eq!(narrowed.review_cadence, original.review_cadence);
        assert_eq!(narrowed.expires_at_ms, original.expires_at_ms);
    }

    #[test]
    fn collected_rebalance_backtest_is_durable_and_satisfies_the_start_gate() {
        let backtests = BacktestStore::in_memory().expect("backtests");
        let runtime = test_runtime(backtests.clone());
        let collector = replay_collector();
        let cost_model = BacktestCostModel {
            taker_fee_bps: 0,
            observed_round_trip_cost_bps: 0,
            measurement_source: "signet ledger sample".into(),
            measured_at_ms: 10,
        };
        let config = RebalanceToTargetConfig {
            network: Network::Signet,
            target_synthetic_usd_weight_bps: 5_000,
            drift_threshold_bps: 100,
            cost_margin_bps: 0,
            maximum_order_value_usd_cents: 5_000,
            liquidity_utilization_bps: 10_000,
            cost_measurement: RebalanceCostMeasurement {
                observed_round_trip_cost_bps: 0,
                traded_notional_sats: 100_000,
                realized_cost_sats: 0,
                sample_count: 1,
                measured_at_ms: 10,
                source: "signet ledger sample".into(),
            },
        };
        let recorded = runtime
            .run_rebalance_backtest(
                &collector,
                config.clone(),
                0,
                400,
                cost_model,
                BacktestPolicy {
                    minimum_trade_count: 1,
                    minimum_expectancy_millisats: 0,
                    maximum_drawdown_sats: 1,
                },
                500,
            )
            .expect("run backtest");

        assert!(recorded.report.passed());
        assert_eq!(recorded.report.trade_count, 1);
        assert_eq!(
            runtime.backtest_reports(None, 20).expect("reports").len(),
            1
        );
        let approval = backtests
            .require_passing(
                REBALANCE_STRATEGY_ID,
                "1",
                TradingNetwork::Signet,
                &serde_json::to_value(config).expect("config"),
            )
            .expect("backtest approval");
        assert_eq!(approval.report_digest, recorded.report_digest);
    }
}
