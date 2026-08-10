//! Equity curve with an aligned underwater drawdown pane.

use std::sync::Arc;

use documented::Documented;
use gpui::{Point, point, px};

use crate::components::viz::{
    LinearScale, MarketTokens, Plot, PlotFrame, fill_polygon, format_usd_cents, stroke_polyline,
};
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EquityPoint {
    pub time_ms: i64,
    pub equity_cents: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EquitySeries {
    points: Vec<EquityPoint>,
}

impl EquitySeries {
    pub fn new(points: Vec<EquityPoint>) -> Self {
        Self { points }
    }

    pub fn points(&self) -> &[EquityPoint] {
        &self.points
    }

    pub fn domains(&self) -> Option<((f64, f64), (f64, f64))> {
        let first = self.points.first()?;
        let last = self.points.last()?;
        if last.time_ms <= first.time_ms {
            return None;
        }
        let minimum = self
            .points
            .iter()
            .map(|point| point.equity_cents as f64)
            .fold(f64::INFINITY, f64::min);
        let maximum = self
            .points
            .iter()
            .map(|point| point.equity_cents as f64)
            .fold(f64::NEG_INFINITY, f64::max);
        let span = (maximum - minimum).max(maximum.abs() * 1e-6).max(1.0);
        Some((
            (first.time_ms as f64, last.time_ms as f64),
            (minimum - span * 0.08, maximum + span * 0.08),
        ))
    }

    pub fn drawdowns(&self) -> Vec<(i64, f64)> {
        let mut peak = 0i64;
        self.points
            .iter()
            .map(|point| {
                peak = peak.max(point.equity_cents);
                let drawdown = if peak > 0 {
                    point.equity_cents as f64 / peak as f64 - 1.0
                } else {
                    0.0
                };
                (point.time_ms, drawdown.min(0.0))
            })
            .collect()
    }
}

pub trait EquitySeriesSource {
    fn equity_series(&self) -> EquitySeries;
}

pub struct DemoEquitySeriesSource;

impl EquitySeriesSource for DemoEquitySeriesSource {
    fn equity_series(&self) -> EquitySeries {
        let start = 1_754_700_000_000i64;
        EquitySeries::new(
            (0..120)
                .scan(1_000_000i64, |equity, index| {
                    let change = ((index as f64 / 8.0).sin() * 2_800.0
                        + (index as f64 / 19.0).cos() * 1_100.0
                        + 420.0) as i64;
                    *equity = equity.saturating_add(change);
                    Some(EquityPoint {
                        time_ms: start + index * 3_600_000,
                        equity_cents: *equity,
                    })
                })
                .collect(),
        )
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Ledger-ready equity history plus a shared-time-axis drawdown pane.
pub struct EquityCurve {
    series: EquitySeries,
    width: f32,
    height: f32,
    tokens: Option<MarketTokens>,
}

impl EquityCurve {
    pub fn new(series: EquitySeries) -> Self {
        Self {
            series,
            width: 560.0,
            height: 300.0,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl EquitySeriesSource) -> Self {
        Self::new(source.equity_series())
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width.max(160.0);
        self.height = height.max(120.0);
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

fn visible_equity(frame: &PlotFrame, points: &[EquityPoint]) -> Vec<Point<gpui::Pixels>> {
    let left = f32::from(frame.plot_bounds.origin.x) - 2.0;
    let right = f32::from(frame.plot_bounds.origin.x + frame.plot_bounds.size.width) + 2.0;
    points
        .iter()
        .filter_map(|value| {
            let x = f32::from(frame.x_at(value.time_ms as f64));
            (x >= left && x <= right).then(|| point(px(x), frame.y_at(value.equity_cents as f64)))
        })
        .collect()
}

impl RenderOnce for EquityCurve {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let domains = self.series.domains().unwrap_or(((0.0, 1.0), (0.0, 1.0)));
        let drawdowns = Arc::new(self.series.drawdowns());
        let points = Arc::new(self.series.points);
        let minimum_drawdown = drawdowns
            .iter()
            .map(|(_, value)| *value)
            .fold(0.0, f64::min)
            .min(-0.001);
        let mut plot = Plot::new(domains.0, domains.1)
            .sub_pane(0.25)
            .plot_size(self.width, self.height)
            .value_formatter(|value| format_usd_cents(value.round() as i64))
            .layer(move |frame, window, _| {
                let equity = visible_equity(frame, &points);
                stroke_polyline(window, &equity, px(1.75), frame.tokens.up);
                let Some(pane) = frame.sub_pane_bounds else {
                    return;
                };
                let scale = LinearScale::new(
                    (minimum_drawdown, 0.0),
                    (
                        f32::from(pane.origin.y + pane.size.height),
                        f32::from(pane.origin.y),
                    ),
                );
                let left = f32::from(pane.origin.x) - 2.0;
                let right = f32::from(pane.origin.x + pane.size.width) + 2.0;
                let underwater: Vec<Point<gpui::Pixels>> = drawdowns
                    .iter()
                    .filter_map(|(time_ms, value)| {
                        let x = f32::from(frame.x_at(*time_ms as f64));
                        (x >= left && x <= right).then(|| point(px(x), px(scale.map(*value))))
                    })
                    .collect();
                if underwater.len() >= 2 {
                    let zero = px(scale.map(0.0));
                    let mut area = Vec::with_capacity(underwater.len() + 2);
                    if let Some(first) = underwater.first() {
                        area.push(point(first.x, zero));
                    }
                    area.extend(underwater.iter().copied());
                    if let Some(last) = underwater.last() {
                        area.push(point(last.x, zero));
                    }
                    fill_polygon(window, &area, frame.tokens.down.opacity(0.18));
                    stroke_polyline(window, &underwater, px(1.25), frame.tokens.down);
                }
            });
        if let Some(tokens) = self.tokens {
            plot = plot.tokens(tokens);
        }
        div()
            .debug_selector(|| "market.equity_curve".into())
            .child(plot)
    }
}

impl Component for EquityCurve {
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
                "Equity and drawdown",
                vec![single_example(
                    "Ledger-shaped equity with an underwater sub-pane",
                    EquityCurve::from_source(&DemoEquitySeriesSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Separate pane and underwater fill preserve drawdown",
                    EquityCurve::from_source(&DemoEquitySeriesSource)
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
    fn drawdown_is_never_positive_and_starts_at_zero() {
        let series = DemoEquitySeriesSource.equity_series();
        let drawdowns = series.drawdowns();
        assert_eq!(drawdowns.first().map(|(_, value)| *value), Some(0.0));
        assert!(drawdowns.iter().all(|(_, value)| *value <= 0.0));
        assert!(drawdowns.iter().any(|(_, value)| *value < 0.0));
    }
}
