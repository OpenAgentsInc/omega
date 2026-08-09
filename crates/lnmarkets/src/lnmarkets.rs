use std::sync::Arc;

use futures::{AsyncReadExt as _, FutureExt as _, future::BoxFuture};
use gpui::{App, AppContext as _, Global, Task};
use http_client::{AsyncBody, HttpClient};
use lnmarkets_data::{Collector, CollectorConfig, CollectorHandle, MarketDataStore};
use parking_lot::Mutex;

mod review_turn;
mod signet_soak;
mod trading_runtime;

pub use agent_wakeup::WakeupSource;
pub use lnmarkets_ui::{
    LnMarketsOperatorPanel, OperatorConsoleSnapshot, OperatorConsoleSource, OperatorReviewTurn,
    OperatorStrategySnapshot,
};
pub use review_turn::{PORTFOLIO_REVIEW_SCHEMA, PORTFOLIO_REVIEW_TOKEN_BUDGET, PortfolioReview};
pub use signet_soak::{
    SIGNET_SOAK_SCHEMA, SignetSoakEvidence, SignetSoakReceipt, SignetSoakRefusal, SignetSoakStatus,
    SoakBudget, SoakLimitBreach, SoakReconciliationSample, SoakReviewTurn, SoakStrategyObservation,
    SoakWindow,
};
pub use trading_runtime::{
    ReviewTurnHistory, ReviewTurnOutcome, StrategyRuntimeSnapshot, TradingRuntime,
};

pub use lnmarkets_client::*;
pub use lnmarkets_data::{
    AccountAllocation, AccountDriftFeatures, CANDLE_TOPIC, CollectorHealth, CollectorHistory,
    CollectorStatus, FUNDING_SETTLEMENT_TOPIC, FeatureSnapshot, FundingFeatures, FundingSign,
    LiquidityFeatures, ORACLE_INDEX_TOPIC, StoredMarketEvent, VolatilityFeatures,
};
pub use lnmarkets_trading::{
    FundingCarryConfig, FundingCarryInstrument, RebalanceCostMeasurement, RebalanceToTargetConfig,
    StrategyLifecycleEvent,
};
pub use lnmarkets_ui::LnMarketsSettingsPage;
pub use trading_ledger::{LedgerEntry, LedgerQuery, LedgerStore, ProfitReport};
pub use trading_mandate::{
    MandateDecision, MandateRefusal, MandateRevision, MandateSnapshot, MandateStore, ReviewCadence,
    TradingInstruction, TradingMandate, TradingNetwork,
};

const MAX_TRANSPORT_RESPONSE_BYTES: u64 = 1_048_577;

pub const REST_HOSTS: &[&str] = &["api.signet.lnmarkets.com", "api.lnmarkets.com"];
pub const STREAM_HOSTS: &[&str] = &["stream.signet.lnmarkets.com", "stream.lnmarkets.com"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PluginManifest {
    pub name: &'static str,
    pub rest_hosts: &'static [&'static str],
    pub stream_hosts: &'static [&'static str],
}

pub const MANIFEST: PluginManifest = PluginManifest {
    name: "LN Markets",
    rest_hosts: REST_HOSTS,
    stream_hosts: STREAM_HOSTS,
};

pub struct LnMarketsPlugin {
    collector: Arc<Mutex<Option<CollectorHandle>>>,
    trading_runtime: Result<Arc<TradingRuntime>, String>,
    _collector_task: Task<()>,
}

impl Global for LnMarketsPlugin {}

pub fn init(http_client: Arc<dyn HttpClient>, cx: &mut App) {
    let _registrations = (
        lnmarkets_data::REGISTRATION,
        lnmarkets_trading::REGISTRATION,
    );
    let collector = Arc::new(Mutex::new(None));
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
    let trading_runtime = TradingRuntime::open_default()
        .map(Arc::new)
        .map_err(|error| format!("could not open the LN Markets trading runtime: {error:#}"));
    if let Err(error) = &trading_runtime {
        log::error!("{error}");
    }
    cx.set_global(LnMarketsPlugin {
        collector,
        trading_runtime,
        _collector_task: collector_task,
    });
    lnmarkets_ui::init_operator_panel(cx);
}

pub fn collector(cx: &App) -> Option<CollectorHandle> {
    cx.try_global::<LnMarketsPlugin>()
        .and_then(|plugin| plugin.collector.lock().clone())
}

pub fn trading_runtime(cx: &App) -> Result<Arc<TradingRuntime>, String> {
    cx.try_global::<LnMarketsPlugin>()
        .ok_or_else(|| "LN Markets is not initialized".to_string())?
        .trading_runtime
        .clone()
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
    let Some(plugin) = cx.try_global::<LnMarketsPlugin>() else {
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

pub fn operator_console_source(cx: &App) -> Result<Arc<dyn OperatorConsoleSource>, String> {
    let plugin = cx
        .try_global::<LnMarketsPlugin>()
        .ok_or_else(|| "LN Markets is not initialized".to_string())?;
    Ok(Arc::new(PluginOperatorConsoleSource {
        collector: plugin.collector.clone(),
        trading_runtime: plugin.trading_runtime.clone(),
    }))
}

struct PluginOperatorConsoleSource {
    collector: Arc<Mutex<Option<CollectorHandle>>>,
    trading_runtime: Result<Arc<TradingRuntime>, String>,
}

impl OperatorConsoleSource for PluginOperatorConsoleSource {
    fn snapshot(&self, now_ms: i64) -> OperatorConsoleSnapshot {
        let collector = self.collector.lock().clone();
        let collector_health = collector.as_ref().map(CollectorHandle::health);
        let runtime = match &self.trading_runtime {
            Ok(runtime) => runtime,
            Err(error) => {
                let mut snapshot = OperatorConsoleSnapshot::unavailable(now_ms, error.clone());
                snapshot.collector = collector_health;
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
            .mandate
            .as_ref()
            .map(|mandate| mandate.review_cadence.clone());
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
            strategies,
            ledger: Some(review.daily_report),
            mandate: Some(review.mandate),
            review_cadence,
            pending_wakeups: runtime.pending_review_wakeup_count(),
            review_history,
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
