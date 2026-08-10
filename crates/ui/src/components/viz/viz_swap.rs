use std::time::Duration;

use documented::Documented;
use gpui::{Animation, AnimationExt as _, px};

use crate::components::viz::{VizPalette, VizProgressRail, format_sats};
use crate::prelude::*;
use crate::traits::animation_ext::CommonAnimationExt as _;

/// The rails a swap moves value between. Each asset keeps its market accent
/// color; the name never rides on color alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapAsset {
    Lightning,
    Bitcoin,
    Liquid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapNetwork {
    Demo,
    Regtest,
}

impl SwapNetwork {
    pub fn label(self) -> &'static str {
        match self {
            Self::Demo => "demo",
            Self::Regtest => "regtest",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapQuoteKind {
    Firm,
    Indicative,
}

impl SwapQuoteKind {
    fn verification(self) -> &'static str {
        match self {
            Self::Firm => "firm quote · exact terms awaiting execution",
            Self::Indicative => "indicative route · signed quotes obtained at execution",
        }
    }
}

impl SwapAsset {
    pub fn ticker(&self) -> &'static str {
        match self {
            Self::Lightning => "LN",
            Self::Bitcoin => "BTC",
            Self::Liquid => "L-BTC",
        }
    }

    pub fn color(&self, palette: &VizPalette) -> gpui::Hsla {
        match self {
            Self::Lightning => palette.channel,
            Self::Bitcoin => palette.bitcoin,
            Self::Liquid => palette.liquid,
        }
    }
}

/// The inline lifecycle of a market swap, collapsed from the MKT-SWP
/// submarine state machine to the five moments a person watches for, plus
/// the recovery exit that is always drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapStage {
    /// A firm quote is on the table with exact execution terms.
    Quote,
    /// Both sides signed the bilateral swap contract.
    Contract,
    /// Funding is broadcast and confirming on the source rail.
    Funding,
    /// The payment is in flight on the destination rail.
    Executing,
    /// Settled, with local verification passed.
    Settled,
    /// The swap unwound through its pre-signed exit; funds returned.
    Refunded,
}

impl SwapStage {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Quote => "quote",
            Self::Contract => "contract",
            Self::Funding => "funding",
            Self::Executing => "executing",
            Self::Settled => "settled",
            Self::Refunded => "refunded",
        }
    }

    /// A glyph that repeats the stage without color.
    pub fn glyph(&self) -> &'static str {
        match self {
            Self::Quote => "?",
            Self::Contract => "≡",
            Self::Funding => "…",
            Self::Executing => "→",
            Self::Settled => "✓",
            Self::Refunded => "↩",
        }
    }

    fn verification(&self) -> &'static str {
        match self {
            Self::Quote => "quote awaiting execution",
            Self::Contract => "exit package persisted before any funding",
            Self::Funding | Self::Executing => "provider status is a claim · verifying locally",
            Self::Settled => "verified locally · zero-loss close",
            Self::Refunded => "exit package executed · nothing lost",
        }
    }

    fn rail_position(&self) -> (usize, Option<usize>, bool) {
        // (completed, active, error) against the five-stop rail.
        match self {
            Self::Quote => (0, Some(0), false),
            Self::Contract => (1, Some(1), false),
            Self::Funding => (2, Some(2), false),
            Self::Executing => (3, Some(3), false),
            Self::Settled => (5, None, false),
            Self::Refunded => (2, Some(2), true),
        }
    }
}

/// An inline conversation card for a market asset swap: the pair and amount,
/// the provider quote, a lifecycle rail across the MKT-SWP stages, and a
/// verification caption that never upgrades a counterparty claim to truth.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct SwapCard {
    from: SwapAsset,
    to: SwapAsset,
    amount_sats: u64,
    provider: SharedString,
    fee_bps: Option<u32>,
    network: SwapNetwork,
    quote_kind: SwapQuoteKind,
    stage: SwapStage,
    palette: Option<VizPalette>,
}

impl SwapCard {
    pub fn new(
        from: SwapAsset,
        to: SwapAsset,
        amount_sats: u64,
        provider: impl Into<SharedString>,
        fee_bps: u32,
    ) -> Self {
        Self {
            from,
            to,
            amount_sats,
            provider: provider.into(),
            fee_bps: Some(fee_bps),
            network: SwapNetwork::Demo,
            quote_kind: SwapQuoteKind::Firm,
            stage: SwapStage::Quote,
            palette: None,
        }
    }

    pub fn stage(mut self, stage: SwapStage) -> Self {
        self.stage = stage;
        self
    }

    pub fn network(mut self, network: SwapNetwork) -> Self {
        self.network = network;
        self
    }

    pub fn quote_kind(mut self, quote_kind: SwapQuoteKind) -> Self {
        self.quote_kind = quote_kind;
        self
    }

    pub fn without_fee(mut self) -> Self {
        self.fee_bps = None;
        self
    }

    /// Overrides the theme palette; used by the grayscale audit preview.
    pub fn palette(mut self, palette: VizPalette) -> Self {
        self.palette = Some(palette);
        self
    }
}

const RAIL_STAGES: [&str; 5] = ["quote", "contract", "funding", "executing", "settled"];

impl SwapStage {
    /// The header status affordance, matching the elicitation card's
    /// icon-plus-word vocabulary; in-flight stages rotate.
    fn status_icon(&self) -> (IconName, Color, bool) {
        match self {
            Self::Quote => (IconName::Info, Color::Info, false),
            Self::Contract | Self::Funding | Self::Executing => {
                (IconName::TodoProgress, Color::Accent, true)
            }
            Self::Settled => (IconName::Check, Color::Success, false),
            Self::Refunded => (IconName::RotateCcw, Color::Warning, false),
        }
    }
}

impl RenderOnce for SwapCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(|| VizPalette::from_theme(cx));
        let stage = self.stage;
        let network = self.network;
        let verification = if stage == SwapStage::Quote {
            self.quote_kind.verification()
        } else {
            stage.verification()
        };
        let (completed, active, error) = stage.rail_position();
        let colors = cx.theme().colors();
        // The tool-call card recipe from the transcript renderers.
        let card_border = colors.border.opacity(0.8);
        let header_background = colors
            .element_background
            .blend(colors.editor_foreground.opacity(0.025));

        let asset_dot = |asset: SwapAsset| {
            div()
                .size(px(8.))
                .rounded_full()
                .border_1()
                .border_color(asset.color(&palette))
                .when(asset != self.from || stage != SwapStage::Quote, |dot| {
                    dot.bg(asset.color(&palette).opacity(0.6))
                })
        };

        let mut rail = VizProgressRail::new(RAIL_STAGES)
            .completed(completed)
            .error(error)
            .scale(1.0);
        if let Some(active) = active {
            rail = rail.active(active);
        }
        if let Some(palette) = self.palette {
            rail = rail.palette(palette);
        }

        let (status_icon, status_color, status_rotates) = stage.status_icon();
        let status_icon = Icon::new(status_icon)
            .size(IconSize::Small)
            .color(status_color);
        let status_icon = if status_rotates {
            status_icon.with_rotate_animation(2).into_any_element()
        } else {
            status_icon.into_any_element()
        };

        let provider_caption = self
            .fee_bps
            .map(|fee_bps| format!("{} · {fee_bps} bps", self.provider))
            .unwrap_or_else(|| self.provider.to_string());

        v_flex()
            .w(px(420.))
            .my_1p5()
            .rounded_md()
            .border_1()
            .when(stage == SwapStage::Refunded, |card| card.border_dashed())
            .border_color(card_border)
            .bg(colors.editor_background)
            .overflow_hidden()
            .child(
                h_flex()
                    .h_8()
                    .w_full()
                    .p_1()
                    .justify_between()
                    .bg(header_background)
                    .child(
                        h_flex()
                            .px_1()
                            .gap_1p5()
                            .items_center()
                            .child(asset_dot(self.from))
                            .child(
                                Label::new(format!(
                                    "{} {} → {}",
                                    format_sats(self.amount_sats),
                                    self.from.ticker(),
                                    self.to.ticker()
                                ))
                                .size(LabelSize::Custom(rems_from_px(13.)))
                                .buffer_font(cx),
                            )
                            .child(asset_dot(self.to)),
                    )
                    .child(
                        h_flex()
                            .px_1()
                            .gap_1p5()
                            .items_center()
                            .child(
                                Label::new(network.label())
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(status_icon)
                            .child(
                                Label::new(stage.label())
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    ),
            )
            .child(div().px_2().child(rail))
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .pb_2()
                    .gap_2()
                    .justify_between()
                    .child(
                        div().min_w_0().flex_1().child(
                            Label::new(provider_caption)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .buffer_font(cx)
                                .truncate(),
                        ),
                    )
                    .child(div().flex_none().child(
                        Label::new(verification).size(LabelSize::XSmall).color(
                            if stage == SwapStage::Settled {
                                Color::Success
                            } else {
                                Color::Muted
                            },
                        ),
                    )),
            )
    }
}

pub(crate) fn demo_stage(delta: f32) -> SwapStage {
    // Six equal beats with a settled hold at the end of the loop.
    match (delta * 6.0) as u32 {
        0 => SwapStage::Quote,
        1 => SwapStage::Contract,
        2 => SwapStage::Funding,
        3 => SwapStage::Executing,
        _ => SwapStage::Settled,
    }
}

pub(crate) fn demo_card() -> SwapCard {
    SwapCard::new(
        SwapAsset::Lightning,
        SwapAsset::Bitcoin,
        50_000,
        "provider-b",
        22,
    )
}

fn regtest_card(from: SwapAsset, to: SwapAsset, stage: SwapStage) -> SwapCard {
    SwapCard::new(
        from,
        to,
        100_000,
        "232aa9c2d3642abf9ba89e4c9f704b018630acfaf3e2c9faa2faa2b708341b18",
        22,
    )
    .network(SwapNetwork::Regtest)
    .stage(stage)
}

fn market_operation_label(operation: &str) -> &'static str {
    match operation {
        "market_network_status" => "network status",
        "market_swap_quote" => "quote",
        "market_execute_swap" => "execute",
        "market_swap_status" => "swap status",
        _ => "market operation",
    }
}

/// The fail-closed result for a mainnet market tool call.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct MarketWarningCard {
    operation: SharedString,
    warning: SharedString,
}

impl MarketWarningCard {
    pub fn new(operation: impl Into<SharedString>, warning: impl Into<SharedString>) -> Self {
        Self {
            operation: operation.into(),
            warning: warning.into(),
        }
    }
}

impl RenderOnce for MarketWarningCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.theme().colors();
        let operation = market_operation_label(self.operation.as_ref());

        v_flex()
            .w(px(420.))
            .my_1p5()
            .rounded_md()
            .border_1()
            .border_color(colors.border.opacity(0.8))
            .bg(colors.editor_background)
            .overflow_hidden()
            .child(
                h_flex()
                    .h_8()
                    .w_full()
                    .px_2()
                    .gap_2()
                    .bg(colors
                        .element_background
                        .blend(colors.editor_foreground.opacity(0.025)))
                    .child(
                        Icon::new(IconName::Warning)
                            .size(IconSize::Small)
                            .color(Color::Warning),
                    )
                    .child(Label::new("Mainnet blocked").size(LabelSize::Small))
                    .child(div().flex_1())
                    .child(
                        Label::new(operation)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .child(
                div().px_3().py_2().child(
                    Label::new(self.warning)
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            )
    }
}

impl Component for MarketWarningCard {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        const WARNING: &str =
            "Mainnet swap tools are blocked. No mainnet request was sent and no funds moved.";

        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Blocked mainnet operations",
                vec![single_example(
                    "Every market operation fails closed before network or state access",
                    v_flex()
                        .gap_2()
                        .children(
                            [
                                "market_network_status",
                                "market_swap_quote",
                                "market_execute_swap",
                                "market_swap_status",
                            ]
                            .into_iter()
                            .map(|operation| MarketWarningCard::new(operation, WARNING)),
                        )
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

impl Component for SwapCard {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let stages = [
            SwapStage::Quote,
            SwapStage::Contract,
            SwapStage::Funding,
            SwapStage::Executing,
            SwapStage::Settled,
            SwapStage::Refunded,
        ];

        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "In conversation",
                vec![single_example(
                    "A swap requested conversationally, replaying its lifecycle",
                    v_flex()
                        .gap_2()
                        .max_w(px(560.))
                        .child(
                            // The primary-interface user bubble, verbatim from
                            // the transcript renderer.
                            h_flex().w_full().justify_end().child(
                                div()
                                    .flex_none()
                                    .max_w(relative(0.8))
                                    .px(px(16.))
                                    .py(px(10.))
                                    .rounded(px(16.))
                                    .bg(cx.theme().colors().elevated_surface_background)
                                    .text_size(px(14.))
                                    .child("Swap 50,000 sats from Lightning to on-chain BTC"),
                            ),
                        )
                        .child(
                            Label::new(
                                "Best firm quote is provider-b at 22 bps. Approving runs the \
                                 swap below.",
                            )
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                        )
                        .child(demo_card().with_animation(
                            "swap-card-demo",
                            Animation::new(Duration::from_secs(12)).repeat(),
                            |card, delta| card.stage(demo_stage(delta)),
                        ))
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Network modes",
                vec![single_example(
                    "Demo fixtures, live regtest routes, and verified regtest settlement",
                    v_flex()
                        .gap_2()
                        .child(demo_card())
                        .child(
                            regtest_card(
                                SwapAsset::Lightning,
                                SwapAsset::Bitcoin,
                                SwapStage::Quote,
                            )
                            .quote_kind(SwapQuoteKind::Indicative),
                        )
                        .child(
                            regtest_card(
                                SwapAsset::Lightning,
                                SwapAsset::Bitcoin,
                                SwapStage::Contract,
                            )
                            .without_fee(),
                        )
                        .child(
                            regtest_card(
                                SwapAsset::Bitcoin,
                                SwapAsset::Lightning,
                                SwapStage::Settled,
                            )
                            .without_fee(),
                        )
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Demo asset pairs",
                vec![single_example(
                    "Every directed fixture pair across Lightning, Bitcoin, and Liquid Bitcoin",
                    v_flex()
                        .gap_2()
                        .children([
                            SwapCard::new(
                                SwapAsset::Lightning,
                                SwapAsset::Bitcoin,
                                50_000,
                                "provider-b",
                                22,
                            ),
                            SwapCard::new(
                                SwapAsset::Lightning,
                                SwapAsset::Liquid,
                                50_000,
                                "provider-b",
                                22,
                            ),
                            SwapCard::new(
                                SwapAsset::Bitcoin,
                                SwapAsset::Lightning,
                                50_000,
                                "provider-b",
                                22,
                            ),
                            SwapCard::new(
                                SwapAsset::Bitcoin,
                                SwapAsset::Liquid,
                                50_000,
                                "provider-b",
                                22,
                            ),
                            SwapCard::new(
                                SwapAsset::Liquid,
                                SwapAsset::Lightning,
                                50_000,
                                "provider-b",
                                22,
                            ),
                            SwapCard::new(
                                SwapAsset::Liquid,
                                SwapAsset::Bitcoin,
                                50_000,
                                "provider-b",
                                22,
                            ),
                        ])
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Stages",
                vec![single_example(
                    "Every lifecycle stage, including the always-drawn exit",
                    v_flex()
                        .gap_2()
                        .children(stages.iter().map(|stage| demo_card().stage(*stage)))
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Stage survives without hue",
                    v_flex()
                        .gap_2()
                        .children(
                            [
                                SwapStage::Executing,
                                SwapStage::Settled,
                                SwapStage::Refunded,
                            ]
                            .iter()
                            .map(|stage| {
                                demo_card()
                                    .stage(*stage)
                                    .palette(VizPalette::from_theme(cx).grayscale())
                            }),
                        )
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
    fn network_and_quote_labels_are_explicit() {
        assert_eq!(SwapNetwork::Demo.label(), "demo");
        assert_eq!(SwapNetwork::Regtest.label(), "regtest");
        assert_eq!(
            SwapQuoteKind::Firm.verification(),
            "firm quote · exact terms awaiting execution"
        );
        assert_eq!(
            SwapQuoteKind::Indicative.verification(),
            "indicative route · signed quotes obtained at execution"
        );
    }

    #[test]
    fn the_demo_walks_the_lifecycle_and_holds_on_settled() {
        assert_eq!(demo_stage(0.0), SwapStage::Quote);
        assert_eq!(demo_stage(0.2), SwapStage::Contract);
        assert_eq!(demo_stage(0.4), SwapStage::Funding);
        assert_eq!(demo_stage(0.55), SwapStage::Executing);
        assert_eq!(demo_stage(0.7), SwapStage::Settled);
        assert_eq!(demo_stage(0.99), SwapStage::Settled);
    }
}
