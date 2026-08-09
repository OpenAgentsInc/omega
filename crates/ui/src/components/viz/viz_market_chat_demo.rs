use std::time::Duration;

use documented::Documented;
use gpui::{Animation, AnimationExt as _, px, relative};

use crate::CopyButton;
use crate::components::viz::{NetworkCard, VizPalette, demo_card, demo_shape_fixture, demo_stage};
use crate::prelude::*;

/// The primary-interface user bubble, verbatim from the transcript renderer,
/// with the hover copy affordance messages carry in the real chat.
fn user_message(index: usize, text: &'static str, cx: &App) -> gpui::AnyElement {
    let group = SharedString::from(format!("market-demo-user-{index}"));
    h_flex()
        .id(("market-demo-user", index))
        .group(group.clone())
        .w_full()
        .pt(px(7.0))
        .pb(px(7.0))
        .justify_end()
        .gap_1()
        .child(
            CopyButton::new(("market-demo-user-copy", index), text)
                .icon_size(IconSize::Small)
                .tooltip_label("Copy Message")
                .visible_on_hover(group),
        )
        .child(
            div()
                .flex_none()
                .max_w(relative(0.8))
                .px(px(16.0))
                .py(px(10.0))
                .rounded(px(16.0))
                .bg(cx.theme().colors().elevated_surface_background)
                .text_size(px(14.0))
                .child(text),
        )
        .into_any_element()
}

/// Agent prose at the transcript's text size — no bubble, no border — with
/// the hover copy affordance.
fn agent_message(index: usize, text: &'static str, cx: &App) -> gpui::AnyElement {
    let group = SharedString::from(format!("market-demo-agent-{index}"));
    h_flex()
        .id(("market-demo-agent", index))
        .group(group.clone())
        .w_full()
        .py(px(7.0))
        .gap_1()
        .items_start()
        .child(div().flex_1().min_w_0().text_ui(cx).child(text))
        .child(
            CopyButton::new(("market-demo-agent-copy", index), text)
                .icon_size(IconSize::Small)
                .tooltip_label("Copy Message")
                .visible_on_hover(group),
        )
        .into_any_element()
}

/// One conversation exercising both market cards: the person asks what the
/// network looks like and gets the inline map, then requests a swap and
/// watches the swap card walk its lifecycle. Every value is a demo fixture.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct MarketChatDemo {
    /// 0..=1 through the replay loop; drives the network pulses and the swap
    /// stage together.
    phase: f32,
    palette: Option<VizPalette>,
}

impl MarketChatDemo {
    pub fn new() -> Self {
        Self {
            phase: 0.6,
            palette: None,
        }
    }

    pub fn phase(mut self, phase: f32) -> Self {
        self.phase = phase;
        self
    }

    /// Overrides the theme palette; used by the grayscale audit preview.
    pub fn palette(mut self, palette: VizPalette) -> Self {
        self.palette = Some(palette);
        self
    }
}

impl Default for MarketChatDemo {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for MarketChatDemo {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut network_card = NetworkCard::new(demo_shape_fixture()).time(self.phase * 36.0);
        let mut swap_card = demo_card().stage(demo_stage(self.phase));
        if let Some(palette) = self.palette {
            network_card = network_card.palette(palette);
            swap_card = swap_card.palette(palette);
        }

        v_flex()
            .gap_1()
            .max_w(px(560.))
            .child(user_message(
                0,
                "What does the swap network look like right now?",
                cx,
            ))
            .child(agent_message(
                0,
                "Two pinned relays and three providers are live. Here is the map.",
                cx,
            ))
            .child(network_card)
            .child(user_message(
                1,
                "Nice. Swap 50,000 sats from Lightning to on-chain BTC.",
                cx,
            ))
            .child(agent_message(
                1,
                "Best firm quote is provider-b at 22 bps. Approving runs the swap below.",
                cx,
            ))
            .child(swap_card)
    }
}

impl Component for MarketChatDemo {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Replay",
                vec![single_example(
                    "Network question, then a swap walking its lifecycle",
                    MarketChatDemo::new()
                        .with_animation(
                            "market-chat-demo",
                            Animation::new(Duration::from_secs(18)).repeat(),
                            |demo, delta| demo.phase(delta),
                        )
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "The whole exchange survives without hue",
                    MarketChatDemo::new()
                        .phase(0.45)
                        .palette(VizPalette::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}
