use std::sync::Arc;

use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, SharedString, Window, px};
use ui::{CopyButton, MarketTokens, QrCodeCanvas, prelude::*};

use crate::portfolio_accounting::format_asset_amount;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferDirection {
    Deposit,
    Withdraw,
}

impl TransferDirection {
    fn label(self) -> &'static str {
        match self {
            Self::Deposit => "Deposit",
            Self::Withdraw => "Withdraw",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransferRail {
    LightningInvoice {
        invoice: SharedString,
        expires_at_ms: i64,
    },
    Onchain {
        address: SharedString,
    },
    EvmBridge {
        bridge: SharedString,
        source_chain: SharedString,
        destination_chain: SharedString,
        destination: SharedString,
    },
}

impl TransferRail {
    fn label(&self) -> &'static str {
        match self {
            Self::LightningInvoice { .. } => "Lightning invoice",
            Self::Onchain { .. } => "On-chain Bitcoin",
            Self::EvmBridge { .. } => "EVM bridge",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferRequest {
    pub direction: TransferDirection,
    pub venue: SharedString,
    pub network: SharedString,
    pub asset: SharedString,
    pub amount: i64,
    pub rail: TransferRail,
}

type TransferHandler = Arc<dyn Fn(&TransferRequest, &mut Window, &mut App) + 'static>;

#[derive(IntoElement, RegisterComponent)]
pub struct DepositWithdrawFlow {
    request: TransferRequest,
    now_ms: i64,
    on_continue: Option<TransferHandler>,
    tokens: Option<MarketTokens>,
}

impl DepositWithdrawFlow {
    pub fn new(request: TransferRequest, now_ms: i64) -> Self {
        Self {
            request,
            now_ms,
            on_continue: None,
            tokens: None,
        }
    }

    pub fn on_continue(
        mut self,
        handler: impl Fn(&TransferRequest, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_continue = Some(Arc::new(handler));
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

fn payload_row(
    id: &'static str,
    label: &'static str,
    payload: SharedString,
    tokens: MarketTokens,
) -> AnyElement {
    h_flex()
        .w_full()
        .gap_2()
        .child(
            Label::new(label)
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_ellipsis()
                .font_family("monospace")
                .text_size(px(11.0))
                .text_color(tokens.text)
                .child(payload.clone()),
        )
        .child(CopyButton::new(id, payload).icon_size(IconSize::XSmall))
        .into_any_element()
}

impl RenderOnce for DepositWithdrawFlow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let rail_detail = match &self.request.rail {
            TransferRail::LightningInvoice {
                invoice,
                expires_at_ms,
            } => {
                let qr = QrCodeCanvas::encode(invoice.as_bytes())
                    .ok()
                    .map(|qr| qr.size(136.0).tokens(tokens));
                let qr_missing = qr.is_none();
                h_flex()
                    .items_start()
                    .gap_3()
                    .child(
                        div()
                            .size(px(136.0))
                            .when_some(qr, |this, qr| this.child(qr))
                            .when(qr_missing, |this| {
                                this.flex()
                                    .items_center()
                                    .justify_center()
                                    .border_1()
                                    .border_color(tokens.grid)
                                    .child(
                                        Label::new("QR unavailable")
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    )
                            }),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_2()
                            .child(payload_row(
                                "transfer-lightning-invoice",
                                "Invoice",
                                invoice.clone(),
                                tokens,
                            ))
                            .child(
                                Label::new(crate::format::format_countdown(
                                    *expires_at_ms,
                                    self.now_ms,
                                ))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            ),
                    )
                    .into_any_element()
            }
            TransferRail::Onchain { address } => payload_row(
                "transfer-onchain-address",
                "Address",
                address.clone(),
                tokens,
            ),
            TransferRail::EvmBridge {
                bridge,
                source_chain,
                destination_chain,
                destination,
            } => v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child(Label::new(source_chain.clone()).size(LabelSize::Small))
                        .child(Icon::new(IconName::ArrowRight).size(IconSize::XSmall))
                        .child(Label::new(destination_chain.clone()).size(LabelSize::Small))
                        .child(
                            Label::new(bridge.clone())
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                )
                .child(payload_row(
                    "transfer-bridge-destination",
                    "Destination",
                    destination.clone(),
                    tokens,
                ))
                .child(
                    Label::new("Use the named bridge and destination network")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .into_any_element(),
        };
        let request = self.request;
        let callback_request = request.clone();
        let handler = self.on_continue;
        v_flex()
            .debug_selector(|| "command_center.deposit_withdraw_flow".into())
            .w_full()
            .max_w(px(620.0))
            .gap_3()
            .p_3()
            .border_1()
            .border_color(tokens.grid)
            .bg(tokens.surface)
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        Label::new(format!(
                            "{} · {}",
                            request.direction.label(),
                            request.rail.label()
                        ))
                        .size(LabelSize::Small),
                    )
                    .child(
                        Label::new(format!("{} · {}", request.venue, request.network))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .child(
                div()
                    .font(ui::market_number_font(cx))
                    .text_size(px(16.0))
                    .text_color(tokens.text)
                    .child(format_asset_amount(request.amount, request.asset.as_ref())),
            )
            .child(rail_detail)
            .child(
                h_flex().justify_end().child(
                    Button::new("transfer-continue", "Continue")
                        .style(ButtonStyle::Filled)
                        .when_some(handler, move |button, handler| {
                            button.on_click(move |_, window, cx| {
                                handler(&callback_request, window, cx)
                            })
                        }),
                ),
            )
    }
}

fn demo_lightning() -> TransferRequest {
    TransferRequest {
        direction: TransferDirection::Deposit,
        venue: "lnmarkets".into(),
        network: "signet".into(),
        asset: "sats".into(),
        amount: 250_000,
        rail: TransferRail::LightningInvoice {
            invoice: "lntbs2500n1pomega298pp5marketaccountingdemo".into(),
            expires_at_ms: 1_754_703_600_000,
        },
    }
}

fn demo_bridge() -> TransferRequest {
    TransferRequest {
        direction: TransferDirection::Withdraw,
        venue: "hyperliquid".into(),
        network: "testnet".into(),
        asset: "USDC".into(),
        amount: 250_000_00,
        rail: TransferRail::EvmBridge {
            bridge: "Hyperliquid bridge".into(),
            source_chain: "Hyperliquid".into(),
            destination_chain: "Arbitrum Sepolia".into(),
            destination: "0x298000000000000000000000000000000000c0de".into(),
        },
    }
}

impl Component for DepositWithdrawFlow {
    fn scope() -> ComponentScope {
        ComponentScope::Input
    }

    fn description() -> &'static str {
        "Venue-specific deposit and withdrawal rail summary with Lightning QR and explicit bridge routing."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let now_ms = 1_754_700_000_000;
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Deposits and withdrawals",
                vec![
                    single_example(
                        "Lightning invoice deposit",
                        DepositWithdrawFlow::new(demo_lightning(), now_ms).into_any_element(),
                    ),
                    single_example(
                        "EVM bridge withdrawal",
                        DepositWithdrawFlow::new(demo_bridge(), now_ms).into_any_element(),
                    ),
                ],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Rail, network, and direction remain textual",
                    DepositWithdrawFlow::new(demo_lightning(), now_ms)
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
    fn lightning_deposit_keeps_invoice_and_expiry_typed() {
        let request = demo_lightning();
        let TransferRail::LightningInvoice {
            invoice,
            expires_at_ms,
        } = request.rail
        else {
            panic!("demo must use the Lightning rail");
        };
        assert!(invoice.starts_with("lntbs"));
        assert!(expires_at_ms > 1_754_700_000_000);
    }

    #[test]
    fn bridge_withdrawal_names_both_networks() {
        let request = demo_bridge();
        let TransferRail::EvmBridge {
            source_chain,
            destination_chain,
            ..
        } = request.rail
        else {
            panic!("demo must use the bridge rail");
        };
        assert_ne!(source_chain, destination_chain);
        assert_eq!(request.direction, TransferDirection::Withdraw);
    }
}
