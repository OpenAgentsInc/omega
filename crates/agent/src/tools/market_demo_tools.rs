use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

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

use crate::{AgentTool, ToolCallEventStream, ToolInput, ToolPermissionContext};

const MANIFEST_URL: &str = "https://bazaar.openagents.com/bazaar-public-regtest.json";
const FALLBACK_RELAYS: &[&str] = &[
    "wss://relay-a.34-41-78-122.nip.io",
    "wss://relay-b.34-41-78-122.sslip.io",
];
const LIVE_DISCLOSURE: &str = "LIVE public regtest coordination data, read-only; relay and provider claims are unverified until a requester verifies locally.";
const DEMO_DISCLOSURE: &str =
    "DEMO DATA: deterministic fixture, not the live network; no real funds move.";
const SWAP_STAGES: &[&str] = &["contract", "funding", "executing", "settled"];

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
    from: MarketAsset,
    to: MarketAsset,
    amount_sats: u64,
    stage_index: usize,
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
/// Execute a demo quote after the person approves it. No real funds move.
pub struct MarketExecuteSwapInput {
    quote_id: String,
}

pub struct MarketExecuteSwapTool {
    state: Arc<Mutex<MarketDemoState>>,
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
            Ok(input) => format!("Execute demo swap for {}", input.quote_id).into(),
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
            let authorize = cx.update(|cx| {
                event_stream.authorize_always_prompt(
                    format!("Execute demo swap for {}", input.quote_id),
                    ToolPermissionContext::new(Self::NAME, vec![input.quote_id.clone()]),
                    cx,
                )
            });
            authorize
                .await
                .map_err(|error| MarketToolOutput::error(error.to_string()))?;
            execute_swap(&self.state, input).map_err(MarketToolOutput::error)
        })
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
/// Read the current stage of a demo swap. Each read advances the fixture by one stage.
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

pub fn market_demo_tools(
    http_client: Arc<HttpClientWithUrl>,
) -> (
    MarketNetworkStatusTool,
    MarketSwapQuoteTool,
    MarketExecuteSwapTool,
    MarketSwapStatusTool,
) {
    let state = Arc::new(Mutex::new(MarketDemoState::default()));
    (
        MarketNetworkStatusTool::new(http_client),
        MarketSwapQuoteTool {
            state: state.clone(),
        },
        MarketExecuteSwapTool {
            state: state.clone(),
        },
        MarketSwapStatusTool { state },
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

fn execute_swap(
    state: &Mutex<MarketDemoState>,
    input: MarketExecuteSwapInput,
) -> Result<MarketToolOutput, String> {
    let mut state = state.lock();
    let quote = state.quotes.get(&input.quote_id).cloned().ok_or_else(|| {
        format!(
            "unknown quote_id {}; request a fresh quote first",
            input.quote_id
        )
    })?;
    state.swap_counter = state.swap_counter.saturating_add(1);
    let swap = Swap {
        id: format!("demo-swap-{}", state.swap_counter),
        from: quote.from,
        to: quote.to,
        amount_sats: quote.amount_sats,
        stage_index: 0,
    };
    state.swaps.insert(swap.id.clone(), swap.clone());
    Ok(MarketToolOutput::success(swap_view(&swap)))
}

fn swap_status(
    state: &Mutex<MarketDemoState>,
    input: MarketSwapStatusInput,
) -> Result<MarketToolOutput, String> {
    let mut state = state.lock();
    let swap = state
        .swaps
        .get_mut(&input.swap_id)
        .ok_or_else(|| format!("unknown swap_id {}", input.swap_id))?;
    if swap.stage_index < SWAP_STAGES.len().saturating_sub(1) {
        swap.stage_index += 1;
    }
    Ok(MarketToolOutput::success(swap_view(swap)))
}

fn swap_view(swap: &Swap) -> Value {
    let stage = SWAP_STAGES
        .get(swap.stage_index)
        .copied()
        .unwrap_or("settled");
    let verification = match stage {
        "contract" => "exit package persisted before any funding",
        "funding" | "executing" => "provider status is a claim · verifying locally",
        _ => "verified locally · zero-loss close",
    };
    json!({
        "schema": "omega.market-demo.swap.v1",
        "disclosure": DEMO_DISCLOSURE,
        "swap_id": swap.id,
        "from": swap.from.as_str(),
        "to": swap.to.as_str(),
        "amount_sats": swap.amount_sats,
        "provider": "provider-b",
        "fee_bps": 22,
        "stage": stage,
        "verification": verification,
        "stages_completed": SWAP_STAGES.iter().take(swap.stage_index).copied().collect::<Vec<_>>(),
        "stages_remaining": SWAP_STAGES.iter().skip(swap.stage_index.saturating_add(1)).copied().collect::<Vec<_>>()
    })
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

        let swap = execute_swap(&state, MarketExecuteSwapInput { quote_id }).expect("swap");
        assert_eq!(swap.0["stage"], "contract");
        let swap_id = swap.0["swap_id"].as_str().expect("swap id").to_string();
        let status = swap_status(&state, MarketSwapStatusInput { swap_id }).expect("status");
        assert_eq!(status.0["stage"], "funding");
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
