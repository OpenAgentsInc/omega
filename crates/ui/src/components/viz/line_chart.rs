//! Reusable time-series line and area chart.

use std::sync::Arc;

use documented::Documented;
use gpui::{Point, point, px};

use crate::components::viz::{
    MarketTokens, Plot, PlotFrame, fill_polygon, format_with_decimals, stroke_polyline,
};
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinePoint {
    pub time_ms: i64,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineSeries {
    points: Vec<LinePoint>,
    decimals: usize,
}

impl LineSeries {
    pub fn new(points: Vec<LinePoint>, decimals: usize) -> Self {
        Self { points, decimals }
    }

    pub fn points(&self) -> &[LinePoint] {
        &self.points
    }

    pub fn domains(&self) -> Option<((f64, f64), (f64, f64))> {
        let first = self.points.first()?;
        let last = self.points.last()?;
        let mut minimum = f64::INFINITY;
        let mut maximum = f64::NEG_INFINITY;
        for point in &self.points {
            if point.value.is_finite() {
                minimum = minimum.min(point.value);
                maximum = maximum.max(point.value);
            }
        }
        if !minimum.is_finite() || !maximum.is_finite() || last.time_ms <= first.time_ms {
            return None;
        }
        let span = (maximum - minimum).max(maximum.abs() * 1e-6).max(1e-9);
        Some((
            (first.time_ms as f64, last.time_ms as f64),
            (minimum - span * 0.08, maximum + span * 0.08),
        ))
    }
}

pub trait LineSeriesSource {
    fn line_series(&self) -> LineSeries;
}

pub struct DemoLineSeriesSource;

impl LineSeriesSource for DemoLineSeriesSource {
    fn line_series(&self) -> LineSeries {
        let start = 1_754_700_000_000i64;
        let points = (0..120)
            .map(|index| LinePoint {
                time_ms: start + index * 60_000,
                value: 104.0 + (index as f64 / 8.0).sin() * 2.4 + index as f64 * 0.025,
            })
            .collect();
        LineSeries::new(points, 2)
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// A line or filled-area time series on the shared plot kernel.
pub struct LineChart {
    series: LineSeries,
    area: bool,
    width: f32,
    height: f32,
    tokens: Option<MarketTokens>,
}

impl LineChart {
    pub fn new(series: LineSeries) -> Self {
        Self {
            series,
            area: false,
            width: 560.0,
            height: 260.0,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl LineSeriesSource) -> Self {
        Self::new(source.line_series())
    }

    pub fn area(mut self, area: bool) -> Self {
        self.area = area;
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width.max(160.0);
        self.height = height.max(100.0);
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

fn visible_points(frame: &PlotFrame, points: &[LinePoint]) -> Vec<Point<gpui::Pixels>> {
    let left = f32::from(frame.plot_bounds.origin.x) - 2.0;
    let right = f32::from(frame.plot_bounds.origin.x + frame.plot_bounds.size.width) + 2.0;
    points
        .iter()
        .filter(|point| point.value.is_finite())
        .filter_map(|value| {
            let x = f32::from(frame.x_at(value.time_ms as f64));
            (x >= left && x <= right).then(|| point(px(x), frame.y_at(value.value)))
        })
        .collect()
}

impl RenderOnce for LineChart {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let domains = self.series.domains().unwrap_or(((0.0, 1.0), (0.0, 1.0)));
        let decimals = self.series.decimals;
        let points = Arc::new(self.series.points);
        let area = self.area;
        let mut plot = Plot::new(domains.0, domains.1)
            .plot_size(self.width, self.height)
            .value_formatter(move |value| format_with_decimals(value, decimals))
            .layer(move |frame, window, _| {
                let points = visible_points(frame, &points);
                if area && points.len() >= 2 {
                    let mut polygon = Vec::with_capacity(points.len() + 2);
                    let bottom = frame.plot_bounds.origin.y + frame.plot_bounds.size.height;
                    if let Some(first) = points.first() {
                        polygon.push(point(first.x, bottom));
                    }
                    polygon.extend(points.iter().copied());
                    if let Some(last) = points.last() {
                        polygon.push(point(last.x, bottom));
                    }
                    fill_polygon(window, &polygon, frame.tokens.up.opacity(0.14));
                }
                stroke_polyline(window, &points, px(1.5), frame.tokens.up);
            });
        if let Some(tokens) = self.tokens {
            plot = plot.tokens(tokens);
        }
        div()
            .debug_selector(|| "market.line_chart".into())
            .child(plot)
    }
}

impl Component for LineChart {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let series = DemoLineSeriesSource.line_series();
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Line and area",
                vec![
                    single_example("Line", LineChart::new(series.clone()).into_any_element()),
                    single_example(
                        "Area",
                        LineChart::new(series.clone()).area(true).into_any_element(),
                    ),
                ],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Stroke and fill geometry remain distinct without hue",
                    LineChart::new(series)
                        .area(true)
                        .tokens(MarketTokens::from_theme(cx).grayscale())
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
    fn demo_series_has_ordered_finite_domains() {
        let series = DemoLineSeriesSource.line_series();
        let Some((x, y)) = series.domains() else {
            panic!("demo line series must have domains");
        };
        assert!(x.0 < x.1);
        assert!(y.0 < y.1);
        assert_eq!(series.points().len(), 120);
    }
}
