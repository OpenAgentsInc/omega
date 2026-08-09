#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TradingRegistration;

pub const REGISTRATION: TradingRegistration = TradingRegistration;

pub use strategy_engine::{
    LifecycleSink, MandateAuthority, OrderIntent, StrategyCommand, StrategyEngine,
    StrategyHaltReason, StrategyLifecycleEvent, StrategyProgram, StrategyServiceHandle,
    StrategyStatus, StrategyStep, StrategyTick, VenueExecution, VenueExecutor, VenueRiskSnapshot,
    WakeupSink, background_service,
};

pub type LnMarketsStrategyTick = StrategyTick<lnmarkets_data::FeatureSnapshot>;

pub trait LnMarketsStrategy: StrategyProgram<Features = lnmarkets_data::FeatureSnapshot> {}

impl<Program> LnMarketsStrategy for Program where
    Program: StrategyProgram<Features = lnmarkets_data::FeatureSnapshot>
{
}
