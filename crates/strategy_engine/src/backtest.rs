use std::{path::Path, sync::Arc};

use anyhow::{Context as _, Result, bail};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use trading_mandate::TradingNetwork;

use crate::{OrderIntent, StrategyProgram, StrategyTick};

pub const BACKTEST_SCHEMA: &str = "omega.strategy.backtest.v1";
const BASIS_POINTS_DENOMINATOR: u64 = 10_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BacktestCostModel {
    pub taker_fee_bps: u32,
    pub observed_round_trip_cost_bps: u32,
    pub measurement_source: String,
    pub measured_at_ms: i64,
}

impl BacktestCostModel {
    pub fn validate(&self) -> Result<()> {
        if self.taker_fee_bps > 10_000 {
            bail!("backtest taker fee must not exceed 10000 basis points");
        }
        if self.observed_round_trip_cost_bps > 10_000 {
            bail!("backtest round-trip cost must not exceed 10000 basis points");
        }
        if self.measurement_source.trim().is_empty() || self.measurement_source.len() > 500 {
            bail!("backtest cost measurement source must contain from 1 through 500 bytes");
        }
        if self.measured_at_ms < 0 {
            bail!("backtest cost measurement timestamp must not be negative");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BacktestPolicy {
    pub minimum_trade_count: u64,
    pub minimum_expectancy_millisats: i64,
    pub maximum_drawdown_sats: u64,
}

impl BacktestPolicy {
    pub fn validate(&self) -> Result<()> {
        if self.minimum_trade_count == 0 {
            bail!("backtest minimum trade count must be greater than zero");
        }
        if self.maximum_drawdown_sats == 0 {
            bail!("backtest maximum drawdown must be greater than zero");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BacktestOutcome {
    Passed,
    Failed { reasons: Vec<String> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BacktestReport {
    pub schema: String,
    pub strategy_id: String,
    pub strategy_version: String,
    pub network: TradingNetwork,
    pub parameter_digest: String,
    pub data_digest: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub created_at_ms: i64,
    pub tick_count: u64,
    pub trade_count: u64,
    pub gross_profit_sats: i64,
    pub taker_fees_sats: u64,
    pub round_trip_cost_sats: u64,
    pub funding_sats: i64,
    pub net_profit_sats: i64,
    pub expectancy_millisats: i64,
    pub maximum_drawdown_sats: u64,
    pub cost_model: BacktestCostModel,
    pub policy: BacktestPolicy,
    pub outcome: BacktestOutcome,
}

impl BacktestReport {
    pub fn report_digest(&self) -> Result<String> {
        digest_json(self)
    }

    pub fn passed(&self) -> bool {
        matches!(self.outcome, BacktestOutcome::Passed)
    }

    fn validate(&self) -> Result<()> {
        if self.schema != BACKTEST_SCHEMA {
            bail!("unsupported backtest report schema {:?}", self.schema);
        }
        validate_identifier("strategy ID", &self.strategy_id)?;
        validate_identifier("strategy version", &self.strategy_version)?;
        validate_digest("parameter digest", &self.parameter_digest)?;
        validate_digest("data digest", &self.data_digest)?;
        if self.from_ms < 0 || self.to_ms < self.from_ms || self.created_at_ms < 0 {
            bail!("backtest report timestamps are invalid");
        }
        if self.tick_count == 0 {
            bail!("backtest report must include at least one tick");
        }
        self.cost_model.validate()?;
        self.policy.validate()?;
        let expected_net = self
            .gross_profit_sats
            .checked_add(self.funding_sats)
            .and_then(|value| value.checked_sub(i64::try_from(self.taker_fees_sats).ok()?))
            .and_then(|value| value.checked_sub(i64::try_from(self.round_trip_cost_sats).ok()?))
            .context("backtest report totals overflowed")?;
        if self.net_profit_sats != expected_net {
            bail!("backtest net profit does not match its cost components");
        }
        let expected_expectancy = if self.trade_count == 0 {
            0
        } else {
            self.net_profit_sats
                .checked_mul(1_000)
                .and_then(|value| value.checked_div(i64::try_from(self.trade_count).ok()?))
                .context("backtest report expectancy overflowed")?
        };
        if self.expectancy_millisats != expected_expectancy {
            bail!("backtest expectancy does not match net profit and trade count");
        }
        let expected_reasons = gate_reasons(
            self.trade_count,
            self.expectancy_millisats,
            self.maximum_drawdown_sats,
            &self.policy,
        );
        match (&self.outcome, expected_reasons.is_empty()) {
            (BacktestOutcome::Passed, true) => {}
            (BacktestOutcome::Failed { reasons }, false) if *reasons == expected_reasons => {}
            _ => bail!("backtest outcome does not match its measured gate values"),
        }
        if let BacktestOutcome::Failed { reasons } = &self.outcome
            && (reasons.is_empty() || reasons.iter().any(|reason| reason.trim().is_empty()))
        {
            bail!("failed backtest report must name each failed gate");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BacktestTick<Features> {
    pub occurred_at_ms: i64,
    pub features: Features,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulatedTrade {
    pub gross_profit_sats: i64,
    pub notional_sats: u64,
    pub funding_sats: i64,
}

pub trait BacktestExecutionModel<Features> {
    fn execute(
        &mut self,
        intent: &OrderIntent,
        tick: &BacktestTick<Features>,
    ) -> Result<SimulatedTrade>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BacktestApproval {
    pub report_digest: String,
    pub created_at_ms: i64,
}

pub trait BacktestGate: Send + Sync + 'static {
    fn require_passing(
        &self,
        strategy_id: &str,
        strategy_version: &str,
        network: TradingNetwork,
        config: &Value,
    ) -> Result<BacktestApproval>;
}

pub fn parameter_digest(
    strategy_id: &str,
    strategy_version: &str,
    config: &Value,
) -> Result<String> {
    validate_identifier("strategy ID", strategy_id)?;
    validate_identifier("strategy version", strategy_version)?;
    #[derive(Serialize)]
    struct ParameterSet<'a> {
        strategy_id: &'a str,
        strategy_version: &'a str,
        config: &'a Value,
    }
    digest_json(&ParameterSet {
        strategy_id,
        strategy_version,
        config,
    })
}

pub fn run_backtest<Program, Model>(
    program: &Program,
    config: &Program::Config,
    ticks: &[BacktestTick<Program::Features>],
    model: &mut Model,
    network: TradingNetwork,
    cost_model: BacktestCostModel,
    policy: BacktestPolicy,
    created_at_ms: i64,
) -> Result<BacktestReport>
where
    Program: StrategyProgram,
    Program::Features: Serialize,
    Model: BacktestExecutionModel<Program::Features>,
{
    if ticks.is_empty() {
        bail!("backtest requires collected market data");
    }
    if ticks
        .windows(2)
        .any(|window| window[1].occurred_at_ms < window[0].occurred_at_ms)
    {
        bail!("backtest ticks must be chronological");
    }
    if created_at_ms < 0 {
        bail!("backtest creation timestamp must not be negative");
    }
    cost_model.validate()?;
    policy.validate()?;
    program.validate_config(config)?;
    let config_json = serde_json::to_value(config)?;
    let parameter_digest = parameter_digest(
        program.strategy_id(),
        program.strategy_version(),
        &config_json,
    )?;
    let data_digest = digest_json(ticks)?;
    let mut state = program.initial_state(config)?;
    let mut gross_profit_sats = 0_i64;
    let mut taker_fees_sats = 0_u64;
    let mut round_trip_cost_sats = 0_u64;
    let mut funding_sats = 0_i64;
    let mut net_profit_sats = 0_i64;
    let mut peak_profit_sats = 0_i64;
    let mut maximum_drawdown_sats = 0_u64;
    let mut trade_count = 0_u64;

    for tick in ticks {
        if tick.occurred_at_ms < 0 {
            bail!("backtest tick timestamp must not be negative");
        }
        let live_tick = StrategyTick {
            occurred_at_ms: tick.occurred_at_ms,
            features: tick.features.clone(),
        };
        let step = program.on_tick(config, &state, &live_tick)?;
        for intent in &step.intents {
            intent.validate()?;
            let trade = model.execute(intent, tick)?;
            let taker_fee = basis_point_cost(trade.notional_sats, cost_model.taker_fee_bps)?;
            let round_trip_cost =
                basis_point_cost(trade.notional_sats, cost_model.observed_round_trip_cost_bps)?;
            gross_profit_sats = gross_profit_sats
                .checked_add(trade.gross_profit_sats)
                .context("backtest gross profit overflowed")?;
            taker_fees_sats = taker_fees_sats
                .checked_add(taker_fee)
                .context("backtest taker fees overflowed")?;
            round_trip_cost_sats = round_trip_cost_sats
                .checked_add(round_trip_cost)
                .context("backtest round-trip costs overflowed")?;
            funding_sats = funding_sats
                .checked_add(trade.funding_sats)
                .context("backtest funding overflowed")?;
            let net_change = trade
                .gross_profit_sats
                .checked_add(trade.funding_sats)
                .and_then(|value| value.checked_sub(i64::try_from(taker_fee).ok()?))
                .and_then(|value| value.checked_sub(i64::try_from(round_trip_cost).ok()?))
                .context("backtest net trade result overflowed")?;
            net_profit_sats = net_profit_sats
                .checked_add(net_change)
                .context("backtest net profit overflowed")?;
            peak_profit_sats = peak_profit_sats.max(net_profit_sats);
            let drawdown = peak_profit_sats.saturating_sub(net_profit_sats);
            maximum_drawdown_sats = maximum_drawdown_sats.max(
                u64::try_from(drawdown).context("backtest drawdown exceeded supported range")?,
            );
            trade_count = trade_count
                .checked_add(1)
                .context("backtest trade count overflowed")?;
        }
        state = step.next_state;
    }

    let expectancy_millisats = if trade_count == 0 {
        0
    } else {
        net_profit_sats
            .checked_mul(1_000)
            .and_then(|value| value.checked_div(i64::try_from(trade_count).ok()?))
            .context("backtest expectancy overflowed")?
    };
    let reasons = gate_reasons(
        trade_count,
        expectancy_millisats,
        maximum_drawdown_sats,
        &policy,
    );
    let outcome = if reasons.is_empty() {
        BacktestOutcome::Passed
    } else {
        BacktestOutcome::Failed { reasons }
    };
    let report = BacktestReport {
        schema: BACKTEST_SCHEMA.into(),
        strategy_id: program.strategy_id().into(),
        strategy_version: program.strategy_version().into(),
        network,
        parameter_digest,
        data_digest,
        from_ms: ticks
            .first()
            .context("missing first backtest tick")?
            .occurred_at_ms,
        to_ms: ticks
            .last()
            .context("missing last backtest tick")?
            .occurred_at_ms,
        created_at_ms,
        tick_count: u64::try_from(ticks.len()).context("backtest tick count overflowed")?,
        trade_count,
        gross_profit_sats,
        taker_fees_sats,
        round_trip_cost_sats,
        funding_sats,
        net_profit_sats,
        expectancy_millisats,
        maximum_drawdown_sats,
        cost_model,
        policy,
        outcome,
    };
    report.validate()?;
    Ok(report)
}

#[derive(Clone)]
pub struct BacktestStore {
    connection: Arc<Mutex<Connection>>,
}

impl BacktestStore {
    pub fn default_path() -> std::path::PathBuf {
        paths::data_dir()
            .join("threads")
            .join("strategy-backtests.db")
    }

    pub fn open_default() -> Result<Self> {
        Self::open(&Self::default_path())
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create backtest directory {parent:?}"))?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS strategy_backtest_reports (
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                 report_digest TEXT NOT NULL UNIQUE,
                 strategy_id TEXT NOT NULL,
                 strategy_version TEXT NOT NULL,
                 network TEXT NOT NULL,
                 parameter_digest TEXT NOT NULL,
                 outcome TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 report_json TEXT NOT NULL
             ) STRICT;
             CREATE INDEX IF NOT EXISTS idx_strategy_backtest_gate
                 ON strategy_backtest_reports(
                     strategy_id, strategy_version, network, parameter_digest, sequence DESC
                 );
             CREATE TRIGGER IF NOT EXISTS strategy_backtest_no_update
             BEFORE UPDATE ON strategy_backtest_reports
             BEGIN SELECT RAISE(ABORT, 'backtest reports are append-only'); END;
             CREATE TRIGGER IF NOT EXISTS strategy_backtest_no_delete
             BEFORE DELETE ON strategy_backtest_reports
             BEGIN SELECT RAISE(ABORT, 'backtest reports are append-only'); END;",
        )?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn record(&self, report: &BacktestReport) -> Result<String> {
        report.validate()?;
        let report_digest = report.report_digest()?;
        let report_json = serde_json::to_string(report)?;
        self.connection.lock().execute(
            "INSERT INTO strategy_backtest_reports (
                 report_digest, strategy_id, strategy_version, network,
                 parameter_digest, outcome, created_at_ms, report_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                report_digest,
                report.strategy_id,
                report.strategy_version,
                network_name(report.network),
                report.parameter_digest,
                if report.passed() { "passed" } else { "failed" },
                report.created_at_ms,
                report_json,
            ],
        )?;
        Ok(report_digest)
    }

    pub fn latest(
        &self,
        strategy_id: &str,
        strategy_version: &str,
        network: TradingNetwork,
        parameter_digest: &str,
    ) -> Result<Option<BacktestReport>> {
        validate_identifier("strategy ID", strategy_id)?;
        validate_identifier("strategy version", strategy_version)?;
        validate_digest("parameter digest", parameter_digest)?;
        let row = self
            .connection
            .lock()
            .query_row(
                "SELECT report_digest, report_json
                 FROM strategy_backtest_reports
                 WHERE strategy_id = ?1 AND strategy_version = ?2
                   AND network = ?3 AND parameter_digest = ?4
                 ORDER BY sequence DESC LIMIT 1",
                params![
                    strategy_id,
                    strategy_version,
                    network_name(network),
                    parameter_digest,
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        row.map(|(stored_digest, report_json)| decode_stored_report(&stored_digest, &report_json))
            .transpose()
    }

    pub fn reports(&self, strategy_id: Option<&str>, limit: usize) -> Result<Vec<BacktestReport>> {
        if let Some(strategy_id) = strategy_id {
            validate_identifier("strategy ID", strategy_id)?;
        }
        if !(1..=500).contains(&limit) {
            bail!("backtest report history limit must be from 1 through 500");
        }
        let limit = i64::try_from(limit).context("backtest report history limit overflowed")?;
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT report_digest, report_json
             FROM strategy_backtest_reports
             WHERE (?1 IS NULL OR strategy_id = ?1)
             ORDER BY sequence DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![strategy_id, limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (stored_digest, report_json) = row?;
            decode_stored_report(&stored_digest, &report_json)
        })
        .collect()
    }
}

impl BacktestGate for BacktestStore {
    fn require_passing(
        &self,
        strategy_id: &str,
        strategy_version: &str,
        network: TradingNetwork,
        config: &Value,
    ) -> Result<BacktestApproval> {
        let parameter_digest = parameter_digest(strategy_id, strategy_version, config)?;
        let report = self
            .latest(strategy_id, strategy_version, network, &parameter_digest)?
            .with_context(|| {
                format!(
                    "strategy {strategy_id} version {strategy_version} has no backtest artifact for this parameter set on {}",
                    network_name(network)
                )
            })?;
        if !report.passed() {
            bail!(
                "latest backtest for strategy {strategy_id} version {strategy_version} did not pass"
            );
        }
        Ok(BacktestApproval {
            report_digest: report.report_digest()?,
            created_at_ms: report.created_at_ms,
        })
    }
}

fn basis_point_cost(notional_sats: u64, basis_points: u32) -> Result<u64> {
    let numerator = notional_sats
        .checked_mul(u64::from(basis_points))
        .context("backtest cost overflowed")?;
    numerator
        .checked_add(BASIS_POINTS_DENOMINATOR - 1)
        .map(|value| value / BASIS_POINTS_DENOMINATOR)
        .context("backtest cost rounding overflowed")
}

fn gate_reasons(
    trade_count: u64,
    expectancy_millisats: i64,
    maximum_drawdown_sats: u64,
    policy: &BacktestPolicy,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if trade_count < policy.minimum_trade_count {
        reasons.push(format!(
            "trade count {trade_count} is below minimum {}",
            policy.minimum_trade_count
        ));
    }
    if expectancy_millisats < policy.minimum_expectancy_millisats {
        reasons.push(format!(
            "expectancy {expectancy_millisats} millisats is below minimum {}",
            policy.minimum_expectancy_millisats
        ));
    }
    if maximum_drawdown_sats > policy.maximum_drawdown_sats {
        reasons.push(format!(
            "maximum drawdown {maximum_drawdown_sats} sats exceeds limit {}",
            policy.maximum_drawdown_sats
        ));
    }
    reasons
}

fn digest_json(value: &(impl Serialize + ?Sized)) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn decode_stored_report(stored_digest: &str, report_json: &str) -> Result<BacktestReport> {
    let report: BacktestReport = serde_json::from_str(report_json)?;
    report.validate()?;
    let computed_digest = report.report_digest()?;
    if stored_digest != computed_digest {
        bail!("backtest report digest does not match its stored artifact");
    }
    Ok(report)
}

fn validate_identifier(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 200 {
        bail!("backtest {label} must contain from 1 through 200 bytes");
    }
    Ok(())
}

fn validate_digest(label: &str, value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("backtest {label} must be a 64-character hexadecimal value");
    }
    Ok(())
}

fn network_name(network: TradingNetwork) -> &'static str {
    match network {
        TradingNetwork::Signet => "signet",
        TradingNetwork::Mainnet => "mainnet",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{OrderKind, OrderQuantity, OrderSide, QuantityUnit, StrategyStep, VenueProtection};

    #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
    struct TestConfig {
        trade: bool,
    }

    #[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
    struct TestState {
        ticks: u64,
    }

    #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
    struct TestFeatures {
        gross_profit_sats: i64,
        funding_sats: i64,
    }

    struct TestProgram;

    impl StrategyProgram for TestProgram {
        type Config = TestConfig;
        type State = TestState;
        type Features = TestFeatures;

        fn strategy_id(&self) -> &'static str {
            "backtest_strategy"
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
            let next_state = TestState {
                ticks: state.ticks.checked_add(1).context("test tick overflow")?,
            };
            let intents = config
                .trade
                .then(|| OrderIntent {
                    intent_id: format!("intent-{}", tick.occurred_at_ms),
                    instrument: "btc_usd".into(),
                    side: OrderSide::Buy,
                    kind: OrderKind::Market,
                    quantity: OrderQuantity {
                        amount: 10_000,
                        unit: QuantityUnit::Sats,
                    },
                    limit_price: None,
                    reduce_only: false,
                    protection: Some(VenueProtection {
                        stop_loss_price: 90.0,
                        take_profit_price: Some(110.0),
                    }),
                    metadata: json!({}),
                })
                .into_iter()
                .collect();
            Ok(StrategyStep {
                next_state,
                intents,
            })
        }
    }

    struct TestModel;

    impl BacktestExecutionModel<TestFeatures> for TestModel {
        fn execute(
            &mut self,
            _intent: &OrderIntent,
            tick: &BacktestTick<TestFeatures>,
        ) -> Result<SimulatedTrade> {
            Ok(SimulatedTrade {
                gross_profit_sats: tick.features.gross_profit_sats,
                notional_sats: 10_000,
                funding_sats: tick.features.funding_sats,
            })
        }
    }

    fn ticks() -> Vec<BacktestTick<TestFeatures>> {
        [(100, 100, 5), (200, -50, -5), (300, 100, 5)]
            .into_iter()
            .map(
                |(occurred_at_ms, gross_profit_sats, funding_sats)| BacktestTick {
                    occurred_at_ms,
                    features: TestFeatures {
                        gross_profit_sats,
                        funding_sats,
                    },
                },
            )
            .collect()
    }

    fn cost_model() -> BacktestCostModel {
        BacktestCostModel {
            taker_fee_bps: 10,
            observed_round_trip_cost_bps: 5,
            measurement_source: "signet ledger sample".into(),
            measured_at_ms: 50,
        }
    }

    fn policy(minimum_expectancy_millisats: i64) -> BacktestPolicy {
        BacktestPolicy {
            minimum_trade_count: 3,
            minimum_expectancy_millisats,
            maximum_drawdown_sats: 100,
        }
    }

    #[test]
    fn replay_uses_strategy_code_and_measured_integer_costs() {
        let report = run_backtest(
            &TestProgram,
            &TestConfig { trade: true },
            &ticks(),
            &mut TestModel,
            TradingNetwork::Signet,
            cost_model(),
            policy(30_000),
            400,
        )
        .expect("report");

        assert_eq!(report.trade_count, 3);
        assert_eq!(report.gross_profit_sats, 150);
        assert_eq!(report.taker_fees_sats, 30);
        assert_eq!(report.round_trip_cost_sats, 15);
        assert_eq!(report.funding_sats, 5);
        assert_eq!(report.net_profit_sats, 110);
        assert_eq!(report.expectancy_millisats, 36_666);
        assert_eq!(report.maximum_drawdown_sats, 70);
        assert_eq!(report.outcome, BacktestOutcome::Passed);
    }

    #[test]
    fn no_collected_data_produces_no_artifact() {
        let error = run_backtest(
            &TestProgram,
            &TestConfig { trade: true },
            &[],
            &mut TestModel,
            TradingNetwork::Signet,
            cost_model(),
            policy(0),
            1,
        )
        .expect_err("missing data");
        assert!(error.to_string().contains("requires collected market data"));
    }

    #[test]
    fn latest_exact_parameter_artifact_controls_the_live_gate() {
        let store = BacktestStore::in_memory().expect("store");
        let config = TestConfig { trade: true };
        let config_json = serde_json::to_value(&config).expect("config JSON");
        let passing = run_backtest(
            &TestProgram,
            &config,
            &ticks(),
            &mut TestModel,
            TradingNetwork::Signet,
            cost_model(),
            policy(30_000),
            400,
        )
        .expect("passing report");
        let passing_digest = store.record(&passing).expect("record passing");
        assert_eq!(
            store
                .require_passing(
                    TestProgram.strategy_id(),
                    TestProgram.strategy_version(),
                    TradingNetwork::Signet,
                    &config_json,
                )
                .expect("passing gate")
                .report_digest,
            passing_digest
        );

        let failed = run_backtest(
            &TestProgram,
            &config,
            &ticks(),
            &mut TestModel,
            TradingNetwork::Signet,
            cost_model(),
            policy(40_000),
            500,
        )
        .expect("failed report");
        assert!(matches!(failed.outcome, BacktestOutcome::Failed { .. }));
        store.record(&failed).expect("record failed");
        let reports = store
            .reports(Some(TestProgram.strategy_id()), 10)
            .expect("report history");
        assert_eq!(reports, vec![failed, passing]);
        assert!(
            store
                .require_passing(
                    TestProgram.strategy_id(),
                    TestProgram.strategy_version(),
                    TradingNetwork::Signet,
                    &config_json,
                )
                .expect_err("latest failure wins")
                .to_string()
                .contains("did not pass")
        );

        let different_config =
            serde_json::to_value(TestConfig { trade: false }).expect("different config JSON");
        assert!(
            store
                .require_passing(
                    TestProgram.strategy_id(),
                    TestProgram.strategy_version(),
                    TradingNetwork::Signet,
                    &different_config,
                )
                .expect_err("exact parameter gate")
                .to_string()
                .contains("no backtest artifact")
        );
    }

    #[test]
    fn stored_reports_are_append_only_and_digest_checked() {
        let store = BacktestStore::in_memory().expect("store");
        let report = run_backtest(
            &TestProgram,
            &TestConfig { trade: true },
            &ticks(),
            &mut TestModel,
            TradingNetwork::Signet,
            cost_model(),
            policy(30_000),
            400,
        )
        .expect("report");
        store.record(&report).expect("record");

        let update = store.connection.lock().execute(
            "UPDATE strategy_backtest_reports SET outcome = 'failed'",
            [],
        );
        assert!(update.is_err());
        let delete = store
            .connection
            .lock()
            .execute("DELETE FROM strategy_backtest_reports", []);
        assert!(delete.is_err());
    }
}
