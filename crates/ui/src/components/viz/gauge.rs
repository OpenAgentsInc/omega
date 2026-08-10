//! Threshold-aware market gauge shared by mandate, rate-budget, and book views.

use documented::Documented;
use gpui::{Bounds, PathBuilder, Pixels, Point, canvas, point, px};

use crate::components::viz::{MarketTokens, market_number_font, stroke_polyline};
use crate::prelude::*;

pub const DEFAULT_WARN_FRACTION: f32 = 0.7;
pub const DEFAULT_CRITICAL_FRACTION: f32 = 0.9;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeterZone {
    Safe,
    Warn,
    Critical,
}

impl MeterZone {
    pub fn from_fraction(fraction: f32) -> Self {
        Self::with_thresholds(fraction, DEFAULT_WARN_FRACTION, DEFAULT_CRITICAL_FRACTION)
    }

    pub fn with_thresholds(fraction: f32, warn: f32, critical: f32) -> Self {
        if fraction >= critical {
            Self::Critical
        } else if fraction >= warn {
            Self::Warn
        } else {
            Self::Safe
        }
    }
}

#[derive(Clone, Debug)]
pub struct HeadroomMeter {
    pub label: SharedString,
    pub used_display: SharedString,
    pub limit_display: SharedString,
    pub fraction: f32,
}

impl HeadroomMeter {
    pub fn zone(&self) -> MeterZone {
        MeterZone::from_fraction(self.fraction)
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// A threshold bar with safe, warning, and critical zones.
pub struct Gauge {
    meter: HeadroomMeter,
    warn_fraction: f32,
    critical_fraction: f32,
    width: f32,
    tokens: Option<MarketTokens>,
}

impl Gauge {
    pub fn new(meter: HeadroomMeter) -> Self {
        Self {
            meter,
            warn_fraction: DEFAULT_WARN_FRACTION,
            critical_fraction: DEFAULT_CRITICAL_FRACTION,
            width: 360.0,
            tokens: None,
        }
    }

    pub fn thresholds(mut self, warn_fraction: f32, critical_fraction: f32) -> Self {
        self.warn_fraction = warn_fraction.clamp(0.0, 1.0);
        self.critical_fraction = critical_fraction.clamp(self.warn_fraction, 1.0);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(80.0);
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

fn fill_rectangle(window: &mut Window, bounds: Bounds<Pixels>, color: gpui::Hsla) {
    let left = bounds.origin.x;
    let top = bounds.origin.y;
    let right = left + bounds.size.width;
    let bottom = top + bounds.size.height;
    let mut builder = PathBuilder::fill();
    builder.move_to(point(left, top));
    builder.line_to(point(right, top));
    builder.line_to(point(right, bottom));
    builder.line_to(point(left, bottom));
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

impl RenderOnce for Gauge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let fill_fraction = self.meter.fraction.clamp(0.0, 1.0);
        let zone = MeterZone::with_thresholds(
            self.meter.fraction,
            self.warn_fraction,
            self.critical_fraction,
        );
        let fill_color = match zone {
            MeterZone::Safe => tokens.up,
            MeterZone::Warn => tokens.down.opacity(0.65),
            MeterZone::Critical => tokens.down,
        };
        let warn_fraction = self.warn_fraction;
        let critical_fraction = self.critical_fraction;
        let number_font = market_number_font(cx);

        v_flex()
            .debug_selector(|| "market.gauge".into())
            .w(px(self.width))
            .gap_0p5()
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(Label::new(self.meter.label).size(LabelSize::XSmall))
                    .child(
                        div()
                            .font(number_font)
                            .text_size(px(11.))
                            .text_color(tokens.muted)
                            .child(format!(
                                "{} / {}",
                                self.meter.used_display, self.meter.limit_display
                            )),
                    ),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        fill_rectangle(window, bounds, tokens.grid.opacity(0.45));
                        let fill_bounds = Bounds::new(
                            bounds.origin,
                            gpui::size(bounds.size.width * fill_fraction, bounds.size.height),
                        );
                        fill_rectangle(window, fill_bounds, fill_color);
                        for threshold in [warn_fraction, critical_fraction] {
                            let x = bounds.origin.x + bounds.size.width * threshold;
                            stroke_polyline(
                                window,
                                &[
                                    Point::new(x, bounds.origin.y),
                                    Point::new(x, bounds.origin.y + bounds.size.height),
                                ],
                                px(1.),
                                tokens.text.opacity(0.7),
                            );
                        }
                    },
                )
                .w_full()
                .h(px(7.)),
            )
    }
}

impl Component for Gauge {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let examples = |tokens: Option<MarketTokens>| {
            [
                ("Order rate", "18/h", "60/h", 0.30),
                ("Mandate usage", "$7,600", "$10,000", 0.76),
                ("Liquidation buffer", "18 bps", "20 bps", 0.95),
            ]
            .into_iter()
            .map(|(label, used, limit, fraction)| {
                let mut gauge = Gauge::new(HeadroomMeter {
                    label: label.into(),
                    used_display: used.into(),
                    limit_display: limit.into(),
                    fraction,
                });
                if let Some(tokens) = tokens {
                    gauge = gauge.tokens(tokens);
                }
                gauge
            })
            .collect::<Vec<_>>()
        };
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Threshold gauges",
                vec![single_example(
                    "Safe, warning, and critical zones with threshold ticks",
                    v_flex().gap_2().children(examples(None)).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Labels, fill position, and ticks preserve the state",
                    v_flex()
                        .gap_2()
                        .children(examples(Some(MarketTokens::from_theme(cx).grayscale())))
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
    fn zones_split_at_shared_thresholds() {
        assert_eq!(MeterZone::from_fraction(0.69), MeterZone::Safe);
        assert_eq!(MeterZone::from_fraction(0.7), MeterZone::Warn);
        assert_eq!(MeterZone::from_fraction(0.9), MeterZone::Critical);
    }
}
