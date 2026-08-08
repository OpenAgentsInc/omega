use documented::Documented;
use gpui::{canvas, point, px};

use crate::components::viz::{
    SceneText, SceneTextAnchor, VizPalette, stroke_line, stroke_rounded_rect, viz_font,
};
use crate::prelude::*;

/// A labeled dashed region grouping the nodes that share an operational or
/// custody domain, with an uppercase monospace caption.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct VizZone {
    width: f32,
    height: f32,
    label: SharedString,
    detail: Option<SharedString>,
    scale: f32,
    palette: Option<VizPalette>,
}

impl VizZone {
    pub fn new(width: f32, height: f32, label: impl Into<SharedString>) -> Self {
        Self {
            width,
            height,
            label: label.into(),
            detail: None,
            scale: 1.5,
            palette: None,
        }
    }

    /// A muted detail appended `· detail` after the label.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
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

impl RenderOnce for VizZone {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(|| VizPalette::from_theme(cx));
        let scale = self.scale;
        let width = self.width;
        let height = self.height;
        let label = self.label.clone();
        let detail = self.detail;
        let font = viz_font(cx);

        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let origin = bounds.origin;
                let size = gpui::size(px(width * scale), px(height * scale));
                window.paint_quad(
                    gpui::fill(
                        gpui::Bounds { origin, size },
                        palette.node_fill.opacity(0.4),
                    )
                    .corner_radii(px(10.0 * scale)),
                );
                stroke_rounded_rect(
                    window,
                    origin,
                    size,
                    px(10.0 * scale),
                    px(scale),
                    Some((px(3.0 * scale), px(3.0 * scale))),
                    palette.boundary,
                );

                let mut text = SceneText::new(px(7.5 * scale));
                text.push(&label.to_uppercase(), font.clone(), palette.muted);
                if let Some(detail) = &detail {
                    text.push(&format!(" · {detail}"), font.clone(), palette.muted);
                }
                text.paint(
                    window,
                    cx,
                    SceneTextAnchor::Left,
                    point(origin.x + px(10.0 * scale), origin.y + px(12.0 * scale)),
                );
            },
        )
        .w(px(width * scale))
        .h(px(height * scale))
    }
}

/// The custody boundary drawn as a first-class element: a vertical dashed
/// divider with a side label on each half. Money-colored edges never cross it.
#[derive(IntoElement, Documented)]
pub struct VizBoundary {
    height: f32,
    label_left: SharedString,
    label_right: SharedString,
    scale: f32,
    palette: Option<VizPalette>,
}

impl VizBoundary {
    pub fn new(
        height: f32,
        label_left: impl Into<SharedString>,
        label_right: impl Into<SharedString>,
    ) -> Self {
        Self {
            height,
            label_left: label_left.into(),
            label_right: label_right.into(),
            scale: 1.5,
            palette: None,
        }
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

impl RenderOnce for VizBoundary {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(|| VizPalette::from_theme(cx));
        let scale = self.scale;
        let height = self.height;
        let label_left = self.label_left.clone();
        let label_right = self.label_right;
        let font = viz_font(cx);
        let label_columns = (label_left.len().max(label_right.len()) as f32) * 7.5 * 0.62 + 14.0;

        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let center_x = bounds.origin.x + bounds.size.width / 2.;
                stroke_line(
                    window,
                    point(center_x, bounds.origin.y),
                    point(center_x, bounds.origin.y + px(height * scale)),
                    px(scale),
                    Some((px(6.0 * scale), px(4.0 * scale))),
                    palette.boundary,
                );

                let label_y = bounds.origin.y + px(8.0 * scale);
                let mut left = SceneText::new(px(7.5 * scale));
                left.push(&label_left.to_uppercase(), font.clone(), palette.muted);
                left.paint(
                    window,
                    cx,
                    SceneTextAnchor::Right,
                    point(center_x - px(7.0 * scale), label_y),
                );
                let mut right = SceneText::new(px(7.5 * scale));
                right.push(&label_right.to_uppercase(), font.clone(), palette.muted);
                right.paint(
                    window,
                    cx,
                    SceneTextAnchor::Left,
                    point(center_x + px(7.0 * scale), label_y),
                );
            },
        )
        .w(px(label_columns * 2.0 * scale))
        .h(px(height * scale))
    }
}

impl Component for VizZone {
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
                "Zones",
                vec![
                    single_example(
                        "Coordination zone",
                        VizZone::new(190.0, 90.0, "coordination")
                            .detail("no custody")
                            .into_any_element(),
                    ),
                    single_example(
                        "Provider zone",
                        VizZone::new(190.0, 90.0, "provider-a")
                            .detail("custody")
                            .into_any_element(),
                    ),
                ],
            ))
            .child(example_group_with_title(
                "Custody boundary",
                vec![single_example(
                    "A drawn divider, not a caption",
                    VizBoundary::new(110.0, "no custody", "custody").into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Boundary reads from dashes alone",
                    VizBoundary::new(110.0, "no custody", "custody")
                        .palette(VizPalette::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}
