use std::sync::Arc;

use documented::Documented;
use gpui::px;

use crate::components::viz::{
    MarketDirection, MarketTokens, format_with_decimals, market_number_font,
};
use crate::prelude::*;
use crate::{Chip, Table};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenOrderStatus {
    Resting,
    PartiallyFilled,
}
#[derive(Debug, Clone, PartialEq)]
pub struct OpenOrder {
    pub client_order_id: SharedString,
    pub venue_order_id: SharedString,
    pub instrument: SharedString,
    pub direction: MarketDirection,
    pub price: f64,
    pub remaining: f64,
    pub status: OpenOrderStatus,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOrderAction {
    Cancel { client_order_id: SharedString },
    Modify { client_order_id: SharedString },
    CancelAll { client_order_ids: Vec<SharedString> },
}
pub trait OpenOrdersSource {
    fn open_orders(&self) -> Vec<OpenOrder>;
}
pub struct DemoOpenOrdersSource;
impl OpenOrdersSource for DemoOpenOrdersSource {
    fn open_orders(&self) -> Vec<OpenOrder> {
        (0..24)
            .map(|index| OpenOrder {
                client_order_id: format!("cloid-{index:04}").into(),
                venue_order_id: format!("oid-{:06}", 830_000 + index).into(),
                instrument: if index % 2 == 0 {
                    "BTC-PERP"
                } else {
                    "ETH-PERP"
                }
                .into(),
                direction: if index % 3 == 0 {
                    MarketDirection::Down
                } else {
                    MarketDirection::Up
                },
                price: 116_000.0 + index as f64 * 21.5,
                remaining: 0.01 + index as f64 * 0.002,
                status: if index % 5 == 0 {
                    OpenOrderStatus::PartiallyFilled
                } else {
                    OpenOrderStatus::Resting
                },
            })
            .collect()
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Virtualized open-order table with typed cancel and modify intents.
pub struct OpenOrdersTable {
    orders: Vec<OpenOrder>,
    tokens: Option<MarketTokens>,
    on_action: Option<Arc<dyn Fn(OpenOrderAction, &mut Window, &mut App) + 'static>>,
}
impl OpenOrdersTable {
    pub fn from_source(source: &impl OpenOrdersSource) -> Self {
        Self {
            orders: source.open_orders(),
            tokens: None,
            on_action: None,
        }
    }
    pub fn cancel_action(&self, client_order_id: SharedString) -> OpenOrderAction {
        OpenOrderAction::Cancel { client_order_id }
    }
    pub fn modify_action(&self, client_order_id: SharedString) -> OpenOrderAction {
        OpenOrderAction::Modify { client_order_id }
    }
    pub fn cancel_all_action(&self) -> OpenOrderAction {
        OpenOrderAction::CancelAll {
            client_order_ids: self
                .orders
                .iter()
                .map(|order| order.client_order_id.clone())
                .collect(),
        }
    }
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
    pub fn on_action(
        mut self,
        handler: impl Fn(OpenOrderAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Arc::new(handler));
        self
    }
}
impl RenderOnce for OpenOrdersTable {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let cancel_all_action = self.cancel_all_action();
        let cancel_all_handler = self.on_action.clone();
        let row_handler = self.on_action;
        let orders = Arc::new(self.orders);
        let count = orders.len();
        v_flex()
            .debug_selector(|| "market.open_orders".into())
            .w(px(820.))
            .h(px(300.))
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("Open orders").size(LabelSize::Small))
                    .child(
                        Button::new("open-orders-cancel-all", "Cancel all").when_some(
                            cancel_all_handler,
                            |button, handler| {
                                button.on_click(move |_, window, cx| {
                                    handler(cancel_all_action.clone(), window, cx);
                                })
                            },
                        ),
                    ),
            )
            .child(
                Table::new(8)
                    .width(px(820.))
                    .header(vec![
                        "instrument",
                        "side",
                        "price",
                        "remaining",
                        "status",
                        "cloid",
                        "oid",
                        "actions",
                    ])
                    .uniform_list(
                        "market-open-orders-rows",
                        count,
                        move |range, _window, cx| {
                            range
                                .filter_map(|row_index| {
                                    orders.get(row_index).map(|order| (row_index, order))
                                })
                                .map(|(row_index, order)| {
                                    let modify_handler = row_handler.clone();
                                    let cancel_handler = row_handler.clone();
                                    let modify_action = OpenOrderAction::Modify {
                                        client_order_id: order.client_order_id.clone(),
                                    };
                                    let cancel_action = OpenOrderAction::Cancel {
                                        client_order_id: order.client_order_id.clone(),
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
                                        Label::new(order.instrument.clone())
                                            .size(LabelSize::Small)
                                            .into_any_element(),
                                        div()
                                            .text_color(tokens.direction_color(order.direction))
                                            .child(order.direction.glyph())
                                            .into_any_element(),
                                        number(format_with_decimals(order.price, 2)),
                                        number(format_with_decimals(order.remaining, 3)),
                                        Chip::new(match order.status {
                                            OpenOrderStatus::Resting => "◉ resting",
                                            OpenOrderStatus::PartiallyFilled => "◐ partial",
                                        })
                                        .into_any_element(),
                                        Label::new(order.client_order_id.clone())
                                            .size(LabelSize::XSmall)
                                            .into_any_element(),
                                        Label::new(order.venue_order_id.clone())
                                            .size(LabelSize::XSmall)
                                            .into_any_element(),
                                        h_flex()
                                            .gap_1()
                                            .child(
                                                Button::new(("modify", row_index), "Modify")
                                                    .when_some(
                                                        modify_handler,
                                                        |button, handler| {
                                                            button.on_click(move |_, window, cx| {
                                                                handler(
                                                                    modify_action.clone(),
                                                                    window,
                                                                    cx,
                                                                );
                                                            })
                                                        },
                                                    ),
                                            )
                                            .child(
                                                Button::new(("cancel", row_index), "Cancel")
                                                    .when_some(
                                                        cancel_handler,
                                                        |button, handler| {
                                                            button.on_click(move |_, window, cx| {
                                                                handler(
                                                                    cancel_action.clone(),
                                                                    window,
                                                                    cx,
                                                                );
                                                            })
                                                        },
                                                    ),
                                            )
                                            .into_any_element(),
                                    ]
                                })
                                .collect()
                        },
                    ),
            )
    }
}
impl Component for OpenOrdersTable {
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
                "Open orders",
                vec![single_example(
                    "Virtualized cancel/modify table",
                    OpenOrdersTable::from_source(&DemoOpenOrdersSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Side glyphs and status labels preserve state",
                    OpenOrdersTable::from_source(&DemoOpenOrdersSource)
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
    fn batch_cancel_contains_every_visible_order_id() {
        let table = OpenOrdersTable::from_source(&DemoOpenOrdersSource);
        assert!(
            matches!(table.cancel_all_action(), OpenOrderAction::CancelAll { client_order_ids } if client_order_ids.len() == 24)
        );
    }
}
