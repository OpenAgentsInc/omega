use std::{collections::VecDeque, future::Future, sync::Arc};

use anyhow::{Context as _, Result, bail};
use gpui::{AppContext as _, AsyncApp, Task};
use lnmarkets_client::{LnMarketsClient, Network};
use lnmarkets_data::FeatureSnapshot;
use lnmarkets_trading::{
    BacktestStore, FundingCarryConfig, FundingCarryExecutor, FundingCarryFeatures,
    FundingCarryProgram, MemoryLifecycleSink, MemoryWakeupSink, RebalanceToTargetConfig,
    RebalanceToTargetProgram, StrategyCommand, StrategyEngine, StrategyLifecycleEvent,
    StrategyServiceHandle, SyntheticUsdExecutor, background_service,
};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use trading_ledger::{LedgerQuery, LedgerStore, ProfitReport};
use trading_mandate::{
    MandateRevision, MandateSnapshot, MandateStore, ReviewCadence, TradingNetwork,
};

use crate::review_turn::{PortfolioReview, hourly_start};

const REBALANCE_STRATEGY_ID: &str = "rebalance_to_target";
const FUNDING_STRATEGY_ID: &str = "funding_carry";

type RebalanceHandle = StrategyServiceHandle<RebalanceToTargetConfig, FeatureSnapshot>;
type FundingHandle = StrategyServiceHandle<FundingCarryConfig, FundingCarryFeatures>;

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
    review_session_id: Mutex<Option<String>>,
    rebalance: Mutex<Option<RebalanceHandle>>,
    funding: Mutex<Option<FundingHandle>>,
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
            review_session_id: Mutex::new(None),
            rebalance: Mutex::new(None),
            funding: Mutex::new(None),
        })
    }

    pub fn profit_report(&self, query: &LedgerQuery) -> Result<ProfitReport> {
        self.ledger.profit_report(query)
    }

    pub fn ledger_entries(&self, query: &LedgerQuery) -> Result<Vec<trading_ledger::LedgerEntry>> {
        self.ledger.entries(query)
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
            .mandate
            .context("no active mandate is available to narrow")?;
        let proposal = self.mandate.propose(narrowed_mandate(mandate))?;
        self.mandate.save_restriction(proposal, changed_at_ms)
    }

    pub fn revoke_mandate(&self, changed_at_ms: i64) -> Result<MandateSnapshot> {
        self.mandate.revoke(changed_at_ms)
    }

    pub fn strategy_snapshots(&self) -> Vec<StrategyRuntimeSnapshot> {
        [REBALANCE_STRATEGY_ID, FUNDING_STRATEGY_ID]
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
            .mandate
            .map(|mandate| mandate.review_cadence))
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
        handle
            .request(StrategyCommand::Start { config, at_ms })
            .await
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
        handle.request(StrategyCommand::Adjust { config }).await
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
        let mandate = snapshot.mandate.context(
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
    mandate.max_venue_balance_sats = (mandate.max_venue_balance_sats / 2).max(1);
    mandate.max_position_usd = (mandate.max_position_usd / 2).max(1);
    mandate.max_leverage = (mandate.max_leverage / 2).max(1);
    mandate.daily_loss_stop_sats = (mandate.daily_loss_stop_sats / 2).max(1);
    mandate.max_orders_per_hour /= 2;
    mandate
}

fn require_signet(client: &LnMarketsClient, network: Network) -> Result<()> {
    if network != Network::Signet || client.network() != Network::Signet {
        bail!("automated LN Markets strategies are restricted to signet");
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
    use std::collections::BTreeSet;

    use trading_mandate::{ReviewCadence, TradingMandate, TradingNetwork};

    use super::narrowed_mandate;

    #[test]
    fn one_click_narrowing_only_reduces_mandate_authority() {
        let original = TradingMandate {
            network: TradingNetwork::Signet,
            objective: "Keep risk bounded".into(),
            max_venue_balance_sats: 100_000,
            max_position_usd: 500,
            max_leverage: 5,
            daily_loss_stop_sats: 5_000,
            max_orders_per_hour: 9,
            min_liquidation_buffer_bps: 1_500,
            allowed_strategies: BTreeSet::from(["funding_carry".into()]),
            review_cadence: ReviewCadence::Interval { seconds: 3_600 },
            expires_at_ms: 10_000,
        };
        let narrowed = narrowed_mandate(original.clone());

        assert_eq!(narrowed.max_venue_balance_sats, 50_000);
        assert_eq!(narrowed.max_position_usd, 250);
        assert_eq!(narrowed.max_leverage, 2);
        assert_eq!(narrowed.daily_loss_stop_sats, 2_500);
        assert_eq!(narrowed.max_orders_per_hour, 4);
        assert_eq!(
            narrowed.min_liquidation_buffer_bps,
            original.min_liquidation_buffer_bps
        );
        assert_eq!(narrowed.allowed_strategies, original.allowed_strategies);
        assert_eq!(narrowed.review_cadence, original.review_cadence);
        assert_eq!(narrowed.expires_at_ms, original.expires_at_ms);
    }
}
