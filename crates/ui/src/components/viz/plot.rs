//! The shared canvas plot kernel (omega#284).
//!
//! One plot base generalized from the viz geometry so every chart is a
//! configuration, not a fork: margin layout, linear numeric/time and value
//! scales, 1-2-5 and natural-boundary tick generation, axis labels, a
//! crosshair with axis readout chips, and a layer system that draws through
//! the data→pixel transforms with `PathBuilder`. Rendering laws apply: GPUI
//! canvas only, no SVG; layers draw only what falls inside the plot rect;
//! dashed strokes stay on straight open paths (the lyon dash sampler panics
//! on closed curved paths — see `dash_polyline`).
//!
//! Zoom and pan are out of scope for v1 (fixed window with autoscale), but
//! every transform runs through [`LinearScale`], so interaction later means
//! changing domains, not rewriting layers.

use chrono::{DateTime, Datelike, Utc};
use documented::Documented;
use gpui::{App, Bounds, Font, PathBuilder, Pixels, Point, Window, canvas, fill, point, px, size};

use crate::components::viz::{
    MarketTokens, SceneText, SceneTextAnchor, format_with_decimals, market_number_font,
};
use crate::prelude::*;

/// Logical-pixel margins around the plot rect; the right margin carries the
/// value axis and the bottom margin the time axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlotMargins {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Default for PlotMargins {
    fn default() -> Self {
        Self {
            top: 8.0,
            right: 56.0,
            bottom: 20.0,
            left: 8.0,
        }
    }
}

/// A linear data→pixel transform. Domains are f64 (millisecond timestamps
/// need the width); ranges are pixel positions. Inverted ranges are fine —
/// value axes map larger values to smaller y.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearScale {
    domain: (f64, f64),
    range: (f32, f32),
}

impl LinearScale {
    /// A degenerate domain (zero span or non-finite) maps everything to the
    /// range midpoint instead of dividing by zero.
    pub fn new(domain: (f64, f64), range: (f32, f32)) -> Self {
        Self { domain, range }
    }

    pub fn domain(&self) -> (f64, f64) {
        self.domain
    }

    fn span(&self) -> Option<f64> {
        let span = self.domain.1 - self.domain.0;
        (span.is_finite() && span != 0.0).then_some(span)
    }

    pub fn map(&self, value: f64) -> f32 {
        match self.span() {
            Some(span) => {
                let fraction = (value - self.domain.0) / span;
                self.range.0 + (fraction * (self.range.1 - self.range.0) as f64) as f32
            }
            None => (self.range.0 + self.range.1) / 2.0,
        }
    }

    pub fn invert(&self, position: f32) -> f64 {
        let range_span = self.range.1 - self.range.0;
        match self.span() {
            Some(span) if range_span != 0.0 => {
                let fraction = ((position - self.range.0) / range_span) as f64;
                self.domain.0 + fraction * span
            }
            _ => (self.domain.0 + self.domain.1) / 2.0,
        }
    }
}

/// The largest 1-2-5 step producing at most `target` intervals over `span`.
pub fn nice_step(span: f64, target: usize) -> f64 {
    let target = target.max(1) as f64;
    let raw = span / target;
    if !raw.is_finite() || raw <= 0.0 {
        return 1.0;
    }
    let magnitude = 10f64.powf(raw.log10().floor());
    let normalized = raw / magnitude;
    let factor = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    factor * magnitude
}

/// Round-valued ticks covering `[min, max]` with 1-2-5 stepping.
pub fn nice_ticks(min: f64, max: f64, target: usize) -> Vec<f64> {
    if !min.is_finite() || !max.is_finite() || max <= min {
        return Vec::new();
    }
    let step = nice_step(max - min, target);
    let mut ticks = Vec::new();
    let mut tick = (min / step).ceil() * step;
    // Bound iterations so pathological float steps can never spin.
    for _ in 0..64 {
        if tick > max + step * 1e-9 {
            break;
        }
        // Snap near-zero artifacts of the ceil/multiply round trip.
        ticks.push(if tick.abs() < step * 1e-9 { 0.0 } else { tick });
        tick += step;
    }
    ticks
}

/// The number of fraction digits a step size needs to stay unambiguous.
pub fn decimals_for_step(step: f64) -> usize {
    if !step.is_finite() || step <= 0.0 {
        return 0;
    }
    (-step.log10().floor()).max(0.0) as usize
}

const SECOND_MS: i64 = 1_000;
const MINUTE_MS: i64 = 60 * SECOND_MS;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const DAY_MS: i64 = 24 * HOUR_MS;

const TIME_STEPS_MS: [i64; 24] = [
    SECOND_MS,
    2 * SECOND_MS,
    5 * SECOND_MS,
    10 * SECOND_MS,
    15 * SECOND_MS,
    30 * SECOND_MS,
    MINUTE_MS,
    2 * MINUTE_MS,
    5 * MINUTE_MS,
    10 * MINUTE_MS,
    15 * MINUTE_MS,
    30 * MINUTE_MS,
    HOUR_MS,
    2 * HOUR_MS,
    3 * HOUR_MS,
    6 * HOUR_MS,
    12 * HOUR_MS,
    DAY_MS,
    2 * DAY_MS,
    7 * DAY_MS,
    14 * DAY_MS,
    30 * DAY_MS,
    90 * DAY_MS,
    365 * DAY_MS,
];

/// Time ticks at natural boundaries (whole seconds, minutes, hours, UTC
/// days), returned with the chosen step so labels can match its granularity.
pub fn time_ticks(start_ms: i64, end_ms: i64, target: usize) -> (Vec<i64>, i64) {
    if end_ms <= start_ms {
        return (Vec::new(), SECOND_MS);
    }
    let span = end_ms - start_ms;
    let target = target.max(1) as i64;
    let step = TIME_STEPS_MS
        .iter()
        .copied()
        .find(|step| span / step <= target)
        .unwrap_or(365 * DAY_MS);
    let mut ticks = Vec::new();
    let mut tick = start_ms.div_euclid(step) * step;
    if tick < start_ms {
        tick += step;
    }
    while tick <= end_ms && ticks.len() < 64 {
        ticks.push(tick);
        tick += step;
    }
    (ticks, step)
}

/// A tick label at the granularity of `step_ms`: seconds under a minute,
/// `HH:MM` under a day, `Mon DD` under a year, else the year.
pub fn time_tick_label(ms: i64, step_ms: i64) -> String {
    let Some(time) = DateTime::<Utc>::from_timestamp_millis(ms) else {
        return String::new();
    };
    if step_ms < MINUTE_MS {
        time.format("%H:%M:%S").to_string()
    } else if step_ms < DAY_MS {
        time.format("%H:%M").to_string()
    } else if step_ms < 365 * DAY_MS {
        time.format("%b %d").to_string()
    } else {
        time.year().to_string()
    }
}

/// A full timestamp for hover readouts, at crosshair precision.
pub fn time_readout_label(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|time| time.format("%b %d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// The pointer state a frame exposes to layers when the cursor is inside the
/// plot rect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlotHover {
    pub position: Point<Pixels>,
    pub time_ms: f64,
    pub value: f64,
}

/// Everything a layer needs to draw: absolute bounds, the data→pixel
/// scales, the optional sub-pane sharing the time axis, resolved tokens, and
/// the hover state.
pub struct PlotFrame {
    pub bounds: Bounds<Pixels>,
    pub plot_bounds: Bounds<Pixels>,
    pub sub_pane_bounds: Option<Bounds<Pixels>>,
    pub x: LinearScale,
    pub y: LinearScale,
    pub tokens: MarketTokens,
    pub number_font: Font,
    pub hover: Option<PlotHover>,
}

impl PlotFrame {
    pub fn x_at(&self, time_ms: f64) -> Pixels {
        px(self.x.map(time_ms))
    }

    pub fn y_at(&self, value: f64) -> Pixels {
        px(self.y.map(value))
    }
}

type PlotLayer = Box<dyn Fn(&PlotFrame, &mut Window, &mut App) + 'static>;
type ValueFormatter = Box<dyn Fn(f64) -> String + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlotXAxis {
    TimeMillis,
    Numeric { decimals: usize },
}

/// The canvas plot base: axes, grid, tick labels, transforms, crosshair, and
/// caller-supplied layers drawn through the frame. Fixed window with
/// caller-computed domains; charts autoscale by choosing the domains.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct Plot {
    x_domain: (f64, f64),
    y_domain: (f64, f64),
    margins: PlotMargins,
    width: f32,
    height: f32,
    sub_pane_fraction: Option<f32>,
    x_tick_target: usize,
    y_tick_target: usize,
    crosshair: bool,
    x_axis: PlotXAxis,
    value_formatter: Option<ValueFormatter>,
    layers: Vec<PlotLayer>,
    tokens: Option<MarketTokens>,
}

const SUB_PANE_GAP: f32 = 6.0;

impl Plot {
    /// `x_domain` is a millisecond time window; `y_domain` the value window.
    pub fn new(x_domain: (f64, f64), y_domain: (f64, f64)) -> Self {
        Self {
            x_domain,
            y_domain,
            margins: PlotMargins::default(),
            width: 560.0,
            height: 280.0,
            sub_pane_fraction: None,
            x_tick_target: 6,
            y_tick_target: 5,
            crosshair: true,
            x_axis: PlotXAxis::TimeMillis,
            value_formatter: None,
            layers: Vec::new(),
            tokens: None,
        }
    }

    pub fn margins(mut self, margins: PlotMargins) -> Self {
        self.margins = margins;
        self
    }

    pub fn plot_size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Reserves the bottom `fraction` of the inner rect as a sub-pane that
    /// shares the time axis (volume, oscillators); the value scale then
    /// covers only the main pane.
    pub fn sub_pane(mut self, fraction: f32) -> Self {
        self.sub_pane_fraction = Some(fraction.clamp(0.05, 0.5));
        self
    }

    pub fn tick_targets(mut self, x: usize, y: usize) -> Self {
        self.x_tick_target = x;
        self.y_tick_target = y;
        self
    }

    pub fn crosshair(mut self, crosshair: bool) -> Self {
        self.crosshair = crosshair;
        self
    }

    /// Uses 1-2-5 numeric ticks on the horizontal axis instead of the default
    /// millisecond time ticks. Depth, histogram, calibration, and heatmap
    /// clients share this path rather than forking the kernel.
    pub fn numeric_x_axis(mut self, decimals: usize) -> Self {
        self.x_axis = PlotXAxis::Numeric { decimals };
        self
    }

    /// Formats value-axis labels and the crosshair value chip; defaults to
    /// grouped decimals at the tick step's precision.
    pub fn value_formatter(mut self, formatter: impl Fn(f64) -> String + 'static) -> Self {
        self.value_formatter = Some(Box::new(formatter));
        self
    }

    /// Adds a drawing layer; layers paint in insertion order, above the grid
    /// and below the crosshair.
    pub fn layer(mut self, layer: impl Fn(&PlotFrame, &mut Window, &mut App) + 'static) -> Self {
        self.layers.push(Box::new(layer));
        self
    }

    /// Overrides the theme tokens; used by the grayscale audit preview.
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

pub(crate) fn stroke_polyline(
    window: &mut Window,
    points: &[Point<Pixels>],
    width: Pixels,
    color: gpui::Hsla,
) {
    if points.len() < 2 {
        return;
    }
    let mut builder = PathBuilder::stroke(width);
    let mut iterator = points.iter();
    if let Some(first) = iterator.next() {
        builder.move_to(*first);
    }
    for position in iterator {
        builder.line_to(*position);
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

pub(crate) fn fill_polygon(window: &mut Window, points: &[Point<Pixels>], color: gpui::Hsla) {
    if points.len() < 3 {
        return;
    }
    let mut builder = PathBuilder::fill();
    let mut points = points.iter();
    if let Some(first) = points.next() {
        builder.move_to(*first);
    }
    for point in points {
        builder.line_to(*point);
    }
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

pub(crate) fn fill_rectangle(
    window: &mut Window,
    top_left: Point<Pixels>,
    bottom_right: Point<Pixels>,
    color: gpui::Hsla,
) {
    fill_polygon(
        window,
        &[
            top_left,
            point(bottom_right.x, top_left.y),
            bottom_right,
            point(top_left.x, bottom_right.y),
        ],
        color,
    );
}

/// A small filled chip with right- or center-anchored text, used for axis
/// readouts where the label needs a background to stay legible over data.
fn paint_axis_chip(
    window: &mut Window,
    cx: &mut App,
    text: String,
    font: Font,
    anchor: Point<Pixels>,
    centered: bool,
    background: gpui::Hsla,
    foreground: gpui::Hsla,
) {
    if text.is_empty() {
        return;
    }
    let font_size = px(10.);
    let run = gpui::TextRun {
        len: text.len(),
        font,
        color: foreground,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(text.into(), font_size, &[run], None);
    let padding = px(4.);
    let chip_width = line.width + padding * 2.;
    let chip_height = px(16.);
    let origin = if centered {
        point(anchor.x - chip_width / 2., anchor.y - chip_height / 2.)
    } else {
        point(anchor.x, anchor.y - chip_height / 2.)
    };
    window.paint_quad(fill(
        Bounds::new(origin, size(chip_width, chip_height)),
        background,
    ));
    let line_height = font_size * 1.4;
    if line
        .paint(
            point(
                origin.x + padding,
                origin.y + (chip_height - line_height) / 2.,
            ),
            line_height,
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        )
        .is_err()
    {
        log::warn!("plot axis chip failed to paint");
    }
}

impl RenderOnce for Plot {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let number_font = market_number_font(cx);
        let margins = self.margins;
        let x_domain = self.x_domain;
        let y_domain = self.y_domain;
        let sub_pane_fraction = self.sub_pane_fraction;
        let x_tick_target = self.x_tick_target;
        let y_tick_target = self.y_tick_target;
        let crosshair = self.crosshair;
        let x_axis = self.x_axis;
        let value_formatter = self.value_formatter;
        let layers = self.layers;

        div()
            .w(px(self.width))
            .h(px(self.height))
            // The crosshair follows the pointer, so hover repaints; leaving
            // the plot clears it on the last move inside these bounds.
            .on_mouse_move(|_, window, _| window.refresh())
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let plot_left = bounds.origin.x + px(margins.left);
                        let plot_right = bounds.origin.x + bounds.size.width - px(margins.right);
                        let plot_top = bounds.origin.y + px(margins.top);
                        let plot_bottom = bounds.origin.y + bounds.size.height - px(margins.bottom);
                        if plot_right <= plot_left || plot_bottom <= plot_top {
                            return;
                        }
                        let inner = Bounds::from_corners(
                            point(plot_left, plot_top),
                            point(plot_right, plot_bottom),
                        );
                        let (main_bottom, sub_pane_bounds) = match sub_pane_fraction {
                            Some(fraction) => {
                                let sub_height = f32::from(inner.size.height) * fraction;
                                let sub_top = plot_bottom - px(sub_height);
                                (
                                    sub_top - px(SUB_PANE_GAP),
                                    Some(Bounds::from_corners(
                                        point(plot_left, sub_top),
                                        point(plot_right, plot_bottom),
                                    )),
                                )
                            }
                            None => (plot_bottom, None),
                        };
                        let plot_bounds = Bounds::from_corners(
                            point(plot_left, plot_top),
                            point(plot_right, main_bottom),
                        );

                        let x_scale = LinearScale::new(
                            x_domain,
                            (f32::from(plot_left), f32::from(plot_right)),
                        );
                        let y_scale = LinearScale::new(
                            y_domain,
                            (f32::from(main_bottom), f32::from(plot_top)),
                        );

                        let mouse = window.mouse_position();
                        let hover =
                            (crosshair && plot_bounds.contains(&mouse)).then(|| PlotHover {
                                position: mouse,
                                time_ms: x_scale.invert(f32::from(mouse.x)),
                                value: y_scale.invert(f32::from(mouse.y)),
                            });

                        let frame = PlotFrame {
                            bounds,
                            plot_bounds,
                            sub_pane_bounds,
                            x: x_scale,
                            y: y_scale,
                            tokens,
                            number_font: number_font.clone(),
                            hover,
                        };

                        let y_ticks = nice_ticks(y_domain.0, y_domain.1, y_tick_target);
                        let y_step = nice_step(y_domain.1 - y_domain.0, y_tick_target);
                        let y_decimals = decimals_for_step(y_step);
                        let format_value = |value: f64| match &value_formatter {
                            Some(formatter) => formatter(value),
                            None => format_with_decimals(value, y_decimals),
                        };
                        let x_ticks: Vec<(f64, String)> = match x_axis {
                            PlotXAxis::TimeMillis => {
                                let (ticks, step) =
                                    time_ticks(x_domain.0 as i64, x_domain.1 as i64, x_tick_target);
                                ticks
                                    .into_iter()
                                    .map(|tick| (tick as f64, time_tick_label(tick, step)))
                                    .collect()
                            }
                            PlotXAxis::Numeric { decimals } => {
                                nice_ticks(x_domain.0, x_domain.1, x_tick_target)
                                    .into_iter()
                                    .map(|tick| (tick, format_with_decimals(tick, decimals)))
                                    .collect()
                            }
                        };

                        // Grid under everything.
                        for tick in &y_ticks {
                            let y = frame.y_at(*tick);
                            stroke_polyline(
                                window,
                                &[point(plot_left, y), point(plot_right, y)],
                                px(1.),
                                tokens.grid,
                            );
                        }
                        for (tick, _) in &x_ticks {
                            let x = frame.x_at(*tick);
                            stroke_polyline(
                                window,
                                &[point(x, plot_top), point(x, plot_bottom)],
                                px(1.),
                                tokens.grid,
                            );
                        }

                        // Axis labels in the margins.
                        for tick in &y_ticks {
                            let mut text = SceneText::new(px(10.));
                            text.push(&format_value(*tick), number_font.clone(), tokens.muted);
                            text.paint(
                                window,
                                cx,
                                SceneTextAnchor::Left,
                                point(plot_right + px(6.), frame.y_at(*tick)),
                            );
                        }
                        for (tick, label) in &x_ticks {
                            let mut text = SceneText::new(px(10.));
                            text.push(label, number_font.clone(), tokens.muted);
                            text.paint(
                                window,
                                cx,
                                SceneTextAnchor::Center,
                                point(frame.x_at(*tick), plot_bottom + px(10.)),
                            );
                        }

                        for layer in &layers {
                            layer(&frame, window, cx);
                        }

                        if let Some(hover) = frame.hover {
                            let dash = Some((px(3.), px(3.)));
                            crate::components::viz::stroke_line(
                                window,
                                point(plot_left, hover.position.y),
                                point(plot_right, hover.position.y),
                                px(1.),
                                dash,
                                tokens.muted,
                            );
                            crate::components::viz::stroke_line(
                                window,
                                point(hover.position.x, plot_top),
                                point(hover.position.x, plot_bottom),
                                px(1.),
                                dash,
                                tokens.muted,
                            );
                            paint_axis_chip(
                                window,
                                cx,
                                format_value(hover.value),
                                number_font.clone(),
                                point(plot_right + px(2.), hover.position.y),
                                false,
                                tokens.surface,
                                tokens.text,
                            );
                            paint_axis_chip(
                                window,
                                cx,
                                match x_axis {
                                    PlotXAxis::TimeMillis => {
                                        time_readout_label(hover.time_ms as i64)
                                    }
                                    PlotXAxis::Numeric { decimals } => {
                                        format_with_decimals(hover.time_ms, decimals)
                                    }
                                },
                                number_font.clone(),
                                point(hover.position.x, plot_bottom + px(10.)),
                                true,
                                tokens.surface,
                                tokens.text,
                            );
                        }
                    },
                )
                .size_full(),
            )
    }
}

impl Component for Plot {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let start_ms = 1_754_700_000_000i64;
        let demo_series = move |frame: &PlotFrame, window: &mut Window, _: &mut App| {
            let color = frame.tokens.up;
            let points: Vec<Point<Pixels>> = (0..=120)
                .map(|index| {
                    let time_ms = start_ms as f64 + index as f64 * 60_000.0;
                    let value = 104.0 + 3.0 * (index as f64 / 9.0).sin() + index as f64 * 0.02;
                    point(frame.x_at(time_ms), frame.y_at(value))
                })
                .collect();
            stroke_polyline(window, &points, px(1.5), color);
        };
        let demo_sub_pane = move |frame: &PlotFrame, window: &mut Window, _: &mut App| {
            let Some(sub_pane) = frame.sub_pane_bounds else {
                return;
            };
            for index in (0..120).step_by(4) {
                let time_ms = start_ms as f64 + index as f64 * 60_000.0;
                let magnitude = 0.2 + 0.8 * ((index as f64 / 5.0).cos().abs());
                let x = frame.x_at(time_ms);
                let bar_top = sub_pane.origin.y + sub_pane.size.height * (1.0 - magnitude as f32);
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(x - px(2.), bar_top),
                        point(x + px(2.), sub_pane.origin.y + sub_pane.size.height),
                    ),
                    frame.tokens.muted.opacity(0.4),
                ));
            }
        };

        let plot = |tokens: Option<MarketTokens>| {
            let mut plot = Plot::new(
                (start_ms as f64, start_ms as f64 + 120.0 * 60_000.0),
                (100.0, 110.0),
            )
            .sub_pane(0.2)
            .layer(demo_series)
            .layer(demo_sub_pane);
            if let Some(tokens) = tokens {
                plot = plot.tokens(tokens);
            }
            plot
        };

        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Plot kernel",
                vec![single_example(
                    "Axes, 1-2-5 value ticks, natural time boundaries, a line \
                     layer, a sub-pane sharing the time axis, and a hover \
                     crosshair with axis readouts",
                    plot(None).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Structure reads without hue",
                    plot(Some(MarketTokens::from_theme(cx).grayscale())).into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn linear_scale_maps_and_inverts() {
        let scale = LinearScale::new((0.0, 100.0), (10.0, 210.0));
        assert_eq!(scale.map(0.0), 10.0);
        assert_eq!(scale.map(100.0), 210.0);
        assert_eq!(scale.map(50.0), 110.0);
        assert!(close(scale.invert(110.0), 50.0));
        // Inverted ranges (value axes) work the same way.
        let y_axis = LinearScale::new((100.0, 110.0), (200.0, 0.0));
        assert_eq!(y_axis.map(100.0), 200.0);
        assert_eq!(y_axis.map(110.0), 0.0);
        assert!(close(y_axis.invert(100.0), 105.0));
    }

    #[test]
    fn degenerate_domains_do_not_divide_by_zero() {
        let scale = LinearScale::new((5.0, 5.0), (0.0, 100.0));
        assert_eq!(scale.map(5.0), 50.0);
        assert!(scale.map(999.0).is_finite());
        assert!(scale.invert(0.0).is_finite());
        let nan = LinearScale::new((f64::NAN, 5.0), (0.0, 100.0));
        assert!(nan.map(1.0).is_finite());
    }

    #[test]
    fn nice_steps_follow_one_two_five() {
        assert_eq!(nice_step(10.0, 10), 1.0);
        assert_eq!(nice_step(10.0, 5), 2.0);
        assert_eq!(nice_step(10.0, 2), 5.0);
        // Raw 25 rounds up within its decade to 50.
        assert_eq!(nice_step(100.0, 4), 50.0);
        // Raw 0.25 rounds up within its decade to 0.5.
        assert_eq!(nice_step(1.0, 4), 0.5);
        assert_eq!(nice_step(0.01, 5), 0.002);
    }

    #[test]
    fn nice_ticks_land_on_round_values_and_cover_the_domain() {
        let ticks = nice_ticks(102.3, 109.8, 5);
        assert_eq!(ticks, vec![104.0, 106.0, 108.0]);
        let ticks = nice_ticks(-1.2, 1.2, 5);
        assert_eq!(ticks, vec![-1.0, -0.5, 0.0, 0.5, 1.0]);
        assert!(nice_ticks(5.0, 5.0, 5).is_empty());
        assert!(nice_ticks(f64::NAN, 1.0, 5).is_empty());
    }

    #[test]
    fn decimals_match_step_precision() {
        assert_eq!(decimals_for_step(10.0), 0);
        assert_eq!(decimals_for_step(1.0), 0);
        assert_eq!(decimals_for_step(0.5), 1);
        assert_eq!(decimals_for_step(0.25), 1); // log10(0.25) floor = -1
        assert_eq!(decimals_for_step(0.01), 2);
        assert_eq!(decimals_for_step(0.0), 0);
    }

    #[test]
    fn time_ticks_land_on_natural_boundaries() {
        // A 30-minute window: five-minute boundaries.
        let start = 1_754_700_000_000i64; // 2025-08-09 00:40:00 UTC
        let (ticks, step) = time_ticks(start, start + 30 * MINUTE_MS, 6);
        assert_eq!(step, 5 * MINUTE_MS);
        assert!(!ticks.is_empty());
        for tick in &ticks {
            assert_eq!(tick % (5 * MINUTE_MS), 0, "tick {tick} not on a boundary");
        }
        assert!(*ticks.first().unwrap_or(&0) >= start);
        assert!(*ticks.last().unwrap_or(&i64::MAX) <= start + 30 * MINUTE_MS);

        // A two-hour window: 30-minute boundaries.
        let (_, step) = time_ticks(start, start + 2 * HOUR_MS, 5);
        assert_eq!(step, 30 * MINUTE_MS);

        // A week: day boundaries (UTC midnights).
        let (ticks, step) = time_ticks(start, start + 7 * DAY_MS, 8);
        assert_eq!(step, DAY_MS);
        for tick in &ticks {
            assert_eq!(tick % DAY_MS, 0);
        }

        let (empty, _) = time_ticks(start, start, 6);
        assert!(empty.is_empty());
    }

    #[test]
    fn time_labels_match_step_granularity() {
        let ms = 1_754_700_000_000i64;
        assert_eq!(time_tick_label(ms, 15 * SECOND_MS), "00:40:00");
        assert_eq!(time_tick_label(ms, 5 * MINUTE_MS), "00:40");
        assert_eq!(time_tick_label(ms, DAY_MS), "Aug 09");
        assert_eq!(time_tick_label(ms, 365 * DAY_MS), "2025");
        assert_eq!(time_readout_label(ms), "Aug 09 00:40:00");
    }
}
