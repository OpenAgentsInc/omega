use std::{
    collections::BTreeSet,
    fs,
    future::Future,
    io::{self, Write as _},
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_io::Timer;
use atomic_write_file::AtomicWriteFile;
use futures::{FutureExt as _, StreamExt as _, pin_mut, select, stream::FuturesUnordered};
use omega_identity::{AccountRef, NostrPublicKeyHex};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

const CACHE_SCHEMA: &str = "openagents.omega.identity-hydration-cache.v1";
const GENERATION_SCHEMA: &str = "openagents.omega.identity-hydration-generation.v1";
const CONSENT_SCHEMA: &str = "openagents.omega.bulk-decrypt-consent.v1";
const MAX_CACHE_DOCUMENT_BYTES: usize = 1024 * 1024;
const MAX_CACHE_KEY_BYTES: usize = 128;
const MAX_PLAN_SOURCES: usize = 16;
const MAX_SOURCE_ITEM_LIMIT: u32 = 10_000;
const MAX_SOURCE_DEADLINE_MILLISECONDS: u64 = 120_000;
const MAX_OVERALL_DEADLINE_MILLISECONDS: u64 = 300_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HydrationAccountFence {
    pub account_ref: AccountRef,
    pub public_key_hex: NostrPublicKeyHex,
    pub generation: u64,
}

impl HydrationAccountFence {
    pub fn new(
        account_ref: AccountRef,
        public_key_hex: NostrPublicKeyHex,
        generation: u64,
    ) -> Result<Self, HydrationError> {
        if generation == 0 {
            return Err(HydrationError::InvalidFence);
        }
        Ok(Self {
            account_ref,
            public_key_hex,
            generation,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HydrationSource {
    Profile,
    RelayPreferences,
    Nip29GroupList,
    MembershipMetadata,
    RecentRooms,
    HostedAccount,
    HostedDevice,
    BuzzProfile,
    ArmadaProfile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HydrationTrigger {
    Startup,
    Imported,
    Recovered,
    Switched,
    RemoteSigner,
    BackgroundContinuation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HydrationSourcePlan {
    pub source: HydrationSource,
    pub deadline_milliseconds: u64,
    pub item_limit: u32,
}

impl HydrationSourcePlan {
    pub fn new(
        source: HydrationSource,
        deadline_milliseconds: u64,
    ) -> Result<Self, HydrationError> {
        if deadline_milliseconds == 0 || deadline_milliseconds > MAX_SOURCE_DEADLINE_MILLISECONDS {
            return Err(HydrationError::InvalidDeadline);
        }
        Ok(Self {
            source,
            deadline_milliseconds,
            item_limit: default_item_limit(source),
        })
    }

    pub fn with_item_limit(mut self, item_limit: u32) -> Result<Self, HydrationError> {
        if item_limit == 0 || item_limit > MAX_SOURCE_ITEM_LIMIT {
            return Err(HydrationError::InvalidLimit);
        }
        self.item_limit = item_limit;
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HydrationPlan {
    pub fence: HydrationAccountFence,
    pub trigger: HydrationTrigger,
    pub overall_deadline_milliseconds: u64,
    pub fresh_unpublished_candidate: bool,
    pub sources: Vec<HydrationSourcePlan>,
}

impl HydrationPlan {
    pub fn new(
        fence: HydrationAccountFence,
        trigger: HydrationTrigger,
        overall_deadline_milliseconds: u64,
        fresh_unpublished_candidate: bool,
        sources: impl IntoIterator<Item = HydrationSourcePlan>,
    ) -> Result<Self, HydrationError> {
        if overall_deadline_milliseconds == 0
            || overall_deadline_milliseconds > MAX_OVERALL_DEADLINE_MILLISECONDS
        {
            return Err(HydrationError::InvalidDeadline);
        }
        let sources = if fresh_unpublished_candidate {
            Vec::new()
        } else {
            sources.into_iter().collect()
        };
        let plan = Self {
            fence,
            trigger,
            overall_deadline_milliseconds,
            fresh_unpublished_candidate,
            sources,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), HydrationError> {
        if self.fence.generation == 0
            || self.overall_deadline_milliseconds == 0
            || self.overall_deadline_milliseconds > MAX_OVERALL_DEADLINE_MILLISECONDS
            || self.sources.len() > MAX_PLAN_SOURCES
        {
            return Err(HydrationError::InvalidPlan);
        }
        if self.fresh_unpublished_candidate && !self.sources.is_empty() {
            return Err(HydrationError::InvalidPlan);
        }
        let mut unique = BTreeSet::new();
        for source in &self.sources {
            if source.deadline_milliseconds == 0
                || source.deadline_milliseconds > MAX_SOURCE_DEADLINE_MILLISECONDS
                || source.item_limit == 0
                || source.item_limit > MAX_SOURCE_ITEM_LIMIT
                || !unique.insert(source.source)
            {
                return Err(HydrationError::InvalidPlan);
            }
        }
        Ok(())
    }

    pub fn continuation(
        &self,
        receipt: &HydrationReceipt,
        overall_deadline_milliseconds: u64,
    ) -> Result<Option<Self>, HydrationError> {
        if receipt.fence != self.fence {
            return Err(HydrationError::StaleGeneration);
        }
        let retryable = receipt
            .sources
            .iter()
            .filter(|source| source.outcome.is_retryable())
            .map(|source| source.source)
            .collect::<BTreeSet<_>>();
        let sources = self
            .sources
            .iter()
            .filter(|source| retryable.contains(&source.source))
            .cloned()
            .collect::<Vec<_>>();
        if sources.is_empty() {
            return Ok(None);
        }
        Self::new(
            self.fence.clone(),
            HydrationTrigger::BackgroundContinuation,
            overall_deadline_milliseconds,
            false,
            sources,
        )
        .map(Some)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheFallbackReason {
    Offline,
    Timeout,
    Failure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutScope {
    Source,
    Overall,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum HydrationSourceOutcome {
    Complete {
        items: u32,
    },
    Cached {
        items: u32,
        reason: CacheFallbackReason,
    },
    Stale {
        cached_items: u32,
    },
    Locked {
        ciphertext_items: u32,
    },
    Disabled,
    Offline,
    TimedOut {
        scope: TimeoutScope,
    },
    Failed,
}

impl HydrationSourceOutcome {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Cached { .. }
                | Self::Stale { .. }
                | Self::Offline
                | Self::TimedOut { .. }
                | Self::Failed
        )
    }

    fn exceeds_item_limit(&self, item_limit: u32) -> bool {
        match self {
            Self::Complete { items } | Self::Cached { items, .. } => *items > item_limit,
            Self::Stale { cached_items } => *cached_items > item_limit,
            Self::Locked { ciphertext_items } => *ciphertext_items > item_limit,
            Self::Disabled | Self::Offline | Self::TimedOut { .. } | Self::Failed => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HydrationSourceReceipt {
    pub source: HydrationSource,
    pub outcome: HydrationSourceOutcome,
    pub elapsed_milliseconds: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HydrationState {
    Complete,
    Partial,
    Offline,
    Failed,
    SkippedFresh,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HydrationReceipt {
    pub fence: HydrationAccountFence,
    pub trigger: HydrationTrigger,
    pub state: HydrationState,
    pub started_at: u64,
    pub elapsed_milliseconds: u64,
    pub background_continuation_available: bool,
    pub sources: Vec<HydrationSourceReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HydrationSourceRequest {
    pub fence: HydrationAccountFence,
    pub trigger: HydrationTrigger,
    pub source: HydrationSource,
    pub item_limit: u32,
}

pub trait HydrationSourceRunner: Send + Sync {
    fn hydrate(
        &self,
        request: HydrationSourceRequest,
    ) -> Pin<Box<dyn Future<Output = HydrationSourceOutcome> + Send + 'static>>;
}

#[derive(Clone)]
pub struct HydrationScheduler {
    runner: Arc<dyn HydrationSourceRunner>,
}

impl HydrationScheduler {
    pub fn new(runner: Arc<dyn HydrationSourceRunner>) -> Self {
        Self { runner }
    }

    pub async fn run(&self, plan: HydrationPlan) -> Result<HydrationReceipt, HydrationError> {
        plan.validate()?;
        let started_at = unix_time();
        if plan.fresh_unpublished_candidate {
            return Ok(HydrationReceipt {
                fence: plan.fence,
                trigger: plan.trigger,
                state: HydrationState::SkippedFresh,
                started_at,
                elapsed_milliseconds: 0,
                background_continuation_available: false,
                sources: Vec::new(),
            });
        }
        if plan.sources.is_empty() {
            return Ok(HydrationReceipt {
                fence: plan.fence,
                trigger: plan.trigger,
                state: HydrationState::Failed,
                started_at,
                elapsed_milliseconds: 0,
                background_continuation_available: false,
                sources: Vec::new(),
            });
        }

        let started = Instant::now();
        let overall_deadline = started + Duration::from_millis(plan.overall_deadline_milliseconds);
        let mut pending = FuturesUnordered::new();
        for source_plan in plan.sources.iter().cloned() {
            let request = HydrationSourceRequest {
                fence: plan.fence.clone(),
                trigger: plan.trigger,
                source: source_plan.source,
                item_limit: source_plan.item_limit,
            };
            let runner = self.runner.clone();
            pending.push(run_source(runner, request, source_plan, overall_deadline));
        }

        let mut sources = Vec::with_capacity(plan.sources.len());
        while let Some(receipt) = pending.next().await {
            sources.push(receipt);
        }
        sources.sort_by_key(|source| source.source);
        let state = aggregate_state(&sources);
        Ok(HydrationReceipt {
            fence: plan.fence,
            trigger: plan.trigger,
            state,
            started_at,
            elapsed_milliseconds: duration_milliseconds(started.elapsed()),
            background_continuation_available: sources
                .iter()
                .any(|source| source.outcome.is_retryable()),
            sources,
        })
    }
}

async fn run_source(
    runner: Arc<dyn HydrationSourceRunner>,
    request: HydrationSourceRequest,
    source_plan: HydrationSourcePlan,
    overall_deadline: Instant,
) -> HydrationSourceReceipt {
    let started = Instant::now();
    let source_deadline = started + Duration::from_millis(source_plan.deadline_milliseconds);
    let deadline = source_deadline.min(overall_deadline);
    let timeout_scope = if source_deadline <= overall_deadline {
        TimeoutScope::Source
    } else {
        TimeoutScope::Overall
    };
    let hydrate = runner.hydrate(request).fuse();
    let timeout = async move {
        Timer::at(deadline).await;
    }
    .fuse();
    pin_mut!(hydrate, timeout);
    let outcome = select! {
        outcome = hydrate => outcome,
        _ = timeout => HydrationSourceOutcome::TimedOut { scope: timeout_scope },
    };
    let outcome = if outcome.exceeds_item_limit(source_plan.item_limit) {
        HydrationSourceOutcome::Failed
    } else {
        outcome
    };
    HydrationSourceReceipt {
        source: source_plan.source,
        outcome,
        elapsed_milliseconds: duration_milliseconds(started.elapsed()),
    }
}

fn aggregate_state(sources: &[HydrationSourceReceipt]) -> HydrationState {
    if sources.is_empty() {
        return HydrationState::Failed;
    }
    if sources.iter().all(|source| {
        matches!(
            source.outcome,
            HydrationSourceOutcome::Complete { .. } | HydrationSourceOutcome::Disabled
        )
    }) {
        return HydrationState::Complete;
    }
    let any_offline = sources.iter().any(|source| {
        matches!(
            source.outcome,
            HydrationSourceOutcome::Offline
                | HydrationSourceOutcome::Cached {
                    reason: CacheFallbackReason::Offline,
                    ..
                }
        )
    });
    if any_offline
        && sources.iter().all(|source| {
            matches!(
                source.outcome,
                HydrationSourceOutcome::Offline
                    | HydrationSourceOutcome::Cached {
                        reason: CacheFallbackReason::Offline,
                        ..
                    }
                    | HydrationSourceOutcome::Disabled
            )
        })
    {
        return HydrationState::Offline;
    }
    let any_failed = sources.iter().any(|source| {
        matches!(
            source.outcome,
            HydrationSourceOutcome::Failed | HydrationSourceOutcome::Stale { .. }
        )
    });
    if any_failed
        && sources.iter().all(|source| {
            matches!(
                source.outcome,
                HydrationSourceOutcome::Failed
                    | HydrationSourceOutcome::Stale { .. }
                    | HydrationSourceOutcome::Disabled
            )
        })
    {
        return HydrationState::Failed;
    }
    HydrationState::Partial
}

fn default_item_limit(source: HydrationSource) -> u32 {
    match source {
        HydrationSource::Profile
        | HydrationSource::HostedAccount
        | HydrationSource::HostedDevice
        | HydrationSource::BuzzProfile
        | HydrationSource::ArmadaProfile => 1,
        HydrationSource::RelayPreferences => 128,
        HydrationSource::Nip29GroupList => 256,
        HydrationSource::MembershipMetadata => 512,
        HydrationSource::RecentRooms => 200,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HydrationCacheArea {
    Ciphertext,
    Plaintext,
    Profiles,
    Relays,
    Groups,
    Receipts,
    Consent,
}

impl HydrationCacheArea {
    fn directory(self) -> &'static str {
        match self {
            Self::Ciphertext => "ciphertext",
            Self::Plaintext => "plaintext",
            Self::Profiles => "profiles",
            Self::Relays => "relays",
            Self::Groups => "groups",
            Self::Receipts => "receipts",
            Self::Consent => "consent",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaintextPersistencePolicy {
    #[default]
    Never,
    NonPrivateNonExpiring,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaintextClassification {
    pub private: bool,
    pub expires_at: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BulkDecryptConsentState {
    #[default]
    Unknown,
    Allowed,
    Declined,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkDecryptConsent {
    pub schema: String,
    pub fence: HydrationAccountFence,
    pub signer_capability_ref: String,
    pub state: BulkDecryptConsentState,
    pub updated_at: u64,
}

impl BulkDecryptConsent {
    pub fn new(
        fence: HydrationAccountFence,
        signer_capability_ref: impl Into<String>,
        state: BulkDecryptConsentState,
        updated_at: u64,
    ) -> Result<Self, HydrationCacheError> {
        let consent = Self {
            schema: CONSENT_SCHEMA.to_string(),
            fence,
            signer_capability_ref: signer_capability_ref.into(),
            state,
            updated_at,
        };
        consent.validate()?;
        Ok(consent)
    }

    fn validate(&self) -> Result<(), HydrationCacheError> {
        if self.schema != CONSENT_SCHEMA
            || !valid_cache_key(&self.signer_capability_ref)
            || self.updated_at == 0
        {
            return Err(HydrationCacheError::InvalidDocument);
        }
        Ok(())
    }

    pub fn permits_bulk_decrypt(&self) -> bool {
        self.state == BulkDecryptConsentState::Allowed
    }

    pub fn suppresses_prompt(&self) -> bool {
        self.state != BulkDecryptConsentState::Unknown
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheEnvelope {
    schema: String,
    fence: HydrationAccountFence,
    area: HydrationCacheArea,
    value: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationMarker {
    schema: String,
    fence: HydrationAccountFence,
}

#[derive(Clone, Debug)]
pub struct HydrationCache {
    root: PathBuf,
    fence: HydrationAccountFence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HydrationAccountState {
    pub latest_receipt: Option<HydrationReceipt>,
    pub bulk_decrypt_consent: BulkDecryptConsentState,
    pub plaintext_policy: PlaintextPersistencePolicy,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalProfileState {
    Skipped { updated_at: u64 },
    SavedLocally { profile: Value, updated_at: u64 },
}

impl HydrationCache {
    pub fn system(fence: HydrationAccountFence) -> Result<Self, HydrationCacheError> {
        Self::for_data_root(paths::data_dir().to_path_buf(), fence)
    }

    pub fn for_data_root(
        data_root: impl Into<PathBuf>,
        fence: HydrationAccountFence,
    ) -> Result<Self, HydrationCacheError> {
        Self::open(data_root.into().join("identity").join("hydration"), fence)
    }

    pub fn open(
        root: impl Into<PathBuf>,
        fence: HydrationAccountFence,
    ) -> Result<Self, HydrationCacheError> {
        let cache = Self {
            root: root.into(),
            fence,
        };
        cache.prepare_partition()?;
        Ok(cache)
    }

    pub fn fence(&self) -> &HydrationAccountFence {
        &self.fence
    }

    pub fn account_root(&self) -> PathBuf {
        self.root
            .join("accounts")
            .join(self.fence.public_key_hex.as_str())
    }

    pub fn write(
        &self,
        area: HydrationCacheArea,
        key: &str,
        value: Value,
    ) -> Result<(), HydrationCacheError> {
        validate_cache_key(key)?;
        self.ensure_current_generation()?;
        let envelope = CacheEnvelope {
            schema: CACHE_SCHEMA.to_string(),
            fence: self.fence.clone(),
            area,
            value,
        };
        let bytes = serde_json::to_vec(&envelope)?;
        if bytes.len() > MAX_CACHE_DOCUMENT_BYTES {
            return Err(HydrationCacheError::DocumentTooLarge);
        }
        let directory = self.area_directory(area);
        prepare_directory(&directory)?;
        let path = directory.join(format!("{key}.json"));
        reject_symlink(&path)?;
        atomic_owner_only_write(&path, &bytes)?;
        if let Err(error) = self.ensure_current_generation() {
            remove_if_owned_by_fence(&path, &self.fence)?;
            return Err(error);
        }
        Ok(())
    }

    pub fn read(
        &self,
        area: HydrationCacheArea,
        key: &str,
    ) -> Result<Option<Value>, HydrationCacheError> {
        validate_cache_key(key)?;
        self.ensure_current_generation()?;
        let path = self.area_directory(area).join(format!("{key}.json"));
        let Some(bytes) = read_bounded(&path)? else {
            return Ok(None);
        };
        let envelope: CacheEnvelope = serde_json::from_slice(&bytes)?;
        if envelope.schema != CACHE_SCHEMA || envelope.fence != self.fence || envelope.area != area
        {
            return Err(HydrationCacheError::StaleGeneration);
        }
        Ok(Some(envelope.value))
    }

    pub fn write_plaintext(
        &self,
        key: &str,
        value: Value,
        policy: PlaintextPersistencePolicy,
        classification: PlaintextClassification,
    ) -> Result<bool, HydrationCacheError> {
        if policy == PlaintextPersistencePolicy::Never
            || classification.private
            || classification.expires_at.is_some()
        {
            return Ok(false);
        }
        self.write(HydrationCacheArea::Plaintext, key, value)?;
        Ok(true)
    }

    pub fn write_hydration_receipt(
        &self,
        receipt: &HydrationReceipt,
    ) -> Result<(), HydrationCacheError> {
        if receipt.fence != self.fence {
            return Err(HydrationCacheError::StaleGeneration);
        }
        self.write(
            HydrationCacheArea::Receipts,
            "latest",
            serde_json::to_value(receipt)?,
        )
    }

    pub fn read_hydration_receipt(&self) -> Result<Option<HydrationReceipt>, HydrationCacheError> {
        let Some(value) = self.read(HydrationCacheArea::Receipts, "latest")? else {
            return Ok(None);
        };
        let receipt: HydrationReceipt = serde_json::from_value(value)?;
        if receipt.fence != self.fence {
            return Err(HydrationCacheError::StaleGeneration);
        }
        Ok(Some(receipt))
    }

    pub fn record_profile_skipped(&self) -> Result<(), HydrationCacheError> {
        self.write(
            HydrationCacheArea::Profiles,
            "local-profile",
            serde_json::to_value(LocalProfileState::Skipped {
                updated_at: unix_time().max(1),
            })?,
        )
    }

    pub fn save_local_profile(&self, profile: Value) -> Result<(), HydrationCacheError> {
        self.write(
            HydrationCacheArea::Profiles,
            "local-profile",
            serde_json::to_value(LocalProfileState::SavedLocally {
                profile,
                updated_at: unix_time().max(1),
            })?,
        )
    }

    pub fn read_local_profile_state(
        &self,
    ) -> Result<Option<LocalProfileState>, HydrationCacheError> {
        self.read(HydrationCacheArea::Profiles, "local-profile")?
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    pub fn write_plaintext_persistence_policy(
        &self,
        policy: PlaintextPersistencePolicy,
    ) -> Result<(), HydrationCacheError> {
        self.write(
            HydrationCacheArea::Consent,
            "plaintext-policy",
            serde_json::to_value(policy)?,
        )
    }

    pub fn read_plaintext_persistence_policy(
        &self,
    ) -> Result<PlaintextPersistencePolicy, HydrationCacheError> {
        self.read(HydrationCacheArea::Consent, "plaintext-policy")?
            .map(serde_json::from_value)
            .transpose()
            .map(|policy| policy.unwrap_or_default())
            .map_err(Into::into)
    }

    pub fn set_bulk_decrypt_consent(
        &self,
        signer_capability_ref: impl Into<String>,
        state: BulkDecryptConsentState,
    ) -> Result<(), HydrationCacheError> {
        let consent = BulkDecryptConsent::new(
            self.fence.clone(),
            signer_capability_ref,
            state,
            unix_time().max(1),
        )?;
        self.write_bulk_decrypt_consent(&consent)
    }

    pub fn write_bulk_decrypt_consent(
        &self,
        consent: &BulkDecryptConsent,
    ) -> Result<(), HydrationCacheError> {
        consent.validate()?;
        if consent.fence != self.fence {
            return Err(HydrationCacheError::StaleGeneration);
        }
        self.write(
            HydrationCacheArea::Consent,
            &consent.signer_capability_ref,
            serde_json::to_value(consent)?,
        )
    }

    pub fn read_bulk_decrypt_consent(
        &self,
        signer_capability_ref: &str,
    ) -> Result<Option<BulkDecryptConsent>, HydrationCacheError> {
        validate_cache_key(signer_capability_ref)?;
        let Some(value) = self.read(HydrationCacheArea::Consent, signer_capability_ref)? else {
            return Ok(None);
        };
        let consent: BulkDecryptConsent = serde_json::from_value(value)?;
        consent.validate()?;
        if consent.fence != self.fence || consent.signer_capability_ref != signer_capability_ref {
            return Ok(None);
        }
        Ok(Some(consent))
    }

    pub fn inspect_account_state(
        &self,
        signer_capability_ref: &str,
    ) -> Result<HydrationAccountState, HydrationCacheError> {
        let latest_receipt = self.read_hydration_receipt()?;
        let bulk_decrypt_consent = self
            .read_bulk_decrypt_consent(signer_capability_ref)?
            .map_or(BulkDecryptConsentState::Unknown, |consent| consent.state);
        let plaintext_policy = self.read_plaintext_persistence_policy()?;
        Ok(HydrationAccountState {
            latest_receipt,
            bulk_decrypt_consent,
            plaintext_policy,
        })
    }

    pub fn purge_area(&self, area: HydrationCacheArea) -> Result<(), HydrationCacheError> {
        self.ensure_current_generation()?;
        let directory = self.area_directory(area);
        remove_scoped_directory(&directory, &self.account_root())?;
        if directory.try_exists()? {
            return Err(HydrationCacheError::PurgeVerification);
        }
        Ok(())
    }

    pub fn purge_account(self) -> Result<(), HydrationCacheError> {
        self.ensure_current_generation()?;
        let account_root = self.account_root();
        remove_scoped_directory(&account_root, &self.root.join("accounts"))?;
        if account_root.try_exists()? {
            return Err(HydrationCacheError::PurgeVerification);
        }
        Ok(())
    }

    fn prepare_partition(&self) -> Result<(), HydrationCacheError> {
        prepare_directory(&self.root)?;
        let accounts = self.root.join("accounts");
        prepare_directory(&accounts)?;
        let account_root = self.account_root();
        prepare_directory(&account_root)?;
        let marker_path = account_root.join("generation.json");
        let existing = read_bounded(&marker_path)?
            .map(|bytes| serde_json::from_slice::<GenerationMarker>(&bytes))
            .transpose()?;
        if let Some(existing) = existing {
            if existing.schema != GENERATION_SCHEMA
                || existing.fence.account_ref != self.fence.account_ref
                || existing.fence.public_key_hex != self.fence.public_key_hex
            {
                return Err(HydrationCacheError::InvalidDocument);
            }
            if existing.fence.generation > self.fence.generation {
                return Err(HydrationCacheError::StaleGeneration);
            }
            if existing.fence.generation == self.fence.generation {
                return Ok(());
            }
        }
        let marker = GenerationMarker {
            schema: GENERATION_SCHEMA.to_string(),
            fence: self.fence.clone(),
        };
        atomic_owner_only_write(&marker_path, &serde_json::to_vec(&marker)?)?;
        self.ensure_current_generation()
    }

    fn ensure_current_generation(&self) -> Result<(), HydrationCacheError> {
        let marker_path = self.account_root().join("generation.json");
        let bytes = read_bounded(&marker_path)?.ok_or(HydrationCacheError::StaleGeneration)?;
        let marker: GenerationMarker = serde_json::from_slice(&bytes)?;
        if marker.schema != GENERATION_SCHEMA || marker.fence != self.fence {
            return Err(HydrationCacheError::StaleGeneration);
        }
        Ok(())
    }

    fn area_directory(&self, area: HydrationCacheArea) -> PathBuf {
        self.account_root().join(area.directory())
    }
}

#[derive(Clone)]
pub struct HydrationCoordinator {
    scheduler: HydrationScheduler,
    cache: HydrationCache,
}

impl HydrationCoordinator {
    pub fn new(scheduler: HydrationScheduler, cache: HydrationCache) -> Self {
        Self { scheduler, cache }
    }

    pub fn cache(&self) -> &HydrationCache {
        &self.cache
    }

    pub async fn run(
        &self,
        plan: HydrationPlan,
    ) -> Result<HydrationReceipt, HydrationCoordinatorError> {
        if plan.fence != *self.cache.fence() {
            return Err(HydrationError::StaleGeneration.into());
        }
        let receipt = self.scheduler.run(plan).await?;
        self.cache.write_hydration_receipt(&receipt)?;
        Ok(receipt)
    }

    pub async fn continue_from(
        &self,
        original_plan: &HydrationPlan,
        overall_deadline_milliseconds: u64,
    ) -> Result<Option<HydrationReceipt>, HydrationCoordinatorError> {
        let Some(receipt) = self.cache.read_hydration_receipt()? else {
            return Ok(None);
        };
        let Some(plan) = original_plan.continuation(&receipt, overall_deadline_milliseconds)?
        else {
            return Ok(None);
        };
        self.run(plan).await.map(Some)
    }
}

fn validate_cache_key(key: &str) -> Result<(), HydrationCacheError> {
    if valid_cache_key(key) {
        Ok(())
    } else {
        Err(HydrationCacheError::InvalidKey)
    }
}

fn valid_cache_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_CACHE_KEY_BYTES
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn prepare_directory(path: &Path) -> Result<(), HydrationCacheError> {
    reject_symlink(path)?;
    fs::create_dir_all(path)?;
    reject_symlink(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), HydrationCacheError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(HydrationCacheError::UnsafePath),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn atomic_owner_only_write(path: &Path, bytes: &[u8]) -> Result<(), HydrationCacheError> {
    if bytes.len() > MAX_CACHE_DOCUMENT_BYTES {
        return Err(HydrationCacheError::DocumentTooLarge);
    }
    reject_symlink(path)?;
    let mut file = AtomicWriteFile::open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.commit()?;
    Ok(())
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, HydrationCacheError> {
    reject_symlink(path)?;
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.len() > MAX_CACHE_DOCUMENT_BYTES as u64 {
        return Err(HydrationCacheError::InvalidDocument);
    }
    Ok(Some(fs::read(path)?))
}

fn remove_if_owned_by_fence(
    path: &Path,
    fence: &HydrationAccountFence,
) -> Result<(), HydrationCacheError> {
    let Some(bytes) = read_bounded(path)? else {
        return Ok(());
    };
    let envelope: CacheEnvelope = serde_json::from_slice(&bytes)?;
    if envelope.fence == *fence {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn remove_scoped_directory(path: &Path, parent: &Path) -> Result<(), HydrationCacheError> {
    if path.parent() != Some(parent) {
        return Err(HydrationCacheError::UnsafePath);
    }
    reject_symlink(path)?;
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn duration_milliseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HydrationError {
    #[error("the hydration account fence is invalid")]
    InvalidFence,
    #[error("the hydration deadline is invalid")]
    InvalidDeadline,
    #[error("the hydration plan is invalid")]
    InvalidPlan,
    #[error("the hydration item limit is invalid")]
    InvalidLimit,
    #[error("the hydration account generation is stale")]
    StaleGeneration,
}

#[derive(Debug, Error)]
pub enum HydrationCacheError {
    #[error("the hydration cache key is invalid")]
    InvalidKey,
    #[error("the hydration cache path is unsafe")]
    UnsafePath,
    #[error("the hydration cache document is invalid")]
    InvalidDocument,
    #[error("the hydration cache document exceeds its bound")]
    DocumentTooLarge,
    #[error("the hydration cache account generation is stale")]
    StaleGeneration,
    #[error("the hydration cache purge could not be verified")]
    PurgeVerification,
    #[error("hydration cache storage is unavailable")]
    Io(#[from] io::Error),
    #[error("the hydration cache document could not be encoded")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum HydrationCoordinatorError {
    #[error(transparent)]
    Hydration(#[from] HydrationError),
    #[error(transparent)]
    Cache(#[from] HydrationCacheError),
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::*;

    fn fence(generation: u64) -> HydrationAccountFence {
        HydrationAccountFence::new(
            AccountRef::new("omega-account-fixture").expect("account ref"),
            NostrPublicKeyHex::new(
                "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            )
            .expect("public key"),
            generation,
        )
        .expect("fence")
    }

    fn source_plan(source: HydrationSource, deadline_milliseconds: u64) -> HydrationSourcePlan {
        HydrationSourcePlan::new(source, deadline_milliseconds).expect("source plan")
    }

    struct FakeRunner {
        calls: AtomicUsize,
        outcomes: Mutex<BTreeMap<HydrationSource, HydrationSourceOutcome>>,
        stalled: Mutex<BTreeSet<HydrationSource>>,
    }

    impl FakeRunner {
        fn new(
            outcomes: impl IntoIterator<Item = (HydrationSource, HydrationSourceOutcome)>,
        ) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                outcomes: Mutex::new(outcomes.into_iter().collect()),
                stalled: Mutex::new(BTreeSet::new()),
            }
        }

        fn stall(&self, source: HydrationSource) {
            self.stalled.lock().expect("stalled lock").insert(source);
        }
    }

    impl HydrationSourceRunner for FakeRunner {
        fn hydrate(
            &self,
            request: HydrationSourceRequest,
        ) -> Pin<Box<dyn Future<Output = HydrationSourceOutcome> + Send + 'static>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let outcome = self
                .outcomes
                .lock()
                .expect("outcomes lock")
                .get(&request.source)
                .cloned()
                .unwrap_or(HydrationSourceOutcome::Failed);
            let stalled = self
                .stalled
                .lock()
                .expect("stalled lock")
                .contains(&request.source);
            Box::pin(async move {
                if stalled {
                    futures::future::pending::<()>().await;
                }
                outcome
            })
        }
    }

    #[test]
    fn fresh_candidate_skips_every_source_call() {
        let runner = Arc::new(FakeRunner::new([]));
        let plan = HydrationPlan::new(
            fence(1),
            HydrationTrigger::Startup,
            100,
            true,
            [source_plan(HydrationSource::Profile, 50)],
        )
        .expect("plan");
        let receipt =
            smol::block_on(HydrationScheduler::new(runner.clone()).run(plan)).expect("receipt");
        assert_eq!(receipt.state, HydrationState::SkippedFresh);
        assert!(receipt.sources.is_empty());
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn source_item_limits_are_bounded_and_enforced() {
        assert!(matches!(
            source_plan(HydrationSource::RecentRooms, 50).with_item_limit(0),
            Err(HydrationError::InvalidLimit)
        ));
        let runner = Arc::new(FakeRunner::new([(
            HydrationSource::RecentRooms,
            HydrationSourceOutcome::Complete { items: 3 },
        )]));
        let plan = HydrationPlan::new(
            fence(1),
            HydrationTrigger::Imported,
            100,
            false,
            [source_plan(HydrationSource::RecentRooms, 50)
                .with_item_limit(2)
                .expect("item limit")],
        )
        .expect("plan");
        let receipt = smol::block_on(HydrationScheduler::new(runner).run(plan)).expect("receipt");
        assert_eq!(receipt.state, HydrationState::Failed);
        assert_eq!(
            receipt.sources.first().expect("source receipt").outcome,
            HydrationSourceOutcome::Failed
        );
    }

    #[test]
    fn scheduler_reports_complete_partial_offline_and_failed() {
        let cases = [
            (
                vec![
                    (
                        HydrationSource::Profile,
                        HydrationSourceOutcome::Complete { items: 1 },
                    ),
                    (
                        HydrationSource::RelayPreferences,
                        HydrationSourceOutcome::Disabled,
                    ),
                ],
                HydrationState::Complete,
            ),
            (
                vec![
                    (
                        HydrationSource::Profile,
                        HydrationSourceOutcome::Complete { items: 1 },
                    ),
                    (
                        HydrationSource::RelayPreferences,
                        HydrationSourceOutcome::Offline,
                    ),
                ],
                HydrationState::Partial,
            ),
            (
                vec![
                    (HydrationSource::Profile, HydrationSourceOutcome::Offline),
                    (
                        HydrationSource::RelayPreferences,
                        HydrationSourceOutcome::Cached {
                            items: 2,
                            reason: CacheFallbackReason::Offline,
                        },
                    ),
                ],
                HydrationState::Offline,
            ),
            (
                vec![
                    (HydrationSource::Profile, HydrationSourceOutcome::Failed),
                    (
                        HydrationSource::RelayPreferences,
                        HydrationSourceOutcome::Failed,
                    ),
                ],
                HydrationState::Failed,
            ),
            (
                vec![
                    (
                        HydrationSource::Profile,
                        HydrationSourceOutcome::Stale { cached_items: 1 },
                    ),
                    (
                        HydrationSource::RelayPreferences,
                        HydrationSourceOutcome::Complete { items: 1 },
                    ),
                ],
                HydrationState::Partial,
            ),
        ];
        for (outcomes, expected) in cases {
            let sources = outcomes
                .iter()
                .map(|(source, _)| source_plan(*source, 100))
                .collect::<Vec<_>>();
            let runner = Arc::new(FakeRunner::new(outcomes));
            let plan =
                HydrationPlan::new(fence(1), HydrationTrigger::Imported, 200, false, sources)
                    .expect("plan");
            let receipt =
                smol::block_on(HydrationScheduler::new(runner).run(plan)).expect("receipt");
            assert_eq!(receipt.state, expected);
        }
    }

    #[test]
    fn source_and_overall_timeouts_are_distinct_and_retryable() {
        let runner = Arc::new(FakeRunner::new([
            (
                HydrationSource::Profile,
                HydrationSourceOutcome::Complete { items: 1 },
            ),
            (
                HydrationSource::RelayPreferences,
                HydrationSourceOutcome::Complete { items: 1 },
            ),
        ]));
        runner.stall(HydrationSource::Profile);
        runner.stall(HydrationSource::RelayPreferences);
        let plan = HydrationPlan::new(
            fence(2),
            HydrationTrigger::Switched,
            40,
            false,
            [
                source_plan(HydrationSource::Profile, 20),
                source_plan(HydrationSource::RelayPreferences, 80),
            ],
        )
        .expect("plan");
        let receipt =
            smol::block_on(HydrationScheduler::new(runner).run(plan.clone())).expect("receipt");
        assert!(receipt.sources.iter().any(|source| {
            source.outcome
                == HydrationSourceOutcome::TimedOut {
                    scope: TimeoutScope::Source,
                }
        }));
        assert!(receipt.sources.iter().any(|source| {
            source.outcome
                == HydrationSourceOutcome::TimedOut {
                    scope: TimeoutScope::Overall,
                }
        }));
        assert_eq!(receipt.state, HydrationState::Partial);
        let continuation = plan
            .continuation(&receipt, 100)
            .expect("continuation")
            .expect("retryable plan");
        assert_eq!(
            continuation.trigger,
            HydrationTrigger::BackgroundContinuation
        );
        assert_eq!(continuation.sources.len(), 2);
    }

    #[test]
    fn cache_is_partitioned_and_stale_generation_cannot_write_after_a_b_a() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let first = HydrationCache::open(directory.path(), fence(1)).expect("first cache");
        first
            .write(
                HydrationCacheArea::Profiles,
                "profile",
                serde_json::json!({"name": "first"}),
            )
            .expect("first write");
        let third = HydrationCache::open(directory.path(), fence(3)).expect("third cache");
        third
            .write(
                HydrationCacheArea::Profiles,
                "profile",
                serde_json::json!({"name": "third"}),
            )
            .expect("third write");
        assert!(matches!(
            first.write(
                HydrationCacheArea::Profiles,
                "profile",
                serde_json::json!({"name": "stale"})
            ),
            Err(HydrationCacheError::StaleGeneration)
        ));
        assert_eq!(
            third
                .read(HydrationCacheArea::Profiles, "profile")
                .expect("read"),
            Some(serde_json::json!({"name": "third"}))
        );
    }

    #[test]
    fn plaintext_policy_never_persists_private_or_expiring_content() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let cache = HydrationCache::open(directory.path(), fence(1)).expect("cache");
        assert!(
            !cache
                .write_plaintext(
                    "never",
                    serde_json::json!("secret"),
                    PlaintextPersistencePolicy::Never,
                    PlaintextClassification {
                        private: false,
                        expires_at: None,
                    },
                )
                .expect("policy")
        );
        assert!(
            !cache
                .write_plaintext(
                    "private",
                    serde_json::json!("secret"),
                    PlaintextPersistencePolicy::NonPrivateNonExpiring,
                    PlaintextClassification {
                        private: true,
                        expires_at: None,
                    },
                )
                .expect("private policy")
        );
        assert!(
            !cache
                .write_plaintext(
                    "expiring",
                    serde_json::json!("secret"),
                    PlaintextPersistencePolicy::NonPrivateNonExpiring,
                    PlaintextClassification {
                        private: false,
                        expires_at: Some(10),
                    },
                )
                .expect("expiry policy")
        );
        assert!(
            cache
                .write_plaintext(
                    "public",
                    serde_json::json!("public"),
                    PlaintextPersistencePolicy::NonPrivateNonExpiring,
                    PlaintextClassification {
                        private: false,
                        expires_at: None,
                    },
                )
                .expect("public policy")
        );
    }

    #[test]
    fn declined_bulk_decrypt_consent_is_durable_and_suppresses_prompts() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let cache = HydrationCache::open(directory.path(), fence(4)).expect("cache");
        let consent = BulkDecryptConsent::new(
            fence(4),
            "capability-fixture",
            BulkDecryptConsentState::Declined,
            100,
        )
        .expect("consent");
        cache
            .write_bulk_decrypt_consent(&consent)
            .expect("write consent");
        let reopened = HydrationCache::open(directory.path(), fence(4)).expect("reopen");
        let loaded = reopened
            .read_bulk_decrypt_consent("capability-fixture")
            .expect("read consent")
            .expect("stored consent");
        assert_eq!(loaded.state, BulkDecryptConsentState::Declined);
        assert!(loaded.suppresses_prompt());
        assert!(!loaded.permits_bulk_decrypt());
        assert!(
            reopened
                .read_bulk_decrypt_consent("other-capability")
                .expect("other signer")
                .is_none()
        );
    }

    #[test]
    fn account_state_facade_persists_policy_consent_and_receipt() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let cache = HydrationCache::open(directory.path(), fence(4)).expect("cache");
        cache
            .write_plaintext_persistence_policy(PlaintextPersistencePolicy::NonPrivateNonExpiring)
            .expect("write policy");
        cache
            .set_bulk_decrypt_consent("capability-fixture", BulkDecryptConsentState::Allowed)
            .expect("write consent");
        let receipt = HydrationReceipt {
            fence: fence(4),
            trigger: HydrationTrigger::Startup,
            state: HydrationState::Complete,
            started_at: 1,
            elapsed_milliseconds: 2,
            background_continuation_available: false,
            sources: vec![HydrationSourceReceipt {
                source: HydrationSource::Profile,
                outcome: HydrationSourceOutcome::Complete { items: 1 },
                elapsed_milliseconds: 1,
            }],
        };
        cache
            .write_hydration_receipt(&receipt)
            .expect("write receipt");

        let state = cache
            .inspect_account_state("capability-fixture")
            .expect("inspect state");
        assert_eq!(state.latest_receipt, Some(receipt));
        assert_eq!(
            state.plaintext_policy,
            PlaintextPersistencePolicy::NonPrivateNonExpiring
        );
        assert_eq!(state.bulk_decrypt_consent, BulkDecryptConsentState::Allowed);
    }

    #[test]
    fn purge_areas_and_account_are_verified() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let cache = HydrationCache::open(directory.path(), fence(5)).expect("cache");
        cache
            .write(
                HydrationCacheArea::Ciphertext,
                "group-list",
                serde_json::json!("ciphertext"),
            )
            .expect("ciphertext");
        cache
            .write(
                HydrationCacheArea::Relays,
                "relay-list",
                serde_json::json!(["wss://relay.example"]),
            )
            .expect("relays");
        cache
            .purge_area(HydrationCacheArea::Ciphertext)
            .expect("purge ciphertext");
        assert!(
            cache
                .read(HydrationCacheArea::Ciphertext, "group-list")
                .expect("read purged")
                .is_none()
        );
        let account_root = cache.account_root();
        cache.purge_account().expect("purge account");
        assert!(!account_root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn cache_files_and_directories_are_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("temporary directory");
        let cache = HydrationCache::open(directory.path(), fence(6)).expect("cache");
        cache
            .write(HydrationCacheArea::Groups, "groups", serde_json::json!([]))
            .expect("write");
        let area = cache.account_root().join("groups");
        let file = area.join("groups.json");
        assert_eq!(
            fs::metadata(area).expect("area").permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(file).expect("file").permissions().mode() & 0o777,
            0o600
        );
    }
}
