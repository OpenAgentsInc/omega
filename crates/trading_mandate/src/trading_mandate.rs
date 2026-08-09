use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use parking_lot::Mutex;
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const MAX_IDENTIFIER_LENGTH: usize = 200;
const MAX_OBJECTIVE_LENGTH: usize = 1_000;
const MIN_REVIEW_INTERVAL_SECONDS: u64 = 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingNetwork {
    Signet,
    Mainnet,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReviewCadence {
    FundingSettlement,
    Interval { seconds: u64 },
}

impl ReviewCadence {
    fn validate(&self) -> Result<()> {
        if let Self::Interval { seconds } = self
            && *seconds < MIN_REVIEW_INTERVAL_SECONDS
        {
            bail!("mandate review interval must be at least {MIN_REVIEW_INTERVAL_SECONDS} seconds");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TradingMandate {
    pub network: TradingNetwork,
    pub objective: String,
    pub max_venue_balance_sats: u64,
    pub max_position_usd: u64,
    pub max_leverage: u8,
    pub daily_loss_stop_sats: u64,
    // Old mandates receive no hourly order authority until a person reviews them.
    #[serde(default)]
    pub max_orders_per_hour: u32,
    // Old mandates require maximum distance from liquidation until reviewed.
    #[serde(default = "legacy_liquidation_buffer_bps")]
    pub min_liquidation_buffer_bps: u32,
    pub allowed_strategies: BTreeSet<String>,
    pub review_cadence: ReviewCadence,
    pub expires_at_ms: i64,
}

impl TradingMandate {
    pub fn validate(&self) -> Result<()> {
        let objective = self.objective.trim();
        if objective.is_empty() {
            bail!("mandate objective must not be empty");
        }
        if objective.len() > MAX_OBJECTIVE_LENGTH {
            bail!("mandate objective must not exceed {MAX_OBJECTIVE_LENGTH} bytes");
        }
        if self.max_venue_balance_sats == 0 {
            bail!("mandate venue balance limit must be positive");
        }
        if self.max_position_usd == 0 {
            bail!("mandate position limit must be positive");
        }
        if !(1..=100).contains(&self.max_leverage) {
            bail!("mandate leverage must be from 1 through 100");
        }
        if self.daily_loss_stop_sats == 0 {
            bail!("mandate daily loss stop must be positive");
        }
        if self.min_liquidation_buffer_bps > 10_000 {
            bail!("mandate liquidation buffer must not exceed 10000 basis points");
        }
        if self.allowed_strategies.is_empty() {
            bail!("mandate must allow at least one strategy");
        }
        for strategy_id in &self.allowed_strategies {
            validate_identifier("strategy ID", strategy_id)?;
        }
        self.review_cadence.validate()?;
        if self.expires_at_ms <= 0 {
            bail!("mandate expiry must be a positive timestamp");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MandateChangeClass {
    Creation,
    Widening,
    Restriction,
    Unchanged,
}

impl MandateChangeClass {
    pub fn needs_ui_approval(self) -> bool {
        matches!(self, Self::Creation | Self::Widening)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MandateProposal {
    base_revision: u64,
    candidate: TradingMandate,
    change_class: MandateChangeClass,
    digest: String,
}

impl MandateProposal {
    pub fn base_revision(&self) -> u64 {
        self.base_revision
    }

    pub fn candidate(&self) -> &TradingMandate {
        &self.candidate
    }

    pub fn change_class(&self) -> MandateChangeClass {
        self.change_class
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MandateRevisionKind {
    Creation,
    Widening,
    Restriction,
    Revocation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MandateRevision {
    pub revision: u64,
    pub changed_at_ms: i64,
    pub kind: MandateRevisionKind,
    pub mandate: Option<TradingMandate>,
    pub approval_digest: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MandateSnapshot {
    pub revision: u64,
    pub mandate: Option<TradingMandate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradingInstruction {
    pub network: TradingNetwork,
    pub strategy_id: String,
    pub venue_balance_after_sats: u64,
    pub position_notional_usd: u64,
    pub leverage: u8,
    pub daily_realized_loss_sats: u64,
    pub orders_last_hour: u32,
    pub liquidation_buffer_bps: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredRiskPosture {
    FlatRisk,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MandateRefusal {
    Missing,
    Expired {
        expires_at_ms: i64,
    },
    NetworkNotAllowed,
    StrategyNotAllowed {
        strategy_id: String,
    },
    VenueBalanceLimit {
        limit_sats: u64,
        requested_sats: u64,
    },
    PositionLimit {
        limit_usd: u64,
        requested_usd: u64,
    },
    LeverageLimit {
        limit: u8,
        requested: u8,
    },
    DailyLossStop {
        limit_sats: u64,
        loss_sats: u64,
    },
    HourlyOrderLimit {
        limit: u32,
        orders_last_hour: u32,
    },
    LiquidationBufferFloor {
        minimum_bps: u32,
        current_bps: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MandateDecision {
    Authorized {
        revision: u64,
    },
    Refused {
        reason: MandateRefusal,
        required_posture: RequiredRiskPosture,
    },
}

#[derive(Clone)]
pub struct MandateStore {
    connection: Arc<Mutex<Connection>>,
}

impl MandateStore {
    pub fn default_path() -> PathBuf {
        paths::data_dir().join("threads").join("trading-mandate.db")
    }

    pub fn open_default() -> Result<Self> {
        Self::open(&Self::default_path())
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("could not create trading mandate directory {parent:?}")
            })?;
        }
        let connection = Connection::open(path)
            .with_context(|| format!("could not open trading mandate store {path:?}"))?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS trading_mandate_revisions (
                 revision INTEGER PRIMARY KEY CHECK (revision > 0),
                 changed_at_ms INTEGER NOT NULL CHECK (changed_at_ms >= 0),
                 kind TEXT NOT NULL,
                 mandate_json TEXT,
                 approval_digest TEXT
             ) STRICT;
             CREATE TRIGGER IF NOT EXISTS trading_mandate_no_update
             BEFORE UPDATE ON trading_mandate_revisions BEGIN
                 SELECT RAISE(ABORT, 'trading mandate history is append-only');
             END;
             CREATE TRIGGER IF NOT EXISTS trading_mandate_no_delete
             BEFORE DELETE ON trading_mandate_revisions BEGIN
                 SELECT RAISE(ABORT, 'trading mandate history is append-only');
             END;",
        )?;
        let store = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        store.verify()?;
        Ok(store)
    }

    pub fn snapshot(&self) -> Result<MandateSnapshot> {
        let revisions = load_revisions(&self.connection.lock())?;
        verify_revisions(&revisions)?;
        Ok(snapshot_from_revisions(&revisions))
    }

    pub fn history(&self) -> Result<Vec<MandateRevision>> {
        let revisions = load_revisions(&self.connection.lock())?;
        verify_revisions(&revisions)?;
        Ok(revisions)
    }

    pub fn verify(&self) -> Result<()> {
        verify_revisions(&load_revisions(&self.connection.lock())?)
    }

    pub fn propose(&self, candidate: TradingMandate) -> Result<MandateProposal> {
        candidate.validate()?;
        let snapshot = self.snapshot()?;
        make_proposal(snapshot.revision, snapshot.mandate.as_ref(), candidate)
    }

    pub fn save_restriction(
        &self,
        proposal: MandateProposal,
        changed_at_ms: i64,
    ) -> Result<MandateSnapshot> {
        validate_timestamp(changed_at_ms)?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let revisions = load_revisions(&transaction)?;
        verify_revisions(&revisions)?;
        let proposal = revalidate_proposal(&proposal, &revisions)?;
        match proposal.change_class {
            MandateChangeClass::Creation | MandateChangeClass::Widening => {
                bail!("creating or widening a mandate requires explicit UI approval")
            }
            MandateChangeClass::Unchanged => {
                transaction.commit()?;
                return Ok(snapshot_from_revisions(&revisions));
            }
            MandateChangeClass::Restriction => {}
        }
        let revision = append_revision(
            &transaction,
            &revisions,
            changed_at_ms,
            MandateRevisionKind::Restriction,
            Some(proposal.candidate),
            None,
        )?;
        transaction.commit()?;
        Ok(MandateSnapshot {
            revision: revision.revision,
            mandate: revision.mandate,
        })
    }

    /// This is the single widening door. Repository policy limits production
    /// call sites to the settings UI, which must bind the accepted prompt to
    /// this exact proposal before calling it.
    pub fn apply_ui_approved(
        &self,
        proposal: MandateProposal,
        approved_at_ms: i64,
    ) -> Result<MandateSnapshot> {
        validate_timestamp(approved_at_ms)?;
        if proposal.candidate.expires_at_ms <= approved_at_ms {
            bail!("an approved mandate must expire in the future");
        }
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let revisions = load_revisions(&transaction)?;
        verify_revisions(&revisions)?;
        let proposal = revalidate_proposal(&proposal, &revisions)?;
        let kind = match proposal.change_class {
            MandateChangeClass::Creation => MandateRevisionKind::Creation,
            MandateChangeClass::Widening => MandateRevisionKind::Widening,
            MandateChangeClass::Restriction | MandateChangeClass::Unchanged => {
                bail!("UI approval is reserved for mandate creation or widening")
            }
        };
        let revision = append_revision(
            &transaction,
            &revisions,
            approved_at_ms,
            kind,
            Some(proposal.candidate),
            Some(proposal.digest),
        )?;
        transaction.commit()?;
        Ok(MandateSnapshot {
            revision: revision.revision,
            mandate: revision.mandate,
        })
    }

    pub fn revoke(&self, changed_at_ms: i64) -> Result<MandateSnapshot> {
        validate_timestamp(changed_at_ms)?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let revisions = load_revisions(&transaction)?;
        verify_revisions(&revisions)?;
        if snapshot_from_revisions(&revisions).mandate.is_none() {
            transaction.commit()?;
            return Ok(snapshot_from_revisions(&revisions));
        }
        let revision = append_revision(
            &transaction,
            &revisions,
            changed_at_ms,
            MandateRevisionKind::Revocation,
            None,
            None,
        )?;
        transaction.commit()?;
        Ok(MandateSnapshot {
            revision: revision.revision,
            mandate: revision.mandate,
        })
    }

    pub fn authorize(
        &self,
        instruction: &TradingInstruction,
        now_ms: i64,
    ) -> Result<MandateDecision> {
        validate_timestamp(now_ms)?;
        validate_identifier("strategy ID", &instruction.strategy_id)?;
        let snapshot = self.snapshot()?;
        let Some(mandate) = snapshot.mandate else {
            return Ok(refused(MandateRefusal::Missing));
        };
        if now_ms >= mandate.expires_at_ms {
            return Ok(refused(MandateRefusal::Expired {
                expires_at_ms: mandate.expires_at_ms,
            }));
        }
        if instruction.network != mandate.network {
            return Ok(refused(MandateRefusal::NetworkNotAllowed));
        }
        if !mandate
            .allowed_strategies
            .contains(&instruction.strategy_id)
        {
            return Ok(refused(MandateRefusal::StrategyNotAllowed {
                strategy_id: instruction.strategy_id.clone(),
            }));
        }
        if instruction.venue_balance_after_sats > mandate.max_venue_balance_sats {
            return Ok(refused(MandateRefusal::VenueBalanceLimit {
                limit_sats: mandate.max_venue_balance_sats,
                requested_sats: instruction.venue_balance_after_sats,
            }));
        }
        if instruction.position_notional_usd > mandate.max_position_usd {
            return Ok(refused(MandateRefusal::PositionLimit {
                limit_usd: mandate.max_position_usd,
                requested_usd: instruction.position_notional_usd,
            }));
        }
        if instruction.leverage > mandate.max_leverage {
            return Ok(refused(MandateRefusal::LeverageLimit {
                limit: mandate.max_leverage,
                requested: instruction.leverage,
            }));
        }
        if instruction.daily_realized_loss_sats >= mandate.daily_loss_stop_sats {
            return Ok(refused(MandateRefusal::DailyLossStop {
                limit_sats: mandate.daily_loss_stop_sats,
                loss_sats: instruction.daily_realized_loss_sats,
            }));
        }
        if instruction.orders_last_hour >= mandate.max_orders_per_hour {
            return Ok(refused(MandateRefusal::HourlyOrderLimit {
                limit: mandate.max_orders_per_hour,
                orders_last_hour: instruction.orders_last_hour,
            }));
        }
        if instruction.position_notional_usd > 0
            && instruction.leverage > 1
            && instruction.liquidation_buffer_bps < mandate.min_liquidation_buffer_bps
        {
            return Ok(refused(MandateRefusal::LiquidationBufferFloor {
                minimum_bps: mandate.min_liquidation_buffer_bps,
                current_bps: instruction.liquidation_buffer_bps,
            }));
        }
        Ok(MandateDecision::Authorized {
            revision: snapshot.revision,
        })
    }
}

fn refused(reason: MandateRefusal) -> MandateDecision {
    MandateDecision::Refused {
        reason,
        required_posture: RequiredRiskPosture::FlatRisk,
    }
}

fn revalidate_proposal(
    proposal: &MandateProposal,
    revisions: &[MandateRevision],
) -> Result<MandateProposal> {
    let snapshot = snapshot_from_revisions(revisions);
    if snapshot.revision != proposal.base_revision {
        bail!("mandate changed after the proposal was displayed; review the current limits again");
    }
    let current = make_proposal(
        snapshot.revision,
        snapshot.mandate.as_ref(),
        proposal.candidate.clone(),
    )?;
    if current != *proposal {
        bail!("mandate proposal no longer matches the approved limits");
    }
    Ok(current)
}

fn make_proposal(
    base_revision: u64,
    current: Option<&TradingMandate>,
    candidate: TradingMandate,
) -> Result<MandateProposal> {
    candidate.validate()?;
    let change_class = classify_change(current, &candidate);
    let digest = proposal_digest(base_revision, &candidate)?;
    Ok(MandateProposal {
        base_revision,
        candidate,
        change_class,
        digest,
    })
}

fn classify_change(
    current: Option<&TradingMandate>,
    candidate: &TradingMandate,
) -> MandateChangeClass {
    let Some(current) = current else {
        return MandateChangeClass::Creation;
    };
    if current == candidate {
        return MandateChangeClass::Unchanged;
    }
    let cadence_widens = match (&current.review_cadence, &candidate.review_cadence) {
        (
            ReviewCadence::Interval {
                seconds: current_seconds,
            },
            ReviewCadence::Interval {
                seconds: candidate_seconds,
            },
        ) => candidate_seconds > current_seconds,
        (ReviewCadence::FundingSettlement, ReviewCadence::FundingSettlement) => false,
        _ => true,
    };
    let widens = candidate.network != current.network
        || candidate.objective != current.objective
        || candidate.max_venue_balance_sats > current.max_venue_balance_sats
        || candidate.max_position_usd > current.max_position_usd
        || candidate.max_leverage > current.max_leverage
        || candidate.daily_loss_stop_sats > current.daily_loss_stop_sats
        || candidate.max_orders_per_hour > current.max_orders_per_hour
        || candidate.min_liquidation_buffer_bps < current.min_liquidation_buffer_bps
        || !candidate
            .allowed_strategies
            .is_subset(&current.allowed_strategies)
        || cadence_widens
        || candidate.expires_at_ms > current.expires_at_ms;
    if widens {
        MandateChangeClass::Widening
    } else {
        MandateChangeClass::Restriction
    }
}

const fn legacy_liquidation_buffer_bps() -> u32 {
    10_000
}

fn proposal_digest(base_revision: u64, candidate: &TradingMandate) -> Result<String> {
    #[derive(Serialize)]
    struct DigestPayload<'a> {
        base_revision: u64,
        candidate: &'a TradingMandate,
    }
    let bytes = serde_json::to_vec(&DigestPayload {
        base_revision,
        candidate,
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn snapshot_from_revisions(revisions: &[MandateRevision]) -> MandateSnapshot {
    revisions
        .last()
        .map_or_else(MandateSnapshot::default, |last| MandateSnapshot {
            revision: last.revision,
            mandate: last.mandate.clone(),
        })
}

fn append_revision(
    transaction: &Transaction<'_>,
    revisions: &[MandateRevision],
    changed_at_ms: i64,
    kind: MandateRevisionKind,
    mandate: Option<TradingMandate>,
    approval_digest: Option<String>,
) -> Result<MandateRevision> {
    let revision = revisions.last().map_or(Ok(1_u64), |last| {
        last.revision
            .checked_add(1)
            .context("mandate revision overflowed")
    })?;
    let revision_i64 = i64::try_from(revision).context("mandate revision exceeded SQLite range")?;
    let kind_json = serde_json::to_string(&kind)?;
    let mandate_json = mandate.as_ref().map(serde_json::to_string).transpose()?;
    transaction.execute(
        "INSERT INTO trading_mandate_revisions (
             revision, changed_at_ms, kind, mandate_json, approval_digest
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            revision_i64,
            changed_at_ms,
            kind_json,
            mandate_json,
            approval_digest,
        ],
    )?;
    Ok(MandateRevision {
        revision,
        changed_at_ms,
        kind,
        mandate,
        approval_digest,
    })
}

fn load_revisions(connection: &Connection) -> Result<Vec<MandateRevision>> {
    let mut statement = connection.prepare(
        "SELECT revision, changed_at_ms, kind, mandate_json, approval_digest
         FROM trading_mandate_revisions ORDER BY revision",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut revisions = Vec::new();
    for row in rows {
        let (revision, changed_at_ms, kind_json, mandate_json, approval_digest) = row?;
        revisions.push(MandateRevision {
            revision: u64::try_from(revision).context("mandate revision was negative")?,
            changed_at_ms,
            kind: serde_json::from_str(&kind_json)
                .context("mandate revision kind is not valid JSON")?,
            mandate: mandate_json
                .map(|json| serde_json::from_str(&json))
                .transpose()
                .context("mandate revision is not valid JSON")?,
            approval_digest,
        });
    }
    Ok(revisions)
}

fn verify_revisions(revisions: &[MandateRevision]) -> Result<()> {
    let mut expected_revision = 1_u64;
    let mut current: Option<TradingMandate> = None;
    for revision in revisions {
        if revision.revision != expected_revision {
            bail!(
                "trading mandate revision gap: expected {}, found {}",
                expected_revision,
                revision.revision
            );
        }
        validate_timestamp(revision.changed_at_ms)?;
        match (&revision.kind, &revision.mandate) {
            (MandateRevisionKind::Revocation, None) => {
                if revision.approval_digest.is_some() {
                    bail!("mandate revocation must not carry a widening approval");
                }
                if current.is_none() {
                    bail!("mandate history revokes an absent mandate");
                }
                current = None;
            }
            (kind, Some(mandate)) => {
                mandate.validate()?;
                let proposal = make_proposal(
                    revision.revision.saturating_sub(1),
                    current.as_ref(),
                    mandate.clone(),
                )?;
                let expected_kind = match proposal.change_class {
                    MandateChangeClass::Creation => MandateRevisionKind::Creation,
                    MandateChangeClass::Widening => MandateRevisionKind::Widening,
                    MandateChangeClass::Restriction => MandateRevisionKind::Restriction,
                    MandateChangeClass::Unchanged => {
                        bail!("mandate history contains an unchanged revision")
                    }
                };
                if *kind != expected_kind {
                    bail!("mandate revision kind does not match its limit change");
                }
                match proposal.change_class.needs_ui_approval() {
                    true if revision.approval_digest.as_deref() != Some(&proposal.digest) => {
                        bail!("mandate creation or widening lacks its bound UI approval")
                    }
                    false if revision.approval_digest.is_some() => {
                        bail!("mandate restriction carries an unexpected widening approval")
                    }
                    _ => {}
                }
                current = Some(mandate.clone());
            }
            (_, None) => bail!("non-revocation mandate history row has no mandate"),
        }
        expected_revision = expected_revision
            .checked_add(1)
            .context("mandate revision overflowed during verification")?;
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("mandate {label} must not be empty");
    }
    if value.len() > MAX_IDENTIFIER_LENGTH {
        bail!("mandate {label} must not exceed {MAX_IDENTIFIER_LENGTH} bytes");
    }
    Ok(())
}

fn validate_timestamp(timestamp_ms: i64) -> Result<()> {
    if timestamp_ms < 0 {
        bail!("mandate timestamp must not be negative");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strategies(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn mandate() -> TradingMandate {
        TradingMandate {
            network: TradingNetwork::Signet,
            objective: "Maximize ledger profit in sats".into(),
            max_venue_balance_sats: 100_000,
            max_position_usd: 500,
            max_leverage: 3,
            daily_loss_stop_sats: 5_000,
            max_orders_per_hour: 12,
            min_liquidation_buffer_bps: 1_500,
            allowed_strategies: strategies(&["rebalance_to_target"]),
            review_cadence: ReviewCadence::FundingSettlement,
            expires_at_ms: 10_000,
        }
    }

    fn approve(store: &MandateStore, candidate: TradingMandate, now: i64) -> MandateSnapshot {
        let proposal = store.propose(candidate).expect("proposal");
        assert!(proposal.change_class().needs_ui_approval());
        store
            .apply_ui_approved(proposal, now)
            .expect("approved mandate")
    }

    fn instruction() -> TradingInstruction {
        TradingInstruction {
            network: TradingNetwork::Signet,
            strategy_id: "rebalance_to_target".into(),
            venue_balance_after_sats: 80_000,
            position_notional_usd: 400,
            leverage: 2,
            daily_realized_loss_sats: 1_000,
            orders_last_hour: 2,
            liquidation_buffer_bps: 2_500,
        }
    }

    #[test]
    fn legacy_mandate_json_defaults_new_limits_to_fail_closed_values() {
        let value = serde_json::json!({
            "network": "signet",
            "objective": "Legacy mandate",
            "max_venue_balance_sats": 100_000,
            "max_position_usd": 500,
            "max_leverage": 3,
            "daily_loss_stop_sats": 5_000,
            "allowed_strategies": ["rebalance_to_target"],
            "review_cadence": { "type": "funding_settlement" },
            "expires_at_ms": 10_000
        });
        let decoded: TradingMandate = serde_json::from_value(value).expect("legacy mandate");

        assert_eq!(decoded.max_orders_per_hour, 0);
        assert_eq!(decoded.min_liquidation_buffer_bps, 10_000);
    }

    #[test]
    fn creation_and_every_widening_dimension_require_approval() {
        let store = MandateStore::in_memory().expect("store");
        let initial = mandate();
        let creation = store.propose(initial.clone()).expect("creation");
        assert_eq!(creation.change_class(), MandateChangeClass::Creation);
        assert!(store.save_restriction(creation, 1).is_err());
        approve(&store, initial.clone(), 1);

        let mut candidates = Vec::new();
        let mut network = initial.clone();
        network.network = TradingNetwork::Mainnet;
        candidates.push(network);
        let mut objective = initial.clone();
        objective.objective = "A changed objective".into();
        candidates.push(objective);
        let mut venue = initial.clone();
        venue.max_venue_balance_sats += 1;
        candidates.push(venue);
        let mut position = initial.clone();
        position.max_position_usd += 1;
        candidates.push(position);
        let mut leverage = initial.clone();
        leverage.max_leverage += 1;
        candidates.push(leverage);
        let mut loss = initial.clone();
        loss.daily_loss_stop_sats += 1;
        candidates.push(loss);
        let mut order_count = initial.clone();
        order_count.max_orders_per_hour += 1;
        candidates.push(order_count);
        let mut liquidation_buffer = initial.clone();
        liquidation_buffer.min_liquidation_buffer_bps -= 1;
        candidates.push(liquidation_buffer);
        let mut strategies = initial.clone();
        strategies.allowed_strategies.insert("funding_carry".into());
        candidates.push(strategies);
        let mut cadence = initial.clone();
        cadence.review_cadence = ReviewCadence::Interval { seconds: 3_600 };
        candidates.push(cadence);
        let mut expiry = initial;
        expiry.expires_at_ms += 1;
        candidates.push(expiry);

        for candidate in candidates {
            let proposal = store.propose(candidate).expect("widening proposal");
            assert_eq!(proposal.change_class(), MandateChangeClass::Widening);
            assert!(store.save_restriction(proposal, 2).is_err());
        }
    }

    #[test]
    fn narrowing_and_revocation_apply_without_approval() {
        let store = MandateStore::in_memory().expect("store");
        let initial = mandate();
        approve(&store, initial.clone(), 1);
        let mut narrow = initial;
        narrow.max_position_usd -= 1;
        narrow.allowed_strategies.clear();
        narrow
            .allowed_strategies
            .insert("rebalance_to_target".into());
        narrow.expires_at_ms -= 1;
        let proposal = store.propose(narrow.clone()).expect("restriction");
        assert_eq!(proposal.change_class(), MandateChangeClass::Restriction);
        let snapshot = store.save_restriction(proposal, 2).expect("save");
        assert_eq!(snapshot.mandate, Some(narrow));
        assert!(store.revoke(3).expect("revoke").mandate.is_none());
    }

    #[test]
    fn approval_is_bound_to_the_displayed_revision_and_candidate() {
        let store = MandateStore::in_memory().expect("store");
        let initial = mandate();
        approve(&store, initial.clone(), 1);
        let mut wider = initial.clone();
        wider.max_leverage += 1;
        let stale = store.propose(wider).expect("widening");

        let mut narrower = initial;
        narrower.max_position_usd -= 1;
        let restriction = store.propose(narrower).expect("restriction");
        store.save_restriction(restriction, 2).expect("save");
        assert!(store.apply_ui_approved(stale, 3).is_err());
    }

    #[test]
    fn instruction_enforcement_fails_closed_for_every_limit() {
        let store = MandateStore::in_memory().expect("store");
        assert_eq!(
            store.authorize(&instruction(), 1).expect("decision"),
            refused(MandateRefusal::Missing)
        );
        approve(&store, mandate(), 1);
        assert_eq!(
            store.authorize(&instruction(), 2).expect("decision"),
            MandateDecision::Authorized { revision: 1 }
        );

        let cases = [
            (
                TradingInstruction {
                    network: TradingNetwork::Mainnet,
                    ..instruction()
                },
                MandateRefusal::NetworkNotAllowed,
            ),
            (
                TradingInstruction {
                    strategy_id: "funding_carry".into(),
                    ..instruction()
                },
                MandateRefusal::StrategyNotAllowed {
                    strategy_id: "funding_carry".into(),
                },
            ),
            (
                TradingInstruction {
                    venue_balance_after_sats: 100_001,
                    ..instruction()
                },
                MandateRefusal::VenueBalanceLimit {
                    limit_sats: 100_000,
                    requested_sats: 100_001,
                },
            ),
            (
                TradingInstruction {
                    position_notional_usd: 501,
                    ..instruction()
                },
                MandateRefusal::PositionLimit {
                    limit_usd: 500,
                    requested_usd: 501,
                },
            ),
            (
                TradingInstruction {
                    leverage: 4,
                    ..instruction()
                },
                MandateRefusal::LeverageLimit {
                    limit: 3,
                    requested: 4,
                },
            ),
            (
                TradingInstruction {
                    daily_realized_loss_sats: 5_000,
                    ..instruction()
                },
                MandateRefusal::DailyLossStop {
                    limit_sats: 5_000,
                    loss_sats: 5_000,
                },
            ),
            (
                TradingInstruction {
                    orders_last_hour: 12,
                    ..instruction()
                },
                MandateRefusal::HourlyOrderLimit {
                    limit: 12,
                    orders_last_hour: 12,
                },
            ),
            (
                TradingInstruction {
                    liquidation_buffer_bps: 1_499,
                    ..instruction()
                },
                MandateRefusal::LiquidationBufferFloor {
                    minimum_bps: 1_500,
                    current_bps: 1_499,
                },
            ),
        ];
        for (request, reason) in cases {
            assert_eq!(
                store.authorize(&request, 2).expect("decision"),
                refused(reason)
            );
        }
        assert_eq!(
            store.authorize(&instruction(), 10_000).expect("expiry"),
            refused(MandateRefusal::Expired {
                expires_at_ms: 10_000
            })
        );
    }

    #[test]
    fn history_is_append_only_and_rejects_unapproved_widening() {
        let store = MandateStore::in_memory().expect("store");
        approve(&store, mandate(), 1);
        assert!(
            store
                .connection
                .lock()
                .execute(
                    "UPDATE trading_mandate_revisions SET changed_at_ms = 2 WHERE revision = 1",
                    [],
                )
                .is_err()
        );
        store
            .connection
            .lock()
            .execute_batch("DROP TRIGGER trading_mandate_no_update")
            .expect("drop test trigger");
        store
            .connection
            .lock()
            .execute(
                "UPDATE trading_mandate_revisions SET approval_digest = NULL WHERE revision = 1",
                [],
            )
            .expect("tamper fixture");
        assert!(store.verify().is_err());
    }

    #[test]
    fn store_survives_restart_beside_the_thread_store_shape() {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("threads/trading-mandate.db");
        let store = MandateStore::open(&path).expect("store");
        approve(&store, mandate(), 1);
        drop(store);
        let reopened = MandateStore::open(&path).expect("reopen");
        assert_eq!(reopened.snapshot().expect("snapshot").revision, 1);
        assert_eq!(reopened.history().expect("history").len(), 1);
    }
}
