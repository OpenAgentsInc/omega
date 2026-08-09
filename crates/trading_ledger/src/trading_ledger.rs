use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use parking_lot::Mutex;
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const MAX_IDENTIFIER_LENGTH: usize = 200;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LedgerAccount {
    VenueBalance { venue: String },
    TradingProfit,
    FeeExpense,
    FundingIncome,
    External,
    BalanceAdjustment,
}

impl LedgerAccount {
    fn validate(&self) -> Result<()> {
        if let Self::VenueBalance { venue } = self {
            validate_identifier("venue", venue)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LedgerEntryKind {
    Order,
    Fill,
    Fee,
    FundingSettlement,
    Deposit,
    Withdrawal,
    BalanceAdjustment,
    ReconciliationMismatch {
        venue: String,
        expected_sats: i64,
        observed_sats: i64,
        difference_sats: i64,
    },
}

impl LedgerEntryKind {
    fn requires_postings(&self) -> bool {
        matches!(
            self,
            Self::Fee
                | Self::FundingSettlement
                | Self::Deposit
                | Self::Withdrawal
                | Self::BalanceAdjustment
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LedgerPosting {
    pub account: LedgerAccount,
    pub amount_sats: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntryDraft {
    pub event_id: String,
    pub occurred_at_ms: i64,
    pub strategy_id: String,
    pub kind: LedgerEntryKind,
    pub postings: Vec<LedgerPosting>,
    pub metadata: Value,
}

impl LedgerEntryDraft {
    pub fn new(
        event_id: impl Into<String>,
        occurred_at_ms: i64,
        strategy_id: impl Into<String>,
        kind: LedgerEntryKind,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            occurred_at_ms,
            strategy_id: strategy_id.into(),
            kind,
            postings: Vec::new(),
            metadata: Value::Object(Default::default()),
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_identifier("event ID", &self.event_id)?;
        validate_identifier("strategy ID", &self.strategy_id)?;
        if self.occurred_at_ms < 0 {
            bail!("ledger event timestamp must not be negative");
        }
        if !self.metadata.is_object() {
            bail!("ledger metadata must be a JSON object");
        }
        if let LedgerEntryKind::ReconciliationMismatch {
            venue,
            expected_sats,
            observed_sats,
            difference_sats,
        } = &self.kind
        {
            validate_identifier("venue", venue)?;
            let expected_difference = observed_sats
                .checked_sub(*expected_sats)
                .context("reconciliation difference overflowed")?;
            if *difference_sats != expected_difference {
                bail!("reconciliation alert carries an invalid balance difference");
            }
        }
        if self.kind.requires_postings() && self.postings.is_empty() {
            bail!(
                "ledger {:?} entry requires double-entry postings",
                self.kind
            );
        }
        if self.postings.is_empty() {
            return Ok(());
        }
        if self.postings.len() < 2 {
            bail!("a financial ledger entry requires at least two postings");
        }
        let mut accounts = BTreeSet::new();
        let mut sum = 0_i64;
        for posting in &self.postings {
            posting.account.validate()?;
            if posting.amount_sats == 0 {
                bail!("ledger postings must not have a zero amount");
            }
            if !accounts.insert(&posting.account) {
                bail!("a ledger entry must not repeat an account");
            }
            sum = sum
                .checked_add(posting.amount_sats)
                .context("ledger posting sum overflowed")?;
        }
        if sum != 0 {
            bail!("ledger postings do not balance: {sum} sats");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub sequence: u64,
    pub event_id: String,
    pub occurred_at_ms: i64,
    pub strategy_id: String,
    pub kind: LedgerEntryKind,
    pub postings: Vec<LedgerPosting>,
    pub metadata: Value,
    pub previous_hash: String,
    pub entry_hash: String,
}

impl LedgerEntry {
    fn as_draft(&self) -> LedgerEntryDraft {
        LedgerEntryDraft {
            event_id: self.event_id.clone(),
            occurred_at_ms: self.occurred_at_ms,
            strategy_id: self.strategy_id.clone(),
            kind: self.kind.clone(),
            postings: self.postings.clone(),
            metadata: self.metadata.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LedgerQuery {
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub strategy_id: Option<String>,
}

impl LedgerQuery {
    fn validate(&self) -> Result<()> {
        if let Some(from_ms) = self.from_ms
            && from_ms < 0
        {
            bail!("ledger query start timestamp must not be negative");
        }
        if let Some(to_ms) = self.to_ms
            && to_ms < 0
        {
            bail!("ledger query end timestamp must not be negative");
        }
        if let (Some(from_ms), Some(to_ms)) = (self.from_ms, self.to_ms)
            && from_ms > to_ms
        {
            bail!("ledger query start must not be after its end");
        }
        if let Some(strategy_id) = &self.strategy_id {
            validate_identifier("strategy ID", strategy_id)?;
        }
        Ok(())
    }

    fn includes(&self, entry: &LedgerEntry) -> bool {
        self.from_ms
            .is_none_or(|from_ms| entry.occurred_at_ms >= from_ms)
            && self.to_ms.is_none_or(|to_ms| entry.occurred_at_ms <= to_ms)
            && self
                .strategy_id
                .as_ref()
                .is_none_or(|strategy_id| &entry.strategy_id == strategy_id)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StrategyProfit {
    pub strategy_id: String,
    pub profit_sats: i64,
    pub fees_paid_sats: i64,
    pub funding_collected_sats: i64,
    pub worst_drawdown_sats: i64,
    pub entry_count: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfitReport {
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub strategies: Vec<StrategyProfit>,
    pub total_profit_sats: i64,
    pub total_fees_paid_sats: i64,
    pub total_funding_collected_sats: i64,
    pub worst_drawdown_sats: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReconciliationOutcome {
    Matched { venue: String, balance_sats: i64 },
    Mismatch { alert: LedgerEntry },
}

#[derive(Clone)]
pub struct LedgerStore {
    connection: Arc<Mutex<Connection>>,
}

impl LedgerStore {
    pub fn default_path() -> PathBuf {
        paths::data_dir().join("threads").join("trading-ledger.db")
    }

    pub fn open_default() -> Result<Self> {
        Self::open(&Self::default_path())
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create trading ledger directory {parent:?}"))?;
        }
        let connection = Connection::open(path)
            .with_context(|| format!("could not open trading ledger {path:?}"))?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS trading_ledger_entries (
                 sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
                 event_id TEXT NOT NULL UNIQUE,
                 occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms >= 0),
                 strategy_id TEXT NOT NULL,
                 kind_json TEXT NOT NULL,
                 metadata_json TEXT NOT NULL,
                 previous_hash TEXT NOT NULL,
                 entry_hash TEXT NOT NULL UNIQUE
             ) STRICT;
             CREATE TABLE IF NOT EXISTS trading_ledger_postings (
                 sequence INTEGER NOT NULL,
                 posting_index INTEGER NOT NULL CHECK (posting_index >= 0),
                 account_json TEXT NOT NULL,
                 amount_sats INTEGER NOT NULL CHECK (amount_sats != 0),
                 PRIMARY KEY (sequence, posting_index),
                 FOREIGN KEY (sequence) REFERENCES trading_ledger_entries(sequence)
             ) STRICT;
             CREATE TRIGGER IF NOT EXISTS trading_ledger_entries_no_update
             BEFORE UPDATE ON trading_ledger_entries BEGIN
                 SELECT RAISE(ABORT, 'trading ledger entries are append-only');
             END;
             CREATE TRIGGER IF NOT EXISTS trading_ledger_entries_no_delete
             BEFORE DELETE ON trading_ledger_entries BEGIN
                 SELECT RAISE(ABORT, 'trading ledger entries are append-only');
             END;
             CREATE TRIGGER IF NOT EXISTS trading_ledger_postings_no_update
             BEFORE UPDATE ON trading_ledger_postings BEGIN
                 SELECT RAISE(ABORT, 'trading ledger postings are append-only');
             END;
             CREATE TRIGGER IF NOT EXISTS trading_ledger_postings_no_delete
             BEFORE DELETE ON trading_ledger_postings BEGIN
                 SELECT RAISE(ABORT, 'trading ledger postings are append-only');
             END;",
        )?;
        let store = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        store.verify()?;
        Ok(store)
    }

    pub fn append(&self, draft: LedgerEntryDraft) -> Result<LedgerEntry> {
        draft.validate()?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let entries = load_entries(&transaction)?;
        verify_entries(&entries)?;
        let entry = append_verified(&transaction, draft, &entries)?;
        transaction.commit()?;
        Ok(entry)
    }

    pub fn verify(&self) -> Result<()> {
        verify_entries(&load_entries(&self.connection.lock())?)
    }

    pub fn entries(&self, query: &LedgerQuery) -> Result<Vec<LedgerEntry>> {
        query.validate()?;
        let entries = load_entries(&self.connection.lock())?;
        verify_entries(&entries)?;
        Ok(entries
            .into_iter()
            .filter(|entry| query.includes(entry))
            .collect())
    }

    pub fn venue_balance(&self, venue: &str) -> Result<i64> {
        validate_identifier("venue", venue)?;
        let entries = self.entries(&LedgerQuery::default())?;
        venue_balance_from_entries(&entries, venue)
    }

    pub fn reconcile(
        &self,
        event_id: impl Into<String>,
        occurred_at_ms: i64,
        strategy_id: impl Into<String>,
        venue: impl Into<String>,
        observed_sats: i64,
    ) -> Result<ReconciliationOutcome> {
        let event_id = event_id.into();
        let strategy_id = strategy_id.into();
        let venue = venue.into();
        validate_identifier("event ID", &event_id)?;
        validate_identifier("strategy ID", &strategy_id)?;
        validate_identifier("venue", &venue)?;
        if occurred_at_ms < 0 {
            bail!("reconciliation timestamp must not be negative");
        }
        if observed_sats < 0 {
            bail!("observed venue balance must not be negative");
        }
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let entries = load_entries(&transaction)?;
        verify_entries(&entries)?;
        let expected_sats = venue_balance_from_entries(&entries, &venue)?;
        let difference_sats = observed_sats
            .checked_sub(expected_sats)
            .context("reconciliation difference overflowed")?;
        if difference_sats == 0 {
            transaction.commit()?;
            return Ok(ReconciliationOutcome::Matched {
                venue,
                balance_sats: observed_sats,
            });
        }
        let draft = LedgerEntryDraft::new(
            event_id,
            occurred_at_ms,
            strategy_id,
            LedgerEntryKind::ReconciliationMismatch {
                venue,
                expected_sats,
                observed_sats,
                difference_sats,
            },
        );
        draft.validate()?;
        let alert = append_verified(&transaction, draft, &entries)?;
        transaction.commit()?;
        Ok(ReconciliationOutcome::Mismatch { alert })
    }

    pub fn profit_report(&self, query: &LedgerQuery) -> Result<ProfitReport> {
        let entries = self.entries(query)?;
        let mut strategies = BTreeMap::<String, ProfitAccumulator>::new();
        let mut total = ProfitAccumulator::default();
        for entry in entries {
            let delta = entry_profit_delta(&entry)?;
            strategies
                .entry(entry.strategy_id.clone())
                .or_default()
                .record(&entry, delta)?;
            total.record(&entry, delta)?;
        }
        Ok(ProfitReport {
            from_ms: query.from_ms,
            to_ms: query.to_ms,
            strategies: strategies
                .into_iter()
                .map(|(strategy_id, accumulator)| accumulator.finish(strategy_id))
                .collect(),
            total_profit_sats: total.profit_sats,
            total_fees_paid_sats: total.fees_paid_sats,
            total_funding_collected_sats: total.funding_collected_sats,
            worst_drawdown_sats: total.worst_drawdown_sats,
        })
    }
}

fn append_verified(
    transaction: &Transaction<'_>,
    draft: LedgerEntryDraft,
    entries: &[LedgerEntry],
) -> Result<LedgerEntry> {
    if let Some(existing) = entries
        .iter()
        .find(|entry| entry.event_id == draft.event_id)
    {
        if existing.as_draft() == draft {
            return Ok(existing.clone());
        }
        bail!(
            "ledger event ID {:?} already names different content",
            draft.event_id
        );
    }

    let sequence = entries.last().map_or(Ok(1_u64), |entry| {
        entry
            .sequence
            .checked_add(1)
            .context("ledger sequence overflowed")
    })?;
    let previous_hash = entries
        .last()
        .map(|entry| entry.entry_hash.clone())
        .unwrap_or_else(|| GENESIS_HASH.to_owned());
    let entry_hash = hash_entry(sequence, &previous_hash, &draft)?;
    let sequence_i64 = i64::try_from(sequence).context("ledger sequence exceeded SQLite range")?;
    transaction.execute(
        "INSERT INTO trading_ledger_entries (
             sequence, event_id, occurred_at_ms, strategy_id, kind_json, metadata_json,
             previous_hash, entry_hash
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            sequence_i64,
            draft.event_id,
            draft.occurred_at_ms,
            draft.strategy_id,
            serde_json::to_string(&draft.kind)?,
            serde_json::to_string(&draft.metadata)?,
            previous_hash,
            entry_hash,
        ],
    )?;
    for (posting_index, posting) in draft.postings.iter().enumerate() {
        transaction.execute(
            "INSERT INTO trading_ledger_postings (
                 sequence, posting_index, account_json, amount_sats
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                sequence_i64,
                i64::try_from(posting_index).context("ledger posting index overflowed")?,
                serde_json::to_string(&posting.account)?,
                posting.amount_sats,
            ],
        )?;
    }
    Ok(LedgerEntry {
        sequence,
        event_id: draft.event_id,
        occurred_at_ms: draft.occurred_at_ms,
        strategy_id: draft.strategy_id,
        kind: draft.kind,
        postings: draft.postings,
        metadata: draft.metadata,
        previous_hash,
        entry_hash,
    })
}

fn venue_balance_from_entries(entries: &[LedgerEntry], venue: &str) -> Result<i64> {
    entries
        .iter()
        .flat_map(|entry| &entry.postings)
        .filter_map(|posting| match &posting.account {
            LedgerAccount::VenueBalance {
                venue: posting_venue,
            } if posting_venue == venue => Some(posting.amount_sats),
            _ => None,
        })
        .try_fold(0_i64, |balance, amount| {
            balance
                .checked_add(amount)
                .context("venue balance overflowed")
        })
}

#[derive(Default)]
struct ProfitAccumulator {
    profit_sats: i64,
    fees_paid_sats: i64,
    funding_collected_sats: i64,
    peak_profit_sats: i64,
    worst_drawdown_sats: i64,
    entry_count: usize,
}

impl ProfitAccumulator {
    fn record(&mut self, entry: &LedgerEntry, profit_delta: i64) -> Result<()> {
        self.profit_sats = self
            .profit_sats
            .checked_add(profit_delta)
            .context("ledger profit overflowed")?;
        for posting in &entry.postings {
            match posting.account {
                LedgerAccount::FeeExpense if posting.amount_sats > 0 => {
                    self.fees_paid_sats = self
                        .fees_paid_sats
                        .checked_add(posting.amount_sats)
                        .context("ledger fee total overflowed")?;
                }
                LedgerAccount::FundingIncome => {
                    self.funding_collected_sats = self
                        .funding_collected_sats
                        .checked_sub(posting.amount_sats)
                        .context("ledger funding total overflowed")?;
                }
                _ => {}
            }
        }
        self.peak_profit_sats = self.peak_profit_sats.max(self.profit_sats);
        self.worst_drawdown_sats = self.worst_drawdown_sats.max(
            self.peak_profit_sats
                .checked_sub(self.profit_sats)
                .context("ledger drawdown overflowed")?,
        );
        self.entry_count = self.entry_count.saturating_add(1);
        Ok(())
    }

    fn finish(self, strategy_id: String) -> StrategyProfit {
        StrategyProfit {
            strategy_id,
            profit_sats: self.profit_sats,
            fees_paid_sats: self.fees_paid_sats,
            funding_collected_sats: self.funding_collected_sats,
            worst_drawdown_sats: self.worst_drawdown_sats,
            entry_count: self.entry_count,
        }
    }
}

fn entry_profit_delta(entry: &LedgerEntry) -> Result<i64> {
    let profit_accounts = entry
        .postings
        .iter()
        .filter_map(|posting| match posting.account {
            LedgerAccount::TradingProfit
            | LedgerAccount::FeeExpense
            | LedgerAccount::FundingIncome => Some(posting.amount_sats),
            _ => None,
        })
        .try_fold(0_i64, |sum, amount| {
            sum.checked_add(amount)
                .context("ledger profit postings overflowed")
        })?;
    profit_accounts
        .checked_neg()
        .context("ledger profit could not be negated")
}

fn validate_identifier(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("ledger {label} must not be empty");
    }
    if value.len() > MAX_IDENTIFIER_LENGTH {
        bail!("ledger {label} must not exceed {MAX_IDENTIFIER_LENGTH} bytes");
    }
    Ok(())
}

fn load_entries(connection: &Connection) -> Result<Vec<LedgerEntry>> {
    let mut statement = connection.prepare(
        "SELECT sequence, event_id, occurred_at_ms, strategy_id, kind_json, metadata_json,
                previous_hash, entry_hash
         FROM trading_ledger_entries ORDER BY sequence",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
        ))
    })?;
    let mut entries = Vec::new();
    for row in rows {
        let (
            sequence,
            event_id,
            occurred_at_ms,
            strategy_id,
            kind_json,
            metadata_json,
            previous_hash,
            entry_hash,
        ) = row?;
        let sequence = u64::try_from(sequence).context("ledger sequence was negative")?;
        entries.push(LedgerEntry {
            sequence,
            event_id,
            occurred_at_ms,
            strategy_id,
            kind: serde_json::from_str(&kind_json)
                .context("ledger entry kind is not valid JSON")?,
            postings: load_postings(connection, sequence)?,
            metadata: serde_json::from_str(&metadata_json)
                .context("ledger entry metadata is not valid JSON")?,
            previous_hash,
            entry_hash,
        });
    }
    Ok(entries)
}

fn load_postings(connection: &Connection, sequence: u64) -> Result<Vec<LedgerPosting>> {
    let mut statement = connection.prepare(
        "SELECT account_json, amount_sats FROM trading_ledger_postings
         WHERE sequence = ?1 ORDER BY posting_index",
    )?;
    let rows = statement.query_map(
        params![i64::try_from(sequence).context("ledger sequence exceeded SQLite range")?],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    )?;
    let mut postings = Vec::new();
    for row in rows {
        let (account_json, amount_sats) = row?;
        postings.push(LedgerPosting {
            account: serde_json::from_str(&account_json)
                .context("ledger posting account is not valid JSON")?,
            amount_sats,
        });
    }
    Ok(postings)
}

fn verify_entries(entries: &[LedgerEntry]) -> Result<()> {
    let mut expected_sequence = 1_u64;
    let mut expected_previous_hash = GENESIS_HASH.to_owned();
    for entry in entries {
        if entry.sequence != expected_sequence {
            bail!(
                "trading ledger sequence gap: expected {}, found {}",
                expected_sequence,
                entry.sequence
            );
        }
        if entry.previous_hash != expected_previous_hash {
            bail!(
                "trading ledger hash chain is broken at sequence {}",
                entry.sequence
            );
        }
        let draft = entry.as_draft();
        draft.validate().with_context(|| {
            format!(
                "invalid trading ledger entry at sequence {}",
                entry.sequence
            )
        })?;
        let expected_hash = hash_entry(entry.sequence, &entry.previous_hash, &draft)?;
        if entry.entry_hash != expected_hash {
            bail!(
                "trading ledger entry hash mismatch at sequence {}",
                entry.sequence
            );
        }
        expected_previous_hash = entry.entry_hash.clone();
        expected_sequence = expected_sequence
            .checked_add(1)
            .context("ledger sequence overflowed during verification")?;
    }
    Ok(())
}

#[derive(Serialize)]
struct EntryHashPayload<'a> {
    sequence: u64,
    previous_hash: &'a str,
    event_id: &'a str,
    occurred_at_ms: i64,
    strategy_id: &'a str,
    kind: &'a LedgerEntryKind,
    postings: &'a [LedgerPosting],
    metadata: &'a Value,
}

fn hash_entry(sequence: u64, previous_hash: &str, draft: &LedgerEntryDraft) -> Result<String> {
    let payload = serde_json::to_vec(&EntryHashPayload {
        sequence,
        previous_hash,
        event_id: &draft.event_id,
        occurred_at_ms: draft.occurred_at_ms,
        strategy_id: &draft.strategy_id,
        kind: &draft.kind,
        postings: &draft.postings,
        metadata: &draft.metadata,
    })?;
    Ok(format!("{:x}", Sha256::digest(payload)))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn venue() -> LedgerAccount {
        LedgerAccount::VenueBalance {
            venue: "lnmarkets".into(),
        }
    }

    fn posting(account: LedgerAccount, amount_sats: i64) -> LedgerPosting {
        LedgerPosting {
            account,
            amount_sats,
        }
    }

    fn draft(
        event_id: &str,
        occurred_at_ms: i64,
        strategy_id: &str,
        kind: LedgerEntryKind,
        postings: Vec<LedgerPosting>,
    ) -> LedgerEntryDraft {
        LedgerEntryDraft {
            event_id: event_id.into(),
            occurred_at_ms,
            strategy_id: strategy_id.into(),
            kind,
            postings,
            metadata: json!({"source": "test"}),
        }
    }

    #[test]
    fn appends_every_entry_type_in_one_contiguous_hash_chain() {
        let store = LedgerStore::in_memory().expect("store");
        let entries = [
            draft("order", 1, "alpha", LedgerEntryKind::Order, vec![]),
            draft(
                "fill",
                2,
                "alpha",
                LedgerEntryKind::Fill,
                vec![
                    posting(venue(), 20),
                    posting(LedgerAccount::TradingProfit, -20),
                ],
            ),
            draft(
                "fee",
                3,
                "alpha",
                LedgerEntryKind::Fee,
                vec![posting(venue(), -5), posting(LedgerAccount::FeeExpense, 5)],
            ),
            draft(
                "funding",
                4,
                "alpha",
                LedgerEntryKind::FundingSettlement,
                vec![
                    posting(venue(), 3),
                    posting(LedgerAccount::FundingIncome, -3),
                ],
            ),
            draft(
                "deposit",
                5,
                "alpha",
                LedgerEntryKind::Deposit,
                vec![posting(venue(), 50), posting(LedgerAccount::External, -50)],
            ),
            draft(
                "withdrawal",
                6,
                "alpha",
                LedgerEntryKind::Withdrawal,
                vec![posting(venue(), -10), posting(LedgerAccount::External, 10)],
            ),
            draft(
                "adjustment",
                7,
                "alpha",
                LedgerEntryKind::BalanceAdjustment,
                vec![
                    posting(venue(), 100),
                    posting(LedgerAccount::BalanceAdjustment, -100),
                ],
            ),
        ];
        for (index, draft) in entries.into_iter().enumerate() {
            assert_eq!(
                store.append(draft).expect("append").sequence,
                u64::try_from(index + 1).expect("sequence")
            );
        }
        store.verify().expect("verify");
        assert_eq!(store.venue_balance("lnmarkets").expect("balance"), 158);
    }

    #[test]
    fn duplicate_event_is_idempotent_but_conflicting_content_is_rejected() {
        let store = LedgerStore::in_memory().expect("store");
        let event = draft("same", 1, "alpha", LedgerEntryKind::Order, vec![]);
        assert_eq!(store.append(event.clone()).expect("first").sequence, 1);
        assert_eq!(store.append(event).expect("replay").sequence, 1);

        let error = store
            .append(draft("same", 2, "alpha", LedgerEntryKind::Order, vec![]))
            .expect_err("conflicting replay");
        assert!(error.to_string().contains("different content"));
    }

    #[test]
    fn unbalanced_or_single_sided_financial_entries_are_rejected() {
        let store = LedgerStore::in_memory().expect("store");
        let unbalanced = draft(
            "bad",
            1,
            "alpha",
            LedgerEntryKind::Fee,
            vec![posting(venue(), -5), posting(LedgerAccount::FeeExpense, 4)],
        );
        assert!(
            store
                .append(unbalanced)
                .expect_err("unbalanced")
                .to_string()
                .contains("do not balance")
        );
        let single_sided = draft(
            "bad-single",
            2,
            "alpha",
            LedgerEntryKind::Fill,
            vec![posting(venue(), 1)],
        );
        assert!(
            store
                .append(single_sided)
                .expect_err("single sided")
                .to_string()
                .contains("at least two postings")
        );
    }

    #[test]
    fn sequence_gaps_fail_closed_for_reads_and_writes() {
        let store = LedgerStore::in_memory().expect("store");
        for event_id in ["one", "two", "three"] {
            store
                .append(draft(event_id, 1, "alpha", LedgerEntryKind::Order, vec![]))
                .expect("append");
        }
        store
            .connection
            .lock()
            .execute_batch(
                "DROP TRIGGER trading_ledger_entries_no_delete;
                 DELETE FROM trading_ledger_entries WHERE sequence = 2;",
            )
            .expect("corrupt fixture");

        assert!(
            store
                .verify()
                .expect_err("gap")
                .to_string()
                .contains("sequence gap")
        );
        assert!(
            store
                .append(draft("four", 4, "alpha", LedgerEntryKind::Order, vec![]))
                .expect_err("append across gap")
                .to_string()
                .contains("sequence gap")
        );
    }

    #[test]
    fn sqlite_refuses_updates_and_deletes() {
        let store = LedgerStore::in_memory().expect("store");
        store
            .append(draft("one", 1, "alpha", LedgerEntryKind::Order, vec![]))
            .expect("append");
        let connection = store.connection.lock();
        assert!(
            connection
                .execute("UPDATE trading_ledger_entries SET strategy_id = 'beta'", [])
                .expect_err("update")
                .to_string()
                .contains("append-only")
        );
        assert!(
            connection
                .execute("DELETE FROM trading_ledger_entries", [])
                .expect_err("delete")
                .to_string()
                .contains("append-only")
        );
    }

    #[test]
    fn hash_tampering_is_detected() {
        let store = LedgerStore::in_memory().expect("store");
        store
            .append(draft("one", 1, "alpha", LedgerEntryKind::Order, vec![]))
            .expect("append");
        store
            .connection
            .lock()
            .execute_batch(
                "DROP TRIGGER trading_ledger_entries_no_update;
                 UPDATE trading_ledger_entries SET metadata_json = '{\"tampered\":true}';",
            )
            .expect("tamper fixture");
        assert!(
            store
                .verify()
                .expect_err("tampering")
                .to_string()
                .contains("hash mismatch")
        );
    }

    #[test]
    fn reconciliation_mismatch_appends_an_alert_without_correcting_balance() {
        let store = LedgerStore::in_memory().expect("store");
        store
            .append(draft(
                "opening",
                1,
                "system",
                LedgerEntryKind::BalanceAdjustment,
                vec![
                    posting(venue(), 100),
                    posting(LedgerAccount::BalanceAdjustment, -100),
                ],
            ))
            .expect("opening balance");
        let outcome = store
            .reconcile("snapshot-1", 2, "system", "lnmarkets", 90)
            .expect("reconcile");
        let ReconciliationOutcome::Mismatch { alert } = outcome else {
            panic!("expected mismatch");
        };
        assert_eq!(alert.sequence, 2);
        assert!(matches!(
            alert.kind,
            LedgerEntryKind::ReconciliationMismatch {
                expected_sats: 100,
                observed_sats: 90,
                difference_sats: -10,
                ..
            }
        ));
        assert!(alert.postings.is_empty());
        assert_eq!(store.venue_balance("lnmarkets").expect("balance"), 100);
        assert!(matches!(
            store
                .reconcile("snapshot-2", 3, "system", "lnmarkets", 100)
                .expect("matched reconciliation"),
            ReconciliationOutcome::Matched { .. }
        ));
        assert_eq!(
            store
                .entries(&LedgerQuery::default())
                .expect("entries")
                .len(),
            2
        );
    }

    #[test]
    fn profit_report_attributes_profit_fees_funding_and_drawdown() {
        let store = LedgerStore::in_memory().expect("store");
        for event in [
            draft(
                "profit",
                10,
                "alpha",
                LedgerEntryKind::Fill,
                vec![
                    posting(venue(), 20),
                    posting(LedgerAccount::TradingProfit, -20),
                ],
            ),
            draft(
                "fee",
                20,
                "alpha",
                LedgerEntryKind::Fee,
                vec![posting(venue(), -5), posting(LedgerAccount::FeeExpense, 5)],
            ),
            draft(
                "funding",
                30,
                "alpha",
                LedgerEntryKind::FundingSettlement,
                vec![
                    posting(venue(), 3),
                    posting(LedgerAccount::FundingIncome, -3),
                ],
            ),
            draft(
                "beta-fee",
                40,
                "beta",
                LedgerEntryKind::Fee,
                vec![posting(venue(), -4), posting(LedgerAccount::FeeExpense, 4)],
            ),
        ] {
            store.append(event).expect("append");
        }

        let report = store
            .profit_report(&LedgerQuery::default())
            .expect("report");
        assert_eq!(report.total_profit_sats, 14);
        assert_eq!(report.total_fees_paid_sats, 9);
        assert_eq!(report.total_funding_collected_sats, 3);
        assert_eq!(report.worst_drawdown_sats, 6);
        assert_eq!(report.strategies.len(), 2);
        assert_eq!(
            report.strategies[0],
            StrategyProfit {
                strategy_id: "alpha".into(),
                profit_sats: 18,
                fees_paid_sats: 5,
                funding_collected_sats: 3,
                worst_drawdown_sats: 5,
                entry_count: 3,
            }
        );

        let alpha_after_fee = store
            .profit_report(&LedgerQuery {
                from_ms: Some(20),
                to_ms: Some(30),
                strategy_id: Some("alpha".into()),
            })
            .expect("filtered report");
        assert_eq!(alpha_after_fee.total_profit_sats, -2);
        assert_eq!(alpha_after_fee.worst_drawdown_sats, 5);
    }

    #[test]
    fn ledger_survives_restart_next_to_the_thread_store_shape() {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("threads").join("trading-ledger.db");
        let store = LedgerStore::open(&path).expect("store");
        let entry = store
            .append(draft("one", 1, "alpha", LedgerEntryKind::Order, vec![]))
            .expect("append");
        drop(store);

        let reopened = LedgerStore::open(&path).expect("reopen");
        assert_eq!(
            reopened.entries(&LedgerQuery::default()).expect("entries"),
            vec![entry]
        );
    }
}
