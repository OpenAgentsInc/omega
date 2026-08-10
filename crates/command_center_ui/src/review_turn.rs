use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, FontWeight, SharedString, Window};
use plugin_api::ReviewTurnEvidence;
use ui::prelude::*;
use ui::{MarketTokens, market_number_font};

use crate::format::{format_duration_ms, format_wall_clock};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewTurnDecision {
    Action { summary: SharedString },
    NoChange,
    Failed { reason: SharedString },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewTurnValue {
    pub at_ms: i64,
    pub trigger: SharedString,
    pub read_sources: Vec<SharedString>,
    pub prediction: Option<SharedString>,
    pub decision: ReviewTurnDecision,
    pub model_id: SharedString,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub token_cost_microusd: Option<u64>,
    pub wall_clock_ms: u64,
}

impl ReviewTurnValue {
    pub fn from_evidence(
        evidence: &ReviewTurnEvidence,
        prediction: Option<SharedString>,
        decision: ReviewTurnDecision,
        token_cost_microusd: Option<u64>,
    ) -> Self {
        Self {
            at_ms: evidence.at_ms,
            trigger: evidence.source.transcript_label().into(),
            read_sources: evidence
                .tool_calls
                .iter()
                .map(|call| SharedString::from(call.name.clone()))
                .collect(),
            prediction,
            decision,
            model_id: evidence.model_id.clone().into(),
            input_tokens: evidence.token_usage.input_total(),
            output_tokens: evidence.token_usage.output_tokens,
            token_cost_microusd,
            wall_clock_ms: evidence.wall_clock_ms,
        }
    }
}

pub trait ReviewTurnSource {
    fn review_turn(&self) -> ReviewTurnValue;
}

#[derive(IntoElement, RegisterComponent)]
pub struct ReviewTurnCard {
    value: ReviewTurnValue,
    tokens: Option<MarketTokens>,
}

impl ReviewTurnCard {
    pub fn from_source(source: &impl ReviewTurnSource) -> Self {
        Self::new(source.review_turn())
    }

    pub fn new(value: ReviewTurnValue) -> Self {
        Self {
            value,
            tokens: None,
        }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

fn fact(label: &'static str, value: impl Into<SharedString>) -> AnyElement {
    h_flex()
        .justify_between()
        .gap_4()
        .child(
            Label::new(label)
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .child(Label::new(value.into()).size(LabelSize::XSmall))
        .into_any_element()
}

impl RenderOnce for ReviewTurnCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let (decision_icon, decision_color, decision) = match self.value.decision {
            ReviewTurnDecision::Action { summary } => {
                (IconName::PlayFilled, Color::Success, summary)
            }
            ReviewTurnDecision::NoChange => {
                (IconName::Dash, Color::Muted, SharedString::from("none"))
            }
            ReviewTurnDecision::Failed { reason } => (IconName::Warning, Color::Error, reason),
        };
        let reads = if self.value.read_sources.is_empty() {
            SharedString::from("none")
        } else {
            self.value
                .read_sources
                .iter()
                .map(SharedString::as_ref)
                .collect::<Vec<_>>()
                .join(" · ")
                .into()
        };
        let cost = self
            .value
            .token_cost_microusd
            .map(|microusd| format!("${:.4}", microusd as f64 / 1_000_000.0))
            .unwrap_or_else(|| "unpriced".into());

        v_flex()
            .debug_selector(|| "command_center.review_turn".into())
            .w(px(560.))
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(tokens.grid)
            .bg(tokens.surface)
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        Label::new("Review turn")
                            .size(LabelSize::Small)
                            .weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        div()
                            .font(market_number_font(cx))
                            .text_size(px(11.))
                            .text_color(tokens.muted)
                            .child(format_wall_clock(self.value.at_ms)),
                    ),
            )
            .child(fact("Trigger", self.value.trigger))
            .child(fact("Read", reads))
            .child(fact(
                "Prediction",
                self.value.prediction.unwrap_or_else(|| "none".into()),
            ))
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        Label::new("Decision")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Icon::new(decision_icon)
                                    .size(IconSize::XSmall)
                                    .color(decision_color),
                            )
                            .child(
                                Label::new(decision)
                                    .size(LabelSize::XSmall)
                                    .color(decision_color),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .child(fact("Model", self.value.model_id))
                    .child(
                        div()
                            .font(market_number_font(cx))
                            .text_size(px(11.))
                            .text_color(tokens.text)
                            .child(format!(
                                "{} in · {} out · {} · {}",
                                self.value.input_tokens,
                                self.value.output_tokens,
                                cost,
                                format_duration_ms(
                                    i64::try_from(self.value.wall_clock_ms).unwrap_or(i64::MAX),
                                )
                            )),
                    ),
            )
    }
}

fn demo_value() -> ReviewTurnValue {
    ReviewTurnValue {
        at_ms: 1_786_276_800_000,
        trigger: "funding sign flip".into(),
        read_sources: vec!["features".into(), "ledger".into(), "mandate".into()],
        prediction: Some("BTC-PERP down · 64% · 8h".into()),
        decision: ReviewTurnDecision::Action {
            summary: "reduce carry target".into(),
        },
        model_id: "claude-sonnet-4".into(),
        input_tokens: 704,
        output_tokens: 126,
        token_cost_microusd: Some(5_912),
        wall_clock_ms: 2_840,
    }
}

impl Component for ReviewTurnCard {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        "Scheduled review evidence: sources, prediction, decision, tokens, cost, and duration."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let grayscale = MarketTokens::from_theme(cx).grayscale();
        example_group_with_title(
            "Review turn",
            vec![
                single_example(
                    "Normal",
                    ReviewTurnCard::new(demo_value()).into_any_element(),
                ),
                single_example(
                    "Grayscale audit",
                    ReviewTurnCard::new(demo_value())
                        .tokens(grayscale)
                        .into_any_element(),
                ),
            ],
        )
        .vertical()
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use agent_wakeup::WakeupSource;
    use plugin_api::{ReviewTokenUsage, ReviewToolCall};
    use serde_json::Value;

    use super::*;

    #[test]
    fn evidence_adapter_preserves_measured_tokens_and_reads() {
        let evidence = ReviewTurnEvidence {
            at_ms: 10,
            completed_at_ms: 20,
            wall_clock_ms: 10,
            model_id: "model".into(),
            token_usage: ReviewTokenUsage {
                input_tokens: 5,
                output_tokens: 3,
                cache_creation_input_tokens: 2,
                cache_read_input_tokens: 1,
            },
            tool_calls: vec![ReviewToolCall {
                name: "ledger".into(),
                input: Value::Null,
            }],
            source: WakeupSource::ScheduledReview {
                cadence: "15m".into(),
            },
            reasoning_note_present: true,
            tracked_tool_calls: 1,
            tokens_used: 11,
        };
        let value =
            ReviewTurnValue::from_evidence(&evidence, None, ReviewTurnDecision::NoChange, None);
        assert_eq!(value.input_tokens, 8);
        assert_eq!(value.output_tokens, 3);
        assert_eq!(value.read_sources, vec![SharedString::from("ledger")]);
    }
}
