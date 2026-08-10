//! Return-distribution histogram with signed bins.

use std::sync::Arc;

use documented::Documented;
use gpui::{point, px};

use crate::components::viz::{MarketTokens, Plot, fill_rectangle, format_with_decimals};
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistogramBin {
    pub lower: f64,
    pub upper: f64,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnSeries {
    values: Vec<f64>,
}

impl ReturnSeries {
    pub fn new(values: Vec<f64>) -> Self {
        Self { values }
    }

    pub fn values(&self) -> &[f64] {
        &self.values
    }

    pub fn bins(&self, count: usize) -> Vec<HistogramBin> {
        let finite: Vec<f64> = self
            .values
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .collect();
        let Some(minimum) = finite.iter().copied().reduce(f64::min) else {
            return Vec::new();
        };
        let Some(maximum) = finite.iter().copied().reduce(f64::max) else {
            return Vec::new();
        };
        let count = count.clamp(1, 64);
        let span = (maximum - minimum).max(1e-9);
        let width = span / count as f64;
        let mut totals = vec![0usize; count];
        for value in finite {
            let raw = ((value - minimum) / width).floor() as usize;
            let index = raw.min(count.saturating_sub(1));
            if let Some(total) = totals.get_mut(index) {
                *total = total.saturating_add(1);
            }
        }
        totals
            .into_iter()
            .enumerate()
            .map(|(index, total)| HistogramBin {
                lower: minimum + index as f64 * width,
                upper: minimum + (index + 1) as f64 * width,
                count: total,
            })
            .collect()
    }
}

pub trait ReturnHistogramSource {
    fn return_series(&self) -> ReturnSeries;
}

pub struct DemoReturnHistogramSource;

impl ReturnHistogramSource for DemoReturnHistogramSource {
    fn return_series(&self) -> ReturnSeries {
        ReturnSeries::new(
            (0..240)
                .map(|index| {
                    (index as f64 / 9.0).sin() * 0.013 + (index as f64 / 23.0).cos() * 0.007 + 0.001
                })
                .collect(),
        )
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// A return histogram whose bar side and sign preserve direction without hue.
pub struct ReturnsHistogram {
    series: ReturnSeries,
    bin_count: usize,
    width: f32,
    height: f32,
    tokens: Option<MarketTokens>,
}

impl ReturnsHistogram {
    pub fn new(series: ReturnSeries) -> Self {
        Self {
            series,
            bin_count: 24,
            width: 560.0,
            height: 260.0,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl ReturnHistogramSource) -> Self {
        Self::new(source.return_series())
    }

    pub fn bin_count(mut self, count: usize) -> Self {
        self.bin_count = count.clamp(1, 64);
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

impl RenderOnce for ReturnsHistogram {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let bins = Arc::new(self.series.bins(self.bin_count));
        let x_domain = bins
            .first()
            .zip(bins.last())
            .map(|(first, last)| (first.lower, last.upper))
            .unwrap_or((-0.01, 0.01));
        let maximum = bins.iter().map(|bin| bin.count).max().unwrap_or(1).max(1) as f64;
        let mut plot = Plot::new(x_domain, (0.0, maximum * 1.08))
            .numeric_x_axis(3)
            .plot_size(self.width, self.height)
            .value_formatter(|value| format_with_decimals(value, 0))
            .layer(move |frame, window, _| {
                for bin in bins.iter() {
                    let left = frame.x_at(bin.lower);
                    let right = frame.x_at(bin.upper);
                    if right < frame.plot_bounds.origin.x
                        || left > frame.plot_bounds.origin.x + frame.plot_bounds.size.width
                    {
                        continue;
                    }
                    let top = frame.y_at(bin.count as f64);
                    let bottom = frame.y_at(0.0);
                    let midpoint = (bin.lower + bin.upper) / 2.0;
                    let color = if midpoint >= 0.0 {
                        frame.tokens.up
                    } else {
                        frame.tokens.down
                    };
                    fill_rectangle(
                        window,
                        point(left + px(0.5), top),
                        point(right - px(0.5), bottom),
                        color.opacity(0.48),
                    );
                }
            });
        if let Some(tokens) = self.tokens {
            plot = plot.tokens(tokens);
        }
        div()
            .debug_selector(|| "market.returns_histogram".into())
            .child(plot)
    }
}

impl Component for ReturnsHistogram {
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
                "Returns histogram",
                vec![single_example(
                    "Signed return bins",
                    ReturnsHistogram::from_source(&DemoReturnHistogramSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Zero separates loss and gain bins",
                    ReturnsHistogram::from_source(&DemoReturnHistogramSource)
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
    fn bins_account_for_every_finite_return() {
        let series = DemoReturnHistogramSource.return_series();
        let bins = series.bins(24);
        assert_eq!(
            bins.iter().map(|bin| bin.count).sum::<usize>(),
            series.values().len()
        );
        assert!(bins.windows(2).all(|pair| match pair {
            [first, second] => first.upper <= second.lower,
            _ => true,
        }));
    }
}
