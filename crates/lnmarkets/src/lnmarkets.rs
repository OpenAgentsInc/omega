use std::sync::Arc;

use futures::{AsyncReadExt as _, FutureExt as _, future::BoxFuture};
use gpui::{App, AppContext as _, Global, Task};
use http_client::{AsyncBody, HttpClient};
use lnmarkets_data::{Collector, CollectorConfig, CollectorHandle, MarketDataStore};
use parking_lot::Mutex;

mod trading_runtime;

pub use trading_runtime::{StrategyRuntimeSnapshot, TradingRuntime};

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
