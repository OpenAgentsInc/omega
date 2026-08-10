//! Candle fill markers and a ledger-linked realized trade log.

use std::ops::Range;
use std::sync::Arc;

use documented::Documented;
use gpui::{PathBuilder, point, px};

use crate::Table;
use crate::components::viz::{
    CandleSeries, CandleSource, DemoCandleSource, MarketDirection, MarketTokens, Plot,
    draw_candles, format_duration_ms, format_usd_cents, format_with_decimals, market_number_font,
};
use crate::prelude::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FillSide {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FillEffect {
    Entry,
    Exit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandleFill {
    pub fill_id: SharedString,
    pub ledger_entry_id: SharedString,
    pub time_ms: i64,
    pub price: f64,
    pub quantity: f64,
    pub side: FillSide,
    pub effect: FillEffect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FillsOnCandlesData {
    pub candles: CandleSeries,
    pub fills: Vec<CandleFill>,
}

pub trait FillsOnCandlesSource {
    fn fills_on_candles(&self) -> FillsOnCandlesData;
}

pub struct DemoFillsOnCandlesSource;

impl FillsOnCandlesSource for DemoFillsOnCandlesSource {
    fn fills_on_candles(&self) -> FillsOnCandlesData {
        let candles = DemoCandleSource::default().series();
        let fills = [16usize, 33, 52, 71]
            .into_iter()
            .filter_map(|index| {
                let candle = candles.candles().get(index)?;
                Some(CandleFill {
                    fill_id: format!("fill-{index}").into(),
                    ledger_entry_id: format!("ledger-{index}").into(),
                    time_ms: candle.time_ms,
                    price: candle.close,
                    quantity: 0.01 + index as f64 / 10_000.0,
                    side: if index % 2 == 0 {
                        FillSide::Buy
                    } else {
                        FillSide::Sell
                    },
                    effect: if index % 3 == 0 {
                        FillEffect::Exit
                    } else {
                        FillEffect::Entry
                    },
                })
            })
            .collect();
        FillsOnCandlesData { candles, fills }
    }
}

fn paint_fill_marker(
    frame: &crate::components::viz::PlotFrame,
    window: &mut Window,
    fill: &CandleFill,
) {
    let x = frame.x_at(fill.time_ms as f64);
    let y = frame.y_at(fill.price);
    let (tip, left, right) = match fill.side {
        FillSide::Buy => (
            point(x, y - px(7.0)),
            point(x - px(5.0), y + px(4.0)),
            point(x + px(5.0), y + px(4.0)),
        ),
        FillSide::Sell => (
            point(x, y + px(7.0)),
            point(x - px(5.0), y - px(4.0)),
            point(x + px(5.0), y - px(4.0)),
        ),
    };
    let color = match fill.side {
        FillSide::Buy => frame.tokens.up,
        FillSide::Sell => frame.tokens.down,
    };
    let mut builder = match fill.effect {
        FillEffect::Entry => PathBuilder::fill(),
        FillEffect::Exit => PathBuilder::stroke(px(1.5)),
    };
    builder.move_to(tip);
    builder.line_to(left);
    builder.line_to(right);
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Engine fills painted as entry/exit markers over the shared candlestick plot.
pub struct FillsOnCandlesChart {
    data: FillsOnCandlesData,
    width: f32,
    height: f32,
    tokens: Option<MarketTokens>,
}

impl FillsOnCandlesChart {
    pub fn new(data: FillsOnCandlesData) -> Self {
        Self {
            data,
            width: 620.0,
            height: 320.0,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl FillsOnCandlesSource) -> Self {
        Self::new(source.fills_on_candles())
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for FillsOnCandlesChart {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let Some(time_domain) = self.data.candles.time_domain() else {
            return div().into_any_element();
        };
        let Some(price_domain) = self.data.candles.price_domain() else {
            return div().into_any_element();
        };
        let price_decimals = self.data.candles.price_decimals();
        let candles = Arc::new(self.data.candles.candles().to_vec());
        let fills = Arc::new(self.data.fills);
        let max_volume = self.data.candles.max_volume();
        let mut plot = Plot::new(time_domain, price_domain)
            .plot_size(self.width, self.height)
            .value_formatter(move |value| format_with_decimals(value, price_decimals))
            .layer(move |frame, window, cx| {
                draw_candles(frame, window, cx, &candles, max_volume, price_decimals);
            })
            .layer(move |frame, window, _| {
                let left = f32::from(frame.plot_bounds.origin.x) - 8.0;
                let right =
                    f32::from(frame.plot_bounds.origin.x + frame.plot_bounds.size.width) + 8.0;
                for fill in fills.iter() {
                    let x = f32::from(frame.x_at(fill.time_ms as f64));
                    if x >= left && x <= right {
                        paint_fill_marker(frame, window, fill);
                    }
                }
            });
        if let Some(tokens) = self.tokens {
            plot = plot.tokens(tokens);
        }
        div()
            .debug_selector(|| "market.fills_on_candles".into())
            .child(plot)
            .into_any_element()
    }
}

impl Component for FillsOnCandlesChart {
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
                "Fills on candles",
                vec![single_example(
                    "Filled entries and outlined exits link execution to price",
                    FillsOnCandlesChart::from_source(&DemoFillsOnCandlesSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Triangle direction and fill treatment preserve side and effect",
                    FillsOnCandlesChart::from_source(&DemoFillsOnCandlesSource)
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerEntryLink {
    pub ledger_entry_id: SharedString,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeLogRow {
    pub position_id: SharedString,
    pub instrument: SharedString,
    pub side: FillSide,
    pub realized_pnl_cents: i64,
    pub duration_ms: i64,
    pub costs_cents: i64,
    pub quantity: f64,
    pub ledger: LedgerEntryLink,
}

pub trait TradeLogSource {
    fn trade_log(&self) -> Vec<TradeLogRow>;
}

pub struct DemoTradeLogSource;

impl TradeLogSource for DemoTradeLogSource {
    fn trade_log(&self) -> Vec<TradeLogRow> {
        (0..18)
            .map(|index| TradeLogRow {
                position_id: format!("pos-{index:03}").into(),
                instrument: "BTC-USD-PERP.HYPERLIQUID".into(),
                side: if index % 2 == 0 {
                    FillSide::Buy
                } else {
                    FillSide::Sell
                },
                realized_pnl_cents: (index as i64 - 5) * 1_725,
                duration_ms: (index as i64 + 1) * 181_000,
                costs_cents: (index as i64 + 2) * 39,
                quantity: 0.01 + index as f64 * 0.002,
                ledger: LedgerEntryLink {
                    ledger_entry_id: format!("entry-{index:03}").into(),
                },
            })
            .collect()
    }
}

type LedgerOpenHandler = Arc<dyn Fn(&LedgerEntryLink, &mut Window, &mut App) + 'static>;

#[derive(IntoElement, RegisterComponent, Documented)]
/// Visible trade rows with realized PnL, duration, costs, and ledger-browser links.
pub struct TradeLogTable {
    rows: Vec<TradeLogRow>,
    visible_range: Range<usize>,
    on_open_ledger: Option<LedgerOpenHandler>,
    tokens: Option<MarketTokens>,
}

impl TradeLogTable {
    pub fn new(rows: Vec<TradeLogRow>) -> Self {
        Self {
            visible_range: 0..rows.len().min(50),
            rows,
            on_open_ledger: None,
            tokens: None,
        }
    }

    pub fn from_source(source: &impl TradeLogSource) -> Self {
        Self::new(source.trade_log())
    }

    pub fn visible_range(mut self, range: Range<usize>) -> Self {
        self.visible_range = range;
        self
    }

    pub fn on_open_ledger(
        mut self,
        handler: impl Fn(&LedgerEntryLink, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_ledger = Some(Arc::new(handler));
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

fn text_cell(text: impl Into<SharedString>) -> AnyElement {
    div()
        .text_size(px(11.0))
        .child(text.into())
        .into_any_element()
}

impl RenderOnce for TradeLogTable {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let number_font = market_number_font(cx);
        let start = self.visible_range.start.min(self.rows.len());
        let end = self.visible_range.end.min(self.rows.len()).max(start);
        let mut table = Table::new(7).header(
            [
                "Position",
                "Instrument",
                "Side",
                "Quantity",
                "Realized",
                "Duration / costs",
                "Ledger",
            ]
            .into_iter()
            .map(text_cell)
            .collect(),
        );
        for row in self.rows.into_iter().skip(start).take(end - start) {
            let direction = MarketDirection::of_i64(row.realized_pnl_cents);
            let ledger = row.ledger.clone();
            let handler = self.on_open_ledger.clone();
            let side = match row.side {
                FillSide::Buy => "▲ Buy",
                FillSide::Sell => "▼ Sell",
            };
            table = table.row(vec![
                text_cell(row.position_id),
                text_cell(row.instrument),
                text_cell(side),
                div()
                    .font(number_font.clone())
                    .child(format_with_decimals(row.quantity, 4))
                    .into_any_element(),
                div()
                    .font(number_font.clone())
                    .text_color(tokens.direction_color(direction))
                    .child(format!(
                        "{} {}",
                        direction.glyph(),
                        format_usd_cents(row.realized_pnl_cents)
                    ))
                    .into_any_element(),
                div()
                    .font(number_font.clone())
                    .child(format!(
                        "{} · {}",
                        format_duration_ms(row.duration_ms),
                        format_usd_cents(row.costs_cents)
                    ))
                    .into_any_element(),
                div()
                    .id(ledger.ledger_entry_id.clone())
                    .cursor_pointer()
                    .text_color(tokens.up)
                    .child(ledger.ledger_entry_id.clone())
                    .when_some(handler, move |this, handler| {
                        this.on_click(move |_, window, cx| handler(&ledger, window, cx))
                    })
                    .into_any_element(),
            ]);
        }
        div()
            .debug_selector(|| "market.trade_log_table".into())
            .child(table)
    }
}

impl Component for TradeLogTable {
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
                "Trade log",
                vec![single_example(
                    "Visible realized positions link to ledger entry IDs",
                    TradeLogTable::from_source(&DemoTradeLogSource)
                        .visible_range(0..8)
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Signed PnL and side glyphs survive without hue",
                    TradeLogTable::from_source(&DemoTradeLogSource)
                        .visible_range(0..5)
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
    fn fill_fixture_links_every_marker_to_a_ledger_entry() {
        let data = DemoFillsOnCandlesSource.fills_on_candles();
        assert!(!data.fills.is_empty());
        assert!(
            data.fills
                .iter()
                .all(|fill| !fill.ledger_entry_id.is_empty())
        );
    }

    #[test]
    fn trade_log_carries_realized_cost_duration_and_ledger_identity() {
        let rows = DemoTradeLogSource.trade_log();
        assert_eq!(rows.len(), 18);
        assert!(rows.iter().all(|row| row.duration_ms > 0));
        assert!(rows.iter().all(|row| row.costs_cents >= 0));
        assert!(
            rows.iter()
                .all(|row| !row.ledger.ledger_entry_id.is_empty())
        );
    }
}
