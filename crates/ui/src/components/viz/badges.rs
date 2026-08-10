//! Shared market tier, verification, venue, and environment badges.

use documented::Documented;

use crate::Chip;
use crate::components::viz::{MarketEnvironment, MarketTokens};
use crate::prelude::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarketBadgeKind {
    Open,
    Verified,
    Private,
    OpenAgents,
    VenueConnected,
    VenueDegraded,
    VenueHalted,
    Environment(MarketEnvironment),
}

impl MarketBadgeKind {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "○ OPEN",
            Self::Verified => "✓ VERIFIED",
            Self::Private => "◆ PRIVATE",
            Self::OpenAgents => "◇ OPENAGENTS",
            Self::VenueConnected => "● CONNECTED",
            Self::VenueDegraded => "◐ DEGRADED",
            Self::VenueHalted => "■ HALTED",
            Self::Environment(MarketEnvironment::Demo) => "D DEMO",
            Self::Environment(MarketEnvironment::Testnet) => "T TESTNET",
            Self::Environment(MarketEnvironment::Mainnet) => "M MAINNET",
        }
    }

    fn color(self, tokens: MarketTokens) -> gpui::Hsla {
        match self {
            Self::Verified | Self::OpenAgents | Self::VenueConnected => tokens.up,
            Self::VenueDegraded | Self::VenueHalted => tokens.down,
            Self::Environment(MarketEnvironment::Demo | MarketEnvironment::Testnet) => tokens.down,
            Self::Open | Self::Private | Self::Environment(MarketEnvironment::Mainnet) => {
                tokens.flat
            }
        }
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// One chip from the shared market status vocabulary.
pub struct MarketBadge {
    kind: MarketBadgeKind,
    tokens: Option<MarketTokens>,
}

impl MarketBadge {
    pub fn new(kind: MarketBadgeKind) -> Self {
        Self { kind, tokens: None }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for MarketBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let color = self.kind.color(tokens);
        div().debug_selector(|| "market.badge".into()).child(
            Chip::new(self.kind.label())
                .label_color(Color::Custom(color))
                .border_color(color.opacity(0.7))
                .bg_color(color.opacity(0.08)),
        )
    }
}

impl Component for MarketBadge {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let kinds = [
            MarketBadgeKind::Open,
            MarketBadgeKind::Verified,
            MarketBadgeKind::Private,
            MarketBadgeKind::OpenAgents,
            MarketBadgeKind::VenueConnected,
            MarketBadgeKind::VenueDegraded,
            MarketBadgeKind::VenueHalted,
            MarketBadgeKind::Environment(MarketEnvironment::Testnet),
            MarketBadgeKind::Environment(MarketEnvironment::Mainnet),
        ];
        let row = |tokens: Option<MarketTokens>| {
            h_flex()
                .flex_wrap()
                .gap_2()
                .children(kinds.into_iter().map(|kind| {
                    let mut badge = MarketBadge::new(kind);
                    if let Some(tokens) = tokens {
                        badge = badge.tokens(tokens);
                    }
                    badge
                }))
                .into_any_element()
        };
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Market badges",
                vec![single_example(
                    "Tier, verification, venue, and network vocabulary",
                    row(None),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Glyph and label retain every distinction",
                    row(Some(MarketTokens::from_theme(cx).grayscale())),
                )],
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_badge_has_structural_text() {
        for kind in [
            MarketBadgeKind::Open,
            MarketBadgeKind::Verified,
            MarketBadgeKind::Private,
            MarketBadgeKind::OpenAgents,
            MarketBadgeKind::VenueConnected,
            MarketBadgeKind::VenueDegraded,
            MarketBadgeKind::VenueHalted,
            MarketBadgeKind::Environment(MarketEnvironment::Demo),
            MarketBadgeKind::Environment(MarketEnvironment::Testnet),
            MarketBadgeKind::Environment(MarketEnvironment::Mainnet),
        ] {
            assert!(kind.label().split_whitespace().count() >= 2);
        }
    }
}
