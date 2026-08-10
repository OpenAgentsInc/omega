use acp_thread::{ToolCall, ToolCallContent, ToolCallStatus};
use gpui::{AnyElement, App};
use serde_json::Value;

pub(crate) fn is_plugin_card_tool_call(tool_call: &ToolCall, cx: &App) -> bool {
    tool_payload(tool_call, cx).is_some_and(|payload| renderer_for(&payload, cx).is_some())
}

pub(crate) fn plugin_tool_card(tool_call: &ToolCall, cx: &App) -> Option<AnyElement> {
    if !matches!(
        tool_call.status,
        ToolCallStatus::Pending
            | ToolCallStatus::InProgress
            | ToolCallStatus::Completed
            | ToolCallStatus::Failed
    ) {
        return None;
    }
    let payload = tool_payload(tool_call, cx)?;
    let renderer = renderer_for(&payload, cx)?;
    (renderer.render)(&payload, cx)
}

fn renderer_for(
    payload: &Value,
    cx: &App,
) -> Option<std::rc::Rc<plugin_api::CardRendererRegistration>> {
    let schema = schema(payload)?;
    plugin_api::registry(cx)?.card_renderer(schema)
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
    value.get("schema")?.as_str()
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, rc::Rc};

    use gpui::{
        Context, FontWeight, IntoElement, Render, TestAppContext, VisualTestContext, Window, div,
        size,
    };
    use plugin_api::{CardRendererRegistration, PluginRegistry};
    use serde_json::json;
    use ui::prelude::*;

    use super::*;

    const FIXTURE_SCHEMA: &str = "omega.test.card.v1";

    struct PluginCardTestView;

    impl Render for PluginCardTestView {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let payload = json!({ "schema": FIXTURE_SCHEMA, "value": "42" });
            renderer_for(&payload, cx)
                .and_then(|renderer| (renderer.render)(&payload, cx))
                .unwrap_or_else(|| div().into_any_element())
        }
    }

    fn fixture_renderer(value: &Value, cx: &App) -> Option<AnyElement> {
        let value = value.get("value")?.as_str()?;
        Some(
            div()
                .id("plugin-card-fixture")
                .debug_selector(|| "plugin-card-fixture".to_string())
                .px_4()
                .py_3()
                .border_1()
                .border_color(cx.theme().colors().border_variant)
                .font_weight(FontWeight::SEMIBOLD)
                .child(value.to_string())
                .into_any_element(),
        )
    }

    #[test]
    fn extracts_raw_and_wrapped_payloads() {
        let raw = json!({ "schema": FIXTURE_SCHEMA, "value": 1 });
        assert_eq!(extract_payload(&raw), Some(raw));

        let wrapped = json!({
            "result": {
                "content": [{
                    "text": serde_json::to_string(&json!({
                        "schema": FIXTURE_SCHEMA,
                        "value": 2,
                    })).expect("payload"),
                }],
            },
        });
        assert_eq!(
            extract_payload(&wrapped).and_then(|value| value.get("value").cloned()),
            Some(json!(2))
        );
    }

    #[test]
    fn unrelated_json_falls_back() {
        assert!(extract_payload(&json!({ "value": 1 })).is_none());
    }

    #[gpui::test]
    fn exact_schema_renderer_paints(cx: &mut TestAppContext) {
        crate::test_support::init_test(cx);
        let mut registry = PluginRegistry::new(PathBuf::from("/data"));
        registry
            .add_card_renderer(CardRendererRegistration {
                plugin_id: "test",
                schema: FIXTURE_SCHEMA,
                render: Rc::new(fixture_renderer),
            })
            .expect("register fixture renderer");
        cx.update(|cx| plugin_api::init_global(registry, cx));

        let window = cx.add_window(|_window, _cx| PluginCardTestView);
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(size(px(640.), px(240.)));
        visual.run_until_parked();
        assert!(visual.debug_bounds("plugin-card-fixture").is_some());
    }
}
