use documented::Documented;
use gpui::{canvas, point, px};

use crate::components::viz::{
    SceneText, SceneTextAnchor, VizEdgeClass, VizPalette, fill_circle, polar, stroke_circle,
    viz_font,
};
use crate::prelude::*;

/// Whether a port receives or emits records. Direction is encoded by fill
/// alone — output ports are filled with the class color, input ports are
/// hollow — the one place fill carries a meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VizPortDirection {
    Input,
    Output,
}

/// A class-colored connection point on a node's rim, placed by polar angle
/// (0° = east, 90° = south).
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct VizPort {
    class: VizEdgeClass,
    direction: VizPortDirection,
    angle_deg: f32,
    node_radius: f32,
    label: Option<SharedString>,
    scale: f32,
    palette: Option<VizPalette>,
}

impl VizPort {
    pub fn new(class: VizEdgeClass, direction: VizPortDirection, angle_deg: f32) -> Self {
        Self {
            class,
            direction,
            angle_deg,
            node_radius: 18.0,
            label: None,
            scale: 1.5,
            palette: None,
        }
    }

    pub fn node_radius(mut self, node_radius: f32) -> Self {
        self.node_radius = node_radius;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
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

const PORT_RADIUS: f32 = 3.0;

impl RenderOnce for VizPort {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(|| VizPalette::from_theme(cx));
        let scale = self.scale;
        let node_radius = self.node_radius;
        let angle_deg = self.angle_deg;
        let class = self.class;
        let direction = self.direction;
        let label = self.label;
        let font = viz_font(cx);

        let extent = node_radius + PORT_RADIUS + 4.0;
        let label_rows = if label.is_some() { 14.0 } else { 0.0 };
        let canvas_size = extent * 2.0 * scale;

        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let center = point(
                    bounds.origin.x + bounds.size.width / 2.,
                    bounds.origin.y + px(extent * scale),
                );
                stroke_circle(
                    window,
                    center,
                    px(node_radius * scale),
                    px(1.25 * scale),
                    None,
                    palette.node,
                );

                let (port_x, port_y) = polar(0.0, 0.0, node_radius, angle_deg);
                let port_center =
                    point(center.x + px(port_x * scale), center.y + px(port_y * scale));
                let color = class.color(&palette);
                let fill = match direction {
                    VizPortDirection::Output => color,
                    VizPortDirection::Input => palette.node_fill,
                };
                fill_circle(window, port_center, px(PORT_RADIUS * scale), fill);
                stroke_circle(
                    window,
                    port_center,
                    px(PORT_RADIUS * scale),
                    px(scale),
                    None,
                    color,
                );

                if let Some(label) = &label {
                    let mut text = SceneText::new(px(8.0 * scale));
                    text.push(label, font.clone(), palette.muted);
                    text.paint(
                        window,
                        cx,
                        SceneTextAnchor::Center,
                        point(center.x, center.y + px((extent + 7.0) * scale)),
                    );
                }
            },
        )
        .w(px(canvas_size))
        .h(px(canvas_size + label_rows * scale))
    }
}

impl Component for VizPort {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let classes = [
            VizEdgeClass::Socket,
            VizEdgeClass::Giftwrap,
            VizEdgeClass::Channel,
            VizEdgeClass::Rpc,
        ];
        let row = |direction: VizPortDirection, palette: Option<VizPalette>| {
            h_flex()
                .gap_2()
                .children(classes.iter().enumerate().map(|(index, class)| {
                    let angle = 210.0 - index as f32 * 70.0;
                    let mut port = VizPort::new(*class, direction, angle).label(class.label());
                    if let Some(palette) = palette {
                        port = port.palette(palette);
                    }
                    port
                }))
                .into_any_element()
        };

        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Direction by fill",
                vec![
                    single_example("Output — filled", row(VizPortDirection::Output, None)),
                    single_example("Input — hollow", row(VizPortDirection::Input, None)),
                ],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Direction survives without hue",
                    row(
                        VizPortDirection::Output,
                        Some(VizPalette::from_theme(cx).grayscale()),
                    ),
                )],
            ))
            .into_any_element()
    }
}
