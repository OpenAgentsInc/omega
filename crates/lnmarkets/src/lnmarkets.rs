use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context as _;
use futures::{AsyncReadExt as _, FutureExt as _, future::BoxFuture};
use gpui::{App, AppContext as _, Global, Task};
use http_client::{AsyncBody, HttpClient};
use lnmarkets_data::{Collector, CollectorConfig, CollectorHandle, MarketDataStore};
use parking_lot::Mutex;
use plugin_api::{
    BackgroundServiceRegistration, CardSchemaRegistration, HostDeclaration, Maturity,
    ObservedVenueMode, PluginManifest, PluginRegistry, ProbedVenueAssumption, Protocol,
    SettingsPageRegistration, VenueAccountMode, VenueActionCapability, VenueActionClass,
    VenueActionStatus, VenueCapabilities, VenueCapabilityStore, VenueMarginMode,
};

mod agent_tools;
mod counterparty_exposure;
mod normalized_carry;
mod prediction_resolution;
mod review_driver;
mod review_turn;
mod signet_soak;
mod trading_runtime;

pub use agent_tools::*;
pub use agent_wakeup::WakeupSource;
pub use lnmarkets_ui::{
    LnMarketsOperatorPanel, OperatorBacktestSnapshot, OperatorConsoleSnapshot,
    OperatorConsoleSource, OperatorReviewTurn, OperatorStrategySnapshot,
};
pub use review_driver::LnMarketsReviewDriver;
pub use review_turn::{PORTFOLIO_REVIEW_SCHEMA, PORTFOLIO_REVIEW_TOKEN_BUDGET, PortfolioReview};
pub use signet_soak::{
    SIGNET_SOAK_SCHEMA, SignetSoakEvidence, SignetSoakReceipt, SignetSoakRefusal, SignetSoakStatus,
    SoakBudget, SoakLimitBreach, SoakReconciliationSample, SoakReviewTurn, SoakStrategyObservation,
    SoakWindow,
};
pub use trading_runtime::{
    RecordedBacktest, ReviewTurnHistory, ReviewTurnOutcome, StrategyRuntimeSnapshot, TradingRuntime,
};

pub use lnmarkets_client::*;
pub use lnmarkets_data::{
    AccountAllocation, AccountDriftFeatures, CANDLE_TOPIC, CollectorHealth, CollectorHistory,
    CollectorStatus, FUNDING_SETTLEMENT_TOPIC, FeatureSnapshot, FundingFeatures, FundingSign,
    IndexFeatures, LiquidityFeatures, ORACLE_INDEX_TOPIC, StoredMarketEvent, VolatilityFeatures,
};
pub use lnmarkets_trading::{
    BacktestCostModel, BacktestOutcome, BacktestPolicy, BacktestReport, FundingCarryConfig,
    FundingCarryInstrument, RebalanceCostMeasurement, RebalanceToTargetConfig,
    StrategyLifecycleEvent, ThresholdSwingAction, ThresholdSwingBacktestModel,
    ThresholdSwingConfig, ThresholdSwingPosition, ThresholdSwingState, ThresholdSwingWindow,
};
pub use lnmarkets_ui::LnMarketsSettingsPage;
pub use normalized_carry::LN_MARKETS_FUNDING_SETTLEMENT_INTERVAL_MS;
pub use trading_ledger::{
    AssetId, Counterparty, CounterpartyExposure, CounterpartyExposureDivergence,
    CounterpartySnapshot, LedgerEntry, LedgerQuery, LedgerStore, ProfitReport,
};
pub use trading_mandate::{
    MandateDecision, MandateRefusal, MandateRevision, MandateSnapshot, MandateStore, ReviewCadence,
    TradingInstruction, TradingMandate, TradingNetwork,
};

const MAX_TRANSPORT_RESPONSE_BYTES: u64 = 1_048_577;
pub(crate) const CAPABILITY_MAX_AGE_MS: i64 = 15_000;

pub const MANIFEST: PluginManifest = PluginManifest {
    id: "lnmarkets",
    name: "LN Markets",
    version: env!("CARGO_PKG_VERSION"),
    // Automated strategies remain signet-only; manual account and swap tools
    // may address mainnet.
    maturity: Maturity::Signet,
    hosts: &[
        HostDeclaration {
            host: "api.signet.lnmarkets.com",
            purpose: "LN Markets plugin v3 signet account, market-data, and trading requests",
            protocols: &[Protocol::Https],
        },
        HostDeclaration {
            host: "api.lnmarkets.com",
            purpose: "LN Markets plugin v3 mainnet account, market-data, and explicitly requested trading calls",
            protocols: &[Protocol::Https],
        },
        HostDeclaration {
            host: "stream.signet.lnmarkets.com",
            purpose: "LN Markets plugin v1 signet market and account streams",
            protocols: &[Protocol::Wss],
        },
        HostDeclaration {
            host: "stream.lnmarkets.com",
            purpose: "LN Markets plugin v1 mainnet market and account streams",
            protocols: &[Protocol::Wss],
        },
    ],
};

/// The LN Markets plugin. Everything the application sees is registered
/// through [`plugin_api::PluginRegistry`]; core crates never name this crate.
pub struct LnMarketsPlugin;

impl LnMarketsPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LnMarketsPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl plugin_api::OmegaPlugin for LnMarketsPlugin {
    fn manifest(&self) -> &'static PluginManifest {
        &MANIFEST
    }

    fn register(&self, registry: &mut PluginRegistry, cx: &mut App) {
        let venue_capabilities = registry.venue_capabilities();
        match unix_now_ms() {
            Ok(probed_at_ms) => {
                if let Err(error) = refresh_venue_capabilities(&venue_capabilities, probed_at_ms) {
                    log::error!("could not publish LN Markets venue capabilities: {error}");
                }
            }
            Err(error) => {
                log::error!("could not read the clock for LN Markets capabilities: {error}")
            }
        }
        start_market_data_service(cx.http_client(), venue_capabilities, cx);
        registry.add_background_service(BackgroundServiceRegistration {
            plugin_id: MANIFEST.id,
            service_id: "lnmarkets_market_data_collector",
            description: "App-lifetime LN Markets market-data collector and strategy tick loop",
        });
        registry.add_review_driver(std::rc::Rc::new(LnMarketsReviewDriver));
        registry.add_extension(agent_tools_registration());
        registry.add_settings_page(settings_page_registration());
        registry.add_extension(operator_panel_loader());
        for schema in [
            "omega.lnmarkets.account.v2",
            "omega.lnmarkets.market-data.v2",
            "omega.lnmarkets.market-data.v3",
            "omega.lnmarkets.features.v1",
            "omega.lnmarkets.ledger.v1",
            "omega.lnmarkets.mandate.v1",
            "omega.lnmarkets.prediction.v1",
            "omega.lnmarkets.prediction_summary.v1",
            "omega.lnmarkets.strategy.v1",
            "omega.lnmarkets.backtest_tool.v1",
            "omega.lnmarkets.backtest_history.v1",
            "omega.lnmarkets.swap.v1",
        ] {
            registry.add_card_schema(CardSchemaRegistration {
                plugin_id: MANIFEST.id,
                schema,
            });
        }
    }
}

fn settings_page_registration() -> SettingsPageRegistration {
    SettingsPageRegistration {
        plugin_id: MANIFEST.id,
        section: "Trading",
        title: "LN Markets",
        description: "Connect an LN Markets account and test its API credentials.",
        search_aliases: &["LNMarkets", "Bitcoin", "synthetic USD"],
        page_key: "lnmarkets",
        build: std::rc::Rc::new(|window, cx| {
            let page = cx.new(|cx| {
                LnMarketsSettingsPage::new(
                    http_transport(cx.http_client()),
                    zed_credentials_provider::global(cx),
                    window,
                    cx,
                )
            });
            page.update(cx, |page, cx| page.load(window, cx));
            page.into()
        }),
    }
}

fn operator_panel_loader() -> workspace::PluginPanelLoader {
    workspace::PluginPanelLoader {
        plugin_id: MANIFEST.id,
        load: std::rc::Rc::new(|workspace_handle, mut cx| {
            Box::pin(async move {
                let source = cx
                    .update(|_window, cx| operator_console_source(cx))
                    .context("failed to obtain the LN Markets operator source")?
                    .map_err(anyhow::Error::msg)?;
                let operator_panel =
                    LnMarketsOperatorPanel::load(workspace_handle.clone(), source, cx.clone())
                        .await
                        .context("failed to load the LN Markets operator panel")?;
                workspace_handle.update_in(&mut cx, |workspace, window, cx| {
                    workspace.add_panel(operator_panel, window, cx);
                })?;
                Ok(())
            })
        }),
    }
}

struct LnMarketsGlobal {
    collector: Arc<Mutex<Option<CollectorHandle>>>,
    trading_runtime: Result<Arc<TradingRuntime>, String>,
    venue_capabilities: VenueCapabilityStore,
    _collector_task: Task<()>,
    _strategy_tick_task: Task<()>,
}

impl Global for LnMarketsGlobal {}

fn start_market_data_service(
    http_client: Arc<dyn HttpClient>,
    venue_capabilities: VenueCapabilityStore,
    cx: &mut App,
) {
    let _registrations = (
        lnmarkets_data::REGISTRATION,
        lnmarkets_trading::REGISTRATION,
    );
    let collector = Arc::new(Mutex::new(None));
    let strategy_capability_guard = venue_capabilities.guard(
        MANIFEST.id,
        VenueActionClass::StrategyExecution,
        CAPABILITY_MAX_AGE_MS,
    );
    let trading_runtime = TradingRuntime::open_default(strategy_capability_guard)
        .map(Arc::new)
        .map_err(|error| format!("could not open the LN Markets trading runtime: {error:#}"));
    if let Err(error) = &trading_runtime {
        log::error!("{error}");
    }
    let collector_task = cx.spawn({
        let collector = collector.clone();
        let credentials_provider = zed_credentials_provider::global(cx);
        let transport = http_transport(http_client);
        async move |_cx| {
            let stored_credentials = match credentials_provider
                .read_credentials(CREDENTIAL_STORAGE_URL, _cx)
                .await
            {
                Ok(Some((_username, encoded))) => match StoredCredentials::decode(&encoded) {
                    Ok(stored) => match stored.credentials() {
                        Ok(credentials) => Some((stored.network, credentials)),
                        Err(error) => {
                            log::warn!("could not load LN Markets collector credentials: {error}");
                            None
                        }
                    },
                    Err(error) => {
                        log::warn!("could not decode LN Markets collector credentials: {error}");
                        None
                    }
                },
                Ok(None) => None,
                Err(error) => {
                    log::warn!("could not read LN Markets collector credentials: {error}");
                    None
                }
            };
            let (network, config, client) = match stored_credentials {
                Some((network, credentials)) => (
                    network,
                    CollectorConfig::authenticated(network, credentials.clone()),
                    LnMarketsClient::authenticated(transport, network, credentials),
                ),
                None => (
                    Network::Signet,
                    CollectorConfig::public(Network::Signet),
                    LnMarketsClient::public(transport, Network::Signet),
                ),
            };
            let store_path = paths::data_dir().join("threads").join("lnmarkets.db");
            let store = match MarketDataStore::open(
                &store_path,
                MarketDataStore::default_retention(),
            ) {
                Ok(store) => store,
                Err(error) => {
                    log::error!(
                        "could not start LN Markets {network:?} collector at {store_path:?}: {error:#}"
                    );
                    return;
                }
            };
            let service = Collector::new(client, store, config);
            *collector.lock() = Some(service.handle());
            _cx.background_spawn(service.run()).await;
        }
    });
    let strategy_tick_task = cx.background_spawn({
        let collector = collector.clone();
        let trading_runtime = trading_runtime.clone();
        let venue_capabilities = venue_capabilities.clone();
        let executor = cx.background_executor().clone();
        async move {
            loop {
                executor.timer(Duration::from_secs(5)).await;
                let Some(collector) = collector.lock().clone() else {
                    continue;
                };
                let Ok(runtime) = &trading_runtime else {
                    return;
                };
                let at_ms = match SystemTime::now().duration_since(UNIX_EPOCH) {
                    Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
                    Err(error) => {
                        log::error!("could not read the LN Markets strategy clock: {error}");
                        continue;
                    }
                };
                if let Err(error) = refresh_venue_capabilities(&venue_capabilities, at_ms) {
                    log::warn!("could not refresh LN Markets venue capabilities: {error}");
                }
                if collector.health().network == Network::Signet {
                    match counterparty_exposure::snapshot_from_collector(&collector, at_ms) {
                        Ok(Some(snapshot)) => {
                            if let Err(error) = runtime.record_counterparty_exposure(snapshot) {
                                log::warn!(
                                    "could not record LN Markets counterparty exposure: {error:#}"
                                );
                            }
                        }
                        Ok(None) => {}
                        Err(error) => log::warn!(
                            "could not derive LN Markets counterparty exposure: {error:#}"
                        ),
                    }
                }
                if let Err(error) = runtime.process_collected_tick(&collector, at_ms).await {
                    log::warn!("LN Markets strategy tick was not processed: {error:#}");
                }
                if let Err(error) = runtime.resolve_matured_predictions(&collector, at_ms) {
                    log::warn!("LN Markets predictions were not resolved: {error:#}");
                }
            }
        }
    });
    cx.set_global(LnMarketsGlobal {
        collector,
        trading_runtime,
        venue_capabilities,
        _collector_task: collector_task,
        _strategy_tick_task: strategy_tick_task,
    });
    lnmarkets_ui::init_operator_panel(cx);
}

fn unix_now_ms() -> Result<i64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?;
    i64::try_from(elapsed.as_millis()).map_err(|_| "system timestamp overflowed i64".to_string())
}

fn refresh_venue_capabilities(
    store: &VenueCapabilityStore,
    probed_at_ms: i64,
) -> Result<(), plugin_api::VenueCapabilityPublicationError> {
    let action = |action_class| {
        ProbedVenueAssumption::new(
            VenueActionCapability {
                action_class,
                status: VenueActionStatus::Supported,
            },
            probed_at_ms,
        )
    };
    store.publish(VenueCapabilities {
        venue_id: MANIFEST.id.to_string(),
        account_mode: ProbedVenueAssumption::new(
            ObservedVenueMode::known(VenueAccountMode::SingleAccount, "single_account"),
            probed_at_ms,
        ),
        margin_mode: ProbedVenueAssumption::new(
            ObservedVenueMode::known(VenueMarginMode::VenueManaged, "venue_managed"),
            probed_at_ms,
        ),
        actions: vec![
            action(VenueActionClass::AssetSwap),
            action(VenueActionClass::StrategyExecution),
        ],
    })
}

pub fn collector(cx: &App) -> Option<CollectorHandle> {
    cx.try_global::<LnMarketsGlobal>()
        .and_then(|plugin| plugin.collector.lock().clone())
}

pub fn trading_runtime(cx: &App) -> Result<Arc<TradingRuntime>, String> {
    cx.try_global::<LnMarketsGlobal>()
        .ok_or_else(|| "LN Markets is not initialized".to_string())?
        .trading_runtime
        .clone()
}

pub fn venue_capability_store(cx: &App) -> Option<VenueCapabilityStore> {
    cx.try_global::<LnMarketsGlobal>()
        .map(|plugin| plugin.venue_capabilities.clone())
}

pub fn portfolio_review_instruction(
    session_id: &str,
    now_ms: i64,
    trigger: &str,
    cx: &App,
) -> Result<Option<String>, String> {
    let runtime = trading_runtime(cx)?;
    if !runtime.is_review_session(session_id) {
        return Ok(None);
    }
    let (feature_status, features) = match collector(cx) {
        None => ("collector_starting".to_string(), None),
        Some(collector) => match collector.features() {
            Ok(Some(features)) => ("ready".to_string(), Some(features)),
            Ok(None) => ("collecting".to_string(), None),
            Err(error) => (format!("error: {error:#}"), None),
        },
    };
    runtime
        .portfolio_review(now_ms, trigger, feature_status, features)
        .and_then(|review| review.instruction().map_err(Into::into))
        .map(Some)
        .map_err(|error| format!("could not prepare LN Markets portfolio review: {error:#}"))
}

pub fn portfolio_review_cadence(
    session_id: &str,
    cx: &App,
) -> Result<Option<ReviewCadence>, String> {
    let Some(plugin) = cx.try_global::<LnMarketsGlobal>() else {
        return Ok(None);
    };
    plugin
        .trading_runtime
        .clone()?
        .review_cadence(session_id)
        .map_err(|error| format!("could not read portfolio review cadence: {error:#}"))
}

pub fn pending_portfolio_wakeup(session_id: &str, cx: &App) -> Option<(WakeupSource, String)> {
    trading_runtime(cx)
        .ok()
        .and_then(|runtime| runtime.pending_review_wakeup(session_id))
}

pub fn acknowledge_portfolio_wakeup(
    session_id: &str,
    source: &WakeupSource,
    instruction: &str,
    cx: &App,
) -> bool {
    trading_runtime(cx)
        .is_ok_and(|runtime| runtime.acknowledge_review_wakeup(session_id, source, instruction))
}

pub fn record_portfolio_review_turn(
    session_id: &str,
    at_ms: i64,
    source: WakeupSource,
    outcome: ReviewTurnOutcome,
    cx: &App,
) -> bool {
    trading_runtime(cx)
        .is_ok_and(|runtime| runtime.record_review_turn(session_id, at_ms, source, outcome))
}

pub fn record_signet_soak_review_turn(session_id: &str, turn: SoakReviewTurn, cx: &App) -> bool {
    trading_runtime(cx).is_ok_and(|runtime| runtime.record_soak_review_turn(session_id, turn))
}

pub fn record_portfolio_review_evidence(
    session_id: &str,
    record: review_accounting::ReviewCostRecord,
    cx: &App,
) -> bool {
    let result = match trading_runtime(cx) {
        Ok(runtime) => runtime.record_review_cost(session_id, record),
        Err(error) => Err(anyhow::Error::msg(error)),
    };
    result.unwrap_or_else(|error| {
        log::error!("could not append review-turn cost evidence: {error:#}");
        false
    })
}

pub fn operator_console_source(cx: &App) -> Result<Arc<dyn OperatorConsoleSource>, String> {
    let plugin = cx
        .try_global::<LnMarketsGlobal>()
        .ok_or_else(|| "LN Markets is not initialized".to_string())?;
    Ok(Arc::new(PluginOperatorConsoleSource {
        collector: plugin.collector.clone(),
        trading_runtime: plugin.trading_runtime.clone(),
        venue_capabilities: plugin.venue_capabilities.clone(),
    }))
}

struct PluginOperatorConsoleSource {
    collector: Arc<Mutex<Option<CollectorHandle>>>,
    trading_runtime: Result<Arc<TradingRuntime>, String>,
    venue_capabilities: VenueCapabilityStore,
}

impl OperatorConsoleSource for PluginOperatorConsoleSource {
    fn snapshot(&self, now_ms: i64) -> OperatorConsoleSnapshot {
        let collector = self.collector.lock().clone();
        let collector_health = collector.as_ref().map(CollectorHandle::health);
        let venue_capabilities =
            self.venue_capabilities
                .report(MANIFEST.id, now_ms, CAPABILITY_MAX_AGE_MS);
        let runtime = match &self.trading_runtime {
            Ok(runtime) => runtime,
            Err(error) => {
                let mut snapshot = OperatorConsoleSnapshot::unavailable(now_ms, error.clone());
                snapshot.collector = collector_health;
                snapshot.venue_capabilities = Some(venue_capabilities);
                return snapshot;
            }
        };
        let (feature_status, features) = match collector.as_ref() {
            None => ("collector_starting".to_string(), None),
            Some(collector) => match collector.features() {
                Ok(Some(features)) => ("ready".to_string(), Some(features)),
                Ok(None) => ("collecting".to_string(), None),
                Err(error) => (format!("error: {error:#}"), None),
            },
        };
        let review =
            match runtime.portfolio_review(now_ms, "operator_panel", feature_status, features) {
                Ok(review) => review,
                Err(error) => {
                    let mut snapshot = OperatorConsoleSnapshot::unavailable(
                        now_ms,
                        format!("Could not read the trading runtime: {error:#}"),
                    );
                    snapshot.collector = collector_health;
                    snapshot.venue_capabilities = Some(venue_capabilities);
                    return snapshot;
                }
            };
        let headroom = review
            .limit_headroom
            .as_ref()
            .map(|limits| &limits.by_strategy);
        let strategies = review
            .strategies
            .iter()
            .map(|strategy| {
                let limits = headroom.and_then(|limits| {
                    limits
                        .iter()
                        .find(|limits| limits.strategy_id == strategy.strategy_id)
                });
                OperatorStrategySnapshot {
                    strategy_id: strategy.strategy_id.clone(),
                    status: strategy.status.clone(),
                    state: strategy.state.as_ref().map(ToString::to_string),
                    last_action: strategy.last_action.clone(),
                    daily_loss_headroom_sats: limits.map(|limits| limits.daily_loss_headroom_sats),
                    order_headroom: limits.map(|limits| limits.order_headroom),
                }
            })
            .collect();
        let review_cadence = review
            .mandate
            .mandates
            .first()
            .map(|mandate| mandate.review_cadence.clone());
        let backtests = review
            .backtests
            .iter()
            .map(|report| OperatorBacktestSnapshot {
                strategy_id: report.strategy_id.clone(),
                outcome: if report.passed() { "passed" } else { "failed" }.to_string(),
                created_at_ms: report.created_at_ms,
                trade_count: report.trade_count,
                expectancy_millisats: report.expectancy_millisats,
                maximum_drawdown_sats: report.maximum_drawdown_sats,
                parameter_digest: report.parameter_digest.clone(),
            })
            .collect();
        let review_history = runtime
            .review_turn_history()
            .into_iter()
            .map(|turn| OperatorReviewTurn {
                at_ms: turn.at_ms,
                trigger: turn.source.transcript_label(),
                outcome: match turn.outcome {
                    ReviewTurnOutcome::Completed => "completed",
                    ReviewTurnOutcome::Failed => "failed",
                }
                .to_string(),
            })
            .collect();
        OperatorConsoleSnapshot {
            generated_at_ms: now_ms,
            collector: collector_health,
            venue_capabilities: Some(venue_capabilities),
            strategies,
            backtests,
            ledger: Some(review.daily_report),
            mandate: Some(review.mandate),
            review_cadence,
            pending_wakeups: runtime.pending_review_wakeup_count(),
            review_history,
            review_costs: Some(review.review_costs),
            runtime_error: None,
        }
    }

    fn narrow_mandate(&self, changed_at_ms: i64) -> anyhow::Result<()> {
        self.trading_runtime
            .clone()
            .map_err(anyhow::Error::msg)?
            .narrow_mandate(changed_at_ms)?;
        Ok(())
    }

    fn revoke_mandate(&self, changed_at_ms: i64) -> anyhow::Result<()> {
        self.trading_runtime
            .clone()
            .map_err(anyhow::Error::msg)?
            .revoke_mandate(changed_at_ms)?;
        Ok(())
    }
}

struct OmegaHttpTransport {
    http_client: Arc<dyn HttpClient>,
}

impl HttpTransport for OmegaHttpTransport {
    fn send(
        &self,
        request: http::Request<Vec<u8>>,
    ) -> BoxFuture<'static, Result<http::Response<Vec<u8>>, TransportFailure>> {
        let http_client = self.http_client.clone();
        async move {
            let request = request.map(AsyncBody::from);
            let response = http_client
                .send(request)
                .await
                .map_err(classify_transport_failure)?;
            let (parts, body) = response.into_parts();
            let mut bytes = Vec::new();
            body.take(MAX_TRANSPORT_RESPONSE_BYTES)
                .read_to_end(&mut bytes)
                .await
                .map_err(|source| {
                    let kind = if source.kind() == std::io::ErrorKind::TimedOut {
                        TransportFailureKind::ReadTimeout
                    } else {
                        TransportFailureKind::Other
                    };
                    TransportFailure::new(kind, source)
                })?;
            Ok(http::Response::from_parts(parts, bytes))
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_capability_surface_is_complete_and_fresh() {
        let store = VenueCapabilityStore::default();
        refresh_venue_capabilities(&store, 1_000).expect("publish capabilities");

        let report = store.report(MANIFEST.id, 1_001, CAPABILITY_MAX_AGE_MS);
        assert_eq!(
            report.verification.status,
            plugin_api::VenueCapabilityVerificationStatus::Verified
        );
        assert!(!report.verification.stale);
        for action_class in [
            VenueActionClass::AssetSwap,
            VenueActionClass::StrategyExecution,
        ] {
            store
                .guard(MANIFEST.id, action_class, CAPABILITY_MAX_AGE_MS)
                .require_effectful(1_001)
                .expect("reference action is supported");
        }
    }
}

fn classify_transport_failure(source: anyhow::Error) -> TransportFailure {
    let kind = source
        .chain()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>())
        .map(|error| match (error.is_connect(), error.is_timeout()) {
            (true, true) => TransportFailureKind::ConnectTimeout,
            (true, false) => TransportFailureKind::Connect,
            (false, true) => TransportFailureKind::WriteTimeout,
            (false, false) => TransportFailureKind::Other,
        })
        .unwrap_or(TransportFailureKind::Other);
    TransportFailure::new(kind, source)
}

pub fn http_transport(http_client: Arc<dyn HttpClient>) -> Arc<dyn HttpTransport> {
    Arc::new(OmegaHttpTransport { http_client })
}
