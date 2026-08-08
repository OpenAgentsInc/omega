use std::time::Duration;

use documented::Documented;
use gpui::{Animation, AnimationExt as _, px};

use crate::components::viz::{VizPalette, VizProgressRail};
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
    /// A firm quote is on the table, awaiting the person's approval.
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

    /// The truthful settlement caption: relay and provider claims stay
    /// labeled unverified until local verification passes.
    fn verification(&self) -> &'static str {
        match self {
            Self::Quote => "firm quote · reserves capacity on approval",
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

fn format_sats(amount: u64) -> String {
    let digits = amount.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(character);
    }
    format!("{grouped} sats")
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
    fee_bps: u32,
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
            fee_bps,
            stage: SwapStage::Quote,
            palette: None,
        }
    }

    pub fn stage(mut self, stage: SwapStage) -> Self {
        self.stage = stage;
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
                    .justify_between()
                    .child(
                        Label::new(format!("{} · {} bps", self.provider, self.fee_bps))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .buffer_font(cx),
                    )
                    .child(
                        Label::new(stage.verification())
                            .size(LabelSize::XSmall)
                            .color(if stage == SwapStage::Settled {
                                Color::Success
                            } else {
                                Color::Muted
                            }),
                    ),
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
    fn amounts_group_thousands() {
        assert_eq!(format_sats(0), "0 sats");
        assert_eq!(format_sats(950), "950 sats");
        assert_eq!(format_sats(50_000), "50,000 sats");
        assert_eq!(format_sats(1_234_567), "1,234,567 sats");
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
