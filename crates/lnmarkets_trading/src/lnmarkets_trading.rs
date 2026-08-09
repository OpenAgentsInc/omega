mod funding_carry;
mod rebalance_to_target;
mod threshold_swing;

pub use funding_carry::{
    FUNDING_CARRY_SCHEMA, FundingCarryAction, FundingCarryBacktestModel, FundingCarryConfig,
    FundingCarryExecutor, FundingCarryFeatures, FundingCarryInstrument, FundingCarryPosition,
    FundingCarryProgram, FundingCarryState, FundingFeeSyncReport, funding_carry_features,
    sync_funding_fees,
};
pub use rebalance_to_target::{
    REBALANCE_TO_TARGET_SCHEMA, RebalanceAction, RebalanceBacktestModel, RebalanceCostMeasurement,
    RebalanceToTargetConfig, RebalanceToTargetProgram, RebalanceToTargetState,
    SyntheticUsdExecutor, measure_rebalance_cost,
};
pub use threshold_swing::{
    THRESHOLD_SWING_SCHEMA, ThresholdSwingAction, ThresholdSwingBacktestModel,
    ThresholdSwingConfig, ThresholdSwingExecutor, ThresholdSwingPosition, ThresholdSwingProgram,
    ThresholdSwingState, ThresholdSwingWindow,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TradingRegistration;

pub const REGISTRATION: TradingRegistration = TradingRegistration;

pub use strategy_engine::{
    BACKTEST_SCHEMA, BacktestApproval, BacktestCostModel, BacktestExecutionModel, BacktestGate,
    BacktestOutcome, BacktestPolicy, BacktestReport, BacktestStore, BacktestTick, CancelIntent,
    CancelOutcome, LifecycleSink, MandateAuthority, MemoryLifecycleSink, MemoryWakeupSink,
    OpenOrder, OrderIntent, SimulatedSettlement, SimulatedTrade, StrategyCommand, StrategyEngine,
    StrategyHaltReason, StrategyLifecycleEvent, StrategyProgram, StrategyServiceHandle,
    StrategyStatus, StrategyStep, StrategyTick, VenueExecution, VenueExecutor, VenueRiskSnapshot,
    WakeupSink, background_service, parameter_digest, run_backtest,
};

pub type LnMarketsStrategyTick = StrategyTick<lnmarkets_data::FeatureSnapshot>;
pub type LnMarketsBacktestTick = BacktestTick<lnmarkets_data::FeatureSnapshot>;

#[derive(Clone, Debug, PartialEq)]
pub struct LnMarketsBacktestReplay {
    pub schema: String,
    pub network: lnmarkets_client::Network,
    pub candle_count: u64,
    pub oracle_index_count: u64,
    pub funding_settlement_count: u64,
    pub ticks: Vec<LnMarketsBacktestTick>,
}

pub fn collected_backtest_replay(
    store: &lnmarkets_data::MarketDataStore,
    network: lnmarkets_client::Network,
    from_ms: i64,
    to_ms: i64,
) -> anyhow::Result<LnMarketsBacktestReplay> {
    let replay = store.feature_replay(network, from_ms, to_ms)?;
    Ok(LnMarketsBacktestReplay {
        schema: replay.schema,
        network: replay.network,
        candle_count: replay.candle_count,
        oracle_index_count: replay.oracle_index_count,
        funding_settlement_count: replay.funding_settlement_count,
        ticks: replay
            .ticks
            .into_iter()
            .map(|tick| BacktestTick {
                occurred_at_ms: tick.occurred_at_ms,
                features: tick.features,
            })
            .collect(),
    })
}

pub trait LnMarketsStrategy: StrategyProgram<Features = lnmarkets_data::FeatureSnapshot> {}

impl<Program> LnMarketsStrategy for Program where
    Program: StrategyProgram<Features = lnmarkets_data::FeatureSnapshot>
{
}
