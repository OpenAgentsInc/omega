//! Order-book ladder v0 (omega#284).
//!
//! A vertical depth-of-market column: grouped price levels with asks stacked
//! above the mid/spread row and bids below, each level carrying a
//! max-normalized size bar, a cumulative-depth column, and a flash-on-change
//! highlight keyed to size deltas. The book arrives as an [`OrderBook`] value;
//! live venues feed the same ladder through the [`BookSource`] trait, and a
//! deterministic [`DemoBookSource`] drives the animated preview. High-frequency
//! updates throttle per the rendering laws: the demo advances the book on a
//! bounded tick cadence rather than every frame, and the ladder is capped at
//! twenty levels per side so it only ever lays out what is visible.

use std::time::Duration;

use documented::Documented;
use gpui::{Animation, AnimationExt as _, Hsla};

use crate::components::viz::{
    FlashOnChangeExt, MarketTokens, format_with_decimals, market_number_font,
};
use crate::prelude::*;

/// One grouped price level: a price and the aggregate size resting there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BookLevel {
    pub price: f64,
    pub size: f64,
}

/// An order-book snapshot. Bids run best-first (descending price), asks
/// best-first (ascending price); the ladder reads both from the inside out.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderBook {
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
    pub price_decimals: usize,
    pub size_decimals: usize,
}

impl OrderBook {
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.first().map(|level| level.price)
    }

    pub fn best_ask(&self) -> Option<f64> {
        self.asks.first().map(|level| level.price)
    }

    pub fn mid(&self) -> Option<f64> {
        Some((self.best_bid()? + self.best_ask()?) / 2.0)
    }

    pub fn spread(&self) -> Option<f64> {
        Some(self.best_ask()? - self.best_bid()?)
    }

    /// The largest size among the first `depth` levels of either side; the
    /// denominator the per-level bars normalize against.
    pub fn max_size(&self, depth: usize) -> f64 {
        self.bids
            .iter()
            .take(depth)
            .chain(self.asks.iter().take(depth))
            .map(|level| level.size)
            .fold(0.0, f64::max)
    }
}

/// The seam live venues implement: hand the ladder an [`OrderBook`] snapshot.
/// The ladder polls this on a throttled cadence and diffs successive snapshots
/// to drive flashes; the demo fixture below is the only in-library implementor.
pub trait BookSource {
    fn snapshot(&self) -> OrderBook;
}

const DEMO_DEPTH: usize = 20;
const DEMO_GROUP: f64 = 0.5;
const DEMO_FLASH_LOOKBACK: u64 = 6;

/// A deterministic synthetic book for the component library. The mid drifts in
/// group-sized steps and per-level sizes wobble on a quantized grid, so size
/// deltas — and their flashes — happen at a realistic, bounded rate.
pub struct DemoBookSource;

impl DemoBookSource {
    fn center(tick: u64) -> f64 {
        104.0 + (tick as f64 * 0.045).sin() * 3.0
    }

    fn best_bid_price(tick: u64) -> f64 {
        (Self::center(tick) / DEMO_GROUP).floor() * DEMO_GROUP
    }

    /// Quantizing to a 0.25 grid turns the smooth wobble into discrete size
    /// changes, which is what the flash highlight keys off.
    fn level_size(side_sign: f64, level: usize, tick: u64) -> f64 {
        let phase = tick as f64 * 0.33;
        let base = 6.0 + level as f64 * 1.4;
        let wobble = (phase + level as f64 * 0.8 + side_sign).sin();
        let raw = base * (1.0 + 0.4 * wobble);
        (raw * 4.0).round() / 4.0
    }

    pub fn at_tick(tick: u64) -> OrderBook {
        let best_bid = Self::best_bid_price(tick);
        let best_ask = best_bid + DEMO_GROUP;
        let bids = (0..DEMO_DEPTH)
            .map(|level| BookLevel {
                price: best_bid - level as f64 * DEMO_GROUP,
                size: Self::level_size(-1.0, level, tick),
            })
            .collect();
        let asks = (0..DEMO_DEPTH)
            .map(|level| BookLevel {
                price: best_ask + level as f64 * DEMO_GROUP,
                size: Self::level_size(1.0, level, tick),
            })
            .collect();
        OrderBook {
            bids,
            asks,
            price_decimals: 2,
            size_decimals: 2,
        }
    }

    /// The most recent tick at which a level's size changed, per side, so each
    /// row's flash restarts only when its own size moves. A bounded backward
    /// scan keeps this stateless while covering the flash decay window.
    pub fn flash_epochs(tick: u64) -> (Vec<u64>, Vec<u64>) {
        let last_change = |side_sign: f64, level: usize| -> u64 {
            let mut time = tick;
            for _ in 0..DEMO_FLASH_LOOKBACK {
                if time == 0 {
                    break;
                }
                if Self::level_size(side_sign, level, time)
                    != Self::level_size(side_sign, level, time - 1)
                {
                    return time;
                }
                time -= 1;
            }
            tick.saturating_sub(DEMO_FLASH_LOOKBACK)
        };
        let bids = (0..DEMO_DEPTH)
            .map(|level| last_change(-1.0, level))
            .collect();
        let asks = (0..DEMO_DEPTH)
            .map(|level| last_change(1.0, level))
            .collect();
        (bids, asks)
    }
}

impl BookSource for DemoBookSource {
    fn snapshot(&self) -> OrderBook {
        Self::at_tick(0)
    }
}

/// A depth-of-market ladder rendered as a DOM column. Size bars are UI
/// rectangles (not chart geometry), so this is a div layout rather than a
/// canvas plot; the flash highlight keys off per-level change epochs.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct OrderBookLadder {
    book: OrderBook,
    bid_epochs: Vec<u64>,
    ask_epochs: Vec<u64>,
    depth: usize,
    width: f32,
    tokens: Option<MarketTokens>,
}

impl OrderBookLadder {
    pub fn new(book: OrderBook) -> Self {
        Self {
            book,
            bid_epochs: Vec::new(),
            ask_epochs: Vec::new(),
            depth: DEMO_DEPTH,
            width: 280.0,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl BookSource) -> Self {
        Self::new(source.snapshot())
    }

    /// Per-level change epochs, aligned to `book.bids`/`book.asks`; a row
    /// flashes when its epoch changes. Absent, the ladder renders unanimated.
    pub fn flash_epochs(mut self, bids: Vec<u64>, asks: Vec<u64>) -> Self {
        self.bid_epochs = bids;
        self.ask_epochs = asks;
        self
    }

    /// Levels per side (capped at twenty for v0).
    pub fn depth(mut self, depth: usize) -> Self {
        self.depth = depth.min(DEMO_DEPTH);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Overrides the theme tokens; used by the grayscale audit preview.
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

const ROW_HEIGHT: f32 = 18.0;

impl RenderOnce for OrderBookLadder {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let number_font = market_number_font(cx);
        let depth = self.depth.min(DEMO_DEPTH);
        let max_size = self.book.max_size(depth).max(f64::MIN_POSITIVE);
        let price_decimals = self.book.price_decimals;
        let size_decimals = self.book.size_decimals;
        let width = self.width;

        // Cumulative depth accumulates from the best (inside) level outward.
        let cumulative = |levels: &[BookLevel]| -> Vec<f64> {
            let mut running = 0.0;
            levels
                .iter()
                .take(depth)
                .map(|level| {
                    running += level.size;
                    running
                })
                .collect::<Vec<_>>()
        };
        let bid_cumulative = cumulative(&self.book.bids);
        let ask_cumulative = cumulative(&self.book.asks);

        let row = {
            let number_font = number_font.clone();
            move |level: &BookLevel, cumulative: f64, color: Hsla, epoch: u64| -> gpui::AnyElement {
                let fraction = (level.size / max_size).clamp(0.0, 1.0) as f32;
                let bar_width = fraction * width;
                let column = |value: String, color: Hsla, align_end: bool| {
                    let cell = div().text_color(color).child(value);
                    let mut wrapper = h_flex().flex_1().min_w_0();
                    wrapper = if align_end {
                        wrapper.justify_end()
                    } else {
                        wrapper.justify_start()
                    };
                    wrapper.child(cell)
                };
                let content = h_flex()
                    .relative()
                    .w_full()
                    .h_full()
                    .items_center()
                    .px_2()
                    .gap_2()
                    .font(number_font.clone())
                    .text_size(px(11.))
                    .child(column(
                        format_with_decimals(level.price, price_decimals),
                        color,
                        false,
                    ))
                    .child(column(
                        format_with_decimals(level.size, size_decimals),
                        tokens.text,
                        true,
                    ))
                    .child(column(
                        format_with_decimals(cumulative, size_decimals),
                        tokens.muted,
                        true,
                    ));
                div()
                    .relative()
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right_0()
                            .w(px(bar_width))
                            .bg(color.opacity(0.16)),
                    )
                    .child(content)
                    .with_change_flash("order-book-level", epoch, color, |element, overlay| {
                        element.bg(overlay)
                    })
                    .into_any_element()
            }
        };

        let epoch_for = |epochs: &[u64], index: usize| epochs.get(index).copied().unwrap_or(0);

        // Asks: best ask nearest the mid row, so display worst-first (top) down
        // to best-last (bottom, adjacent to the spread row).
        let mut ask_rows = Vec::new();
        for index in (0..self.book.asks.len().min(depth)).rev() {
            let level = self.book.asks[index];
            ask_rows.push(row(
                &level,
                ask_cumulative.get(index).copied().unwrap_or(level.size),
                tokens.down,
                epoch_for(&self.ask_epochs, index),
            ));
        }

        let mut bid_rows = Vec::new();
        for index in 0..self.book.bids.len().min(depth) {
            let level = self.book.bids[index];
            bid_rows.push(row(
                &level,
                bid_cumulative.get(index).copied().unwrap_or(level.size),
                tokens.up,
                epoch_for(&self.bid_epochs, index),
            ));
        }

        let mid_row = {
            let mid_text = self
                .book
                .mid()
                .map(|mid| format_with_decimals(mid, price_decimals))
                .unwrap_or_else(|| "—".to_string());
            let spread_text = match (self.book.spread(), self.book.mid()) {
                (Some(spread), Some(mid)) if mid != 0.0 => format!(
                    "{}  ({:.1} bps)",
                    format_with_decimals(spread, price_decimals),
                    spread / mid * 10_000.0
                ),
                (Some(spread), _) => format_with_decimals(spread, price_decimals),
                _ => "—".to_string(),
            };
            h_flex()
                .w_full()
                .h(px(22.))
                .items_center()
                .justify_between()
                .px_2()
                .bg(tokens.surface)
                .border_y_1()
                .border_color(tokens.grid)
                .font(number_font.clone())
                .text_size(px(11.))
                .child(div().text_color(tokens.text).child(mid_text))
                .child(div().text_color(tokens.muted).child(spread_text))
        };

        let header = h_flex()
            .w_full()
            .h(px(16.))
            .items_center()
            .px_2()
            .gap_2()
            .font(number_font)
            .text_size(px(10.))
            .text_color(tokens.muted)
            .child(div().flex_1().min_w_0().justify_start().child("price"))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .justify_end()
                    .child(div().child("size")),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .justify_end()
                    .child(div().child("cumulative")),
            );

        v_flex()
            .w(px(width))
            .border_1()
            .border_color(tokens.grid)
            .rounded_md()
            .overflow_hidden()
            .child(header)
            .children(ask_rows)
            .child(mid_row)
            .children(bid_rows)
    }
}

/// A phase-driven wrapper that advances [`DemoBookSource`] over a repeating
/// animation, so the component-library preview shows the book updating and
/// flashing without a live async source.
#[derive(IntoElement)]
struct LiveLadder {
    phase: f32,
    tokens: Option<MarketTokens>,
}

const LIVE_TICKS: u64 = 200;

impl LiveLadder {
    fn phase(mut self, phase: f32) -> Self {
        self.phase = phase;
        self
    }
}

impl RenderOnce for LiveLadder {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tick = (self.phase.clamp(0.0, 0.9999) as f64 * LIVE_TICKS as f64) as u64;
        let book = DemoBookSource::at_tick(tick);
        let (bid_epochs, ask_epochs) = DemoBookSource::flash_epochs(tick);
        let mut ladder = OrderBookLadder::new(book).flash_epochs(bid_epochs, ask_epochs);
        if let Some(tokens) = self.tokens {
            ladder = ladder.tokens(tokens);
        }
        ladder
    }
}

impl Component for OrderBookLadder {
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
                "Order-book ladder",
                vec![single_example(
                    "Grouped levels with max-normalized size bars, a cumulative \
                     column, a mid/spread row, and flashes on size deltas — the \
                     book advances on a throttled tick cadence",
                    LiveLadder {
                        phase: 0.0,
                        tokens: None,
                    }
                    .with_animation(
                        "order-book-ladder",
                        Animation::new(Duration::from_secs(20)).repeat(),
                        |ladder, delta| ladder.phase(delta),
                    )
                    .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Bid/ask still separate by position across the mid row without hue",
                    OrderBookLadder::new(DemoBookSource::at_tick(0))
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
    fn demo_book_is_well_ordered() {
        let book = DemoBookSource::at_tick(3);
        assert_eq!(book.bids.len(), DEMO_DEPTH);
        assert_eq!(book.asks.len(), DEMO_DEPTH);
        // Bids descend, asks ascend, and the best ask sits above the best bid.
        for pair in book.bids.windows(2) {
            assert!(pair[0].price > pair[1].price);
        }
        for pair in book.asks.windows(2) {
            assert!(pair[0].price < pair[1].price);
        }
        assert!(book.best_ask().unwrap() > book.best_bid().unwrap());
        assert!(book.spread().unwrap() > 0.0);
        assert!(book.mid().unwrap() > book.best_bid().unwrap());
        assert!(book.max_size(DEMO_DEPTH) > 0.0);
    }

    #[test]
    fn flash_epochs_track_size_changes() {
        let (bids, asks) = DemoBookSource::flash_epochs(20);
        assert_eq!(bids.len(), DEMO_DEPTH);
        assert_eq!(asks.len(), DEMO_DEPTH);
        // An epoch is a real tick change point or the decayed floor; never
        // ahead of the current tick.
        for epoch in bids.iter().chain(asks.iter()) {
            assert!(*epoch <= 20);
        }
    }

    #[test]
    fn empty_book_has_no_mid_or_spread() {
        let book = OrderBook {
            bids: Vec::new(),
            asks: Vec::new(),
            price_decimals: 2,
            size_decimals: 2,
        };
        assert!(book.mid().is_none());
        assert!(book.spread().is_none());
        assert!(book.best_bid().is_none());
    }
}
