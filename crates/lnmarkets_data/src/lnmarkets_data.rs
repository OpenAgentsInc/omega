use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context as _, Result, anyhow, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use lnmarkets_client::{
    CandleResolution, CandlesQuery, Credentials, LnMarketsClient, LnMarketsStreamClient, Network,
    Pagination, StreamEvent, StreamTopic,
};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

const DEFAULT_RETENTION: Duration = Duration::from_secs(90 * 24 * 60 * 60);
const DEFAULT_BACKFILL_LIMIT: u16 = 1_000;
const MAX_BACKFILL_PAGES: usize = 256;
const STREAM_BATCH_SIZE: usize = 100;
const STREAM_BATCH_TIMEOUT: Duration = Duration::from_secs(30);
const STREAM_RESTART_DELAY: Duration = Duration::from_secs(5);
const FUNDING_EMA_ALPHA: f64 = 0.2;
const ONE_HOUR_MS: i64 = 60 * 60 * 1_000;
const SIX_HOURS_MS: i64 = 6 * ONE_HOUR_MS;
const ONE_DAY_MS: i64 = 24 * ONE_HOUR_MS;

pub const CANDLE_TOPIC: &str = "rest/futures/candles/1h";
pub const FUNDING_SETTLEMENT_TOPIC: &str = "rest/futures/funding-settlements";
pub const ORACLE_INDEX_TOPIC: &str = "rest/oracle/index";
pub const STREAM_BUCKETS_TOPIC: &str = "futures/inverse/btc_usd/buckets";
pub const STREAM_FUNDING_TOPIC: &str = "futures/inverse/btc_usd/funding";
pub const STREAM_LAST_PRICE_TOPIC: &str = "futures/inverse/btc_usd/lastPrice";
pub const STREAM_OHLC_ONE_MINUTE_TOPIC: &str = "futures/inverse/btc_usd/ohlc/1m";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DataRegistration;

pub const REGISTRATION: DataRegistration = DataRegistration;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    RestBackfill,
    Stream,
}

impl EventSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::RestBackfill => "rest_backfill",
            Self::Stream => "stream",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredMarketEvent {
    pub network: Network,
    pub topic: String,
    pub event_time_ms: i64,
    pub received_at_ms: i64,
    pub source: EventSource,
    pub payload: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimedValue {
    pub time_ms: i64,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiquidityTier {
    pub min_size: f64,
    pub max_size: f64,
    pub bid_price: Option<f64>,
    pub ask_price: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountAllocation {
    pub btc_sats: f64,
    pub synthetic_usd: f64,
    pub target_btc_weight: f64,
}

impl AccountAllocation {
    pub fn validate(self) -> Result<Self> {
        if !self.btc_sats.is_finite() || self.btc_sats < 0.0 {
            bail!("account BTC balance must be a non-negative finite number");
        }
        if !self.synthetic_usd.is_finite() || self.synthetic_usd < 0.0 {
            bail!("account synthetic USD balance must be a non-negative finite number");
        }
        if !self.target_btc_weight.is_finite() || !(0.0..=1.0).contains(&self.target_btc_weight) {
            bail!("target BTC weight must be between zero and one");
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeatureInput {
    pub prices: Vec<TimedValue>,
    pub index_prices: Vec<TimedValue>,
    pub funding_rates: Vec<TimedValue>,
    pub liquidity_time_ms: Option<i64>,
    pub liquidity_tiers: Vec<LiquidityTier>,
    pub account: Option<AccountAllocation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingSign {
    Negative,
    Neutral,
    Positive,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VolatilityFeatures {
    pub one_hour: Option<f64>,
    pub six_hours: Option<f64>,
    pub one_day: Option<f64>,
    pub price_points: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexFeatures {
    pub current_price: Option<f64>,
    pub one_hour_move: Option<f64>,
    pub six_hours_move: Option<f64>,
    pub one_day_move: Option<f64>,
    pub price_points: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FundingFeatures {
    pub current_rate: Option<f64>,
    pub ema: Option<f64>,
    pub sign: FundingSign,
    pub sign_flipped_at_ms: Option<i64>,
    pub samples: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiquidityFeatures {
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub spread: Option<f64>,
    pub spread_bps: Option<f64>,
    pub bid_depth: Option<f64>,
    pub ask_depth: Option<f64>,
    pub tier_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountDriftFeatures {
    pub btc_value_usd: f64,
    pub synthetic_usd: f64,
    pub current_btc_weight: f64,
    pub target_btc_weight: f64,
    pub drift: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeatureSnapshot {
    pub schema: String,
    pub as_of_ms: Option<i64>,
    #[serde(default)]
    pub index: IndexFeatures,
    pub volatility: VolatilityFeatures,
    pub funding: FundingFeatures,
    pub liquidity: LiquidityFeatures,
    pub account_drift: Option<AccountDriftFeatures>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeatureReplayTick {
    pub occurred_at_ms: i64,
    pub features: FeatureSnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeatureReplayDataset {
    pub schema: String,
    pub network: Network,
    pub from_ms: i64,
    pub to_ms: i64,
    pub candle_count: u64,
    pub oracle_index_count: u64,
    pub funding_settlement_count: u64,
    pub ticks: Vec<FeatureReplayTick>,
}

pub fn derive_features(mut input: FeatureInput) -> Result<FeatureSnapshot> {
    input.prices.retain(valid_price);
    input.index_prices.retain(valid_price);
    input.funding_rates.retain(valid_funding_rate);
    input.prices.sort_by_key(|point| point.time_ms);
    input.index_prices.sort_by_key(|point| point.time_ms);
    input.funding_rates.sort_by_key(|point| point.time_ms);
    input.prices.dedup_by_key(|point| point.time_ms);
    input.index_prices.dedup_by_key(|point| point.time_ms);
    input.funding_rates.dedup_by_key(|point| point.time_ms);
    let account = input.account.map(AccountAllocation::validate).transpose()?;
    let as_of_ms = input
        .prices
        .last()
        .map(|point| point.time_ms)
        .into_iter()
        .chain(input.index_prices.last().map(|point| point.time_ms))
        .chain(input.funding_rates.last().map(|point| point.time_ms))
        .chain(input.liquidity_time_ms)
        .max();
    let volatility = VolatilityFeatures {
        one_hour: realized_volatility(&input.prices, as_of_ms, ONE_HOUR_MS),
        six_hours: realized_volatility(&input.prices, as_of_ms, SIX_HOURS_MS),
        one_day: realized_volatility(&input.prices, as_of_ms, ONE_DAY_MS),
        price_points: input.prices.len(),
    };
    let index = IndexFeatures {
        current_price: input.index_prices.last().map(|point| point.value),
        one_hour_move: window_move(&input.index_prices, as_of_ms, ONE_HOUR_MS),
        six_hours_move: window_move(&input.index_prices, as_of_ms, SIX_HOURS_MS),
        one_day_move: window_move(&input.index_prices, as_of_ms, ONE_DAY_MS),
        price_points: input.index_prices.len(),
    };
    let funding = funding_features(&input.funding_rates);
    let liquidity = liquidity_features(&input.liquidity_tiers);
    let last_price = input
        .index_prices
        .last()
        .or_else(|| input.prices.last())
        .map(|point| point.value);
    let account_drift = match (account, last_price) {
        (Some(account), Some(last_price)) => account_drift(account, last_price),
        _ => None,
    };
    Ok(FeatureSnapshot {
        schema: "omega.lnmarkets.features.v1".into(),
        as_of_ms,
        index,
        volatility,
        funding,
        liquidity,
        account_drift,
    })
}

fn window_move(prices: &[TimedValue], as_of_ms: Option<i64>, window_ms: i64) -> Option<f64> {
    let cutoff = as_of_ms?.saturating_sub(window_ms);
    let mut prices = prices
        .iter()
        .filter(|point| point.time_ms >= cutoff)
        .map(|point| point.value);
    let first = prices.next()?;
    let last = prices.next_back()?;
    Some((last / first).ln())
}

fn valid_price(point: &TimedValue) -> bool {
    point.time_ms >= 0 && point.value.is_finite() && point.value > 0.0
}

fn valid_funding_rate(point: &TimedValue) -> bool {
    point.time_ms >= 0 && point.value.is_finite()
}

fn realized_volatility(
    prices: &[TimedValue],
    as_of_ms: Option<i64>,
    window_ms: i64,
) -> Option<f64> {
    let cutoff = as_of_ms?.saturating_sub(window_ms);
    let prices = prices
        .iter()
        .filter(|point| point.time_ms >= cutoff)
        .map(|point| point.value)
        .collect::<Vec<_>>();
    if prices.len() < 2 {
        return None;
    }
    let sum_squared_log_returns = prices
        .windows(2)
        .map(|window| (window[1] / window[0]).ln().powi(2))
        .sum::<f64>();
    Some(sum_squared_log_returns.sqrt())
}

fn funding_features(rates: &[TimedValue]) -> FundingFeatures {
    let mut ema: Option<f64> = None;
    let mut previous_sign = FundingSign::Neutral;
    let mut sign_flipped_at_ms = None;
    for rate in rates {
        ema = Some(match ema {
            Some(previous) => FUNDING_EMA_ALPHA * rate.value + (1.0 - FUNDING_EMA_ALPHA) * previous,
            None => rate.value,
        });
        let sign = funding_sign(rate.value);
        if sign != FundingSign::Neutral
            && previous_sign != FundingSign::Neutral
            && sign != previous_sign
        {
            sign_flipped_at_ms = Some(rate.time_ms);
        }
        if sign != FundingSign::Neutral {
            previous_sign = sign;
        }
    }
    let current_rate = rates.last().map(|rate| rate.value);
    FundingFeatures {
        current_rate,
        ema,
        sign: current_rate
            .map(funding_sign)
            .unwrap_or(FundingSign::Neutral),
        sign_flipped_at_ms,
        samples: rates.len(),
    }
}

fn funding_sign(rate: f64) -> FundingSign {
    if rate > 0.0 {
        FundingSign::Positive
    } else if rate < 0.0 {
        FundingSign::Negative
    } else {
        FundingSign::Neutral
    }
}

fn liquidity_features(tiers: &[LiquidityTier]) -> LiquidityFeatures {
    let best_bid = tiers
        .iter()
        .filter_map(|tier| tier.bid_price)
        .filter(|price| price.is_finite() && *price > 0.0)
        .reduce(f64::max);
    let best_ask = tiers
        .iter()
        .filter_map(|tier| tier.ask_price)
        .filter(|price| price.is_finite() && *price > 0.0)
        .reduce(f64::min);
    let spread = best_bid
        .zip(best_ask)
        .and_then(|(bid, ask)| (ask >= bid).then_some(ask - bid));
    let midpoint = best_bid
        .zip(best_ask)
        .map(|(bid, ask)| (bid + ask) / 2.0)
        .filter(|midpoint| *midpoint > 0.0);
    let bid_depth = tiers
        .iter()
        .filter(|tier| tier.bid_price.is_some())
        .map(|tier| tier.max_size)
        .filter(|size| size.is_finite() && *size >= 0.0)
        .reduce(f64::max);
    let ask_depth = tiers
        .iter()
        .filter(|tier| tier.ask_price.is_some())
        .map(|tier| tier.max_size)
        .filter(|size| size.is_finite() && *size >= 0.0)
        .reduce(f64::max);
    LiquidityFeatures {
        best_bid,
        best_ask,
        spread,
        spread_bps: spread
            .zip(midpoint)
            .map(|(spread, midpoint)| spread / midpoint * 10_000.0),
        bid_depth,
        ask_depth,
        tier_count: tiers.len(),
    }
}

fn account_drift(account: AccountAllocation, last_price: f64) -> Option<AccountDriftFeatures> {
    let btc_value_usd = account.btc_sats / 100_000_000.0 * last_price;
    let total_value_usd = btc_value_usd + account.synthetic_usd;
    if !total_value_usd.is_finite() || total_value_usd <= 0.0 {
        return None;
    }
    let current_btc_weight = btc_value_usd / total_value_usd;
    Some(AccountDriftFeatures {
        btc_value_usd,
        synthetic_usd: account.synthetic_usd,
        current_btc_weight,
        target_btc_weight: account.target_btc_weight,
        drift: current_btc_weight - account.target_btc_weight,
    })
}

#[derive(Clone)]
pub struct MarketDataStore {
    connection: Arc<Mutex<Connection>>,
    retention: Duration,
}

impl MarketDataStore {
    pub fn open(path: &Path, retention: Duration) -> Result<Self> {
        if retention.is_zero() {
            bail!("LN Markets retention must be greater than zero");
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("could not create LN Markets data directory {parent:?}")
            })?;
        }
        let connection = Connection::open(path)
            .with_context(|| format!("could not open LN Markets data store {path:?}"))?;
        Self::from_connection(connection, retention)
    }

    pub fn in_memory(retention: Duration) -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, retention)
    }

    pub fn default_retention() -> Duration {
        DEFAULT_RETENTION
    }

    fn from_connection(connection: Connection, retention: Duration) -> Result<Self> {
        if retention.is_zero() {
            bail!("LN Markets retention must be greater than zero");
        }
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS lnmarkets_market_events (
                 network TEXT NOT NULL,
                 topic TEXT NOT NULL,
                 event_key TEXT NOT NULL,
                 event_time_ms INTEGER NOT NULL,
                 received_at_ms INTEGER NOT NULL,
                 source TEXT NOT NULL,
                 payload_json TEXT NOT NULL,
                 PRIMARY KEY (network, topic, event_key)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS idx_lnmarkets_market_events_time
                 ON lnmarkets_market_events(network, topic, event_time_ms DESC);
             CREATE TABLE IF NOT EXISTS lnmarkets_collector_state (
                 network TEXT NOT NULL,
                 state_key TEXT NOT NULL,
                 state_value TEXT NOT NULL,
                 PRIMARY KEY (network, state_key)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS lnmarkets_feature_snapshots (
                 network TEXT PRIMARY KEY,
                 computed_at_ms INTEGER NOT NULL,
                 snapshot_json TEXT NOT NULL
             ) STRICT;",
        )?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            retention,
        })
    }

    pub fn insert(
        &self,
        network: Network,
        topic: &str,
        event_time_ms: i64,
        source: EventSource,
        payload: &Value,
    ) -> Result<bool> {
        let received_at_ms = Utc::now().timestamp_millis();
        let event_key = event_key(topic, event_time_ms, source, payload)?;
        let payload_json = serde_json::to_string(payload)?;
        let changed = self.connection.lock().execute(
            "INSERT OR IGNORE INTO lnmarkets_market_events (
                 network, topic, event_key, event_time_ms, received_at_ms, source, payload_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                network_name(network),
                topic,
                event_key,
                event_time_ms,
                received_at_ms,
                source.as_str(),
                payload_json,
            ],
        )?;
        self.refresh_features(network)?;
        Ok(changed > 0)
    }

    pub fn insert_backfill_batch(
        &self,
        network: Network,
        topic: &str,
        events: &[(i64, Value)],
    ) -> Result<usize> {
        let received_at_ms = Utc::now().timestamp_millis();
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let mut inserted = 0;
        {
            let mut statement = transaction.prepare(
                "INSERT OR IGNORE INTO lnmarkets_market_events (
                     network, topic, event_key, event_time_ms, received_at_ms, source, payload_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (event_time_ms, payload) in events {
                let event_key =
                    event_key(topic, *event_time_ms, EventSource::RestBackfill, payload)?;
                let payload_json = serde_json::to_string(payload)?;
                inserted += statement.execute(params![
                    network_name(network),
                    topic,
                    event_key,
                    event_time_ms,
                    received_at_ms,
                    EventSource::RestBackfill.as_str(),
                    payload_json,
                ])?;
            }
        }
        transaction.commit()?;
        drop(connection);
        self.refresh_features(network)?;
        Ok(inserted)
    }

    pub fn insert_stream_batch(&self, network: Network, events: &[StreamEvent]) -> Result<usize> {
        let received_at_ms = Utc::now().timestamp_millis();
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let mut inserted = 0;
        {
            let mut statement = transaction.prepare(
                "INSERT OR IGNORE INTO lnmarkets_market_events (
                     network, topic, event_key, event_time_ms, received_at_ms, source, payload_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for event in events {
                let event_time_ms = event_timestamp_ms(&event.data).unwrap_or(received_at_ms);
                let topic = event.topic.as_str();
                let event_key = event_key(topic, event_time_ms, EventSource::Stream, &event.data)?;
                let payload_json = serde_json::to_string(&event.data)?;
                inserted += statement.execute(params![
                    network_name(network),
                    topic,
                    event_key,
                    event_time_ms,
                    received_at_ms,
                    EventSource::Stream.as_str(),
                    payload_json,
                ])?;
            }
        }
        transaction.commit()?;
        drop(connection);
        self.refresh_features(network)?;
        Ok(inserted)
    }

    pub fn recent(
        &self,
        network: Network,
        topic: &str,
        limit: usize,
    ) -> Result<Vec<StoredMarketEvent>> {
        if limit == 0 || limit > 10_000 {
            bail!("history limit must be between 1 and 10000");
        }
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT event_time_ms, received_at_ms, source, payload_json
             FROM lnmarkets_market_events
             WHERE network = ?1 AND topic = ?2
             ORDER BY event_time_ms DESC
             LIMIT ?3",
        )?;
        let rows =
            statement.query_map(params![network_name(network), topic, limit as i64], |row| {
                let source: String = row.get(2)?;
                let payload_json: String = row.get(3)?;
                Ok((row.get(0)?, row.get(1)?, source, payload_json))
            })?;
        let mut events = Vec::new();
        for row in rows {
            let (event_time_ms, received_at_ms, source, payload_json) = row?;
            events.push(StoredMarketEvent {
                network,
                topic: topic.to_owned(),
                event_time_ms,
                received_at_ms,
                source: parse_source(&source)?,
                payload: serde_json::from_str(&payload_json)?,
            });
        }
        Ok(events)
    }

    pub fn range(
        &self,
        network: Network,
        topic: &str,
        from_ms: i64,
        to_ms: Option<i64>,
        limit: usize,
    ) -> Result<Vec<StoredMarketEvent>> {
        if limit == 0 || limit > 10_000 {
            bail!("history limit must be between 1 and 10000");
        }
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT event_time_ms, received_at_ms, source, payload_json
             FROM lnmarkets_market_events
             WHERE network = ?1 AND topic = ?2 AND event_time_ms >= ?3
               AND (?4 IS NULL OR event_time_ms <= ?4)
             ORDER BY event_time_ms DESC
             LIMIT ?5",
        )?;
        let rows = statement.query_map(
            params![network_name(network), topic, from_ms, to_ms, limit as i64],
            |row| {
                let source: String = row.get(2)?;
                let payload_json: String = row.get(3)?;
                Ok((row.get(0)?, row.get(1)?, source, payload_json))
            },
        )?;
        let mut events = Vec::new();
        for row in rows {
            let (event_time_ms, received_at_ms, source, payload_json) = row?;
            events.push(StoredMarketEvent {
                network,
                topic: topic.to_owned(),
                event_time_ms,
                received_at_ms,
                source: parse_source(&source)?,
                payload: serde_json::from_str(&payload_json)?,
            });
        }
        events.reverse();
        Ok(events)
    }

    pub fn topics(&self, network: Network) -> Result<Vec<String>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT DISTINCT topic FROM lnmarkets_market_events
             WHERE network = ?1 ORDER BY topic",
        )?;
        let rows = statement.query_map(params![network_name(network)], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn event_count(&self, network: Network) -> Result<u64> {
        let count: i64 = self.connection.lock().query_row(
            "SELECT COUNT(*) FROM lnmarkets_market_events WHERE network = ?1",
            params![network_name(network)],
            |row| row.get(0),
        )?;
        u64::try_from(count).context("LN Markets event count was negative")
    }

    pub fn latest_event_time(&self, network: Network, topic: &str) -> Result<Option<i64>> {
        self.connection
            .lock()
            .query_row(
                "SELECT MAX(event_time_ms) FROM lnmarkets_market_events
                 WHERE network = ?1 AND topic = ?2",
                params![network_name(network), topic],
                |row| row.get(0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(Into::into)
    }

    pub fn prune(&self, now_ms: i64) -> Result<usize> {
        let retention_ms = i64::try_from(self.retention.as_millis())
            .context("LN Markets retention does not fit in milliseconds")?;
        let cutoff = now_ms.saturating_sub(retention_ms);
        self.connection
            .lock()
            .execute(
                "DELETE FROM lnmarkets_market_events WHERE received_at_ms < ?1",
                params![cutoff],
            )
            .map_err(Into::into)
    }

    pub fn set_state(&self, network: Network, key: &str, value: &str) -> Result<()> {
        self.connection.lock().execute(
            "INSERT INTO lnmarkets_collector_state(network, state_key, state_value)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(network, state_key) DO UPDATE SET state_value = excluded.state_value",
            params![network_name(network), key, value],
        )?;
        Ok(())
    }

    pub fn state(&self, network: Network, key: &str) -> Result<Option<String>> {
        self.connection
            .lock()
            .query_row(
                "SELECT state_value FROM lnmarkets_collector_state
                 WHERE network = ?1 AND state_key = ?2",
                params![network_name(network), key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_account_allocation(
        &self,
        network: Network,
        allocation: AccountAllocation,
    ) -> Result<()> {
        let allocation = allocation.validate()?;
        self.set_state(
            network,
            "account_allocation",
            &serde_json::to_string(&allocation)?,
        )?;
        self.refresh_features(network)
    }

    pub fn feature_snapshot(&self, network: Network) -> Result<Option<FeatureSnapshot>> {
        let snapshot_json = self
            .connection
            .lock()
            .query_row(
                "SELECT snapshot_json FROM lnmarkets_feature_snapshots WHERE network = ?1",
                params![network_name(network)],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        snapshot_json
            .map(|snapshot| serde_json::from_str(&snapshot).map_err(Into::into))
            .transpose()
    }

    pub fn feature_replay(
        &self,
        network: Network,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<FeatureReplayDataset> {
        if from_ms < 0 || to_ms < from_ms {
            bail!("feature replay timestamps are invalid");
        }
        let candles = self.range(network, CANDLE_TOPIC, from_ms, Some(to_ms), 10_000)?;
        if candles.is_empty() {
            bail!("feature replay requires collected candles");
        }
        let funding_settlements = self.range(
            network,
            FUNDING_SETTLEMENT_TOPIC,
            from_ms,
            Some(to_ms),
            10_000,
        )?;
        if funding_settlements.is_empty() {
            bail!("feature replay requires collected funding settlements");
        }
        let oracle_indices =
            self.range(network, ORACLE_INDEX_TOPIC, from_ms, Some(to_ms), 10_000)?;
        if oracle_indices.is_empty() {
            bail!("feature replay requires collected oracle indices");
        }

        let candle_count =
            u64::try_from(candles.len()).context("feature replay candle count overflowed")?;
        let oracle_index_count = u64::try_from(oracle_indices.len())
            .context("feature replay oracle index count overflowed")?;
        let funding_settlement_count = u64::try_from(funding_settlements.len())
            .context("feature replay funding count overflowed")?;
        let mut changes = BTreeMap::<i64, (Vec<f64>, Vec<f64>, Vec<f64>)>::new();
        for candle in candles {
            let close = event_numeric_value(&candle.payload, &["close"]).with_context(|| {
                format!("candle at {} has no close price", candle.event_time_ms)
            })?;
            changes
                .entry(candle.event_time_ms)
                .or_default()
                .0
                .push(close);
        }
        for settlement in funding_settlements {
            let rate = funding_rate(&settlement.payload).with_context(|| {
                format!(
                    "funding settlement at {} has no funding rate",
                    settlement.event_time_ms
                )
            })?;
            changes
                .entry(settlement.event_time_ms)
                .or_default()
                .1
                .push(rate);
        }
        for index in oracle_indices {
            let value = event_numeric_value(&index.payload, &["index"]).with_context(|| {
                format!("oracle index at {} has no index price", index.event_time_ms)
            })?;
            changes
                .entry(index.event_time_ms)
                .or_default()
                .2
                .push(value);
        }

        let mut input = FeatureInput::default();
        let mut ticks = Vec::with_capacity(changes.len());
        for (occurred_at_ms, (prices, rates, index_prices)) in changes {
            input
                .prices
                .extend(prices.into_iter().map(|value| TimedValue {
                    time_ms: occurred_at_ms,
                    value,
                }));
            input
                .funding_rates
                .extend(rates.into_iter().map(|value| TimedValue {
                    time_ms: occurred_at_ms,
                    value,
                }));
            input
                .index_prices
                .extend(index_prices.into_iter().map(|value| TimedValue {
                    time_ms: occurred_at_ms,
                    value,
                }));
            if input.prices.len() > 1_000 {
                input.prices.drain(..input.prices.len() - 1_000);
            }
            if input.funding_rates.len() > 1_000 {
                input
                    .funding_rates
                    .drain(..input.funding_rates.len() - 1_000);
            }
            if input.index_prices.len() > 1_000 {
                input.index_prices.drain(..input.index_prices.len() - 1_000);
            }
            ticks.push(FeatureReplayTick {
                occurred_at_ms,
                features: derive_features(input.clone())?,
            });
        }

        Ok(FeatureReplayDataset {
            schema: "omega.lnmarkets.feature_replay.v1".into(),
            network,
            from_ms,
            to_ms,
            candle_count,
            oracle_index_count,
            funding_settlement_count,
            ticks,
        })
    }

    fn refresh_features(&self, network: Network) -> Result<()> {
        let snapshot = derive_features(self.feature_input(network)?)?;
        let computed_at_ms = Utc::now().timestamp_millis();
        let snapshot_json = serde_json::to_string(&snapshot)?;
        self.connection.lock().execute(
            "INSERT INTO lnmarkets_feature_snapshots(network, computed_at_ms, snapshot_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(network) DO UPDATE SET
                 computed_at_ms = excluded.computed_at_ms,
                 snapshot_json = excluded.snapshot_json",
            params![network_name(network), computed_at_ms, snapshot_json],
        )?;
        Ok(())
    }

    fn feature_input(&self, network: Network) -> Result<FeatureInput> {
        let mut prices = self
            .recent(network, STREAM_OHLC_ONE_MINUTE_TOPIC, 2_000)?
            .into_iter()
            .chain(self.recent(network, CANDLE_TOPIC, 1_000)?)
            .filter_map(|event| {
                event_numeric_value(&event.payload, &["close", "lastPrice", "index"]).map(|value| {
                    TimedValue {
                        time_ms: event.event_time_ms,
                        value,
                    }
                })
            })
            .collect::<Vec<_>>();
        for topic in [
            STREAM_LAST_PRICE_TOPIC,
            "futures/inverse/btc_usd/index",
            "futures/inverse/btc_usd/ticker",
        ] {
            prices.extend(
                self.recent(network, topic, 1)?
                    .into_iter()
                    .filter_map(|event| {
                        event_numeric_value(&event.payload, &["lastPrice", "index", "close"]).map(
                            |value| TimedValue {
                                time_ms: event.event_time_ms,
                                value,
                            },
                        )
                    }),
            );
        }
        let mut index_prices = self
            .recent(network, ORACLE_INDEX_TOPIC, 1_000)?
            .into_iter()
            .filter_map(|event| {
                event_numeric_value(&event.payload, &["index"]).map(|value| TimedValue {
                    time_ms: event.event_time_ms,
                    value,
                })
            })
            .collect::<Vec<_>>();
        for topic in [
            "futures/inverse/btc_usd/index",
            "futures/inverse/btc_usd/ticker",
        ] {
            index_prices.extend(
                self.recent(network, topic, 1)?
                    .into_iter()
                    .filter_map(|event| {
                        event_numeric_value(&event.payload, &["index"]).map(|value| TimedValue {
                            time_ms: event.event_time_ms,
                            value,
                        })
                    }),
            );
        }
        let funding_rates = self
            .recent(network, FUNDING_SETTLEMENT_TOPIC, 1_000)?
            .into_iter()
            .chain(self.recent(network, STREAM_FUNDING_TOPIC, 1_000)?)
            .chain(self.recent(network, "futures/inverse/btc_usd/ticker", 1_000)?)
            .filter_map(|event| {
                funding_rate(&event.payload).map(|value| TimedValue {
                    time_ms: event.event_time_ms,
                    value,
                })
            })
            .collect();
        let buckets = self.recent(network, STREAM_BUCKETS_TOPIC, 1)?;
        let liquidity_time_ms = buckets.first().map(|event| event.event_time_ms);
        let liquidity_tiers = buckets
            .first()
            .map(|event| parse_liquidity_tiers(&event.payload))
            .unwrap_or_default();
        let account = self
            .state(network, "account_allocation")?
            .map(|allocation| serde_json::from_str(&allocation))
            .transpose()?;
        Ok(FeatureInput {
            prices,
            index_prices,
            funding_rates,
            liquidity_time_ms,
            liquidity_tiers,
            account,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectorStatus {
    Starting,
    Backfilling,
    Connecting,
    Streaming,
    Degraded,
    Stopped,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectorHealth {
    pub network: Network,
    pub status: CollectorStatus,
    pub authenticated: bool,
    pub subscribed_topics: BTreeSet<String>,
    pub last_event_by_topic_ms: BTreeMap<String, i64>,
    pub last_backfill_at_ms: Option<i64>,
    pub backfill_completed_surfaces: usize,
    pub backfill_total_surfaces: usize,
    pub backfill_rows: usize,
    pub last_stream_event_at_ms: Option<i64>,
    pub lag_ms: Option<i64>,
    pub stored_events: u64,
    pub last_error: Option<String>,
}

impl CollectorHealth {
    fn starting(network: Network, authenticated: bool) -> Self {
        Self {
            network,
            status: CollectorStatus::Starting,
            authenticated,
            subscribed_topics: BTreeSet::new(),
            last_event_by_topic_ms: BTreeMap::new(),
            last_backfill_at_ms: None,
            backfill_completed_surfaces: 0,
            backfill_total_surfaces: 3,
            backfill_rows: 0,
            last_stream_event_at_ms: None,
            lag_ms: None,
            stored_events: 0,
            last_error: None,
        }
    }
}

#[derive(Clone)]
pub struct CollectorHandle {
    store: MarketDataStore,
    health: Arc<Mutex<CollectorHealth>>,
}

impl CollectorHandle {
    pub fn health(&self) -> CollectorHealth {
        self.health.lock().clone()
    }

    pub fn store(&self) -> &MarketDataStore {
        &self.store
    }

    pub fn recent(&self, topic: &str, limit: usize) -> Result<Vec<StoredMarketEvent>> {
        let network = self.health.lock().network;
        self.store.recent(network, topic, limit)
    }

    pub fn history(&self, from: &str, to: Option<&str>, limit: usize) -> Result<CollectorHistory> {
        let from_ms = parse_iso_timestamp(from)?;
        let to_ms = to.map(parse_iso_timestamp).transpose()?;
        let network = self.health.lock().network;
        Ok(CollectorHistory {
            network,
            candle_resolution: CandleResolution::OneHour.to_string(),
            candles: self
                .store
                .range(network, CANDLE_TOPIC, from_ms, to_ms, limit)?,
            funding_settlements: self.store.range(
                network,
                FUNDING_SETTLEMENT_TOPIC,
                from_ms,
                to_ms,
                limit,
            )?,
            oracle_indices: self
                .store
                .range(network, ORACLE_INDEX_TOPIC, from_ms, to_ms, limit)?,
        })
    }

    pub fn features(&self) -> Result<Option<FeatureSnapshot>> {
        let network = self.health.lock().network;
        self.store.feature_snapshot(network)
    }

    pub fn set_account_allocation(&self, allocation: AccountAllocation) -> Result<()> {
        let network = self.health.lock().network;
        self.store.set_account_allocation(network, allocation)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectorHistory {
    pub network: Network,
    pub candle_resolution: String,
    pub candles: Vec<StoredMarketEvent>,
    pub funding_settlements: Vec<StoredMarketEvent>,
    pub oracle_indices: Vec<StoredMarketEvent>,
}

#[derive(Clone)]
pub struct CollectorConfig {
    pub network: Network,
    pub credentials: Option<Credentials>,
    pub retention: Duration,
    pub candle_resolution: CandleResolution,
}

impl CollectorConfig {
    pub fn public(network: Network) -> Self {
        Self {
            network,
            credentials: None,
            retention: DEFAULT_RETENTION,
            candle_resolution: CandleResolution::OneHour,
        }
    }

    pub fn authenticated(network: Network, credentials: Credentials) -> Self {
        Self {
            credentials: Some(credentials),
            ..Self::public(network)
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BackfillReport {
    pub candles: usize,
    pub funding_settlements: usize,
    pub oracle_indices: usize,
    pub errors: BTreeMap<String, String>,
}

pub struct Collector {
    client: LnMarketsClient,
    store: MarketDataStore,
    config: CollectorConfig,
    health: Arc<Mutex<CollectorHealth>>,
}

impl Collector {
    pub fn new(client: LnMarketsClient, store: MarketDataStore, config: CollectorConfig) -> Self {
        let authenticated = config.credentials.is_some();
        Self {
            client,
            store,
            health: Arc::new(Mutex::new(CollectorHealth::starting(
                config.network,
                authenticated,
            ))),
            config,
        }
    }

    pub fn handle(&self) -> CollectorHandle {
        CollectorHandle {
            store: self.store.clone(),
            health: self.health.clone(),
        }
    }

    pub async fn backfill_once(&self) -> BackfillReport {
        self.update_health(|health| {
            health.status = CollectorStatus::Backfilling;
            health.last_error = None;
            health.backfill_completed_surfaces = 0;
            health.backfill_rows = 0;
        });
        let from = retention_cutoff(self.config.retention);
        let mut report = BackfillReport::default();

        if let Err(error) = self.backfill_candles(&from, &mut report).await {
            report.errors.insert("candles".into(), error.to_string());
        }
        self.update_backfill_progress(1, &report);
        if let Err(error) = self.backfill_funding(&from, &mut report).await {
            report
                .errors
                .insert("funding_settlements".into(), error.to_string());
        }
        self.update_backfill_progress(2, &report);
        if let Err(error) = self.backfill_oracle(&from, &mut report).await {
            report
                .errors
                .insert("oracle_index".into(), error.to_string());
        }
        self.update_backfill_progress(3, &report);
        if let Err(error) = self.store.prune(Utc::now().timestamp_millis()) {
            report.errors.insert("retention".into(), error.to_string());
        }

        let now = Utc::now().timestamp_millis();
        if let Err(error) =
            self.store
                .set_state(self.config.network, "last_backfill_at_ms", &now.to_string())
        {
            report.errors.insert("state".into(), error.to_string());
        }
        let stored_events = match self.store.event_count(self.config.network) {
            Ok(count) => count,
            Err(error) => {
                report
                    .errors
                    .insert("event_count".into(), error.to_string());
                0
            }
        };
        self.update_health(|health| {
            health.last_backfill_at_ms = Some(now);
            health.stored_events = stored_events;
            if report.errors.is_empty() {
                health.status = CollectorStatus::Connecting;
            } else {
                health.status = CollectorStatus::Degraded;
                health.last_error = Some(format_backfill_errors(&report.errors));
            }
        });
        report
    }

    pub async fn run(self) {
        loop {
            let report = self.backfill_once().await;
            if !report.errors.is_empty() {
                log_backfill_errors(&report.errors);
            }
            if let Err(error) = self.stream_until_failure().await {
                self.update_health(|health| {
                    health.status = CollectorStatus::Degraded;
                    health.last_error = Some(error.to_string());
                });
                log::warn!("LN Markets collector stream failed: {error:#}");
            }
            async_io::Timer::after(STREAM_RESTART_DELAY).await;
        }
    }

    async fn backfill_candles(&self, from: &str, report: &mut BackfillReport) -> Result<()> {
        let mut query = CandlesQuery {
            from_: from.to_owned(),
            to: None,
            limit: Some(DEFAULT_BACKFILL_LIMIT),
            cursor: None,
            resolution: self.config.candle_resolution,
        };
        for page_number in 0..MAX_BACKFILL_PAGES {
            let page = self.client.candles(&query).await?;
            let events = page
                .data
                .iter()
                .map(|candle| backfill_event(&candle.time, candle))
                .collect::<Result<Vec<_>>>()?;
            self.store
                .insert_backfill_batch(self.config.network, CANDLE_TOPIC, &events)?;
            report.candles = report.candles.saturating_add(events.len());
            match query.next_page(&page) {
                Some(next) => query = next,
                None => return Ok(()),
            }
            if page_number + 1 == MAX_BACKFILL_PAGES {
                bail!("candle backfill exceeded {MAX_BACKFILL_PAGES} pages");
            }
        }
        Ok(())
    }

    async fn backfill_funding(&self, from: &str, report: &mut BackfillReport) -> Result<()> {
        let mut pagination = Pagination::default()
            .with_time_range(from.to_owned(), None)
            .with_limit(DEFAULT_BACKFILL_LIMIT)?;
        for page_number in 0..MAX_BACKFILL_PAGES {
            let page = self.client.funding_settlements(&pagination).await?;
            let events = page
                .data
                .iter()
                .map(|settlement| backfill_event(&settlement.time, settlement))
                .collect::<Result<Vec<_>>>()?;
            self.store.insert_backfill_batch(
                self.config.network,
                FUNDING_SETTLEMENT_TOPIC,
                &events,
            )?;
            report.funding_settlements = report.funding_settlements.saturating_add(events.len());
            match page.next_page(&pagination) {
                Some(next) => pagination = next,
                None => return Ok(()),
            }
            if page_number + 1 == MAX_BACKFILL_PAGES {
                bail!("funding backfill exceeded {MAX_BACKFILL_PAGES} pages");
            }
        }
        Ok(())
    }

    async fn backfill_oracle(&self, from: &str, report: &mut BackfillReport) -> Result<()> {
        let pagination = Pagination::default()
            .with_time_range(from.to_owned(), None)
            .with_limit(DEFAULT_BACKFILL_LIMIT)?;
        let indices = self.client.oracle_index(&pagination).await?;
        let events = indices
            .iter()
            .map(|index| backfill_event(&index.time, index))
            .collect::<Result<Vec<_>>>()?;
        self.store
            .insert_backfill_batch(self.config.network, ORACLE_INDEX_TOPIC, &events)?;
        report.oracle_indices = report.oracle_indices.saturating_add(events.len());
        Ok(())
    }

    async fn stream_until_failure(&self) -> Result<()> {
        self.update_health(|health| health.status = CollectorStatus::Connecting);
        let mut stream = LnMarketsStreamClient::connect(self.config.network).await?;
        stream
            .hello("omega-lnmarkets-collector", env!("CARGO_PKG_VERSION"))
            .await?;
        if let Some(credentials) = &self.config.credentials {
            let authentication = stream.authenticate(credentials).await?;
            if !authentication.authenticated {
                bail!("LN Markets stream rejected collector authentication");
            }
        }
        let topics = collector_topics(self.config.credentials.is_some())?;
        let subscription = stream.subscribe(&topics).await?;
        let subscribed_topics = subscription
            .subscribed
            .iter()
            .map(|topic| topic.as_str().to_owned())
            .collect();
        self.update_health(|health| {
            health.status = CollectorStatus::Streaming;
            health.subscribed_topics = subscribed_topics;
            health.last_error = None;
        });

        loop {
            let events = stream
                .collect_events(STREAM_BATCH_SIZE, STREAM_BATCH_TIMEOUT)
                .await?;
            if events.is_empty() {
                continue;
            }
            let inserted = self
                .store
                .insert_stream_batch(self.config.network, &events)?;
            let now = Utc::now().timestamp_millis();
            let newest_event_ms = events
                .iter()
                .filter_map(|event| event_timestamp_ms(&event.data))
                .max()
                .unwrap_or(now);
            let event_updates = events
                .iter()
                .map(|event| {
                    (
                        event.topic.as_str().to_owned(),
                        event_timestamp_ms(&event.data).unwrap_or(now),
                    )
                })
                .collect::<Vec<_>>();
            let stored_events = self.store.event_count(self.config.network)?;
            self.update_health(|health| {
                health.status = CollectorStatus::Streaming;
                health.last_stream_event_at_ms = Some(now);
                health.lag_ms = Some(now.saturating_sub(newest_event_ms));
                health.stored_events = stored_events;
                health.last_error = None;
                for (topic, event_time_ms) in &event_updates {
                    health
                        .last_event_by_topic_ms
                        .insert(topic.clone(), *event_time_ms);
                }
            });
            if inserted > 0 {
                self.store.prune(now)?;
            }
        }
    }

    fn update_health(&self, update: impl FnOnce(&mut CollectorHealth)) {
        update(&mut self.health.lock());
    }

    fn update_backfill_progress(&self, completed_surfaces: usize, report: &BackfillReport) {
        let rows = report
            .candles
            .saturating_add(report.funding_settlements)
            .saturating_add(report.oracle_indices);
        self.update_health(|health| {
            health.backfill_completed_surfaces = completed_surfaces;
            health.backfill_rows = rows;
        });
    }
}

pub fn collector_topics(authenticated: bool) -> Result<Vec<StreamTopic>> {
    let mut names = vec![
        "futures/inverse/btc_usd/ticker",
        "futures/inverse/btc_usd/lastPrice",
        "futures/inverse/btc_usd/index",
        "futures/inverse/btc_usd/buckets",
        "futures/inverse/btc_usd/funding",
        "futures/inverse/btc_usd/ohlc/1m",
    ];
    if authenticated {
        names.extend([
            "announcements",
            "wallet/deposit",
            "wallet/withdrawal",
            "futures/inverse/btc_usd/isolated/trades",
            "futures/inverse/btc_usd/cross/orders",
            "futures/inverse/btc_usd/cross/position",
        ]);
    }
    names
        .into_iter()
        .map(StreamTopic::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn network_name(network: Network) -> &'static str {
    match network {
        Network::Signet => "signet",
        Network::Mainnet => "mainnet",
    }
}

fn backfill_event<T: Serialize>(time: &str, value: &T) -> Result<(i64, Value)> {
    Ok((parse_iso_timestamp(time)?, serde_json::to_value(value)?))
}

fn parse_source(source: &str) -> Result<EventSource> {
    match source {
        "rest_backfill" => Ok(EventSource::RestBackfill),
        "stream" => Ok(EventSource::Stream),
        _ => Err(anyhow!("unknown LN Markets event source {source:?}")),
    }
}

fn event_numeric_value(payload: &Value, fields: &[&str]) -> Option<f64> {
    fields
        .iter()
        .find_map(|field| payload.get(*field).and_then(json_number))
}

fn funding_rate(payload: &Value) -> Option<f64> {
    payload
        .get("fundingRate")
        .and_then(json_number)
        .or_else(|| {
            payload
                .get("current")
                .and_then(|current| current.get("rate"))
                .and_then(json_number)
        })
        .or_else(|| {
            payload
                .get("funding")
                .and_then(|funding| funding.get("rate"))
                .and_then(json_number)
        })
}

fn json_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        .filter(|value| value.is_finite())
}

fn parse_liquidity_tiers(payload: &Value) -> Vec<LiquidityTier> {
    payload
        .get("buckets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tier| {
            Some(LiquidityTier {
                min_size: tier.get("minSize").and_then(json_number)?,
                max_size: tier.get("maxSize").and_then(json_number)?,
                bid_price: tier.get("bidPrice").and_then(json_number),
                ask_price: tier.get("askPrice").and_then(json_number),
            })
        })
        .collect()
}

fn event_key(
    topic: &str,
    event_time_ms: i64,
    source: EventSource,
    payload: &Value,
) -> Result<String> {
    let payload = serde_json::to_vec(payload)?;
    let mut hasher = Sha256::new();
    hasher.update(topic.as_bytes());
    hasher.update(event_time_ms.to_be_bytes());
    hasher.update(source.as_str().as_bytes());
    hasher.update(payload);
    Ok(format!("{:x}", hasher.finalize()))
}

fn parse_iso_timestamp(time: &str) -> Result<i64> {
    DateTime::parse_from_rfc3339(time)
        .with_context(|| format!("invalid LN Markets timestamp {time:?}"))
        .map(|time| time.timestamp_millis())
}

fn event_timestamp_ms(payload: &Value) -> Option<i64> {
    [
        "time",
        "timestamp",
        "createdAt",
        "updatedAt",
        "settledAt",
        "filledAt",
    ]
    .into_iter()
    .find_map(|field| payload.get(field).and_then(timestamp_value_ms))
}

fn timestamp_value_ms(value: &Value) -> Option<i64> {
    if let Some(value) = value.as_i64() {
        return Some(if value < 10_000_000_000 {
            value.saturating_mul(1_000)
        } else {
            value
        });
    }
    if let Some(value) = value.as_u64() {
        let value = i64::try_from(value).ok()?;
        return Some(if value < 10_000_000_000 {
            value.saturating_mul(1_000)
        } else {
            value
        });
    }
    value
        .as_str()
        .and_then(|value| parse_iso_timestamp(value).ok())
}

fn retention_cutoff(retention: Duration) -> String {
    let retention = chrono::Duration::from_std(retention).unwrap_or(chrono::Duration::MAX);
    Utc::now()
        .checked_sub_signed(retention)
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn format_backfill_errors(errors: &BTreeMap<String, String>) -> String {
    errors
        .iter()
        .map(|(surface, error)| format!("{surface}: {error}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn log_backfill_errors(errors: &BTreeMap<String, String>) {
    log::warn!(
        "LN Markets collector backfill completed with errors: {}",
        format_backfill_errors(errors)
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::{FutureExt as _, future::BoxFuture};
    use http::{Method, Request, Response, StatusCode};
    use lnmarkets_client::{HttpTransport, TransportFailure, TransportFailureKind};
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct BackfillTransport {
        requests: Mutex<Vec<String>>,
    }

    impl HttpTransport for BackfillTransport {
        fn send(
            &self,
            request: Request<Vec<u8>>,
        ) -> BoxFuture<'static, Result<Response<Vec<u8>>, TransportFailure>> {
            let path = request.uri().path().to_owned();
            let method = request.method().clone();
            self.requests.lock().push(path.clone());
            async move {
                if method != Method::GET {
                    return Err(TransportFailure::new(
                        TransportFailureKind::Other,
                        anyhow!("unexpected method {method}"),
                    ));
                }
                let body = match path.as_str() {
                    "/v3/futures/candles" => json!({
                        "data": [{
                            "close": 100.0,
                            "high": 101.0,
                            "low": 98.0,
                            "open": 99.0,
                            "time": "2026-08-09T00:00:00.000Z",
                            "volume": 42
                        }],
                        "nextCursor": null
                    }),
                    "/v3/futures/funding-settlements" => json!({
                        "data": [{
                            "id": "funding-1",
                            "time": "2026-08-09T00:00:00.000Z",
                            "fundingRate": 0.0001,
                            "fixingPrice": 100.0
                        }],
                        "nextCursor": null
                    }),
                    "/v3/oracle/index" => json!([{
                        "index": 100.0,
                        "time": "2026-08-09T00:00:00.000Z"
                    }]),
                    _ => {
                        return Response::builder()
                            .status(StatusCode::NOT_FOUND)
                            .body(Vec::new())
                            .map_err(|error| {
                                TransportFailure::new(TransportFailureKind::Other, error)
                            });
                    }
                };
                Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(serde_json::to_vec(&body).map_err(|error| {
                        TransportFailure::new(TransportFailureKind::Other, error)
                    })?)
                    .map_err(|error| TransportFailure::new(TransportFailureKind::Other, error))
            }
            .boxed()
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-10,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn realized_volatility_uses_log_returns_inside_each_window() {
        let snapshot = derive_features(FeatureInput {
            prices: vec![
                TimedValue {
                    time_ms: 0,
                    value: 100.0,
                },
                TimedValue {
                    time_ms: ONE_HOUR_MS / 2,
                    value: 110.0,
                },
                TimedValue {
                    time_ms: 2 * ONE_HOUR_MS,
                    value: 121.0,
                },
            ],
            ..FeatureInput::default()
        })
        .expect("features");

        assert_eq!(snapshot.volatility.one_hour, None);
        let expected = (2.0_f64 * (1.1_f64.ln()).powi(2)).sqrt();
        assert_close(
            snapshot.volatility.six_hours.expect("six-hour volatility"),
            expected,
        );
        assert_close(
            snapshot.volatility.one_day.expect("one-day volatility"),
            expected,
        );
    }

    #[test]
    fn index_features_measure_log_moves_inside_each_window() {
        let snapshot = derive_features(FeatureInput {
            index_prices: vec![
                TimedValue {
                    time_ms: 0,
                    value: 80.0,
                },
                TimedValue {
                    time_ms: 23 * ONE_HOUR_MS + ONE_HOUR_MS / 2,
                    value: 100.0,
                },
                TimedValue {
                    time_ms: 24 * ONE_HOUR_MS + ONE_HOUR_MS / 2,
                    value: 110.0,
                },
                TimedValue {
                    time_ms: 25 * ONE_HOUR_MS,
                    value: 121.0,
                },
            ],
            ..FeatureInput::default()
        })
        .expect("features");

        assert_eq!(snapshot.index.current_price, Some(121.0));
        assert_close(
            snapshot.index.one_hour_move.expect("one-hour move"),
            (121.0_f64 / 110.0).ln(),
        );
        let longer_move = (121.0_f64 / 100.0).ln();
        assert_close(
            snapshot.index.six_hours_move.expect("six-hour move"),
            longer_move,
        );
        assert_close(
            snapshot.index.one_day_move.expect("one-day move"),
            longer_move,
        );
    }

    #[test]
    fn funding_features_pin_ema_sign_and_latest_flip() {
        let snapshot = derive_features(FeatureInput {
            funding_rates: vec![
                TimedValue {
                    time_ms: 1,
                    value: 0.01,
                },
                TimedValue {
                    time_ms: 2,
                    value: 0.02,
                },
                TimedValue {
                    time_ms: 3,
                    value: -0.01,
                },
            ],
            ..FeatureInput::default()
        })
        .expect("features");

        assert_close(snapshot.funding.ema.expect("funding EMA"), 0.0076);
        assert_eq!(snapshot.funding.current_rate, Some(-0.01));
        assert_eq!(snapshot.funding.sign, FundingSign::Negative);
        assert_eq!(snapshot.funding.sign_flipped_at_ms, Some(3));
    }

    #[test]
    fn feature_replay_requires_both_histories_and_uses_live_derivation() {
        let store = MarketDataStore::in_memory(Duration::from_secs(60)).expect("store");
        store
            .insert_backfill_batch(
                Network::Signet,
                CANDLE_TOPIC,
                &[
                    (100, json!({"close": 100.0})),
                    (300, json!({"close": 110.0})),
                ],
            )
            .expect("candles");
        let missing_funding = store
            .feature_replay(Network::Signet, 0, 400)
            .expect_err("funding history required");
        assert!(
            missing_funding
                .to_string()
                .contains("requires collected funding settlements")
        );
        store
            .insert_backfill_batch(
                Network::Signet,
                FUNDING_SETTLEMENT_TOPIC,
                &[
                    (200, json!({"fundingRate": 0.01})),
                    (400, json!({"fundingRate": -0.02})),
                ],
            )
            .expect("funding");
        let missing_indices = store
            .feature_replay(Network::Signet, 0, 400)
            .expect_err("oracle index history required");
        assert!(
            missing_indices
                .to_string()
                .contains("requires collected oracle indices")
        );
        store
            .insert_backfill_batch(
                Network::Signet,
                ORACLE_INDEX_TOPIC,
                &[
                    (100, json!({"index": 100.0})),
                    (300, json!({"index": 110.0})),
                ],
            )
            .expect("oracle indices");

        let replay = store
            .feature_replay(Network::Signet, 0, 400)
            .expect("feature replay");
        assert_eq!(replay.candle_count, 2);
        assert_eq!(replay.oracle_index_count, 2);
        assert_eq!(replay.funding_settlement_count, 2);
        assert_eq!(
            replay
                .ticks
                .iter()
                .map(|tick| tick.occurred_at_ms)
                .collect::<Vec<_>>(),
            vec![100, 200, 300, 400]
        );
        let final_features = &replay.ticks.last().expect("final tick").features;
        assert_eq!(final_features.volatility.price_points, 2);
        assert_eq!(final_features.index.current_price, Some(110.0));
        assert_eq!(final_features.index.price_points, 2);
        assert_eq!(final_features.funding.current_rate, Some(-0.02));
        assert_eq!(final_features.funding.sign, FundingSign::Negative);
        assert_eq!(final_features.funding.sign_flipped_at_ms, Some(400));
    }

    #[test]
    fn liquidity_features_measure_spread_and_available_depth() {
        let snapshot = derive_features(FeatureInput {
            liquidity_tiers: vec![
                LiquidityTier {
                    min_size: 0.0,
                    max_size: 1_000.0,
                    bid_price: Some(99.0),
                    ask_price: Some(101.0),
                },
                LiquidityTier {
                    min_size: 1_000.0,
                    max_size: 10_000.0,
                    bid_price: Some(98.0),
                    ask_price: Some(102.0),
                },
            ],
            ..FeatureInput::default()
        })
        .expect("features");

        assert_eq!(snapshot.liquidity.best_bid, Some(99.0));
        assert_eq!(snapshot.liquidity.best_ask, Some(101.0));
        assert_eq!(snapshot.liquidity.spread, Some(2.0));
        assert_eq!(snapshot.liquidity.spread_bps, Some(200.0));
        assert_eq!(snapshot.liquidity.bid_depth, Some(10_000.0));
        assert_eq!(snapshot.liquidity.ask_depth, Some(10_000.0));
        assert_eq!(snapshot.liquidity.tier_count, 2);
    }

    #[test]
    fn account_drift_compares_current_and_target_btc_weights() {
        let snapshot = derive_features(FeatureInput {
            prices: vec![TimedValue {
                time_ms: 1,
                value: 50_000.0,
            }],
            account: Some(AccountAllocation {
                btc_sats: 100_000_000.0,
                synthetic_usd: 50_000.0,
                target_btc_weight: 0.6,
            }),
            ..FeatureInput::default()
        })
        .expect("features");
        let drift = snapshot.account_drift.expect("account drift");

        assert_eq!(drift.btc_value_usd, 50_000.0);
        assert_eq!(drift.synthetic_usd, 50_000.0);
        assert_eq!(drift.current_btc_weight, 0.5);
        assert_eq!(drift.target_btc_weight, 0.6);
        assert_close(drift.drift, -0.1);
    }

    #[test]
    fn store_materializes_feature_snapshot_on_write() {
        let store = MarketDataStore::in_memory(Duration::from_secs(60)).expect("store");
        store
            .insert_stream_batch(
                Network::Signet,
                &[
                    StreamEvent {
                        topic: StreamTopic::new(STREAM_OHLC_ONE_MINUTE_TOPIC).expect("topic"),
                        data: json!({"time": 1_000, "close": 100.0}),
                    },
                    StreamEvent {
                        topic: StreamTopic::new(STREAM_OHLC_ONE_MINUTE_TOPIC).expect("topic"),
                        data: json!({"time": 2_000, "close": 110.0}),
                    },
                    StreamEvent {
                        topic: StreamTopic::new(STREAM_FUNDING_TOPIC).expect("topic"),
                        data: json!({"time": 2_000, "current": {"rate": 0.001}}),
                    },
                    StreamEvent {
                        topic: StreamTopic::new(STREAM_BUCKETS_TOPIC).expect("topic"),
                        data: json!({
                            "time": 2_000,
                            "buckets": [{
                                "minSize": 0,
                                "maxSize": 10_000,
                                "bidPrice": 109,
                                "askPrice": 111
                            }]
                        }),
                    },
                ],
            )
            .expect("stream batch");
        store
            .set_account_allocation(
                Network::Signet,
                AccountAllocation {
                    btc_sats: 100_000_000.0,
                    synthetic_usd: 110.0,
                    target_btc_weight: 0.6,
                },
            )
            .expect("account allocation");

        let snapshot = store
            .feature_snapshot(Network::Signet)
            .expect("snapshot read")
            .expect("snapshot");
        assert_eq!(snapshot.schema, "omega.lnmarkets.features.v1");
        assert_eq!(snapshot.volatility.price_points, 2);
        assert_eq!(snapshot.funding.sign, FundingSign::Positive);
        assert_eq!(snapshot.liquidity.spread, Some(2.0));
        assert_close(
            snapshot
                .account_drift
                .expect("account drift")
                .current_btc_weight,
            0.5,
        );
    }

    #[test]
    fn store_persists_history_and_state() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("lnmarkets.db");
        let store = MarketDataStore::open(&path, Duration::from_secs(60)).expect("store");
        let payload = json!({"time": "2026-08-09T00:00:00.000Z", "price": 100});
        assert!(
            store
                .insert(
                    Network::Signet,
                    CANDLE_TOPIC,
                    1_786_233_600_000,
                    EventSource::RestBackfill,
                    &payload,
                )
                .expect("insert")
        );
        assert!(
            !store
                .insert(
                    Network::Signet,
                    CANDLE_TOPIC,
                    1_786_233_600_000,
                    EventSource::RestBackfill,
                    &payload,
                )
                .expect("duplicate")
        );
        store
            .set_state(Network::Signet, "cursor", "next")
            .expect("state");
        drop(store);

        let reopened = MarketDataStore::open(&path, Duration::from_secs(60)).expect("reopen");
        assert_eq!(reopened.event_count(Network::Signet).expect("count"), 1);
        assert_eq!(
            reopened.state(Network::Signet, "cursor").expect("state"),
            Some("next".into())
        );
        assert_eq!(
            reopened
                .recent(Network::Signet, CANDLE_TOPIC, 10)
                .expect("history")[0]
                .payload,
            payload
        );
    }

    #[test]
    fn retention_prunes_by_received_time() {
        let store = MarketDataStore::in_memory(Duration::from_secs(1)).expect("store");
        store
            .insert(
                Network::Signet,
                CANDLE_TOPIC,
                1,
                EventSource::RestBackfill,
                &json!({"time": 1}),
            )
            .expect("insert");
        assert_eq!(
            store
                .prune(Utc::now().timestamp_millis().saturating_add(2_000))
                .expect("prune"),
            1
        );
        assert_eq!(store.event_count(Network::Signet).expect("count"), 0);
    }

    #[test]
    fn history_range_is_filtered_and_chronological() {
        let store = MarketDataStore::in_memory(Duration::from_secs(60)).expect("store");
        for event_time_ms in [1_000, 2_000, 3_000] {
            store
                .insert(
                    Network::Signet,
                    CANDLE_TOPIC,
                    event_time_ms,
                    EventSource::RestBackfill,
                    &json!({"time": event_time_ms}),
                )
                .expect("insert");
        }
        let history = store
            .range(Network::Signet, CANDLE_TOPIC, 1_500, Some(3_000), 2)
            .expect("range");
        assert_eq!(
            history
                .iter()
                .map(|event| event.event_time_ms)
                .collect::<Vec<_>>(),
            [2_000, 3_000]
        );
    }

    #[test]
    fn collector_topic_set_adds_private_topics_only_after_authentication() {
        let public = collector_topics(false).expect("public topics");
        let authenticated = collector_topics(true).expect("authenticated topics");
        assert!(public.iter().all(|topic| !topic.is_private()));
        assert!(authenticated.len() > public.len());
        assert!(authenticated.iter().any(StreamTopic::is_private));
        assert!(
            public
                .iter()
                .any(|topic| topic.as_str() == "futures/inverse/btc_usd/ohlc/1m")
        );
    }

    #[test]
    fn stream_batch_updates_topic_history() {
        let store = MarketDataStore::in_memory(Duration::from_secs(60)).expect("store");
        let events = vec![StreamEvent {
            topic: StreamTopic::new("futures/inverse/btc_usd/ticker").expect("topic"),
            data: json!({"time": 1_786_233_600_000_i64, "lastPrice": 100}),
        }];
        assert_eq!(
            store
                .insert_stream_batch(Network::Signet, &events)
                .expect("insert"),
            1
        );
        let history = store
            .recent(Network::Signet, "futures/inverse/btc_usd/ticker", 10)
            .expect("history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].source, EventSource::Stream);
    }

    #[test]
    fn backfill_writes_every_public_history_surface() {
        futures::executor::block_on(async {
            let transport = Arc::new(BackfillTransport::default());
            let client = LnMarketsClient::public(transport.clone(), Network::Signet);
            let store = MarketDataStore::in_memory(Duration::from_secs(60)).expect("store");
            let collector = Collector::new(
                client,
                store.clone(),
                CollectorConfig::public(Network::Signet),
            );
            let report = collector.backfill_once().await;
            assert!(report.errors.is_empty(), "{:?}", report.errors);
            assert_eq!(report.candles, 1);
            assert_eq!(report.funding_settlements, 1);
            assert_eq!(report.oracle_indices, 1);
            assert_eq!(store.event_count(Network::Signet).expect("count"), 3);
            assert_eq!(
                transport.requests.lock().as_slice(),
                [
                    "/v3/futures/candles",
                    "/v3/futures/funding-settlements",
                    "/v3/oracle/index",
                ]
            );
            let health = collector.handle().health();
            assert_eq!(health.status, CollectorStatus::Connecting);
            assert_eq!(health.stored_events, 3);
            assert!(health.last_backfill_at_ms.is_some());
        });
    }

    #[test]
    fn timestamp_parser_accepts_stream_milliseconds_seconds_and_rest_iso() {
        assert_eq!(
            timestamp_value_ms(&json!(1_700_000_000)),
            Some(1_700_000_000_000)
        );
        assert_eq!(
            timestamp_value_ms(&json!(1_700_000_000_123_i64)),
            Some(1_700_000_000_123)
        );
        assert_eq!(
            timestamp_value_ms(&json!("2026-08-09T00:00:00.000Z")),
            Some(1_786_233_600_000)
        );
    }
}
