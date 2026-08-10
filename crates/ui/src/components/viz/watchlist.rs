use std::sync::Arc;

use documented::Documented;
use gpui::px;

use crate::Table;
use crate::components::viz::{
    HighFrequencyBatch, MarketTokens, Sparkline, format_signed_percent, format_with_decimals,
    market_number_font,
};
use crate::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct WatchlistRow {
    pub instrument: SharedString,
    pub venue: SharedString,
    pub last: f64,
    pub change_fraction: f64,
    pub volume_24h: f64,
    pub sparkline: Vec<f64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchlistSort {
    Instrument,
    Last,
    Change,
    Volume,
}
pub trait WatchlistSource {
    fn watchlist_snapshots(&self) -> Vec<Vec<WatchlistRow>>;
}
pub struct DemoWatchlistSource;
impl WatchlistSource for DemoWatchlistSource {
    fn watchlist_snapshots(&self) -> Vec<Vec<WatchlistRow>> {
        (0..32)
            .map(|tick| {
                ["BTC-PERP", "ETH-PERP", "SOL-PERP", "HYPE-PERP", "XRP-PERP"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, instrument)| {
                        let base = [116_400.0, 4_120.0, 184.0, 44.0, 3.2]
                            .get(index)
                            .copied()
                            .unwrap_or_default();
                        WatchlistRow {
                            instrument: instrument.into(),
                            venue: "Hyperliquid".into(),
                            last: base + tick as f64 * (index as f64 + 1.0) * 0.03,
                            change_fraction: (index as f64 - 2.0) * 0.006,
                            volume_24h: 4_000_000_000.0 / (index + 1) as f64,
                            sparkline: (0..48)
                                .map(|sample| {
                                    base + (sample as f64 / 5.0).sin() * (index as f64 + 1.0)
                                })
                                .collect(),
                        }
                    })
                    .collect()
            })
            .collect()
    }
}
fn latest_rows(source: &impl WatchlistSource) -> Vec<WatchlistRow> {
    let mut batch = HighFrequencyBatch::with_capacity(16, 64);
    for snapshot in source.watchlist_snapshots() {
        batch.push(snapshot);
    }
    batch.take_latest_for_frame(1, 16).unwrap_or_default()
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Sortable, virtualized watchlist with frame-coalesced snapshots.
pub struct WatchlistTable {
    rows: Vec<WatchlistRow>,
    sort: WatchlistSort,
    descending: bool,
    tokens: Option<MarketTokens>,
}
impl WatchlistTable {
    pub fn from_source(source: &impl WatchlistSource) -> Self {
        Self {
            rows: latest_rows(source),
            sort: WatchlistSort::Volume,
            descending: true,
            tokens: None,
        }
    }
    pub fn sort_by(mut self, sort: WatchlistSort, descending: bool) -> Self {
        self.sort = sort;
        self.descending = descending;
        self
    }
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}
impl RenderOnce for WatchlistTable {
    fn render(mut self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        self.rows.sort_by(|left, right| match self.sort {
            WatchlistSort::Instrument => left.instrument.cmp(&right.instrument),
            WatchlistSort::Last => left.last.total_cmp(&right.last),
            WatchlistSort::Change => left.change_fraction.total_cmp(&right.change_fraction),
            WatchlistSort::Volume => left.volume_24h.total_cmp(&right.volume_24h),
        });
        if self.descending {
            self.rows.reverse();
        }
        let rows = Arc::new(self.rows);
        let count = rows.len();
        let sort = self.sort;
        let arrow = if self.descending { "▼" } else { "▲" };
        let heading = |label: &'static str, column: WatchlistSort| -> String {
            if sort == column {
                format!("{label} {arrow}")
            } else {
                label.into()
            }
        };
        div()
            .debug_selector(|| "market.watchlist".into())
            .w(px(720.))
            .h(px(250.))
            .child(
                Table::new(6)
                    .width(px(720.))
                    .header(vec![
                        heading("instrument", WatchlistSort::Instrument),
                        "venue".into(),
                        heading("last", WatchlistSort::Last),
                        heading("change", WatchlistSort::Change),
                        heading("24h volume", WatchlistSort::Volume),
                        "trend".into(),
                    ])
                    .uniform_list("market-watchlist-rows", count, move |range, _window, cx| {
                        range
                            .filter_map(|index| rows.get(index))
                            .map(|row| {
                                let (change, direction) =
                                    format_signed_percent(row.change_fraction, 2);
                                let number = |text: String, color| {
                                    div()
                                        .font(market_number_font(cx))
                                        .text_size(px(11.))
                                        .text_color(color)
                                        .child(text)
                                        .into_any_element()
                                };
                                vec![
                                    Label::new(row.instrument.clone())
                                        .size(LabelSize::Small)
                                        .into_any_element(),
                                    Label::new(row.venue.clone())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .into_any_element(),
                                    number(format_with_decimals(row.last, 2), tokens.text),
                                    number(
                                        format!("{} {change}", direction.glyph()),
                                        tokens.direction_color(direction),
                                    ),
                                    number(
                                        crate::components::viz::format_compact(row.volume_24h),
                                        tokens.text,
                                    ),
                                    Sparkline::new(row.sparkline.clone())
                                        .size(80., 20.)
                                        .tokens(tokens)
                                        .into_any_element(),
                                ]
                            })
                            .collect()
                    }),
            )
    }
}
impl Component for WatchlistTable {
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
                "Watchlist",
                vec![single_example(
                    "Sortable virtualized snapshot",
                    WatchlistTable::from_source(&DemoWatchlistSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Signs, arrows, and sparklines preserve direction",
                    WatchlistTable::from_source(&DemoWatchlistSource)
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
    fn watchlist_coalesces_latest_snapshot() {
        let rows = latest_rows(&DemoWatchlistSource);
        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|row| row.sparkline.len() == 48));
    }
}
