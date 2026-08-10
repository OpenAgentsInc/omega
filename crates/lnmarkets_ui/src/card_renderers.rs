use std::rc::Rc;

use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, FontWeight, Window, div};
use plugin_api::CardRendererRegistration;
use serde_json::{Value, json};
use ui::prelude::*;

pub fn card_renderer_registrations() -> Vec<CardRendererRegistration> {
    [
        (
            "omega.lnmarkets.features.v1",
            render_features as CardRenderer,
        ),
        ("omega.lnmarkets.ledger.v1", render_ledger as CardRenderer),
        (
            "omega.lnmarkets.strategy.v1",
            render_strategy as CardRenderer,
        ),
        (
            "omega.lnmarkets.backtest_tool.v1",
            render_backtest as CardRenderer,
        ),
        (
            "omega.lnmarkets.backtest_history.v1",
            render_backtest as CardRenderer,
        ),
        ("omega.lnmarkets.mandate.v1", render_mandate as CardRenderer),
        (
            "omega.lnmarkets.prediction.v1",
            render_prediction as CardRenderer,
        ),
    ]
    .into_iter()
    .map(|(schema, render)| CardRendererRegistration {
        plugin_id: "lnmarkets",
        schema,
        render: Rc::new(render),
    })
    .collect()
}

type CardRenderer = fn(&Value, &App) -> Option<AnyElement>;

fn render_features(payload: &Value, cx: &App) -> Option<AnyElement> {
    let status = string_field(payload, "status", "collecting");
    let collector = payload.get("collector");
    let features = payload.get("features");
    Some(render_card(
        "Derived market features",
        &status,
        vec![
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
        ],
        cx,
    ))
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

fn render_backtest(payload: &Value, cx: &App) -> Option<AnyElement> {
    let report = payload.get("report").or_else(|| {
        payload
            .get("reports")
            .and_then(Value::as_array)
            .and_then(|reports| reports.first())
    })?;
    let status = payload
        .get("status")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            if report
                .get("outcome")
                .and_then(|outcome| outcome.get("status"))
                == Some(&Value::String("passed".into()))
            {
                "passed".into()
            } else {
                "failed".into()
            }
        });
    Some(render_card(
        "Strategy backtest",
        &status,
        vec![
            (
                "Strategy".into(),
                string_field(report, "strategy_id", "unknown"),
            ),
            (
                "Trades".into(),
                report
                    .get("trade_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .to_string(),
            ),
            (
                "Expectancy".into(),
                format!(
                    "{} millisats",
                    report
                        .get("expectancy_millisats")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                ),
            ),
            (
                "Max drawdown".into(),
                format!(
                    "{} sats",
                    report
                        .get("maximum_drawdown_sats")
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                ),
            ),
        ],
        cx,
    ))
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

// The prediction card is the venue-neutral command-center component
// (omega#284); this renderer only adapts the stored event out of the tool
// payload.
fn render_prediction(payload: &Value, _cx: &App) -> Option<AnyElement> {
    let event: prediction_events::PredictionEvent =
        serde_json::from_value(payload.get("prediction")?.clone()).ok()?;
    let data = command_center_ui::PredictionCardData::from_event(&event, None);
    Some(
        command_center_ui::PredictionCard::new(data, command_center_ui::unix_now_ms())
            .into_any_element(),
    )
}

fn render_card(title: &str, status: &str, rows: Vec<(String, String)>, cx: &App) -> AnyElement {
    let card_id = match title {
        "Derived market features" => "features",
        "Strategy profit ledger" => "ledger",
        "Trading strategies" => "strategy",
        "Strategy backtest" => "backtest",
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
        "LN Markets feature, ledger, backtest, strategy lifecycle, and mandate cards."
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
                preview_group("Backtest", ["passed", "failed"], |status| {
                    json!({
                        "schema": "omega.lnmarkets.backtest_tool.v1",
                        "status": status,
                        "report": {
                            "strategy_id": "threshold_swing",
                            "trade_count": 18,
                            "expectancy_millisats": if status == "passed" { 2400 } else { -800 },
                            "maximum_drawdown_sats": if status == "passed" { 350 } else { 2200 },
                            "outcome": { "status": status },
                        },
                    })
                }, cx),
                preview_group("Prediction", ["recorded"], |_status| {
                    json!({
                        "schema": "omega.lnmarkets.prediction.v1",
                        "status": "recorded",
                        "prediction": {
                            "sequence": 12,
                            "prediction_id": "pred-demo",
                            "schema_version": 1,
                            "emitted_at_ms": 1786276800000_i64,
                            "actor": { "type": "agent", "agent_id": "trading-session" },
                            "mandate_scope": { "venue": "lnmarkets", "network": "signet" },
                            "instrument": "BTCUSD",
                            "forecast": {
                                "type": "directional",
                                "direction": "up",
                                "probability_micros": 720000,
                            },
                            "confidence_micros": 720000,
                            "horizon_ms": 3600000_u64,
                            "resolution_rule": {
                                "source": "stored_last_price",
                                "baseline_at_ms": 1786276800000_i64,
                                "resolve_at_ms": 1786280400000_i64,
                                "flat_tolerance_bps": 25,
                            },
                            "scoring_rule": "brier",
                            "observation_refs": [],
                            "private_payload_ref": Value::Null,
                            "subsequent_decision_id": "decision-demo",
                        },
                    })
                }, cx),
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
                let payload = payload(status);
                let schema = schema(&payload)?;
                let renderer = card_renderer_registrations()
                    .into_iter()
                    .find(|renderer| renderer.schema == schema)?;
                Some(single_example(status, (renderer.render)(&payload, cx)?).width(px(640.)))
            })
            .collect(),
    )
    .vertical()
    .into_any_element()
}

fn schema(value: &Value) -> Option<&str> {
    value.get("schema")?.as_str()
}
