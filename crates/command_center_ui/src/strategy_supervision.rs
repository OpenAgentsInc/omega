use std::sync::Arc;

use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, FontWeight, SharedString, Window};
use strategy_engine::{StrategyHaltReason, StrategyStatus};
use ui::prelude::*;
use ui::{Banner, MarketDirection, MarketTokens, Severity, market_number_font};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StrategyPhase {
    Idle,
    Assessing,
    AwaitingFill,
    Managing,
    Halted,
}

impl StrategyPhase {
    fn icon(self) -> IconName {
        match self {
            Self::Idle => IconName::Dash,
            Self::Assessing => IconName::Eye,
            Self::AwaitingFill => IconName::ArrowRightLeft,
            Self::Managing => IconName::PlayFilled,
            Self::Halted => IconName::Stop,
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Idle => Color::Muted,
            Self::Assessing => Color::Info,
            Self::AwaitingFill => Color::Warning,
            Self::Managing => Color::Success,
            Self::Halted => Color::Error,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StrategyPosition {
    pub instrument: SharedString,
    pub direction: MarketDirection,
    pub units: f64,
    pub unrealized_pnl: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StrategyCardValue {
    pub strategy_id: SharedString,
    pub phase: StrategyPhase,
    pub position: Option<StrategyPosition>,
    pub last_action: SharedString,
    pub halt_reason: Option<SharedString>,
}

impl StrategyCardValue {
    pub fn with_status(mut self, status: &StrategyStatus) -> Self {
        match status {
            StrategyStatus::Idle => self.phase = StrategyPhase::Idle,
            StrategyStatus::Running { .. } => self.phase = StrategyPhase::Managing,
            StrategyStatus::Halted { reason, .. } => {
                self.phase = StrategyPhase::Halted;
                self.halt_reason = Some(halt_reason_label(reason));
            }
        }
        self
    }
}

pub trait StrategyCardSource {
    fn strategy_card(&self) -> StrategyCardValue;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StrategyCardAction {
    Halt { strategy_id: SharedString },
    Resume { strategy_id: SharedString },
}

fn halt_reason_label(reason: &StrategyHaltReason) -> SharedString {
    match reason {
        StrategyHaltReason::Manual { reason } => reason.clone().into(),
        StrategyHaltReason::ProgramError { .. } => "program error".into(),
        StrategyHaltReason::MandateError { .. } => "mandate unavailable".into(),
        StrategyHaltReason::RiskLimit { .. } => "mandate limit".into(),
        StrategyHaltReason::MissingVenueProtection { .. } => "protection missing".into(),
        StrategyHaltReason::UnknownCancelOutcome { .. } => "cancel unknown".into(),
        StrategyHaltReason::VenueError { .. } => "venue error".into(),
        StrategyHaltReason::VenueCapability { .. } => "capability refused".into(),
        StrategyHaltReason::LedgerError { .. } => "ledger error".into(),
    }
}

#[derive(IntoElement, RegisterComponent)]
pub struct StrategyCard {
    value: StrategyCardValue,
    tokens: Option<MarketTokens>,
    on_action: Option<Arc<dyn Fn(StrategyCardAction, &mut Window, &mut App) + 'static>>,
}

impl StrategyCard {
    pub fn from_source(source: &impl StrategyCardSource) -> Self {
        Self::new(source.strategy_card())
    }

    pub fn new(value: StrategyCardValue) -> Self {
        Self {
            value,
            tokens: None,
            on_action: None,
        }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }

    pub fn on_action(
        mut self,
        handler: impl Fn(StrategyCardAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Arc::new(handler));
        self
    }
}

impl RenderOnce for StrategyCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let strategy_id = self.value.strategy_id.clone();
        let action = if self.value.phase == StrategyPhase::Halted {
            StrategyCardAction::Resume { strategy_id }
        } else {
            StrategyCardAction::Halt { strategy_id }
        };
        let action_label = if self.value.phase == StrategyPhase::Halted {
            "Resume"
        } else {
            "Halt"
        };
        let action_handler = self.on_action;
        let position = self.value.position.map(|position| {
            let direction_color = tokens.direction_color(position.direction);
            h_flex()
                .gap_4()
                .child(
                    Label::new(position.instrument)
                        .size(LabelSize::Small)
                        .color(Color::Default),
                )
                .child(
                    div()
                        .font(market_number_font(cx))
                        .text_color(direction_color)
                        .child(format!(
                            "{} {:.3} · {:+.2}",
                            position.direction.glyph(),
                            position.units,
                            position.unrealized_pnl
                        )),
                )
        });

        v_flex()
            .debug_selector(|| "command_center.strategy_card".into())
            .w(px(520.))
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
                        Label::new(self.value.strategy_id)
                            .size(LabelSize::Small)
                            .weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        Icon::new(self.value.phase.icon())
                            .size(IconSize::Small)
                            .color(self.value.phase.color()),
                    ),
            )
            .when_some(position, |this, position| this.child(position))
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        Label::new("Last action")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(Label::new(self.value.last_action).size(LabelSize::XSmall)),
            )
            .when_some(self.value.halt_reason, |this, reason| {
                this.child(
                    h_flex()
                        .gap_2()
                        .child(
                            Icon::new(IconName::Warning)
                                .size(IconSize::XSmall)
                                .color(Color::Error),
                        )
                        .child(
                            Label::new(reason)
                                .size(LabelSize::XSmall)
                                .color(Color::Error),
                        ),
                )
            })
            .child(h_flex().justify_end().child(
                Button::new("strategy-card-action", action_label).when_some(
                    action_handler,
                    |button, handler| {
                        button.on_click(move |_, window, cx| {
                            handler(action.clone(), window, cx);
                        })
                    },
                ),
            ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HaltResumePath {
    ResumeStrategy,
    ReviewMandate,
    InspectVenue,
}

impl HaltResumePath {
    fn label(&self) -> &'static str {
        match self {
            Self::ResumeStrategy => "Resume",
            Self::ReviewMandate => "Review mandate",
            Self::InspectVenue => "Inspect venue",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HaltBannerValue {
    pub strategy_id: SharedString,
    pub reason: SharedString,
    pub resume_path: HaltResumePath,
}

pub trait HaltBannerSource {
    fn halt_banner(&self) -> HaltBannerValue;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HaltBannerAction {
    pub strategy_id: SharedString,
    pub path: HaltResumePath,
}

#[derive(IntoElement, RegisterComponent)]
pub struct StrategyHaltBanner {
    value: HaltBannerValue,
    grayscale: bool,
    on_action: Option<Arc<dyn Fn(HaltBannerAction, &mut Window, &mut App) + 'static>>,
}

impl StrategyHaltBanner {
    pub fn from_source(source: &impl HaltBannerSource) -> Self {
        Self::new(source.halt_banner())
    }

    pub fn new(value: HaltBannerValue) -> Self {
        Self {
            value,
            grayscale: false,
            on_action: None,
        }
    }

    pub fn tokens(mut self, _tokens: MarketTokens) -> Self {
        self.grayscale = true;
        self
    }

    pub fn on_action(
        mut self,
        handler: impl Fn(HaltBannerAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Arc::new(handler));
        self
    }
}

impl RenderOnce for StrategyHaltBanner {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let label = self.value.resume_path.label();
        let action = HaltBannerAction {
            strategy_id: self.value.strategy_id.clone(),
            path: self.value.resume_path,
        };
        let severity = if self.grayscale {
            Severity::Info
        } else {
            Severity::Error
        };
        div()
            .debug_selector(|| "command_center.halt_banner".into())
            .w(px(620.))
            .child(
                Banner::new()
                    .severity(severity)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Label::new(self.value.strategy_id).size(LabelSize::Small))
                            .child(Label::new(self.value.reason).size(LabelSize::XSmall)),
                    )
                    .action_slot(Button::new("halt-resume-path", label).when_some(
                        self.on_action,
                        |button, handler| {
                            button.on_click(move |_, window, cx| {
                                handler(action.clone(), window, cx);
                            })
                        },
                    )),
            )
    }
}

fn demo_strategy() -> StrategyCardValue {
    StrategyCardValue {
        strategy_id: "funding-carry".into(),
        phase: StrategyPhase::Managing,
        position: Some(StrategyPosition {
            instrument: "BTC-PERP".into(),
            direction: MarketDirection::Down,
            units: 0.08,
            unrealized_pnl: 177.60,
        }),
        last_action: "margin protected · 14:32".into(),
        halt_reason: None,
    }
}

impl Component for StrategyCard {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        "Venue-neutral strategy lifecycle, position, last action, and halt state."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let grayscale = MarketTokens::from_theme(cx).grayscale();
        example_group_with_title(
            "Strategy supervision",
            vec![
                single_example(
                    "Normal",
                    StrategyCard::new(demo_strategy()).into_any_element(),
                ),
                single_example(
                    "Grayscale audit",
                    StrategyCard::new(demo_strategy())
                        .tokens(grayscale)
                        .into_any_element(),
                ),
            ],
        )
        .vertical()
        .into_any_element()
    }
}

impl Component for StrategyHaltBanner {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        "Typed strategy halt reason with the smallest available resume path."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let value = HaltBannerValue {
            strategy_id: "funding-carry".into(),
            reason: "mandate limit".into(),
            resume_path: HaltResumePath::ReviewMandate,
        };
        example_group_with_title(
            "Halt banner",
            vec![
                single_example(
                    "Normal",
                    StrategyHaltBanner::new(value.clone()).into_any_element(),
                ),
                single_example(
                    "Grayscale audit",
                    StrategyHaltBanner::new(value)
                        .tokens(MarketTokens::from_theme(cx).grayscale())
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
    use super::*;

    #[test]
    fn halted_status_replaces_phase_and_reason() {
        let value = demo_strategy().with_status(&StrategyStatus::Halted {
            halted_at_ms: 10,
            reason: StrategyHaltReason::Manual {
                reason: "operator".into(),
            },
        });
        assert_eq!(value.phase, StrategyPhase::Halted);
        assert_eq!(value.halt_reason.as_deref(), Some("operator"));
    }
}
