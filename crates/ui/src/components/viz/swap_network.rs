//! Swap and network-lane component-library surfaces.

use std::sync::Arc;

use documented::Documented;
use gpui::px;

use crate::components::viz::{
    MarketTokens, QrCodeCanvas, format_sats, market_number_font, time_readout_label,
};
use crate::prelude::*;
use crate::{Chip, CopyButton, Table};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferRail {
    Onchain,
    Lightning,
    Liquid,
    Ark,
}

impl TransferRail {
    pub fn label(self) -> &'static str {
        match self {
            Self::Onchain => "Onchain",
            Self::Lightning => "Lightning",
            Self::Liquid => "Liquid",
            Self::Ark => "Ark",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Onchain => "₿",
            Self::Lightning => "ϟ",
            Self::Liquid => "≈",
            Self::Ark => "⌁",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RailChoice {
    pub rail: TransferRail,
    pub fee_hint: SharedString,
    pub latency_hint: SharedString,
    pub available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RailSelectorValue {
    pub asset: SharedString,
    pub network: SharedString,
    pub selected: TransferRail,
    pub choices: Vec<RailChoice>,
}

pub trait RailSelectorSource {
    fn rail_selector(&self) -> RailSelectorValue;
}

pub struct DemoRailSelectorSource;

impl RailSelectorSource for DemoRailSelectorSource {
    fn rail_selector(&self) -> RailSelectorValue {
        RailSelectorValue {
            asset: "BTC".into(),
            network: "testnet".into(),
            selected: TransferRail::Lightning,
            choices: vec![
                RailChoice {
                    rail: TransferRail::Onchain,
                    fee_hint: "4–12 sat/vB".into(),
                    latency_hint: "~30 min".into(),
                    available: true,
                },
                RailChoice {
                    rail: TransferRail::Lightning,
                    fee_hint: "0.1% max".into(),
                    latency_hint: "~2 sec".into(),
                    available: true,
                },
                RailChoice {
                    rail: TransferRail::Liquid,
                    fee_hint: "0.1 sat/vB".into(),
                    latency_hint: "~2 min".into(),
                    available: true,
                },
                RailChoice {
                    rail: TransferRail::Ark,
                    fee_hint: "batch quote".into(),
                    latency_hint: "next round".into(),
                    available: false,
                },
            ],
        }
    }
}

type RailSelectHandler = Arc<dyn Fn(TransferRail, &mut Window, &mut App) + 'static>;

#[derive(IntoElement, RegisterComponent, Documented)]
/// A rail choice with explicit asset, network, fee, latency, and availability.
pub struct RailSelector {
    value: RailSelectorValue,
    on_select: Option<RailSelectHandler>,
    tokens: Option<MarketTokens>,
}

impl RailSelector {
    pub fn from_source(source: &impl RailSelectorSource) -> Self {
        Self {
            value: source.rail_selector(),
            on_select: None,
            tokens: None,
        }
    }

    pub fn on_select(
        mut self,
        handler: impl Fn(TransferRail, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(Arc::new(handler));
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for RailSelector {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let number_font = market_number_font(cx);
        let selected = self.value.selected;
        let handler = self.on_select;
        v_flex()
            .debug_selector(|| "market.rail_selector".into())
            .w(px(600.))
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("Rail").size(LabelSize::Small))
                    .child(
                        Label::new(format!("{} · {}", self.value.asset, self.value.network))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .children(self.value.choices.into_iter().map(move |choice| {
                        let is_selected = choice.rail == selected;
                        let choice_handler = handler.clone();
                        let rail = choice.rail;
                        v_flex()
                            .id(SharedString::from(format!(
                                "rail-choice-{}",
                                choice.rail.label()
                            )))
                            .flex_1()
                            .min_w_0()
                            .p_2()
                            .gap_1()
                            .border_1()
                            .border_color(if is_selected { tokens.up } else { tokens.grid })
                            .bg(if is_selected {
                                tokens.up.opacity(0.08)
                            } else {
                                tokens.surface
                            })
                            .opacity(if choice.available { 1.0 } else { 0.5 })
                            .when(choice.available && choice_handler.is_some(), |this| {
                                this.cursor_pointer().on_click(move |_, window, cx| {
                                    if let Some(handler) = choice_handler.as_ref() {
                                        handler(rail, window, cx);
                                    }
                                })
                            })
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(format!(
                                        "{} {}",
                                        choice.rail.glyph(),
                                        choice.rail.label()
                                    ))
                                    .when(is_selected, |this| this.child("●"))
                                    .when(!choice.available, |this| this.child("×")),
                            )
                            .child(
                                Label::new(choice.fee_hint)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                div()
                                    .font(number_font.clone())
                                    .text_size(px(11.))
                                    .text_color(tokens.muted)
                                    .child(choice.latency_hint),
                            )
                    })),
            )
    }
}

impl Component for RailSelector {
    fn scope() -> ComponentScope {
        ComponentScope::Input
    }
    fn description() -> &'static str {
        Self::DOCS
    }
    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Rail selector",
                vec![single_example(
                    "Typed rail choices with fee and latency",
                    RailSelector::from_source(&DemoRailSelectorSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Selection and availability retain glyphs and borders",
                    RailSelector::from_source(&DemoRailSelectorSource)
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LightningInvoiceKind {
    Bolt11,
    TaprootAsset,
}

impl LightningInvoiceKind {
    fn label(self) -> &'static str {
        match self {
            Self::Bolt11 => "BOLT11",
            Self::TaprootAsset => "Taproot Asset",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvoicePaymentState {
    Pending,
    Paid,
    Expired,
}

impl InvoicePaymentState {
    fn glyph(self) -> &'static str {
        match self {
            Self::Pending => "◷",
            Self::Paid => "✓",
            Self::Expired => "×",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LightningInvoiceValue {
    pub kind: LightningInvoiceKind,
    pub invoice: SharedString,
    pub asset: SharedString,
    pub network: SharedString,
    pub amount_sats: u64,
    pub payment_state: InvoicePaymentState,
    pub checked_at_ms: i64,
}

pub trait LightningInvoiceSource {
    fn lightning_invoice(&self) -> LightningInvoiceValue;
}

pub struct DemoLightningInvoiceSource;

impl LightningInvoiceSource for DemoLightningInvoiceSource {
    fn lightning_invoice(&self) -> LightningInvoiceValue {
        LightningInvoiceValue {
            kind: LightningInvoiceKind::Bolt11,
            invoice: "lntb250u1pomega300pp5qexampleinvoiceforcomponentpreview".into(),
            asset: "BTC".into(),
            network: "testnet".into(),
            amount_sats: 25_000,
            payment_state: InvoicePaymentState::Pending,
            checked_at_ms: 1_786_310_400_000,
        }
    }
}

type InvoiceRefreshHandler = Arc<dyn Fn(&mut Window, &mut App) + 'static>;

#[derive(IntoElement, RegisterComponent, Documented)]
/// A typed Lightning invoice snapshot with canvas QR, copy, and refresh intent.
pub struct LightningInvoiceDisplay {
    value: LightningInvoiceValue,
    on_refresh: Option<InvoiceRefreshHandler>,
    tokens: Option<MarketTokens>,
}

impl LightningInvoiceDisplay {
    pub fn from_source(source: &impl LightningInvoiceSource) -> Self {
        Self {
            value: source.lightning_invoice(),
            on_refresh: None,
            tokens: None,
        }
    }

    pub fn on_refresh(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_refresh = Some(Arc::new(handler));
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for LightningInvoiceDisplay {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let qr = QrCodeCanvas::encode(self.value.invoice.as_bytes())
            .map(|qr| qr.size(132.).tokens(tokens).into_any_element())
            .unwrap_or_else(|_| {
                div()
                    .size(px(132.))
                    .child("QR unavailable")
                    .into_any_element()
            });
        let invoice = self.value.invoice.clone();
        let state = self.value.payment_state;
        let refresh = self.on_refresh;
        v_flex()
            .debug_selector(|| "market.lightning_invoice".into())
            .w(px(520.))
            .p_3()
            .gap_3()
            .border_1()
            .border_color(tokens.grid)
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Label::new(self.value.kind.label()).size(LabelSize::Small))
                            .child(
                                Label::new(format!(
                                    "{} · {}",
                                    self.value.asset, self.value.network
                                ))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            ),
                    )
                    .child(
                        div()
                            .text_color(match state {
                                InvoicePaymentState::Paid => tokens.up,
                                InvoicePaymentState::Expired => tokens.down,
                                InvoicePaymentState::Pending => tokens.muted,
                            })
                            .child(state.glyph()),
                    ),
            )
            .child(
                h_flex().gap_3().child(qr).child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_2()
                        .child(
                            div()
                                .font(market_number_font(cx))
                                .text_size(px(16.))
                                .child(format_sats(self.value.amount_sats)),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .id("invoice-value")
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .font(market_number_font(cx))
                                        .text_size(px(11.))
                                        .child(invoice.clone()),
                                )
                                .child(CopyButton::new("copy-lightning-invoice", invoice)),
                        )
                        .child(
                            h_flex()
                                .justify_between()
                                .child(
                                    Label::new(time_readout_label(self.value.checked_at_ms))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .when_some(refresh, |this, handler| {
                                    this.child(
                                        Button::new("refresh-invoice-state", "Refresh").on_click(
                                            move |_, window, cx| {
                                                handler(window, cx);
                                            },
                                        ),
                                    )
                                }),
                        ),
                ),
            )
    }
}

impl Component for LightningInvoiceDisplay {
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
                "Lightning invoice",
                vec![single_example(
                    "Canvas QR and typed payment snapshot",
                    LightningInvoiceDisplay::from_source(&DemoLightningInvoiceSource)
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Payment glyph and QR remain legible without hue",
                    LightningInvoiceDisplay::from_source(&DemoLightningInvoiceSource)
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationTargetKind {
    Address,
    Invoice,
}

impl VerificationTargetKind {
    fn label(self) -> &'static str {
        match self {
            Self::Address => "address",
            Self::Invoice => "invoice",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChecksumState {
    Verified,
    Invalid,
    NotAvailable,
}

impl ChecksumState {
    fn glyph(self) -> &'static str {
        match self {
            Self::Verified => "✓",
            Self::Invalid => "×",
            Self::NotAvailable => "—",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationValue {
    pub kind: VerificationTargetKind,
    pub value: SharedString,
    pub asset: SharedString,
    pub network: SharedString,
    pub checksum: ChecksumState,
}

pub trait VerificationSource {
    fn verification(&self) -> VerificationValue;
}

pub struct DemoVerificationSource;

impl VerificationSource for DemoVerificationSource {
    fn verification(&self) -> VerificationValue {
        VerificationValue {
            kind: VerificationTargetKind::Address,
            value: "tb1p6m4u0h8w3x7d2k9q5c4v8n0j3s7f2e6a9y5r4t".into(),
            asset: "BTC".into(),
            network: "testnet".into(),
            checksum: ChecksumState::Verified,
        }
    }
}

fn truncate_middle(value: &str, visible: usize) -> String {
    let count = value.chars().count();
    if count <= visible.saturating_mul(2) {
        return value.to_owned();
    }
    let head: String = value.chars().take(visible).collect();
    let tail: String = value
        .chars()
        .rev()
        .take(visible)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// A copyable address or invoice with explicit asset, network, and checksum.
pub struct VerificationRow {
    value: VerificationValue,
    revealed: bool,
    on_reveal: Option<Arc<dyn Fn(bool, &mut Window, &mut App) + 'static>>,
    tokens: Option<MarketTokens>,
}

impl VerificationRow {
    pub fn from_source(source: &impl VerificationSource) -> Self {
        Self {
            value: source.verification(),
            revealed: false,
            on_reveal: None,
            tokens: None,
        }
    }

    pub fn revealed(mut self, revealed: bool) -> Self {
        self.revealed = revealed;
        self
    }

    pub fn on_reveal(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_reveal = Some(Arc::new(handler));
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for VerificationRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let display = if self.revealed {
            self.value.value.to_string()
        } else {
            truncate_middle(&self.value.value, 10)
        };
        h_flex()
            .debug_selector(|| "market.verification_row".into())
            .w(px(650.))
            .p_2()
            .gap_2()
            .border_1()
            .border_color(tokens.grid)
            .child(
                v_flex()
                    .w(px(120.))
                    .child(Label::new(self.value.kind.label()).size(LabelSize::XSmall))
                    .child(
                        Label::new(format!("{} · {}", self.value.asset, self.value.network))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font(market_number_font(cx))
                    .text_size(px(11.))
                    .child(display),
            )
            .child(
                div()
                    .text_color(match self.value.checksum {
                        ChecksumState::Verified => tokens.up,
                        ChecksumState::Invalid => tokens.down,
                        ChecksumState::NotAvailable => tokens.muted,
                    })
                    .child(self.value.checksum.glyph()),
            )
            .child(
                Button::new(
                    "verification-reveal",
                    if self.revealed { "Hide" } else { "Reveal" },
                )
                .when_some(self.on_reveal, |button, handler| {
                    let revealed = self.revealed;
                    button.on_click(move |_, window, cx| {
                        handler(!revealed, window, cx);
                    })
                }),
            )
            .child(CopyButton::new("verification-copy", self.value.value))
    }
}

impl Component for VerificationRow {
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
                "Verification row",
                vec![single_example(
                    "Truncated checksum-verified address with unambiguous labels",
                    VerificationRow::from_source(&DemoVerificationSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Checksum glyph preserves verification without hue",
                    VerificationRow::from_source(&DemoVerificationSource)
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferHistoryKind {
    Swap,
    Transfer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferHistoryOutcome {
    Settled,
    Refunded,
    Pending,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceiptLink {
    pub receipt_id: SharedString,
    pub provider_signed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapTransferHistoryRow {
    pub occurred_at_ms: i64,
    pub kind: TransferHistoryKind,
    pub rail: TransferRail,
    pub source_asset: SharedString,
    pub source_amount_sats: u64,
    pub destination_asset: SharedString,
    pub destination_amount_sats: u64,
    pub provider: SharedString,
    pub outcome: TransferHistoryOutcome,
    pub receipt: ReceiptLink,
}

pub trait SwapTransferHistorySource {
    fn swap_transfer_history(&self) -> Vec<SwapTransferHistoryRow>;
}

pub struct DemoSwapTransferHistorySource;

impl SwapTransferHistorySource for DemoSwapTransferHistorySource {
    fn swap_transfer_history(&self) -> Vec<SwapTransferHistoryRow> {
        (0..24)
            .map(|index| SwapTransferHistoryRow {
                occurred_at_ms: 1_786_310_400_000 - index * 180_000,
                kind: if index % 3 == 0 {
                    TransferHistoryKind::Transfer
                } else {
                    TransferHistoryKind::Swap
                },
                rail: match index % 4 {
                    0 => TransferRail::Onchain,
                    1 => TransferRail::Lightning,
                    2 => TransferRail::Liquid,
                    _ => TransferRail::Ark,
                },
                source_asset: "BTC".into(),
                source_amount_sats: 40_000 + index as u64 * 1_250,
                destination_asset: if index % 2 == 0 { "L-BTC" } else { "BTC" }.into(),
                destination_amount_sats: 39_800 + index as u64 * 1_245,
                provider: format!("provider-{:02}", index % 4).into(),
                outcome: match index % 5 {
                    0 => TransferHistoryOutcome::Pending,
                    1 => TransferHistoryOutcome::Refunded,
                    _ => TransferHistoryOutcome::Settled,
                },
                receipt: ReceiptLink {
                    receipt_id: format!("mkt-receipt-{index:04}").into(),
                    provider_signed: index % 5 != 0,
                },
            })
            .collect()
    }
}

type ReceiptOpenHandler = Arc<dyn Fn(&ReceiptLink, &mut Window, &mut App) + 'static>;

#[derive(IntoElement, RegisterComponent, Documented)]
/// A virtualized swap and transfer history with provider-signed receipt links.
pub struct SwapTransferHistoryTable {
    rows: Vec<SwapTransferHistoryRow>,
    on_open_receipt: Option<ReceiptOpenHandler>,
    tokens: Option<MarketTokens>,
}

impl SwapTransferHistoryTable {
    pub fn from_source(source: &impl SwapTransferHistorySource) -> Self {
        Self {
            rows: source.swap_transfer_history(),
            on_open_receipt: None,
            tokens: None,
        }
    }

    pub fn on_open_receipt(
        mut self,
        handler: impl Fn(&ReceiptLink, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_receipt = Some(Arc::new(handler));
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for SwapTransferHistoryTable {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let rows = Arc::new(self.rows);
        let count = rows.len();
        let handler = self.on_open_receipt;
        v_flex()
            .debug_selector(|| "market.swap_transfer_history".into())
            .w(px(900.))
            .h(px(320.))
            .gap_2()
            .child(Label::new("Swap & transfer history").size(LabelSize::Small))
            .child(
                Table::new(8)
                    .width(px(900.))
                    .header(vec![
                        "Time", "Kind", "Rail", "Sent", "Received", "Provider", "Outcome",
                        "Receipt",
                    ])
                    .uniform_list("market-swap-history-rows", count, move |range, _, cx| {
                        range
                            .filter_map(|index| rows.get(index))
                            .map(|row| {
                                let receipt = row.receipt.clone();
                                let receipt_handler = handler.clone();
                                let number = |value: String| {
                                    div()
                                        .font(market_number_font(cx))
                                        .text_size(px(11.))
                                        .child(value)
                                        .into_any_element()
                                };
                                let outcome = match row.outcome {
                                    TransferHistoryOutcome::Settled => "✓",
                                    TransferHistoryOutcome::Refunded => "↩",
                                    TransferHistoryOutcome::Pending => "◷",
                                };
                                vec![
                                    number(time_readout_label(row.occurred_at_ms)),
                                    Chip::new(match row.kind {
                                        TransferHistoryKind::Swap => "swap",
                                        TransferHistoryKind::Transfer => "transfer",
                                    })
                                    .into_any_element(),
                                    Label::new(format!(
                                        "{} {}",
                                        row.rail.glyph(),
                                        row.rail.label()
                                    ))
                                    .size(LabelSize::XSmall)
                                    .into_any_element(),
                                    number(format!(
                                        "{} {}",
                                        format_sats(row.source_amount_sats),
                                        row.source_asset
                                    )),
                                    number(format!(
                                        "{} {}",
                                        format_sats(row.destination_amount_sats),
                                        row.destination_asset
                                    )),
                                    Label::new(row.provider.clone())
                                        .size(LabelSize::XSmall)
                                        .into_any_element(),
                                    div()
                                        .text_color(match row.outcome {
                                            TransferHistoryOutcome::Settled => tokens.up,
                                            TransferHistoryOutcome::Refunded => tokens.down,
                                            TransferHistoryOutcome::Pending => tokens.muted,
                                        })
                                        .child(outcome)
                                        .into_any_element(),
                                    h_flex()
                                        .id(receipt.receipt_id.clone())
                                        .gap_1()
                                        .cursor_pointer()
                                        .child(if receipt.provider_signed {
                                            "✓"
                                        } else {
                                            "—"
                                        })
                                        .child(receipt.receipt_id.clone())
                                        .when_some(receipt_handler, move |this, handler| {
                                            this.on_click(move |_, window, cx| {
                                                handler(&receipt, window, cx);
                                            })
                                        })
                                        .into_any_element(),
                                ]
                            })
                            .collect()
                    }),
            )
    }
}

impl Component for SwapTransferHistoryTable {
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
                "Swap and transfer history",
                vec![single_example(
                    "Virtualized rows link to provider-signed receipt IDs",
                    SwapTransferHistoryTable::from_source(&DemoSwapTransferHistorySource)
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Outcome and receipt verification retain structural glyphs",
                    SwapTransferHistoryTable::from_source(&DemoSwapTransferHistorySource)
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
    fn rail_fixture_covers_every_required_rail() {
        let value = DemoRailSelectorSource.rail_selector();
        assert_eq!(value.choices.len(), 4);
        assert!(value.choices.iter().any(|choice| !choice.available));
    }

    #[test]
    fn invoice_snapshot_is_typed_and_network_labeled() {
        let value = DemoLightningInvoiceSource.lightning_invoice();
        assert_eq!(value.kind, LightningInvoiceKind::Bolt11);
        assert_eq!(value.network.as_ref(), "testnet");
        assert!(value.checked_at_ms > 0);
    }

    #[test]
    fn verification_truncation_preserves_both_ends() {
        assert_eq!(truncate_middle("abcdefghijklmnop", 4), "abcd…mnop");
        assert_eq!(truncate_middle("short", 4), "short");
    }

    #[test]
    fn every_history_row_links_a_receipt() {
        let rows = DemoSwapTransferHistorySource.swap_transfer_history();
        assert_eq!(rows.len(), 24);
        assert!(
            rows.iter()
                .all(|row| row.receipt.receipt_id.starts_with("mkt-receipt-"))
        );
    }
}
