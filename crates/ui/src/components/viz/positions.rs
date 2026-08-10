use std::sync::Arc;

use documented::Documented;
use gpui::px;

use crate::components::viz::{
    Gauge, HeadroomMeter, MarketDirection, MarketTokens, format_signed, format_with_decimals,
    market_number_font,
};
use crate::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    pub position_id: SharedString,
    pub instrument: SharedString,
    pub direction: MarketDirection,
    pub units: f64,
    pub entry_price: f64,
    pub mark_price: f64,
    pub liquidation_price: f64,
    pub unrealized_pnl: f64,
}
#[derive(Debug, Clone, PartialEq)]
pub enum PositionAction {
    Close {
        position_id: SharedString,
    },
    Reduce {
        position_id: SharedString,
        fraction_bps: u32,
    },
}
pub trait PositionsSource {
    fn positions(&self) -> Vec<Position>;
}
pub struct DemoPositionsSource;
impl PositionsSource for DemoPositionsSource {
    fn positions(&self) -> Vec<Position> {
        vec![
            Position {
                position_id: "position-btc-1".into(),
                instrument: "BTC-PERP".into(),
                direction: MarketDirection::Up,
                units: 0.08,
                entry_price: 114_200.0,
                mark_price: 116_420.0,
                liquidation_price: 82_800.0,
                unrealized_pnl: 177.60,
            },
            Position {
                position_id: "position-eth-1".into(),
                instrument: "ETH-PERP".into(),
                direction: MarketDirection::Down,
                units: 1.4,
                entry_price: 4_180.0,
                mark_price: 4_120.0,
                liquidation_price: 5_360.0,
                unrealized_pnl: 84.0,
            },
        ]
    }
}
fn liquidation_fraction(position: &Position) -> f32 {
    let total = (position.entry_price - position.liquidation_price)
        .abs()
        .max(1e-9);
    let remaining = (position.mark_price - position.liquidation_price).abs();
    (1.0 - remaining / total).clamp(0.0, 1.0) as f32
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Positions with liquidation-distance gauges and typed close/reduce intents.
pub struct PositionsPanel {
    positions: Vec<Position>,
    tokens: Option<MarketTokens>,
    on_action: Option<Arc<dyn Fn(PositionAction, &mut Window, &mut App) + 'static>>,
}
impl PositionsPanel {
    pub fn from_source(source: &impl PositionsSource) -> Self {
        Self {
            positions: source.positions(),
            tokens: None,
            on_action: None,
        }
    }
    pub fn close_action(position_id: SharedString) -> PositionAction {
        PositionAction::Close { position_id }
    }
    pub fn reduce_action(position_id: SharedString, fraction_bps: u32) -> PositionAction {
        PositionAction::Reduce {
            position_id,
            fraction_bps: fraction_bps.min(10_000),
        }
    }
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
    pub fn on_action(
        mut self,
        handler: impl Fn(PositionAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Arc::new(handler));
        self
    }
}
impl RenderOnce for PositionsPanel {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let on_action = self.on_action;
        v_flex()
            .debug_selector(|| "market.positions".into())
            .w(px(620.))
            .gap_3()
            .children(
                self.positions
                    .into_iter()
                    .enumerate()
                    .map(|(row_index, position)| {
                        let reduce_handler = on_action.clone();
                        let close_handler = on_action.clone();
                        let reduce_action =
                            Self::reduce_action(position.position_id.clone(), 2_500);
                        let close_action = Self::close_action(position.position_id.clone());
                        let (pnl, pnl_direction) = format_signed(position.unrealized_pnl, 2);
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
                        v_flex()
                            .gap_2()
                            .p_2()
                            .border_b_1()
                            .border_color(tokens.grid)
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(
                                        Label::new(position.instrument.clone())
                                            .size(LabelSize::Small),
                                    )
                                    .child(
                                        div()
                                            .font(market_number_font(cx))
                                            .text_color(tokens.direction_color(pnl_direction))
                                            .child(format!("{} ${pnl}", pnl_direction.glyph())),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_4()
                                    .child(metric(
                                        "entry",
                                        format_with_decimals(position.entry_price, 2),
                                    ))
                                    .child(metric(
                                        "mark",
                                        format_with_decimals(position.mark_price, 2),
                                    ))
                                    .child(metric(
                                        "liquidation",
                                        format_with_decimals(position.liquidation_price, 2),
                                    ))
                                    .child(metric("size", format_with_decimals(position.units, 3))),
                            )
                            .child(
                                Gauge::new(HeadroomMeter {
                                    label: "liquidation distance".into(),
                                    used_display: format_with_decimals(position.mark_price, 2)
                                        .into(),
                                    limit_display: format_with_decimals(
                                        position.liquidation_price,
                                        2,
                                    )
                                    .into(),
                                    fraction: liquidation_fraction(&position),
                                })
                                .width(590.)
                                .tokens(tokens),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new(("reduce", row_index), "Reduce 25%").when_some(
                                            reduce_handler,
                                            |button, handler| {
                                                button.on_click(move |_, window, cx| {
                                                    handler(reduce_action.clone(), window, cx);
                                                })
                                            },
                                        ),
                                    )
                                    .child(Button::new(("close", row_index), "Close").when_some(
                                        close_handler,
                                        |button, handler| {
                                            button.on_click(move |_, window, cx| {
                                                handler(close_action.clone(), window, cx);
                                            })
                                        },
                                    )),
                            )
                    }),
            )
    }
}
impl Component for PositionsPanel {
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
                "Positions",
                vec![single_example(
                    "PnL and liquidation headroom",
                    PositionsPanel::from_source(&DemoPositionsSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Signs and gauges preserve position risk",
                    PositionsPanel::from_source(&DemoPositionsSource)
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
    fn liquidation_distance_is_bounded() {
        assert!(
            DemoPositionsSource
                .positions()
                .iter()
                .all(|position| (0.0..=1.0).contains(&liquidation_fraction(position)))
        );
    }
    #[test]
    fn reduction_is_bounded_to_full_position() {
        assert_eq!(
            PositionsPanel::reduce_action("p".into(), 20_000),
            PositionAction::Reduce {
                position_id: "p".into(),
                fraction_bps: 10_000
            }
        );
    }
}
