use documented::Documented;
use plugin_api::{VenueAccountMode, VenueCapabilityReport, VenueCapabilityVerificationStatus};

use crate::Chip;
use crate::components::viz::{MarketBadge, MarketBadgeKind, MarketTokens};
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VenueConnectionState {
    Connected,
    Degraded,
    Halted,
}
#[derive(Debug, Clone)]
pub struct VenueStatus {
    pub venue: SharedString,
    pub connection: VenueConnectionState,
    pub capabilities: VenueCapabilityReport,
}
pub trait VenueStatusSource {
    fn venue_status(&self) -> VenueStatus;
}
pub struct DemoVenueStatusSource;
impl VenueStatusSource for DemoVenueStatusSource {
    fn venue_status(&self) -> VenueStatus {
        use plugin_api::{
            ObservedVenueMode, ProbedVenueAssumption, VenueCapabilities,
            VenueCapabilityVerification, VenueMarginMode,
        };
        VenueStatus {
            venue: "Hyperliquid".into(),
            connection: VenueConnectionState::Connected,
            capabilities: VenueCapabilityReport {
                capabilities: Some(VenueCapabilities {
                    venue_id: "hyperliquid".into(),
                    account_mode: ProbedVenueAssumption::new(
                        ObservedVenueMode::known(VenueAccountMode::UnifiedAccount, "unified"),
                        1_754_700_000_000,
                    ),
                    margin_mode: ProbedVenueAssumption::new(
                        ObservedVenueMode::known(VenueMarginMode::Cross, "cross"),
                        1_754_700_000_000,
                    ),
                    actions: Vec::new(),
                }),
                verification: VenueCapabilityVerification {
                    status: VenueCapabilityVerificationStatus::Verified,
                    stale: false,
                    oldest_probed_at_ms: Some(1_754_700_000_000),
                    newest_probed_at_ms: Some(1_754_700_000_000),
                    reasons: Vec::new(),
                },
            },
        }
    }
}
fn account_mode_label(report: &VenueCapabilityReport) -> &'static str {
    match report
        .capabilities
        .as_ref()
        .and_then(|capabilities| capabilities.account_mode.value.typed)
    {
        Some(VenueAccountMode::SingleAccount) => "single",
        Some(VenueAccountMode::UnifiedAccount) => "unified",
        Some(VenueAccountMode::PortfolioMargin) => "portfolio",
        None => "unknown",
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Venue connectivity plus the probed plugin account mode.
pub struct VenueStatusBadge {
    status: VenueStatus,
    tokens: Option<MarketTokens>,
}
impl VenueStatusBadge {
    pub fn from_source(source: &impl VenueStatusSource) -> Self {
        Self {
            status: source.venue_status(),
            tokens: None,
        }
    }
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}
impl RenderOnce for VenueStatusBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let kind = match self.status.connection {
            VenueConnectionState::Connected => MarketBadgeKind::VenueConnected,
            VenueConnectionState::Degraded => MarketBadgeKind::VenueDegraded,
            VenueConnectionState::Halted => MarketBadgeKind::VenueHalted,
        };
        let verified = self.status.capabilities.verification.status
            == VenueCapabilityVerificationStatus::Verified
            && !self.status.capabilities.verification.stale;
        h_flex()
            .debug_selector(|| "market.venue_status".into())
            .gap_2()
            .child(Label::new(self.status.venue).size(LabelSize::Small))
            .child(MarketBadge::new(kind).tokens(tokens))
            .child(
                Chip::new(format!(
                    "{} {}",
                    if verified { "✓" } else { "?" },
                    account_mode_label(&self.status.capabilities)
                ))
                .label_color(Color::Custom(if verified {
                    tokens.up
                } else {
                    tokens.down
                })),
            )
    }
}
impl Component for VenueStatusBadge {
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
                "Venue status",
                vec![single_example(
                    "Connection and account mode",
                    VenueStatusBadge::from_source(&DemoVenueStatusSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Glyphs and labels preserve both states",
                    VenueStatusBadge::from_source(&DemoVenueStatusSource)
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
    fn reads_typed_account_mode() {
        let status = DemoVenueStatusSource.venue_status();
        assert_eq!(account_mode_label(&status.capabilities), "unified");
    }
}
