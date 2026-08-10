//! Dense matrix heatmap with signed, tabular cell labels.

use std::sync::Arc;

use documented::Documented;
use gpui::{point, px};

use crate::components::viz::{MarketTokens, Plot, SceneText, SceneTextAnchor, fill_rectangle};
use crate::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct HeatmapData {
    rows: usize,
    columns: usize,
    values: Vec<f64>,
}

impl HeatmapData {
    pub fn new(rows: usize, columns: usize, values: Vec<f64>) -> Self {
        Self {
            rows,
            columns,
            values,
        }
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn columns(&self) -> usize {
        self.columns
    }
    pub fn value(&self, row: usize, column: usize) -> Option<f64> {
        if row >= self.rows || column >= self.columns {
            return None;
        }
        row.checked_mul(self.columns)
            .and_then(|offset| offset.checked_add(column))
            .and_then(|index| self.values.get(index))
            .copied()
    }
    pub fn is_complete(&self) -> bool {
        self.rows.checked_mul(self.columns) == Some(self.values.len())
    }
}

pub trait HeatmapSource {
    fn heatmap(&self) -> HeatmapData;
}

pub struct DemoHeatmapSource;

impl HeatmapSource for DemoHeatmapSource {
    fn heatmap(&self) -> HeatmapData {
        let side = 8usize;
        HeatmapData::new(
            side,
            side,
            (0..side)
                .flat_map(|row| {
                    (0..side).map(move |column| {
                        if row == column {
                            1.0
                        } else {
                            ((row as f64 - column as f64) * 0.72).cos() * 0.75
                        }
                    })
                })
                .collect(),
        )
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// A source-driven signed matrix rendered entirely on the GPUI canvas.
pub struct Heatmap {
    data: HeatmapData,
    width: f32,
    height: f32,
    tokens: Option<MarketTokens>,
}

impl Heatmap {
    pub fn new(data: HeatmapData) -> Self {
        Self {
            data,
            width: 440.0,
            height: 360.0,
            tokens: None,
        }
    }
    pub fn from_source(source: &impl HeatmapSource) -> Self {
        Self::new(source.heatmap())
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

impl RenderOnce for Heatmap {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let rows = self.data.rows().max(1);
        let columns = self.data.columns().max(1);
        let data = Arc::new(self.data);
        let mut plot = Plot::new((0.0, columns as f64), (0.0, rows as f64))
            .numeric_x_axis(0)
            .plot_size(self.width, self.height)
            .value_formatter(|value| format!("{value:.0}"))
            .layer(move |frame, window, cx| {
                for row in 0..rows {
                    for column in 0..columns {
                        let Some(value) = data.value(row, column).filter(|value| value.is_finite())
                        else {
                            continue;
                        };
                        let left = frame.x_at(column as f64);
                        let right = frame.x_at((column + 1) as f64);
                        let top = frame.y_at((row + 1) as f64);
                        let bottom = frame.y_at(row as f64);
                        if right < frame.plot_bounds.origin.x
                            || left > frame.plot_bounds.origin.x + frame.plot_bounds.size.width
                            || bottom < frame.plot_bounds.origin.y
                            || top > frame.plot_bounds.origin.y + frame.plot_bounds.size.height
                        {
                            continue;
                        }
                        let magnitude = value.abs().clamp(0.0, 1.0) as f32;
                        let color = if value >= 0.0 {
                            frame.tokens.up
                        } else {
                            frame.tokens.down
                        };
                        fill_rectangle(
                            window,
                            point(left + px(0.5), top + px(0.5)),
                            point(right - px(0.5), bottom - px(0.5)),
                            color.opacity(0.12 + magnitude * 0.62),
                        );
                        if f32::from(right - left) >= 28.0 && f32::from(bottom - top) >= 18.0 {
                            let label = format!("{value:+.2}");
                            let mut text = SceneText::new(px(9.0));
                            text.push(&label, frame.number_font.clone(), frame.tokens.text);
                            text.paint(
                                window,
                                cx,
                                SceneTextAnchor::Center,
                                point((left + right) / 2.0, (top + bottom) / 2.0),
                            );
                        }
                    }
                }
            });
        if let Some(tokens) = self.tokens {
            plot = plot.tokens(tokens);
        }
        div().debug_selector(|| "market.heatmap".into()).child(plot)
    }
}

impl Component for Heatmap {
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
                "Heatmap",
                vec![single_example(
                    "Signed correlation matrix",
                    Heatmap::from_source(&DemoHeatmapSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Intensity and signed axis readout survive without hue",
                    Heatmap::from_source(&DemoHeatmapSource)
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
    fn matrix_access_is_checked_and_complete() {
        let data = DemoHeatmapSource.heatmap();
        assert!(data.is_complete());
        assert_eq!(data.value(0, 0), Some(1.0));
        assert_eq!(data.value(data.rows(), 0), None);
        assert_eq!(data.value(0, data.columns()), None);
    }
}
