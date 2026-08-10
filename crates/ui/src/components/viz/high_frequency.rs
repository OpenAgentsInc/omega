//! Coalescing gate for high-frequency market surfaces.
//!
//! Keep this value in the owning GPUI entity. Feed callbacks call [`push`],
//! then the entity's animation-frame callback calls [`take_for_frame`] for an
//! event batch or [`take_latest_for_frame`] for a snapshot and invokes
//! `cx.notify()` only when it returns `Some`. This is the reusable form of the
//! `LiveLadder` discipline: bounded batching, no more than one delivery per
//! frame, plus an optional minimum delivery interval.

use std::collections::VecDeque;

use documented::Documented;
use gpui::px;

use crate::components::viz::{MarketTokens, market_number_font};
use crate::prelude::*;

#[derive(Clone, Debug)]
pub struct HighFrequencyBatch<T> {
    pending: VecDeque<T>,
    capacity: usize,
    minimum_interval_ms: u64,
    next_delivery_at_ms: u64,
    delivered_frame: Option<u64>,
    received: u64,
    delivered: u64,
    dropped: u64,
}

impl<T> HighFrequencyBatch<T> {
    pub fn new(minimum_interval_ms: u64) -> Self {
        Self::with_capacity(minimum_interval_ms, 1_024)
    }

    pub fn with_capacity(minimum_interval_ms: u64, capacity: usize) -> Self {
        Self {
            pending: VecDeque::new(),
            capacity: capacity.max(1),
            minimum_interval_ms,
            next_delivery_at_ms: 0,
            delivered_frame: None,
            received: 0,
            delivered: 0,
            dropped: 0,
        }
    }

    /// Queues an update in a bounded buffer, dropping the oldest only when the
    /// caller-specified capacity is exhausted.
    pub fn push(&mut self, value: T) {
        if self.pending.len() == self.capacity {
            drop(self.pending.pop_front());
            self.dropped = self.dropped.saturating_add(1);
        }
        self.pending.push_back(value);
        self.received = self.received.saturating_add(1);
    }

    fn delivery_allowed(&self, frame_id: u64, now_ms: u64) -> bool {
        self.delivered_frame != Some(frame_id) && now_ms >= self.next_delivery_at_ms
    }

    fn mark_delivery(&mut self, frame_id: u64, now_ms: u64) {
        self.delivered_frame = Some(frame_id);
        self.next_delivery_at_ms = now_ms.saturating_add(self.minimum_interval_ms);
        self.delivered = self.delivered.saturating_add(1);
    }

    /// Drains all pending events as one bounded frame batch. Tape-like
    /// consumers use this path so no event is lost before capacity is reached.
    pub fn take_for_frame(&mut self, frame_id: u64, now_ms: u64) -> Option<Vec<T>> {
        if !self.delivery_allowed(frame_id, now_ms) || self.pending.is_empty() {
            return None;
        }
        let values = self.pending.drain(..).collect();
        self.mark_delivery(frame_id, now_ms);
        Some(values)
    }

    /// Returns only the newest pending snapshot. Book and watchlist consumers
    /// use this path because intermediate snapshots are already superseded.
    pub fn take_latest_for_frame(&mut self, frame_id: u64, now_ms: u64) -> Option<T> {
        if !self.delivery_allowed(frame_id, now_ms) {
            return None;
        }
        let value = self.pending.pop_back()?;
        self.pending.clear();
        self.mark_delivery(frame_id, now_ms);
        Some(value)
    }

    pub fn pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn received(&self) -> u64 {
        self.received
    }

    pub fn delivered(&self) -> u64 {
        self.delivered
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Component-library proof of latest-value per-frame batching.
pub struct HighFrequencyUpdateDemo {
    tokens: Option<MarketTokens>,
}

impl HighFrequencyUpdateDemo {
    pub fn new() -> Self {
        Self { tokens: None }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl Default for HighFrequencyUpdateDemo {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for HighFrequencyUpdateDemo {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let mut batch = HighFrequencyBatch::new(16);
        for sequence in 1..=64 {
            batch.push(sequence);
        }
        let delivered = batch.take_latest_for_frame(1, 16).unwrap_or_default();
        let metric = |label: &'static str, value: String| {
            h_flex()
                .w(px(220.))
                .justify_between()
                .child(
                    Label::new(label)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .font(market_number_font(cx))
                        .text_size(px(11.))
                        .text_color(tokens.text)
                        .child(value),
                )
        };
        v_flex()
            .debug_selector(|| "market.high_frequency_batch".into())
            .gap_1()
            .child(metric("feed updates", batch.received().to_string()))
            .child(metric("frame deliveries", batch.delivered().to_string()))
            .child(metric("latest sequence", delivered.to_string()))
    }
}

impl Component for HighFrequencyUpdateDemo {
    fn scope() -> ComponentScope {
        ComponentScope::Utilities
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Per-frame batching",
                vec![single_example(
                    "Sixty-four feed updates coalesce to the latest frame value",
                    HighFrequencyUpdateDemo::new().into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "The update contract is numeric and color-independent",
                    HighFrequencyUpdateDemo::new()
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
    fn coalesces_to_latest_once_per_frame() {
        let mut batch = HighFrequencyBatch::new(0);
        batch.push(1);
        batch.push(2);
        batch.push(3);
        assert_eq!(batch.take_latest_for_frame(10, 100), Some(3));
        batch.push(4);
        assert_eq!(batch.take_latest_for_frame(10, 100), None);
        assert_eq!(batch.take_latest_for_frame(11, 100), Some(4));
    }

    #[test]
    fn minimum_interval_throttles_without_dropping_latest() {
        let mut batch = HighFrequencyBatch::new(16);
        batch.push(1);
        assert_eq!(batch.take_latest_for_frame(1, 100), Some(1));
        batch.push(2);
        batch.push(3);
        assert_eq!(batch.take_latest_for_frame(2, 115), None);
        assert!(batch.pending());
        assert_eq!(batch.take_latest_for_frame(3, 116), Some(3));
    }

    #[test]
    fn event_batches_preserve_order_until_the_bound() {
        let mut batch = HighFrequencyBatch::with_capacity(0, 3);
        for value in 1..=4 {
            batch.push(value);
        }
        assert_eq!(batch.dropped(), 1);
        assert_eq!(batch.take_for_frame(1, 0), Some(vec![2, 3, 4]));
    }
}
