use documented::Documented;
use gpui::px;

use crate::components::viz::{
    FundingCadence, FundingCountdown, FundingCountdownSource, FundingSchedule, MarketTokens,
    format_compact, format_signed_percent, format_with_decimals, market_number_font,
};
use crate::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct MarketStats {
    pub venue: SharedString,
    pub volume_24h: f64,
    pub open_interest: f64,
    pub funding_fraction: f64,
    pub spread_bps: f64,
    pub funding_anchor_ms: i64,
    pub now_ms: i64,
    pub funding_cadence: FundingCadence,
}
pub trait MarketStatsSource {
    fn market_stats(&self) -> MarketStats;
}
pub struct DemoMarketStatsSource;
impl MarketStatsSource for DemoMarketStatsSource {
    fn market_stats(&self) -> MarketStats {
        MarketStats {
            venue: "Hyperliquid".into(),
            volume_24h: 4_830_000_000.0,
            open_interest: 1_240_000_000.0,
            funding_fraction: 0.000_117,
            spread_bps: 0.8,
            funding_anchor_ms: 0,
            now_ms: 1_754_700_000_000,
            funding_cadence: FundingCadence::Hourly,
        }
    }
}
impl FundingCountdownSource for MarketStats {
    fn funding_schedule(&self) -> FundingSchedule {
        FundingSchedule {
            venue: self.venue.clone(),
            cadence: self.funding_cadence,
            anchor_ms: self.funding_anchor_ms,
            now_ms: self.now_ms,
        }
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Compact 24-hour market statistics for cards and panels.
pub struct MarketStatsStrip {
    stats: MarketStats,
    tokens: Option<MarketTokens>,
}
impl MarketStatsStrip {
    pub fn from_source(source: &impl MarketStatsSource) -> Self {
        Self {
            stats: source.market_stats(),
            tokens: None,
        }
    }
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}
impl RenderOnce for MarketStatsStrip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let metric = |label, value: String| {
            v_flex()
                .gap_0p5()
                .child(
                    Label::new(label)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .font(market_number_font(cx))
                        .text_size(px(11.))
                        .text_color(tokens.text)
                        .child(value),
                )
        };
        let (funding, direction) = format_signed_percent(self.stats.funding_fraction, 4);
        h_flex()
            .debug_selector(|| "market.stats_strip".into())
            .gap_5()
            .p_2()
            .child(metric("24h volume", format_compact(self.stats.volume_24h)))
            .child(metric(
                "open interest",
                format_compact(self.stats.open_interest),
            ))
            .child(metric(
                "funding",
                format!("{} {funding}", direction.glyph()),
            ))
            .child(metric(
                "spread",
                format!("{} bps", format_with_decimals(self.stats.spread_bps, 1)),
            ))
            .child(FundingCountdown::from_source(&self.stats).tokens(tokens))
    }
}
impl Component for MarketStatsStrip {
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
                "Market stats",
                vec![single_example(
                    "Volume, OI, funding, spread",
                    MarketStatsStrip::from_source(&DemoMarketStatsSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Labels, signs, and units preserve meaning",
                    MarketStatsStrip::from_source(&DemoMarketStatsSource)
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
    fn demo_stats_are_finite() {
        let stats = DemoMarketStatsSource.market_stats();
        assert!(
            [
                stats.volume_24h,
                stats.open_interest,
                stats.funding_fraction,
                stats.spread_bps
            ]
            .iter()
            .all(|value| value.is_finite())
        );
    }
}
