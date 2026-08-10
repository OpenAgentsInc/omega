//! Funding-rate history with sign bars, settlements, and EMA overlay.

use std::sync::Arc;

use documented::Documented;
use gpui::{Point, point, px};

use crate::components::viz::{
    MarketTokens, Plot, PlotFrame, fill_rectangle, format_signed_percent, stroke_line,
    stroke_polyline,
};
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FundingPoint {
    pub time_ms: i64,
    /// Funding as a fraction, so `0.0001` is one basis point.
    pub rate: f64,
    pub settlement: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FundingSeries {
    points: Vec<FundingPoint>,
}

impl FundingSeries {
    pub fn new(points: Vec<FundingPoint>) -> Self {
        Self { points }
    }

    pub fn points(&self) -> &[FundingPoint] {
        &self.points
    }

    pub fn domains(&self) -> Option<((f64, f64), (f64, f64))> {
        let first = self.points.first()?;
        let last = self.points.last()?;
        if last.time_ms <= first.time_ms {
            return None;
        }
        let mut minimum = 0.0f64;
        let mut maximum = 0.0f64;
        for point in &self.points {
            if point.rate.is_finite() {
                minimum = minimum.min(point.rate);
                maximum = maximum.max(point.rate);
            }
        }
        let span = (maximum - minimum).max(1e-6);
        Some((
            (first.time_ms as f64, last.time_ms as f64),
            (minimum - span * 0.08, maximum + span * 0.08),
        ))
    }

    pub fn ema(&self, period: usize) -> Vec<(i64, f64)> {
        let alpha = 2.0 / (period.max(1) as f64 + 1.0);
        let mut average: Option<f64> = None;
        self.points
            .iter()
            .filter(|point| point.rate.is_finite())
            .map(|point| {
                let next = average
                    .map(|previous| previous + alpha * (point.rate - previous))
                    .unwrap_or(point.rate);
                average = Some(next);
                (point.time_ms, next)
            })
            .collect()
    }
}

pub trait FundingSeriesSource {
    fn funding_series(&self) -> FundingSeries;
}

pub struct DemoFundingSeriesSource;

impl FundingSeriesSource for DemoFundingSeriesSource {
    fn funding_series(&self) -> FundingSeries {
        let start = 1_754_700_000_000i64;
        FundingSeries::new(
            (0..96)
                .map(|index| FundingPoint {
                    time_ms: start + index * 3_600_000,
                    rate: (index as f64 / 7.0).sin() * 0.00018
                        + (index as f64 / 19.0).cos() * 0.00005,
                    settlement: index % 8 == 0,
                })
                .collect(),
        )
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Funding bars and history line with settlement ticks and an EMA.
pub struct FundingChart {
    series: FundingSeries,
    ema_period: usize,
    width: f32,
    height: f32,
    tokens: Option<MarketTokens>,
}

impl FundingChart {
    pub fn new(series: FundingSeries) -> Self {
        Self {
            series,
            ema_period: 12,
            width: 560.0,
            height: 260.0,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl FundingSeriesSource) -> Self {
        Self::new(source.funding_series())
    }

    pub fn ema_period(mut self, period: usize) -> Self {
        self.ema_period = period.max(1);
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

fn visible_line(frame: &PlotFrame, values: &[(i64, f64)]) -> Vec<Point<gpui::Pixels>> {
    let left = f32::from(frame.plot_bounds.origin.x) - 2.0;
    let right = f32::from(frame.plot_bounds.origin.x + frame.plot_bounds.size.width) + 2.0;
    values
        .iter()
        .filter_map(|(time_ms, value)| {
            let x = f32::from(frame.x_at(*time_ms as f64));
            (x >= left && x <= right).then(|| point(px(x), frame.y_at(*value)))
        })
        .collect()
}

impl RenderOnce for FundingChart {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let domains = self.series.domains().unwrap_or(((0.0, 1.0), (-1.0, 1.0)));
        let ema = Arc::new(self.series.ema(self.ema_period));
        let points = Arc::new(self.series.points);
        let mut plot = Plot::new(domains.0, domains.1)
            .plot_size(self.width, self.height)
            .value_formatter(|value| format_signed_percent(value, 3).0)
            .layer(move |frame, window, _| {
                let zero = frame.y_at(0.0);
                let cell_width =
                    f32::from(frame.plot_bounds.size.width) / points.len().max(1) as f32;
                let bar_half = (cell_width * 0.3).clamp(1.0, 5.0);
                let left = f32::from(frame.plot_bounds.origin.x) - bar_half;
                let right =
                    f32::from(frame.plot_bounds.origin.x + frame.plot_bounds.size.width) + bar_half;
                let mut rate_line = Vec::new();
                for value in points.iter() {
                    let x = f32::from(frame.x_at(value.time_ms as f64));
                    if x < left || x > right || !value.rate.is_finite() {
                        continue;
                    }
                    let y = frame.y_at(value.rate);
                    let color = if value.rate >= 0.0 {
                        frame.tokens.up
                    } else {
                        frame.tokens.down
                    };
                    let (top, bottom) = if y < zero { (y, zero) } else { (zero, y) };
                    fill_rectangle(
                        window,
                        point(px(x - bar_half), top),
                        point(px(x + bar_half), bottom),
                        color.opacity(0.24),
                    );
                    rate_line.push(point(px(x), y));
                    if value.settlement {
                        stroke_line(
                            window,
                            point(px(x), frame.plot_bounds.origin.y),
                            point(px(x), frame.plot_bounds.origin.y + px(7.)),
                            px(1.5),
                            None,
                            frame.tokens.text,
                        );
                    }
                }
                stroke_polyline(window, &rate_line, px(1.), frame.tokens.muted);
                let ema_line = visible_line(frame, &ema);
                stroke_polyline(window, &ema_line, px(1.75), frame.tokens.flat);
            });
        if let Some(tokens) = self.tokens {
            plot = plot.tokens(tokens);
        }
        div()
            .debug_selector(|| "market.funding_chart".into())
            .child(plot)
    }
}

impl Component for FundingChart {
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
                "Funding history",
                vec![single_example(
                    "Signed bars, settlements, history line, and EMA",
                    FundingChart::from_source(&DemoFundingSeriesSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Zero baseline and bar direction preserve funding sign",
                    FundingChart::from_source(&DemoFundingSeriesSource)
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
    fn ema_is_finite_and_aligned() {
        let series = DemoFundingSeriesSource.funding_series();
        let ema = series.ema(12);
        assert_eq!(ema.len(), series.points().len());
        assert!(ema.iter().all(|(_, value)| value.is_finite()));
        assert!(series.domains().is_some());
    }
}
