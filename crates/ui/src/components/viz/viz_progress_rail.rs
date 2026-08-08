use documented::Documented;
use gpui::{canvas, point, px};

use crate::components::viz::{
    SceneText, SceneTextAnchor, VizPalette, even_dash, fill_circle, stroke_circle, stroke_line,
    viz_font,
};
use crate::prelude::*;

/// A protocol lifecycle rail: stage dots on a line, a completed span, a
/// perimeter-dashed ring on the active stage, and an error state that dashes
/// the span red — progress reads from fill and dash, never color alone.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct VizProgressRail {
    stages: Vec<SharedString>,
    completed: usize,
    active: Option<usize>,
    error: bool,
    show_all_labels: bool,
    scale: f32,
    palette: Option<VizPalette>,
}

impl VizProgressRail {
    pub fn new(stages: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
        Self {
            stages: stages.into_iter().map(Into::into).collect(),
            completed: 0,
            active: None,
            error: false,
            show_all_labels: true,
            scale: 1.5,
            palette: None,
        }
    }

    /// The number of stages completed, counted from the start.
    pub fn completed(mut self, completed: usize) -> Self {
        self.completed = completed;
        self
    }

    pub fn active(mut self, active: usize) -> Self {
        self.active = Some(active);
        self
    }

    pub fn error(mut self, error: bool) -> Self {
        self.error = error;
        self
    }

    /// When false, only the active stage's label renders.
    pub fn show_all_labels(mut self, show_all_labels: bool) -> Self {
        self.show_all_labels = show_all_labels;
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

const RAIL_WIDTH: f32 = 340.0;
const RAIL_MARGIN: f32 = 18.0;
const RAIL_Y: f32 = 14.0;
const DOT_RADIUS: f32 = 4.0;
const RING_RADIUS: f32 = 7.5;

impl RenderOnce for VizProgressRail {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(|| VizPalette::from_theme(cx));
        let scale = self.scale;
        let stages = self.stages.clone();
        let completed = self.completed.min(stages.len());
        let active = self.active.filter(|index| *index < stages.len());
        let error = self.error;
        let show_all_labels = self.show_all_labels;
        let font = viz_font(cx);

        let canvas_height = if show_all_labels { 40.0 } else { 34.0 };

        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                if stages.len() < 2 {
                    return;
                }
                let scene = |x: f32, y: f32| {
                    point(
                        bounds.origin.x + px(x * scale),
                        bounds.origin.y + px(y * scale),
                    )
                };
                let x0 = RAIL_MARGIN;
                let x1 = RAIL_WIDTH + RAIL_MARGIN;
                let step = (x1 - x0) / (stages.len() - 1) as f32;
                let stage_x = |index: usize| x0 + step * index as f32;

                stroke_line(
                    window,
                    scene(x0, RAIL_Y),
                    scene(x1, RAIL_Y),
                    px(scale),
                    None,
                    palette.boundary,
                );

                let reached = active
                    .unwrap_or(completed.saturating_sub(1))
                    .max(completed.saturating_sub(1));
                if completed > 0 || active.is_some() {
                    let span_color = if error {
                        palette.danger
                    } else {
                        palette.socket
                    };
                    let span_dash = error.then(|| (px(4.0 * scale), px(3.0 * scale)));
                    stroke_line(
                        window,
                        scene(x0, RAIL_Y),
                        scene(stage_x(reached), RAIL_Y),
                        px(1.5 * scale),
                        span_dash,
                        span_color,
                    );
                }

                for (index, _stage) in stages.iter().enumerate() {
                    let center = scene(stage_x(index), RAIL_Y);
                    let is_completed = index < completed;
                    let is_active = active == Some(index);
                    let dot_color = if error && is_active {
                        palette.danger
                    } else if is_completed || is_active {
                        palette.socket
                    } else {
                        palette.muted
                    };
                    if is_completed {
                        fill_circle(window, center, px(DOT_RADIUS * scale), dot_color);
                    } else {
                        fill_circle(window, center, px(DOT_RADIUS * scale), palette.node_fill);
                        let dash = (error && is_active).then(|| (px(2.0 * scale), px(2.0 * scale)));
                        stroke_circle(
                            window,
                            center,
                            px(DOT_RADIUS * scale),
                            px(scale),
                            dash,
                            dot_color,
                        );
                    }
                    if is_active {
                        let circumference = 2.0 * std::f32::consts::PI * RING_RADIUS;
                        let (dash_length, gap_length) = even_dash(circumference, 3.0, 3.0);
                        stroke_circle(
                            window,
                            center,
                            px(RING_RADIUS * scale),
                            px(scale),
                            Some((px(dash_length * scale), px(gap_length * scale))),
                            dot_color,
                        );
                    }
                }

                if show_all_labels {
                    for (index, stage) in stages.iter().enumerate() {
                        let mut text = SceneText::new(px(7.0 * scale));
                        text.push(stage, font.clone(), palette.muted);
                        text.paint(
                            window,
                            cx,
                            SceneTextAnchor::Center,
                            scene(stage_x(index), RAIL_Y + 18.0),
                        );
                    }
                } else if let Some(active) = active {
                    if let Some(stage) = stages.get(active) {
                        let mut text = SceneText::new(px(7.5 * scale));
                        text.push(stage, font.clone(), palette.node_text);
                        text.paint(
                            window,
                            cx,
                            SceneTextAnchor::Center,
                            scene((x0 + x1) / 2.0, RAIL_Y + 16.0),
                        );
                    }
                }
            },
        )
        .w(px((RAIL_WIDTH + RAIL_MARGIN * 2.0) * scale))
        .h(px(canvas_height * scale))
    }
}

impl Component for VizProgressRail {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let stages = ["discovered", "rfq", "quote", "order", "contract", "close"];

        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Lifecycle rails",
                vec![
                    single_example(
                        "In progress",
                        VizProgressRail::new(stages)
                            .completed(3)
                            .active(3)
                            .into_any_element(),
                    ),
                    single_example(
                        "Complete",
                        VizProgressRail::new(stages).completed(6).into_any_element(),
                    ),
                    single_example(
                        "Error, active label only",
                        VizProgressRail::new(stages)
                            .completed(2)
                            .active(2)
                            .error(true)
                            .show_all_labels(false)
                            .into_any_element(),
                    ),
                ],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Progress reads from fill and dash",
                    VizProgressRail::new(stages)
                        .completed(3)
                        .active(3)
                        .error(true)
                        .palette(VizPalette::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}
