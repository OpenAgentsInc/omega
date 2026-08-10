use agent_wakeup::{AgentWakeup, WakeupSource};
use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, PathBuilder, SharedString, Window, point, px};
use ui::prelude::*;
use ui::{MarketTokens, Plot, PlotMargins, market_number_font};

use crate::format::format_wall_clock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WakeupTimelineKind {
    Upcoming,
    Event,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WakeupTimelineItem {
    pub at_ms: i64,
    pub kind: WakeupTimelineKind,
    pub source: SharedString,
    pub token_budget: u64,
}

impl WakeupTimelineItem {
    pub fn from_wakeup(wakeup: &AgentWakeup) -> Self {
        Self {
            at_ms: i64::try_from(wakeup.emitted_at_ms).unwrap_or(i64::MAX),
            kind: WakeupTimelineKind::Event,
            source: wakeup.source.transcript_label().into(),
            token_budget: wakeup.token_budget,
        }
    }

    pub fn scheduled(at_ms: i64, cadence: impl Into<SharedString>, token_budget: u64) -> Self {
        Self {
            at_ms,
            kind: WakeupTimelineKind::Upcoming,
            source: cadence.into(),
            token_budget,
        }
    }
}

pub trait WakeupTimelineSource {
    fn wakeup_timeline(&self) -> Vec<WakeupTimelineItem>;
}

#[derive(IntoElement, RegisterComponent)]
pub struct WakeupTimeline {
    items: Vec<WakeupTimelineItem>,
    now_ms: i64,
    tokens: Option<MarketTokens>,
}

impl WakeupTimeline {
    pub fn from_source(source: &impl WakeupTimelineSource, now_ms: i64) -> Self {
        Self::new(source.wakeup_timeline(), now_ms)
    }

    pub fn new(mut items: Vec<WakeupTimelineItem>, now_ms: i64) -> Self {
        items.sort_by_key(|item| item.at_ms);
        Self {
            items,
            now_ms,
            tokens: None,
        }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }

    pub fn domain(&self) -> (f64, f64) {
        let first = self.items.first().map_or(self.now_ms, |item| item.at_ms);
        let last = self.items.last().map_or(self.now_ms, |item| item.at_ms);
        let padding = 15 * 60 * 1_000;
        (
            first.min(self.now_ms).saturating_sub(padding) as f64,
            last.max(self.now_ms).saturating_add(padding) as f64,
        )
    }
}

impl RenderOnce for WakeupTimeline {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let domain = self.domain();
        let plot_items = self.items.clone();
        let plot = Plot::new(domain, (0.0, 1.0))
            .plot_size(620., 130.)
            .margins(PlotMargins {
                top: 12.,
                right: 20.,
                bottom: 22.,
                left: 20.,
            })
            .tick_targets(5, 2)
            .crosshair(false)
            .tokens(tokens)
            .layer(move |frame, window, _cx| {
                for item in &plot_items {
                    let timestamp = item.at_ms as f64;
                    if timestamp < domain.0 || timestamp > domain.1 {
                        continue;
                    }
                    let x = frame.x_at(timestamp);
                    let y = frame.y_at(match item.kind {
                        WakeupTimelineKind::Upcoming => 0.72,
                        WakeupTimelineKind::Event => 0.28,
                    });
                    let color = match item.kind {
                        WakeupTimelineKind::Upcoming => frame.tokens.up,
                        WakeupTimelineKind::Event => frame.tokens.down,
                    };
                    let mut marker = PathBuilder::stroke(px(2.));
                    marker.move_to(point(x - px(4.), y));
                    marker.line_to(point(x + px(4.), y));
                    marker.move_to(point(x, y - px(4.)));
                    marker.line_to(point(x, y + px(4.)));
                    if let Ok(path) = marker.build() {
                        window.paint_path(path, color);
                    }
                }
            });

        v_flex()
            .debug_selector(|| "command_center.wakeup_timeline".into())
            .w(px(640.))
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(tokens.grid)
            .bg(tokens.surface)
            .child(Label::new("Wakeups").size(LabelSize::Small))
            .child(plot)
            .children(self.items.into_iter().rev().take(4).map(|item| {
                let (icon, color) = match item.kind {
                    WakeupTimelineKind::Upcoming => (IconName::Clock, Color::Info),
                    WakeupTimelineKind::Event => (IconName::BellRing, Color::Warning),
                };
                h_flex()
                    .gap_2()
                    .child(Icon::new(icon).size(IconSize::XSmall).color(color))
                    .child(Label::new(item.source).size(LabelSize::XSmall))
                    .child(
                        div()
                            .flex_1()
                            .text_right()
                            .font(market_number_font(cx))
                            .text_size(px(11.))
                            .text_color(tokens.muted)
                            .child(format!(
                                "{} · {} tokens",
                                format_wall_clock(item.at_ms),
                                item.token_budget
                            )),
                    )
            }))
    }
}

fn demo_items() -> Vec<WakeupTimelineItem> {
    let now = 1_786_276_800_000_i64;
    vec![
        WakeupTimelineItem {
            at_ms: now - 3_600_000,
            kind: WakeupTimelineKind::Event,
            source: WakeupSource::FundingSignFlip {
                previous_bps: -2,
                current_bps: 4,
            }
            .transcript_label()
            .into(),
            token_budget: 1_024,
        },
        WakeupTimelineItem {
            at_ms: now - 900_000,
            kind: WakeupTimelineKind::Event,
            source: "strategy halt".into(),
            token_budget: 1_024,
        },
        WakeupTimelineItem::scheduled(now + 900_000, "15m cadence", 1_024),
        WakeupTimelineItem::scheduled(now + 1_800_000, "15m cadence", 1_024),
    ]
}

impl Component for WakeupTimeline {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        "Plot-kernel timeline for upcoming cadence and bounded event-triggered wakeups."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let now = 1_786_276_800_000_i64;
        let grayscale = MarketTokens::from_theme(cx).grayscale();
        example_group_with_title(
            "Wakeup timeline",
            vec![
                single_example(
                    "Normal",
                    WakeupTimeline::new(demo_items(), now).into_any_element(),
                ),
                single_example(
                    "Grayscale audit",
                    WakeupTimeline::new(demo_items(), now)
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
    use super::*;

    #[test]
    fn timeline_sorts_items_and_contains_now() {
        let now = 100;
        let timeline = WakeupTimeline::new(
            vec![
                WakeupTimelineItem::scheduled(200, "later", 10),
                WakeupTimelineItem::scheduled(50, "earlier", 10),
            ],
            now,
        );
        assert_eq!(timeline.items.first().map(|item| item.at_ms), Some(50));
        let domain = timeline.domain();
        assert!(domain.0 <= now as f64 && domain.1 >= now as f64);
    }
}
