//! Renderers for engine-computed indicator overlays and oscillator panes.

use std::sync::Arc;

use documented::Documented;
use gpui::{Point, point, px};

use crate::components::viz::{
    MarketTokens, Plot, PlotFrame, fill_polygon, format_with_decimals, stroke_polyline,
};
use crate::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnalyticsPoint {
    pub time_ms: i64,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnalyticsLine {
    pub label: SharedString,
    pub points: Vec<AnalyticsPoint>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnalyticsBand {
    pub upper: AnalyticsLine,
    pub middle: Option<AnalyticsLine>,
    pub lower: AnalyticsLine,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndicatorOverlayData {
    pub price: AnalyticsLine,
    pub moving_averages: Vec<AnalyticsLine>,
    pub bollinger: Option<AnalyticsBand>,
    pub keltner: Option<AnalyticsBand>,
    pub donchian: Option<AnalyticsBand>,
    pub ichimoku: Option<AnalyticsBand>,
    pub vwap: Option<AnalyticsLine>,
    pub decimals: usize,
}

pub trait IndicatorOverlaySource {
    fn indicator_overlays(&self) -> IndicatorOverlayData;
}

pub struct DemoIndicatorOverlaySource;

fn demo_line(label: &'static str, offset: f64, scale: f64) -> AnalyticsLine {
    let start = 1_754_700_000_000i64;
    AnalyticsLine {
        label: label.into(),
        points: (0..96)
            .map(|index| AnalyticsPoint {
                time_ms: start + index * 300_000,
                value: 104.0 + (index as f64 / scale).sin() * 2.1 + index as f64 * 0.018 + offset,
            })
            .collect(),
    }
}

fn demo_band(label: &'static str, spread: f64, scale: f64) -> AnalyticsBand {
    AnalyticsBand {
        upper: demo_line(label, spread, scale),
        middle: Some(demo_line(label, 0.0, scale)),
        lower: demo_line(label, -spread, scale),
    }
}

impl IndicatorOverlaySource for DemoIndicatorOverlaySource {
    fn indicator_overlays(&self) -> IndicatorOverlayData {
        IndicatorOverlayData {
            price: demo_line("Price", 0.0, 7.0),
            moving_averages: vec![
                demo_line("EMA 20", -0.25, 9.0),
                demo_line("SMA 50", -0.55, 13.0),
            ],
            bollinger: Some(demo_band("Bollinger", 1.35, 9.0)),
            keltner: Some(demo_band("Keltner", 0.95, 11.0)),
            donchian: Some(demo_band("Donchian", 1.75, 15.0)),
            ichimoku: Some(demo_band("Ichimoku", 0.65, 17.0)),
            vwap: Some(demo_line("VWAP", -0.1, 12.0)),
            decimals: 2,
        }
    }
}

fn line_points(frame: &PlotFrame, line: &AnalyticsLine) -> Vec<Point<gpui::Pixels>> {
    let left = f32::from(frame.plot_bounds.origin.x) - 2.0;
    let right = f32::from(frame.plot_bounds.origin.x + frame.plot_bounds.size.width) + 2.0;
    line.points
        .iter()
        .filter(|point| point.value.is_finite())
        .filter_map(|value| {
            let x = f32::from(frame.x_at(value.time_ms as f64));
            (x >= left && x <= right).then(|| point(px(x), frame.y_at(value.value)))
        })
        .collect()
}

fn line_domain(lines: impl Iterator<Item = AnalyticsLine>) -> ((f64, f64), (f64, f64)) {
    let mut first_time = i64::MAX;
    let mut last_time = i64::MIN;
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for line in lines {
        for point in line.points {
            if point.value.is_finite() {
                first_time = first_time.min(point.time_ms);
                last_time = last_time.max(point.time_ms);
                minimum = minimum.min(point.value);
                maximum = maximum.max(point.value);
            }
        }
    }
    if first_time >= last_time || !minimum.is_finite() || !maximum.is_finite() {
        return ((0.0, 1.0), (0.0, 1.0));
    }
    let padding = (maximum - minimum).max(1e-6) * 0.08;
    (
        (first_time as f64, last_time as f64),
        (minimum - padding, maximum + padding),
    )
}

fn band_lines(band: &Option<AnalyticsBand>) -> impl Iterator<Item = AnalyticsLine> + '_ {
    band.iter().flat_map(|band| {
        [
            Some(band.upper.clone()),
            band.middle.clone(),
            Some(band.lower.clone()),
        ]
        .into_iter()
        .flatten()
    })
}

fn paint_band(frame: &PlotFrame, window: &mut Window, band: &AnalyticsBand, color: gpui::Hsla) {
    let upper = line_points(frame, &band.upper);
    let lower = line_points(frame, &band.lower);
    if upper.len() >= 2 && lower.len() >= 2 {
        let mut polygon = Vec::with_capacity(upper.len() + lower.len());
        polygon.extend(upper.iter().copied());
        polygon.extend(lower.iter().rev().copied());
        fill_polygon(window, &polygon, color.opacity(0.055));
    }
    stroke_polyline(window, &upper, px(1.0), color.opacity(0.7));
    stroke_polyline(window, &lower, px(1.0), color.opacity(0.7));
    if let Some(middle) = &band.middle {
        stroke_polyline(
            window,
            &line_points(frame, middle),
            px(0.75),
            color.opacity(0.45),
        );
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Moving averages, volatility/channel bands, Ichimoku cloud, and VWAP over engine price values.
pub struct IndicatorOverlayChart {
    data: IndicatorOverlayData,
    width: f32,
    height: f32,
    tokens: Option<MarketTokens>,
}

impl IndicatorOverlayChart {
    pub fn new(data: IndicatorOverlayData) -> Self {
        Self {
            data,
            width: 620.0,
            height: 320.0,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl IndicatorOverlaySource) -> Self {
        Self::new(source.indicator_overlays())
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width.max(200.0);
        self.height = height.max(140.0);
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for IndicatorOverlayChart {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let all_lines = std::iter::once(self.data.price.clone())
            .chain(self.data.moving_averages.iter().cloned())
            .chain(band_lines(&self.data.bollinger))
            .chain(band_lines(&self.data.keltner))
            .chain(band_lines(&self.data.donchian))
            .chain(band_lines(&self.data.ichimoku))
            .chain(self.data.vwap.iter().cloned());
        let domains = line_domain(all_lines);
        let data = Arc::new(self.data);
        let decimals = data.decimals;
        let mut plot = Plot::new(domains.0, domains.1)
            .plot_size(self.width, self.height)
            .value_formatter(move |value| format_with_decimals(value, decimals))
            .layer(move |frame, window, _| {
                stroke_polyline(
                    window,
                    &line_points(frame, &data.price),
                    px(1.75),
                    frame.tokens.text,
                );
                for (index, average) in data.moving_averages.iter().enumerate() {
                    let color = if index % 2 == 0 {
                        frame.tokens.up
                    } else {
                        frame.tokens.down
                    };
                    stroke_polyline(window, &line_points(frame, average), px(1.2), color);
                }
                if let Some(band) = &data.bollinger {
                    paint_band(frame, window, band, frame.tokens.up);
                }
                if let Some(band) = &data.keltner {
                    paint_band(frame, window, band, frame.tokens.down);
                }
                if let Some(band) = &data.donchian {
                    paint_band(frame, window, band, frame.tokens.flat);
                }
                if let Some(band) = &data.ichimoku {
                    paint_band(frame, window, band, frame.tokens.up.opacity(0.72));
                }
                if let Some(vwap) = &data.vwap {
                    stroke_polyline(
                        window,
                        &line_points(frame, vwap),
                        px(2.2),
                        frame.tokens.down,
                    );
                }
            });
        if let Some(tokens) = self.tokens {
            plot = plot.tokens(tokens);
        }
        div()
            .debug_selector(|| "market.indicator_overlays".into())
            .child(plot)
    }
}

impl Component for IndicatorOverlayChart {
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
                "Indicator overlays",
                vec![single_example(
                    "MA family, Bollinger, Keltner, Donchian, Ichimoku, and VWAP",
                    IndicatorOverlayChart::from_source(&DemoIndicatorOverlaySource)
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Stroke widths and band geometry preserve every layer",
                    IndicatorOverlayChart::from_source(&DemoIndicatorOverlaySource)
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OscillatorKind {
    Rsi,
    Macd,
    Stochastics,
    Cci,
    Atr,
}

impl OscillatorKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rsi => "RSI",
            Self::Macd => "MACD",
            Self::Stochastics => "Stochastics",
            Self::Cci => "CCI",
            Self::Atr => "ATR",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OscillatorPaneData {
    pub kind: OscillatorKind,
    pub domain: (f64, f64),
    pub reference_lines: Vec<f64>,
    pub series: Vec<AnalyticsLine>,
}

pub trait OscillatorPaneSource {
    fn oscillator_panes(&self) -> Vec<OscillatorPaneData>;
}

pub struct DemoOscillatorPaneSource;

impl OscillatorPaneSource for DemoOscillatorPaneSource {
    fn oscillator_panes(&self) -> Vec<OscillatorPaneData> {
        let line = |label, center: f64, amplitude: f64, scale: f64| {
            let mut line = demo_line(label, 0.0, scale);
            for (index, point) in line.points.iter_mut().enumerate() {
                point.value = center + (index as f64 / scale).sin() * amplitude;
            }
            line
        };
        vec![
            OscillatorPaneData {
                kind: OscillatorKind::Rsi,
                domain: (0.0, 100.0),
                reference_lines: vec![30.0, 70.0],
                series: vec![line("RSI", 52.0, 28.0, 7.0)],
            },
            OscillatorPaneData {
                kind: OscillatorKind::Macd,
                domain: (-3.0, 3.0),
                reference_lines: vec![0.0],
                series: vec![line("MACD", 0.0, 2.0, 8.0), line("Signal", 0.0, 1.45, 10.0)],
            },
            OscillatorPaneData {
                kind: OscillatorKind::Stochastics,
                domain: (0.0, 100.0),
                reference_lines: vec![20.0, 80.0],
                series: vec![line("%K", 50.0, 39.0, 5.0), line("%D", 50.0, 31.0, 7.0)],
            },
            OscillatorPaneData {
                kind: OscillatorKind::Cci,
                domain: (-240.0, 240.0),
                reference_lines: vec![-100.0, 0.0, 100.0],
                series: vec![line("CCI", 0.0, 180.0, 6.0)],
            },
            OscillatorPaneData {
                kind: OscillatorKind::Atr,
                domain: (0.0, 5.0),
                reference_lines: Vec::new(),
                series: vec![line("ATR", 2.2, 1.1, 12.0)],
            },
        ]
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// RSI, MACD, Stochastics, CCI, and ATR panes sharing one engine time domain.
pub struct OscillatorStack {
    panes: Vec<OscillatorPaneData>,
    width: f32,
    tokens: Option<MarketTokens>,
}

impl OscillatorStack {
    pub fn new(panes: Vec<OscillatorPaneData>) -> Self {
        Self {
            panes,
            width: 620.0,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl OscillatorPaneSource) -> Self {
        Self::new(source.oscillator_panes())
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(200.0);
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for OscillatorStack {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let shared_time = line_domain(
            self.panes
                .iter()
                .flat_map(|pane| pane.series.iter().cloned()),
        )
        .0;
        let tokens = self.tokens;
        let width = self.width;
        v_flex()
            .debug_selector(|| "market.oscillator_stack".into())
            .gap_1()
            .children(self.panes.into_iter().map(move |pane| {
                let kind = pane.kind;
                let series = Arc::new(pane.series);
                let references = pane.reference_lines;
                let mut plot = Plot::new(shared_time, pane.domain)
                    .plot_size(width, 112.0)
                    .tick_targets(5, 3)
                    .value_formatter(|value| format_with_decimals(value, 1))
                    .layer(move |frame, window, _| {
                        for reference in &references {
                            crate::components::viz::stroke_line(
                                window,
                                point(frame.plot_bounds.origin.x, frame.y_at(*reference)),
                                point(
                                    frame.plot_bounds.origin.x + frame.plot_bounds.size.width,
                                    frame.y_at(*reference),
                                ),
                                px(1.0),
                                Some((px(3.0), px(3.0))),
                                frame.tokens.grid,
                            );
                        }
                        for (index, line) in series.iter().enumerate() {
                            let color = if index % 2 == 0 {
                                frame.tokens.up
                            } else {
                                frame.tokens.down
                            };
                            stroke_polyline(window, &line_points(frame, line), px(1.25), color);
                        }
                    });
                if let Some(tokens) = tokens {
                    plot = plot.tokens(tokens);
                }
                v_flex()
                    .child(
                        Label::new(kind.label())
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(plot)
            }))
    }
}

impl Component for OscillatorStack {
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
                "Oscillator sub-panes",
                vec![single_example(
                    "Engine-computed values share one time domain",
                    OscillatorStack::from_source(&DemoOscillatorPaneSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Reference levels and multiple strokes remain visible",
                    OscillatorStack::from_source(&DemoOscillatorPaneSource)
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
    fn demo_covers_every_requested_indicator_and_oscillator() {
        let overlays = DemoIndicatorOverlaySource.indicator_overlays();
        assert_eq!(overlays.moving_averages.len(), 2);
        assert!(overlays.bollinger.is_some());
        assert!(overlays.keltner.is_some());
        assert!(overlays.donchian.is_some());
        assert!(overlays.ichimoku.is_some());
        assert!(overlays.vwap.is_some());
        let panes = DemoOscillatorPaneSource.oscillator_panes();
        assert_eq!(panes.len(), 5);
        assert_eq!(
            panes.first().map(|pane| pane.kind),
            Some(OscillatorKind::Rsi)
        );
        assert_eq!(
            panes.last().map(|pane| pane.kind),
            Some(OscillatorKind::Atr)
        );
    }
}
