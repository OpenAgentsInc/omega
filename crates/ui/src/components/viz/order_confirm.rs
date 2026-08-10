use std::sync::Arc;

use documented::Documented;
use gpui::px;

use crate::components::viz::{
    MarketEnvironment, MarketTokens, format_usd_cents, market_number_font,
};
use crate::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct OrderConfirmation {
    pub request_id: SharedString,
    pub exact_order: SharedString,
    pub estimated_cost_cents: i64,
    pub headroom_consumed_bps: u32,
    pub counterparty: SharedString,
    pub environment: MarketEnvironment,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderConfirmationAction {
    Approve { request_id: SharedString },
    Reject { request_id: SharedString },
}
pub trait OrderConfirmationSource {
    fn order_confirmation(&self) -> OrderConfirmation;
}
pub struct DemoOrderConfirmationSource;
impl OrderConfirmationSource for DemoOrderConfirmationSource {
    fn order_confirmation(&self) -> OrderConfirmation {
        OrderConfirmation {
            request_id: "order-demo-42".into(),
            exact_order: "BUY 0.080 BTC-PERP · LIMIT 116,400.00 · GTC".into(),
            estimated_cost_cents: 38_800,
            headroom_consumed_bps: 1_940,
            counterparty: "Hyperliquid".into(),
            environment: MarketEnvironment::Testnet,
        }
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Exact reusable order confirmation for inline cards and modal hosts.
pub struct OrderConfirmDialog {
    confirmation: OrderConfirmation,
    tokens: Option<MarketTokens>,
    on_action: Option<Arc<dyn Fn(OrderConfirmationAction, &mut Window, &mut App) + 'static>>,
}
impl OrderConfirmDialog {
    pub fn from_source(source: &impl OrderConfirmationSource) -> Self {
        Self {
            confirmation: source.order_confirmation(),
            tokens: None,
            on_action: None,
        }
    }
    pub fn approve_action(&self) -> OrderConfirmationAction {
        OrderConfirmationAction::Approve {
            request_id: self.confirmation.request_id.clone(),
        }
    }
    pub fn reject_action(&self) -> OrderConfirmationAction {
        OrderConfirmationAction::Reject {
            request_id: self.confirmation.request_id.clone(),
        }
    }
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
    pub fn on_action(
        mut self,
        handler: impl Fn(OrderConfirmationAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Arc::new(handler));
        self
    }
}
impl RenderOnce for OrderConfirmDialog {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let approve_action = self.approve_action();
        let reject_action = self.reject_action();
        let approve_handler = self.on_action.clone();
        let reject_handler = self.on_action;
        let row = |label, value: String| {
            h_flex()
                .justify_between()
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
            .debug_selector(|| "market.order_confirm".into())
            .w(px(500.))
            .gap_3()
            .p_3()
            .child(Label::new("Confirm order").size(LabelSize::Large))
            .child(
                div()
                    .font(market_number_font(cx))
                    .text_size(px(12.))
                    .text_color(tokens.text)
                    .child(self.confirmation.exact_order),
            )
            .child(row(
                "estimated cost",
                format_usd_cents(self.confirmation.estimated_cost_cents),
            ))
            .child(row(
                "mandate headroom",
                format!(
                    "{:.2}%",
                    self.confirmation.headroom_consumed_bps as f64 / 100.0
                ),
            ))
            .child(row(
                "counterparty",
                self.confirmation.counterparty.to_string(),
            ))
            .child(row("network", self.confirmation.environment.label().into()))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("order-confirm-reject", "Reject").when_some(
                        reject_handler,
                        |button, handler| {
                            button.on_click(move |_, window, cx| {
                                handler(reject_action.clone(), window, cx);
                            })
                        },
                    ))
                    .child(
                        Button::new("order-confirm-approve", "Confirm order")
                            .style(ButtonStyle::Filled)
                            .when_some(approve_handler, |button, handler| {
                                button.on_click(move |_, window, cx| {
                                    handler(approve_action.clone(), window, cx);
                                })
                            }),
                    ),
            )
    }
}
impl Component for OrderConfirmDialog {
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
                "Order confirmation",
                vec![single_example(
                    "Exact order and authority boundary",
                    OrderConfirmDialog::from_source(&DemoOrderConfirmationSource)
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Every decision input remains named",
                    OrderConfirmDialog::from_source(&DemoOrderConfirmationSource)
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
    fn actions_retain_exact_request_id() {
        let dialog = OrderConfirmDialog::from_source(&DemoOrderConfirmationSource);
        assert_eq!(
            dialog.approve_action(),
            OrderConfirmationAction::Approve {
                request_id: "order-demo-42".into()
            }
        );
    }
}
