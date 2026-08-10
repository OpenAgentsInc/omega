//! Typed adapters from the Nautilus testnet stream into Omega's shared market
//! value/source traits. The stream remains the authority; this module only
//! groups trades into display candles and converts decimal book levels.

use std::collections::BTreeMap;

use nautilus_sidecar::{NautilusMarketSnapshot, StreamEvent};
use ui::{BookLevel, BookSource, Candle, CandleSeries, CandleSource, OrderBook};

const CANDLE_INTERVAL_MS: i64 = 60_000;
const MAX_CANDLES: usize = 240;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NautilusAccountSummary {
    pub account_ready: bool,
    pub account_id: Option<String>,
    pub balance_count: usize,
    pub available_margin_cents: Option<i64>,
    pub collateral_currency: Option<String>,
    pub order_count: usize,
    pub position_count: usize,
    pub fill_count: usize,
    pub frame_count: u64,
}

#[derive(Clone, Debug)]
pub struct NautilusLiveSnapshot {
    market: NautilusMarketSnapshot,
}

impl NautilusLiveSnapshot {
    pub fn new(market: NautilusMarketSnapshot) -> Self {
        Self { market }
    }

    pub fn account_summary(&self) -> NautilusAccountSummary {
        let (account_id, balance_count, available_margin_cents, collateral_currency) =
            match self.market.account.as_ref() {
                Some(StreamEvent::Account { state, .. }) => {
                    let balance = state
                        .get("balances")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|balances| {
                            balances.iter().find(|balance| {
                                balance
                                    .get("currency")
                                    .and_then(currency_code)
                                    .is_some_and(|currency| currency.contains("USD"))
                            })
                        });
                    let currency = balance
                        .and_then(|balance| balance.get("currency"))
                        .and_then(currency_code)
                        .map(str::to_owned);
                    let available = balance
                        .and_then(|balance| balance.get("free").or_else(|| balance.get("total")))
                        .and_then(decimal_value)
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .map(|value| (value * 100.0).round() as i64);
                    (
                        state
                            .get("account_id")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                        state
                            .get("balances")
                            .and_then(serde_json::Value::as_array)
                            .map_or(0, Vec::len),
                        available,
                        currency,
                    )
                }
                _ => (None, 0, None, None),
            };
        NautilusAccountSummary {
            account_ready: self.market.account.is_some(),
            account_id,
            balance_count,
            available_margin_cents,
            collateral_currency,
            order_count: self.market.orders.len(),
            position_count: self.market.positions.len(),
            fill_count: self.market.recent_fills.len(),
            frame_count: self.market.frame_count,
        }
    }

    pub fn latest_quote(&self) -> Option<(f64, f64)> {
        let StreamEvent::Quote {
            bid_price,
            ask_price,
            ..
        } = self.market.latest_quote.as_ref()?
        else {
            return None;
        };
        Some((bid_price.parse().ok()?, ask_price.parse().ok()?))
    }
}

fn currency_code(value: &serde_json::Value) -> Option<&str> {
    value.as_str().or_else(|| {
        value
            .as_object()
            .and_then(|object| object.get("code").or_else(|| object.get("currency")))
            .and_then(serde_json::Value::as_str)
    })
}

fn decimal_value(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| {
            value
                .as_str()
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse().ok())
        })
        .or_else(|| {
            value.as_object().and_then(|object| {
                object
                    .get("amount")
                    .or_else(|| object.get("value"))
                    .and_then(decimal_value)
            })
        })
}

pub struct NautilusBookSource {
    snapshot: NautilusMarketSnapshot,
}

impl NautilusBookSource {
    pub fn new(snapshot: NautilusMarketSnapshot) -> Self {
        Self { snapshot }
    }
}

impl BookSource for NautilusBookSource {
    fn snapshot(&self) -> OrderBook {
        let parse = |level: &nautilus_sidecar::RenderableBookLevel| {
            Some(BookLevel {
                price: level.price.parse().ok()?,
                size: level.size.parse().ok()?,
            })
        };
        let mut bids = self
            .snapshot
            .bids
            .iter()
            .filter_map(parse)
            .collect::<Vec<_>>();
        let mut asks = self
            .snapshot
            .asks
            .iter()
            .filter_map(parse)
            .collect::<Vec<_>>();
        bids.sort_by(|left, right| {
            right
                .price
                .partial_cmp(&left.price)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        asks.sort_by(|left, right| {
            left.price
                .partial_cmp(&right.price)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        OrderBook {
            bids,
            asks,
            price_decimals: 2,
            size_decimals: 4,
        }
    }
}

#[derive(Clone, Copy)]
struct CandleAccumulator {
    time_ms: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

impl CandleAccumulator {
    fn new(time_ms: i64, price: f64, size: f64) -> Self {
        Self {
            time_ms,
            open: price,
            high: price,
            low: price,
            close: price,
            volume: size,
        }
    }

    fn update(&mut self, price: f64, size: f64) {
        self.high = self.high.max(price);
        self.low = self.low.min(price);
        self.close = price;
        self.volume += size;
    }

    fn candle(self) -> Candle {
        Candle {
            time_ms: self.time_ms,
            open: self.open,
            high: self.high,
            low: self.low,
            close: self.close,
            volume: self.volume,
        }
    }
}

pub struct NautilusCandleSource {
    snapshot: NautilusMarketSnapshot,
}

impl NautilusCandleSource {
    pub fn new(snapshot: NautilusMarketSnapshot) -> Self {
        Self { snapshot }
    }
}

impl CandleSource for NautilusCandleSource {
    fn series(&self) -> CandleSeries {
        let mut buckets = BTreeMap::<i64, CandleAccumulator>::new();
        for event in &self.snapshot.recent_trades {
            let StreamEvent::Trade {
                price,
                size,
                ts_event,
                ..
            } = event
            else {
                continue;
            };
            let Some(price) = price.parse::<f64>().ok().filter(|price| price.is_finite()) else {
                continue;
            };
            let Some(size) = size.parse::<f64>().ok().filter(|size| size.is_finite()) else {
                continue;
            };
            let time_ms = i64::try_from(ts_event / 1_000_000).unwrap_or(i64::MAX);
            let bucket = time_ms - time_ms.rem_euclid(CANDLE_INTERVAL_MS);
            buckets
                .entry(bucket)
                .and_modify(|candle| candle.update(price, size))
                .or_insert_with(|| CandleAccumulator::new(bucket, price, size));
        }
        let candles = buckets
            .into_values()
            .rev()
            .take(MAX_CANDLES)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(CandleAccumulator::candle)
            .collect();
        CandleSeries::new(candles, 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trade(sequence: u64, price: &str, size: &str, ts_event: u64) -> StreamEvent {
        serde_json::from_value(serde_json::json!({
            "type": "trade",
            "schema": "omega.nautilus.stream.v1",
            "generation": 1,
            "sequence": sequence,
            "network": "testnet",
            "instrument_id": "BTC-USD-PERP.HYPERLIQUID",
            "price": price,
            "size": size,
            "aggressor_side": "BUYER",
            "trade_id": format!("trade-{sequence}"),
            "ts_event": ts_event,
            "ts_init": ts_event
        }))
        .expect("typed trade")
    }

    #[test]
    fn live_trades_group_into_ordered_minute_candles() {
        let snapshot = NautilusMarketSnapshot {
            recent_trades: vec![
                trade(1, "65000", "0.1", 60_000_000_000),
                trade(2, "65100", "0.2", 61_000_000_000),
                trade(3, "64900", "0.3", 120_000_000_000),
            ],
            ..Default::default()
        };
        let series = NautilusCandleSource::new(snapshot).series();
        assert_eq!(series.candles().len(), 2);
        let Some(first) = series.candles().first() else {
            assert!(false, "first live candle is missing");
            return;
        };
        assert_eq!(first.open, 65_000.0);
        assert_eq!(first.close, 65_100.0);
        assert!((first.volume - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn book_adapter_sorts_numeric_prices_best_first() {
        let snapshot = NautilusMarketSnapshot {
            bids: vec![
                nautilus_sidecar::RenderableBookLevel {
                    price: "9999".into(),
                    size: "1".into(),
                },
                nautilus_sidecar::RenderableBookLevel {
                    price: "10000".into(),
                    size: "2".into(),
                },
            ],
            asks: vec![nautilus_sidecar::RenderableBookLevel {
                price: "10001".into(),
                size: "3".into(),
            }],
            ..Default::default()
        };
        let book = NautilusBookSource::new(snapshot).snapshot();
        assert_eq!(book.best_bid(), Some(10_000.0));
        assert_eq!(book.best_ask(), Some(10_001.0));
    }

    #[test]
    fn account_summary_reads_live_collateral_without_inventing_a_balance() {
        let account = serde_json::from_value(serde_json::json!({
            "type": "account",
            "schema": "omega.nautilus.stream.v1",
            "generation": 1,
            "sequence": 1,
            "network": "testnet",
            "account_id": "HYPERLIQUID-001",
            "balances": [{
                "currency": "USDC",
                "free": "123.45 USDC",
                "locked": "1.00 USDC",
                "total": "124.45 USDC",
                "type": "MARGIN"
            }]
        }))
        .expect("typed account");
        let summary = NautilusLiveSnapshot::new(NautilusMarketSnapshot {
            account: Some(account),
            ..Default::default()
        })
        .account_summary();
        assert_eq!(summary.account_id.as_deref(), Some("HYPERLIQUID-001"));
        assert_eq!(summary.collateral_currency.as_deref(), Some("USDC"));
        assert_eq!(summary.available_margin_cents, Some(12_345));
    }
}
