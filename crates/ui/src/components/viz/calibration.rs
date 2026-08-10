//! Forecast calibration curve against the ideal diagonal.

use std::sync::Arc;

use documented::Documented;
use gpui::{point, px};

use crate::components::viz::{MarketTokens, Plot, fill_circle, stroke_line, stroke_polyline};
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibrationObservation {
    pub confidence: f64,
    pub outcome: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibrationBin {
    pub expected: f64,
    pub realized: f64,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationSeries {
    observations: Vec<CalibrationObservation>,
}

impl CalibrationSeries {
    pub fn new(observations: Vec<CalibrationObservation>) -> Self {
        Self { observations }
    }

    pub fn observations(&self) -> &[CalibrationObservation] {
        &self.observations
    }

    pub fn bins(&self, count: usize) -> Vec<CalibrationBin> {
        let count = count.clamp(1, 32);
        let mut totals = vec![(0.0f64, 0usize, 0usize); count];
        for observation in &self.observations {
            if !observation.confidence.is_finite() {
                continue;
            }
            let confidence = observation.confidence.clamp(0.0, 1.0);
            let index = ((confidence * count as f64).floor() as usize).min(count.saturating_sub(1));
            if let Some((sum, outcomes, samples)) = totals.get_mut(index) {
                *sum += confidence;
                *outcomes = outcomes.saturating_add(usize::from(observation.outcome));
                *samples = samples.saturating_add(1);
            }
        }
        totals
            .into_iter()
            .filter_map(|(sum, outcomes, samples)| {
                (samples > 0).then_some(CalibrationBin {
                    expected: sum / samples as f64,
                    realized: outcomes as f64 / samples as f64,
                    count: samples,
                })
            })
            .collect()
    }
}

pub trait CalibrationSource {
    fn calibration_series(&self) -> CalibrationSeries;
}

pub struct DemoCalibrationSource;

impl CalibrationSource for DemoCalibrationSource {
    fn calibration_series(&self) -> CalibrationSeries {
        CalibrationSeries::new(
            (0..400)
                .map(|index| {
                    let confidence = ((index * 37) % 101) as f64 / 100.0;
                    let threshold = ((index * 61 + 17) % 100) as f64 / 100.0;
                    CalibrationObservation {
                        confidence,
                        outcome: threshold < confidence * 0.92 + 0.04,
                    }
                })
                .collect(),
        )
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Binned forecast reliability with a dashed ideal-calibration reference.
pub struct CalibrationChart {
    series: CalibrationSeries,
    bin_count: usize,
    width: f32,
    height: f32,
    tokens: Option<MarketTokens>,
}

impl CalibrationChart {
    pub fn new(series: CalibrationSeries) -> Self {
        Self {
            series,
            bin_count: 10,
            width: 420.0,
            height: 320.0,
            tokens: None,
        }
    }
    pub fn from_source(source: &impl CalibrationSource) -> Self {
        Self::new(source.calibration_series())
    }
    pub fn bin_count(mut self, count: usize) -> Self {
        self.bin_count = count.clamp(1, 32);
        self
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

impl RenderOnce for CalibrationChart {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let bins = Arc::new(self.series.bins(self.bin_count));
        let mut plot = Plot::new((0.0, 1.0), (0.0, 1.0))
            .numeric_x_axis(1)
            .plot_size(self.width, self.height)
            .value_formatter(|value| format!("{:.0}%", value * 100.0))
            .layer(move |frame, window, _| {
                stroke_line(
                    window,
                    point(frame.x_at(0.0), frame.y_at(0.0)),
                    point(frame.x_at(1.0), frame.y_at(1.0)),
                    px(1.0),
                    Some((px(4.0), px(4.0))),
                    frame.tokens.muted,
                );
                let points: Vec<_> = bins
                    .iter()
                    .filter(|bin| bin.expected.is_finite() && bin.realized.is_finite())
                    .map(|bin| point(frame.x_at(bin.expected), frame.y_at(bin.realized)))
                    .collect();
                stroke_polyline(window, &points, px(1.5), frame.tokens.up);
                for (bin, position) in bins.iter().zip(points.iter()) {
                    fill_circle(
                        window,
                        *position,
                        px((bin.count as f32).sqrt().clamp(2.5, 6.0)),
                        frame.tokens.up,
                    );
                }
            });
        if let Some(tokens) = self.tokens {
            plot = plot.tokens(tokens);
        }
        div()
            .debug_selector(|| "market.calibration_chart".into())
            .child(plot)
    }
}

impl Component for CalibrationChart {
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
                "Calibration",
                vec![single_example(
                    "Reliability curve and ideal reference",
                    CalibrationChart::from_source(&DemoCalibrationSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Dashed diagonal distinguishes the reference",
                    CalibrationChart::from_source(&DemoCalibrationSource)
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
    fn bins_account_for_samples_and_stay_in_unit_square() {
        let series = DemoCalibrationSource.calibration_series();
        let bins = series.bins(10);
        assert_eq!(
            bins.iter().map(|bin| bin.count).sum::<usize>(),
            series.observations().len()
        );
        assert!(
            bins.iter()
                .all(|bin| (0.0..=1.0).contains(&bin.expected)
                    && (0.0..=1.0).contains(&bin.realized))
        );
    }
}
