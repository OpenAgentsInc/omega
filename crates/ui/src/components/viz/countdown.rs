//! Compact relative-time primitive for expiries, funding, and safety timers.

use documented::Documented;
use gpui::px;

use crate::components::viz::{MarketTokens, format_countdown, market_number_font};
use crate::prelude::*;

#[derive(IntoElement, RegisterComponent, Documented)]
/// Compact deadline readout with tabular numerals and explicit relative text.
pub struct Countdown {
    deadline_ms: i64,
    now_ms: i64,
    label: Option<SharedString>,
    urgent_within_ms: i64,
    tokens: Option<MarketTokens>,
}

impl Countdown {
    pub fn new(deadline_ms: i64, now_ms: i64) -> Self {
        Self {
            deadline_ms,
            now_ms,
            label: None,
            urgent_within_ms: 60_000,
            tokens: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn urgent_within_ms(mut self, urgent_within_ms: i64) -> Self {
        self.urgent_within_ms = urgent_within_ms.max(0);
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }

    pub fn remaining_ms(&self) -> i64 {
        self.deadline_ms.saturating_sub(self.now_ms)
    }
}

impl RenderOnce for Countdown {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let remaining = self.remaining_ms();
        let color = if remaining <= 0 {
            tokens.down
        } else if remaining <= self.urgent_within_ms {
            tokens.down
        } else {
            tokens.text
        };
        h_flex()
            .debug_selector(|| "market.countdown".into())
            .gap_1()
            .when_some(self.label, |this, label| {
                this.child(
                    Label::new(label)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
            })
            .child(
                div()
                    .font(market_number_font(cx))
                    .text_size(px(11.))
                    .text_color(color)
                    .child(format_countdown(self.deadline_ms, self.now_ms)),
            )
    }
}

impl Component for Countdown {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let now = 1_754_700_000_000i64;
        let row = |tokens: Option<MarketTokens>| {
            let deadlines = [
                ("Funding", now + 2 * 3_600_000 + 14 * 60_000),
                ("Quote", now + 42_000),
                ("Expired", now - 5_000),
            ];
            h_flex()
                .gap_4()
                .children(deadlines.into_iter().map(|(label, deadline)| {
                    let mut countdown = Countdown::new(deadline, now).label(label);
                    if let Some(tokens) = tokens {
                        countdown = countdown.tokens(tokens);
                    }
                    countdown
                }))
                .into_any_element()
        };
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Relative time",
                vec![single_example(
                    "Future, urgent, and elapsed deadlines",
                    row(None),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Relative wording carries state without hue",
                    row(Some(MarketTokens::from_theme(cx).grayscale())),
                )],
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_time_saturates_without_overflow() {
        assert_eq!(Countdown::new(i64::MAX, i64::MIN).remaining_ms(), i64::MAX);
        assert_eq!(Countdown::new(5_000, 10_000).remaining_ms(), -5_000);
    }
}
