//! Renders market tool results as the inline market cards.
//!
//! When a tool call's result carries an `omega.market-demo.*` schema (Omega's
//! built-in market tools or the external demo MCP server), the transcript
//! draws the typed card — the network panorama
//! or the swap lifecycle card — instead of raw JSON prose. Detection is by
//! payload schema, not tool name alone, so it works identically for the
//! native agent's MCP tools and for external ACP agents.

use acp_thread::{ToolCall, ToolCallContent, ToolCallStatus};
use gpui::{AnyElement, App};
use serde_json::Value;
use ui::prelude::*;
use ui::{
    NetworkCard, PanoramaNetwork, PanoramaProvider, PanoramaRelay, PanoramaStats, PanoramaTrust,
    SwapAsset, SwapCard, SwapStage, VizNodeState,
};

pub(crate) const MARKET_SCHEMA_PREFIX: &str = "omega.market-demo.";

const MARKET_TOOL_NAMES: [&str; 4] = [
    "market_network_status",
    "market_swap_quote",
    "market_execute_swap",
    "market_swap_status",
];

/// Cheap, context-free check used to keep market tool calls out of the
/// collapsed tool-chip groups so their cards render standalone.
pub(crate) fn is_market_tool_call(tool_call: &ToolCall) -> bool {
    market_signals(
        tool_call.tool_name.as_deref(),
        tool_call.raw_input.as_ref(),
        tool_call.raw_output.as_ref(),
    )
}

/// Detection from the context-free fields alone. Adapters differ: the native
/// agent sets `tool_name`; codex-acp sets neither name nor a flat payload but
/// wraps the call as `raw_input: {server, tool, arguments}` and
/// `raw_output: {result, error}`.
fn market_signals(
    tool_name: Option<&str>,
    raw_input: Option<&Value>,
    raw_output: Option<&Value>,
) -> bool {
    if let Some(name) = tool_name
        && MARKET_TOOL_NAMES.iter().any(|tool| name.contains(tool))
    {
        return true;
    }
    if let Some(input) = raw_input {
        let named = [input.get("tool"), input.get("server")]
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|name| {
                name.contains("market-demo")
                    || MARKET_TOOL_NAMES.iter().any(|tool| name.contains(tool))
            });
        if named {
            return true;
        }
    }
    raw_output.is_some_and(|raw| extract_market_payload(raw).is_some())
}

/// The card for a market tool call with a renderable snapshot, or `None` to
/// fall through to the ordinary tool-call rendering.
pub(crate) fn market_tool_card(tool_call: &ToolCall, cx: &App) -> Option<AnyElement> {
    if !market_card_status_is_renderable(&tool_call.status) {
        return None;
    }
    let payload = market_payload(tool_call, cx)?;
    match payload.get("schema")?.as_str()? {
        "omega.market-demo.network-status.v1" => {
            Some(NetworkCard::new(parse_network(&payload)?).into_any_element())
        }
        "omega.market-demo.quote.v1" | "omega.market-demo.swap.v1" => {
            Some(parse_swap_card(&payload)?.into_any_element())
        }
        _ => None,
    }
}

fn market_card_status_is_renderable(status: &ToolCallStatus) -> bool {
    matches!(
        status,
        ToolCallStatus::Pending | ToolCallStatus::InProgress | ToolCallStatus::Completed
    )
}

fn schema_of(value: &Value) -> Option<String> {
    let schema = value.get("schema")?.as_str()?;
    schema
        .starts_with(MARKET_SCHEMA_PREFIX)
        .then(|| schema.to_string())
}

/// Digs the market payload out of an arbitrary tool-result value. Hosts nest
/// it differently: the payload itself, codex-acp's `{result, error}` wrapper,
/// an MCP `content`/`structuredContent` envelope, or a JSON string — each
/// layer is peeled recursively.
fn extract_market_payload(value: &Value) -> Option<Value> {
    if schema_of(value).is_some() {
        return Some(value.clone());
    }
    if let Some(text) = value.as_str() {
        return parse_json_text(text);
    }
    for key in ["result", "structuredContent", "structured_content"] {
        if let Some(inner) = value.get(key)
            && let Some(found) = extract_market_payload(inner)
        {
            return Some(found);
        }
    }
    if let Some(content) = value.get("content").and_then(Value::as_array) {
        for entry in content {
            if let Some(text) = entry.get("text").and_then(Value::as_str)
                && let Some(found) = parse_json_text(text)
            {
                return Some(found);
            }
        }
    }
    None
}

/// The payload for rendering: the structured output when present, else the
/// rendered text content.
fn market_payload(tool_call: &ToolCall, cx: &App) -> Option<Value> {
    if let Some(raw) = &tool_call.raw_output
        && let Some(value) = extract_market_payload(raw)
    {
        return Some(value);
    }
    for content in &tool_call.content {
        if let ToolCallContent::ContentBlock(block) = content
            && let Some(value) = parse_json_text(&block.to_markdown(cx))
        {
            return Some(value);
        }
    }
    None
}

fn parse_json_text(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|rest| rest.trim_end_matches("```"))
        .unwrap_or(trimmed);
    let value: Value = serde_json::from_str(trimmed.trim()).ok()?;
    schema_of(&value)?;
    Some(value)
}

fn parse_state(value: &Value) -> VizNodeState {
    match value.as_str().unwrap_or_default() {
        "starting" => VizNodeState::Starting,
        "degraded" => VizNodeState::Degraded,
        "offline" => VizNodeState::Offline,
        _ => VizNodeState::Ready,
    }
}

fn parse_trust(value: &Value) -> PanoramaTrust {
    match value.as_str().unwrap_or_default() {
        "discovered" => PanoramaTrust::Discovered,
        _ => PanoramaTrust::Pinned,
    }
}

fn parse_network(payload: &Value) -> Option<PanoramaNetwork> {
    let relay_values = payload.get("relays")?.as_array()?;
    let relay_labels: Vec<&str> = relay_values
        .iter()
        .map(|relay| relay.get("label").and_then(Value::as_str).unwrap_or(""))
        .collect();
    let relays: Vec<PanoramaRelay> = relay_values
        .iter()
        .map(|relay| PanoramaRelay {
            label: relay
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("relay")
                .to_string()
                .into(),
            state: parse_state(relay.get("state").unwrap_or(&Value::Null)),
            trust: parse_trust(relay.get("trust").unwrap_or(&Value::Null)),
        })
        .collect();

    let providers: Vec<PanoramaProvider> = payload
        .get("providers")?
        .as_array()?
        .iter()
        .map(|provider| {
            let relay_indices = provider
                .get("relays")
                .and_then(Value::as_array)
                .map(|homes| {
                    homes
                        .iter()
                        .filter_map(Value::as_str)
                        .filter_map(|home| relay_labels.iter().position(|label| *label == home))
                        .collect()
                })
                .unwrap_or_default();
            PanoramaProvider {
                label: provider
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("provider")
                    .to_string()
                    .into(),
                state: parse_state(provider.get("state").unwrap_or(&Value::Null)),
                trust: parse_trust(provider.get("trust").unwrap_or(&Value::Null)),
                relay_indices,
                fee_bps: provider.get("fee_bps").and_then(Value::as_u64).unwrap_or(0) as u32,
                volume_sat_24h: provider
                    .get("volume_sat_24h")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            }
        })
        .collect();

    let stats = payload.get("stats").cloned().unwrap_or(Value::Null);
    let swaps_24h = stats.get("swaps_24h").and_then(Value::as_u64);
    let any_relay_ready = relays
        .iter()
        .any(|relay| relay.state == VizNodeState::Ready);
    let activity = if any_relay_ready {
        (0.1 + swaps_24h.unwrap_or(0) as f32 / 400.0).min(1.0)
    } else {
        0.0
    };

    Some(PanoramaNetwork {
        name: payload
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("market")
            .to_string()
            .into(),
        relays,
        providers,
        // The one client this snapshot can vouch for is the caller's own
        // session; a crowd is never invented.
        client_count: 1,
        stats: PanoramaStats {
            swaps_24h,
            volume_sat_24h: stats.get("volume_sat_24h").and_then(Value::as_u64),
            operator_fee_sat_24h: stats.get("operator_fee_sat_24h").and_then(Value::as_u64),
        },
        activity,
    })
}

fn parse_asset(value: &Value) -> Option<SwapAsset> {
    match value.as_str()? {
        "LN" => Some(SwapAsset::Lightning),
        "BTC" => Some(SwapAsset::Bitcoin),
        "L-BTC" => Some(SwapAsset::Liquid),
        _ => None,
    }
}

fn parse_swap_card(payload: &Value) -> Option<SwapCard> {
    let stage = if payload.get("schema")?.as_str()? == "omega.market-demo.quote.v1" {
        SwapStage::Quote
    } else {
        parse_swap_stage(payload)?
    };
    Some(
        SwapCard::new(
            parse_asset(payload.get("from")?)?,
            parse_asset(payload.get("to")?)?,
            payload.get("amount_sats")?.as_u64()?,
            payload
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or("provider")
                .to_string(),
            payload.get("fee_bps").and_then(Value::as_u64).unwrap_or(0) as u32,
        )
        .stage(stage),
    )
}

fn parse_swap_stage(payload: &Value) -> Option<SwapStage> {
    if let Some(history) = payload.get("status_history").and_then(Value::as_array) {
        let stage = project_swap_stage(history)?;
        if payload.get("stage").and_then(Value::as_str) != Some(stage.label()) {
            return None;
        }
        return Some(stage);
    }
    swap_stage(payload.get("stage")?.as_str()?)
}

fn project_swap_stage(history: &[Value]) -> Option<SwapStage> {
    const EXPECTED_STAGES: [&str; 4] = ["contract", "funding", "executing", "settled"];
    if history.is_empty() || history.len() > EXPECTED_STAGES.len() {
        return None;
    }
    let mut previous_status_id: Option<&str> = None;
    let mut status_ids = Vec::with_capacity(history.len());
    for (sequence, status) in history.iter().enumerate() {
        let status_id = status.get("status_id")?.as_str()?;
        if status_ids.contains(&status_id) {
            return None;
        }
        let previous = status.get("previous")?;
        let previous = if previous.is_null() {
            None
        } else {
            Some(previous.as_str()?)
        };
        if status.get("sequence")?.as_u64()? != u64::try_from(sequence).ok()?
            || status.get("stage")?.as_str()? != *EXPECTED_STAGES.get(sequence)?
            || previous != previous_status_id
        {
            return None;
        }
        let expected_authority = if matches!(sequence, 1 | 2) {
            "provider_claim"
        } else {
            "requester_local"
        };
        if status.get("authority")?.as_str()? != expected_authority {
            return None;
        }
        status_ids.push(status_id);
        previous_status_id = Some(status_id);
    }
    swap_stage(history.last()?.get("stage")?.as_str()?)
}

fn swap_stage(stage: &str) -> Option<SwapStage> {
    match stage {
        "contract" => Some(SwapStage::Contract),
        "funding" => Some(SwapStage::Funding),
        "executing" => Some(SwapStage::Executing),
        "settled" => Some(SwapStage::Settled),
        "refunded" => Some(SwapStage::Refunded),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_network_json() -> Value {
        serde_json::json!({
            "schema": "omega.market-demo.network-status.v1",
            "name": "public regtest (demo)",
            "relays": [
                {"label": "relay-a", "state": "ready", "trust": "pinned"},
                {"label": "relay-b", "state": "offline", "trust": "pinned"},
            ],
            "providers": [
                {
                    "label": "provider-b",
                    "state": "ready",
                    "trust": "pinned",
                    "relays": ["relay-a", "relay-b"],
                    "fee_bps": 22,
                    "volume_sat_24h": 5100000,
                },
                {
                    "label": "joiner",
                    "state": "degraded",
                    "trust": "discovered",
                    "relays": ["relay-b"],
                    "fee_bps": 30,
                    "volume_sat_24h": 150000,
                },
            ],
            "stats": {"swaps_24h": 128, "volume_sat_24h": 7650000},
        })
    }

    #[test]
    fn the_network_payload_parses_into_the_panorama_shape() {
        let network = parse_network(&demo_network_json()).expect("network parses");
        assert_eq!(network.relays.len(), 2);
        assert_eq!(network.relays[1].state, VizNodeState::Offline);
        assert_eq!(network.providers.len(), 2);
        assert_eq!(network.providers[0].relay_indices, vec![0, 1]);
        assert_eq!(network.providers[1].trust, PanoramaTrust::Discovered);
        assert_eq!(network.stats.swaps_24h, Some(128));
        assert_eq!(network.stats.operator_fee_sat_24h, None);
        assert!(network.activity > 0.4 && network.activity <= 1.0);
    }

    #[test]
    fn swap_payloads_parse_for_quotes_and_every_stage() {
        let quote = serde_json::json!({
            "schema": "omega.market-demo.quote.v1",
            "from": "LN", "to": "BTC", "amount_sats": 50000,
            "provider": "provider-b", "fee_bps": 22,
        });
        assert!(parse_swap_card(&quote).is_some());
        for stage in ["contract", "funding", "executing", "settled", "refunded"] {
            let swap = serde_json::json!({
                "schema": "omega.market-demo.swap.v1",
                "from": "LN", "to": "BTC", "amount_sats": 50000,
                "provider": "provider-b", "fee_bps": 22, "stage": stage,
            });
            assert!(parse_swap_card(&swap).is_some(), "stage {stage} parses");
        }
        let unknown_stage = serde_json::json!({
            "schema": "omega.market-demo.swap.v1",
            "from": "LN", "to": "BTC", "amount_sats": 50000, "stage": "melted",
        });
        assert!(parse_swap_card(&unknown_stage).is_none());
    }

    #[test]
    fn swap_stage_is_projected_from_contiguous_status_history() {
        let mut swap = serde_json::json!({
            "schema": "omega.market-demo.swap.v1",
            "from": "LN", "to": "BTC", "amount_sats": 50000,
            "provider": "provider-b", "fee_bps": 22, "stage": "executing",
            "status_history": [
                {
                    "status_id": "status-0", "sequence": 0, "previous": null,
                    "stage": "contract", "authority": "requester_local"
                },
                {
                    "status_id": "status-1", "sequence": 1, "previous": "status-0",
                    "stage": "funding", "authority": "provider_claim"
                },
                {
                    "status_id": "status-2", "sequence": 2, "previous": "status-1",
                    "stage": "executing", "authority": "provider_claim"
                }
            ]
        });
        assert_eq!(parse_swap_stage(&swap), Some(SwapStage::Executing));

        swap["status_history"][2]["previous"] = Value::String("status-0".to_string());
        assert!(parse_swap_stage(&swap).is_none());
    }

    #[test]
    fn in_progress_market_calls_can_render_streamed_cards() {
        assert!(market_card_status_is_renderable(
            &ToolCallStatus::InProgress
        ));
        assert!(market_card_status_is_renderable(&ToolCallStatus::Completed));
        assert!(!market_card_status_is_renderable(&ToolCallStatus::Failed));
    }

    /// codex-acp wraps MCP calls as `raw_input: {server, tool, arguments}`
    /// and `raw_output: {result: <MCP result>, error}` with no `tool_name`;
    /// the first shipped detector missed both, so codex tool calls rendered
    /// as generic chips instead of cards.
    #[test]
    fn codex_acp_wrapped_payloads_are_recognized() {
        let raw_input = serde_json::json!({
            "server": "market-demo",
            "tool": "market_network_status",
            "arguments": {},
        });
        assert!(market_signals(None, Some(&raw_input), None));

        let raw_output = serde_json::json!({
            "result": {
                "content": [
                    {"type": "text", "text": demo_network_json().to_string()},
                ],
            },
            "error": null,
        });
        assert!(market_signals(None, None, Some(&raw_output)));
        let payload = extract_market_payload(&raw_output).expect("payload extracted");
        assert!(parse_network(&payload).is_some());

        let structured = serde_json::json!({
            "result": {"structuredContent": demo_network_json()},
        });
        assert!(extract_market_payload(&structured).is_some());

        let foreign = serde_json::json!({
            "result": {"content": [{"type": "text", "text": "{\"schema\": \"other.v1\"}"}]},
        });
        assert!(!market_signals(None, None, Some(&foreign)));
        assert!(!market_signals(
            Some("shell_command"),
            Some(&serde_json::json!({"command": "ls"})),
            None
        ));
    }

    #[test]
    fn fenced_and_enveloped_payloads_are_recognized() {
        let text = format!("```json\n{}\n```", demo_network_json());
        assert!(parse_json_text(&text).is_some());
        assert!(parse_json_text("not json").is_none());
        assert!(
            parse_json_text("{\"schema\": \"other.thing.v1\"}").is_none(),
            "foreign schemas are not claimed"
        );
    }
}
