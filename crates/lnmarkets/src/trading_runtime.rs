use std::{future::Future, sync::Arc};

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
use trading_mandate::{MandateRevision, MandateSnapshot, MandateStore, TradingNetwork};

const REBALANCE_STRATEGY_ID: &str = "rebalance_to_target";
const FUNDING_STRATEGY_ID: &str = "funding_carry";

type RebalanceHandle = StrategyServiceHandle<RebalanceToTargetConfig, FeatureSnapshot>;
type FundingHandle = StrategyServiceHandle<FundingCarryConfig, FundingCarryFeatures>;

#[derive(Clone, Debug, Serialize)]
pub struct StrategyRuntimeSnapshot {
    pub strategy_id: String,
    pub status: String,
    pub started_at_ms: Option<i64>,
    pub halted_at_ms: Option<i64>,
    pub halt_reason: Option<Value>,
    pub state: Option<Value>,
    pub lifecycle_event_count: usize,
}

pub struct TradingRuntime {
    ledger: LedgerStore,
    mandate: MandateStore,
    backtests: BacktestStore,
    lifecycle: MemoryLifecycleSink,
    wakeups: MemoryWakeupSink,
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

    pub fn strategy_snapshots(&self) -> Vec<StrategyRuntimeSnapshot> {
        [REBALANCE_STRATEGY_ID, FUNDING_STRATEGY_ID]
            .into_iter()
            .map(|strategy_id| self.strategy_snapshot(strategy_id))
            .collect()
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
