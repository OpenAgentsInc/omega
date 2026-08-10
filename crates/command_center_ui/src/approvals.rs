use std::sync::Arc;

use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, FontWeight, SharedString, Window, uniform_list};
use trading_mandate::{
    MandateChangeClass, MandateProposal, ReviewCadence, TradingMandate, TradingNetwork,
};
use ui::prelude::*;
use ui::{MarketTokens, market_number_font};

use crate::format::format_wall_clock;
use crate::mandate_status_card::demo_mandate;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingApprovalKind {
    MandateWidening {
        venue: SharedString,
        digest: SharedString,
    },
    OrderConfirmation {
        instrument: SharedString,
        request_id: SharedString,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingApproval {
    pub approval_id: SharedString,
    pub requested_at_ms: i64,
    pub summary: SharedString,
    pub kind: PendingApprovalKind,
}

pub trait ApprovalQueueSource {
    fn pending_approvals(&self) -> Vec<PendingApproval>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalQueueAction {
    Review { approval_id: SharedString },
    Reject { approval_id: SharedString },
}

#[derive(IntoElement, RegisterComponent)]
pub struct ApprovalQueue {
    approvals: Vec<PendingApproval>,
    tokens: Option<MarketTokens>,
    on_action: Option<Arc<dyn Fn(ApprovalQueueAction, &mut Window, &mut App) + 'static>>,
}

impl ApprovalQueue {
    pub fn from_source(source: &impl ApprovalQueueSource) -> Self {
        Self::new(source.pending_approvals())
    }

    pub fn new(approvals: Vec<PendingApproval>) -> Self {
        Self {
            approvals,
            tokens: None,
            on_action: None,
        }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }

    pub fn on_action(
        mut self,
        handler: impl Fn(ApprovalQueueAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Arc::new(handler));
        self
    }
}

impl RenderOnce for ApprovalQueue {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let approvals = Arc::new(self.approvals);
        let count = approvals.len();
        let on_action = self.on_action;
        v_flex()
            .debug_selector(|| "command_center.approval_queue".into())
            .w(px(680.))
            .h(px(280.))
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(tokens.grid)
            .bg(tokens.surface)
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("Approvals").size(LabelSize::Small))
                    .child(
                        div()
                            .font(market_number_font(cx))
                            .text_size(px(11.))
                            .text_color(tokens.muted)
                            .child(count.to_string()),
                    ),
            )
            .when(count == 0, |this| {
                this.child(
                    Label::new("Queue clear")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
            })
            .when(count > 0, |this| {
                this.child(uniform_list(
                    "command-center-approval-rows",
                    count,
                    move |range, _window, _cx| {
                        range
                            .filter_map(|row_index| {
                                approvals
                                    .get(row_index)
                                    .map(|approval| (row_index, approval))
                            })
                            .map(|(row_index, approval)| {
                                let (icon, scope) = match &approval.kind {
                                    PendingApprovalKind::MandateWidening { venue, .. } => {
                                        (IconName::Lock, venue.clone())
                                    }
                                    PendingApprovalKind::OrderConfirmation {
                                        instrument, ..
                                    } => (IconName::ArrowRightLeft, instrument.clone()),
                                };
                                let review_action = ApprovalQueueAction::Review {
                                    approval_id: approval.approval_id.clone(),
                                };
                                let reject_action = ApprovalQueueAction::Reject {
                                    approval_id: approval.approval_id.clone(),
                                };
                                let review_handler = on_action.clone();
                                let reject_handler = on_action.clone();
                                h_flex()
                                    .w_full()
                                    .px_2()
                                    .py_2()
                                    .gap_2()
                                    .border_b_1()
                                    .border_color(tokens.grid)
                                    .child(
                                        Icon::new(icon)
                                            .size(IconSize::XSmall)
                                            .color(Color::Warning),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .gap_0p5()
                                            .child(
                                                Label::new(approval.summary.clone())
                                                    .size(LabelSize::Small),
                                            )
                                            .child(
                                                Label::new(format!(
                                                    "{} · {}",
                                                    scope,
                                                    format_wall_clock(approval.requested_at_ms)
                                                ))
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                            ),
                                    )
                                    .child(
                                        Button::new(("approval-review", row_index), "Review")
                                            .when_some(review_handler, |button, handler| {
                                                button.on_click(move |_, window, cx| {
                                                    handler(review_action.clone(), window, cx);
                                                })
                                            }),
                                    )
                                    .child(
                                        Button::new(("approval-reject", row_index), "Reject")
                                            .when_some(reject_handler, |button, handler| {
                                                button.on_click(move |_, window, cx| {
                                                    handler(reject_action.clone(), window, cx);
                                                })
                                            }),
                                    )
                            })
                            .collect()
                    },
                ))
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitChangeDirection {
    Wider,
    Narrower,
    Same,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MandateDiffRow {
    pub label: SharedString,
    pub previous: SharedString,
    pub candidate: SharedString,
    pub direction: LimitChangeDirection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MandateEditorValue {
    pub base_revision: u64,
    pub current: Option<TradingMandate>,
    pub candidate: TradingMandate,
    pub change_class: MandateChangeClass,
    pub approval_digest: SharedString,
}

impl MandateEditorValue {
    pub fn from_proposal(current: Option<&TradingMandate>, proposal: &MandateProposal) -> Self {
        Self {
            base_revision: proposal.base_revision(),
            current: current.cloned(),
            candidate: proposal.candidate().clone(),
            change_class: proposal.change_class(),
            approval_digest: proposal.digest().to_owned().into(),
        }
    }

    pub fn validation_error(&self) -> Option<SharedString> {
        self.candidate
            .validate()
            .err()
            .map(|error| error.to_string().into())
    }

    pub fn diff_rows(&self) -> Vec<MandateDiffRow> {
        let Some(current) = &self.current else {
            return vec![MandateDiffRow {
                label: "Authority".into(),
                previous: "none".into(),
                candidate: "new mandate".into(),
                direction: LimitChangeDirection::Wider,
            }];
        };
        let asset = self.candidate.collateral_asset.to_string();
        let rows = [
            numeric_diff(
                "Venue balance",
                current.max_venue_balance,
                self.candidate.max_venue_balance,
                &asset,
                false,
            ),
            numeric_diff(
                "Position",
                current.max_position_usd,
                self.candidate.max_position_usd,
                "USD",
                false,
            ),
            numeric_diff(
                "Leverage",
                u64::from(current.max_leverage),
                u64::from(self.candidate.max_leverage),
                "×",
                false,
            ),
            numeric_diff(
                "Daily loss",
                current.daily_loss_stop,
                self.candidate.daily_loss_stop,
                &asset,
                false,
            ),
            numeric_diff(
                "Orders",
                u64::from(current.max_orders_per_hour),
                u64::from(self.candidate.max_orders_per_hour),
                "/h",
                false,
            ),
            numeric_diff(
                "Liquidation buffer",
                u64::from(current.min_liquidation_buffer_bps),
                u64::from(self.candidate.min_liquidation_buffer_bps),
                "bps",
                true,
            ),
            numeric_diff(
                "Expiry",
                u64::try_from(current.expires_at_ms).unwrap_or(0),
                u64::try_from(self.candidate.expires_at_ms).unwrap_or(0),
                "ms",
                false,
            ),
        ];
        rows.into_iter().collect()
    }
}

pub trait MandateEditorSource {
    fn mandate_editor(&self) -> MandateEditorValue;
}

fn numeric_diff(
    label: &'static str,
    previous: u64,
    candidate: u64,
    unit: &str,
    lower_widens: bool,
) -> MandateDiffRow {
    let ordering = candidate.cmp(&previous);
    let direction = if ordering.is_eq() {
        LimitChangeDirection::Same
    } else if (ordering.is_gt() && !lower_widens) || (ordering.is_lt() && lower_widens) {
        LimitChangeDirection::Wider
    } else {
        LimitChangeDirection::Narrower
    };
    MandateDiffRow {
        label: label.into(),
        previous: format!("{previous} {unit}").into(),
        candidate: format!("{candidate} {unit}").into(),
        direction,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MandateEditorAction {
    SaveRestriction {
        base_revision: u64,
        approval_digest: SharedString,
    },
    ReviewApproval {
        base_revision: u64,
        approval_digest: SharedString,
    },
    Approve {
        base_revision: u64,
        approval_digest: SharedString,
    },
    Reject {
        base_revision: u64,
        approval_digest: SharedString,
    },
}

type MandateActionHandler = Arc<dyn Fn(MandateEditorAction, &mut Window, &mut App) + 'static>;

#[derive(IntoElement, RegisterComponent)]
pub struct MandateEditor {
    value: MandateEditorValue,
    tokens: Option<MarketTokens>,
    on_action: Option<MandateActionHandler>,
}

impl MandateEditor {
    pub fn from_source(source: &impl MandateEditorSource) -> Self {
        Self::new(source.mandate_editor())
    }

    pub fn new(value: MandateEditorValue) -> Self {
        Self {
            value,
            tokens: None,
            on_action: None,
        }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }

    pub fn on_action(
        mut self,
        handler: impl Fn(MandateEditorAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Arc::new(handler));
        self
    }
}

fn change_visual(
    direction: LimitChangeDirection,
    tokens: MarketTokens,
) -> (&'static str, gpui::Hsla) {
    match direction {
        LimitChangeDirection::Wider => ("+", tokens.down),
        LimitChangeDirection::Narrower => ("−", tokens.up),
        LimitChangeDirection::Same => ("=", tokens.muted),
    }
}

fn diff_row(row: MandateDiffRow, tokens: MarketTokens, cx: &App) -> AnyElement {
    let (glyph, color) = change_visual(row.direction, tokens);
    h_flex()
        .w_full()
        .gap_3()
        .child(
            Label::new(row.label)
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .child(
            div()
                .flex_1()
                .text_right()
                .font(market_number_font(cx))
                .text_size(px(11.))
                .text_color(tokens.muted)
                .child(row.previous),
        )
        .child(div().w_4().text_center().text_color(color).child(glyph))
        .child(
            div()
                .w_32()
                .font(market_number_font(cx))
                .text_size(px(11.))
                .text_color(tokens.text)
                .child(row.candidate),
        )
        .into_any_element()
}

fn editor_body(value: &MandateEditorValue, tokens: MarketTokens, cx: &App) -> AnyElement {
    let network = match value.candidate.network {
        TradingNetwork::Signet => "signet",
        TradingNetwork::Testnet => "testnet",
        TradingNetwork::Mainnet => "mainnet",
    };
    let cadence = match value.candidate.review_cadence {
        ReviewCadence::FundingSettlement => "funding settlement".to_owned(),
        ReviewCadence::Interval { seconds } => format!("{seconds}s"),
    };
    let strategies = value
        .candidate
        .allowed_strategies
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" · ");
    v_flex()
        .gap_2()
        .child(
            Label::new(format!(
                "{} · {} · {}",
                value.candidate.venue, value.candidate.collateral_asset, network
            ))
            .size(LabelSize::XSmall)
            .color(Color::Muted),
        )
        .children(
            value
                .diff_rows()
                .into_iter()
                .map(|row| diff_row(row, tokens, cx)),
        )
        .child(
            h_flex()
                .justify_between()
                .child(
                    Label::new("Review")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(Label::new(cadence).size(LabelSize::XSmall)),
        )
        .child(
            h_flex()
                .justify_between()
                .child(
                    Label::new("Strategies")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(Label::new(strategies).size(LabelSize::XSmall)),
        )
        .child(
            h_flex()
                .justify_between()
                .child(
                    Label::new("Expires")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new(format_wall_clock(value.candidate.expires_at_ms))
                        .size(LabelSize::XSmall),
                ),
        )
        .into_any_element()
}

impl RenderOnce for MandateEditor {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let validation_error = self.value.validation_error();
        let (button_label, action) = match self.value.change_class {
            MandateChangeClass::Creation | MandateChangeClass::Widening => (
                "Review approval",
                MandateEditorAction::ReviewApproval {
                    base_revision: self.value.base_revision,
                    approval_digest: self.value.approval_digest.clone(),
                },
            ),
            MandateChangeClass::Restriction => (
                "Save restriction",
                MandateEditorAction::SaveRestriction {
                    base_revision: self.value.base_revision,
                    approval_digest: self.value.approval_digest.clone(),
                },
            ),
            MandateChangeClass::Unchanged => (
                "No changes",
                MandateEditorAction::SaveRestriction {
                    base_revision: self.value.base_revision,
                    approval_digest: self.value.approval_digest.clone(),
                },
            ),
        };
        let enabled =
            validation_error.is_none() && self.value.change_class != MandateChangeClass::Unchanged;
        let handler = self.on_action;
        v_flex()
            .debug_selector(|| "command_center.mandate_editor".into())
            .w(px(620.))
            .gap_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(tokens.grid)
            .bg(tokens.surface)
            .child(
                Label::new("Mandate")
                    .size(LabelSize::Small)
                    .weight(FontWeight::SEMIBOLD),
            )
            .child(editor_body(&self.value, tokens, cx))
            .when_some(validation_error, |this, error| {
                this.child(
                    Label::new(error)
                        .size(LabelSize::XSmall)
                        .color(Color::Error),
                )
            })
            .child(
                h_flex().justify_end().child(
                    Button::new("mandate-editor-submit", button_label)
                        .style(ButtonStyle::Filled)
                        .disabled(!enabled)
                        .when_some(handler, |button, handler| {
                            button.on_click(move |_, window, cx| {
                                handler(action.clone(), window, cx);
                            })
                        }),
                ),
            )
    }
}

#[derive(IntoElement, RegisterComponent)]
pub struct MandateApprovalDialog {
    value: MandateEditorValue,
    tokens: Option<MarketTokens>,
    on_action: Option<MandateActionHandler>,
}

impl MandateApprovalDialog {
    pub fn new(value: MandateEditorValue) -> Self {
        Self {
            value,
            tokens: None,
            on_action: None,
        }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }

    pub fn on_action(
        mut self,
        handler: impl Fn(MandateEditorAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Arc::new(handler));
        self
    }
}

impl RenderOnce for MandateApprovalDialog {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let approve = MandateEditorAction::Approve {
            base_revision: self.value.base_revision,
            approval_digest: self.value.approval_digest.clone(),
        };
        let reject = MandateEditorAction::Reject {
            base_revision: self.value.base_revision,
            approval_digest: self.value.approval_digest.clone(),
        };
        let approve_handler = self.on_action.clone();
        let reject_handler = self.on_action;
        v_flex()
            .debug_selector(|| "command_center.mandate_approval".into())
            .w(px(620.))
            .gap_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(tokens.down.opacity(0.7))
            .bg(tokens.surface)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Icon::new(IconName::Lock)
                            .size(IconSize::Small)
                            .color(Color::Warning),
                    )
                    .child(
                        Label::new("Approve wider authority")
                            .size(LabelSize::Small)
                            .weight(FontWeight::SEMIBOLD),
                    ),
            )
            .child(editor_body(&self.value, tokens, cx))
            .child(
                div()
                    .font(market_number_font(cx))
                    .text_size(px(10.))
                    .text_color(tokens.muted)
                    .child(format!(
                        "revision {} · {}",
                        self.value.base_revision, self.value.approval_digest
                    )),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("mandate-approval-reject", "Reject").when_some(
                        reject_handler,
                        |button, handler| {
                            button.on_click(move |_, window, cx| {
                                handler(reject.clone(), window, cx);
                            })
                        },
                    ))
                    .child(
                        Button::new("mandate-approval-approve", "Approve")
                            .style(ButtonStyle::Filled)
                            .when_some(approve_handler, |button, handler| {
                                button.on_click(move |_, window, cx| {
                                    handler(approve.clone(), window, cx);
                                })
                            }),
                    ),
            )
    }
}

fn demo_approvals() -> Vec<PendingApproval> {
    vec![
        PendingApproval {
            approval_id: "approval-mandate-4".into(),
            requested_at_ms: 1_786_276_800_000,
            summary: "Position cap 500 → 750 USD".into(),
            kind: PendingApprovalKind::MandateWidening {
                venue: "hyperliquid · testnet".into(),
                digest: "8f9a…12c4".into(),
            },
        },
        PendingApproval {
            approval_id: "approval-order-8".into(),
            requested_at_ms: 1_786_276_860_000,
            summary: "BUY 0.080 BTC-PERP · LIMIT".into(),
            kind: PendingApprovalKind::OrderConfirmation {
                instrument: "BTC-PERP".into(),
                request_id: "order-8".into(),
            },
        },
    ]
}

fn demo_editor() -> MandateEditorValue {
    let current = demo_mandate();
    let mut candidate = current.clone();
    candidate.max_position_usd = 750;
    candidate.max_orders_per_hour = 8;
    MandateEditorValue {
        base_revision: 4,
        current: Some(current),
        candidate,
        change_class: MandateChangeClass::Widening,
        approval_digest: "8f9a4eead31812c4".into(),
    }
}

impl Component for ApprovalQueue {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        "Unified queue for mandate widening and exact order approvals."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let grayscale = MarketTokens::from_theme(cx).grayscale();
        example_group_with_title(
            "Approval queue",
            vec![
                single_example(
                    "Normal",
                    ApprovalQueue::new(demo_approvals()).into_any_element(),
                ),
                single_example(
                    "Grayscale audit",
                    ApprovalQueue::new(demo_approvals())
                        .tokens(grayscale)
                        .into_any_element(),
                ),
            ],
        )
        .vertical()
        .into_any_element()
    }
}

impl Component for MandateEditor {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        "Typed mandate limits, units, strategy scope, expiry, validation, and widening diff."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let grayscale = MarketTokens::from_theme(cx).grayscale();
        example_group_with_title(
            "Mandate editor",
            vec![
                single_example(
                    "Normal",
                    MandateEditor::new(demo_editor()).into_any_element(),
                ),
                single_example(
                    "Grayscale audit",
                    MandateEditor::new(demo_editor())
                        .tokens(grayscale)
                        .into_any_element(),
                ),
            ],
        )
        .vertical()
        .into_any_element()
    }
}

impl Component for MandateApprovalDialog {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        "Exact revision-bound approval ceremony for mandate creation or widening."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let grayscale = MarketTokens::from_theme(cx).grayscale();
        example_group_with_title(
            "Mandate approval",
            vec![
                single_example(
                    "Normal",
                    MandateApprovalDialog::new(demo_editor()).into_any_element(),
                ),
                single_example(
                    "Grayscale audit",
                    MandateApprovalDialog::new(demo_editor())
                        .tokens(grayscale)
                        .into_any_element(),
                ),
            ],
        )
        .vertical()
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reversed_floor_marks_a_lower_buffer_as_wider() {
        let row = numeric_diff("buffer", 1_500, 1_000, "bps", true);
        assert_eq!(row.direction, LimitChangeDirection::Wider);
    }

    #[test]
    fn demo_mixes_widening_and_narrowing_without_hiding_ceremony() {
        let value = demo_editor();
        let rows = value.diff_rows();
        assert!(
            rows.iter()
                .any(|row| row.direction == LimitChangeDirection::Wider)
        );
        assert!(
            rows.iter()
                .any(|row| row.direction == LimitChangeDirection::Narrower)
        );
        assert!(value.change_class.needs_ui_approval());
    }
}
