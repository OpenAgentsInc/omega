use documented::Documented;
use gpui::{canvas, point, px};

use crate::components::viz::{
    SceneText, SceneTextAnchor, VizAnchor, VizPalette, arc_head, edge_geometry, fill_circle,
    stroke_circle, stroke_line, viz_font,
};
use crate::prelude::*;

/// The five market edge classes. Each binds a shape difference plus a color
/// difference — solid, long dash, double stroke, dotted, fine dotted — so
/// classes survive grayscale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VizEdgeClass {
    /// Relay WebSocket: solid, the heaviest stroke.
    Socket,
    /// NIP-59 gift-wrapped private records: long dash.
    Giftwrap,
    /// Lightning channel: double parallel stroke.
    Channel,
    /// Provider→rail RPC: dotted, muted.
    Rpc,
    /// The requester's independent chain observation: fine dotted.
    Evidence,
}

impl VizEdgeClass {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Socket => "socket",
            Self::Giftwrap => "giftwrap",
            Self::Channel => "channel",
            Self::Rpc => "rpc",
            Self::Evidence => "evidence",
        }
    }

    pub fn color(&self, palette: &VizPalette) -> gpui::Hsla {
        match self {
            Self::Socket => palette.socket,
            Self::Giftwrap => palette.giftwrap,
            Self::Channel => palette.channel,
            Self::Rpc => palette.muted,
            Self::Evidence => palette.ok,
        }
    }

    /// (stroke width, dash cycle, double stroke), in logical units.
    pub fn stroke_style(&self) -> (f32, Option<(f32, f32)>, bool) {
        match self {
            Self::Socket => (1.5, None, false),
            Self::Giftwrap => (1.25, Some((7.0, 4.0)), false),
            Self::Channel => (1.25, None, true),
            Self::Rpc => (1.0, Some((2.0, 3.0)), false),
            Self::Evidence => (1.0, Some((1.0, 4.0)), false),
        }
    }
}

/// How an edge terminates at its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VizEdgeHead {
    #[default]
    None,
    /// An open chevron at the target surface.
    Arrow,
    /// A concentric arc hugging a circular target's rim.
    Arc,
}

/// A market edge between two surface-anchored endpoints. Class is encoded by
/// stroke shape and color together; the label (with an optional state suffix
/// in the class color) sits above the midpoint.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct VizEdge {
    class: VizEdgeClass,
    label: Option<SharedString>,
    state: Option<SharedString>,
    head: VizEdgeHead,
    length: f32,
    scale: f32,
    palette: Option<VizPalette>,
}

impl VizEdge {
    pub fn new(class: VizEdgeClass) -> Self {
        Self {
            class,
            label: None,
            state: None,
            head: VizEdgeHead::None,
            length: 170.0,
            scale: 1.5,
            palette: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// A state suffix rendered `· state` in the edge class color.
    pub fn state(mut self, state: impl Into<SharedString>) -> Self {
        self.state = Some(state.into());
        self
    }

    pub fn head(mut self, head: VizEdgeHead) -> Self {
        self.head = head;
        self
    }

    pub fn length(mut self, length: f32) -> Self {
        self.length = length;
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Overrides the theme palette; used by the grayscale audit preview.
    pub fn palette(mut self, palette: VizPalette) -> Self {
        self.palette = Some(palette);
        self
    }
}

const ENDPOINT_RADIUS: f32 = 10.0;
const LABEL_FONT_SIZE: f32 = 8.5;
const DOUBLE_STROKE_OFFSET: f32 = 1.4;

impl RenderOnce for VizEdge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(|| VizPalette::from_theme(cx));
        let scale = self.scale;
        let length = self.length;
        let class = self.class;
        let head = self.head;
        let label = self.label.clone();
        let state = self.state;
        let font = viz_font(cx);

        let canvas_width = (length + ENDPOINT_RADIUS * 2.0 + 16.0) * scale;
        let canvas_height = 44.0 * scale;

        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let scene = |x: f32, y: f32| {
                    point(
                        bounds.origin.x + px(x * scale),
                        bounds.origin.y + px(y * scale),
                    )
                };
                let mid_y = 28.0;
                let from = VizAnchor::circle(ENDPOINT_RADIUS + 8.0, mid_y, ENDPOINT_RADIUS);
                let to = VizAnchor::circle(ENDPOINT_RADIUS + 8.0 + length, mid_y, ENDPOINT_RADIUS);

                for anchor in [&from, &to] {
                    let center = scene(anchor.x, anchor.y);
                    fill_circle(
                        window,
                        center,
                        px(ENDPOINT_RADIUS * scale),
                        palette.node_fill,
                    );
                    stroke_circle(
                        window,
                        center,
                        px(ENDPOINT_RADIUS * scale),
                        px(1.25 * scale),
                        None,
                        palette.node,
                    );
                }

                let padding_to = if head == VizEdgeHead::Arc { 6.0 } else { 2.0 };
                let geometry = edge_geometry(&from, &to, 2.0, padding_to);
                let (width, dash, double) = class.stroke_style();
                let color = class.color(&palette);
                let dash_px = dash.map(|(dash, gap)| (px(dash * scale), px(gap * scale)));

                if double {
                    let (nx, ny) = (-geometry.uy, geometry.ux);
                    for direction in [-1.0, 1.0] {
                        let offset = DOUBLE_STROKE_OFFSET * direction;
                        stroke_line(
                            window,
                            scene(geometry.x0 + nx * offset, geometry.y0 + ny * offset),
                            scene(geometry.x1 + nx * offset, geometry.y1 + ny * offset),
                            px(width * scale),
                            dash_px,
                            color,
                        );
                    }
                } else {
                    stroke_line(
                        window,
                        scene(geometry.x0, geometry.y0),
                        scene(geometry.x1, geometry.y1),
                        px(width * scale),
                        dash_px,
                        color,
                    );
                }

                match head {
                    VizEdgeHead::None => {}
                    VizEdgeHead::Arrow => {
                        // An open chevron, matching Bazaar's marker path.
                        let tip = (geometry.x1, geometry.y1);
                        let back = 6.0;
                        let spread = 3.0;
                        let (ux, uy) = (geometry.ux, geometry.uy);
                        let (nx, ny) = (-uy, ux);
                        for direction in [-1.0, 1.0] {
                            stroke_line(
                                window,
                                scene(
                                    tip.0 - ux * back + nx * spread * direction,
                                    tip.1 - uy * back + ny * spread * direction,
                                ),
                                scene(tip.0, tip.1),
                                px(1.25 * scale),
                                None,
                                color,
                            );
                        }
                    }
                    VizEdgeHead::Arc => {
                        if let Some(arc) = arc_head(&to, geometry.approach_deg, 2.5, 42.0) {
                            let mut builder = gpui::PathBuilder::stroke(px(1.25 * scale));
                            builder.move_to(scene(arc.start.0, arc.start.1));
                            builder.arc_to(
                                point(px(arc.radius * scale), px(arc.radius * scale)),
                                px(0.),
                                false,
                                true,
                                scene(arc.end.0, arc.end.1),
                            );
                            if let Ok(path) = builder.build() {
                                window.paint_path(path, color);
                            }
                        }
                    }
                }

                if label.is_some() || state.is_some() {
                    let mut text = SceneText::new(px(LABEL_FONT_SIZE * scale));
                    if let Some(label) = &label {
                        text.push(label, font.clone(), palette.muted);
                    }
                    if let Some(state) = &state {
                        text.push(&format!(" · {state}"), font.clone(), color);
                    }
                    text.paint(
                        window,
                        cx,
                        SceneTextAnchor::Center,
                        scene(
                            (geometry.x0 + geometry.x1) / 2.0,
                            (geometry.y0 + geometry.y1) / 2.0 - 9.0,
                        ),
                    );
                }
            },
        )
        .w(px(canvas_width))
        .h(px(canvas_height))
    }
}

impl Component for VizEdge {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let gallery = |palette: Option<VizPalette>| {
            let with_palette = |edge: VizEdge| match palette {
                Some(palette) => edge.palette(palette),
                None => edge,
            };
            v_flex()
                .gap_1()
                .child(with_palette(
                    VizEdge::new(VizEdgeClass::Socket)
                        .label("wss")
                        .state("live")
                        .head(VizEdgeHead::Arrow),
                ))
                .child(with_palette(
                    VizEdge::new(VizEdgeClass::Giftwrap)
                        .label("NIP-59 · 39604–39609")
                        .head(VizEdgeHead::Arc),
                ))
                .child(with_palette(
                    VizEdge::new(VizEdgeClass::Channel).label("channel"),
                ))
                .child(with_palette(
                    VizEdge::new(VizEdgeClass::Rpc)
                        .label("rpc")
                        .head(VizEdgeHead::Arrow),
                ))
                .child(with_palette(
                    VizEdge::new(VizEdgeClass::Evidence)
                        .label("evidence, not authority")
                        .head(VizEdgeHead::Arrow),
                ))
                .into_any_element()
        };

        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Edge classes",
                vec![single_example("Shape + color per class", gallery(None))],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Class survives without hue",
                    gallery(Some(VizPalette::from_theme(cx).grayscale())),
                )],
            ))
            .into_any_element()
    }
}
