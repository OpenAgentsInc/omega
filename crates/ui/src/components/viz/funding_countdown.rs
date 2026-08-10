use documented::Documented;

use crate::components::viz::{Countdown, MarketTokens};
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FundingCadence {
    Hourly,
    ThreeTimesDaily,
    EveryHours(u8),
}

impl FundingCadence {
    pub fn interval_ms(self) -> i64 {
        match self {
            Self::Hourly => 3_600_000,
            Self::ThreeTimesDaily => 8 * 3_600_000,
            Self::EveryHours(hours) => i64::from(hours.max(1)) * 3_600_000,
        }
    }
    pub fn next_settlement_ms(self, now_ms: i64, anchor_ms: i64) -> i64 {
        let interval = self.interval_ms();
        let elapsed = now_ms.saturating_sub(anchor_ms);
        anchor_ms.saturating_add((elapsed.div_euclid(interval) + 1).saturating_mul(interval))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FundingSchedule {
    pub venue: SharedString,
    pub cadence: FundingCadence,
    pub anchor_ms: i64,
    pub now_ms: i64,
}
pub trait FundingCountdownSource {
    fn funding_schedule(&self) -> FundingSchedule;
}
pub struct DemoFundingCountdownSource;
impl FundingCountdownSource for DemoFundingCountdownSource {
    fn funding_schedule(&self) -> FundingSchedule {
        FundingSchedule {
            venue: "LN Markets".into(),
            cadence: FundingCadence::ThreeTimesDaily,
            anchor_ms: 0,
            now_ms: 1_754_700_000_000,
        }
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Venue-cadence-aware countdown to the next funding settlement.
pub struct FundingCountdown {
    schedule: FundingSchedule,
    tokens: Option<MarketTokens>,
}
impl FundingCountdown {
    pub fn from_source(source: &impl FundingCountdownSource) -> Self {
        Self {
            schedule: source.funding_schedule(),
            tokens: None,
        }
    }
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}
impl RenderOnce for FundingCountdown {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let deadline = self
            .schedule
            .cadence
            .next_settlement_ms(self.schedule.now_ms, self.schedule.anchor_ms);
        let mut countdown =
            Countdown::new(deadline, self.schedule.now_ms).label(self.schedule.venue);
        if let Some(tokens) = self.tokens {
            countdown = countdown.tokens(tokens);
        }
        div()
            .debug_selector(|| "market.funding_countdown".into())
            .child(countdown)
    }
}
impl Component for FundingCountdown {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }
    fn description() -> &'static str {
        Self::DOCS
    }
    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Funding countdown",
                vec![single_example(
                    "Eight-hour venue cadence",
                    FundingCountdown::from_source(&DemoFundingCountdownSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Venue and relative time carry the schedule",
                    FundingCountdown::from_source(&DemoFundingCountdownSource)
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settlement_respects_venue_cadence() {
        assert_eq!(
            FundingCadence::ThreeTimesDaily.next_settlement_ms(9 * 3_600_000, 0),
            16 * 3_600_000
        );
    }
}
