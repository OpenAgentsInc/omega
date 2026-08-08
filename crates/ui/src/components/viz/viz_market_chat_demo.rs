use std::time::Duration;

use documented::Documented;
use gpui::{Animation, AnimationExt as _, px, relative};

use crate::components::viz::{NetworkCard, VizPalette, demo_card, demo_stage, live_shape_fixture};
use crate::prelude::*;

/// The primary-interface user bubble, verbatim from the transcript renderer.
fn user_bubble(text: &'static str, cx: &App) -> gpui::AnyElement {
    h_flex()
        .w_full()
        .justify_end()
        .child(
            div()
                .flex_none()
                .max_w(relative(0.8))
                .px(px(16.))
                .py(px(10.))
                .rounded(px(16.))
                .bg(cx.theme().colors().elevated_surface_background)
                .text_size(px(14.))
                .child(text),
        )
        .into_any_element()
}

fn agent_line(text: &'static str) -> gpui::AnyElement {
    Label::new(text)
        .size(LabelSize::Small)
        .color(Color::Muted)
        .into_any_element()
}

/// One conversation exercising both market cards: the person asks what the
/// network looks like and gets the inline map, then requests a swap and
/// watches the swap card walk its lifecycle. Pure demo data; the second pass
/// wires both cards to the live network.
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
        let mut network_card = NetworkCard::new(live_shape_fixture()).time(self.phase * 36.0);
        let mut swap_card = demo_card().stage(demo_stage(self.phase));
        if let Some(palette) = self.palette {
            network_card = network_card.palette(palette);
            swap_card = swap_card.palette(palette);
        }

        v_flex()
            .gap_2()
            .max_w(px(560.))
            .child(user_bubble(
                "What does the swap network look like right now?",
                cx,
            ))
            .child(agent_line(
                "Two pinned relays and three providers are live. Here is the map.",
            ))
            .child(network_card)
            .child(user_bubble(
                "Nice. Swap 50,000 sats from Lightning to on-chain BTC.",
                cx,
            ))
            .child(agent_line(
                "Best firm quote is provider-b at 22 bps. Approving runs the swap below.",
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
