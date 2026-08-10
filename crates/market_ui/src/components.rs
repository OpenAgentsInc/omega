//! GPUI renderers for the versioned requester market view.

use gpui::{AnyElement, App, ElementId, IntoElement, RenderOnce, Window};
use ui::{Callout, Icon, IconName, IconSize, Severity, Tooltip, VizChip, VizChipTone, prelude::*};

use crate::view_model::{
    CustodyView, EvidenceRung, ExitPackageView, FeeBreakdownView, MarketSessionViewModel,
    OfferingView, PriceFeedView, ProviderView, QuoteView, ReceiptView, ReservationProofClass,
    ReservationView, TimelineLaneView, TimelineSlotState, TypedErrorView, VerifyChecklistView,
    VerifyState, short_id,
};

fn surface(cx: &App) -> gpui::Div {
    v_flex()
        .w_full()
        .gap_1p5()
        .p_2()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().colors().border)
}

#[derive(Clone, Copy)]
enum MarketStatusCue {
    Active,
    Verified,
    Warning,
    Missing,
    Expired,
}

impl MarketStatusCue {
    fn word(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Verified => "Verified",
            Self::Warning => "Warning",
            Self::Missing => "Missing",
            Self::Expired => "Expired",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Active => IconName::LoadCircle,
            Self::Verified => IconName::Check,
            Self::Warning => IconName::Warning,
            Self::Missing => IconName::Slash,
            Self::Expired => IconName::XCircle,
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Active => Color::Accent,
            Self::Verified => Color::Success,
            Self::Warning | Self::Expired => Color::Warning,
            Self::Missing => Color::Error,
        }
    }
}

fn market_status_cue(
    id: impl Into<ElementId>,
    status: MarketStatusCue,
    context: impl Into<String>,
) -> AnyElement {
    let word = status.word();
    h_flex()
        .id(id)
        .role(gpui::Role::Status)
        .aria_label(format!("{}: {word}", context.into()))
        .tooltip(Tooltip::text(word))
        .child(
            Icon::new(status.icon())
                .size(IconSize::XSmall)
                .color(status.color()),
        )
        .into_any_element()
}

#[derive(IntoElement)]
pub struct ProviderBadge {
    view: ProviderView,
}

impl ProviderBadge {
    pub fn new(view: ProviderView) -> Self {
        Self { view }
    }
}

impl RenderOnce for ProviderBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let status = if self.view.status == "active" {
            MarketStatusCue::Active
        } else if self.view.status.contains("signed") || self.view.status == "fixture" {
            MarketStatusCue::Verified
        } else {
            MarketStatusCue::Warning
        };
        let provider_id = self.view.provider_id.clone();
        h_flex()
            .gap_1p5()
            .items_center()
            .flex_wrap()
            .child(Label::new(self.view.display_name))
            .child(market_status_cue(
                format!("market-provider-status-{provider_id}"),
                status,
                "Provider",
            ))
            .children(
                self.view
                    .profiles
                    .into_iter()
                    .map(|profile| VizChip::new(profile).tone(VizChipTone::Active)),
            )
            .children(self.view.assertions.into_iter().map(|assertion| {
                Label::new(format!(
                    "claim: {} · asserter {}",
                    assertion.assertion,
                    short_id(&assertion.asserter)
                ))
                .size(LabelSize::XSmall)
                .color(Color::Muted)
            }))
            .child(
                Label::new(short_id(&provider_id))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .map(|row| {
                surface(cx)
                    .id(format!("market-provider-{provider_id}"))
                    .role(gpui::Role::Group)
                    .aria_label(format!("Provider {}", short_id(&provider_id)))
                    .child(row)
            })
    }
}

#[derive(IntoElement)]
pub struct OfferingCard {
    view: OfferingView,
    now: u64,
}

impl OfferingCard {
    pub fn new(view: OfferingView, now: u64) -> Self {
        Self { view, now }
    }
}

impl RenderOnce for OfferingCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let freshness = self.now.saturating_sub(self.view.published_at);
        surface(cx)
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(Label::new(self.view.offering_id))
                    .child(
                        VizChip::new(format!("{} v{}", self.view.profile, self.view.version))
                            .tone(VizChipTone::Active),
                    ),
            )
            .children(self.view.sides.into_iter().map(|side| {
                v_flex()
                    .gap_0p5()
                    .child(Label::new(format!(
                        "{} → {} · {}",
                        side.input.display_ticker, side.output.display_ticker, side.direction
                    )))
                    .child(
                        Label::new(format!(
                            "{} .. {} atomic units",
                            side.minimum_amount, side.maximum_amount
                        ))
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                    )
                    .child(
                        Label::new(format!(
                            "{} → {}",
                            side.input.canonical_id, side.output.canonical_id
                        ))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    )
            }))
            .child(
                Label::new(format!(
                    "provider {} · signed head age {}s",
                    short_id(&self.view.provider_id),
                    freshness
                ))
                .size(LabelSize::XSmall)
                .color(Color::Muted),
            )
    }
}

#[derive(IntoElement)]
pub struct ReservationBadge {
    view: ReservationView,
}

impl ReservationBadge {
    pub fn new(view: ReservationView) -> Self {
        Self { view }
    }
}

impl RenderOnce for ReservationBadge {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tone = match self.view.proof_class {
            ReservationProofClass::None => VizChipTone::Neutral,
            ReservationProofClass::ProviderSigned => VizChipTone::Active,
            ReservationProofClass::CovenantReserve => VizChipTone::Ok,
            ReservationProofClass::Other => VizChipTone::Warn,
        };
        h_flex()
            .gap_1()
            .child(VizChip::new(self.view.class).tone(tone))
            .child(
                Label::new(self.view.proof_class.label())
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }
}

#[derive(IntoElement)]
pub struct CustodyStrip {
    view: CustodyView,
}

impl CustodyStrip {
    pub fn new(view: CustodyView) -> Self {
        Self { view }
    }
}

impl RenderOnce for CustodyStrip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut duration = format!(
            "maximum custody duration: {}s",
            self.view.maximum_custody_duration_seconds
        );
        if let Some(height) = self.view.exact_height_bound {
            duration.push_str(&format!(" · exact height {height}"));
        } else {
            duration.push_str(" · no exact height supplied");
        }
        surface(cx)
            .child(
                Label::new("Custody dimensions")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(Label::new(format!(
                "funds control: {}",
                self.view.funds_control
            )))
            .child(Label::new(format!(
                "key control: {}",
                self.view.key_control
            )))
            .child(Label::new(format!(
                "recovery control: {}",
                self.view.recovery_control
            )))
            .child(Label::new(format!(
                "counterparty exposure: {}",
                self.view.counterparty_exposure
            )))
            .child(Label::new(duration))
            .child(Label::new(format!(
                "credential exposure: {}",
                self.view.credential_exposure
            )))
    }
}

#[derive(IntoElement)]
pub struct RungLabel {
    rung: EvidenceRung,
}

impl RungLabel {
    pub fn new(rung: EvidenceRung) -> Self {
        Self { rung }
    }
}

impl RenderOnce for RungLabel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tone = match self.rung {
            EvidenceRung::Pledged | EvidenceRung::Reserved => VizChipTone::Neutral,
            EvidenceRung::Measured | EvidenceRung::Verified => VizChipTone::Active,
            EvidenceRung::Paid | EvidenceRung::Settled => VizChipTone::Ok,
        };
        VizChip::new(self.rung.label()).tone(tone)
    }
}

#[derive(IntoElement)]
pub struct ExpiryCountdown {
    expires_at: u64,
    now: u64,
}

impl ExpiryCountdown {
    pub fn new(expires_at: u64, now: u64) -> Self {
        Self { expires_at, now }
    }
}

impl RenderOnce for ExpiryCountdown {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        if self.expires_at > self.now {
            h_flex()
                .gap_1()
                .child(market_status_cue(
                    ("market-expiry", self.expires_at),
                    MarketStatusCue::Active,
                    format!("Quote expires in {} seconds", self.expires_at - self.now),
                ))
                .child(Label::new(format!("{}s", self.expires_at - self.now)))
                .into_any_element()
        } else {
            market_status_cue(
                ("market-expiry", self.expires_at),
                MarketStatusCue::Expired,
                "Quote expiry enforced locally",
            )
        }
    }
}

#[derive(IntoElement)]
pub struct FeeBreakdown {
    view: FeeBreakdownView,
}

impl FeeBreakdown {
    pub fn new(view: FeeBreakdownView) -> Self {
        Self { view }
    }
}

impl RenderOnce for FeeBreakdown {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        surface(cx)
            .child(
                Label::new("Signed fill promise")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(Label::new(format!(
                "provider fee: {}",
                self.view.provider_fee
            )))
            .child(Label::new(format!(
                "miner fee budget: {}",
                self.view.miner_fee_budget
            )))
            .child(Label::new(format!(
                "lightning routing fee budget: {}",
                self.view.lightning_routing_fee_budget
            )))
            .child(Label::new(format!("fee payer: {}", self.view.fee_payer)))
            .child(Label::new(format!("rounding: {}", self.view.rounding_rule)))
            .child(Label::new(format!(
                "equation: {}",
                self.view.amount_equation
            )))
            .child(Label::new(format!(
                "maximum total fee: {}",
                self.view.maximum_total_fee
            )))
    }
}

#[derive(IntoElement)]
pub struct PriceFeedProvenance {
    view: PriceFeedView,
    now: u64,
}

impl PriceFeedProvenance {
    pub fn new(view: PriceFeedView, now: u64) -> Self {
        Self { view, now }
    }
}

impl RenderOnce for PriceFeedProvenance {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let stale = self.view.is_stale(self.now);
        surface(cx)
            .child(
                h_flex()
                    .gap_1()
                    .child(Label::new("Price-feed provenance"))
                    .child(market_status_cue(
                        ("market-price-feed", self.view.observed_at),
                        if stale {
                            MarketStatusCue::Warning
                        } else {
                            MarketStatusCue::Verified
                        },
                        if stale {
                            "Price feed is stale"
                        } else {
                            "Price feed is within its signed age bound"
                        },
                    )),
            )
            .child(Label::new(format!(
                "{}{}",
                self.view.url, self.view.value_pointer
            )))
            .child(Label::new(format!(
                "observation {} at {} · max age {}s",
                self.view.observed_value, self.view.observed_at, self.view.max_age_seconds
            )))
            .child(
                Label::new(format!("response digest {}", self.view.response_sha256))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }
}

#[derive(IntoElement)]
pub struct QuoteCompareTable {
    quotes: Vec<QuoteView>,
    now: u64,
}

impl QuoteCompareTable {
    pub fn new(quotes: Vec<QuoteView>, now: u64) -> Self {
        Self { quotes, now }
    }
}

impl RenderOnce for QuoteCompareTable {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut quotes = self.quotes;
        quotes.sort_by(|left, right| {
            right
                .output_amount
                .len()
                .cmp(&left.output_amount.len())
                .then_with(|| right.output_amount.cmp(&left.output_amount))
                .then_with(|| {
                    left.fees
                        .maximum_total_fee
                        .len()
                        .cmp(&right.fees.maximum_total_fee.len())
                })
                .then_with(|| {
                    left.fees
                        .maximum_total_fee
                        .cmp(&right.fees.maximum_total_fee)
                })
                .then_with(|| left.provider_id.cmp(&right.provider_id))
        });
        surface(cx)
            .child(
                Label::new("Competing signed quotes · best execution first")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .children(quotes.into_iter().map(|quote| {
                let expiry = ExpiryCountdown::new(quote.expires_at, self.now);
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .flex_wrap()
                    .child(Label::new(short_id(&quote.provider_id)))
                    .child(VizChip::new(quote.quote_class).tone(VizChipTone::Active))
                    .child(Label::new(format!(
                        "{} {} → {} {}",
                        quote.input_amount,
                        quote.input.display_ticker,
                        quote.output_amount,
                        quote.output.display_ticker
                    )))
                    .child(Label::new(format!(
                        "fee ≤ {}",
                        quote.fees.maximum_total_fee
                    )))
                    .child(ReservationBadge::new(quote.reservation))
                    .child(expiry)
            }))
    }
}

#[derive(IntoElement)]
pub struct SessionTimeline {
    lanes: Vec<TimelineLaneView>,
}

impl SessionTimeline {
    pub fn new(lanes: Vec<TimelineLaneView>) -> Self {
        Self { lanes }
    }
}

impl RenderOnce for SessionTimeline {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        surface(cx)
            .child(
                Label::new("Per-signer timeline")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .children(self.lanes.into_iter().map(|lane| {
                h_flex()
                    .gap_1()
                    .items_center()
                    .flex_wrap()
                    .child(Label::new(lane.signer_role))
                    .children(lane.slots.into_iter().map(|slot| {
                        let (label, tone) = match slot.state {
                            TimelineSlotState::Event => (slot.labels.join(" / "), VizChipTone::Ok),
                            TimelineSlotState::Gap => (
                                format!("gap · {}", slot.labels.join(" / ")),
                                VizChipTone::Warn,
                            ),
                            TimelineSlotState::Fork => (
                                format!("fork · {}", slot.labels.join(" / ")),
                                VizChipTone::Warn,
                            ),
                            TimelineSlotState::Malformed => (
                                format!("malformed · {}", slot.labels.join(" / ")),
                                VizChipTone::Warn,
                            ),
                        };
                        VizChip::new(label).tone(tone)
                    }))
            }))
    }
}

#[derive(IntoElement)]
pub struct VerifyChecklist {
    view: VerifyChecklistView,
}

impl VerifyChecklist {
    pub fn new(view: VerifyChecklistView) -> Self {
        Self { view }
    }
}

impl RenderOnce for VerifyChecklist {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let funding_authorized = self.view.funding_authorized();
        surface(cx)
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("Verify before fund"))
                    .child(market_status_cue(
                        "market-funding-authorization",
                        if funding_authorized {
                            MarketStatusCue::Verified
                        } else {
                            MarketStatusCue::Missing
                        },
                        if funding_authorized {
                            "Funding authorization"
                        } else {
                            "Funding authorization is unreachable"
                        },
                    )),
            )
            .children(self.view.rows.into_iter().map(|row| {
                let (glyph, tone) = match row.state {
                    VerifyState::Pending => ("…", VizChipTone::Neutral),
                    VerifyState::Passed => ("✓", VizChipTone::Ok),
                    VerifyState::Failed => ("×", VizChipTone::Warn),
                };
                h_flex()
                    .gap_1()
                    .child(VizChip::new(glyph).tone(tone))
                    .child(Label::new(row.label))
                    .when_some(row.error_code, |line, code| {
                        line.child(Label::new(code).size(LabelSize::XSmall).color(Color::Error))
                    })
            }))
    }
}

#[derive(IntoElement)]
pub struct ExitPackageBadge {
    view: ExitPackageView,
}

impl ExitPackageBadge {
    pub fn new(view: ExitPackageView) -> Self {
        Self { view }
    }
}

impl RenderOnce for ExitPackageBadge {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        h_flex()
            .gap_1()
            .child(Label::new("Exit package"))
            .child(market_status_cue(
                "market-exit-package",
                if self.view.exists {
                    MarketStatusCue::Verified
                } else {
                    MarketStatusCue::Missing
                },
                "Exit package",
            ))
            .when_some(self.view.latest_safe_height, |row, height| {
                row.child(Label::new(format!("latest safe height {height}")))
            })
    }
}

#[derive(IntoElement)]
pub struct ReceiptCard {
    view: ReceiptView,
}

impl ReceiptCard {
    pub fn new(view: ReceiptView) -> Self {
        Self { view }
    }
}

impl RenderOnce for ReceiptCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let receipt_id = self.view.receipt_id.clone();
        let outcome_status = if self.view.outcome.contains("settled")
            || self.view.outcome.contains("closed")
            || self.view.outcome.contains("success")
        {
            MarketStatusCue::Verified
        } else {
            MarketStatusCue::Warning
        };
        surface(cx)
            .id(format!("market-receipt-{receipt_id}"))
            .role(gpui::Role::Group)
            .aria_label(format!("Receipt outcome: {}", self.view.outcome))
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("Receipt"))
                    .child(market_status_cue(
                        format!("market-receipt-status-{receipt_id}"),
                        outcome_status,
                        "Receipt",
                    )),
            )
            .child(Label::new(format!(
                "{} · one signer's claim · {} · redacted: {}",
                self.view.signer_claim,
                self.view.rung.label(),
                self.view.redacted
            )))
    }
}

#[derive(IntoElement)]
pub struct TypedErrorMessage {
    view: TypedErrorView,
}

impl TypedErrorMessage {
    pub fn new(view: TypedErrorView) -> Self {
        Self { view }
    }
}

impl RenderOnce for TypedErrorMessage {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        Callout::new()
            .severity(Severity::Error)
            .title(self.view.code.clone())
            .description(self.view.local_message())
    }
}

#[derive(IntoElement)]
pub struct SwapFlow {
    view: MarketSessionViewModel,
    now: u64,
}

impl SwapFlow {
    pub fn new(view: MarketSessionViewModel, now: u64) -> Self {
        Self { view, now }
    }
}

impl RenderOnce for SwapFlow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let view = self.view;
        let quotes = view.quotes.clone();
        let first_quote = quotes.first().cloned();
        let session_id = view.session_id.clone();
        let phase = view.phase.clone();
        let phase_cue = if phase.contains("terminal") || phase.contains("verified") {
            MarketStatusCue::Verified
        } else if phase.contains("invalid") || phase.contains("cancel") {
            MarketStatusCue::Warning
        } else {
            MarketStatusCue::Active
        };
        v_flex()
            .id(format!("market-swap-flow-{session_id}"))
            .role(gpui::Role::Region)
            .aria_label(format!("Swap session phase: {phase}"))
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(h_flex().gap_1().child(Label::new("Swap session")).child(
                        market_status_cue(
                            format!("market-session-status-{session_id}"),
                            phase_cue,
                            format!("Swap session phase {phase}"),
                        ),
                    ))
                    .child(
                        Label::new(short_id(&session_id))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .child(ProviderBadge::new(view.provider))
            .when_some(view.offering, |flow, offering| {
                flow.child(OfferingCard::new(offering, self.now))
            })
            .child(QuoteCompareTable::new(quotes, self.now))
            .when_some(first_quote, |flow, quote| {
                flow.child(FeeBreakdown::new(quote.fees.clone()))
                    .when_some(quote.price_feed, |flow, price_feed| {
                        flow.child(PriceFeedProvenance::new(price_feed, self.now))
                    })
                    .child(CustodyStrip::new(quote.custody))
            })
            .child(SessionTimeline::new(view.timeline))
            .child(VerifyChecklist::new(view.verification))
            .child(ExitPackageBadge::new(view.exit_package))
            .when_some(view.receipt, |flow, receipt| {
                flow.child(ReceiptCard::new(receipt))
            })
            .children(view.errors.into_iter().map(TypedErrorMessage::new))
    }
}
