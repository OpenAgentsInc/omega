use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use trading_mandate::TradingNetwork;

pub const PREDICTION_SCHEMA_VERSION: u16 = 1;
pub const PROBABILITY_SCALE: u32 = 1_000_000;
const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const MAX_LABEL_LENGTH: usize = 500;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PredictionActor {
    Agent { agent_id: String },
    Strategy { strategy_id: String },
}

impl PredictionActor {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Agent { agent_id } => validate_label("agent ID", agent_id),
            Self::Strategy { strategy_id } => validate_label("strategy ID", strategy_id),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct MandateScope {
    pub venue: String,
    pub network: TradingNetwork,
}

impl MandateScope {
    fn validate(&self) -> Result<()> {
        validate_label("mandate venue", &self.venue)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredictedDirection {
    Up,
    Down,
    Flat,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OutcomeProbability {
    pub outcome: String,
    pub probability_micros: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PredictionForecast {
    Directional {
        direction: PredictedDirection,
        probability_micros: u32,
    },
    Distribution {
        outcomes: Vec<OutcomeProbability>,
    },
}

impl PredictionForecast {
    fn validate(&self, confidence_micros: u32) -> Result<()> {
        validate_probability("confidence", confidence_micros)?;
        match self {
            Self::Directional {
                probability_micros, ..
            } => {
                validate_probability("directional probability", *probability_micros)?;
                if *probability_micros != confidence_micros {
                    bail!("directional probability must equal declared confidence");
                }
            }
            Self::Distribution { outcomes } => {
                if outcomes.len() < 2 {
                    bail!("an outcome distribution must contain at least two outcomes");
                }
                let mut names = BTreeSet::new();
                let mut total = 0_u32;
                let mut maximum = 0_u32;
                for outcome in outcomes {
                    validate_label("outcome", &outcome.outcome)?;
                    if !names.insert(outcome.outcome.as_str()) {
                        bail!("an outcome distribution repeats {:?}", outcome.outcome);
                    }
                    validate_probability("outcome probability", outcome.probability_micros)?;
                    total = total
                        .checked_add(outcome.probability_micros)
                        .context("outcome probabilities overflowed")?;
                    maximum = maximum.max(outcome.probability_micros);
                }
                if total != PROBABILITY_SCALE {
                    bail!("outcome probabilities must sum to {PROBABILITY_SCALE}");
                }
                if confidence_micros != maximum {
                    bail!("distribution confidence must equal its largest probability");
                }
            }
        }
        Ok(())
    }

    fn probability_for(&self, outcome: &ResolvedOutcome) -> Result<(u32, bool)> {
        match (self, outcome) {
            (
                Self::Directional {
                    direction,
                    probability_micros,
                },
                ResolvedOutcome::Direction { direction: actual },
            ) => Ok((*probability_micros, direction == actual)),
            (Self::Distribution { outcomes }, ResolvedOutcome::Named { outcome }) => outcomes
                .iter()
                .find(|candidate| candidate.outcome == *outcome)
                .map(|candidate| (candidate.probability_micros, true))
                .with_context(|| format!("resolved outcome {outcome:?} was not forecast")),
            (Self::Directional { .. }, ResolvedOutcome::Named { .. }) => {
                bail!("a directional prediction requires a directional outcome")
            }
            (Self::Distribution { .. }, ResolvedOutcome::Direction { .. }) => {
                bail!("an outcome distribution requires a named outcome")
            }
        }
    }

    fn is_no_change(&self) -> bool {
        matches!(
            self,
            Self::Directional {
                direction: PredictedDirection::Flat,
                ..
            }
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolutionRule {
    pub source: String,
    pub baseline_at_ms: i64,
    pub resolve_at_ms: i64,
    pub flat_tolerance_bps: u32,
}

impl ResolutionRule {
    fn validate(&self, emitted_at_ms: i64, horizon_ms: u64) -> Result<()> {
        validate_label("resolution source", &self.source)?;
        if self.baseline_at_ms != emitted_at_ms {
            bail!("resolution baseline time must be fixed to emission time");
        }
        let expected_resolve_at = emitted_at_ms
            .checked_add(i64::try_from(horizon_ms).context("prediction horizon is too large")?)
            .context("prediction resolution time overflowed")?;
        if self.resolve_at_ms != expected_resolve_at {
            bail!("resolution time must equal emission time plus the declared horizon");
        }
        if self.flat_tolerance_bps > 10_000 {
            bail!("flat tolerance must not exceed 10000 basis points");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoringRule {
    Brier,
    Logarithmic,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PredictionEventDraft {
    pub schema_version: u16,
    pub emitted_at_ms: i64,
    pub actor: PredictionActor,
    pub mandate_scope: MandateScope,
    pub instrument: String,
    pub forecast: PredictionForecast,
    pub confidence_micros: u32,
    pub horizon_ms: u64,
    pub resolution_rule: ResolutionRule,
    pub scoring_rule: ScoringRule,
    pub observation_refs: Vec<String>,
    pub private_payload_ref: Option<String>,
    pub subsequent_decision_id: String,
}

impl PredictionEventDraft {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != PREDICTION_SCHEMA_VERSION {
            bail!(
                "unsupported prediction schema version {}",
                self.schema_version
            );
        }
        if self.emitted_at_ms < 0 {
            bail!("prediction emission time must not be negative");
        }
        if self.horizon_ms == 0 {
            bail!("prediction horizon must be greater than zero");
        }
        self.actor.validate()?;
        self.mandate_scope.validate()?;
        validate_label("instrument", &self.instrument)?;
        self.forecast.validate(self.confidence_micros)?;
        match (&self.forecast, self.scoring_rule) {
            (PredictionForecast::Directional { .. }, ScoringRule::Brier)
            | (PredictionForecast::Distribution { .. }, ScoringRule::Logarithmic) => {}
            (PredictionForecast::Directional { .. }, ScoringRule::Logarithmic) => {
                bail!("directional predictions must declare Brier scoring")
            }
            (PredictionForecast::Distribution { .. }, ScoringRule::Brier) => {
                bail!("outcome distributions must declare logarithmic scoring")
            }
        }
        self.resolution_rule
            .validate(self.emitted_at_ms, self.horizon_ms)?;
        validate_unique_labels("observation reference", &self.observation_refs)?;
        if let Some(reference) = &self.private_payload_ref {
            validate_label("private payload reference", reference)?;
        }
        validate_label("subsequent decision ID", &self.subsequent_decision_id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PredictionEvent {
    pub sequence: u64,
    pub prediction_id: String,
    #[serde(flatten)]
    pub draft: PredictionEventDraft,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResolvedOutcome {
    Direction { direction: PredictedDirection },
    Named { outcome: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PredictionScore {
    pub sequence: u64,
    pub prediction_id: String,
    pub resolved_at_ms: i64,
    pub resolution_source: String,
    pub outcome: ResolvedOutcome,
    pub forecast_probability_micros: u32,
    pub realized_match: bool,
    pub score_micros: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CalibrationBin {
    pub lower_probability_micros: u32,
    pub upper_probability_micros: u32,
    pub prediction_count: u64,
    pub mean_probability_micros: u32,
    pub observed_frequency_micros: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PredictionSummary {
    pub prediction_count: u64,
    pub resolved_count: u64,
    pub no_change_count: u64,
    pub no_change_frequency_bps: Option<u32>,
    pub mean_score_micros: Option<u64>,
    pub sharpness_micros: Option<u32>,
    pub calibration: Vec<CalibrationBin>,
}

pub trait OutcomeSource {
    fn resolve(&self, prediction: &PredictionEvent) -> Result<Option<ResolvedOutcome>>;
}

#[derive(Clone)]
pub struct PredictionStore {
    connection: Arc<Mutex<Connection>>,
}

impl PredictionStore {
    pub fn default_path() -> PathBuf {
        paths::data_dir()
            .join("threads")
            .join("prediction-events.db")
    }

    pub fn open_default() -> Result<Self> {
        Self::open(&Self::default_path())
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("could not create prediction event directory {parent:?}")
            })?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS prediction_events (
                 sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
                 prediction_id TEXT NOT NULL UNIQUE,
                 subsequent_decision_id TEXT NOT NULL UNIQUE,
                 emitted_at_ms INTEGER NOT NULL CHECK (emitted_at_ms >= 0),
                 event_json TEXT NOT NULL,
                 previous_hash TEXT NOT NULL,
                 entry_hash TEXT NOT NULL UNIQUE
             ) STRICT;
             CREATE TABLE IF NOT EXISTS prediction_scores (
                 sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
                 prediction_id TEXT NOT NULL UNIQUE,
                 resolved_at_ms INTEGER NOT NULL CHECK (resolved_at_ms >= 0),
                 score_json TEXT NOT NULL,
                 previous_hash TEXT NOT NULL,
                 entry_hash TEXT NOT NULL UNIQUE,
                 FOREIGN KEY (prediction_id) REFERENCES prediction_events(prediction_id)
             ) STRICT;
             CREATE TRIGGER IF NOT EXISTS prediction_events_no_update
             BEFORE UPDATE ON prediction_events BEGIN
                 SELECT RAISE(ABORT, 'prediction events are append-only');
             END;
             CREATE TRIGGER IF NOT EXISTS prediction_events_no_delete
             BEFORE DELETE ON prediction_events BEGIN
                 SELECT RAISE(ABORT, 'prediction events are append-only');
             END;
             CREATE TRIGGER IF NOT EXISTS prediction_scores_no_update
             BEFORE UPDATE ON prediction_scores BEGIN
                 SELECT RAISE(ABORT, 'prediction scores are append-only');
             END;
             CREATE TRIGGER IF NOT EXISTS prediction_scores_no_delete
             BEFORE DELETE ON prediction_scores BEGIN
                 SELECT RAISE(ABORT, 'prediction scores are append-only');
             END;",
        )?;
        connection.pragma_update(None, "user_version", PREDICTION_SCHEMA_VERSION)?;
        let store = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        store.verify()?;
        Ok(store)
    }

    pub fn append(&self, draft: PredictionEventDraft) -> Result<PredictionEvent> {
        draft.validate()?;
        let prediction_id = prediction_id(&draft)?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let events = load_events(&transaction)?;
        if let Some(existing) = events
            .iter()
            .find(|event| event.prediction_id == prediction_id)
        {
            if existing.draft == draft {
                return Ok(existing.clone());
            }
            bail!("prediction ID {prediction_id:?} names different content");
        }
        if events
            .iter()
            .any(|event| event.draft.subsequent_decision_id == draft.subsequent_decision_id)
        {
            bail!(
                "decision {:?} already has a prediction",
                draft.subsequent_decision_id
            );
        }
        let sequence = events.last().map_or(Ok(1_u64), |event| {
            event
                .sequence
                .checked_add(1)
                .context("prediction sequence overflowed")
        })?;
        let previous_hash = transaction
            .query_row(
                "SELECT entry_hash FROM prediction_events ORDER BY sequence DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(|| GENESIS_HASH.to_owned());
        let event = PredictionEvent {
            sequence,
            prediction_id,
            draft,
        };
        let event_json = serde_json::to_string(&event)?;
        let entry_hash = hash_record(sequence, &previous_hash, event_json.as_bytes());
        transaction.execute(
            "INSERT INTO prediction_events (
                 sequence, prediction_id, subsequent_decision_id, emitted_at_ms,
                 event_json, previous_hash, entry_hash
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                i64::try_from(sequence).context("prediction sequence exceeded SQLite range")?,
                event.prediction_id,
                event.draft.subsequent_decision_id,
                event.draft.emitted_at_ms,
                event_json,
                previous_hash,
                entry_hash,
            ],
        )?;
        transaction.commit()?;
        Ok(event)
    }

    pub fn events(&self) -> Result<Vec<PredictionEvent>> {
        load_events(&self.connection.lock())
    }

    pub fn prediction(&self, prediction_id: &str) -> Result<Option<PredictionEvent>> {
        validate_label("prediction ID", prediction_id)?;
        Ok(self
            .events()?
            .into_iter()
            .find(|event| event.prediction_id == prediction_id))
    }

    pub fn require_admission(
        &self,
        prediction_id: &str,
        actor: &PredictionActor,
        scope: &MandateScope,
        decision_id: &str,
        action_at_ms: i64,
    ) -> Result<PredictionEvent> {
        if action_at_ms < 0 {
            bail!("prediction admission time must not be negative");
        }
        let event = self
            .prediction(prediction_id)?
            .with_context(|| format!("prediction {prediction_id:?} does not exist"))?;
        if &event.draft.actor != actor {
            bail!("prediction actor does not match the action actor");
        }
        if &event.draft.mandate_scope != scope {
            bail!("prediction mandate scope does not match the action scope");
        }
        if event.draft.subsequent_decision_id != decision_id {
            bail!("prediction is not linked to decision {decision_id:?}");
        }
        if event.draft.emitted_at_ms > action_at_ms {
            bail!("prediction was emitted after its linked action");
        }
        Ok(event)
    }

    pub fn matured(&self, now_ms: i64) -> Result<Vec<PredictionEvent>> {
        if now_ms < 0 {
            bail!("prediction maturity time must not be negative");
        }
        let scored = self
            .scores()?
            .into_iter()
            .map(|score| score.prediction_id)
            .collect::<BTreeSet<_>>();
        Ok(self
            .events()?
            .into_iter()
            .filter(|event| {
                event.draft.resolution_rule.resolve_at_ms <= now_ms
                    && !scored.contains(&event.prediction_id)
            })
            .collect())
    }

    pub fn resolve_matured(
        &self,
        source: &impl OutcomeSource,
        now_ms: i64,
    ) -> Result<Vec<PredictionScore>> {
        let mut resolved = Vec::new();
        for prediction in self.matured(now_ms)? {
            let Some(outcome) = source.resolve(&prediction)? else {
                continue;
            };
            resolved.push(self.append_score(&prediction, outcome, now_ms)?);
        }
        Ok(resolved)
    }

    pub fn scores(&self) -> Result<Vec<PredictionScore>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT sequence, score_json, previous_hash, entry_hash
             FROM prediction_scores ORDER BY sequence",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut scores = Vec::new();
        let mut expected_sequence = 1_u64;
        let mut previous_hash = GENESIS_HASH.to_owned();
        for row in rows {
            let (sequence, score_json, stored_previous_hash, stored_hash) = row?;
            let sequence = u64::try_from(sequence).context("score sequence was negative")?;
            if sequence != expected_sequence || stored_previous_hash != previous_hash {
                bail!("prediction score sequence or hash chain is broken at {sequence}");
            }
            if stored_hash != hash_record(sequence, &previous_hash, score_json.as_bytes()) {
                bail!("prediction score hash mismatch at sequence {sequence}");
            }
            let score: PredictionScore = serde_json::from_str(&score_json)?;
            if score.sequence != sequence {
                bail!("prediction score payload sequence mismatch at {sequence}");
            }
            validate_score(&score)?;
            previous_hash = stored_hash;
            expected_sequence = expected_sequence
                .checked_add(1)
                .context("prediction score sequence overflowed")?;
            scores.push(score);
        }
        Ok(scores)
    }

    pub fn summary(&self) -> Result<PredictionSummary> {
        let events = self.events()?;
        let scores = self.scores()?;
        let prediction_count = u64::try_from(events.len()).unwrap_or(u64::MAX);
        let resolved_count = u64::try_from(scores.len()).unwrap_or(u64::MAX);
        let no_change_count = u64::try_from(
            events
                .iter()
                .filter(|event| event.draft.forecast.is_no_change())
                .count(),
        )
        .unwrap_or(u64::MAX);
        let no_change_frequency_bps = ratio_bps(no_change_count, prediction_count);
        let mean_score_micros = (!scores.is_empty()).then(|| {
            scores
                .iter()
                .map(|score| u128::from(score.score_micros))
                .sum::<u128>()
                .checked_div(scores.len() as u128)
                .and_then(|score| u64::try_from(score).ok())
                .unwrap_or(u64::MAX)
        });
        let directional = scores
            .iter()
            .filter(|score| matches!(score.outcome, ResolvedOutcome::Direction { .. }))
            .collect::<Vec<_>>();
        let sharpness_micros = (!directional.is_empty()).then(|| {
            let total = directional
                .iter()
                .map(|score| {
                    score
                        .forecast_probability_micros
                        .abs_diff(PROBABILITY_SCALE / 2)
                        .saturating_mul(2)
                })
                .map(u64::from)
                .sum::<u64>();
            u32::try_from(total / directional.len() as u64).unwrap_or(u32::MAX)
        });
        Ok(PredictionSummary {
            prediction_count,
            resolved_count,
            no_change_count,
            no_change_frequency_bps,
            mean_score_micros,
            sharpness_micros,
            calibration: calibration_bins(&directional),
        })
    }

    pub fn verify(&self) -> Result<()> {
        self.events()?;
        self.scores().map(|_| ())
    }

    fn append_score(
        &self,
        prediction: &PredictionEvent,
        outcome: ResolvedOutcome,
        resolved_at_ms: i64,
    ) -> Result<PredictionScore> {
        if resolved_at_ms < prediction.draft.resolution_rule.resolve_at_ms {
            bail!("prediction cannot resolve before its fixed horizon");
        }
        let (forecast_probability_micros, realized_match) =
            prediction.draft.forecast.probability_for(&outcome)?;
        let score_micros = score_prediction(
            prediction.draft.scoring_rule,
            forecast_probability_micros,
            realized_match,
        )?;
        self.scores()?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        if let Some(existing_json) = transaction
            .query_row(
                "SELECT score_json FROM prediction_scores WHERE prediction_id = ?1",
                params![&prediction.prediction_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            let existing: PredictionScore = serde_json::from_str(&existing_json)?;
            if existing.outcome == outcome
                && existing.resolved_at_ms == resolved_at_ms
                && existing.resolution_source == prediction.draft.resolution_rule.source
            {
                return Ok(existing);
            }
            bail!(
                "prediction {:?} already has a different score",
                prediction.prediction_id
            );
        }
        let (last_sequence, previous_hash) = transaction
            .query_row(
                "SELECT sequence, entry_hash FROM prediction_scores ORDER BY sequence DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .unwrap_or((0_i64, GENESIS_HASH.to_owned()));
        let sequence = last_sequence
            .checked_add(1)
            .context("prediction score sequence overflowed")?;
        let score = PredictionScore {
            sequence: u64::try_from(sequence).context("score sequence was negative")?,
            prediction_id: prediction.prediction_id.clone(),
            resolved_at_ms,
            resolution_source: prediction.draft.resolution_rule.source.clone(),
            outcome,
            forecast_probability_micros,
            realized_match,
            score_micros,
        };
        validate_score(&score)?;
        let score_json = serde_json::to_string(&score)?;
        let entry_hash = hash_record(score.sequence, &previous_hash, score_json.as_bytes());
        transaction.execute(
            "INSERT INTO prediction_scores (
                 sequence, prediction_id, resolved_at_ms, score_json, previous_hash, entry_hash
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                sequence,
                score.prediction_id,
                score.resolved_at_ms,
                score_json,
                previous_hash,
                entry_hash,
            ],
        )?;
        transaction.commit()?;
        Ok(score)
    }
}

fn load_events(connection: &Connection) -> Result<Vec<PredictionEvent>> {
    let mut statement = connection.prepare(
        "SELECT sequence, event_json, previous_hash, entry_hash
         FROM prediction_events ORDER BY sequence",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut events = Vec::new();
    let mut expected_sequence = 1_u64;
    let mut previous_hash = GENESIS_HASH.to_owned();
    let mut decisions = BTreeSet::new();
    for row in rows {
        let (sequence, event_json, stored_previous_hash, stored_hash) = row?;
        let sequence = u64::try_from(sequence).context("prediction sequence was negative")?;
        if sequence != expected_sequence || stored_previous_hash != previous_hash {
            bail!("prediction sequence or hash chain is broken at {sequence}");
        }
        if stored_hash != hash_record(sequence, &previous_hash, event_json.as_bytes()) {
            bail!("prediction hash mismatch at sequence {sequence}");
        }
        let event: PredictionEvent = serde_json::from_str(&event_json)?;
        if event.sequence != sequence || event.prediction_id != prediction_id(&event.draft)? {
            bail!("prediction payload identity mismatch at sequence {sequence}");
        }
        event.draft.validate()?;
        if !decisions.insert(event.draft.subsequent_decision_id.clone()) {
            bail!("prediction events repeat a subsequent decision ID");
        }
        events.push(event);
        previous_hash = stored_hash;
        expected_sequence = expected_sequence
            .checked_add(1)
            .context("prediction sequence overflowed")?;
    }
    Ok(events)
}

fn prediction_id(draft: &PredictionEventDraft) -> Result<String> {
    Ok(format!(
        "prediction:{:x}",
        Sha256::digest(serde_json::to_vec(draft)?)
    ))
}

fn hash_record(sequence: u64, previous_hash: &str, payload: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sequence.to_be_bytes());
    hasher.update(previous_hash.as_bytes());
    hasher.update(payload);
    format!("{:x}", hasher.finalize())
}

fn score_prediction(
    scoring_rule: ScoringRule,
    probability_micros: u32,
    realized_match: bool,
) -> Result<u64> {
    validate_probability("forecast probability", probability_micros)?;
    match scoring_rule {
        ScoringRule::Brier => {
            let observed = if realized_match { PROBABILITY_SCALE } else { 0 };
            let difference = i64::from(probability_micros) - i64::from(observed);
            let squared = u128::from(difference.unsigned_abs()).pow(2);
            u64::try_from(squared / u128::from(PROBABILITY_SCALE)).context("Brier score overflowed")
        }
        ScoringRule::Logarithmic => {
            let probability = f64::from(probability_micros) / f64::from(PROBABILITY_SCALE);
            let score = -probability.ln() * f64::from(PROBABILITY_SCALE);
            if !score.is_finite() || score < 0.0 || score > u64::MAX as f64 {
                bail!("logarithmic score is outside the durable range");
            }
            Ok(score.round() as u64)
        }
    }
}

fn validate_score(score: &PredictionScore) -> Result<()> {
    validate_label("prediction ID", &score.prediction_id)?;
    validate_label("resolution source", &score.resolution_source)?;
    if score.sequence == 0 || score.resolved_at_ms < 0 {
        bail!("prediction score sequence and timestamp must be positive");
    }
    validate_probability("forecast probability", score.forecast_probability_micros)?;
    if let ResolvedOutcome::Named { outcome } = &score.outcome {
        validate_label("resolved outcome", outcome)?;
    }
    Ok(())
}

fn calibration_bins(scores: &[&PredictionScore]) -> Vec<CalibrationBin> {
    let mut bins = BTreeMap::<u32, (u64, u64, u64)>::new();
    for score in scores {
        let index = (score.forecast_probability_micros / 100_000).min(9);
        let entry = bins.entry(index).or_default();
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry
            .1
            .saturating_add(u64::from(score.forecast_probability_micros));
        entry.2 = entry.2.saturating_add(u64::from(score.realized_match));
    }
    bins.into_iter()
        .map(
            |(index, (count, probability_total, match_count))| CalibrationBin {
                lower_probability_micros: index.saturating_mul(100_000),
                upper_probability_micros: if index == 9 {
                    PROBABILITY_SCALE
                } else {
                    index.saturating_add(1).saturating_mul(100_000)
                },
                prediction_count: count,
                mean_probability_micros: u32::try_from(probability_total / count)
                    .unwrap_or(u32::MAX),
                observed_frequency_micros: ratio_micros(match_count, count).unwrap_or_default(),
            },
        )
        .collect()
}

fn ratio_bps(numerator: u64, denominator: u64) -> Option<u32> {
    (denominator > 0).then(|| {
        u32::try_from(
            u128::from(numerator)
                .saturating_mul(10_000)
                .checked_div(u128::from(denominator))
                .unwrap_or_default(),
        )
        .unwrap_or(u32::MAX)
    })
}

fn ratio_micros(numerator: u64, denominator: u64) -> Option<u32> {
    (denominator > 0).then(|| {
        u32::try_from(
            u128::from(numerator)
                .saturating_mul(u128::from(PROBABILITY_SCALE))
                .checked_div(u128::from(denominator))
                .unwrap_or_default(),
        )
        .unwrap_or(u32::MAX)
    })
}

fn validate_probability(label: &str, probability_micros: u32) -> Result<()> {
    if probability_micros == 0 || probability_micros > PROBABILITY_SCALE {
        bail!("{label} must be from 1 through {PROBABILITY_SCALE} millionths");
    }
    Ok(())
}

fn validate_label(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("prediction {label} must not be empty");
    }
    if value.len() > MAX_LABEL_LENGTH {
        bail!("prediction {label} must not exceed {MAX_LABEL_LENGTH} bytes");
    }
    Ok(())
}

fn validate_unique_labels(label: &str, values: &[String]) -> Result<()> {
    let mut unique = BTreeSet::new();
    for value in values {
        validate_label(label, value)?;
        if !unique.insert(value.as_str()) {
            bail!("prediction repeats {label} {value:?}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directional_draft(
        emitted_at_ms: i64,
        decision_id: &str,
        direction: PredictedDirection,
        probability_micros: u32,
    ) -> PredictionEventDraft {
        PredictionEventDraft {
            schema_version: PREDICTION_SCHEMA_VERSION,
            emitted_at_ms,
            actor: PredictionActor::Strategy {
                strategy_id: "alpha".into(),
            },
            mandate_scope: MandateScope {
                venue: "lnmarkets".into(),
                network: TradingNetwork::Signet,
            },
            instrument: "BTCUSD".into(),
            forecast: PredictionForecast::Directional {
                direction,
                probability_micros,
            },
            confidence_micros: probability_micros,
            horizon_ms: 1_000,
            resolution_rule: ResolutionRule {
                source: "fixture:last_price".into(),
                baseline_at_ms: emitted_at_ms,
                resolve_at_ms: emitted_at_ms + 1_000,
                flat_tolerance_bps: 10,
            },
            scoring_rule: ScoringRule::Brier,
            observation_refs: vec![format!("fixture:{emitted_at_ms}")],
            private_payload_ref: None,
            subsequent_decision_id: decision_id.into(),
        }
    }

    struct FixedOutcome(PredictedDirection);

    impl OutcomeSource for FixedOutcome {
        fn resolve(&self, _prediction: &PredictionEvent) -> Result<Option<ResolvedOutcome>> {
            Ok(Some(ResolvedOutcome::Direction { direction: self.0 }))
        }
    }

    #[test]
    fn prediction_and_score_logs_are_append_only_and_contiguous() {
        let store = PredictionStore::in_memory().expect("store");
        let first = store
            .append(directional_draft(
                1,
                "decision-1",
                PredictedDirection::Up,
                800_000,
            ))
            .expect("first");
        let second = store
            .append(directional_draft(
                2,
                "decision-2",
                PredictedDirection::Down,
                700_000,
            ))
            .expect("second");
        assert_eq!((first.sequence, second.sequence), (1, 2));
        assert_eq!(
            store
                .resolve_matured(&FixedOutcome(PredictedDirection::Up), 2_000)
                .expect("scores")
                .len(),
            2
        );
        let connection = store.connection.lock();
        for statement in [
            "UPDATE prediction_events SET emitted_at_ms = 4",
            "DELETE FROM prediction_events",
            "UPDATE prediction_scores SET resolved_at_ms = 4",
            "DELETE FROM prediction_scores",
        ] {
            assert!(
                connection
                    .execute(statement, [])
                    .expect_err("append-only refusal")
                    .to_string()
                    .contains("append-only")
            );
        }
    }

    #[test]
    fn brier_and_logarithmic_scores_are_deterministic() {
        assert_eq!(
            score_prediction(ScoringRule::Brier, 800_000, true).expect("matched Brier"),
            40_000
        );
        assert_eq!(
            score_prediction(ScoringRule::Brier, 800_000, false).expect("missed Brier"),
            640_000
        );
        assert_eq!(
            score_prediction(ScoringRule::Logarithmic, 250_000, true).expect("log score"),
            1_386_294
        );
    }

    #[test]
    fn admission_requires_exact_pre_action_linkage() {
        let store = PredictionStore::in_memory().expect("store");
        let draft = directional_draft(10, "decision-1", PredictedDirection::Flat, 600_000);
        let actor = draft.actor.clone();
        let scope = draft.mandate_scope.clone();
        let event = store.append(draft).expect("prediction");
        assert!(
            store
                .require_admission(&event.prediction_id, &actor, &scope, "decision-1", 10)
                .is_ok()
        );
        assert!(
            store
                .require_admission(&event.prediction_id, &actor, &scope, "other", 10)
                .is_err()
        );
        assert!(
            store
                .require_admission(&event.prediction_id, &actor, &scope, "decision-1", 9)
                .is_err()
        );
    }

    #[test]
    fn aggregates_report_calibration_sharpness_and_no_change_frequency() {
        let store = PredictionStore::in_memory().expect("store");
        store
            .append(directional_draft(
                1,
                "decision-1",
                PredictedDirection::Flat,
                800_000,
            ))
            .expect("flat");
        store
            .append(directional_draft(
                2,
                "decision-2",
                PredictedDirection::Up,
                800_000,
            ))
            .expect("up");
        store
            .resolve_matured(&FixedOutcome(PredictedDirection::Up), 2_000)
            .expect("resolve");
        let summary = store.summary().expect("summary");
        assert_eq!(summary.prediction_count, 2);
        assert_eq!(summary.resolved_count, 2);
        assert_eq!(summary.no_change_frequency_bps, Some(5_000));
        assert_eq!(summary.sharpness_micros, Some(600_000));
        assert_eq!(summary.calibration.len(), 1);
        assert_eq!(
            summary
                .calibration
                .first()
                .expect("calibration bin")
                .observed_frequency_micros,
            500_000
        );
    }
}
