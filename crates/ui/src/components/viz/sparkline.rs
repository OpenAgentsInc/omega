//! Tiny inline series built from the plot kernel's scales and path painter.

use documented::Documented;
use gpui::{PathBuilder, Point, canvas, point, px};

use crate::components::viz::{
    LinearScale, MarketDirection, MarketTokens, format_with_decimals, market_number_font,
    stroke_polyline,
};
use crate::prelude::*;

const MAX_VISIBLE_POINTS: usize = 256;

#[derive(IntoElement, RegisterComponent, Documented)]
/// Compact line series for cards, tables, and watchlists.
pub struct Sparkline {
    values: Vec<f64>,
    width: f32,
    height: f32,
    decimals: usize,
    tokens: Option<MarketTokens>,
}

impl Sparkline {
    pub fn new(values: impl IntoIterator<Item = f64>) -> Self {
        Self {
            values: values
                .into_iter()
                .filter(|value| value.is_finite())
                .collect(),
            width: 128.0,
            height: 32.0,
            decimals: 2,
            tokens: None,
        }
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width.max(40.0);
        self.height = height.max(16.0);
        self
    }

    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

fn visible_samples(values: &[f64]) -> Vec<f64> {
    if values.len() <= MAX_VISIBLE_POINTS {
        return values.to_vec();
    }
    let step = (values.len() - 1) as f64 / (MAX_VISIBLE_POINTS - 1) as f64;
    (0..MAX_VISIBLE_POINTS)
        .filter_map(|index| values.get((index as f64 * step).round() as usize).copied())
        .collect()
}

impl RenderOnce for Sparkline {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let samples = visible_samples(&self.values);
        let first = samples.first().copied();
        let last = samples.last().copied();
        let direction = match (first, last) {
            (Some(first), Some(last)) => MarketDirection::of_f64(last - first),
            _ => MarketDirection::Flat,
        };
        let color = tokens.direction_color(direction);
        let value_text = last
            .map(|value| format_with_decimals(value, self.decimals))
            .unwrap_or_else(|| "—".to_string());

        h_flex()
            .debug_selector(|| "market.sparkline".into())
            .gap_1()
            .child(
                div().w(px(self.width)).h(px(self.height)).child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            if samples.len() < 2 {
                                return;
                            }
                            let minimum = samples.iter().copied().fold(f64::INFINITY, f64::min);
                            let maximum = samples.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                            let span = (maximum - minimum).max(maximum.abs() * 1e-6).max(1e-9);
                            let y = LinearScale::new(
                                (minimum - span * 0.08, maximum + span * 0.08),
                                (
                                    f32::from(bounds.origin.y + bounds.size.height),
                                    f32::from(bounds.origin.y),
                                ),
                            );
                            let x = LinearScale::new(
                                (0.0, (samples.len() - 1) as f64),
                                (
                                    f32::from(bounds.origin.x),
                                    f32::from(bounds.origin.x + bounds.size.width),
                                ),
                            );
                            let points: Vec<Point<gpui::Pixels>> = samples
                                .iter()
                                .enumerate()
                                .map(|(index, value)| {
                                    point(px(x.map(index as f64)), px(y.map(*value)))
                                })
                                .collect();
                            stroke_polyline(window, &points, px(1.5), color);
                            if let Some(endpoint) = points.last() {
                                let radius = px(2.25);
                                let mut builder = PathBuilder::fill();
                                builder.move_to(point(endpoint.x + radius, endpoint.y));
                                builder.arc_to(
                                    point(radius, radius),
                                    px(0.),
                                    false,
                                    true,
                                    point(endpoint.x - radius, endpoint.y),
                                );
                                builder.arc_to(
                                    point(radius, radius),
                                    px(0.),
                                    false,
                                    true,
                                    point(endpoint.x + radius, endpoint.y),
                                );
                                builder.close();
                                if let Ok(path) = builder.build() {
                                    window.paint_path(path, color);
                                }
                            }
                        },
                    )
                    .size_full(),
                ),
            )
            .child(
                div()
                    .font(market_number_font(cx))
                    .text_size(px(10.))
                    .text_color(color)
                    .child(format!("{} {value_text}", direction.glyph())),
            )
    }
}

impl Component for Sparkline {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let series = |descending: bool| {
            (0..96).map(move |index| {
                let trend = if descending {
                    -(index as f64)
                } else {
                    index as f64
                } * 0.035;
                104.0 + trend + (index as f64 / 6.0).sin() * 1.2
            })
        };
        let row = |tokens: Option<MarketTokens>| {
            h_flex()
                .gap_4()
                .children([false, true].into_iter().map(|descending| {
                    let mut sparkline = Sparkline::new(series(descending));
                    if let Some(tokens) = tokens {
                        sparkline = sparkline.tokens(tokens);
                    }
                    sparkline
                }))
                .into_any_element()
        };
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Sparklines",
                vec![single_example("Inline up and down series", row(None))],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Geometry, endpoint, and direction glyph carry the trend",
                    row(Some(MarketTokens::from_theme(cx).grayscale())),
                )],
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_caps_work_and_preserves_endpoints() {
        let values: Vec<f64> = (0..10_000).map(|value| value as f64).collect();
        let samples = visible_samples(&values);
        assert_eq!(samples.len(), MAX_VISIBLE_POINTS);
        assert_eq!(samples.first(), Some(&0.0));
        assert_eq!(samples.last(), Some(&9_999.0));
    }
}
