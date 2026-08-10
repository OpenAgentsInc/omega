use std::sync::Arc;

use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, SharedString, Window, px};
use ui::{MarketTokens, Table, VizChip, VizChipTone, prelude::*};

use crate::format::format_wall_clock;
use crate::portfolio_accounting::{format_asset_amount, number_cell, text_cell};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReceiptVerificationState {
    ProviderSigned,
    Incomplete { reason: SharedString },
    Invalid { reason: SharedString },
}

impl ReceiptVerificationState {
    fn label(&self) -> &'static str {
        match self {
            Self::ProviderSigned => "✓ provider-signed",
            Self::Incomplete { .. } => "… incomplete",
            Self::Invalid { .. } => "! invalid",
        }
    }

    fn tone(&self) -> VizChipTone {
        match self {
            Self::ProviderSigned => VizChipTone::Ok,
            Self::Incomplete { .. } | Self::Invalid { .. } => VizChipTone::Warn,
        }
    }

    fn reason(&self) -> Option<SharedString> {
        match self {
            Self::ProviderSigned => None,
            Self::Incomplete { reason } | Self::Invalid { reason } => Some(reason.clone()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceiptLegView {
    pub rail: SharedString,
    pub asset: SharedString,
    pub amount: i64,
    pub event_id: SharedString,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceiptFeeView {
    pub label: SharedString,
    pub asset: SharedString,
    pub amount: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VenueRecordLink {
    pub venue: SharedString,
    pub record_id: SharedString,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceiptViewData {
    pub receipt_id: SharedString,
    pub protocol: SharedString,
    pub provider: SharedString,
    pub outcome: SharedString,
    pub created_at_ms: i64,
    pub verification: ReceiptVerificationState,
    pub legs: Vec<ReceiptLegView>,
    pub fees: Vec<ReceiptFeeView>,
    pub venue_records: Vec<VenueRecordLink>,
}

type VenueRecordHandler = Arc<dyn Fn(&VenueRecordLink, &mut Window, &mut App) + 'static>;

#[derive(IntoElement, RegisterComponent)]
pub struct ReceiptViewer {
    data: ReceiptViewData,
    on_open_venue_record: Option<VenueRecordHandler>,
    tokens: Option<MarketTokens>,
}

impl ReceiptViewer {
    pub fn new(data: ReceiptViewData) -> Self {
        Self {
            data,
            on_open_venue_record: None,
            tokens: None,
        }
    }

    pub fn on_open_venue_record(
        mut self,
        handler: impl Fn(&VenueRecordLink, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_venue_record = Some(Arc::new(handler));
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for ReceiptViewer {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let mut legs = Table::new(4).header(
            ["Rail", "Asset", "Amount", "Signed event"]
                .into_iter()
                .map(text_cell)
                .collect(),
        );
        for leg in self.data.legs {
            legs = legs.row(vec![
                text_cell(leg.rail),
                text_cell(leg.asset.clone()),
                number_cell(
                    format_asset_amount(leg.amount, leg.asset.as_ref()),
                    tokens.text,
                    cx,
                ),
                number_cell(leg.event_id, tokens.muted, cx),
            ]);
        }
        let mut fees = Table::new(3).header(
            ["Fee", "Asset", "Amount"]
                .into_iter()
                .map(text_cell)
                .collect(),
        );
        for fee in self.data.fees {
            fees = fees.row(vec![
                text_cell(fee.label),
                text_cell(fee.asset.clone()),
                number_cell(
                    format_asset_amount(fee.amount, fee.asset.as_ref()),
                    tokens.down,
                    cx,
                ),
            ]);
        }
        let handler = self.on_open_venue_record;
        let has_records = !self.data.venue_records.is_empty();
        let records = self
            .data
            .venue_records
            .into_iter()
            .enumerate()
            .map(|(index, record)| {
                let callback_record = record.clone();
                let handler = handler.clone();
                Button::new(
                    ("receipt-venue-record", index),
                    format!("{} · {}", record.venue, record.record_id),
                )
                .label_size(LabelSize::Small)
                .when_some(handler, move |button, handler| {
                    button.on_click(move |_, window, cx| handler(&callback_record, window, cx))
                })
            });
        let reason = self.data.verification.reason();
        let verified = matches!(
            self.data.verification,
            ReceiptVerificationState::ProviderSigned
        );
        v_flex()
            .debug_selector(|| "command_center.receipt_viewer".into())
            .w_full()
            .max_w(px(720.0))
            .border_1()
            .border_color(tokens.grid)
            .bg(tokens.surface)
            .child(
                v_flex()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(tokens.grid)
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .child(
                                Label::new(self.data.receipt_id)
                                    .size(LabelSize::Small)
                                    .color(Color::Default),
                            )
                            .child(
                                VizChip::new(self.data.verification.label())
                                    .kind(39_613)
                                    .tone(self.data.verification.tone())
                                    .scale(1.0),
                            ),
                    )
                    .child(
                        Label::new(format!(
                            "{} · {} · {} · {}",
                            self.data.protocol,
                            self.data.provider,
                            self.data.outcome,
                            format_wall_clock(self.data.created_at_ms)
                        ))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    )
                    .when(verified, |this| {
                        this.child(
                            Label::new("External settlement not independently proven")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                    })
                    .when_some(reason, |this, reason| {
                        this.child(
                            Label::new(reason)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                    }),
            )
            .child(v_flex().gap_1().px_3().py_2().child(legs).child(fees))
            .when(has_records, |this| {
                this.child(
                    h_flex()
                        .px_3()
                        .py_2()
                        .gap_1()
                        .flex_wrap()
                        .border_t_1()
                        .border_color(tokens.grid)
                        .children(records),
                )
            })
    }
}

fn demo_receipt(verification: ReceiptVerificationState) -> ReceiptViewData {
    ReceiptViewData {
        receipt_id: "mkt-receipt-4f1a…9c20".into(),
        protocol: "mkt-swp/2".into(),
        provider: "npub1provider…8n2".into(),
        outcome: "settled".into(),
        created_at_ms: 1_754_700_000_000,
        verification,
        legs: vec![
            ReceiptLegView {
                rail: "lightning".into(),
                asset: "sats".into(),
                amount: 250_000,
                event_id: "event:8bd3…64f0".into(),
            },
            ReceiptLegView {
                rail: "liquid".into(),
                asset: "USDt".into(),
                amount: 16_420,
                event_id: "event:70a1…d833".into(),
            },
        ],
        fees: vec![ReceiptFeeView {
            label: "provider".into(),
            asset: "sats".into(),
            amount: 420,
        }],
        venue_records: vec![
            VenueRecordLink {
                venue: "lightning".into(),
                record_id: "payment:66aa…1d".into(),
            },
            VenueRecordLink {
                venue: "liquid".into(),
                record_id: "txid:120b…f7".into(),
            },
        ],
    }
}

impl Component for ReceiptViewer {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        "Typed receipt legs, fees, proof status, and links to the underlying venue records."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Receipt",
                vec![
                    single_example(
                        "Provider-signed settlement evidence",
                        ReceiptViewer::new(demo_receipt(ReceiptVerificationState::ProviderSigned))
                            .into_any_element(),
                    ),
                    single_example(
                        "Incomplete proof chain",
                        ReceiptViewer::new(demo_receipt(ReceiptVerificationState::Incomplete {
                            reason: "requester confirmation missing".into(),
                        }))
                        .into_any_element(),
                    ),
                ],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Glyphs and labels preserve proof state",
                    ReceiptViewer::new(demo_receipt(ReceiptVerificationState::Invalid {
                        reason: "provider rotation chain mismatch".into(),
                    }))
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
    fn receipt_fixture_keeps_signed_events_and_venue_records() {
        let receipt = demo_receipt(ReceiptVerificationState::ProviderSigned);
        assert!(receipt.legs.iter().all(|leg| !leg.event_id.is_empty()));
        assert_eq!(receipt.venue_records.len(), 2);
        assert_eq!(receipt.verification.label(), "✓ provider-signed");
    }

    #[test]
    fn invalid_receipt_carries_a_structural_reason() {
        let verification = ReceiptVerificationState::Invalid {
            reason: "hash mismatch".into(),
        };
        assert_eq!(verification.reason().as_deref(), Some("hash mismatch"));
        assert_eq!(verification.label(), "! invalid");
    }
}
