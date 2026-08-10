//! Cumulative bid/ask depth chart derived from an order-book snapshot.

use std::sync::Arc;

use documented::Documented;
use gpui::{Point, point, px};

use crate::components::viz::{
    DemoBookSource, MarketTokens, OrderBook, Plot, PlotFrame, fill_polygon, format_with_decimals,
    stroke_line, stroke_polyline,
};
use crate::prelude::*;

pub trait DepthChartSource {
    fn depth_book(&self) -> OrderBook;
}

impl DepthChartSource for DemoBookSource {
    fn depth_book(&self) -> OrderBook {
        DemoBookSource::at_tick(8)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DepthSeries {
    bids: Vec<(f64, f64)>,
    asks: Vec<(f64, f64)>,
    mid: f64,
    price_decimals: usize,
    size_decimals: usize,
}

impl DepthSeries {
    pub fn from_book(book: &OrderBook) -> Option<Self> {
        let mid = book.mid()?;
        let cumulative = |levels: &[crate::BookLevel]| {
            let mut total = 0.0;
            levels
                .iter()
                .filter(|level| level.price.is_finite() && level.size.is_finite())
                .map(|level| {
                    total += level.size.max(0.0);
                    (level.price, total)
                })
                .collect::<Vec<_>>()
        };
        let mut bids = cumulative(&book.bids);
        bids.reverse();
        let asks = cumulative(&book.asks);
        if bids.is_empty() || asks.is_empty() {
            return None;
        }
        Some(Self {
            bids,
            asks,
            mid,
            price_decimals: book.price_decimals,
            size_decimals: book.size_decimals,
        })
    }

    pub fn domains(&self) -> Option<((f64, f64), (f64, f64))> {
        let minimum_price = self.bids.first()?.0;
        let maximum_price = self.asks.last()?.0;
        let maximum_size = self
            .bids
            .iter()
            .chain(&self.asks)
            .map(|(_, size)| *size)
            .fold(0.0, f64::max);
        (minimum_price < maximum_price && maximum_size > 0.0)
            .then_some(((minimum_price, maximum_price), (0.0, maximum_size * 1.08)))
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Bid/ask cumulative depth with a mid marker and hoverable size axis.
pub struct DepthChart {
    series: Option<DepthSeries>,
    width: f32,
    height: f32,
    tokens: Option<MarketTokens>,
}

impl DepthChart {
    pub fn new(book: OrderBook) -> Self {
        Self {
            series: DepthSeries::from_book(&book),
            width: 560.0,
            height: 260.0,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl DepthChartSource) -> Self {
        Self::new(source.depth_book())
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

fn screen_points(frame: &PlotFrame, values: &[(f64, f64)]) -> Vec<Point<gpui::Pixels>> {
    let left = f32::from(frame.plot_bounds.origin.x) - 2.0;
    let right = f32::from(frame.plot_bounds.origin.x + frame.plot_bounds.size.width) + 2.0;
    values
        .iter()
        .filter_map(|(price, size)| {
            let x = f32::from(frame.x_at(*price));
            (x >= left && x <= right).then(|| point(px(x), frame.y_at(*size)))
        })
        .collect()
}

impl RenderOnce for DepthChart {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let series = self.series.unwrap_or(DepthSeries {
            bids: Vec::new(),
            asks: Vec::new(),
            mid: 0.5,
            price_decimals: 2,
            size_decimals: 2,
        });
        let domains = series.domains().unwrap_or(((0.0, 1.0), (0.0, 1.0)));
        let bids = Arc::new(series.bids);
        let asks = Arc::new(series.asks);
        let mid = series.mid;
        let price_decimals = series.price_decimals;
        let size_decimals = series.size_decimals;
        let mut plot = Plot::new(domains.0, domains.1)
            .numeric_x_axis(price_decimals)
            .plot_size(self.width, self.height)
            .value_formatter(move |value| format_with_decimals(value, size_decimals))
            .layer(move |frame, window, _| {
                let bottom = frame.plot_bounds.origin.y + frame.plot_bounds.size.height;
                for (values, color) in [(&bids, frame.tokens.up), (&asks, frame.tokens.down)] {
                    let points = screen_points(frame, values);
                    if points.len() >= 2 {
                        let mut area = Vec::with_capacity(points.len() + 2);
                        if let Some(first) = points.first() {
                            area.push(point(first.x, bottom));
                        }
                        area.extend(points.iter().copied());
                        if let Some(last) = points.last() {
                            area.push(point(last.x, bottom));
                        }
                        fill_polygon(window, &area, color.opacity(0.14));
                        stroke_polyline(window, &points, px(1.5), color);
                    }
                }
                let x = frame.x_at(mid);
                stroke_line(
                    window,
                    point(x, frame.plot_bounds.origin.y),
                    point(x, bottom),
                    px(1.),
                    None,
                    frame.tokens.flat,
                );
            });
        if let Some(tokens) = self.tokens {
            plot = plot.tokens(tokens);
        }
        div()
            .debug_selector(|| "market.depth_chart".into())
            .child(plot)
    }
}

impl Component for DepthChart {
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
                "Depth chart",
                vec![single_example(
                    "Cumulative bids and asks with mid-price marker",
                    DepthChart::from_source(&DemoBookSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Left/right geometry and mid marker preserve book sides",
                    DepthChart::from_source(&DemoBookSource)
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
    fn cumulative_depth_grows_away_from_mid() {
        let Some(series) = DepthSeries::from_book(&DemoBookSource::at_tick(4)) else {
            panic!("demo book must produce depth");
        };
        assert!(
            series
                .bids
                .first()
                .is_some_and(|first| { series.bids.last().is_some_and(|last| first.1 > last.1) })
        );
        assert!(series.asks.windows(2).all(|pair| match pair {
            [first, second] => first.1 <= second.1,
            _ => true,
        }));
        assert!(series.domains().is_some());
    }
}
