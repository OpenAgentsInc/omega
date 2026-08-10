//! Dense strategy statistics and the shared-gauge order-book imbalance view.

use documented::Documented;

use crate::components::viz::{
    Gauge, HeadroomMeter, MarketDirection, MarketTokens, format_usd_cents, format_with_decimals,
    market_number_font,
};
use crate::prelude::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatisticKind {
    Sharpe,
    Sortino,
    Calmar,
    MaxDrawdown,
    ValueAtRisk,
    ExpectedShortfall,
    Expectancy,
    ProfitFactor,
    WinRate,
    TailRatio,
    OmegaRatio,
    Alpha,
    Beta,
}

impl StatisticKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sharpe => "Sharpe",
            Self::Sortino => "Sortino",
            Self::Calmar => "Calmar",
            Self::MaxDrawdown => "Max drawdown",
            Self::ValueAtRisk => "VaR",
            Self::ExpectedShortfall => "Expected shortfall",
            Self::Expectancy => "Expectancy",
            Self::ProfitFactor => "Profit factor",
            Self::WinRate => "Win rate",
            Self::TailRatio => "Tail ratio",
            Self::OmegaRatio => "Omega ratio",
            Self::Alpha => "Alpha",
            Self::Beta => "Beta",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatisticUnit {
    Ratio,
    Percent,
    CurrencyCents,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatisticValue {
    pub kind: StatisticKind,
    pub value: f64,
    pub unit: StatisticUnit,
    pub favorable_when_positive: bool,
}

impl StatisticValue {
    pub fn display(self) -> String {
        match self.unit {
            StatisticUnit::Ratio => format_with_decimals(self.value, 2),
            StatisticUnit::Percent => format!("{}%", format_with_decimals(self.value * 100.0, 1)),
            StatisticUnit::CurrencyCents => format_usd_cents(self.value.round() as i64),
        }
    }

    pub fn direction(self) -> MarketDirection {
        let value = if self.favorable_when_positive {
            self.value
        } else {
            -self.value
        };
        MarketDirection::of_f64(value)
    }
}

pub trait StatisticGridSource {
    fn statistics(&self) -> Vec<StatisticValue>;
}

pub struct DemoStatisticGridSource;

impl StatisticGridSource for DemoStatisticGridSource {
    fn statistics(&self) -> Vec<StatisticValue> {
        use StatisticKind::*;
        use StatisticUnit::*;
        [
            (Sharpe, 1.84, Ratio, true),
            (Sortino, 2.31, Ratio, true),
            (Calmar, 1.27, Ratio, true),
            (MaxDrawdown, 0.083, Percent, false),
            (ValueAtRisk, 0.021, Percent, false),
            (ExpectedShortfall, 0.034, Percent, false),
            (Expectancy, 4_280.0, CurrencyCents, true),
            (ProfitFactor, 1.63, Ratio, true),
            (WinRate, 0.574, Percent, true),
            (TailRatio, 1.19, Ratio, true),
            (OmegaRatio, 1.42, Ratio, true),
            (Alpha, 0.061, Percent, true),
            (Beta, 0.76, Ratio, false),
        ]
        .into_iter()
        .map(
            |(kind, value, unit, favorable_when_positive)| StatisticValue {
                kind,
                value,
                unit,
                favorable_when_positive,
            },
        )
        .collect()
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Dense engine-statistic tiles for a strategy or portfolio.
pub struct StatisticTileGrid {
    values: Vec<StatisticValue>,
    tile_width: f32,
    tokens: Option<MarketTokens>,
}

impl StatisticTileGrid {
    pub fn new(values: Vec<StatisticValue>) -> Self {
        Self {
            values,
            tile_width: 132.0,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl StatisticGridSource) -> Self {
        Self::new(source.statistics())
    }

    pub fn tile_width(mut self, width: f32) -> Self {
        self.tile_width = width.max(96.0);
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for StatisticTileGrid {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let number_font = market_number_font(cx);
        let tile_width = self.tile_width;
        h_flex()
            .debug_selector(|| "market.statistic_tile_grid".into())
            .items_stretch()
            .flex_wrap()
            .gap_1()
            .children(self.values.into_iter().map(|value| {
                let direction = value.direction();
                v_flex()
                    .w(px(tile_width))
                    .p_2()
                    .gap_1()
                    .border_1()
                    .border_color(tokens.grid)
                    .bg(tokens.surface)
                    .child(
                        Label::new(value.kind.label())
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        div()
                            .font(number_font.clone())
                            .text_size(px(15.0))
                            .text_color(tokens.direction_color(direction))
                            .child(format!("{} {}", direction.glyph(), value.display())),
                    )
            }))
    }
}

impl Component for StatisticTileGrid {
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
                "Strategy statistics",
                vec![single_example(
                    "Risk, expectancy, tail, and benchmark statistics",
                    StatisticTileGrid::from_source(&DemoStatisticGridSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Signed glyphs retain favorable and adverse direction",
                    StatisticTileGrid::from_source(&DemoStatisticGridSource)
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BookImbalanceValue {
    pub bid_fraction: f64,
    pub sequence: u64,
}

pub trait BookImbalanceSource {
    fn book_imbalance(&self) -> BookImbalanceValue;
}

pub struct DemoBookImbalanceSource;

impl BookImbalanceSource for DemoBookImbalanceSource {
    fn book_imbalance(&self) -> BookImbalanceValue {
        BookImbalanceValue {
            bid_fraction: 0.63,
            sequence: 42,
        }
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Streaming bid/ask imbalance rendered by the shared gauge primitive.
pub struct BookImbalanceGauge {
    value: BookImbalanceValue,
    width: f32,
    tokens: Option<MarketTokens>,
}

impl BookImbalanceGauge {
    pub fn new(value: BookImbalanceValue) -> Self {
        Self {
            value,
            width: 360.0,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl BookImbalanceSource) -> Self {
        Self::new(source.book_imbalance())
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for BookImbalanceGauge {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let bid_fraction = self.value.bid_fraction.clamp(0.0, 1.0);
        let mut gauge = Gauge::new(HeadroomMeter {
            label: format!("Book imbalance · seq {}", self.value.sequence).into(),
            used_display: format!("Bid {}%", (bid_fraction * 100.0).round()).into(),
            limit_display: format!("Ask {}%", ((1.0 - bid_fraction) * 100.0).round()).into(),
            fraction: bid_fraction as f32,
        })
        .thresholds(0.7, 0.9)
        .width(self.width);
        if let Some(tokens) = self.tokens {
            gauge = gauge.tokens(tokens);
        }
        div()
            .debug_selector(|| "market.book_imbalance_gauge".into())
            .child(gauge)
    }
}

impl Component for BookImbalanceGauge {
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
                "Order-book imbalance",
                vec![single_example(
                    "Bid and ask share the threshold gauge",
                    BookImbalanceGauge::from_source(&DemoBookImbalanceSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Bid/ask labels and fill position retain the reading",
                    BookImbalanceGauge::from_source(&DemoBookImbalanceSource)
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
    fn demo_grid_contains_the_complete_statistic_set() {
        let values = DemoStatisticGridSource.statistics();
        assert_eq!(values.len(), 13);
        assert_eq!(
            values.first().map(|value| value.kind),
            Some(StatisticKind::Sharpe)
        );
        assert_eq!(
            values.last().map(|value| value.kind),
            Some(StatisticKind::Beta)
        );
    }

    #[test]
    fn book_imbalance_is_a_bounded_shared_gauge_fraction() {
        let value = DemoBookImbalanceSource.book_imbalance();
        assert!((0.0..=1.0).contains(&value.bid_fraction));
    }
}
