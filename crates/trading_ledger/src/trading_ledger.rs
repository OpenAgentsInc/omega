use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use parking_lot::Mutex;
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize, Serializer, ser::SerializeStruct as _};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const MAX_IDENTIFIER_LENGTH: usize = 200;
const MAX_ASSET_ID_LENGTH: usize = 32;

/// Ledger schema version stored in SQLite `user_version`. Version 1 databases
/// hard-coded sats; version 2 postings carry an explicit asset; version 3 adds
/// append-only derived counterparty-exposure observations.
pub const LEDGER_SCHEMA_VERSION: i64 = 3;
pub const COUNTERPARTY_EXPOSURE_DIVERGENCE_THRESHOLD_BPS: u32 = 500;

/// A validated asset identifier. Amounts in this ledger are integers in the
/// asset's smallest venue-native unit (satoshis for `sats`, micro-USDC style
/// integer units for `usdc`, and so on); the ledger never converts between
/// assets.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AssetId(String);

impl AssetId {
    pub fn sats() -> Self {
        Self("sats".to_owned())
    }

    pub fn usdc() -> Self {
        Self("usdc".to_owned())
    }

    pub fn new(id: impl Into<String>) -> Result<Self> {
        let id = id.into();
        if id.is_empty() {
            bail!("an asset identifier must not be empty");
        }
        if id.len() > MAX_ASSET_ID_LENGTH {
            bail!("an asset identifier must not exceed {MAX_ASSET_ID_LENGTH} bytes");
        }
        let mut characters = id.chars();
        if !characters
            .next()
            .is_some_and(|first| first.is_ascii_lowercase())
        {
            bail!("an asset identifier must start with a lowercase ASCII letter");
        }
        if !id.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        }) {
            bail!(
                "an asset identifier may contain only lowercase ASCII letters, digits, and underscores"
            );
        }
        Ok(Self(id))
    }

    pub fn is_sats(&self) -> bool {
        self.0 == "sats"
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for AssetId {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<AssetId> for String {
    fn from(asset: AssetId) -> Self {
        asset.0
    }
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationMismatch {
    pub venue: String,
    pub asset: AssetId,
    pub expected: i64,
    pub observed: i64,
    pub difference: i64,
}

// Sats mismatches keep the pre-multi-asset field layout (`expected_sats` and
// friends, no `asset` field) so entry hashes recorded by earlier releases
// still verify against byte-identical serialized kinds.
impl Serialize for ReconciliationMismatch {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.asset.is_sats() {
            let mut state = serializer.serialize_struct("ReconciliationMismatch", 4)?;
            state.serialize_field("venue", &self.venue)?;
            state.serialize_field("expected_sats", &self.expected)?;
            state.serialize_field("observed_sats", &self.observed)?;
            state.serialize_field("difference_sats", &self.difference)?;
            state.end()
        } else {
            let mut state = serializer.serialize_struct("ReconciliationMismatch", 5)?;
            state.serialize_field("venue", &self.venue)?;
            state.serialize_field("asset", &self.asset)?;
            state.serialize_field("expected", &self.expected)?;
            state.serialize_field("observed", &self.observed)?;
            state.serialize_field("difference", &self.difference)?;
            state.end()
        }
    }
}

#[derive(Deserialize)]
struct ReconciliationMismatchRepr {
    venue: String,
    #[serde(default)]
    asset: Option<AssetId>,
    #[serde(default)]
    expected: Option<i64>,
    #[serde(default)]
    expected_sats: Option<i64>,
    #[serde(default)]
    observed: Option<i64>,
    #[serde(default)]
    observed_sats: Option<i64>,
    #[serde(default)]
    difference: Option<i64>,
    #[serde(default)]
    difference_sats: Option<i64>,
}

impl TryFrom<ReconciliationMismatchRepr> for ReconciliationMismatch {
    type Error = anyhow::Error;

    fn try_from(repr: ReconciliationMismatchRepr) -> Result<Self> {
        let pick = |label: &str,
                    current: Option<i64>,
                    legacy: Option<i64>|
         -> Result<(i64, bool)> {
            match (current, legacy) {
                (Some(value), None) => Ok((value, false)),
                (None, Some(value)) => Ok((value, true)),
                _ => bail!(
                    "a reconciliation mismatch requires exactly one of `{label}` and `{label}_sats`"
                ),
            }
        };
        let (expected, expected_legacy) = pick("expected", repr.expected, repr.expected_sats)?;
        let (observed, observed_legacy) = pick("observed", repr.observed, repr.observed_sats)?;
        let (difference, difference_legacy) =
            pick("difference", repr.difference, repr.difference_sats)?;
        let asset = repr.asset.unwrap_or_else(AssetId::sats);
        if (expected_legacy || observed_legacy || difference_legacy) && !asset.is_sats() {
            bail!("a reconciliation mismatch with sats-named fields must not name another asset");
        }
        Ok(Self {
            venue: repr.venue,
            asset,
            expected,
            observed,
            difference,
        })
    }
}

impl<'de> Deserialize<'de> for ReconciliationMismatch {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let repr = ReconciliationMismatchRepr::deserialize(deserializer)?;
        Self::try_from(repr).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LedgerEntryKind {
    Order,
    Cancel,
    Fill,
    Fee,
    FundingSettlement,
    Deposit,
    Withdrawal,
    BalanceAdjustment,
    ReconciliationMismatch(ReconciliationMismatch),
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerPosting {
    pub account: LedgerAccount,
    pub amount: i64,
    pub asset: AssetId,
}

impl LedgerPosting {
    pub fn new(account: LedgerAccount, amount: i64, asset: AssetId) -> Self {
        Self {
            account,
            amount,
            asset,
        }
    }

    pub fn sats(account: LedgerAccount, amount: i64) -> Self {
        Self::new(account, amount, AssetId::sats())
    }
}

// Sats postings keep the pre-multi-asset field layout (`amount_sats`, no
// `asset` field) so entry hashes recorded by earlier releases still verify
// against byte-identical serialized postings.
impl Serialize for LedgerPosting {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.asset.is_sats() {
            let mut state = serializer.serialize_struct("LedgerPosting", 2)?;
            state.serialize_field("account", &self.account)?;
            state.serialize_field("amount_sats", &self.amount)?;
            state.end()
        } else {
            let mut state = serializer.serialize_struct("LedgerPosting", 3)?;
            state.serialize_field("account", &self.account)?;
            state.serialize_field("amount", &self.amount)?;
            state.serialize_field("asset", &self.asset)?;
            state.end()
        }
    }
}

#[derive(Deserialize)]
struct LedgerPostingRepr {
    account: LedgerAccount,
    #[serde(default)]
    amount: Option<i64>,
    #[serde(default)]
    amount_sats: Option<i64>,
    #[serde(default)]
    asset: Option<AssetId>,
}

impl TryFrom<LedgerPostingRepr> for LedgerPosting {
    type Error = anyhow::Error;

    fn try_from(repr: LedgerPostingRepr) -> Result<Self> {
        let (amount, asset) = match (repr.amount, repr.amount_sats) {
            (Some(amount), None) => (amount, repr.asset.unwrap_or_else(AssetId::sats)),
            (None, Some(amount)) => {
                let asset = repr.asset.unwrap_or_else(AssetId::sats);
                if !asset.is_sats() {
                    bail!("a ledger posting with `amount_sats` must not name another asset");
                }
                (amount, asset)
            }
            _ => bail!("a ledger posting requires exactly one of `amount` and `amount_sats`"),
        };
        Ok(Self {
            account: repr.account,
            amount,
            asset,
        })
    }
}

impl<'de> Deserialize<'de> for LedgerPosting {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let repr = LedgerPostingRepr::deserialize(deserializer)?;
        Self::try_from(repr).map_err(serde::de::Error::custom)
    }
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
        if let LedgerEntryKind::ReconciliationMismatch(mismatch) = &self.kind {
            validate_identifier("venue", &mismatch.venue)?;
            let expected_difference = mismatch
                .observed
                .checked_sub(mismatch.expected)
                .context("reconciliation difference overflowed")?;
            if mismatch.difference != expected_difference {
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
        let mut asset_sums = BTreeMap::<&AssetId, i64>::new();
        for posting in &self.postings {
            posting.account.validate()?;
            if posting.amount == 0 {
                bail!("ledger postings must not have a zero amount");
            }
            if !accounts.insert((&posting.account, &posting.asset)) {
                bail!("a ledger entry must not repeat an account within one asset");
            }
            let sum = asset_sums.entry(&posting.asset).or_insert(0);
            *sum = sum
                .checked_add(posting.amount)
                .context("ledger posting sum overflowed")?;
        }
        // Balance is enforced per asset, so a cross-asset conversion can never
        // hide inside one entry; a trade is two legs that each balance, with
        // the price recorded in metadata.
        for (asset, sum) in asset_sums {
            if sum != 0 {
                bail!("ledger postings do not balance: {sum} {asset}");
            }
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssetProfit {
    pub asset: AssetId,
    pub profit: i64,
    pub fees_paid: i64,
    pub funding_collected: i64,
    pub worst_drawdown: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StrategyProfit {
    pub strategy_id: String,
    pub profit_sats: i64,
    pub fees_paid_sats: i64,
    pub funding_collected_sats: i64,
    pub worst_drawdown_sats: i64,
    #[serde(default)]
    pub assets: Vec<AssetProfit>,
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
    #[serde(default)]
    pub assets: Vec<AssetProfit>,
    #[serde(default)]
    pub counterparty_exposures: Vec<CounterpartyExposure>,
}

impl ProfitReport {
    pub fn asset_totals(&self, asset: &AssetId) -> Option<&AssetProfit> {
        self.assets.iter().find(|totals| &totals.asset == asset)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Counterparty {
    Venue { venue: String },
    Provider { provider: String },
}

impl Counterparty {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Venue { venue } => validate_identifier("venue", venue),
            Self::Provider { provider } => validate_identifier("provider", provider),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CounterpartySnapshot {
    pub observed_at_ms: i64,
    pub counterparty: Counterparty,
    pub asset: AssetId,
    pub provider_balance_held: Option<i64>,
    pub unrealized_claims: i64,
    pub in_flight_transfers: i64,
}

impl CounterpartySnapshot {
    fn validate(&self) -> Result<()> {
        if self.observed_at_ms < 0 {
            bail!("counterparty snapshot timestamp must not be negative");
        }
        self.counterparty.validate()?;
        match (&self.counterparty, self.provider_balance_held) {
            (Counterparty::Venue { .. }, Some(_)) => {
                bail!("a venue snapshot must use its ledger balance")
            }
            (Counterparty::Provider { .. }, None) => {
                bail!("a provider snapshot must supply its held balance")
            }
            _ => {}
        }
        if self.in_flight_transfers < 0 {
            bail!("in-flight counterparty transfers must not be negative");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CounterpartyExposureDivergence {
    pub balance_divergence: i64,
    pub threshold: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CounterpartyExposure {
    pub observed_at_ms: i64,
    pub counterparty: Counterparty,
    pub asset: AssetId,
    pub balance_held: i64,
    pub unrealized_claims: i64,
    pub in_flight_transfers: i64,
    pub counterparty_exposure: i64,
    pub balance_divergence: i64,
    pub mandate_venue_balance_cap: Option<i64>,
    pub mandate_cap_headroom: Option<i64>,
    pub divergence_threshold: i64,
    pub divergence_event: Option<CounterpartyExposureDivergence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReconciliationOutcome {
    Matched {
        venue: String,
        asset: AssetId,
        balance: i64,
    },
    Mismatch {
        alert: LedgerEntry,
    },
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
             PRAGMA synchronous = FULL;",
        )?;
        migrate_legacy_sats_postings(&connection)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS trading_ledger_entries (
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
                 amount INTEGER NOT NULL CHECK (amount != 0),
                 asset TEXT NOT NULL CHECK (length(asset) > 0),
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
             END;
             CREATE TABLE IF NOT EXISTS counterparty_exposure_observations (
                 sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
                 observation_id TEXT NOT NULL UNIQUE,
                 observed_at_ms INTEGER NOT NULL CHECK (observed_at_ms >= 0),
                 counterparty_json TEXT NOT NULL,
                 asset TEXT NOT NULL CHECK (length(asset) > 0),
                 exposure_json TEXT NOT NULL
             ) STRICT;
             CREATE TRIGGER IF NOT EXISTS counterparty_exposure_observations_no_update
             BEFORE UPDATE ON counterparty_exposure_observations BEGIN
                 SELECT RAISE(ABORT, 'counterparty exposure observations are append-only');
             END;
             CREATE TRIGGER IF NOT EXISTS counterparty_exposure_observations_no_delete
             BEFORE DELETE ON counterparty_exposure_observations BEGIN
                 SELECT RAISE(ABORT, 'counterparty exposure observations are append-only');
             END;",
        )?;
        connection.pragma_update(None, "user_version", LEDGER_SCHEMA_VERSION)?;
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
        verify_entries(&load_entries(&self.connection.lock())?)?;
        self.latest_counterparty_exposures().map(|_| ())
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
        self.venue_asset_balance(venue, &AssetId::sats())
    }

    pub fn venue_asset_balance(&self, venue: &str, asset: &AssetId) -> Result<i64> {
        validate_identifier("venue", venue)?;
        let entries = self.entries(&LedgerQuery::default())?;
        venue_balance_from_entries(&entries, venue, asset)
    }

    pub fn reconcile(
        &self,
        event_id: impl Into<String>,
        occurred_at_ms: i64,
        strategy_id: impl Into<String>,
        venue: impl Into<String>,
        observed_sats: i64,
    ) -> Result<ReconciliationOutcome> {
        self.reconcile_asset(
            event_id,
            occurred_at_ms,
            strategy_id,
            venue,
            AssetId::sats(),
            observed_sats,
        )
    }

    pub fn reconcile_asset(
        &self,
        event_id: impl Into<String>,
        occurred_at_ms: i64,
        strategy_id: impl Into<String>,
        venue: impl Into<String>,
        asset: AssetId,
        observed: i64,
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
        if observed < 0 {
            bail!("observed venue balance must not be negative");
        }
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let entries = load_entries(&transaction)?;
        verify_entries(&entries)?;
        let expected = venue_balance_from_entries(&entries, &venue, &asset)?;
        let difference = observed
            .checked_sub(expected)
            .context("reconciliation difference overflowed")?;
        if difference == 0 {
            transaction.commit()?;
            return Ok(ReconciliationOutcome::Matched {
                venue,
                asset,
                balance: observed,
            });
        }
        let draft = LedgerEntryDraft::new(
            event_id,
            occurred_at_ms,
            strategy_id,
            LedgerEntryKind::ReconciliationMismatch(ReconciliationMismatch {
                venue,
                asset,
                expected,
                observed,
                difference,
            }),
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
            let deltas = entry_profit_deltas(&entry)?;
            strategies
                .entry(entry.strategy_id.clone())
                .or_default()
                .record(&entry, &deltas)?;
            total.record(&entry, &deltas)?;
        }
        let total_assets = total.asset_profits();
        let sats_totals = sats_profit(&total_assets);
        Ok(ProfitReport {
            from_ms: query.from_ms,
            to_ms: query.to_ms,
            strategies: strategies
                .into_iter()
                .map(|(strategy_id, accumulator)| accumulator.finish(strategy_id))
                .collect(),
            total_profit_sats: sats_totals.profit,
            total_fees_paid_sats: sats_totals.fees_paid,
            total_funding_collected_sats: sats_totals.funding_collected,
            worst_drawdown_sats: sats_totals.worst_drawdown,
            assets: total_assets,
            counterparty_exposures: self.latest_counterparty_exposures()?,
        })
    }

    pub fn record_counterparty_exposure(
        &self,
        snapshot: CounterpartySnapshot,
        mandate_venue_balance_cap: Option<i64>,
    ) -> Result<(CounterpartyExposure, bool)> {
        snapshot.validate()?;
        if mandate_venue_balance_cap.is_some_and(|cap| cap < 0) {
            bail!("mandate venue-balance cap must not be negative");
        }
        let balance_held = match &snapshot.counterparty {
            Counterparty::Venue { venue } => self.venue_asset_balance(venue, &snapshot.asset)?,
            Counterparty::Provider { .. } => snapshot
                .provider_balance_held
                .context("provider counterparty snapshot lost its held balance")?,
        };
        let counterparty_exposure = balance_held
            .checked_add(snapshot.unrealized_claims)
            .and_then(|value| value.checked_add(snapshot.in_flight_transfers))
            .context("counterparty exposure overflowed")?;
        let balance_divergence = counterparty_exposure
            .checked_sub(balance_held)
            .context("counterparty balance divergence overflowed")?;
        let divergence_threshold =
            counterparty_divergence_threshold(balance_held, mandate_venue_balance_cap)?;
        let divergence_event = (balance_divergence.unsigned_abs()
            > divergence_threshold.unsigned_abs())
        .then_some(CounterpartyExposureDivergence {
            balance_divergence,
            threshold: divergence_threshold,
        });
        let mandate_cap_headroom = mandate_venue_balance_cap
            .map(|cap| {
                cap.checked_sub(counterparty_exposure)
                    .context("mandate-cap headroom overflowed")
            })
            .transpose()?;
        let exposure = CounterpartyExposure {
            observed_at_ms: snapshot.observed_at_ms,
            counterparty: snapshot.counterparty,
            asset: snapshot.asset,
            balance_held,
            unrealized_claims: snapshot.unrealized_claims,
            in_flight_transfers: snapshot.in_flight_transfers,
            counterparty_exposure,
            balance_divergence,
            mandate_venue_balance_cap,
            mandate_cap_headroom,
            divergence_threshold,
            divergence_event,
        };
        let exposure_json = serde_json::to_string(&exposure)?;
        let observation_id = content_digest(exposure_json.as_bytes());
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let exists = transaction
            .prepare("SELECT 1 FROM counterparty_exposure_observations WHERE observation_id = ?1")?
            .exists(params![&observation_id])?;
        if exists {
            transaction.commit()?;
            return Ok((exposure, false));
        }
        let next_sequence: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM counterparty_exposure_observations",
            [],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO counterparty_exposure_observations (
                 sequence, observation_id, observed_at_ms, counterparty_json, asset, exposure_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                next_sequence,
                observation_id,
                exposure.observed_at_ms,
                serde_json::to_string(&exposure.counterparty)?,
                exposure.asset.as_str(),
                exposure_json,
            ],
        )?;
        transaction.commit()?;
        Ok((exposure, true))
    }

    pub fn latest_counterparty_exposures(&self) -> Result<Vec<CounterpartyExposure>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT sequence, observation_id, counterparty_json, asset, exposure_json
             FROM counterparty_exposure_observations ORDER BY sequence",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut expected_sequence = 1_i64;
        let mut latest = BTreeMap::<(Counterparty, AssetId), CounterpartyExposure>::new();
        for row in rows {
            let (sequence, observation_id, counterparty_json, asset, exposure_json) = row?;
            if sequence != expected_sequence {
                bail!(
                    "counterparty exposure sequence gap: expected {expected_sequence}, found {sequence}"
                );
            }
            if observation_id != content_digest(exposure_json.as_bytes()) {
                bail!("counterparty exposure observation {sequence} failed its content digest");
            }
            let exposure: CounterpartyExposure = serde_json::from_str(&exposure_json)?;
            exposure.counterparty.validate()?;
            if counterparty_json != serde_json::to_string(&exposure.counterparty)?
                || asset != exposure.asset.as_str()
            {
                bail!("counterparty exposure observation {sequence} has mismatched key columns");
            }
            latest.insert(
                (exposure.counterparty.clone(), exposure.asset.clone()),
                exposure,
            );
            expected_sequence = expected_sequence
                .checked_add(1)
                .context("counterparty exposure sequence overflowed")?;
        }
        Ok(latest.into_values().collect())
    }
}

fn counterparty_divergence_threshold(
    balance_held: i64,
    mandate_venue_balance_cap: Option<i64>,
) -> Result<i64> {
    let basis = balance_held.unsigned_abs().max(
        mandate_venue_balance_cap
            .map(|cap| cap.unsigned_abs())
            .unwrap_or_default(),
    );
    let threshold = u128::from(basis)
        .saturating_mul(u128::from(COUNTERPARTY_EXPOSURE_DIVERGENCE_THRESHOLD_BPS))
        .div_ceil(10_000);
    i64::try_from(threshold).context("counterparty divergence threshold overflowed")
}

fn content_digest(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

// Version 1 stored postings in an `amount_sats` column with no asset. The
// rename plus constant default is the whole data migration: every existing row
// was a sats row by construction, and `ALTER TABLE` does not fire the
// append-only triggers.
fn migrate_legacy_sats_postings(connection: &Connection) -> Result<()> {
    let has_legacy_amount_column = connection
        .prepare(
            "SELECT 1 FROM pragma_table_info('trading_ledger_postings') WHERE name = 'amount_sats'",
        )?
        .exists([])?;
    if has_legacy_amount_column {
        connection.execute_batch(
            "ALTER TABLE trading_ledger_postings RENAME COLUMN amount_sats TO amount;
             ALTER TABLE trading_ledger_postings ADD COLUMN asset TEXT NOT NULL DEFAULT 'sats';",
        )?;
    }
    Ok(())
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
                 sequence, posting_index, account_json, amount, asset
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                sequence_i64,
                i64::try_from(posting_index).context("ledger posting index overflowed")?,
                serde_json::to_string(&posting.account)?,
                posting.amount,
                posting.asset.as_str(),
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

fn venue_balance_from_entries(
    entries: &[LedgerEntry],
    venue: &str,
    asset: &AssetId,
) -> Result<i64> {
    entries
        .iter()
        .flat_map(|entry| &entry.postings)
        .filter_map(|posting| match &posting.account {
            LedgerAccount::VenueBalance {
                venue: posting_venue,
            } if posting_venue == venue && &posting.asset == asset => Some(posting.amount),
            _ => None,
        })
        .try_fold(0_i64, |balance, amount| {
            balance
                .checked_add(amount)
                .context("venue balance overflowed")
        })
}

#[derive(Default)]
struct AssetAccumulator {
    profit: i64,
    fees_paid: i64,
    funding_collected: i64,
    peak_profit: i64,
    worst_drawdown: i64,
}

#[derive(Default)]
struct ProfitAccumulator {
    assets: BTreeMap<AssetId, AssetAccumulator>,
    entry_count: usize,
}

impl ProfitAccumulator {
    fn record(&mut self, entry: &LedgerEntry, deltas: &BTreeMap<AssetId, i64>) -> Result<()> {
        for (asset, delta) in deltas {
            let accumulator = self.assets.entry(asset.clone()).or_default();
            accumulator.profit = accumulator
                .profit
                .checked_add(*delta)
                .context("ledger profit overflowed")?;
        }
        for posting in &entry.postings {
            let accumulator = self.assets.entry(posting.asset.clone()).or_default();
            match posting.account {
                LedgerAccount::FeeExpense if posting.amount > 0 => {
                    accumulator.fees_paid = accumulator
                        .fees_paid
                        .checked_add(posting.amount)
                        .context("ledger fee total overflowed")?;
                }
                LedgerAccount::FundingIncome => {
                    accumulator.funding_collected = accumulator
                        .funding_collected
                        .checked_sub(posting.amount)
                        .context("ledger funding total overflowed")?;
                }
                _ => {}
            }
        }
        for asset in deltas.keys() {
            let accumulator = self
                .assets
                .get_mut(asset)
                .context("profit accumulator lost an asset")?;
            accumulator.peak_profit = accumulator.peak_profit.max(accumulator.profit);
            accumulator.worst_drawdown = accumulator.worst_drawdown.max(
                accumulator
                    .peak_profit
                    .checked_sub(accumulator.profit)
                    .context("ledger drawdown overflowed")?,
            );
        }
        self.entry_count = self.entry_count.saturating_add(1);
        Ok(())
    }

    fn asset_profits(&self) -> Vec<AssetProfit> {
        self.assets
            .iter()
            .map(|(asset, accumulator)| AssetProfit {
                asset: asset.clone(),
                profit: accumulator.profit,
                fees_paid: accumulator.fees_paid,
                funding_collected: accumulator.funding_collected,
                worst_drawdown: accumulator.worst_drawdown,
            })
            .collect()
    }

    fn finish(self, strategy_id: String) -> StrategyProfit {
        let assets = self.asset_profits();
        let sats = sats_profit(&assets);
        StrategyProfit {
            strategy_id,
            profit_sats: sats.profit,
            fees_paid_sats: sats.fees_paid,
            funding_collected_sats: sats.funding_collected,
            worst_drawdown_sats: sats.worst_drawdown,
            assets,
            entry_count: self.entry_count,
        }
    }
}

fn sats_profit(assets: &[AssetProfit]) -> AssetProfit {
    assets
        .iter()
        .find(|totals| totals.asset.is_sats())
        .cloned()
        .unwrap_or(AssetProfit {
            asset: AssetId::sats(),
            profit: 0,
            fees_paid: 0,
            funding_collected: 0,
            worst_drawdown: 0,
        })
}

fn entry_profit_deltas(entry: &LedgerEntry) -> Result<BTreeMap<AssetId, i64>> {
    let mut deltas = BTreeMap::new();
    for posting in &entry.postings {
        if matches!(
            posting.account,
            LedgerAccount::TradingProfit | LedgerAccount::FeeExpense | LedgerAccount::FundingIncome
        ) {
            let sum = deltas.entry(posting.asset.clone()).or_insert(0_i64);
            *sum = sum
                .checked_add(posting.amount)
                .context("ledger profit postings overflowed")?;
        }
    }
    for sum in deltas.values_mut() {
        *sum = sum
            .checked_neg()
            .context("ledger profit could not be negated")?;
    }
    Ok(deltas)
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
        "SELECT account_json, amount, asset FROM trading_ledger_postings
         WHERE sequence = ?1 ORDER BY posting_index",
    )?;
    let rows = statement.query_map(
        params![i64::try_from(sequence).context("ledger sequence exceeded SQLite range")?],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )?;
    let mut postings = Vec::new();
    for row in rows {
        let (account_json, amount, asset) = row?;
        postings.push(LedgerPosting {
            account: serde_json::from_str(&account_json)
                .context("ledger posting account is not valid JSON")?,
            amount,
            asset: AssetId::new(asset).context("ledger posting names an invalid asset")?,
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
        LedgerPosting::sats(account, amount_sats)
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
            draft("cancel", 1, "alpha", LedgerEntryKind::Cancel, vec![]),
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
    fn double_entry_balance_is_enforced_per_asset() {
        let store = LedgerStore::in_memory().expect("store");
        let balanced_per_asset = draft(
            "two-legs",
            1,
            "alpha",
            LedgerEntryKind::Fill,
            vec![
                posting(venue(), -1_000),
                posting(LedgerAccount::TradingProfit, 1_000),
                LedgerPosting::new(venue(), 50, AssetId::usdc()),
                LedgerPosting::new(LedgerAccount::TradingProfit, -50, AssetId::usdc()),
            ],
        );
        store.append(balanced_per_asset).expect("two-leg trade");

        let cross_asset = draft(
            "cross-asset",
            2,
            "alpha",
            LedgerEntryKind::Fill,
            vec![
                posting(venue(), -1_000),
                LedgerPosting::new(LedgerAccount::TradingProfit, 1_000, AssetId::usdc()),
            ],
        );
        assert!(
            store
                .append(cross_asset)
                .expect_err("cross-asset conversion inside one entry")
                .to_string()
                .contains("do not balance")
        );

        let repeated_account = draft(
            "repeat",
            3,
            "alpha",
            LedgerEntryKind::Fill,
            vec![posting(venue(), 5), posting(venue(), -5)],
        );
        assert!(
            store
                .append(repeated_account)
                .expect_err("repeated (account, asset)")
                .to_string()
                .contains("repeat an account")
        );

        assert_eq!(store.venue_balance("lnmarkets").expect("sats"), -1_000);
        assert_eq!(
            store
                .venue_asset_balance("lnmarkets", &AssetId::usdc())
                .expect("usdc"),
            50
        );
    }

    #[test]
    fn sats_serialization_keeps_the_pre_multi_asset_layout() {
        let sats_posting = posting(venue(), 5);
        assert_eq!(
            serde_json::to_string(&sats_posting).expect("serialize"),
            r#"{"account":{"type":"venue_balance","venue":"lnmarkets"},"amount_sats":5}"#
        );
        let decoded: LedgerPosting =
            serde_json::from_str(r#"{"account":{"type":"trading_profit"},"amount_sats":-5}"#)
                .expect("legacy posting");
        assert_eq!(decoded, posting(LedgerAccount::TradingProfit, -5));

        let usdc_posting = LedgerPosting::new(LedgerAccount::TradingProfit, 7, AssetId::usdc());
        let encoded = serde_json::to_string(&usdc_posting).expect("serialize usdc");
        assert_eq!(
            encoded,
            r#"{"account":{"type":"trading_profit"},"amount":7,"asset":"usdc"}"#
        );
        assert_eq!(
            serde_json::from_str::<LedgerPosting>(&encoded).expect("round trip"),
            usdc_posting
        );
        assert!(
            serde_json::from_str::<LedgerPosting>(
                r#"{"account":{"type":"trading_profit"},"amount_sats":7,"asset":"usdc"}"#
            )
            .is_err()
        );

        let kind = LedgerEntryKind::ReconciliationMismatch(ReconciliationMismatch {
            venue: "lnmarkets".into(),
            asset: AssetId::sats(),
            expected: 100,
            observed: 90,
            difference: -10,
        });
        let encoded = serde_json::to_string(&kind).expect("serialize kind");
        assert_eq!(
            encoded,
            r#"{"type":"reconciliation_mismatch","venue":"lnmarkets","expected_sats":100,"observed_sats":90,"difference_sats":-10}"#
        );
        assert_eq!(
            serde_json::from_str::<LedgerEntryKind>(&encoded).expect("round trip"),
            kind
        );
    }

    #[test]
    fn version_one_sats_databases_migrate_in_place() {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("trading-ledger.db");
        {
            let connection = Connection::open(&path).expect("raw connection");
            connection
                .execute_batch(
                    "CREATE TABLE trading_ledger_entries (
                         sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
                         event_id TEXT NOT NULL UNIQUE,
                         occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms >= 0),
                         strategy_id TEXT NOT NULL,
                         kind_json TEXT NOT NULL,
                         metadata_json TEXT NOT NULL,
                         previous_hash TEXT NOT NULL,
                         entry_hash TEXT NOT NULL UNIQUE
                     ) STRICT;
                     CREATE TABLE trading_ledger_postings (
                         sequence INTEGER NOT NULL,
                         posting_index INTEGER NOT NULL CHECK (posting_index >= 0),
                         account_json TEXT NOT NULL,
                         amount_sats INTEGER NOT NULL CHECK (amount_sats != 0),
                         PRIMARY KEY (sequence, posting_index),
                         FOREIGN KEY (sequence) REFERENCES trading_ledger_entries(sequence)
                     ) STRICT;",
                )
                .expect("legacy schema");
            // Sats drafts hash to the same bytes the version 1 code produced,
            // so this fixture reproduces a real pre-migration chain.
            let legacy = draft(
                "legacy-fee",
                1,
                "alpha",
                LedgerEntryKind::Fee,
                vec![posting(venue(), -5), posting(LedgerAccount::FeeExpense, 5)],
            );
            let entry_hash = hash_entry(1, GENESIS_HASH, &legacy).expect("legacy hash");
            connection
                .execute(
                    "INSERT INTO trading_ledger_entries (
                         sequence, event_id, occurred_at_ms, strategy_id, kind_json,
                         metadata_json, previous_hash, entry_hash
                     ) VALUES (1, 'legacy-fee', 1, 'alpha', ?1, ?2, ?3, ?4)",
                    params![
                        serde_json::to_string(&legacy.kind).expect("kind"),
                        serde_json::to_string(&legacy.metadata).expect("metadata"),
                        GENESIS_HASH,
                        entry_hash,
                    ],
                )
                .expect("legacy entry");
            for (posting_index, legacy_posting) in legacy.postings.iter().enumerate() {
                connection
                    .execute(
                        "INSERT INTO trading_ledger_postings (
                             sequence, posting_index, account_json, amount_sats
                         ) VALUES (1, ?1, ?2, ?3)",
                        params![
                            i64::try_from(posting_index).expect("index"),
                            serde_json::to_string(&legacy_posting.account).expect("account"),
                            legacy_posting.amount,
                        ],
                    )
                    .expect("legacy posting");
            }
        }

        let store = LedgerStore::open(&path).expect("migrated store");
        store.verify().expect("hash chain survives migration");
        let entries = store.entries(&LedgerQuery::default()).expect("entries");
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0]
                .postings
                .iter()
                .all(|entry| entry.asset.is_sats())
        );
        assert_eq!(store.venue_balance("lnmarkets").expect("balance"), -5);

        store
            .append(draft(
                "usdc-fill",
                2,
                "alpha",
                LedgerEntryKind::Fill,
                vec![
                    LedgerPosting::new(venue(), 9, AssetId::usdc()),
                    LedgerPosting::new(LedgerAccount::TradingProfit, -9, AssetId::usdc()),
                ],
            ))
            .expect("append after migration");
        assert_eq!(
            store
                .venue_asset_balance("lnmarkets", &AssetId::usdc())
                .expect("usdc"),
            9
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
            &alert.kind,
            LedgerEntryKind::ReconciliationMismatch(ReconciliationMismatch {
                expected: 100,
                observed: 90,
                difference: -10,
                ..
            })
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
    fn reconciliation_is_keyed_by_venue_and_asset() {
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
                    LedgerPosting::new(venue(), 40, AssetId::usdc()),
                    LedgerPosting::new(LedgerAccount::BalanceAdjustment, -40, AssetId::usdc()),
                ],
            ))
            .expect("opening balances");

        assert!(matches!(
            store
                .reconcile_asset("usdc-1", 2, "system", "lnmarkets", AssetId::usdc(), 40)
                .expect("usdc match"),
            ReconciliationOutcome::Matched { asset, balance: 40, .. } if !asset.is_sats()
        ));
        let ReconciliationOutcome::Mismatch { alert } = store
            .reconcile_asset("usdc-2", 3, "system", "lnmarkets", AssetId::usdc(), 39)
            .expect("usdc mismatch")
        else {
            panic!("expected usdc mismatch");
        };
        assert!(matches!(
            &alert.kind,
            LedgerEntryKind::ReconciliationMismatch(ReconciliationMismatch {
                expected: 40,
                observed: 39,
                difference: -1,
                ..
            })
        ));
        // The sats book is untouched by the usdc mismatch.
        assert!(matches!(
            store
                .reconcile("sats-1", 4, "system", "lnmarkets", 100)
                .expect("sats match"),
            ReconciliationOutcome::Matched { balance: 100, .. }
        ));
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
                assets: vec![AssetProfit {
                    asset: AssetId::sats(),
                    profit: 18,
                    fees_paid: 5,
                    funding_collected: 3,
                    worst_drawdown: 5,
                }],
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
    fn profit_report_totals_each_asset_independently() {
        let store = LedgerStore::in_memory().expect("store");
        for event in [
            draft(
                "sats-profit",
                10,
                "alpha",
                LedgerEntryKind::Fill,
                vec![
                    posting(venue(), 20),
                    posting(LedgerAccount::TradingProfit, -20),
                ],
            ),
            draft(
                "usdc-loss",
                20,
                "alpha",
                LedgerEntryKind::Fill,
                vec![
                    LedgerPosting::new(venue(), -8, AssetId::usdc()),
                    LedgerPosting::new(LedgerAccount::TradingProfit, 8, AssetId::usdc()),
                ],
            ),
            draft(
                "usdc-fee",
                30,
                "alpha",
                LedgerEntryKind::Fee,
                vec![
                    LedgerPosting::new(venue(), -2, AssetId::usdc()),
                    LedgerPosting::new(LedgerAccount::FeeExpense, 2, AssetId::usdc()),
                ],
            ),
        ] {
            store.append(event).expect("append");
        }

        let report = store
            .profit_report(&LedgerQuery::default())
            .expect("report");
        assert_eq!(report.total_profit_sats, 20);
        assert_eq!(report.total_fees_paid_sats, 0);
        assert_eq!(report.worst_drawdown_sats, 0);
        assert_eq!(
            report.assets,
            vec![
                AssetProfit {
                    asset: AssetId::sats(),
                    profit: 20,
                    fees_paid: 0,
                    funding_collected: 0,
                    worst_drawdown: 0,
                },
                AssetProfit {
                    asset: AssetId::usdc(),
                    profit: -10,
                    fees_paid: 2,
                    funding_collected: 0,
                    worst_drawdown: 10,
                },
            ]
        );
        assert_eq!(
            report.asset_totals(&AssetId::usdc()).expect("usdc").profit,
            -10
        );
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

    #[test]
    fn unrealized_claims_swell_derived_counterparty_exposure() {
        let store = LedgerStore::in_memory().expect("store");
        store
            .append(draft(
                "opening",
                1,
                "system",
                LedgerEntryKind::Deposit,
                vec![
                    posting(venue(), 100),
                    posting(LedgerAccount::External, -100),
                ],
            ))
            .expect("opening balance");
        let snapshot = CounterpartySnapshot {
            observed_at_ms: 2,
            counterparty: Counterparty::Venue {
                venue: "lnmarkets".into(),
            },
            asset: AssetId::sats(),
            provider_balance_held: None,
            unrealized_claims: 60,
            in_flight_transfers: 0,
        };

        let (exposure, appended) = store
            .record_counterparty_exposure(snapshot.clone(), Some(1_000))
            .expect("exposure");
        assert!(appended);
        assert_eq!(exposure.balance_held, 100);
        assert_eq!(exposure.counterparty_exposure, 160);
        assert_eq!(exposure.balance_divergence, 60);
        assert_eq!(exposure.divergence_threshold, 50);
        assert!(exposure.divergence_event.is_some());
        assert_eq!(exposure.mandate_cap_headroom, Some(840));

        let (_, replay_appended) = store
            .record_counterparty_exposure(snapshot, Some(1_000))
            .expect("idempotent replay");
        assert!(!replay_appended);
        assert_eq!(
            store
                .profit_report(&LedgerQuery::default())
                .expect("report")
                .counterparty_exposures,
            vec![exposure]
        );
    }

    #[test]
    fn pending_withdrawal_is_counted_as_in_flight_exposure() {
        let store = LedgerStore::in_memory().expect("store");
        for entry in [
            draft(
                "opening",
                1,
                "system",
                LedgerEntryKind::Deposit,
                vec![
                    posting(venue(), 100),
                    posting(LedgerAccount::External, -100),
                ],
            ),
            draft(
                "withdrawal",
                2,
                "system",
                LedgerEntryKind::Withdrawal,
                vec![posting(venue(), -20), posting(LedgerAccount::External, 20)],
            ),
        ] {
            store.append(entry).expect("ledger entry");
        }

        let (exposure, _) = store
            .record_counterparty_exposure(
                CounterpartySnapshot {
                    observed_at_ms: 3,
                    counterparty: Counterparty::Venue {
                        venue: "lnmarkets".into(),
                    },
                    asset: AssetId::sats(),
                    provider_balance_held: None,
                    unrealized_claims: 0,
                    in_flight_transfers: 20,
                },
                Some(100),
            )
            .expect("exposure");
        assert_eq!(exposure.balance_held, 80);
        assert_eq!(exposure.in_flight_transfers, 20);
        assert_eq!(exposure.counterparty_exposure, 100);
        assert_eq!(exposure.divergence_threshold, 5);
        assert!(exposure.divergence_event.is_some());
    }

    #[test]
    fn counterparty_exposure_observations_are_append_only() {
        let store = LedgerStore::in_memory().expect("store");
        store
            .record_counterparty_exposure(
                CounterpartySnapshot {
                    observed_at_ms: 1,
                    counterparty: Counterparty::Provider {
                        provider: "custodian".into(),
                    },
                    asset: AssetId::usdc(),
                    provider_balance_held: Some(25),
                    unrealized_claims: 5,
                    in_flight_transfers: 1,
                },
                None,
            )
            .expect("provider exposure");
        let connection = store.connection.lock();
        for statement in [
            "UPDATE counterparty_exposure_observations SET observed_at_ms = 2",
            "DELETE FROM counterparty_exposure_observations",
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
    fn asset_identifiers_are_validated() {
        assert!(AssetId::new("sats").is_ok());
        assert!(AssetId::new("usdc").is_ok());
        assert!(AssetId::new("usdt_perp2").is_ok());
        for invalid in ["", "SATS", "1usd", "usd-c", "usd c", &"a".repeat(33)] {
            assert!(AssetId::new(invalid.to_owned()).is_err(), "{invalid:?}");
        }
        assert!(serde_json::from_str::<AssetId>("\"USDC\"").is_err());
    }
}
