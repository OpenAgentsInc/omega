use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v1 as acp;
use futures::AsyncReadExt as _;
use gpui::{App, AppContext as _, Task};
use http_client::{AsyncBody, HttpClient as _, HttpClientWithUrl, Method, Request};
use language_model::LanguageModelToolResultContent;
use language_models::AllLanguageModelSettings;
use omega_effectd::PublicRelayEvent;
use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use settings::Settings as _;
use ui::SharedString;

use crate::{AgentTool, ToolCallEventStream, ToolInput};

const MANIFEST_URL: &str = "https://bazaar.openagents.com/bazaar-public-regtest.json";
const PRODUCTION_MARKET_API_URL: &str = "https://api.openagents.com/v1/market/regtest/swaps";
const DEVELOPMENT_MARKET_API_URL: &str = "http://127.0.0.1:8080/v1/market/regtest/swaps";
const FALLBACK_RELAYS: &[&str] = &[
    "wss://relay-a.34-41-78-122.nip.io",
    "wss://relay-b.34-41-78-122.sslip.io",
];
const LIVE_DISCLOSURE: &str = "LIVE public regtest coordination data, read-only; relay and provider claims are unverified until a requester verifies locally.";
const DEMO_DISCLOSURE: &str =
    "DEMO DATA: deterministic fixture, not the live network; no real funds move.";
const REGTEST_DISCLOSURE: &str = "REGTEST: live providers and valueless regtest funds; the requester verifies both settlement rails.";
const MAINNET_WARNING: &str =
    "Mainnet swap tools are blocked. No mainnet request was sent and no funds moved.";
const CLOUD_PROVISION_DISCLOSURE: &str =
    "MOCK CLOUD PROVISIONING: no payment is charged and no infrastructure is created.";
const SWAP_STAGES: &[&str] = &["contract", "funding", "executing", "settled"];
const SWAP_STAGE_DELAY: Duration = Duration::from_millis(450);
const CLOUD_PROVISION_STAGES: &[&str] = &["payment", "relay", "provider", "connected"];
const CLOUD_PROVISION_STAGE_DELAY: Duration = Duration::from_millis(450);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MarketToolOutput(Value);

impl From<MarketToolOutput> for LanguageModelToolResultContent {
    fn from(output: MarketToolOutput) -> Self {
        serde_json::to_string_pretty(&output.0)
            .unwrap_or_else(|error| format!("Failed to serialize market tool output: {error}"))
            .into()
    }
}

impl MarketToolOutput {
    fn success(value: Value) -> Self {
        Self(value)
    }

    fn error(message: impl Into<String>) -> Self {
        Self(json!({ "error": message.into() }))
    }
}

#[derive(Default)]
struct MarketDemoState {
    quote_counter: u64,
    swap_counter: u64,
    provision_counter: u64,
    quotes: HashMap<String, Quote>,
    swaps: HashMap<String, Swap>,
    regtest_swaps: HashMap<String, Value>,
}

#[derive(Clone)]
struct Quote {
    network: MarketNetwork,
    from: MarketAsset,
    to: MarketAsset,
    amount_sats: u64,
}

#[derive(Clone)]
struct Swap {
    id: String,
    quote_id: String,
    from: MarketAsset,
    to: MarketAsset,
    amount_sats: u64,
    status_history: Vec<SwapStatusRecord>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MarketNetwork {
    Demo,
    Regtest,
    Mainnet,
}

impl MarketNetwork {
    fn as_str(self) -> &'static str {
        match self {
            Self::Demo => "demo",
            Self::Regtest => "regtest",
            Self::Mainnet => "mainnet",
        }
    }
}

fn migrate_legacy_network(input: &mut Value, network: MarketNetwork) {
    if let Some(input) = input.as_object_mut() {
        input
            .entry("network".to_string())
            .or_insert_with(|| Value::String(network.as_str().to_string()));
    }
}

#[derive(Clone)]
struct SwapStatusRecord {
    id: String,
    sequence: usize,
    previous: Option<String>,
    stage: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum MarketAsset {
    #[serde(rename = "LN")]
    Lightning,
    #[serde(rename = "BTC")]
    Bitcoin,
    #[serde(rename = "L-BTC")]
    LiquidBitcoin,
}

impl MarketAsset {
    fn as_str(self) -> &'static str {
        match self {
            Self::Lightning => "LN",
            Self::Bitcoin => "BTC",
            Self::LiquidBitcoin => "L-BTC",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum MarketCloudRegion {
    #[default]
    #[serde(rename = "us-central1")]
    UsCentral1,
    #[serde(rename = "us-east1")]
    UsEast1,
    #[serde(rename = "europe-west1")]
    EuropeWest1,
}

impl MarketCloudRegion {
    fn as_str(self) -> &'static str {
        match self {
            Self::UsCentral1 => "us-central1",
            Self::UsEast1 => "us-east1",
            Self::EuropeWest1 => "europe-west1",
        }
    }
}

struct CloudProvision {
    id: String,
    provider_name: String,
    region: MarketCloudRegion,
}

pub struct MarketNetworkStatusTool {
    http_client: Arc<HttpClientWithUrl>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
/// Read a demo fixture or the live public regtest network. Mainnet is blocked.
pub struct MarketNetworkStatusInput {
    /// Network mode. Use demo for fixtures or regtest for live valueless infrastructure.
    network: MarketNetwork,
}

impl MarketNetworkStatusTool {
    pub fn new(http_client: Arc<HttpClientWithUrl>) -> Self {
        Self { http_client }
    }
}

impl AgentTool for MarketNetworkStatusTool {
    type Input = MarketNetworkStatusInput;
    type Output = MarketToolOutput;

    const NAME: &'static str = "market_network_status";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn migrate_input_for_replay(input: &mut Value) {
        migrate_legacy_network(input, MarketNetwork::Regtest);
    }

    fn initial_title(&self, input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        match input {
            Ok(input) => format!("Check {} swap network", input.network.as_str()).into(),
            Err(_) => "Check swap network".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let http_client = self.http_client.clone();
        cx.spawn(async move |cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| MarketToolOutput::error(error.to_string()))?;
            match input.network {
                MarketNetwork::Demo => Ok(MarketToolOutput::success(demo_network_status())),
                MarketNetwork::Mainnet => Ok(mainnet_warning("market_network_status")),
                MarketNetwork::Regtest => cx
                    .background_spawn(async move { load_network_status(http_client).await })
                    .await
                    .map(MarketToolOutput::success)
                    .map_err(MarketToolOutput::error),
            }
        })
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
/// Request a demo fixture quote or an indicative live regtest route. Mainnet is blocked.
pub struct MarketSwapQuoteInput {
    /// Network mode. Regtest supports LN to BTC and BTC to LN.
    network: MarketNetwork,
    from: MarketAsset,
    to: MarketAsset,
    /// Swap amount in sats, from 1,000 through 10,000,000.
    amount_sats: u64,
}

pub struct MarketSwapQuoteTool {
    state: Arc<Mutex<MarketDemoState>>,
    http_client: Arc<HttpClientWithUrl>,
}

impl AgentTool for MarketSwapQuoteTool {
    type Input = MarketSwapQuoteInput;
    type Output = MarketToolOutput;

    const NAME: &'static str = "market_swap_quote";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }

    fn migrate_input_for_replay(input: &mut Value) {
        migrate_legacy_network(input, MarketNetwork::Demo);
    }

    fn initial_title(&self, input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        match input {
            Ok(input) => format!(
                "Quote {} sats {} → {}",
                input.amount_sats,
                input.from.as_str(),
                input.to.as_str()
            )
            .into(),
            Err(_) => "Quote swap".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| MarketToolOutput::error(error.to_string()))?;
            match input.network {
                MarketNetwork::Demo => {
                    quote_swap(&self.state, input).map_err(MarketToolOutput::error)
                }
                MarketNetwork::Mainnet => Ok(mainnet_warning("market_swap_quote")),
                MarketNetwork::Regtest => {
                    let network = cx
                        .background_spawn({
                            let http_client = self.http_client.clone();
                            async move { load_network_status(http_client).await }
                        })
                        .await
                        .map_err(MarketToolOutput::error)?;
                    quote_regtest_swap(&self.state, input, &network)
                        .map_err(MarketToolOutput::error)
                }
            }
        })
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
/// Execute a demo fixture or a real regtest swap. Mainnet is blocked.
pub enum MarketExecuteSwapInput {
    Quoted {
        network: MarketNetwork,
        quote_id: String,
    },
    Direct {
        network: MarketNetwork,
        from: MarketAsset,
        to: MarketAsset,
        /// Swap amount in sats, from 1,000 through 10,000,000.
        amount_sats: u64,
    },
}

impl MarketExecuteSwapInput {
    fn network(&self) -> MarketNetwork {
        match self {
            Self::Quoted { network, .. } | Self::Direct { network, .. } => *network,
        }
    }
}

struct ExecutionDetails {
    network: MarketNetwork,
    quote_id: Option<String>,
    from: MarketAsset,
    to: MarketAsset,
    amount_sats: u64,
}

pub struct MarketExecuteSwapTool {
    state: Arc<Mutex<MarketDemoState>>,
    http_client: Option<Arc<HttpClientWithUrl>>,
    stage_delay: Duration,
}

impl AgentTool for MarketExecuteSwapTool {
    type Input = MarketExecuteSwapInput;
    type Output = MarketToolOutput;

    const NAME: &'static str = "market_execute_swap";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Execute
    }

    fn migrate_input_for_replay(input: &mut Value) {
        migrate_legacy_network(input, MarketNetwork::Demo);
    }

    fn initial_title(&self, input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        match input {
            Ok(MarketExecuteSwapInput::Quoted { network, quote_id }) => {
                format!("Execute {} swap for {quote_id}", network.as_str()).into()
            }
            Ok(MarketExecuteSwapInput::Direct {
                network,
                from,
                to,
                amount_sats,
            }) => format!(
                "{} swap {} sats {} → {}",
                network.as_str(),
                amount_sats,
                from.as_str(),
                to.as_str()
            )
            .into(),
            Err(_) => "Execute swap".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| MarketToolOutput::error(error.to_string()))?;
            let network = input.network();
            if network == MarketNetwork::Mainnet {
                return Ok(mainnet_warning("market_execute_swap"));
            }
            if network == MarketNetwork::Regtest {
                let details =
                    execution_details(&self.state, input).map_err(MarketToolOutput::error)?;
                let pending = regtest_pending_swap(&details);
                emit_market_update(&event_stream, &pending);
                let (transport_url, authentication_url) = cx.update(|cx| {
                    let settings = &AllLanguageModelSettings::get_global(cx).openagents;
                    let transport_url = if settings.use_development_api {
                        DEVELOPMENT_MARKET_API_URL
                    } else {
                        PRODUCTION_MARKET_API_URL
                    };
                    (
                        transport_url.to_string(),
                        PRODUCTION_MARKET_API_URL.to_string(),
                    )
                });
                let output = execute_regtest_swap(
                    self.http_client.clone().ok_or_else(|| {
                        MarketToolOutput::error("the regtest HTTP client is not configured")
                    })?,
                    &transport_url,
                    &authentication_url,
                    &details,
                )
                .await
                .map_err(MarketToolOutput::error)?;
                if event_stream.was_cancelled_by_user() {
                    return Err(MarketToolOutput::error("regtest swap was canceled"));
                }
                store_regtest_swap(&self.state, &output).map_err(MarketToolOutput::error)?;
                emit_market_update(&event_stream, &output);
                return Ok(output);
            }
            let (quote_id, quote_output) =
                prepare_execution(&self.state, input).map_err(MarketToolOutput::error)?;
            if let Some(quote_output) = quote_output {
                emit_market_update(&event_stream, &quote_output);
                cx.background_executor().timer(self.stage_delay).await;
                if event_stream.was_cancelled_by_user() {
                    return Err(MarketToolOutput::error("demo swap was canceled"));
                }
            }

            let mut output =
                execute_swap(&self.state, &quote_id).map_err(MarketToolOutput::error)?;
            emit_market_update(&event_stream, &output);

            for _ in 1..SWAP_STAGES.len() {
                cx.background_executor().timer(self.stage_delay).await;
                if event_stream.was_cancelled_by_user() {
                    return Err(MarketToolOutput::error("demo swap was canceled"));
                }
                output = advance_swap(&self.state, output.swap_id()?)
                    .map_err(MarketToolOutput::error)?;
                emit_market_update(&event_stream, &output);
            }

            Ok(output)
        })
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
/// Read a recorded demo or regtest swap. Mainnet is blocked.
pub struct MarketSwapStatusInput {
    network: MarketNetwork,
    swap_id: String,
}

pub struct MarketSwapStatusTool {
    state: Arc<Mutex<MarketDemoState>>,
}

impl AgentTool for MarketSwapStatusTool {
    type Input = MarketSwapStatusInput;
    type Output = MarketToolOutput;

    const NAME: &'static str = "market_swap_status";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }

    fn migrate_input_for_replay(input: &mut Value) {
        migrate_legacy_network(input, MarketNetwork::Demo);
    }

    fn initial_title(&self, input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        match input {
            Ok(input) => format!("Check {} swap {}", input.network.as_str(), input.swap_id).into(),
            Err(_) => "Check swap".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |_cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| MarketToolOutput::error(error.to_string()))?;
            match input.network {
                MarketNetwork::Demo => {
                    swap_status(&self.state, input).map_err(MarketToolOutput::error)
                }
                MarketNetwork::Regtest => regtest_swap_status(&self.state, &input.swap_id)
                    .map_err(MarketToolOutput::error),
                MarketNetwork::Mainnet => Ok(mainnet_warning("market_swap_status")),
            }
        })
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
/// Provision a mock paid provider node and relay connected to OpenAgents cloud. No payment is charged and no infrastructure is created.
pub struct MarketProvisionCloudInput {
    /// Provider label. Defaults to "Omega provider".
    provider_name: Option<String>,
    /// Cloud region. Defaults to us-central1.
    region: Option<MarketCloudRegion>,
}

pub struct MarketProvisionCloudTool {
    state: Arc<Mutex<MarketDemoState>>,
    stage_delay: Duration,
}

impl AgentTool for MarketProvisionCloudTool {
    type Input = MarketProvisionCloudInput;
    type Output = MarketToolOutput;

    const NAME: &'static str = "market_provision_cloud";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Execute
    }

    fn initial_title(&self, input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        match input {
            Ok(input) => format!(
                "Provision {} in {}",
                input.provider_name.as_deref().unwrap_or("Omega provider"),
                input.region.unwrap_or_default().as_str(),
            )
            .into(),
            Err(_) => "Provision provider cloud".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| MarketToolOutput::error(error.to_string()))?;
            let provision =
                start_cloud_provision(&self.state, input).map_err(MarketToolOutput::error)?;
            let mut output =
                cloud_provision_view(&provision, 0).map_err(MarketToolOutput::error)?;
            emit_market_update(&event_stream, &output);

            for stage_index in 1..CLOUD_PROVISION_STAGES.len() {
                cx.background_executor().timer(self.stage_delay).await;
                if event_stream.was_cancelled_by_user() {
                    return Err(MarketToolOutput::error(
                        "mock cloud provisioning was canceled",
                    ));
                }
                output = cloud_provision_view(&provision, stage_index)
                    .map_err(MarketToolOutput::error)?;
                emit_market_update(&event_stream, &output);
            }

            Ok(output)
        })
    }
}

pub fn market_demo_tools(
    http_client: Arc<HttpClientWithUrl>,
) -> (
    MarketNetworkStatusTool,
    MarketSwapQuoteTool,
    MarketExecuteSwapTool,
    MarketSwapStatusTool,
    MarketProvisionCloudTool,
) {
    let state = Arc::new(Mutex::new(MarketDemoState::default()));
    (
        MarketNetworkStatusTool::new(http_client.clone()),
        MarketSwapQuoteTool {
            state: state.clone(),
            http_client: http_client.clone(),
        },
        MarketExecuteSwapTool {
            state: state.clone(),
            http_client: Some(http_client),
            stage_delay: SWAP_STAGE_DELAY,
        },
        MarketSwapStatusTool {
            state: state.clone(),
        },
        MarketProvisionCloudTool {
            state,
            stage_delay: CLOUD_PROVISION_STAGE_DELAY,
        },
    )
}

fn mainnet_warning(operation: &str) -> MarketToolOutput {
    MarketToolOutput::success(json!({
        "schema": "omega.market-demo.warning.v1",
        "network": "mainnet",
        "operation": operation,
        "blocked": true,
        "warning": MAINNET_WARNING
    }))
}

fn demo_network_status() -> Value {
    json!({
        "schema": "omega.market-demo.network-status.v1",
        "network": "demo",
        "source": "fixture",
        "disclosure": DEMO_DISCLOSURE,
        "name": "representative demo network",
        "manifest": {
            "service_state": "ready",
            "bazaar_revision": "demo-fixture",
            "immortal_revision": "demo-fixture"
        },
        "relays": [
            {"label": "relay-a", "url": "wss://relay-a.demo.invalid", "state": "ready", "trust": "fixture"},
            {"label": "relay-b", "url": "wss://relay-b.demo.invalid", "state": "ready", "trust": "fixture"}
        ],
        "providers": [
            {"label": "provider-b", "pubkey": "demo-provider-b", "state": "ready", "trust": "fixture", "relays": ["relay-a", "relay-b"], "fee_bps": 22, "active_offerings": 6},
            {"label": "provider-c", "pubkey": "demo-provider-c", "state": "ready", "trust": "fixture", "relays": ["relay-a", "relay-b"], "fee_bps": 34, "active_offerings": 6}
        ],
        "stats": {}
    })
}

fn quote_regtest_swap(
    state: &Mutex<MarketDemoState>,
    input: MarketSwapQuoteInput,
    network: &Value,
) -> Result<MarketToolOutput, String> {
    validate_regtest_direction(input.from, input.to, input.amount_sats)?;
    let provider = network
        .get("providers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|provider| provider.get("state").and_then(Value::as_str) == Some("ready"))
        .filter_map(|provider| {
            Some((
                provider.get("fee_bps")?.as_u64()?,
                provider.get("label")?.as_str()?.to_string(),
            ))
        })
        .min_by_key(|(fee_bps, _)| *fee_bps)
        .ok_or_else(|| "the live regtest network has no ready swap provider".to_string())?;
    let mut state = state.lock();
    state.quote_counter = state.quote_counter.saturating_add(1);
    let quote_id = format!("regtest-route-{}", state.quote_counter);
    state.quotes.insert(
        quote_id.clone(),
        Quote {
            network: MarketNetwork::Regtest,
            from: input.from,
            to: input.to,
            amount_sats: input.amount_sats,
        },
    );
    Ok(MarketToolOutput::success(json!({
        "schema": "omega.market-demo.quote.v1",
        "network": "regtest",
        "disclosure": "REGTEST INDICATIVE ROUTE: provider availability and fee are live claims; execution obtains signed quotes and verifies both settlement rails.",
        "quote_id": quote_id,
        "from": input.from.as_str(),
        "to": input.to.as_str(),
        "amount_sats": input.amount_sats,
        "provider": provider.1,
        "fee_bps": provider.0,
        "kind": "indicative",
        "expires_in_seconds": 30
    })))
}

fn validate_regtest_direction(
    from: MarketAsset,
    to: MarketAsset,
    amount_sats: u64,
) -> Result<(), String> {
    if !(100_000..=1_000_000).contains(&amount_sats) {
        return Err("regtest amount_sats must be between 100,000 and 1,000,000".into());
    }
    match (from, to) {
        (MarketAsset::Lightning, MarketAsset::Bitcoin)
        | (MarketAsset::Bitcoin, MarketAsset::Lightning) => Ok(()),
        (MarketAsset::LiquidBitcoin, _) | (_, MarketAsset::LiquidBitcoin) => {
            Err("the public regtest service does not support Liquid Bitcoin yet".into())
        }
        _ => Err("regtest supports LN to BTC and BTC to LN".into()),
    }
}

fn execution_details(
    state: &Mutex<MarketDemoState>,
    input: MarketExecuteSwapInput,
) -> Result<ExecutionDetails, String> {
    match input {
        MarketExecuteSwapInput::Quoted { network, quote_id } => {
            let state = state.lock();
            let quote = state.quotes.get(&quote_id).ok_or_else(|| {
                format!("unknown quote_id {quote_id}; request a fresh quote first")
            })?;
            if quote.network != network {
                return Err(format!(
                    "quote_id {quote_id} belongs to network {}",
                    quote.network.as_str()
                ));
            }
            validate_regtest_direction(quote.from, quote.to, quote.amount_sats)?;
            Ok(ExecutionDetails {
                network,
                quote_id: Some(quote_id),
                from: quote.from,
                to: quote.to,
                amount_sats: quote.amount_sats,
            })
        }
        MarketExecuteSwapInput::Direct {
            network,
            from,
            to,
            amount_sats,
        } => {
            validate_regtest_direction(from, to, amount_sats)?;
            Ok(ExecutionDetails {
                network,
                quote_id: None,
                from,
                to,
                amount_sats,
            })
        }
    }
}

fn regtest_pending_swap(details: &ExecutionDetails) -> MarketToolOutput {
    MarketToolOutput::success(json!({
        "schema": "omega.market-demo.swap.v1",
        "network": details.network.as_str(),
        "disclosure": REGTEST_DISCLOSURE,
        "swap_id": "regtest-swap-pending",
        "quote_id": details.quote_id,
        "from": details.from.as_str(),
        "to": details.to.as_str(),
        "amount_sats": details.amount_sats,
        "provider": "selecting signed provider",
        "fee_bps": null,
        "stage": "contract",
        "verification": "The API is obtaining signed regtest quotes.",
        "status_history": [{
            "status_id": "regtest-swap-pending-status-0",
            "sequence": 0,
            "previous": null,
            "stage": "contract",
            "authority": "requester_local"
        }]
    }))
}

async fn execute_regtest_swap(
    http_client: Arc<HttpClientWithUrl>,
    transport_url: &str,
    authentication_url: &str,
    details: &ExecutionDetails,
) -> Result<MarketToolOutput, String> {
    let body = serde_json::to_string(&json!({
        "schema": "openagents.market.swap-execute.v1",
        "network": "regtest",
        "from": details.from.as_str(),
        "to": details.to.as_str(),
        "amount_sats": details.amount_sats
    }))
    .map_err(|error| format!("regtest request could not be encoded: {error}"))?;
    let authorization =
        omega_effectd::sign_nip98_request(authentication_url, "POST", Some(body.as_bytes()), None)
            .await
            .map_err(|error| format!("regtest request could not be signed: {error}"))?;
    let request = Request::builder()
        .method(Method::POST)
        .uri(transport_url)
        .header("authorization", authorization)
        .header("content-type", "application/json")
        .header("content-length", body.len().to_string())
        .body(AsyncBody::from(body))
        .map_err(|error| format!("regtest request could not be built: {error}"))?;
    let mut response = http_client
        .send(request)
        .await
        .map_err(|error| format!("regtest API request failed: {error}"))?;
    let status = response.status();
    let mut response_body = Vec::new();
    response
        .body_mut()
        .take(2 * 1024 * 1024)
        .read_to_end(&mut response_body)
        .await
        .map_err(|error| format!("regtest API response could not be read: {error}"))?;
    let value: Value = serde_json::from_slice(&response_body)
        .map_err(|error| format!("regtest API response was invalid JSON: {error}"))?;
    if !status.is_success() {
        let message = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("the regtest API refused the swap");
        return Err(format!(
            "regtest API returned HTTP {}: {message}",
            status.as_u16()
        ));
    }
    regtest_swap_view(value, details)
}

fn regtest_swap_view(value: Value, details: &ExecutionDetails) -> Result<MarketToolOutput, String> {
    if value.get("schema").and_then(Value::as_str) != Some("openagents.market.regtest-swap.v1")
        || value.get("network").and_then(Value::as_str) != Some("regtest")
        || value.get("stage").and_then(Value::as_str) != Some("settled")
    {
        return Err("the regtest API returned an unknown swap result".into());
    }
    let swap_id = value
        .get("swap_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "the regtest result has no swap_id".to_string())?;
    let provider = value
        .get("provider")
        .and_then(Value::as_str)
        .ok_or_else(|| "the regtest result has no provider".to_string())?;
    let status_history = SWAP_STAGES
        .iter()
        .enumerate()
        .map(|(sequence, stage)| {
            json!({
                "status_id": format!("{swap_id}-status-{sequence}"),
                "sequence": sequence,
                "previous": sequence.checked_sub(1).map(|previous| format!("{swap_id}-status-{previous}")),
                "stage": stage,
                "authority": if matches!(sequence, 1 | 2) { "provider_claim" } else { "requester_local" }
            })
        })
        .collect::<Vec<_>>();
    Ok(MarketToolOutput::success(json!({
        "schema": "omega.market-demo.swap.v1",
        "network": "regtest",
        "network_id": value.get("network_id"),
        "disclosure": REGTEST_DISCLOSURE,
        "swap_id": swap_id,
        "quote_id": details.quote_id,
        "request_id": value.get("request_id"),
        "from": details.from.as_str(),
        "to": details.to.as_str(),
        "amount_sats": details.amount_sats,
        "provider": provider,
        "fee_bps": null,
        "stage": "settled",
        "verification": value.get("verification"),
        "status_history": status_history,
        "projection": {
            "last_valid_status": format!("{swap_id}-status-3"),
            "status_gaps": [],
            "status_forks": [],
            "local_effects_verified": true
        },
        "rail_evidence": value.get("rail_evidence"),
        "quote_provider_pubkeys": value.get("quote_provider_pubkeys"),
        "unselected_released": value.get("unselected_released")
    })))
}

fn store_regtest_swap(
    state: &Mutex<MarketDemoState>,
    output: &MarketToolOutput,
) -> Result<(), String> {
    let swap_id = output
        .0
        .get("swap_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "the regtest output has no swap_id".to_string())?
        .to_string();
    state.lock().regtest_swaps.insert(swap_id, output.0.clone());
    Ok(())
}

fn regtest_swap_status(
    state: &Mutex<MarketDemoState>,
    swap_id: &str,
) -> Result<MarketToolOutput, String> {
    state
        .lock()
        .regtest_swaps
        .get(swap_id)
        .cloned()
        .map(MarketToolOutput::success)
        .ok_or_else(|| format!("unknown regtest swap_id {swap_id}"))
}

async fn load_network_status(http_client: Arc<HttpClientWithUrl>) -> Result<Value, String> {
    let mut response = http_client
        .get(MANIFEST_URL, AsyncBody::default(), true)
        .await
        .map_err(|error| format!("manifest request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "manifest request returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let mut body = Vec::new();
    response
        .body_mut()
        .read_to_end(&mut body)
        .await
        .map_err(|error| format!("manifest response could not be read: {error}"))?;
    let envelope: Value = serde_json::from_slice(&body)
        .map_err(|error| format!("manifest response was invalid JSON: {error}"))?;
    let manifest = envelope.get("manifest").unwrap_or(&envelope);
    let relay_urls = manifest
        .get("relays")
        .and_then(Value::as_array)
        .map(|relays| {
            relays
                .iter()
                .filter_map(|relay| relay.get("websocket_url").and_then(Value::as_str))
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|relays| !relays.is_empty())
        .unwrap_or_else(|| {
            FALLBACK_RELAYS
                .iter()
                .map(|url| (*url).to_string())
                .collect()
        });

    let relay_reads = relay_urls.iter().cloned().map(|url| {
        smol::unblock(move || {
            let result =
                omega_effectd::query_public_events(vec![url.clone()], &[39600, 39601], 256);
            (url, result)
        })
    });
    let relay_reads = futures::future::join_all(relay_reads).await;

    let mut all_events = Vec::new();
    let mut provider_relays: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut relays = Vec::new();
    for (url, result) in relay_reads {
        let label = relay_label(&url);
        match result {
            Ok(events) => {
                for event in &events {
                    provider_relays
                        .entry(event.public_key.clone())
                        .or_default()
                        .insert(label.clone());
                }
                all_events.extend(events);
                relays.push(json!({
                    "label": label,
                    "url": url,
                    "state": "ready",
                    "trust": "pinned"
                }));
            }
            Err(error) => relays.push(json!({
                "label": label,
                "url": url,
                "state": "offline",
                "trust": "pinned",
                "error": error.to_string()
            })),
        }
    }

    let mut heads: BTreeMap<(u16, String, String), PublicRelayEvent> = BTreeMap::new();
    for event in all_events {
        let key = (
            event.kind,
            event.public_key.clone(),
            tag_value(&event, "d").unwrap_or_default().to_string(),
        );
        if heads
            .get(&key)
            .is_none_or(|existing| event.created_at > existing.created_at)
        {
            heads.insert(key, event);
        }
    }

    let pinned_providers = manifest
        .get("providers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|provider| {
            Some((
                provider.get("pubkey")?.as_str()?.to_string(),
                provider.get("role")?.as_str()?.to_string(),
            ))
        })
        .collect::<BTreeMap<_, _>>();

    let mut providers = Vec::new();
    for event in heads
        .values()
        .filter(|event| event.kind == 39600 && tag_value(event, "status") == Some("active"))
    {
        let offerings = heads
            .values()
            .filter(|offering| {
                offering.kind == 39601
                    && offering.public_key == event.public_key
                    && tag_value(offering, "status") == Some("active")
            })
            .collect::<Vec<_>>();
        let minimum_fee = offerings
            .iter()
            .filter_map(|offering| minimum_fee_bps(&offering.content))
            .min();
        let profile_name = serde_json::from_str::<Value>(&event.content)
            .ok()
            .and_then(|profile| {
                profile
                    .get("name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            });
        let pinned_role = pinned_providers.get(&event.public_key);
        providers.push(json!({
            "label": pinned_role.cloned().or(profile_name).unwrap_or_else(|| event.public_key.chars().take(8).collect()),
            "pubkey": event.public_key,
            "state": if offerings.is_empty() { "starting" } else { "ready" },
            "trust": if pinned_role.is_some() { "pinned" } else { "discovered" },
            "relays": provider_relays.get(&event.public_key).cloned().unwrap_or_default(),
            "fee_bps": minimum_fee,
            "active_offerings": offerings.len()
        }));
    }
    for (public_key, role) in &pinned_providers {
        if !providers
            .iter()
            .any(|provider| provider.get("pubkey").and_then(Value::as_str) == Some(public_key))
        {
            providers.push(json!({
                "label": role,
                "pubkey": public_key,
                "state": "unavailable",
                "trust": "pinned",
                "relays": [],
                "fee_bps": null,
                "active_offerings": 0
            }));
        }
    }
    providers.sort_by(|left, right| {
        left.get("label")
            .and_then(Value::as_str)
            .cmp(&right.get("label").and_then(Value::as_str))
    });

    Ok(json!({
        "schema": "omega.market-demo.network-status.v1",
        "network": "regtest",
        "source": "live",
        "disclosure": LIVE_DISCLOSURE,
        "name": "public regtest",
        "manifest": {
            "service_state": manifest.get("service_state"),
            "bazaar_revision": manifest.get("bazaar_revision"),
            "immortal_revision": manifest.get("immortal_revision")
        },
        "relays": relays,
        "providers": providers,
        "stats": {}
    }))
}

fn quote_swap(
    state: &Mutex<MarketDemoState>,
    input: MarketSwapQuoteInput,
) -> Result<MarketToolOutput, String> {
    if input.network != MarketNetwork::Demo {
        return Err("the demo quote function requires network demo".into());
    }
    if input.from == input.to {
        return Err("from and to must be different assets".into());
    }
    if !(1_000..=10_000_000).contains(&input.amount_sats) {
        return Err("amount_sats must be between 1,000 and 10,000,000".into());
    }
    let mut state = state.lock();
    state.quote_counter = state.quote_counter.saturating_add(1);
    let quote_id = format!("demo-quote-{}", state.quote_counter);
    state.quotes.insert(
        quote_id.clone(),
        Quote {
            network: input.network,
            from: input.from,
            to: input.to,
            amount_sats: input.amount_sats,
        },
    );
    let fee_sats = input.amount_sats.saturating_mul(22).div_ceil(10_000);
    let miner_fee_budget_sats = 300;
    Ok(MarketToolOutput::success(json!({
        "schema": "omega.market-demo.quote.v1",
        "network": "demo",
        "disclosure": DEMO_DISCLOSURE,
        "quote_id": quote_id,
        "from": input.from.as_str(),
        "to": input.to.as_str(),
        "amount_sats": input.amount_sats,
        "provider": "provider-b",
        "fee_bps": 22,
        "fee_sats": fee_sats,
        "miner_fee_budget_sats": miner_fee_budget_sats,
        "output_sats": input.amount_sats.saturating_sub(fee_sats + miner_fee_budget_sats),
        "kind": "firm",
        "expires_in_seconds": 120
    })))
}

fn prepare_execution(
    state: &Mutex<MarketDemoState>,
    input: MarketExecuteSwapInput,
) -> Result<(String, Option<MarketToolOutput>), String> {
    match input {
        MarketExecuteSwapInput::Quoted { network, quote_id } => {
            if network != MarketNetwork::Demo {
                return Err("the demo execution function requires network demo".into());
            }
            Ok((quote_id, None))
        }
        MarketExecuteSwapInput::Direct {
            network,
            from,
            to,
            amount_sats,
        } => {
            if network != MarketNetwork::Demo {
                return Err("the demo execution function requires network demo".into());
            }
            let quote = quote_swap(
                state,
                MarketSwapQuoteInput {
                    network,
                    from,
                    to,
                    amount_sats,
                },
            )?;
            let quote_id = quote
                .0
                .get("quote_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| "new demo quote has no quote_id".to_string())?;
            Ok((quote_id, Some(quote)))
        }
    }
}

fn execute_swap(
    state: &Mutex<MarketDemoState>,
    quote_id: &str,
) -> Result<MarketToolOutput, String> {
    let mut state = state.lock();
    let quote = state
        .quotes
        .get(quote_id)
        .cloned()
        .ok_or_else(|| format!("unknown quote_id {quote_id}; request a fresh quote first"))?;
    if quote.network != MarketNetwork::Demo {
        return Err(format!("quote_id {quote_id} is not a demo quote"));
    }
    state.swap_counter = state.swap_counter.saturating_add(1);
    let swap = Swap {
        id: format!("demo-swap-{}", state.swap_counter),
        quote_id: quote_id.to_string(),
        from: quote.from,
        to: quote.to,
        amount_sats: quote.amount_sats,
        status_history: Vec::new(),
    };
    let swap_id = swap.id.clone();
    state.swaps.insert(swap_id.clone(), swap);
    let swap = state
        .swaps
        .get_mut(&swap_id)
        .ok_or_else(|| "new demo swap disappeared".to_string())?;
    append_next_status(swap)?;
    Ok(MarketToolOutput::success(swap_view(swap)?))
}

fn swap_status(
    state: &Mutex<MarketDemoState>,
    input: MarketSwapStatusInput,
) -> Result<MarketToolOutput, String> {
    if input.network != MarketNetwork::Demo {
        return Err("the demo status function requires network demo".into());
    }
    let state = state.lock();
    let swap = state
        .swaps
        .get(&input.swap_id)
        .ok_or_else(|| format!("unknown swap_id {}", input.swap_id))?;
    Ok(MarketToolOutput::success(swap_view(swap)?))
}

fn advance_swap(
    state: &Mutex<MarketDemoState>,
    swap_id: String,
) -> Result<MarketToolOutput, String> {
    let mut state = state.lock();
    let swap = state
        .swaps
        .get_mut(&swap_id)
        .ok_or_else(|| format!("unknown swap_id {swap_id}"))?;
    append_next_status(swap)?;
    Ok(MarketToolOutput::success(swap_view(swap)?))
}

fn append_next_status(swap: &mut Swap) -> Result<(), String> {
    let sequence = swap.status_history.len();
    let Some(stage) = SWAP_STAGES.get(sequence).copied() else {
        return Ok(());
    };
    let previous = swap.status_history.last().map(|status| status.id.clone());
    swap.status_history.push(SwapStatusRecord {
        id: format!("{}-status-{sequence}", swap.id),
        sequence,
        previous,
        stage,
    });
    project_swap(swap)?;
    Ok(())
}

fn project_swap(swap: &Swap) -> Result<&SwapStatusRecord, String> {
    if swap.status_history.is_empty() || swap.status_history.len() > SWAP_STAGES.len() {
        return Err(format!("swap {} has an invalid status history", swap.id));
    }
    for (index, status) in swap.status_history.iter().enumerate() {
        let expected_stage = SWAP_STAGES
            .get(index)
            .copied()
            .ok_or_else(|| format!("swap {} has an unknown status sequence", swap.id))?;
        let expected_previous = index
            .checked_sub(1)
            .and_then(|previous| swap.status_history.get(previous))
            .map(|previous| previous.id.as_str());
        if status.sequence != index
            || status.stage != expected_stage
            || status.previous.as_deref() != expected_previous
        {
            return Err(format!(
                "swap {} has a status gap, fork, or regression",
                swap.id
            ));
        }
    }
    swap.status_history
        .last()
        .ok_or_else(|| format!("swap {} has no current status", swap.id))
}

fn swap_view(swap: &Swap) -> Result<Value, String> {
    let current = project_swap(swap)?;
    let stage_index = current.sequence;
    let stage = current.stage;
    let verification = match stage {
        "contract" => "exit package persisted before any funding",
        "funding" | "executing" => "provider status is a claim · verifying locally",
        _ => "verified locally · zero-loss close",
    };
    let status_history = swap
        .status_history
        .iter()
        .map(|status| {
            json!({
                "status_id": status.id,
                "sequence": status.sequence,
                "previous": status.previous,
                "stage": status.stage,
                "authority": match status.stage {
                    "funding" | "executing" => "provider_claim",
                    _ => "requester_local",
                }
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": "omega.market-demo.swap.v1",
        "network": "demo",
        "disclosure": DEMO_DISCLOSURE,
        "swap_id": swap.id,
        "quote_id": swap.quote_id,
        "from": swap.from.as_str(),
        "to": swap.to.as_str(),
        "amount_sats": swap.amount_sats,
        "provider": "provider-b",
        "fee_bps": 22,
        "stage": stage,
        "verification": verification,
        "status_history": status_history,
        "projection": {
            "last_valid_status": current.id,
            "status_gaps": [],
            "status_forks": [],
            "local_effects_verified": stage == "settled"
        },
        "stages_completed": SWAP_STAGES.iter().take(stage_index).copied().collect::<Vec<_>>(),
        "stages_remaining": SWAP_STAGES.iter().skip(stage_index.saturating_add(1)).copied().collect::<Vec<_>>()
    }))
}

fn start_cloud_provision(
    state: &Mutex<MarketDemoState>,
    input: MarketProvisionCloudInput,
) -> Result<CloudProvision, String> {
    let provider_name = input
        .provider_name
        .unwrap_or_else(|| "Omega provider".to_string());
    let provider_name = provider_name.trim();
    if provider_name.is_empty() || provider_name.chars().count() > 48 {
        return Err("provider_name must contain from 1 through 48 characters".to_string());
    }
    if !provider_name
        .chars()
        .all(|character| character.is_alphanumeric() || " -_.".contains(character))
    {
        return Err(
            "provider_name may contain letters, numbers, spaces, hyphens, underscores, and periods"
                .to_string(),
        );
    }

    let mut state = state.lock();
    state.provision_counter = state.provision_counter.saturating_add(1);
    Ok(CloudProvision {
        id: format!("mock-cloud-{}", state.provision_counter),
        provider_name: provider_name.to_string(),
        region: input.region.unwrap_or_default(),
    })
}

fn cloud_provision_view(
    provision: &CloudProvision,
    stage_index: usize,
) -> Result<MarketToolOutput, String> {
    let stage = CLOUD_PROVISION_STAGES
        .get(stage_index)
        .copied()
        .ok_or_else(|| format!("cloud provision {} has an invalid stage", provision.id))?;
    let relay_ready = stage_index >= 1;
    let provider_ready = stage_index >= 2;
    let connected = stage_index >= 3;
    let completed_count = if connected {
        CLOUD_PROVISION_STAGES.len()
    } else {
        stage_index
    };

    Ok(MarketToolOutput::success(json!({
        "schema": "omega.market-demo.cloud-provision.v1",
        "disclosure": CLOUD_PROVISION_DISCLOSURE,
        "provision_id": provision.id,
        "provider_name": provision.provider_name,
        "region": provision.region.as_str(),
        "stage": stage,
        "billing": {
            "requirement": "paid_account",
            "status": "mock_paid",
            "mode": "mock"
        },
        "relay": {
            "id": format!("{}-relay", provision.id),
            "state": if relay_ready { "ready" } else { "pending" }
        },
        "provider": {
            "id": format!("{}-provider", provision.id),
            "state": if provider_ready { "ready" } else { "pending" }
        },
        "connection": {
            "cloud": "OpenAgents cloud",
            "state": if connected { "connected" } else { "pending" }
        },
        "stages_completed": CLOUD_PROVISION_STAGES
            .iter()
            .take(completed_count)
            .copied()
            .collect::<Vec<_>>(),
        "stages_remaining": CLOUD_PROVISION_STAGES
            .iter()
            .skip(stage_index.saturating_add(1))
            .copied()
            .collect::<Vec<_>>()
    })))
}

fn emit_market_update(event_stream: &ToolCallEventStream, output: &MarketToolOutput) {
    let is_quote =
        output.0.get("schema").and_then(Value::as_str) == Some("omega.market-demo.quote.v1");
    let stage = if is_quote {
        "quote"
    } else {
        output
            .0
            .get("stage")
            .and_then(Value::as_str)
            .unwrap_or("running")
    };
    let execution_id = output
        .0
        .get("swap_id")
        .or_else(|| output.0.get("quote_id"))
        .or_else(|| output.0.get("provision_id"))
        .and_then(Value::as_str)
        .unwrap_or("demo swap");
    let content = serde_json::to_string_pretty(&output.0)
        .unwrap_or_else(|error| format!("Failed to serialize market tool update: {error}"));
    event_stream.update_fields(
        acp::ToolCallUpdateFields::new()
            .title(format!("{execution_id}: {stage}"))
            .content(vec![acp::ToolCallContent::Content(acp::Content::new(
                content,
            ))]),
    );
}

impl MarketToolOutput {
    fn swap_id(&self) -> Result<String, MarketToolOutput> {
        self.0
            .get("swap_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| MarketToolOutput::error("demo swap output has no swap_id"))
    }
}

fn tag_value<'a>(event: &'a PublicRelayEvent, name: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .find(|tag| tag.first().map(String::as_str) == Some(name))
        .and_then(|tag| tag.get(1))
        .map(String::as_str)
}

fn minimum_fee_bps(content: &str) -> Option<u64> {
    serde_json::from_str::<Value>(content)
        .ok()?
        .pointer("/mkt_swp/sides")?
        .as_array()?
        .iter()
        .filter_map(|side| {
            side.get("fee_bps").and_then(|fee| {
                fee.as_u64()
                    .or_else(|| fee.as_str().and_then(|fee| fee.parse().ok()))
            })
        })
        .min()
}

fn relay_label(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.host_str()
                .and_then(|host| host.split('.').next())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt as _;
    use gpui::TestAppContext;

    #[test]
    fn new_market_inputs_still_require_an_explicit_network() {
        assert!(
            serde_json::from_value::<MarketNetworkStatusInput>(json!({})).is_err(),
            "network status must require an explicit network"
        );
        assert!(
            serde_json::from_value::<MarketSwapQuoteInput>(json!({
                "from": "LN",
                "to": "BTC",
                "amount_sats": 50_000
            }))
            .is_err(),
            "quotes must not silently select demo"
        );
        assert!(
            serde_json::from_value::<MarketExecuteSwapInput>(json!({
                "from": "LN",
                "to": "BTC",
                "amount_sats": 50_000
            }))
            .is_err(),
            "direct execution must not silently select demo"
        );
        assert!(
            serde_json::from_value::<MarketExecuteSwapInput>(json!({
                "quote_id": "demo-quote-1"
            }))
            .is_err(),
            "quoted execution must not silently select demo"
        );
        assert!(
            serde_json::from_value::<MarketSwapStatusInput>(json!({
                "swap_id": "demo-swap-1"
            }))
            .is_err(),
            "status reads must not silently select demo"
        );
    }

    #[test]
    fn legacy_market_inputs_migrate_to_their_historical_network_for_replay() {
        let mut network_status = json!({});
        <MarketNetworkStatusTool as AgentTool>::migrate_input_for_replay(&mut network_status);
        let network_status: MarketNetworkStatusInput =
            serde_json::from_value(network_status).expect("legacy network status input");
        assert_eq!(network_status.network, MarketNetwork::Regtest);

        let mut quote = json!({
            "from": "LN",
            "to": "BTC",
            "amount_sats": 50_000
        });
        <MarketSwapQuoteTool as AgentTool>::migrate_input_for_replay(&mut quote);
        let quote: MarketSwapQuoteInput =
            serde_json::from_value(quote).expect("legacy quote input");
        assert_eq!(quote.network, MarketNetwork::Demo);

        let mut direct_execution = json!({
            "from": "LN",
            "to": "BTC",
            "amount_sats": 50_000
        });
        <MarketExecuteSwapTool as AgentTool>::migrate_input_for_replay(&mut direct_execution);
        let direct_execution: MarketExecuteSwapInput =
            serde_json::from_value(direct_execution).expect("legacy direct execution input");
        assert!(matches!(
            direct_execution,
            MarketExecuteSwapInput::Direct {
                network: MarketNetwork::Demo,
                ..
            }
        ));

        let mut quoted_execution = json!({ "quote_id": "demo-quote-1" });
        <MarketExecuteSwapTool as AgentTool>::migrate_input_for_replay(&mut quoted_execution);
        let quoted_execution: MarketExecuteSwapInput =
            serde_json::from_value(quoted_execution).expect("legacy quoted execution input");
        assert!(matches!(
            quoted_execution,
            MarketExecuteSwapInput::Quoted {
                network: MarketNetwork::Demo,
                ..
            }
        ));

        let mut status = json!({ "swap_id": "demo-swap-1" });
        <MarketSwapStatusTool as AgentTool>::migrate_input_for_replay(&mut status);
        let status: MarketSwapStatusInput =
            serde_json::from_value(status).expect("legacy status input");
        assert_eq!(status.network, MarketNetwork::Demo);
    }

    #[gpui::test]
    fn erased_tool_replay_accepts_a_legacy_market_input(cx: &mut App) {
        let tool = MarketSwapStatusTool {
            state: Arc::new(Mutex::new(MarketDemoState::default())),
        }
        .erase();
        let (event_stream, _events) = ToolCallEventStream::test();

        let result = tool.replay(
            json!({ "swap_id": "demo-swap-1" }),
            json!({
                "schema": "omega.market-demo.swap.v1",
                "swap_id": "demo-swap-1",
                "network": "demo",
                "stage": "settled"
            }),
            event_stream,
            cx,
        );

        assert!(
            result.is_ok(),
            "legacy persisted market input should replay"
        );
    }

    #[test]
    fn demo_swap_flow_uses_shared_state() {
        let state = Mutex::new(MarketDemoState::default());
        let quote = quote_swap(
            &state,
            MarketSwapQuoteInput {
                network: MarketNetwork::Demo,
                from: MarketAsset::Lightning,
                to: MarketAsset::Bitcoin,
                amount_sats: 50_000,
            },
        )
        .expect("quote");
        assert_eq!(quote.0["schema"], "omega.market-demo.quote.v1");
        let quote_id = quote.0["quote_id"].as_str().expect("quote id").to_string();

        let swap = execute_swap(&state, &quote_id).expect("swap");
        assert_eq!(swap.0["stage"], "contract");
        let swap_id = swap.0["swap_id"].as_str().expect("swap id").to_string();
        let status = swap_status(
            &state,
            MarketSwapStatusInput {
                network: MarketNetwork::Demo,
                swap_id: swap_id.clone(),
            },
        )
        .expect("status");
        assert_eq!(status.0["stage"], "contract");

        let status = advance_swap(&state, swap_id.clone()).expect("advance");
        assert_eq!(status.0["stage"], "funding");
        let status = advance_swap(&state, swap_id.clone()).expect("advance");
        assert_eq!(status.0["stage"], "executing");
        let status = advance_swap(&state, swap_id.clone()).expect("advance");
        assert_eq!(status.0["stage"], "settled");

        let settled = swap_status(
            &state,
            MarketSwapStatusInput {
                network: MarketNetwork::Demo,
                swap_id,
            },
        )
        .expect("status");
        assert_eq!(settled.0["stage"], "settled");
        assert_eq!(
            settled.0["status_history"].as_array().map(Vec::len),
            Some(4)
        );
        assert_eq!(settled.0["projection"]["status_gaps"], json!([]));
        assert_eq!(settled.0["projection"]["status_forks"], json!([]));
    }

    #[gpui::test]
    async fn explicit_swap_streams_contiguous_lifecycle_without_second_authorization(
        cx: &mut TestAppContext,
    ) {
        let state = Arc::new(Mutex::new(MarketDemoState::default()));
        let tool = Arc::new(MarketExecuteSwapTool {
            state,
            http_client: None,
            stage_delay: Duration::ZERO,
        });
        let (event_stream, mut events) = ToolCallEventStream::test();

        let result = cx
            .update(|cx| {
                tool.run(
                    ToolInput::resolved(MarketExecuteSwapInput::Direct {
                        network: MarketNetwork::Demo,
                        from: MarketAsset::Lightning,
                        to: MarketAsset::Bitcoin,
                        amount_sats: 50_000,
                    }),
                    event_stream,
                    cx,
                )
            })
            .await
            .expect("swap should execute from the person's request");

        assert_eq!(result.0["stage"], "settled");
        assert_eq!(result.0["quote_id"], "demo-quote-1");
        for expected_stage in ["quote", "contract", "funding", "executing", "settled"] {
            let update = events.expect_update_fields().await;
            assert!(
                update
                    .title
                    .as_deref()
                    .is_some_and(|title| title.ends_with(expected_stage)),
                "expected a streamed {expected_stage} update, got {:?}",
                update.title
            );
        }
        assert!(events.next().await.is_none());
    }

    #[gpui::test]
    async fn paid_mock_provision_streams_relay_and_provider_connection(cx: &mut TestAppContext) {
        let tool = Arc::new(MarketProvisionCloudTool {
            state: Arc::new(Mutex::new(MarketDemoState::default())),
            stage_delay: Duration::ZERO,
        });
        let (event_stream, mut events) = ToolCallEventStream::test();

        let result = cx
            .update(|cx| {
                tool.run(
                    ToolInput::resolved(MarketProvisionCloudInput {
                        provider_name: Some("Northstar".to_string()),
                        region: Some(MarketCloudRegion::UsCentral1),
                    }),
                    event_stream,
                    cx,
                )
            })
            .await
            .expect("mock cloud provision should complete");

        assert_eq!(result.0["stage"], "connected");
        assert_eq!(result.0["billing"]["status"], "mock_paid");
        assert_eq!(result.0["relay"]["state"], "ready");
        assert_eq!(result.0["provider"]["state"], "ready");
        assert_eq!(result.0["connection"]["state"], "connected");
        for expected_stage in CLOUD_PROVISION_STAGES.iter().copied() {
            let update = events.expect_update_fields().await;
            assert!(
                update
                    .title
                    .as_deref()
                    .is_some_and(|title| title.ends_with(expected_stage)),
                "expected a streamed {expected_stage} update, got {:?}",
                update.title
            );
        }
        assert!(events.next().await.is_none());
    }

    #[test]
    fn cloud_provision_rejects_an_invalid_provider_name() {
        let result = start_cloud_provision(
            &Mutex::new(MarketDemoState::default()),
            MarketProvisionCloudInput {
                provider_name: Some("bad/provider".to_string()),
                region: None,
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn status_projection_rejects_a_gap_or_regression() {
        let mut swap = Swap {
            id: "demo-swap-gap".to_string(),
            quote_id: "demo-quote-gap".to_string(),
            from: MarketAsset::Lightning,
            to: MarketAsset::Bitcoin,
            amount_sats: 50_000,
            status_history: vec![SwapStatusRecord {
                id: "demo-swap-gap-status-1".to_string(),
                sequence: 1,
                previous: None,
                stage: "funding",
            }],
        };
        assert!(project_swap(&swap).is_err());

        swap.status_history[0].sequence = 0;
        swap.status_history[0].stage = "funding";
        assert!(project_swap(&swap).is_err());
    }

    #[test]
    fn quote_rejects_invalid_inputs() {
        let state = Mutex::new(MarketDemoState::default());
        let result = quote_swap(
            &state,
            MarketSwapQuoteInput {
                network: MarketNetwork::Demo,
                from: MarketAsset::Bitcoin,
                to: MarketAsset::Bitcoin,
                amount_sats: 999,
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn demo_network_status_is_a_labeled_fixture() {
        let status = demo_network_status();
        assert_eq!(status["network"], "demo");
        assert_eq!(status["source"], "fixture");
        assert_eq!(status["providers"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn mainnet_warning_is_blocked_without_state() {
        let state = MarketDemoState::default();
        let warning = mainnet_warning("market_execute_swap");
        assert_eq!(warning.0["network"], "mainnet");
        assert_eq!(warning.0["blocked"], true);
        assert!(state.quotes.is_empty());
        assert!(state.swaps.is_empty());
        assert!(state.regtest_swaps.is_empty());
    }

    #[test]
    fn regtest_has_a_closed_direction_and_amount_contract() {
        assert!(
            validate_regtest_direction(MarketAsset::Lightning, MarketAsset::Bitcoin, 100_000,)
                .is_ok()
        );
        assert!(
            validate_regtest_direction(MarketAsset::Bitcoin, MarketAsset::Lightning, 1_000_000,)
                .is_ok()
        );
        assert!(
            validate_regtest_direction(
                MarketAsset::Lightning,
                MarketAsset::LiquidBitcoin,
                100_000,
            )
                .is_err()
        );
        assert!(
            validate_regtest_direction(MarketAsset::Lightning, MarketAsset::Bitcoin, 99_999,)
                .is_err()
        );
    }
}
