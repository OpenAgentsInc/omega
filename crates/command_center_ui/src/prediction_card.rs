use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, FontWeight, SharedString, Window, relative};
use prediction_events::{
    PredictedDirection, PredictionActor, PredictionEvent, PredictionForecast, PredictionScore,
    ResolvedOutcome,
};
use ui::prelude::*;

use crate::format::{
    direction_color, format_countdown, format_percent_bps, format_probability_micros,
    format_wall_clock,
};

/// A displayable forecast: the directional call or the outcome distribution.
#[derive(Clone, Debug)]
pub enum ForecastDisplay {
    Directional {
        direction: PredictedDirection,
        probability_micros: u32,
    },
    Distribution {
        outcomes: Vec<(SharedString, u32)>,
    },
}

impl ForecastDisplay {
    fn headline(&self) -> (SharedString, Color, IconName) {
        match self {
            Self::Directional { direction, .. } => {
                let (label, icon) = match direction {
                    PredictedDirection::Up => ("up", IconName::OmegaPredictUp),
                    PredictedDirection::Down => ("down", IconName::OmegaPredictDown),
                    PredictedDirection::Flat => ("no change", IconName::Dash),
                };
                (label.into(), direction_color(*direction), icon)
            }
            Self::Distribution { outcomes } => {
                let best = outcomes
                    .iter()
                    .max_by_key(|(_, probability)| *probability)
                    .map(|(outcome, _)| outcome.clone())
                    .unwrap_or_else(|| "distribution".into());
                (best, Color::Accent, IconName::OmegaPredict)
            }
        }
    }
}

/// How the prediction stands: still open, or resolved with an outcome and a
/// score.
#[derive(Clone, Debug)]
pub enum PredictionResolution {
    Pending,
    Resolved {
        matched: bool,
        outcome: SharedString,
        score_micros: u64,
        resolved_at_ms: i64,
    },
}

/// Value-typed prediction card input; build from a stored
/// [`PredictionEvent`] (plus its [`PredictionScore`] once resolved) via
/// [`PredictionCardData::from_event`].
#[derive(Clone, Debug)]
pub struct PredictionCardData {
    pub prediction_id: SharedString,
    pub actor: SharedString,
    pub instrument: SharedString,
    pub forecast: ForecastDisplay,
    pub confidence_micros: u32,
    pub emitted_at_ms: i64,
    pub resolve_at_ms: i64,
    pub resolution_source: SharedString,
    pub flat_tolerance_bps: u32,
    pub resolution: PredictionResolution,
}

fn outcome_label(outcome: &ResolvedOutcome) -> SharedString {
    match outcome {
        ResolvedOutcome::Direction { direction } => match direction {
            PredictedDirection::Up => "up".into(),
            PredictedDirection::Down => "down".into(),
            PredictedDirection::Flat => "no change".into(),
        },
        ResolvedOutcome::Named { outcome } => outcome.clone().into(),
    }
}

impl PredictionCardData {
    pub fn from_event(event: &PredictionEvent, score: Option<&PredictionScore>) -> Self {
        let draft = &event.draft;
        let actor = match &draft.actor {
            PredictionActor::Agent { agent_id } => format!("agent {agent_id}"),
            PredictionActor::Strategy { strategy_id } => format!("strategy {strategy_id}"),
        };
        let forecast = match &draft.forecast {
            PredictionForecast::Directional {
                direction,
                probability_micros,
            } => ForecastDisplay::Directional {
                direction: *direction,
                probability_micros: *probability_micros,
            },
            PredictionForecast::Distribution { outcomes } => ForecastDisplay::Distribution {
                outcomes: outcomes
                    .iter()
                    .map(|outcome| {
                        (
                            SharedString::from(outcome.outcome.clone()),
                            outcome.probability_micros,
                        )
                    })
                    .collect(),
            },
        };
        let resolution = match score {
            Some(score) => PredictionResolution::Resolved {
                matched: score.realized_match,
                outcome: outcome_label(&score.outcome),
                score_micros: score.score_micros,
                resolved_at_ms: score.resolved_at_ms,
            },
            None => PredictionResolution::Pending,
        };
        Self {
            prediction_id: event.prediction_id.clone().into(),
            actor: actor.into(),
            instrument: draft.instrument.clone().into(),
            forecast,
            confidence_micros: draft.confidence_micros,
            emitted_at_ms: draft.emitted_at_ms,
            resolve_at_ms: draft.resolution_rule.resolve_at_ms,
            resolution_source: draft.resolution_rule.source.clone().into(),
            flat_tolerance_bps: draft.resolution_rule.flat_tolerance_bps,
            resolution,
        }
    }

    fn horizon_line(&self, now_ms: i64) -> (String, Color) {
        match &self.resolution {
            PredictionResolution::Pending => {
                if self.resolve_at_ms > now_ms {
                    (
                        format!("resolves {}", format_countdown(self.resolve_at_ms, now_ms)),
                        Color::Muted,
                    )
                } else {
                    ("matured · awaiting resolution".to_string(), Color::Warning)
                }
            }
            PredictionResolution::Resolved { resolved_at_ms, .. } => (
                format!("resolved {}", format_countdown(*resolved_at_ms, now_ms)),
                Color::Muted,
            ),
        }
    }
}

fn confidence_bar(confidence_micros: u32, cx: &App) -> AnyElement {
    let fill = (confidence_micros as f32 / 1_000_000.0).clamp(0.0, 1.0);
    h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .child(
            div()
                .relative()
                .flex_1()
                .h_1p5()
                .rounded_full()
                .bg(cx.theme().colors().element_background)
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .h_full()
                        .rounded_full()
                        .w(relative(fill))
                        .bg(Color::Accent.color(cx)),
                ),
        )
        .child(
            Label::new(format_probability_micros(confidence_micros))
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .into_any_element()
}

fn resolution_chip(resolution: &PredictionResolution) -> Option<AnyElement> {
    match resolution {
        PredictionResolution::Pending => None,
        PredictionResolution::Resolved {
            matched,
            outcome,
            score_micros,
            ..
        } => {
            let (verdict, color, icon) = if *matched {
                ("correct", Color::Success, IconName::Check)
            } else {
                ("missed", Color::Error, IconName::XCircle)
            };
            Some(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(Icon::new(icon).size(IconSize::XSmall).color(color))
                    .child(
                        Label::new(format!("{verdict} · outcome {outcome}"))
                            .size(LabelSize::XSmall)
                            .color(color),
                    )
                    .child(
                        Label::new(format!(
                            "score {}.{:06}",
                            score_micros / 1_000_000,
                            score_micros % 1_000_000
                        ))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    )
                    .into_any_element(),
            )
        }
    }
}

/// The transcript/panel prediction card: instrument, direction or
/// distribution, confidence bar, horizon countdown, resolution rule, and —
/// once resolved — outcome and score.
#[derive(IntoElement, RegisterComponent)]
pub struct PredictionCard {
    data: PredictionCardData,
    now_ms: i64,
}

impl PredictionCard {
    pub fn new(data: PredictionCardData, now_ms: i64) -> Self {
        Self { data, now_ms }
    }
}

impl RenderOnce for PredictionCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let data = self.data;
        let (headline, headline_color, headline_icon) = data.forecast.headline();
        let (horizon, horizon_color) = data.horizon_line(self.now_ms);
        let distribution_rows = match &data.forecast {
            ForecastDisplay::Distribution { outcomes } => outcomes.clone(),
            ForecastDisplay::Directional { .. } => Vec::new(),
        };
        let chip = resolution_chip(&data.resolution);

        v_flex()
            .id("command-center-prediction-card")
            .debug_selector(|| "command_center.prediction_card".into())
            .w_full()
            .max_w(px(520.))
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().surface_background)
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .child(
                                Icon::new(headline_icon)
                                    .size(IconSize::Small)
                                    .color(headline_color),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(Label::new(data.instrument.clone())),
                            )
                            .child(
                                Label::new(headline)
                                    .size(LabelSize::Small)
                                    .color(headline_color),
                            ),
                    )
                    .child(Label::new(horizon).size(LabelSize::XSmall).color(horizon_color)),
            )
            .child(
                v_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_1p5()
                    .child(confidence_bar(data.confidence_micros, cx))
                    .children(distribution_rows.into_iter().map(|(outcome, probability)| {
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_2()
                            .child(Label::new(outcome).size(LabelSize::XSmall))
                            .child(
                                Label::new(format_probability_micros(probability))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                    }))
                    .children(chip)
                    .child(
                        Label::new(format!(
                            "{} · resolves via {} · flat tolerance {} · emitted {}",
                            data.actor,
                            data.resolution_source,
                            format_percent_bps(data.flat_tolerance_bps),
                            format_wall_clock(data.emitted_at_ms),
                        ))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    ),
            )
    }
}

/// The panel list variant: compact one-line rows, newest first.
#[derive(IntoElement)]
pub struct PredictionList {
    rows: Vec<PredictionCardData>,
    now_ms: i64,
}

impl PredictionList {
    pub fn new(mut rows: Vec<PredictionCardData>, now_ms: i64) -> Self {
        rows.sort_by_key(|row| std::cmp::Reverse(row.emitted_at_ms));
        Self { rows, now_ms }
    }
}

impl RenderOnce for PredictionList {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now_ms = self.now_ms;
        v_flex()
            .id("command-center-prediction-list")
            .debug_selector(|| "command_center.prediction_list".into())
            .w_full()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().surface_background)
            .overflow_hidden()
            .when(self.rows.is_empty(), |this| {
                this.child(
                    div().px_3().py_2().child(
                        Label::new("No predictions")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
                )
            })
            .children(self.rows.into_iter().enumerate().map(|(index, row)| {
                let (headline, headline_color, headline_icon) = row.forecast.headline();
                let (horizon, horizon_color) = row.horizon_line(now_ms);
                let verdict = match &row.resolution {
                    PredictionResolution::Pending => None,
                    PredictionResolution::Resolved { matched, .. } => Some(if *matched {
                        Label::new("correct")
                            .size(LabelSize::XSmall)
                            .color(Color::Success)
                    } else {
                        Label::new("missed")
                            .size(LabelSize::XSmall)
                            .color(Color::Error)
                    }),
                };
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .when(index > 0, |this| {
                        this.border_t_1()
                            .border_color(cx.theme().colors().border_variant)
                    })
                    .child(
                        Label::new(format_wall_clock(row.emitted_at_ms))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Icon::new(headline_icon)
                            .size(IconSize::XSmall)
                            .color(headline_color),
                    )
                    .child(Label::new(row.instrument.clone()).size(LabelSize::Small))
                    .child(Label::new(headline).size(LabelSize::XSmall).color(headline_color))
                    .child(
                        Label::new(format_probability_micros(row.confidence_micros))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .children(verdict)
                    .child(
                        div().flex_1().text_right().child(
                            Label::new(horizon)
                                .size(LabelSize::XSmall)
                                .color(horizon_color),
                        ),
                    )
            }))
    }
}

const DEMO_NOW_MS: i64 = 1_786_276_800_000;

fn demo_data(
    id: &str,
    direction: PredictedDirection,
    confidence_micros: u32,
    emitted_offset_ms: i64,
    resolve_offset_ms: i64,
    resolution: PredictionResolution,
) -> PredictionCardData {
    PredictionCardData {
        prediction_id: id.to_string().into(),
        actor: "agent trading-session".into(),
        instrument: "BTCUSD".into(),
        forecast: ForecastDisplay::Directional {
            direction,
            probability_micros: confidence_micros,
        },
        confidence_micros,
        emitted_at_ms: DEMO_NOW_MS + emitted_offset_ms,
        resolve_at_ms: DEMO_NOW_MS + resolve_offset_ms,
        resolution_source: "stored_last_price".into(),
        flat_tolerance_bps: 25,
        resolution,
    }
}

/// The four demo fixtures: pending, resolved-correct, resolved-wrong, and an
/// explicitly-scored "no change" prediction.
pub(crate) fn demo_predictions() -> Vec<PredictionCardData> {
    vec![
        demo_data(
            "pred-104",
            PredictedDirection::Up,
            720_000,
            -10 * 60_000,
            50 * 60_000,
            PredictionResolution::Pending,
        ),
        demo_data(
            "pred-103",
            PredictedDirection::Down,
            640_000,
            -3 * 3_600_000,
            -2 * 3_600_000,
            PredictionResolution::Resolved {
                matched: true,
                outcome: "down".into(),
                score_micros: 129_600,
                resolved_at_ms: DEMO_NOW_MS - 2 * 3_600_000,
            },
        ),
        demo_data(
            "pred-102",
            PredictedDirection::Up,
            810_000,
            -5 * 3_600_000,
            -4 * 3_600_000,
            PredictionResolution::Resolved {
                matched: false,
                outcome: "down".into(),
                score_micros: 656_100,
                resolved_at_ms: DEMO_NOW_MS - 4 * 3_600_000,
            },
        ),
        demo_data(
            "pred-101",
            PredictedDirection::Flat,
            550_000,
            -7 * 3_600_000,
            -6 * 3_600_000,
            PredictionResolution::Resolved {
                matched: true,
                outcome: "no change".into(),
                score_micros: 202_500,
                resolved_at_ms: DEMO_NOW_MS - 6 * 3_600_000,
            },
        ),
    ]
}

impl Component for PredictionCard {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        "Prediction event card: instrument, direction, confidence bar, horizon countdown, resolution rule, and the resolved outcome + score."
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        let predictions = demo_predictions();
        let mut examples: Vec<_> = predictions
            .iter()
            .zip(["Pending", "Resolved correct", "Resolved wrong", "No change (scored)"])
            .map(|(data, name)| {
                single_example(
                    name,
                    PredictionCard::new(data.clone(), DEMO_NOW_MS).into_any_element(),
                )
                .width(px(520.))
            })
            .collect();
        examples.push(
            single_example(
                "Panel list",
                PredictionList::new(predictions, DEMO_NOW_MS).into_any_element(),
            )
            .width(px(640.)),
        );
        v_flex()
            .gap_6()
            .child(example_group_with_title("Prediction cards", examples).vertical())
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prediction_events::{
        MandateScope, PREDICTION_SCHEMA_VERSION, PredictionEventDraft, ResolutionRule, ScoringRule,
    };
    use trading_mandate::TradingNetwork;

    fn stored_event() -> PredictionEvent {
        PredictionEvent {
            sequence: 7,
            prediction_id: "pred-7".into(),
            draft: PredictionEventDraft {
                schema_version: PREDICTION_SCHEMA_VERSION,
                emitted_at_ms: 1_000,
                actor: PredictionActor::Agent {
                    agent_id: "session-1".into(),
                },
                mandate_scope: MandateScope {
                    venue: "lnmarkets".into(),
                    network: TradingNetwork::Signet,
                },
                instrument: "BTCUSD".into(),
                forecast: PredictionForecast::Directional {
                    direction: PredictedDirection::Up,
                    probability_micros: 720_000,
                },
                confidence_micros: 720_000,
                horizon_ms: 3_600_000,
                resolution_rule: ResolutionRule {
                    source: "stored_last_price".into(),
                    baseline_at_ms: 1_000,
                    resolve_at_ms: 3_601_000,
                    flat_tolerance_bps: 25,
                },
                scoring_rule: ScoringRule::Brier,
                observation_refs: Vec::new(),
                private_payload_ref: None,
                subsequent_decision_id: "decision-1".into(),
            },
        }
    }

    #[test]
    fn card_data_from_a_pending_event() {
        let data = PredictionCardData::from_event(&stored_event(), None);
        assert_eq!(data.prediction_id.as_ref(), "pred-7");
        assert_eq!(data.actor.as_ref(), "agent session-1");
        assert_eq!(data.instrument.as_ref(), "BTCUSD");
        assert_eq!(data.confidence_micros, 720_000);
        assert_eq!(data.resolve_at_ms, 3_601_000);
        assert!(matches!(data.resolution, PredictionResolution::Pending));
        let (headline, ..) = data.forecast.headline();
        assert_eq!(headline.as_ref(), "up");
    }

    #[test]
    fn card_data_joins_the_resolved_score() {
        let score = PredictionScore {
            sequence: 7,
            prediction_id: "pred-7".into(),
            resolved_at_ms: 3_700_000,
            resolution_source: "stored_last_price".into(),
            outcome: ResolvedOutcome::Direction {
                direction: PredictedDirection::Down,
            },
            forecast_probability_micros: 720_000,
            realized_match: false,
            score_micros: 518_400,
        };
        let data = PredictionCardData::from_event(&stored_event(), Some(&score));
        match data.resolution {
            PredictionResolution::Resolved {
                matched,
                outcome,
                score_micros,
                resolved_at_ms,
            } => {
                assert!(!matched);
                assert_eq!(outcome.as_ref(), "down");
                assert_eq!(score_micros, 518_400);
                assert_eq!(resolved_at_ms, 3_700_000);
            }
            PredictionResolution::Pending => panic!("score should resolve the card"),
        }
    }

    #[test]
    fn flat_forecast_reads_as_no_change() {
        let mut event = stored_event();
        event.draft.forecast = PredictionForecast::Directional {
            direction: PredictedDirection::Flat,
            probability_micros: 550_000,
        };
        event.draft.confidence_micros = 550_000;
        let data = PredictionCardData::from_event(&event, None);
        let (headline, color, _) = data.forecast.headline();
        assert_eq!(headline.as_ref(), "no change");
        assert_eq!(color, Color::Muted);
    }

    #[test]
    fn matured_pending_prediction_reports_awaiting_resolution() {
        let data = PredictionCardData::from_event(&stored_event(), None);
        let (line, _) = data.horizon_line(4_000_000);
        assert_eq!(line, "matured · awaiting resolution");
        let (line, _) = data.horizon_line(2_000_000);
        assert!(line.starts_with("resolves in"));
    }
}
