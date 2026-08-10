use documented::Documented;
use gpui::px;

use crate::components::viz::{
    FlashOnChangeExt, HighFrequencyBatch, MarketTokens, format_signed_percent,
    format_with_decimals, market_number_font,
};
use crate::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct PriceTick {
    pub instrument: SharedString,
    pub last: f64,
    pub mark: f64,
    pub index: f64,
    pub oracle: f64,
    pub change_fraction: f64,
    pub sequence: u64,
}

pub trait PriceTickerSource {
    fn price_ticks(&self) -> Vec<PriceTick>;
}

pub struct DemoPriceTickerSource;

impl PriceTickerSource for DemoPriceTickerSource {
    fn price_ticks(&self) -> Vec<PriceTick> {
        (0..48)
            .map(|sequence| PriceTick {
                instrument: "BTC-PERP".into(),
                last: 116_420.0 + sequence as f64 * 3.25,
                mark: 116_418.5 + sequence as f64 * 3.1,
                index: 116_401.2 + sequence as f64 * 2.9,
                oracle: 116_407.8 + sequence as f64 * 3.0,
                change_fraction: 0.0184,
                sequence,
            })
            .collect()
    }
}

fn latest_tick(source: &impl PriceTickerSource) -> Option<PriceTick> {
    let mut updates = HighFrequencyBatch::with_capacity(16, 256);
    for tick in source.price_ticks() {
        updates.push(tick);
    }
    updates.take_latest_for_frame(1, 16)
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Last, mark, index, and oracle prices coalesced to one delivery per frame.
pub struct PriceTicker {
    tick: Option<PriceTick>,
    tokens: Option<MarketTokens>,
}

impl PriceTicker {
    pub fn from_source(source: &impl PriceTickerSource) -> Self {
        Self {
            tick: latest_tick(source),
            tokens: None,
        }
    }
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for PriceTicker {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let tick = self.tick.unwrap_or(PriceTick {
            instrument: "—".into(),
            last: 0.0,
            mark: 0.0,
            index: 0.0,
            oracle: 0.0,
            change_fraction: 0.0,
            sequence: 0,
        });
        let (change, direction) = format_signed_percent(tick.change_fraction, 2);
        let color = tokens.direction_color(direction);
        let metric = |label, value: f64| {
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
                        .child(format_with_decimals(value, 2)),
                )
        };
        h_flex()
            .debug_selector(|| "market.price_ticker".into())
            .gap_4()
            .p_2()
            .child(
                v_flex()
                    .gap_0p5()
                    .child(Label::new(tick.instrument).size(LabelSize::Small))
                    .child(
                        div()
                            .font(market_number_font(cx))
                            .text_size(px(18.))
                            .text_color(color)
                            .child(format!(
                                "{} {}",
                                direction.glyph(),
                                format_with_decimals(tick.last, 2)
                            )),
                    ),
            )
            .child(metric("mark", tick.mark))
            .child(metric("index", tick.index))
            .child(metric("oracle", tick.oracle))
            .child(
                div()
                    .font(market_number_font(cx))
                    .text_size(px(11.))
                    .text_color(color)
                    .child(change),
            )
            .with_change_flash(
                "market-price-ticker",
                tick.sequence,
                color,
                |element, overlay| element.bg(overlay),
            )
    }
}

impl Component for PriceTicker {
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
                "Price ticker",
                vec![single_example(
                    "Coalesced market prices",
                    PriceTicker::from_source(&DemoPriceTickerSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Direction glyph and sign preserve movement",
                    PriceTicker::from_source(&DemoPriceTickerSource)
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
    fn ticker_coalesces_to_latest_tick() {
        assert_eq!(
            latest_tick(&DemoPriceTickerSource).map(|tick| tick.sequence),
            Some(47)
        );
    }
}
