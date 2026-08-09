use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v1 as acp;
use futures::AsyncReadExt as _;
use gpui::{App, AppContext as _, Task};
use http_client::{AsyncBody, HttpClientWithUrl};
use language_model::LanguageModelToolResultContent;
use omega_effectd::PublicRelayEvent;
use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ui::SharedString;

use crate::{AgentTool, ToolCallEventStream, ToolInput};

const MANIFEST_URL: &str = "https://bazaar.openagents.com/bazaar-public-regtest.json";
const FALLBACK_RELAYS: &[&str] = &[
    "wss://relay-a.34-41-78-122.nip.io",
    "wss://relay-b.34-41-78-122.sslip.io",
];
const LIVE_DISCLOSURE: &str = "LIVE public regtest coordination data, read-only; relay and provider claims are unverified until a requester verifies locally.";
const DEMO_DISCLOSURE: &str =
    "DEMO DATA: deterministic fixture, not the live network; no real funds move.";
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
}

#[derive(Clone)]
struct Quote {
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
/// Read the live public regtest swap network, including relay health and provider offerings.
pub struct MarketNetworkStatusInput {}

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

    fn initial_title(&self, _input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        "Check swap network".into()
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let http_client = self.http_client.clone();
        cx.spawn(async move |cx| {
            input
                .recv()
                .await
                .map_err(|error| MarketToolOutput::error(error.to_string()))?;
            cx.background_spawn(async move { load_network_status(http_client).await })
                .await
                .map(MarketToolOutput::success)
                .map_err(MarketToolOutput::error)
        })
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
/// Request a demo firm quote between LN, BTC, and L-BTC. No real funds move.
pub struct MarketSwapQuoteInput {
    from: MarketAsset,
    to: MarketAsset,
    /// Swap amount in sats, from 1,000 through 10,000,000.
    amount_sats: u64,
}

pub struct MarketSwapQuoteTool {
    state: Arc<Mutex<MarketDemoState>>,
}

impl AgentTool for MarketSwapQuoteTool {
    type Input = MarketSwapQuoteInput;
    type Output = MarketToolOutput;

    const NAME: &'static str = "market_swap_quote";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
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
            Err(_) => "Quote demo swap".into(),
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
            quote_swap(&self.state, input).map_err(MarketToolOutput::error)
        })
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
/// Quote and execute an authorized demo swap, or execute a prior demo quote. No real funds move.
pub enum MarketExecuteSwapInput {
    Quoted {
        quote_id: String,
    },
    Direct {
        from: MarketAsset,
        to: MarketAsset,
        /// Swap amount in sats, from 1,000 through 10,000,000.
        amount_sats: u64,
    },
}

pub struct MarketExecuteSwapTool {
    state: Arc<Mutex<MarketDemoState>>,
    stage_delay: Duration,
}

impl AgentTool for MarketExecuteSwapTool {
    type Input = MarketExecuteSwapInput;
    type Output = MarketToolOutput;

    const NAME: &'static str = "market_execute_swap";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Execute
    }

    fn initial_title(&self, input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        match input {
            Ok(MarketExecuteSwapInput::Quoted { quote_id }) => {
                format!("Execute demo swap for {quote_id}").into()
            }
            Ok(MarketExecuteSwapInput::Direct {
                from,
                to,
                amount_sats,
            }) => format!(
                "Swap {} sats {} → {}",
                amount_sats,
                from.as_str(),
                to.as_str()
            )
            .into(),
            Err(_) => "Execute demo swap".into(),
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
/// Read the current stage of a demo swap without changing its lifecycle.
pub struct MarketSwapStatusInput {
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

    fn initial_title(&self, input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        match input {
            Ok(input) => format!("Check demo swap {}", input.swap_id).into(),
            Err(_) => "Check demo swap".into(),
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
            swap_status(&self.state, input).map_err(MarketToolOutput::error)
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
        MarketNetworkStatusTool::new(http_client),
        MarketSwapQuoteTool {
            state: state.clone(),
        },
        MarketExecuteSwapTool {
            state: state.clone(),
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
            from: input.from,
            to: input.to,
            amount_sats: input.amount_sats,
        },
    );
    let fee_sats = input.amount_sats.saturating_mul(22).div_ceil(10_000);
    let miner_fee_budget_sats = 300;
    Ok(MarketToolOutput::success(json!({
        "schema": "omega.market-demo.quote.v1",
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
        MarketExecuteSwapInput::Quoted { quote_id } => Ok((quote_id, None)),
        MarketExecuteSwapInput::Direct {
            from,
            to,
            amount_sats,
        } => {
            let quote = quote_swap(
                state,
                MarketSwapQuoteInput {
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
    fn demo_swap_flow_uses_shared_state() {
        let state = Mutex::new(MarketDemoState::default());
        let quote = quote_swap(
            &state,
            MarketSwapQuoteInput {
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

        let settled = swap_status(&state, MarketSwapStatusInput { swap_id }).expect("status");
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
            stage_delay: Duration::ZERO,
        });
        let (event_stream, mut events) = ToolCallEventStream::test();

        let result = cx
            .update(|cx| {
                tool.run(
                    ToolInput::resolved(MarketExecuteSwapInput::Direct {
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
                from: MarketAsset::Bitcoin,
                to: MarketAsset::Bitcoin,
                amount_sats: 999,
            },
        );
        assert!(result.is_err());
    }
}
