use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use agent_wakeup::WakeupSource;
use anyhow::{Context as _, Result, bail};
use parking_lot::Mutex;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const REVIEW_ACCOUNTING_SCHEMA_VERSION: u16 = 1;
const ONE_DAY_MS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

impl ReviewTokenUsage {
    pub fn input_total(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_creation_input_tokens)
            .saturating_add(self.cache_read_input_tokens)
    }

    pub fn total(&self) -> u64 {
        self.input_total().saturating_add(self.output_tokens)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewToolCall {
    pub name: String,
    pub input: Value,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewInterventionKind {
    ParameterChange,
    Intent,
    HaltResponse,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReviewDisposition {
    Intervention { kinds: Vec<ReviewInterventionKind> },
    NoChange,
}

impl ReviewDisposition {
    pub fn is_intervention(&self) -> bool {
        matches!(self, Self::Intervention { .. })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewCostRecord {
    pub schema_version: u16,
    pub turn_id: String,
    pub session_id: String,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub wall_clock_ms: u64,
    pub model_id: String,
    pub token_usage: ReviewTokenUsage,
    pub tool_calls: Vec<ReviewToolCall>,
    pub disposition: ReviewDisposition,
    pub source: WakeupSource,
    pub venues: Vec<String>,
    pub strategies: Vec<String>,
}

impl ReviewCostRecord {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != REVIEW_ACCOUNTING_SCHEMA_VERSION {
            bail!(
                "unsupported review accounting schema version {}",
                self.schema_version
            );
        }
        validate_label("turn ID", &self.turn_id)?;
        validate_label("session ID", &self.session_id)?;
        validate_label("model ID", &self.model_id)?;
        if self.started_at_ms < 0 || self.completed_at_ms < 0 {
            bail!("review timestamps must not be negative");
        }
        if self.completed_at_ms < self.started_at_ms {
            bail!("review completion precedes its start");
        }
        for (label, value) in [
            ("wall clock milliseconds", self.wall_clock_ms),
            ("input tokens", self.token_usage.input_tokens),
            ("output tokens", self.token_usage.output_tokens),
            (
                "cache-creation input tokens",
                self.token_usage.cache_creation_input_tokens,
            ),
            (
                "cache-read input tokens",
                self.token_usage.cache_read_input_tokens,
            ),
        ] {
            if value > i64::MAX as u64 {
                bail!("review {label} exceed the durable store range");
            }
        }
        if self.venues.is_empty() {
            bail!("a review cost record must name at least one venue");
        }
        validate_unique_labels("venue", &self.venues)?;
        validate_unique_labels("strategy", &self.strategies)?;
        for tool_call in &self.tool_calls {
            validate_label("tool name", &tool_call.name)?;
        }
        if let ReviewDisposition::Intervention { kinds } = &self.disposition {
            if kinds.is_empty() {
                bail!("an intervention must name at least one kind");
            }
            let unique_kinds = kinds.iter().copied().collect::<BTreeSet<_>>();
            if unique_kinds.len() != kinds.len() {
                bail!("an intervention repeats a kind");
            }
        }
        Ok(())
    }

    pub fn is_false_wakeup(&self) -> bool {
        !matches!(self.source, WakeupSource::ScheduledReview { .. })
            && matches!(self.disposition, ReviewDisposition::NoChange)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewCostEntry {
    pub sequence: u64,
    pub record: ReviewCostRecord,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReviewAccountingQuery {
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub venue: Option<String>,
    pub strategy: Option<String>,
}

impl ReviewAccountingQuery {
    fn validate(&self) -> Result<()> {
        if self.from_ms.is_some_and(|timestamp| timestamp < 0)
            || self.to_ms.is_some_and(|timestamp| timestamp < 0)
        {
            bail!("review accounting query timestamps must not be negative");
        }
        if self
            .from_ms
            .zip(self.to_ms)
            .is_some_and(|(from_ms, to_ms)| from_ms > to_ms)
        {
            bail!("review accounting query start exceeds its end");
        }
        if let Some(venue) = &self.venue {
            validate_label("venue", venue)?;
        }
        if let Some(strategy) = &self.strategy {
            validate_label("strategy", strategy)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewUnitCost {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub wall_clock_ms: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelReviewCost {
    pub model_id: String,
    pub review_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewCostSummary {
    pub review_count: u64,
    pub intervention_count: u64,
    pub no_change_count: u64,
    pub event_triggered_count: u64,
    pub false_wakeup_count: u64,
    pub false_wakeup_rate_bps: Option<u32>,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub total_wall_clock_ms: u64,
    pub cost_per_review: Option<ReviewUnitCost>,
    pub cost_per_intervention: Option<ReviewUnitCost>,
    pub models: Vec<ModelReviewCost>,
}

#[derive(Clone)]
pub struct ReviewAccountingStore {
    connection: Arc<Mutex<Connection>>,
}

impl ReviewAccountingStore {
    pub fn default_path() -> PathBuf {
        paths::data_dir()
            .join("threads")
            .join("review-accounting.db")
    }

    pub fn open_default() -> Result<Self> {
        Self::open(&Self::default_path())
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("could not create review accounting directory {parent:?}")
            })?;
        }
        let connection = Connection::open(path)
            .with_context(|| format!("could not open review accounting store {path:?}"))?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS review_cost_entries (
                 sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
                 schema_version INTEGER NOT NULL CHECK (schema_version > 0),
                 turn_id TEXT NOT NULL UNIQUE,
                 session_id TEXT NOT NULL,
                 started_at_ms INTEGER NOT NULL CHECK (started_at_ms >= 0),
                 completed_at_ms INTEGER NOT NULL CHECK (completed_at_ms >= started_at_ms),
                 wall_clock_ms INTEGER NOT NULL CHECK (wall_clock_ms >= 0),
                 model_id TEXT NOT NULL,
                 input_tokens INTEGER NOT NULL CHECK (input_tokens >= 0),
                 output_tokens INTEGER NOT NULL CHECK (output_tokens >= 0),
                 cache_creation_input_tokens INTEGER NOT NULL CHECK (cache_creation_input_tokens >= 0),
                 cache_read_input_tokens INTEGER NOT NULL CHECK (cache_read_input_tokens >= 0),
                 tool_calls_json TEXT NOT NULL,
                 disposition_json TEXT NOT NULL,
                 source_json TEXT NOT NULL,
                 venues_json TEXT NOT NULL,
                 strategies_json TEXT NOT NULL
             ) STRICT;",
        )?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn append(&self, record: ReviewCostRecord) -> Result<ReviewCostEntry> {
        record.validate()?;
        let tool_calls_json = serde_json::to_string(&record.tool_calls)?;
        let disposition_json = serde_json::to_string(&record.disposition)?;
        let source_json = serde_json::to_string(&record.source)?;
        let venues_json = serde_json::to_string(&record.venues)?;
        let strategies_json = serde_json::to_string(&record.strategies)?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let next_sequence: u64 = transaction.query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM review_cost_entries",
            [],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO review_cost_entries (
                sequence, schema_version, turn_id, session_id, started_at_ms, completed_at_ms,
                wall_clock_ms, model_id, input_tokens, output_tokens,
                cache_creation_input_tokens, cache_read_input_tokens, tool_calls_json,
                disposition_json, source_json, venues_json, strategies_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                next_sequence,
                record.schema_version,
                &record.turn_id,
                &record.session_id,
                record.started_at_ms,
                record.completed_at_ms,
                record.wall_clock_ms,
                &record.model_id,
                record.token_usage.input_tokens,
                record.token_usage.output_tokens,
                record.token_usage.cache_creation_input_tokens,
                record.token_usage.cache_read_input_tokens,
                tool_calls_json,
                disposition_json,
                source_json,
                venues_json,
                strategies_json,
            ],
        )?;
        transaction.commit()?;
        Ok(ReviewCostEntry {
            sequence: next_sequence,
            record,
        })
    }

    pub fn entries(&self, query: &ReviewAccountingQuery) -> Result<Vec<ReviewCostEntry>> {
        query.validate()?;
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT sequence, schema_version, turn_id, session_id, started_at_ms, completed_at_ms,
                    wall_clock_ms, model_id, input_tokens, output_tokens,
                    cache_creation_input_tokens, cache_read_input_tokens, tool_calls_json,
                    disposition_json, source_json, venues_json, strategies_json
             FROM review_cost_entries ORDER BY sequence",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, u16>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, u64>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, u64>(8)?,
                row.get::<_, u64>(9)?,
                row.get::<_, u64>(10)?,
                row.get::<_, u64>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, String>(13)?,
                row.get::<_, String>(14)?,
                row.get::<_, String>(15)?,
                row.get::<_, String>(16)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (
                sequence,
                schema_version,
                turn_id,
                session_id,
                started_at_ms,
                completed_at_ms,
                wall_clock_ms,
                model_id,
                input_tokens,
                output_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
                tool_calls_json,
                disposition_json,
                source_json,
                venues_json,
                strategies_json,
            ) = row?;
            let record = ReviewCostRecord {
                schema_version,
                turn_id,
                session_id,
                started_at_ms,
                completed_at_ms,
                wall_clock_ms,
                model_id,
                token_usage: ReviewTokenUsage {
                    input_tokens,
                    output_tokens,
                    cache_creation_input_tokens,
                    cache_read_input_tokens,
                },
                tool_calls: serde_json::from_str(&tool_calls_json)?,
                disposition: serde_json::from_str(&disposition_json)?,
                source: serde_json::from_str(&source_json)?,
                venues: serde_json::from_str(&venues_json)?,
                strategies: serde_json::from_str(&strategies_json)?,
            };
            record.validate()?;
            if query
                .from_ms
                .is_some_and(|from_ms| record.started_at_ms < from_ms)
                || query
                    .to_ms
                    .is_some_and(|to_ms| record.started_at_ms > to_ms)
                || query
                    .venue
                    .as_ref()
                    .is_some_and(|venue| !record.venues.contains(venue))
                || query
                    .strategy
                    .as_ref()
                    .is_some_and(|strategy| !record.strategies.contains(strategy))
            {
                continue;
            }
            entries.push(ReviewCostEntry { sequence, record });
        }
        Ok(entries)
    }

    pub fn summary(&self, query: &ReviewAccountingQuery) -> Result<ReviewCostSummary> {
        let entries = self.entries(query)?;
        Ok(summarize(&entries))
    }

    pub fn daily_summary(
        &self,
        now_ms: i64,
        venue: Option<&str>,
        strategy: Option<&str>,
    ) -> Result<ReviewCostSummary> {
        if now_ms < 0 {
            bail!("daily review summary timestamp must not be negative");
        }
        self.summary(&ReviewAccountingQuery {
            from_ms: Some(now_ms.saturating_sub(ONE_DAY_MS).max(0)),
            to_ms: Some(now_ms),
            venue: venue.map(ToOwned::to_owned),
            strategy: strategy.map(ToOwned::to_owned),
        })
    }
}

fn summarize(entries: &[ReviewCostEntry]) -> ReviewCostSummary {
    let mut summary = ReviewCostSummary::default();
    let mut model_costs = std::collections::BTreeMap::<String, ModelReviewCost>::new();
    for entry in entries {
        let record = &entry.record;
        summary.review_count = summary.review_count.saturating_add(1);
        if record.disposition.is_intervention() {
            summary.intervention_count = summary.intervention_count.saturating_add(1);
        } else {
            summary.no_change_count = summary.no_change_count.saturating_add(1);
        }
        if !matches!(record.source, WakeupSource::ScheduledReview { .. }) {
            summary.event_triggered_count = summary.event_triggered_count.saturating_add(1);
        }
        if record.is_false_wakeup() {
            summary.false_wakeup_count = summary.false_wakeup_count.saturating_add(1);
        }
        add_record_costs(&mut summary, record);
        let model = model_costs
            .entry(record.model_id.clone())
            .or_insert_with(|| ModelReviewCost {
                model_id: record.model_id.clone(),
                ..ModelReviewCost::default()
            });
        model.review_count = model.review_count.saturating_add(1);
        model.input_tokens = model
            .input_tokens
            .saturating_add(record.token_usage.input_total());
        model.output_tokens = model
            .output_tokens
            .saturating_add(record.token_usage.output_tokens);
        model.total_tokens = model
            .total_tokens
            .saturating_add(record.token_usage.total());
    }
    summary.false_wakeup_rate_bps =
        ratio_bps(summary.false_wakeup_count, summary.event_triggered_count);
    summary.cost_per_review = average_cost(&summary, summary.review_count);
    summary.cost_per_intervention = average_cost(&summary, summary.intervention_count);
    summary.models = model_costs.into_values().collect();
    summary
}

fn add_record_costs(summary: &mut ReviewCostSummary, record: &ReviewCostRecord) {
    summary.total_input_tokens = summary
        .total_input_tokens
        .saturating_add(record.token_usage.input_total());
    summary.total_output_tokens = summary
        .total_output_tokens
        .saturating_add(record.token_usage.output_tokens);
    summary.total_tokens = summary
        .total_tokens
        .saturating_add(record.token_usage.total());
    summary.total_wall_clock_ms = summary
        .total_wall_clock_ms
        .saturating_add(record.wall_clock_ms);
}

fn average_cost(summary: &ReviewCostSummary, divisor: u64) -> Option<ReviewUnitCost> {
    (divisor > 0).then(|| ReviewUnitCost {
        input_tokens: summary.total_input_tokens / divisor,
        output_tokens: summary.total_output_tokens / divisor,
        total_tokens: summary.total_tokens / divisor,
        wall_clock_ms: summary.total_wall_clock_ms / divisor,
    })
}

fn ratio_bps(numerator: u64, denominator: u64) -> Option<u32> {
    if denominator == 0 {
        return None;
    }
    Some(
        u32::try_from(u128::from(numerator).saturating_mul(10_000) / u128::from(denominator))
            .unwrap_or(u32::MAX),
    )
}

fn validate_unique_labels(label: &str, values: &[String]) -> Result<()> {
    let mut unique = BTreeSet::new();
    for value in values {
        validate_label(label, value)?;
        if !unique.insert(value) {
            bail!("review cost record repeats {label} {value}");
        }
    }
    Ok(())
}

fn validate_label(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{label} must not be empty");
    }
    if value.len() > 256 || value.chars().any(char::is_control) {
        bail!("{label} is invalid");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(
        turn_id: &str,
        started_at_ms: i64,
        source: WakeupSource,
        disposition: ReviewDisposition,
        venue: &str,
        strategy: &str,
        tokens: (u64, u64),
    ) -> ReviewCostRecord {
        ReviewCostRecord {
            schema_version: REVIEW_ACCOUNTING_SCHEMA_VERSION,
            turn_id: turn_id.to_string(),
            session_id: "session-1".to_string(),
            started_at_ms,
            completed_at_ms: started_at_ms + 50,
            wall_clock_ms: 45,
            model_id: "provider/model".to_string(),
            token_usage: ReviewTokenUsage {
                input_tokens: tokens.0,
                output_tokens: tokens.1,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
            tool_calls: vec![ReviewToolCall {
                name: "venue_strategy".to_string(),
                input: serde_json::json!({"action": "status"}),
            }],
            disposition,
            source,
            venues: vec![venue.to_string()],
            strategies: vec![strategy.to_string()],
        }
    }

    #[test]
    fn records_are_append_only_and_duplicate_turns_fail() {
        let store = ReviewAccountingStore::in_memory().expect("store");
        let first = record(
            "turn-1",
            100,
            WakeupSource::ScheduledReview {
                cadence: "hourly".to_string(),
            },
            ReviewDisposition::NoChange,
            "venue-a",
            "carry",
            (100, 20),
        );
        assert_eq!(store.append(first.clone()).expect("first").sequence, 1);
        assert!(store.append(first).is_err());
        assert_eq!(
            store
                .entries(&ReviewAccountingQuery::default())
                .expect("entries")
                .len(),
            1
        );
    }

    #[test]
    fn daily_venue_strategy_costs_and_false_wakeups_are_queryable() {
        let store = ReviewAccountingStore::in_memory().expect("store");
        store
            .append(record(
                "turn-1",
                100,
                WakeupSource::External {
                    event_type: "funding".to_string(),
                    summary: "changed".to_string(),
                },
                ReviewDisposition::NoChange,
                "venue-a",
                "carry",
                (100, 20),
            ))
            .expect("first");
        store
            .append(record(
                "turn-2",
                200,
                WakeupSource::ScheduledReview {
                    cadence: "hourly".to_string(),
                },
                ReviewDisposition::Intervention {
                    kinds: vec![ReviewInterventionKind::ParameterChange],
                },
                "venue-a",
                "carry",
                (200, 40),
            ))
            .expect("second");
        store
            .append(record(
                "turn-3",
                300,
                WakeupSource::ScheduledReview {
                    cadence: "hourly".to_string(),
                },
                ReviewDisposition::NoChange,
                "venue-b",
                "maker",
                (1_000, 100),
            ))
            .expect("third");

        let summary = store
            .summary(&ReviewAccountingQuery {
                from_ms: Some(0),
                to_ms: Some(500),
                venue: Some("venue-a".to_string()),
                strategy: Some("carry".to_string()),
            })
            .expect("summary");
        assert_eq!(summary.review_count, 2);
        assert_eq!(summary.intervention_count, 1);
        assert_eq!(summary.false_wakeup_count, 1);
        assert_eq!(summary.false_wakeup_rate_bps, Some(10_000));
        assert_eq!(summary.total_tokens, 360);
        assert_eq!(
            summary.cost_per_review,
            Some(ReviewUnitCost {
                input_tokens: 150,
                output_tokens: 30,
                total_tokens: 180,
                wall_clock_ms: 45,
            })
        );
        assert_eq!(
            summary.cost_per_intervention,
            Some(ReviewUnitCost {
                input_tokens: 300,
                output_tokens: 60,
                total_tokens: 360,
                wall_clock_ms: 90,
            })
        );
    }
}
