use documented::Documented;
use gpui::{canvas, point, px};

use crate::components::viz::{
    SceneText, SceneTextAnchor, VizAnchor, VizPalette, VizShape, even_dash, fill_circle, perimeter,
    stroke_circle, stroke_rounded_rect, viz_font,
};
use crate::prelude::*;

/// The market actor a node represents; the role picks its stroke color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VizNodeRole {
    Requester,
    Relay,
    Provider,
    Rail,
    Service,
    Neutral,
}

impl VizNodeRole {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Requester => "requester",
            Self::Relay => "relay",
            Self::Provider => "provider",
            Self::Rail => "rail",
            Self::Service => "service",
            Self::Neutral => "neutral",
        }
    }

    fn stroke(&self, palette: &VizPalette) -> gpui::Hsla {
        match self {
            Self::Requester => palette.socket,
            Self::Relay => palette.giftwrap,
            Self::Provider => palette.bitcoin,
            Self::Rail => palette.channel,
            Self::Service => palette.muted,
            Self::Neutral => palette.node,
        }
    }
}

/// Node health. State is encoded redundantly — stroke color, dash pattern,
/// and a label glyph suffix — so it survives grayscale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VizNodeState {
    Ready,
    Starting,
    Degraded,
    Offline,
}

impl VizNodeState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Starting => "starting",
            Self::Degraded => "degraded",
            Self::Offline => "offline",
        }
    }

    /// The dash cycle for this state, in logical units.
    pub fn dash(&self) -> Option<(f32, f32)> {
        match self {
            Self::Ready => None,
            Self::Starting => Some((2.0, 2.5)),
            Self::Degraded => Some((5.0, 2.0)),
            Self::Offline => Some((1.5, 3.0)),
        }
    }

    /// The glyph suffix appended to the node label for this state.
    pub fn glyph(&self) -> &'static str {
        match self {
            Self::Ready => "",
            Self::Starting => " …",
            Self::Degraded => " !",
            Self::Offline => " ×",
        }
    }

    fn stroke_override(&self, palette: &VizPalette) -> Option<gpui::Hsla> {
        match self {
            Self::Ready | Self::Starting => None,
            Self::Degraded => Some(palette.warn),
            Self::Offline => Some(palette.danger),
        }
    }

    fn opacity(&self) -> f32 {
        match self {
            Self::Offline => 0.55,
            _ => 1.0,
        }
    }
}

/// A market network node: circle for actors, rect for rails and services,
/// with role-colored stroke and redundant (dash + glyph + opacity) state
/// encoding, per the Bazaar network visualization spec.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct VizNode {
    shape: VizShape,
    role: VizNodeRole,
    state: VizNodeState,
    label: Option<SharedString>,
    sublabel: Option<SharedString>,
    selected: bool,
    scale: f32,
    palette: Option<VizPalette>,
}

impl VizNode {
    pub fn circle(radius: f32) -> Self {
        Self::new(VizShape::Circle { radius })
    }

    pub fn rect(width: f32, height: f32) -> Self {
        Self::new(VizShape::Rect { width, height })
    }

    fn new(shape: VizShape) -> Self {
        Self {
            shape,
            role: VizNodeRole::Neutral,
            state: VizNodeState::Ready,
            label: None,
            sublabel: None,
            selected: false,
            scale: 1.5,
            palette: None,
        }
    }

    pub fn role(mut self, role: VizNodeRole) -> Self {
        self.role = role;
        self
    }

    pub fn state(mut self, state: VizNodeState) -> Self {
        self.state = state;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn sublabel(mut self, sublabel: impl Into<SharedString>) -> Self {
        self.sublabel = Some(sublabel.into());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Logical-unit to pixel scale. Bazaar constants are viewBox units;
    /// the default renders them at 1.5 px per unit.
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

const RING_PADDING: f32 = 5.0;
const NODE_STROKE_WIDTH: f32 = 1.25;
const LABEL_FONT_SIZE: f32 = 10.0;
const SUBLABEL_FONT_SIZE: f32 = 8.0;

impl RenderOnce for VizNode {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(|| VizPalette::from_theme(cx));
        let scale = self.scale;
        let (half_width, half_height) = match self.shape {
            VizShape::Circle { radius } => (radius, radius),
            VizShape::Rect { width, height } => (width / 2.0, height / 2.0),
        };

        let glyph_extent = half_width + RING_PADDING + 3.0;
        let label_estimate = self
            .label
            .as_ref()
            .map(|label| (label.len() + 2) as f32 * LABEL_FONT_SIZE * 0.62)
            .unwrap_or(0.0)
            .max(
                self.sublabel
                    .as_ref()
                    .map(|sublabel| sublabel.len() as f32 * SUBLABEL_FONT_SIZE * 0.62)
                    .unwrap_or(0.0),
            );
        let canvas_width = (glyph_extent * 2.0).max(label_estimate + 8.0) * scale;
        let top_extent = half_height + RING_PADDING + 3.0;
        let mut bottom_extent = half_height + RING_PADDING + 3.0;
        if self.label.is_some() {
            bottom_extent = half_height + 13.0 + LABEL_FONT_SIZE * 0.7;
        }
        if self.sublabel.is_some() {
            bottom_extent += 11.0;
        }
        let canvas_height = (top_extent + bottom_extent) * scale;

        let shape = self.shape;
        let role = self.role;
        let state = self.state;
        let label = self.label.clone();
        let sublabel = self.sublabel.clone();
        let selected = self.selected;
        let font = viz_font(cx);

        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let center = point(
                    bounds.origin.x + bounds.size.width / 2.,
                    bounds.origin.y + px(top_extent * scale),
                );
                let opacity = state.opacity();
                let stroke = state
                    .stroke_override(&palette)
                    .unwrap_or_else(|| role.stroke(&palette))
                    .opacity(opacity);
                let fill = palette.node_fill.opacity(opacity);
                let dash = state
                    .dash()
                    .map(|(dash, gap)| (px(dash * scale), px(gap * scale)));
                let stroke_width = px(NODE_STROKE_WIDTH * scale);

                match shape {
                    VizShape::Circle { radius } => {
                        let radius_px = px(radius * scale);
                        fill_circle(window, center, radius_px, fill);
                        stroke_circle(window, center, radius_px, stroke_width, dash, stroke);
                    }
                    VizShape::Rect { width, height } => {
                        let size = gpui::size(px(width * scale), px(height * scale));
                        let origin = point(center.x - size.width / 2., center.y - size.height / 2.);
                        window.paint_quad(
                            gpui::fill(gpui::Bounds { origin, size }, fill)
                                .corner_radii(px(6.0 * scale)),
                        );
                        stroke_rounded_rect(
                            window,
                            origin,
                            size,
                            px(6.0 * scale),
                            stroke_width,
                            dash,
                            stroke,
                        );
                    }
                }

                if selected {
                    let ring_anchor = match shape {
                        VizShape::Circle { radius } => {
                            VizAnchor::circle(0.0, 0.0, radius + RING_PADDING)
                        }
                        VizShape::Rect { width, height } => VizAnchor::rect(
                            0.0,
                            0.0,
                            width + RING_PADDING * 2.0,
                            height + RING_PADDING * 2.0,
                        ),
                    };
                    let (dash_length, gap_length) = even_dash(perimeter(&ring_anchor), 3.0, 3.0);
                    let ring_dash = Some((px(dash_length * scale), px(gap_length * scale)));
                    match ring_anchor.shape {
                        VizShape::Circle { radius } => stroke_circle(
                            window,
                            center,
                            px(radius * scale),
                            px(scale),
                            ring_dash,
                            stroke,
                        ),
                        VizShape::Rect { width, height } => {
                            let size = gpui::size(px(width * scale), px(height * scale));
                            let origin =
                                point(center.x - size.width / 2., center.y - size.height / 2.);
                            stroke_rounded_rect(
                                window,
                                origin,
                                size,
                                px(8.0 * scale),
                                px(scale),
                                ring_dash,
                                stroke,
                            );
                        }
                    }
                }

                if let Some(label) = label {
                    let mut text = SceneText::new(px(LABEL_FONT_SIZE * scale));
                    text.push(&label, font.clone(), palette.node_text.opacity(opacity));
                    text.push(state.glyph(), font.clone(), stroke);
                    text.paint(
                        window,
                        cx,
                        SceneTextAnchor::Center,
                        point(center.x, center.y + px((half_height + 13.0) * scale)),
                    );
                }
                if let Some(sublabel) = sublabel {
                    let mut text = SceneText::new(px(SUBLABEL_FONT_SIZE * scale));
                    text.push(&sublabel, font.clone(), palette.muted.opacity(opacity));
                    text.paint(
                        window,
                        cx,
                        SceneTextAnchor::Center,
                        point(center.x, center.y + px((half_height + 24.0) * scale)),
                    );
                }
            },
        )
        .w(px(canvas_width))
        .h(px(canvas_height))
    }
}

impl Component for VizNode {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let roles = [
            VizNodeRole::Requester,
            VizNodeRole::Relay,
            VizNodeRole::Provider,
            VizNodeRole::Rail,
            VizNodeRole::Service,
            VizNodeRole::Neutral,
        ];
        let states = [
            VizNodeState::Ready,
            VizNodeState::Starting,
            VizNodeState::Degraded,
            VizNodeState::Offline,
        ];

        let role_grid = |palette: Option<VizPalette>| {
            v_flex()
                .gap_2()
                .children(roles.iter().map(|role| {
                    h_flex().gap_4().children(states.iter().map(|state| {
                        let mut node =
                            if matches!(role, VizNodeRole::Rail | VizNodeRole::Service) {
                                VizNode::rect(56.0, 20.0)
                            } else {
                                VizNode::circle(16.0)
                            }
                            .role(*role)
                            .state(*state)
                            .label(role.label())
                            .sublabel(state.label());
                        if let Some(palette) = palette {
                            node = node.palette(palette);
                        }
                        node
                    }))
                }))
                .into_any_element()
        };

        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Roles × States",
                vec![single_example("All roles and states", role_grid(None))],
            ))
            .child(example_group_with_title(
                "Selected ring",
                vec![single_example(
                    "Perimeter-fitted dashes",
                    h_flex()
                        .gap_4()
                        .child(
                            VizNode::circle(16.0)
                                .role(VizNodeRole::Relay)
                                .label("relay-a")
                                .selected(true),
                        )
                        .child(
                            VizNode::rect(56.0, 20.0)
                                .role(VizNodeRole::Rail)
                                .label("bitcoind")
                                .selected(true),
                        )
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "State survives without hue",
                    role_grid(Some(VizPalette::from_theme(cx).grayscale())),
                )],
            ))
            .into_any_element()
    }
}
