use std::time::Duration;

use documented::Documented;
use gpui::{Animation, AnimationExt as _, canvas, point, px};

use crate::components::viz::{
    SceneText, SceneTextAnchor, VizNodeRole, VizNodeState, VizPalette, fill_circle, polar,
    stroke_circle, stroke_line, viz_font,
};
use crate::prelude::*;

/// Whether a participant is in the deployment-signed manifest or merely
/// observed on the network. Tier is encoded by opacity plus a textual
/// `· unpinned` suffix — never opacity alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanoramaTrust {
    Fixture,
    Pinned,
    Discovered,
}

#[derive(Debug, Clone)]
pub struct PanoramaRelay {
    pub label: SharedString,
    pub state: VizNodeState,
    pub trust: PanoramaTrust,
}

#[derive(Debug, Clone)]
pub struct PanoramaProvider {
    pub label: SharedString,
    pub state: VizNodeState,
    pub trust: PanoramaTrust,
    /// Indexes into the relay list for this provider's home relays.
    pub relay_indices: Vec<usize>,
    pub fee_bps: u32,
    pub volume_sat_24h: u64,
}

/// Aggregate stats for the HUD. `None` means unknown and renders as `—`;
/// unknown is never drawn as zero.
#[derive(Debug, Clone, Copy, Default)]
pub struct PanoramaStats {
    pub swaps_24h: Option<u64>,
    pub volume_sat_24h: Option<u64>,
    pub operator_fee_sat_24h: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct PanoramaNetwork {
    pub name: SharedString,
    pub relays: Vec<PanoramaRelay>,
    pub providers: Vec<PanoramaProvider>,
    pub client_count: usize,
    pub stats: PanoramaStats,
    /// 0..=1; drives coordination-pulse counts. Zero when no relay is ready.
    pub activity: f32,
}

/// FNV-1a, for seeding the layout from the network name so the same network
/// always draws the same way.
fn hash32(text: &str) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for byte in text.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// mulberry32 — a tiny deterministic PRNG; layout must not consume ambient
/// randomness.
struct Mulberry32(u32);

impl Mulberry32 {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let mut z = self.0;
        z = (z ^ (z >> 15)).wrapping_mul(z | 1);
        z ^= z.wrapping_add((z ^ (z >> 7)).wrapping_mul(z | 61));
        ((z ^ (z >> 14)) as f32) / 4_294_967_296.0
    }
}

const SCENE_WIDTH: f32 = 900.0;
const SCENE_HEIGHT: f32 = 460.0;
const CENTER_X: f32 = 470.0;
const CENTER_Y: f32 = 230.0;
/// The rings are ellipses — the map reads as a wide panorama, not a square.
const RING_Y_SQUASH: f32 = 0.62;

fn place(radius: f32, angle_deg: f32) -> (f32, f32) {
    let (x, y) = polar(CENTER_X, CENTER_Y, radius, angle_deg);
    (x, CENTER_Y + (y - CENTER_Y) * RING_Y_SQUASH)
}

struct Placed {
    x: f32,
    y: f32,
    angle: f32,
}

fn ring_layout(network: &PanoramaNetwork) -> (Vec<Placed>, Vec<Placed>, Vec<(f32, f32, usize)>) {
    let relay_count = network.relays.len().max(1);
    let relay_radius = (120.0 + relay_count as f32 * 8.0).min(172.0);
    let provider_radius = relay_radius + 118.0;

    let relays: Vec<Placed> = (0..network.relays.len())
        .map(|index| {
            let angle = -90.0 + index as f32 / relay_count as f32 * 360.0;
            let (x, y) = polar(CENTER_X, CENTER_Y, relay_radius, angle);
            Placed { x, y, angle }
        })
        .collect();

    // Providers want the circular mean of their home relays' angles, then are
    // redistributed evenly with a half-slot stagger so the rings interleave.
    let provider_count = network.providers.len().max(1);
    let mut desired: Vec<(usize, f32)> = network
        .providers
        .iter()
        .enumerate()
        .map(|(index, provider)| {
            let (sin, cos) = provider
                .relay_indices
                .iter()
                .filter_map(|relay| relays.get(*relay))
                .fold((0.0f32, 0.0f32), |(sin, cos), placed| {
                    let radians = placed.angle.to_radians();
                    (sin + radians.sin(), cos + radians.cos())
                });
            (index, sin.atan2(cos).to_degrees())
        })
        .collect();
    desired.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut providers: Vec<Option<Placed>> = network.providers.iter().map(|_| None).collect();
    for (slot, (provider_index, _)) in desired.iter().enumerate() {
        let angle = -90.0 + (slot as f32 + 0.5) / provider_count as f32 * 360.0;
        let (x, y) = place(provider_radius, angle);
        providers[*provider_index] = Some(Placed { x, y, angle });
    }
    let providers: Vec<Placed> = providers
        .into_iter()
        .map(|placed| {
            placed.unwrap_or(Placed {
                x: CENTER_X,
                y: CENTER_Y,
                angle: 0.0,
            })
        })
        .collect();

    // A seeded client cloud inside the relay ring, each dot homed to a relay.
    let mut random = Mulberry32(hash32(&network.name));
    let clients: Vec<(f32, f32, usize)> = (0..network.client_count)
        .map(|_| {
            let home = (random.next() * relays.len().max(1) as f32) as usize % relays.len().max(1);
            let home_angle = relays.get(home).map(|relay| relay.angle).unwrap_or(0.0);
            let spread = 0.55 + random.next() * 0.5;
            let angle = home_angle + (random.next() - 0.5) * (180.0 / relay_count as f32) * 1.6;
            let radius = relay_radius * (1.0 - spread * 0.72) + random.next() * 18.0;
            let (x, y) = place(radius, angle);
            (x, y, home)
        })
        .collect();

    (relays, providers, clients)
}

fn format_sats_compact(amount: u64) -> String {
    if amount >= 100_000_000 {
        format!("{:.2} BTC", amount as f64 / 100_000_000.0)
    } else if amount >= 1_000_000 {
        format!("{:.1}M sats", amount as f64 / 1_000_000.0)
    } else {
        format!("{amount} sats")
    }
}

/// The birds-eye market map, ported from the Bazaar network panorama:
/// relays on an inner ring, providers interleaved on an outer ring with
/// √-area volume sizing, a seeded client cloud, volume-scaled sockets whose
/// pulses stop (never hide) when infrastructure goes down, and a HUD whose
/// unknown stats render as unknown, never as zero. Layout is deterministic —
/// no force simulation, so no relay is ever visually privileged.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct NetworkPanorama {
    network: PanoramaNetwork,
    /// Animation clock in seconds; the preview drives this with a repeating
    /// animation, so reduced motion freezes pulses at their seeded phases.
    time: f32,
    width: f32,
    show_hud: bool,
    palette: Option<VizPalette>,
}

impl NetworkPanorama {
    pub fn new(network: PanoramaNetwork) -> Self {
        Self {
            network,
            time: 0.42 * 60.0,
            width: 620.0,
            show_hud: true,
            palette: None,
        }
    }

    pub fn time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Hides the in-scene HUD; the inline card carries those stats in its
    /// own chrome instead.
    pub fn show_hud(mut self, show_hud: bool) -> Self {
        self.show_hud = show_hud;
        self
    }

    /// Overrides the theme palette; used by the grayscale audit preview.
    pub fn palette(mut self, palette: VizPalette) -> Self {
        self.palette = Some(palette);
        self
    }
}

impl RenderOnce for NetworkPanorama {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(|| VizPalette::from_theme(cx));
        let network = self.network;
        let time = self.time;
        let show_hud = self.show_hud;
        let font = viz_font(cx);
        let scale = self.width / SCENE_WIDTH;
        let canvas_height = SCENE_HEIGHT * scale;
        // Scene text keeps a legibility floor when the scene shrinks to
        // inline-card widths.
        let text_size = move |logical: f32| px((logical * scale).max(8.0));

        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let scene = |x: f32, y: f32| {
                    point(
                        bounds.origin.x + px(x * scale),
                        bounds.origin.y + px(y * scale),
                    )
                };
                let (relays, providers, clients) = ring_layout(&network);
                let max_volume = network
                    .providers
                    .iter()
                    .map(|provider| provider.volume_sat_24h)
                    .max()
                    .unwrap_or(0)
                    .max(1);

                // Client sockets, sampled at every third client, skipped when
                // the home relay is offline.
                for (index, (x, y, home)) in clients.iter().enumerate() {
                    if index % 3 != 0 {
                        continue;
                    }
                    let Some(relay) = network.relays.get(*home) else {
                        continue;
                    };
                    if relay.state == VizNodeState::Offline {
                        continue;
                    }
                    let Some(placed) = relays.get(*home) else {
                        continue;
                    };
                    stroke_line(
                        window,
                        scene(*x, *y),
                        scene(placed.x, placed.y),
                        px(0.4 * scale),
                        None,
                        palette.socket.opacity(0.14),
                    );
                }

                // Provider↔relay sockets, volume-scaled, state-dashed; pulses
                // ride them while the link is live.
                let mut random = Mulberry32(hash32(&network.name).wrapping_add(1));
                for (provider_index, provider) in network.providers.iter().enumerate() {
                    let Some(from) = providers.get(provider_index) else {
                        continue;
                    };
                    for relay_index in &provider.relay_indices {
                        let (Some(to), Some(relay)) =
                            (relays.get(*relay_index), network.relays.get(*relay_index))
                        else {
                            continue;
                        };
                        let volume_share = provider.volume_sat_24h as f32 / max_volume as f32;
                        let down = provider.state == VizNodeState::Offline
                            || relay.state == VizNodeState::Offline;
                        let degraded = relay.state == VizNodeState::Degraded;
                        let (color, dash, opacity) = if down {
                            (palette.danger, Some((2.0, 4.0)), 0.3)
                        } else if degraded {
                            (palette.warn, Some((5.0, 3.0)), 0.35)
                        } else {
                            (palette.socket, None, 0.45)
                        };
                        stroke_line(
                            window,
                            scene(from.x, from.y),
                            scene(to.x, to.y),
                            px((0.7 + volume_share * 2.4) * scale),
                            dash.map(|(dash, gap)| (px(dash * scale), px(gap * scale))),
                            color.opacity(opacity),
                        );

                        // Coordination pulses: bounded by activity, zero when
                        // the link is down — offline stops pulsing instead of
                        // being hidden.
                        if !down {
                            let count = ((network.activity * (1.0 + volume_share * 5.0)).round()
                                as usize)
                                .max(usize::from(network.activity > 0.0));
                            for _ in 0..count {
                                let phase = random.next();
                                let period = 2.4 + random.next() * 2.2;
                                let reverse = random.next() > 0.5;
                                let mut progress = (time / period + phase).fract();
                                if reverse {
                                    progress = 1.0 - progress;
                                }
                                let x = from.x + (to.x - from.x) * progress;
                                let y = from.y + (to.y - from.y) * progress;
                                fill_circle(
                                    window,
                                    scene(x, y),
                                    px(1.6 * scale),
                                    palette.giftwrap.opacity(0.9),
                                );
                            }
                        }
                    }
                }

                for (x, y, _) in &clients {
                    fill_circle(
                        window,
                        scene(*x, *y),
                        px(1.8 * scale),
                        palette.muted.opacity(0.75),
                    );
                }

                let draw_node = |window: &mut Window,
                                 cx: &mut App,
                                 placed: &Placed,
                                 label: &SharedString,
                                 state: VizNodeState,
                                 trust: PanoramaTrust,
                                 role: VizNodeRole,
                                 radius: f32| {
                    let tier_opacity = match trust {
                        PanoramaTrust::Fixture | PanoramaTrust::Pinned => 1.0,
                        PanoramaTrust::Discovered => 0.55,
                    };
                    let opacity = tier_opacity
                        * match state {
                            VizNodeState::Offline => 0.55,
                            _ => 1.0,
                        };
                    let stroke = match state {
                        VizNodeState::Degraded => palette.warn,
                        VizNodeState::Offline => palette.danger,
                        _ => match role {
                            VizNodeRole::Relay => palette.giftwrap,
                            _ => palette.bitcoin,
                        },
                    }
                    .opacity(opacity);
                    let center = scene(placed.x, placed.y);
                    fill_circle(
                        window,
                        center,
                        px(radius * scale),
                        palette.node_fill.opacity(opacity),
                    );
                    stroke_circle(
                        window,
                        center,
                        px(radius * scale),
                        px(1.25 * scale),
                        state
                            .dash()
                            .map(|(dash, gap)| (px(dash * scale), px(gap * scale))),
                        stroke,
                    );
                    let mut text = SceneText::new(text_size(8.5));
                    let suffix = match trust {
                        PanoramaTrust::Fixture => " · fixture",
                        PanoramaTrust::Pinned => "",
                        PanoramaTrust::Discovered => " · unpinned",
                    };
                    text.push(
                        &format!("{label}{}{suffix}", state.glyph()),
                        font.clone(),
                        palette.node_text.opacity(opacity),
                    );
                    text.paint(
                        window,
                        cx,
                        SceneTextAnchor::Center,
                        scene(placed.x, placed.y + radius + 11.0),
                    );
                };

                for (index, relay) in network.relays.iter().enumerate() {
                    if let Some(placed) = relays.get(index) {
                        draw_node(
                            window,
                            cx,
                            placed,
                            &relay.label,
                            relay.state,
                            relay.trust,
                            VizNodeRole::Relay,
                            13.0,
                        );
                    }
                }
                for (index, provider) in network.providers.iter().enumerate() {
                    if let Some(placed) = providers.get(index) {
                        let volume_share = provider.volume_sat_24h as f32 / max_volume as f32;
                        draw_node(
                            window,
                            cx,
                            placed,
                            &provider.label,
                            provider.state,
                            provider.trust,
                            VizNodeRole::Provider,
                            9.0 + volume_share.sqrt() * 9.0,
                        );
                    }
                }

                if !show_hud {
                    return;
                }
                // HUD: keys muted, values right-aligned; unknown is `—`,
                // never a fabricated zero. Operator fees are the only tinted
                // value.
                let hud_origin = scene(14.0, 14.0);
                window.paint_quad(
                    gpui::fill(
                        gpui::Bounds {
                            origin: hud_origin,
                            size: gpui::size(px(196.0 * scale), px(132.0 * scale)),
                        },
                        palette.node_fill.opacity(0.85),
                    )
                    .corner_radii(px(8.0 * scale)),
                );
                let mut title = SceneText::new(text_size(7.5));
                title.push(&network.name.to_uppercase(), font.clone(), palette.muted);
                title.paint(window, cx, SceneTextAnchor::Left, scene(26.0, 32.0));

                let unknown = "—".to_string();
                let ready = |count: usize, total: usize| format!("{count}/{total}");
                let relay_ready = network
                    .relays
                    .iter()
                    .filter(|relay| relay.state == VizNodeState::Ready)
                    .count();
                let provider_ready = network
                    .providers
                    .iter()
                    .filter(|provider| provider.state == VizNodeState::Ready)
                    .count();
                let rows: [(&str, String, Option<gpui::Hsla>); 6] = [
                    (
                        "swaps 24h",
                        network
                            .stats
                            .swaps_24h
                            .map(|swaps| swaps.to_string())
                            .unwrap_or_else(|| unknown.clone()),
                        None,
                    ),
                    (
                        "volume 24h",
                        network
                            .stats
                            .volume_sat_24h
                            .map(format_sats_compact)
                            .unwrap_or_else(|| unknown.clone()),
                        None,
                    ),
                    (
                        "operator fees",
                        network
                            .stats
                            .operator_fee_sat_24h
                            .map(format_sats_compact)
                            .unwrap_or_else(|| unknown.clone()),
                        Some(palette.bitcoin),
                    ),
                    (
                        "providers",
                        ready(provider_ready, network.providers.len()),
                        None,
                    ),
                    ("relays", ready(relay_ready, network.relays.len()), None),
                    ("clients", network.client_count.to_string(), None),
                ];
                for (index, (key, value, tint)) in rows.iter().enumerate() {
                    let y = 50.0 + index as f32 * 16.0;
                    let mut key_text = SceneText::new(text_size(8.0));
                    key_text.push(key, font.clone(), palette.muted);
                    key_text.paint(window, cx, SceneTextAnchor::Left, scene(26.0, y));
                    let mut value_text = SceneText::new(text_size(8.5));
                    value_text.push(value, font.clone(), tint.unwrap_or(palette.node_text));
                    value_text.paint(window, cx, SceneTextAnchor::Right, scene(198.0, y));
                }
            },
        )
        .w(px(self.width))
        .h(px(canvas_height))
    }
}

/// A fixture shaped like the live public regtest deployment: two pinned
/// relays, three providers, receipts not yet aggregated (stats unknown).
pub fn live_shape_fixture() -> PanoramaNetwork {
    PanoramaNetwork {
        name: "public regtest".into(),
        relays: vec![
            PanoramaRelay {
                label: "relay-a".into(),
                state: VizNodeState::Ready,
                trust: PanoramaTrust::Pinned,
            },
            PanoramaRelay {
                label: "relay-b".into(),
                state: VizNodeState::Ready,
                trust: PanoramaTrust::Pinned,
            },
        ],
        providers: vec![
            PanoramaProvider {
                label: "provider-a".into(),
                state: VizNodeState::Ready,
                trust: PanoramaTrust::Pinned,
                relay_indices: vec![0, 1],
                fee_bps: 18,
                volume_sat_24h: 2_400_000,
            },
            PanoramaProvider {
                label: "provider-b".into(),
                state: VizNodeState::Ready,
                trust: PanoramaTrust::Pinned,
                relay_indices: vec![0, 1],
                fee_bps: 22,
                volume_sat_24h: 5_100_000,
            },
            PanoramaProvider {
                label: "joiner".into(),
                state: VizNodeState::Ready,
                trust: PanoramaTrust::Discovered,
                relay_indices: vec![1],
                fee_bps: 30,
                volume_sat_24h: 150_000,
            },
        ],
        client_count: 9,
        stats: PanoramaStats {
            swaps_24h: Some(128),
            volume_sat_24h: Some(7_650_000),
            operator_fee_sat_24h: Some(16_830),
        },
        activity: 0.25,
    }
}

pub(crate) fn demo_shape_fixture() -> PanoramaNetwork {
    PanoramaNetwork {
        name: "representative demo network".into(),
        relays: vec![
            PanoramaRelay {
                label: "relay-a".into(),
                state: VizNodeState::Ready,
                trust: PanoramaTrust::Fixture,
            },
            PanoramaRelay {
                label: "relay-b".into(),
                state: VizNodeState::Ready,
                trust: PanoramaTrust::Fixture,
            },
        ],
        providers: vec![
            PanoramaProvider {
                label: "provider-b".into(),
                state: VizNodeState::Ready,
                trust: PanoramaTrust::Fixture,
                relay_indices: vec![0, 1],
                fee_bps: 22,
                volume_sat_24h: 0,
            },
            PanoramaProvider {
                label: "provider-c".into(),
                state: VizNodeState::Ready,
                trust: PanoramaTrust::Fixture,
                relay_indices: vec![0, 1],
                fee_bps: 34,
                volume_sat_24h: 0,
            },
        ],
        client_count: 1,
        stats: PanoramaStats::default(),
        activity: 0.2,
    }
}

fn outage_fixture() -> PanoramaNetwork {
    let mut network = live_shape_fixture();
    network.name = "public regtest · outage drill".into();
    // Receipts stop aggregating during the drill; unknown renders as a dash,
    // never as a fabricated zero.
    network.stats = PanoramaStats::default();
    if let Some(relay) = network.relays.get_mut(1) {
        relay.state = VizNodeState::Offline;
    }
    if let Some(provider) = network.providers.get_mut(2) {
        provider.state = VizNodeState::Degraded;
    }
    network
}

impl Component for NetworkPanorama {
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
                "Live shape",
                vec![single_example(
                    "The current public network's shape, pulses in motion",
                    NetworkPanorama::new(live_shape_fixture())
                        .with_animation(
                            "network-panorama-clock",
                            Animation::new(Duration::from_secs(60)).repeat(),
                            |panorama, delta| panorama.time(delta * 60.0),
                        )
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Outage drill",
                vec![single_example(
                    "A dead relay stops pulsing and dashes red; it is never hidden",
                    NetworkPanorama::new(outage_fixture()).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "States and tiers survive without hue",
                    NetworkPanorama::new(outage_fixture())
                        .palette(VizPalette::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

/// The network map as an inline conversation card: tool-call chrome around
/// the panorama scene, with the HUD's aggregates carried in the card footer
/// so they stay legible at transcript widths. Unknown stats render as
/// unknown, never as zero.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct NetworkCard {
    network: PanoramaNetwork,
    time: f32,
    palette: Option<VizPalette>,
}

impl NetworkCard {
    pub fn new(network: PanoramaNetwork) -> Self {
        Self {
            network,
            time: 0.42 * 60.0,
            palette: None,
        }
    }

    pub fn time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }

    /// Overrides the theme palette; used by the grayscale audit preview.
    pub fn palette(mut self, palette: VizPalette) -> Self {
        self.palette = Some(palette);
        self
    }
}

impl RenderOnce for NetworkCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.theme().colors();
        // The tool-call card recipe from the transcript renderers.
        let card_border = colors.border.opacity(0.8);
        let header_background = colors
            .element_background
            .blend(colors.editor_foreground.opacity(0.025));

        let relay_ready = self
            .network
            .relays
            .iter()
            .filter(|relay| relay.state == VizNodeState::Ready)
            .count();
        let provider_ready = self
            .network
            .providers
            .iter()
            .filter(|provider| provider.state == VizNodeState::Ready)
            .count();
        let all_ready = relay_ready == self.network.relays.len()
            && provider_ready == self.network.providers.len();
        let (status_icon, status_color) = if all_ready {
            (IconName::Check, Color::Success)
        } else {
            (IconName::Warning, Color::Warning)
        };

        let unknown = "—".to_string();
        let swaps = self
            .network
            .stats
            .swaps_24h
            .map(|swaps| swaps.to_string())
            .unwrap_or_else(|| unknown.clone());
        let volume = self
            .network
            .stats
            .volume_sat_24h
            .map(format_sats_compact)
            .unwrap_or(unknown);

        let panorama_width = 418.0;
        let mut panorama = NetworkPanorama::new(self.network.clone())
            .width(panorama_width)
            .show_hud(false)
            .time(self.time);
        if let Some(palette) = self.palette {
            panorama = panorama.palette(palette);
        }

        v_flex()
            .w(px(420.))
            .my_1p5()
            .rounded_md()
            .border_1()
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
                        h_flex().px_1().gap_1p5().items_center().child(
                            Label::new(self.network.name.clone())
                                .size(LabelSize::Custom(rems_from_px(13.)))
                                .buffer_font(cx),
                        ),
                    )
                    .child(
                        h_flex()
                            .px_1()
                            .gap_1p5()
                            .items_center()
                            .child(
                                Icon::new(status_icon)
                                    .size(IconSize::Small)
                                    .color(status_color),
                            )
                            .child(
                                Label::new(format!(
                                    "{relay_ready}/{} relays · {provider_ready}/{} providers",
                                    self.network.relays.len(),
                                    self.network.providers.len()
                                ))
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                            ),
                    ),
            )
            .child(panorama)
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1()
                    .justify_between()
                    .border_t_1()
                    .border_color(card_border)
                    .child(
                        Label::new(format!("swaps 24h {swaps}"))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .buffer_font(cx),
                    )
                    .child(
                        Label::new(format!("volume 24h {volume}"))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .buffer_font(cx),
                    ),
            )
    }
}

impl Component for NetworkCard {
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
                "In conversation",
                vec![single_example(
                    "The network map answering a conversational question",
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
                                    .child("What does the swap network look like right now?"),
                            ),
                        )
                        .child(
                            Label::new(
                                "Two pinned relays and three providers are live; receipts \
                                 are not aggregated yet, so volume is unknown.",
                            )
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                        )
                        .child(NetworkCard::new(live_shape_fixture()).with_animation(
                            "network-card-clock",
                            Animation::new(Duration::from_secs(60)).repeat(),
                            |card, delta| card.time(delta * 60.0),
                        ))
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Network modes",
                vec![
                    single_example(
                        "Deterministic demo fixture",
                        NetworkCard::new(demo_shape_fixture()).into_any_element(),
                    ),
                    single_example(
                        "Live public regtest shape",
                        NetworkCard::new(live_shape_fixture()).into_any_element(),
                    ),
                ],
            ))
            .child(example_group_with_title(
                "Outage drill",
                vec![single_example(
                    "Dead infrastructure stops pulsing; it is never hidden",
                    NetworkCard::new(outage_fixture()).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "States and tiers survive without hue",
                    NetworkCard::new(outage_fixture())
                        .palette(VizPalette::from_theme(cx).grayscale())
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
    fn the_layout_is_deterministic_and_finite() {
        let network = live_shape_fixture();
        let (relays_a, providers_a, clients_a) = ring_layout(&network);
        let (relays_b, providers_b, clients_b) = ring_layout(&network);
        assert_eq!(clients_a.len(), clients_b.len());
        for (a, b) in clients_a.iter().zip(&clients_b) {
            assert_eq!(a, b);
        }
        for placed in relays_a.iter().chain(&providers_a) {
            assert!(placed.x.is_finite() && placed.y.is_finite());
        }
        assert_eq!(relays_a.len(), 2);
        assert_eq!(providers_a.len(), 3);
        assert_eq!(relays_b.len(), 2);
        assert_eq!(providers_b.len(), 3);
    }

    #[test]
    fn compact_sats_pick_readable_units() {
        assert_eq!(format_sats_compact(950), "950 sats");
        assert_eq!(format_sats_compact(5_100_000), "5.1M sats");
        assert_eq!(format_sats_compact(2_026_000_000), "20.26 BTC");
    }
}
