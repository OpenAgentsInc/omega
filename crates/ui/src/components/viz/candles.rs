//! Candlestick + volume chart on the plot kernel (omega#284).
//!
//! OHLC candles and a volume histogram share the kernel's time axis: the
//! candle body spans open→close, the wick spans high→low, and the sub-pane
//! carries max-normalized volume bars. Up/down comes from the shared
//! [`MarketTokens`] semantic colors, reinforced structurally by the open/close
//! geometry so direction survives grayscale. A last-price line and a hover
//! OHLC+volume readout ride on top. Data arrives as a [`CandleSeries`] value;
//! live venues feed the same chart through the [`CandleSource`] trait while a
//! [`DemoCandleSource`] drives the component-library preview.

use std::sync::Arc;

use documented::Documented;
use gpui::{Bounds, Font, Hsla, Pixels, Point, Window, fill, point, px};

use crate::components::viz::{
    MarketTokens, SceneText, SceneTextAnchor, decimals_for_step, format_compact,
    format_with_decimals, stroke_polyline,
};
use crate::components::viz::{Plot, PlotFrame};
use crate::prelude::*;

/// One OHLC bar. Timestamps are millisecond epochs to match the kernel's
/// time axis; prices and volume are plain `f64` in the instrument's units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candle {
    pub time_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// A window of candles plus the price precision its readouts should use. The
/// value type charts render; a [`CandleSource`] produces it.
#[derive(Debug, Clone, PartialEq)]
pub struct CandleSeries {
    candles: Vec<Candle>,
    price_decimals: usize,
}

impl CandleSeries {
    pub fn new(candles: Vec<Candle>, price_decimals: usize) -> Self {
        Self {
            candles,
            price_decimals,
        }
    }

    pub fn candles(&self) -> &[Candle] {
        &self.candles
    }

    pub fn price_decimals(&self) -> usize {
        self.price_decimals
    }

    pub fn is_empty(&self) -> bool {
        self.candles.is_empty()
    }

    pub fn last(&self) -> Option<&Candle> {
        self.candles.last()
    }

    /// The median inter-candle spacing, used to pad the time domain by half a
    /// bar on each side so the first and last candles are not clipped.
    fn step_ms(&self) -> f64 {
        let mut deltas: Vec<i64> = self
            .candles
            .windows(2)
            .map(|pair| pair[1].time_ms - pair[0].time_ms)
            .filter(|delta| *delta > 0)
            .collect();
        if deltas.is_empty() {
            return 60_000.0;
        }
        deltas.sort_unstable();
        deltas[deltas.len() / 2] as f64
    }

    /// Time domain padded by half a bar so edge candles have room.
    pub fn time_domain(&self) -> Option<(f64, f64)> {
        let first = self.candles.first()?;
        let last = self.candles.last()?;
        let pad = self.step_ms() / 2.0;
        Some((first.time_ms as f64 - pad, last.time_ms as f64 + pad))
    }

    /// Price domain over [low, high] of every candle, padded so bodies never
    /// touch the plot edges. A flat series still gets a non-degenerate span.
    pub fn price_domain(&self) -> Option<(f64, f64)> {
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for candle in &self.candles {
            min = min.min(candle.low);
            max = max.max(candle.high);
        }
        if !min.is_finite() || !max.is_finite() {
            return None;
        }
        let span = (max - min).max(max.abs() * 1e-3).max(1e-6);
        let pad = span * 0.06;
        Some((min - pad, max + pad))
    }

    pub fn max_volume(&self) -> f64 {
        self.candles
            .iter()
            .map(|candle| candle.volume)
            .fold(0.0, f64::max)
    }
}

/// The seam live venues implement: hand the chart a [`CandleSeries`]. Kept
/// deliberately small so a plugin can adapt any feed; the demo fixture below
/// is the only in-library implementor.
pub trait CandleSource {
    fn series(&self) -> CandleSeries;
}

/// A deterministic synthetic OHLC series for the component library. Produces a
/// drifting, mean-reverting price with per-bar volume so the chart, volume
/// pane, and hover readout have realistic structure without a live feed.
pub struct DemoCandleSource {
    pub start_ms: i64,
    pub count: usize,
}

impl Default for DemoCandleSource {
    fn default() -> Self {
        Self {
            start_ms: 1_754_700_000_000,
            count: 96,
        }
    }
}

impl CandleSource for DemoCandleSource {
    fn series(&self) -> CandleSeries {
        let step_ms = 5 * 60_000;
        let mut candles = Vec::with_capacity(self.count);
        let mut close = 104.0_f64;
        for index in 0..self.count {
            let time_ms = self.start_ms + index as i64 * step_ms;
            let drift = (index as f64 / 11.0).sin() * 0.9 + (index as f64 / 4.3).cos() * 0.35;
            let open = close;
            close = (open + drift).max(1.0);
            let wick = 0.25 + ((index as f64 / 3.0).sin().abs()) * 0.6;
            let high = open.max(close) + wick;
            let low = open.min(close) - wick;
            let volume = 40.0 + (index as f64 / 6.0).cos().abs() * 120.0 + (index % 7) as f64 * 6.0;
            candles.push(Candle {
                time_ms,
                open,
                high,
                low,
                close,
                volume,
            });
        }
        CandleSeries::new(candles, 2)
    }
}

/// A candlestick chart with an optional volume sub-pane, built on [`Plot`].
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct CandlestickChart {
    series: CandleSeries,
    width: f32,
    height: f32,
    volume: bool,
    tokens: Option<MarketTokens>,
}

impl CandlestickChart {
    pub fn new(series: CandleSeries) -> Self {
        Self {
            series,
            width: 560.0,
            height: 300.0,
            volume: true,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl CandleSource) -> Self {
        Self::new(source.series())
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn volume(mut self, volume: bool) -> Self {
        self.volume = volume;
        self
    }

    /// Overrides the theme tokens; used by the grayscale audit preview.
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

/// A filled rectangle from two corners; the candle bodies and volume bars.
fn fill_rect(
    window: &mut Window,
    top_left: Point<Pixels>,
    bottom_right: Point<Pixels>,
    color: Hsla,
) {
    window.paint_quad(fill(Bounds::from_corners(top_left, bottom_right), color));
}

/// Draws every candle and its volume bar, the last-price line, and the hover
/// OHLC readout, all through the frame transforms.
fn draw_candles(
    frame: &PlotFrame,
    window: &mut Window,
    cx: &mut App,
    candles: &[Candle],
    max_volume: f64,
    price_decimals: usize,
) {
    let plot = frame.plot_bounds;
    let plot_left = f32::from(plot.origin.x);
    let plot_right = f32::from(plot.origin.x + plot.size.width);
    let count = candles.len().max(1) as f32;
    let cell = f32::from(plot.size.width) / count;
    let body_half = (cell * 0.34).clamp(1.0, 8.0);
    let tokens = frame.tokens;

    for candle in candles {
        let center = f32::from(frame.x_at(candle.time_ms as f64));
        // Render only what falls in the plot rect (the visibility law).
        if center < plot_left - body_half - 2.0 || center > plot_right + body_half + 2.0 {
            continue;
        }
        let color = if candle.close >= candle.open {
            tokens.up
        } else {
            tokens.down
        };
        let x = px(center);
        stroke_polyline(
            window,
            &[
                point(x, frame.y_at(candle.high)),
                point(x, frame.y_at(candle.low)),
            ],
            px(1.),
            color,
        );
        let y_open = frame.y_at(candle.open);
        let y_close = frame.y_at(candle.close);
        let top = y_open.min(y_close);
        // Dojis (open==close) still show a one-pixel body so they read.
        let bottom = if f32::from(y_open.max(y_close) - top) < 1.0 {
            top + px(1.)
        } else {
            y_open.max(y_close)
        };
        fill_rect(
            window,
            point(x - px(body_half), top),
            point(x + px(body_half), bottom),
            color,
        );

        if let Some(sub_pane) = frame.sub_pane_bounds {
            if max_volume > 0.0 {
                let fraction = (candle.volume / max_volume).clamp(0.0, 1.0) as f32;
                let bar_bottom = sub_pane.origin.y + sub_pane.size.height;
                let bar_height = sub_pane.size.height * fraction;
                fill_rect(
                    window,
                    point(x - px(body_half), bar_bottom - bar_height),
                    point(x + px(body_half), bar_bottom),
                    color.opacity(0.45),
                );
            }
        }
    }

    if let Some(last) = candles.last() {
        let color = if last.close >= last.open {
            tokens.up
        } else {
            tokens.down
        };
        let y = last.close;
        crate::components::viz::stroke_line(
            window,
            point(plot.origin.x, frame.y_at(y)),
            point(plot.origin.x + plot.size.width, frame.y_at(y)),
            px(1.),
            Some((px(4.), px(3.))),
            color,
        );
        // Last-price chip on the value axis.
        let mut text = SceneText::new(px(10.));
        text.push(
            &format_with_decimals(last.close, price_decimals),
            frame.number_font.clone(),
            tokens.text,
        );
        text.paint(
            window,
            cx,
            SceneTextAnchor::Left,
            point(plot.origin.x + plot.size.width + px(6.), frame.y_at(y)),
        );
    }

    if let Some(hover) = frame.hover {
        draw_hover_readout(frame, window, cx, candles, hover.time_ms, price_decimals);
    }
}

/// The OHLC+volume panel anchored to the plot's top-left corner, keyed to the
/// candle nearest the crosshair.
fn draw_hover_readout(
    frame: &PlotFrame,
    window: &mut Window,
    cx: &mut App,
    candles: &[Candle],
    time_ms: f64,
    price_decimals: usize,
) {
    let Some(candle) = candles.iter().min_by(|a, b| {
        let da = (a.time_ms as f64 - time_ms).abs();
        let db = (b.time_ms as f64 - time_ms).abs();
        da.total_cmp(&db)
    }) else {
        return;
    };
    let tokens = frame.tokens;
    let direction_color = if candle.close >= candle.open {
        tokens.up
    } else {
        tokens.down
    };
    let font = &frame.number_font;
    let origin = point(
        frame.plot_bounds.origin.x + px(6.),
        frame.plot_bounds.origin.y + px(6.),
    );
    let panel = Bounds::from_corners(origin, point(origin.x + px(232.), origin.y + px(34.)));
    window.paint_quad(fill(panel, tokens.surface.opacity(0.92)));

    let field = |label: &str, value: String, color: Hsla, font: &Font| {
        let mut text = SceneText::new(px(10.));
        text.push(&format!("{label} "), font.clone(), tokens.muted);
        text.push(&value, font.clone(), color);
        text
    };
    let mut line_one = field(
        "O",
        format_with_decimals(candle.open, price_decimals),
        tokens.text,
        font,
    );
    line_one.push("  H ", font.clone(), tokens.muted);
    line_one.push(
        &format_with_decimals(candle.high, price_decimals),
        font.clone(),
        tokens.text,
    );
    line_one.push("  L ", font.clone(), tokens.muted);
    line_one.push(
        &format_with_decimals(candle.low, price_decimals),
        font.clone(),
        tokens.text,
    );
    line_one.push("  C ", font.clone(), tokens.muted);
    line_one.push(
        &format_with_decimals(candle.close, price_decimals),
        font.clone(),
        direction_color,
    );
    line_one.paint(
        window,
        cx,
        SceneTextAnchor::Left,
        point(origin.x + px(6.), origin.y + px(10.)),
    );

    let line_two = field("Vol", format_compact(candle.volume), tokens.text, font);
    line_two.paint(
        window,
        cx,
        SceneTextAnchor::Left,
        point(origin.x + px(6.), origin.y + px(24.)),
    );
}

impl RenderOnce for CandlestickChart {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let Some(time_domain) = self.series.time_domain() else {
            return div()
                .w(px(self.width))
                .h(px(self.height))
                .into_any_element();
        };
        let Some(price_domain) = self.series.price_domain() else {
            return div()
                .w(px(self.width))
                .h(px(self.height))
                .into_any_element();
        };
        let price_decimals = self.series.price_decimals();
        let value_decimals =
            decimals_for_step((price_domain.1 - price_domain.0) / 5.0).max(price_decimals.min(2));
        let candles: Arc<Vec<Candle>> = Arc::new(self.series.candles().to_vec());
        let max_volume = self.series.max_volume();

        let mut plot = Plot::new(time_domain, price_domain)
            .plot_size(self.width, self.height)
            .tokens(tokens)
            .value_formatter(move |value| format_with_decimals(value, value_decimals))
            .layer(move |frame, window, cx| {
                draw_candles(frame, window, cx, &candles, max_volume, price_decimals);
            });
        if self.volume {
            plot = plot.sub_pane(0.22);
        }
        plot.into_any_element()
    }
}

impl Component for CandlestickChart {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let series = DemoCandleSource::default().series();
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Candlestick + volume",
                vec![single_example(
                    "OHLC candles with wick+body, a max-normalized volume \
                     sub-pane on the shared time axis, a dashed last-price \
                     line, and a hover OHLC+volume readout",
                    CandlestickChart::new(series.clone()).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Direction still reads from open/close geometry without hue",
                    CandlestickChart::new(series)
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
    fn demo_source_produces_a_valid_series() {
        let series = DemoCandleSource::default().series();
        assert!(!series.is_empty());
        for candle in series.candles() {
            assert!(candle.high >= candle.low);
            assert!(candle.high >= candle.open && candle.high >= candle.close);
            assert!(candle.low <= candle.open && candle.low <= candle.close);
            assert!(candle.volume >= 0.0);
        }
        assert!(series.max_volume() > 0.0);
    }

    #[test]
    fn domains_are_ordered_and_padded() {
        let series = DemoCandleSource::default().series();
        let (t0, t1) = series.time_domain().expect("time domain");
        assert!(t1 > t0);
        let (p0, p1) = series.price_domain().expect("price domain");
        assert!(p1 > p0);
        // The padded price domain strictly contains the raw extremes.
        let raw_low = series
            .candles()
            .iter()
            .map(|candle| candle.low)
            .fold(f64::INFINITY, f64::min);
        let raw_high = series
            .candles()
            .iter()
            .map(|candle| candle.high)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(p0 < raw_low && p1 > raw_high);
    }

    #[test]
    fn empty_series_has_no_domains() {
        let series = CandleSeries::new(Vec::new(), 2);
        assert!(series.time_domain().is_none());
        assert!(series.price_domain().is_none());
        assert!(series.last().is_none());
    }

    #[test]
    fn flat_series_still_gets_a_span() {
        let candles = vec![
            Candle {
                time_ms: 0,
                open: 5.0,
                high: 5.0,
                low: 5.0,
                close: 5.0,
                volume: 1.0,
            },
            Candle {
                time_ms: 60_000,
                open: 5.0,
                high: 5.0,
                low: 5.0,
                close: 5.0,
                volume: 1.0,
            },
        ];
        let series = CandleSeries::new(candles, 2);
        let (p0, p1) = series.price_domain().expect("price domain");
        assert!(p1 > p0);
    }
}
