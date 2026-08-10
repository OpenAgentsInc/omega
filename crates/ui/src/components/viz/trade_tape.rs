use std::sync::Arc;

use documented::Documented;
use gpui::px;

use crate::Table;
use crate::components::viz::{
    HighFrequencyBatch, MarketTokens, format_with_decimals, market_number_font,
};
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeSide {
    Buy,
    Sell,
}
impl TradeSide {
    fn glyph(self) -> &'static str {
        match self {
            Self::Buy => "▲",
            Self::Sell => "▼",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct TapeTrade {
    pub trade_id: u64,
    pub time_ms: i64,
    pub price: f64,
    pub size: f64,
    pub side: TradeSide,
}
pub trait TradeTapeSource {
    fn tape_updates(&self) -> Vec<TapeTrade>;
}
pub struct DemoTradeTapeSource;
impl TradeTapeSource for DemoTradeTapeSource {
    fn tape_updates(&self) -> Vec<TapeTrade> {
        (0..180)
            .map(|trade_id| TapeTrade {
                trade_id,
                time_ms: 1_754_700_000_000 + trade_id as i64 * 220,
                price: 116_400.0 + (trade_id as f64 / 7.0).sin() * 28.0,
                size: 0.01 + (trade_id % 17) as f64 * 0.035,
                side: if trade_id % 3 == 0 {
                    TradeSide::Sell
                } else {
                    TradeSide::Buy
                },
            })
            .collect()
    }
}
fn frame_trades(source: &impl TradeTapeSource) -> Vec<TapeTrade> {
    let mut batch = HighFrequencyBatch::with_capacity(0, 256);
    for trade in source.tape_updates() {
        batch.push(trade);
    }
    batch.take_for_frame(1, 0).unwrap_or_default()
}
fn size_bucket(size: f64) -> &'static str {
    if size >= 0.4 {
        "●●●"
    } else if size >= 0.15 {
        "●●"
    } else {
        "●"
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Virtualized time-and-sales tape with bounded per-frame event batches.
pub struct TradeTape {
    trades: Vec<TapeTrade>,
    tokens: Option<MarketTokens>,
}
impl TradeTape {
    pub fn from_source(source: &impl TradeTapeSource) -> Self {
        Self {
            trades: frame_trades(source),
            tokens: None,
        }
    }
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}
impl RenderOnce for TradeTape {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let trades = Arc::new(self.trades);
        let count = trades.len();
        div()
            .debug_selector(|| "market.trade_tape".into())
            .w(px(480.))
            .h(px(260.))
            .child(
                Table::new(4)
                    .width(px(480.))
                    .header(vec!["time", "price", "size", "side"])
                    .uniform_list(
                        "market-trade-tape-rows",
                        count,
                        move |range, _window, cx| {
                            range
                                .filter_map(|index| trades.get(index))
                                .map(|trade| {
                                    let color = match trade.side {
                                        TradeSide::Buy => tokens.up,
                                        TradeSide::Sell => tokens.down,
                                    };
                                    let number = |value: String| {
                                        div()
                                            .font(market_number_font(cx))
                                            .text_size(px(11.))
                                            .text_color(tokens.text)
                                            .child(value)
                                            .into_any_element()
                                    };
                                    vec![
                                        number(format!(
                                            "{:06}",
                                            trade.time_ms.rem_euclid(1_000_000)
                                        )),
                                        number(format_with_decimals(trade.price, 2)),
                                        number(format_with_decimals(trade.size, 3)),
                                        div()
                                            .font(market_number_font(cx))
                                            .text_size(px(11.))
                                            .text_color(color)
                                            .child(format!(
                                                "{} {}",
                                                trade.side.glyph(),
                                                size_bucket(trade.size)
                                            ))
                                            .into_any_element(),
                                    ]
                                })
                                .collect()
                        },
                    ),
            )
    }
}
impl Component for TradeTape {
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
                "Time and sales",
                vec![single_example(
                    "Virtualized bounded trade stream",
                    TradeTape::from_source(&DemoTradeTapeSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Side arrows and size buckets retain meaning",
                    TradeTape::from_source(&DemoTradeTapeSource)
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
    fn tape_batch_preserves_events_and_buckets_size() {
        let trades = frame_trades(&DemoTradeTapeSource);
        assert_eq!(trades.len(), 180);
        assert_eq!(size_bucket(0.5), "●●●");
    }
}
