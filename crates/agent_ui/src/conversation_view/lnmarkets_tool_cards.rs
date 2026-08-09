use acp_thread::{ToolCall, ToolCallContent, ToolCallStatus};
use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, FontWeight, Window, div};
use serde_json::{Value, json};
use ui::prelude::*;

const LNM_SCHEMA_PREFIX: &str = "omega.lnmarkets.";
const LNM_TOOL_NAMES: [&str; 4] = [
    "lnmarkets_features",
    "lnmarkets_ledger",
    "lnmarkets_strategy",
    "lnmarkets_mandate",
];

pub(crate) fn is_lnmarkets_card_tool_call(tool_call: &ToolCall) -> bool {
    tool_call.tool_name.as_deref().is_some_and(|name| {
        LNM_TOOL_NAMES
            .iter()
            .any(|tool_name| name.contains(tool_name))
    }) || tool_call
        .raw_output
        .as_ref()
        .is_some_and(|value| extract_payload(value).is_some())
}

pub(crate) fn lnmarkets_tool_card(tool_call: &ToolCall, cx: &App) -> Option<AnyElement> {
    if !matches!(
        tool_call.status,
        ToolCallStatus::Pending
            | ToolCallStatus::InProgress
            | ToolCallStatus::Completed
            | ToolCallStatus::Failed
    ) {
        return None;
    }
    render_payload(&tool_payload(tool_call, cx)?, cx)
}

fn tool_payload(tool_call: &ToolCall, cx: &App) -> Option<Value> {
    if let Some(raw_output) = &tool_call.raw_output
        && let Some(payload) = extract_payload(raw_output)
    {
        return Some(payload);
    }
    tool_call.content.iter().find_map(|content| {
        let ToolCallContent::ContentBlock(block) = content else {
            return None;
        };
        parse_payload_text(&block.to_markdown(cx))
    })
}

fn extract_payload(value: &Value) -> Option<Value> {
    if schema(value).is_some() {
        return Some(value.clone());
    }
    if let Some(text) = value.as_str() {
        return parse_payload_text(text);
    }
    for key in ["result", "structuredContent", "structured_content"] {
        if let Some(inner) = value.get(key)
            && let Some(payload) = extract_payload(inner)
        {
            return Some(payload);
        }
    }
    value
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| {
            content.iter().find_map(|entry| {
                entry
                    .get("text")
                    .and_then(Value::as_str)
                    .and_then(parse_payload_text)
            })
        })
}

fn parse_payload_text(text: &str) -> Option<Value> {
    let text = text.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .map(|rest| rest.trim_end_matches("```"))
        .unwrap_or(text);
    let value = serde_json::from_str::<Value>(text.trim()).ok()?;
    schema(&value)?;
    Some(value)
}

fn schema(value: &Value) -> Option<&str> {
    let schema = value.get("schema")?.as_str()?;
    schema.starts_with(LNM_SCHEMA_PREFIX).then_some(schema)
}

fn render_payload(payload: &Value, cx: &App) -> Option<AnyElement> {
    match schema(payload)? {
        "omega.lnmarkets.features.v1" => render_features(payload, cx),
        "omega.lnmarkets.ledger.v1" => render_ledger(payload, cx),
        "omega.lnmarkets.strategy.v1" => render_strategy(payload, cx),
        "omega.lnmarkets.mandate.v1" => render_mandate(payload, cx),
        _ => None,
    }
}

fn render_features(payload: &Value, cx: &App) -> Option<AnyElement> {
    let status = string_field(payload, "status", "collecting");
    let collector = payload.get("collector");
    let features = payload.get("features");
    let rows = vec![
        (
            "Network".to_string(),
            collector
                .and_then(|value| value.get("network"))
                .and_then(Value::as_str)
                .unwrap_or("signet")
                .to_string(),
        ),
        (
            "Stored events".to_string(),
            collector
                .and_then(|value| value.get("stored_events"))
                .and_then(Value::as_u64)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "0".to_string()),
        ),
        (
            "As of".to_string(),
            features
                .and_then(|value| value.get("as_of_ms"))
                .and_then(Value::as_i64)
                .map(|value| format!("{value} ms"))
                .unwrap_or_else(|| "waiting for data".to_string()),
        ),
    ];
    Some(render_card("Derived market features", &status, rows, cx))
}

fn render_ledger(payload: &Value, cx: &App) -> Option<AnyElement> {
    let report = payload.get("report")?;
    let total_profit = report
        .get("total_profit_sats")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let status = if string_field(payload, "status", "empty") == "empty" {
        "empty".to_string()
    } else if total_profit < 0 {
        "drawdown".to_string()
    } else {
        "profit".to_string()
    };
    Some(render_card(
        "Strategy profit ledger",
        &status,
        vec![
            ("Profit".to_string(), format!("{total_profit} sats")),
            (
                "Fees".to_string(),
                format!(
                    "{} sats",
                    report
                        .get("total_fees_paid_sats")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                ),
            ),
            (
                "Funding".to_string(),
                format!(
                    "{} sats",
                    report
                        .get("total_funding_collected_sats")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                ),
            ),
            (
                "Worst drawdown".to_string(),
                format!(
                    "{} sats",
                    report
                        .get("worst_drawdown_sats")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                ),
            ),
        ],
        cx,
    ))
}

fn render_strategy(payload: &Value, cx: &App) -> Option<AnyElement> {
    let status = string_field(payload, "status", "idle");
    let rows = payload
        .get("strategies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|strategy| {
            (
                string_field(strategy, "strategy_id", "strategy"),
                string_field(strategy, "status", "idle"),
            )
        })
        .collect::<Vec<_>>();
    Some(render_card("Trading strategies", &status, rows, cx))
}

fn render_mandate(payload: &Value, cx: &App) -> Option<AnyElement> {
    let status = string_field(payload, "status", "missing");
    let snapshot = payload.get("snapshot")?;
    let mandate = snapshot.get("mandate");
    Some(render_card(
        "Trading mandate",
        &status,
        vec![
            (
                "Revision".to_string(),
                snapshot
                    .get("revision")
                    .and_then(Value::as_u64)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "0".to_string()),
            ),
            (
                "Network".to_string(),
                mandate
                    .and_then(|value| value.get("network"))
                    .and_then(Value::as_str)
                    .unwrap_or("not configured")
                    .to_string(),
            ),
            (
                "Objective".to_string(),
                mandate
                    .and_then(|value| value.get("objective"))
                    .and_then(Value::as_str)
                    .unwrap_or("Approve a mandate in Settings")
                    .to_string(),
            ),
        ],
        cx,
    ))
}

fn render_card(title: &str, status: &str, rows: Vec<(String, String)>, cx: &App) -> AnyElement {
    let card_id = match title {
        "Derived market features" => "features",
        "Strategy profit ledger" => "ledger",
        "Trading strategies" => "strategy",
        "Trading mandate" => "mandate",
        _ => "unknown",
    };
    let card_selector = format!("lnmarkets-card-{card_id}-{status}");
    v_flex()
        .id(card_selector.clone())
        .debug_selector(move || card_selector)
        .w_full()
        .max_w(px(640.))
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().colors().border_variant)
        .bg(cx.theme().colors().surface_background)
        .overflow_hidden()
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .gap_3()
                .px_4()
                .py_3()
                .border_b_1()
                .border_color(cx.theme().colors().border_variant)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().colors().text_muted)
                        .child(status.to_string()),
                ),
        )
        .child(
            v_flex()
                .w_full()
                .px_4()
                .py_3()
                .gap_2()
                .children(rows.into_iter().map(|(label, value)| {
                    h_flex()
                        .w_full()
                        .justify_between()
                        .gap_4()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().colors().text_muted)
                                .child(label),
                        )
                        .child(div().text_xs().child(value))
                })),
        )
        .into_any_element()
}

fn string_field(value: &Value, key: &str, fallback: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

#[derive(RegisterComponent)]
pub struct LnMarketsToolCardsPreview;

impl Component for LnMarketsToolCardsPreview {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        "LN Markets feature, ledger, strategy lifecycle, and mandate cards."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_6()
            .children([
                preview_group(
                    "Features",
                    ["collecting", "ready", "degraded"],
                    |status| {
                        json!({
                            "schema": "omega.lnmarkets.features.v1",
                            "status": status,
                            "collector": { "network": "signet", "stored_events": 248 },
                            "features": if status == "collecting" { Value::Null } else { json!({ "as_of_ms": 1786276800000_i64 }) },
                        })
                    },
                    cx,
                ),
                preview_group("Ledger", ["empty", "profit", "drawdown"], |status| {
                    let profit = if status == "drawdown" { -830 } else if status == "profit" { 1420 } else { 0 };
                    json!({
                        "schema": "omega.lnmarkets.ledger.v1",
                        "status": status,
                        "report": {
                            "total_profit_sats": profit,
                            "total_fees_paid_sats": 82,
                            "total_funding_collected_sats": 610,
                            "worst_drawdown_sats": if status == "drawdown" { -1100 } else { -120 },
                        },
                    })
                }, cx),
                preview_group(
                    "Strategy lifecycle",
                    ["idle", "starting", "running", "adjusting", "halted", "error"],
                    |status| {
                        json!({
                            "schema": "omega.lnmarkets.strategy.v1",
                            "status": status,
                            "strategies": [
                                { "strategy_id": "rebalance_to_target", "status": status },
                                { "strategy_id": "funding_carry", "status": "idle" },
                                { "strategy_id": "threshold_swing", "status": "idle" },
                            ],
                        })
                    },
                    cx,
                ),
                preview_group("Mandate", ["missing", "active", "expired"], |status| {
                    json!({
                        "schema": "omega.lnmarkets.mandate.v1",
                        "status": status,
                        "snapshot": {
                            "revision": if status == "missing" { 0 } else { 4 },
                            "mandate": if status == "missing" { Value::Null } else { json!({
                                "network": "signet",
                                "objective": "Bound automated carry and rebalance risk",
                            }) },
                        },
                    })
                }, cx),
            ])
            .into_any_element()
    }
}

fn preview_group<const COUNT: usize>(
    title: &str,
    statuses: [&str; COUNT],
    payload: impl Fn(&str) -> Value,
    cx: &App,
) -> AnyElement {
    example_group_with_title(
        title,
        statuses
            .into_iter()
            .filter_map(|status| {
                Some(single_example(status, render_payload(&payload(status), cx)?).width(px(640.)))
            })
            .collect(),
    )
    .vertical()
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render, TestAppContext, VisualTestContext, size};

    struct LnMarketsCardsTestView;

    impl Render for LnMarketsCardsTestView {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            LnMarketsToolCardsPreview::preview(window, cx)
        }
    }

    #[test]
    fn extracts_wrapped_strategy_updates() {
        let wrapped = json!({
            "result": {
                "content": [{
                    "text": serde_json::to_string(&json!({
                        "schema": "omega.lnmarkets.strategy.v1",
                        "status": "running",
                    })).expect("payload"),
                }],
            },
        });
        assert_eq!(
            extract_payload(&wrapped).and_then(|value| value.get("status").cloned()),
            Some(json!("running"))
        );
    }

    #[test]
    fn rejects_unrelated_json() {
        assert!(extract_payload(&json!({ "schema": "unrelated.v1" })).is_none());
    }

    #[gpui::test]
    fn component_gallery_paints_every_card_state(cx: &mut TestAppContext) {
        crate::test_support::init_test(cx);
        let window = cx.add_window(|_window, _cx| LnMarketsCardsTestView);
        cx.run_until_parked();

        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(size(px(1200.), px(2400.)));
        visual.run_until_parked();

        for selector in [
            "lnmarkets-card-features-collecting",
            "lnmarkets-card-features-ready",
            "lnmarkets-card-features-degraded",
            "lnmarkets-card-ledger-empty",
            "lnmarkets-card-ledger-profit",
            "lnmarkets-card-ledger-drawdown",
            "lnmarkets-card-strategy-idle",
            "lnmarkets-card-strategy-starting",
            "lnmarkets-card-strategy-running",
            "lnmarkets-card-strategy-adjusting",
            "lnmarkets-card-strategy-halted",
            "lnmarkets-card-strategy-error",
            "lnmarkets-card-mandate-missing",
            "lnmarkets-card-mandate-active",
            "lnmarkets-card-mandate-expired",
        ] {
            assert!(
                visual.debug_bounds(selector).is_some(),
                "component gallery did not paint {selector}"
            );
        }
    }
}
