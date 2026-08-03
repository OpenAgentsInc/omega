//! Read-only composition of source-owned Work projections.
//!
//! The index never becomes Work authority. Each adapter retains its exact
//! source reference and generated All Work boundary metadata. Refreshes stage
//! complete pages before replacing a lane, so one failed source cannot erase
//! another lane or a last-qualified offline snapshot.

mod dogfood_fixture;

pub use dogfood_fixture::*;

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use omega_effectd::all_work_contract::{
    AgentDelegate, Completeness, CompletenessState, ContractValidate, ContractVersion, Freshness,
    FreshnessState, HumanAssignee, IsoTimestamp, Nullable, PrincipalRef, PrivacyClass,
    RedactionMetadata, SafeInteger, ShortText, SourceAuthority, SourceAuthorityKind, SourceRef,
    WorkClass, WorkCursor, WorkDomain, WorkIndexReadResult, WorkPriority, WorkRef, WorkState,
    WorkSummary,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const WORK_INDEX_SCHEMA_V1: &str = "openagents.omega.work-index.v1";
pub const THREAD_ADAPTER_ID: &str = "omega.thread-metadata.v1";
pub const FORENSICS_ADAPTER_ID: &str = "omega.forensics-workbench.v1";
pub const EFFECT_ADAPTER_ID: &str = "openagents.omega-effectd.v2";
pub const LOCAL_OWNER_REF: &str = "principal:omega:local-owner";
pub const MAX_INDEX_ITEMS: usize = 10_000;
const STORE_DIR: &str = "work-index-v1";
const STORE_FILE: &str = "snapshot.json";
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum WorkIndexError {
    #[error("invalid All Work value: {0}")]
    InvalidContract(String),
    #[error("unsupported Work Index schema {0:?}")]
    UnsupportedSchema(String),
    #[error("adapter {adapter_id:?} did not request cursor {actual:?}; expected {expected:?}")]
    CursorMismatch {
        adapter_id: String,
        expected: Option<String>,
        actual: Option<String>,
    },
    #[error("adapter {adapter_id:?} has no refresh in progress")]
    RefreshNotStarted { adapter_id: String },
    #[error("adapter lane {adapter_id:?} does not match its admitted source contract")]
    AdapterMismatch { adapter_id: String },
    #[error("adapter {adapter_id:?} projected conflicting identity for {work_ref:?}")]
    ConflictingIdentity {
        adapter_id: String,
        work_ref: String,
    },
    #[error("Work Index exceeds {MAX_INDEX_ITEMS} rows")]
    TooManyItems,
    #[error("Work Index persistence failed: {0}")]
    Persistence(String),
}

impl From<omega_effectd::all_work_contract::ContractValidationError> for WorkIndexError {
    fn from(error: omega_effectd::all_work_contract::ContractValidationError) -> Self {
        Self::InvalidContract(error.to_string())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkIndexView {
    #[default]
    Inbox,
    MyWork,
}

impl WorkIndexView {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Inbox => "Inbox",
            Self::MyWork => "My Work",
        }
    }

    pub const fn route_ref(self) -> &'static str {
        match self {
            Self::Inbox => "work-index:omega:inbox",
            Self::MyWork => "work-index:omega:my-work",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionGroup {
    Question,
    Recoverable,
    Blocked,
    Failed,
    Stale,
    Active,
    Waiting,
    Triage,
    Planned,
    Completed,
    Canceled,
    Archived,
}

impl AttentionGroup {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Question => "Question",
            Self::Recoverable => "Recoverable",
            Self::Blocked => "Blocked",
            Self::Failed => "Failed",
            Self::Stale => "Stale",
            Self::Active => "Active",
            Self::Waiting => "Waiting",
            Self::Triage => "Triage",
            Self::Planned => "Planned",
            Self::Completed => "Completed",
            Self::Canceled => "Canceled",
            Self::Archived => "Archived",
        }
    }

    const fn rank(self) -> u8 {
        match self {
            Self::Question => 0,
            Self::Recoverable => 1,
            Self::Blocked => 2,
            Self::Failed => 3,
            Self::Stale => 4,
            Self::Active => 5,
            Self::Waiting => 6,
            Self::Triage => 7,
            Self::Planned => 8,
            Self::Completed => 9,
            Self::Canceled => 10,
            Self::Archived => 11,
        }
    }

    pub const fn requires_inbox_attention(self) -> bool {
        matches!(
            self,
            Self::Question | Self::Recoverable | Self::Blocked | Self::Failed | Self::Stale
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountabilityKind {
    Owner,
    Assignee,
    Participant,
    DelegatedAgent,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionHint {
    #[default]
    None,
    Question,
    Recoverable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkSourceEntity {
    Thread { thread_ref: String },
    ForensicsCase { case_ref: String },
    ForensicsRun { case_ref: String, run_ref: String },
    EffectWork { work_ref: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkIndexItem {
    pub summary: WorkSummary,
    pub attention: AttentionGroup,
    pub accountability: BTreeSet<AccountabilityKind>,
    pub source_entity: WorkSourceEntity,
}

impl WorkIndexItem {
    pub fn validate(&self) -> Result<(), WorkIndexError> {
        self.summary.validate()?;
        let authority_matches = matches!(
            (&self.source_entity, &self.summary.source_authority.kind),
            (
                WorkSourceEntity::Thread { .. }
                    | WorkSourceEntity::ForensicsCase { .. }
                    | WorkSourceEntity::ForensicsRun { .. },
                SourceAuthorityKind::OmegaNative,
            ) | (
                WorkSourceEntity::EffectWork { .. },
                SourceAuthorityKind::EffectService
            )
        );
        if !authority_matches {
            return Err(WorkIndexError::InvalidContract(
                "source entity and source authority disagree".into(),
            ));
        }
        let identity_matches = match &self.source_entity {
            WorkSourceEntity::Thread { thread_ref } => {
                self.work_ref() == format!("work:omega:thread:{thread_ref}")
                    && self.source_ref() == format!("thread:omega:{thread_ref}")
            }
            WorkSourceEntity::ForensicsCase { case_ref } => {
                self.work_ref() == format!("work:omega:forensics:{case_ref}")
                    && self.source_ref() == format!("forensics:case:{case_ref}")
            }
            WorkSourceEntity::ForensicsRun { run_ref, .. } => {
                self.work_ref() == format!("work:omega:forensics-run:{run_ref}")
                    && self.source_ref() == run_ref
            }
            WorkSourceEntity::EffectWork { work_ref } => self.work_ref() == work_ref,
        };
        if identity_matches {
            Ok(())
        } else {
            Err(WorkIndexError::InvalidContract(
                "source entity and Work identity disagree".into(),
            ))
        }
    }

    pub fn work_ref(&self) -> &str {
        &self.summary.work_ref.0
    }

    pub fn source_ref(&self) -> &str {
        &self.summary.source_authority.source_ref.0
    }

    fn validate_for_lane(
        &self,
        adapter_id: &str,
        adapter_version: &str,
        origin: AdapterOrigin,
    ) -> Result<(), WorkIndexError> {
        self.validate()?;
        let matches_lane = match &self.source_entity {
            WorkSourceEntity::Thread { .. } => {
                adapter_id == THREAD_ADAPTER_ID
                    && adapter_version == THREAD_ADAPTER_ID
                    && origin == AdapterOrigin::OmegaNative
            }
            WorkSourceEntity::ForensicsCase { .. } | WorkSourceEntity::ForensicsRun { .. } => {
                adapter_id == FORENSICS_ADAPTER_ID
                    && adapter_version == FORENSICS_ADAPTER_ID
                    && origin == AdapterOrigin::OmegaNative
            }
            WorkSourceEntity::EffectWork { .. } => {
                adapter_id == EFFECT_ADAPTER_ID
                    && adapter_version == "omega-effectd.v2"
                    && origin == AdapterOrigin::EffectService
            }
        };
        if matches_lane {
            Ok(())
        } else {
            Err(WorkIndexError::AdapterMismatch {
                adapter_id: adapter_id.to_string(),
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterOrigin {
    OmegaNative,
    EffectService,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum AdapterHealth {
    Loading,
    Ready,
    OfflineCached(String),
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkIndexLane {
    pub adapter_id: String,
    pub adapter_version: String,
    pub origin: AdapterOrigin,
    pub health: AdapterHealth,
    pub items: Vec<WorkIndexItem>,
    pub resume_cursor: Option<String>,
    pub completeness: Option<Completeness>,
    pub generated_at: Option<String>,
    #[serde(skip)]
    pending: Option<PendingRefresh>,
}

#[derive(Clone, Debug, PartialEq)]
struct PendingRefresh {
    expected_cursor: Option<String>,
    items: Vec<WorkIndexItem>,
    completeness: Option<Completeness>,
    generated_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdapterPage {
    pub requested_cursor: Option<String>,
    pub next_cursor: Option<String>,
    pub completeness: Completeness,
    pub generated_at: String,
    pub items: Vec<WorkIndexItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkIdentityConflict {
    pub work_ref: String,
    pub first_adapter_id: String,
    pub conflicting_adapter_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkIndexHealth {
    Loading,
    Ready,
    Empty,
    Partial,
    Offline,
    Error,
    Conflict,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkIndexProjection {
    pub health: WorkIndexHealth,
    pub admitted: bool,
    pub rows: Vec<WorkIndexItem>,
    pub conflicts: Vec<WorkIdentityConflict>,
    pub lane_errors: Vec<(String, String)>,
    pub gap_refs: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkIndexQuery {
    pub view: WorkIndexView,
    pub search: Option<String>,
    pub domains: Vec<WorkDomain>,
    pub states: Vec<WorkState>,
    pub attention: Vec<AttentionGroup>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistedWorkIndex {
    pub schema: String,
    pub lanes: Vec<WorkIndexLane>,
    pub selected_work_ref: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkIndex {
    lanes: BTreeMap<String, WorkIndexLane>,
    selected_work_ref: Option<String>,
}

impl WorkIndex {
    pub fn begin_refresh(
        &mut self,
        adapter_id: impl Into<String>,
        adapter_version: impl Into<String>,
        origin: AdapterOrigin,
    ) {
        let adapter_id = adapter_id.into();
        let adapter_version = adapter_version.into();
        let lane = self
            .lanes
            .entry(adapter_id.clone())
            .or_insert_with(|| WorkIndexLane {
                adapter_id: adapter_id.clone(),
                adapter_version: adapter_version.clone(),
                origin,
                health: AdapterHealth::Loading,
                items: Vec::new(),
                resume_cursor: None,
                completeness: None,
                generated_at: None,
                pending: None,
            });
        lane.adapter_version = adapter_version;
        lane.origin = origin;
        lane.health = AdapterHealth::Loading;
        lane.pending = Some(PendingRefresh {
            expected_cursor: None,
            items: Vec::new(),
            completeness: None,
            generated_at: None,
        });
    }

    pub fn begin_resume(&mut self, adapter_id: &str) -> Option<String> {
        let lane = self.lanes.get_mut(adapter_id)?;
        let cursor = lane.resume_cursor.clone()?;
        lane.health = AdapterHealth::Loading;
        lane.pending = Some(PendingRefresh {
            expected_cursor: Some(cursor.clone()),
            items: lane.items.clone(),
            completeness: lane.completeness.clone(),
            generated_at: lane.generated_at.clone(),
        });
        Some(cursor)
    }

    pub fn apply_page(
        &mut self,
        adapter_id: &str,
        page: AdapterPage,
    ) -> Result<bool, WorkIndexError> {
        let other_lane_items = self
            .lanes
            .iter()
            .filter(|(lane_id, _)| lane_id.as_str() != adapter_id)
            .map(|(_, lane)| lane.items.len())
            .sum::<usize>();
        let lane =
            self.lanes
                .get_mut(adapter_id)
                .ok_or_else(|| WorkIndexError::RefreshNotStarted {
                    adapter_id: adapter_id.into(),
                })?;
        let lane_adapter_id = lane.adapter_id.clone();
        let lane_adapter_version = lane.adapter_version.clone();
        let lane_origin = lane.origin;
        let pending = lane
            .pending
            .as_mut()
            .ok_or_else(|| WorkIndexError::RefreshNotStarted {
                adapter_id: adapter_id.into(),
            })?;
        if pending.expected_cursor != page.requested_cursor {
            return Err(WorkIndexError::CursorMismatch {
                adapter_id: adapter_id.into(),
                expected: pending.expected_cursor.clone(),
                actual: page.requested_cursor,
            });
        }

        for item in page.items {
            item.validate_for_lane(&lane_adapter_id, &lane_adapter_version, lane_origin)?;
            if let Some(existing) = pending
                .items
                .iter_mut()
                .find(|existing| existing.work_ref() == item.work_ref())
            {
                if existing.source_ref() != item.source_ref()
                    || (existing.summary.revision == item.summary.revision && existing != &item)
                {
                    return Err(WorkIndexError::ConflictingIdentity {
                        adapter_id: adapter_id.into(),
                        work_ref: item.work_ref().into(),
                    });
                }
                if item.summary.revision.0 > existing.summary.revision.0 {
                    *existing = item;
                }
            } else {
                if other_lane_items.saturating_add(pending.items.len()) >= MAX_INDEX_ITEMS {
                    return Err(WorkIndexError::TooManyItems);
                }
                pending.items.push(item);
            }
        }
        pending.completeness = Some(page.completeness);
        pending.generated_at = Some(page.generated_at);
        pending.expected_cursor = page.next_cursor.clone();

        if page.next_cursor.is_some() {
            return Ok(false);
        }

        let Some(mut completed) = lane.pending.take() else {
            return Err(WorkIndexError::RefreshNotStarted {
                adapter_id: adapter_id.into(),
            });
        };
        completed
            .items
            .sort_by(|left, right| left.work_ref().cmp(right.work_ref()));
        lane.items = completed.items;
        lane.resume_cursor = completed
            .completeness
            .as_ref()
            .and_then(|completeness| completeness.cursor.as_ref())
            .and_then(|cursor| cursor.as_ref())
            .map(|cursor| cursor.0.clone());
        lane.completeness = completed.completeness;
        lane.generated_at = completed.generated_at;
        lane.health = AdapterHealth::Ready;
        self.reconcile_selection();
        Ok(true)
    }

    pub fn fail_refresh(&mut self, adapter_id: &str, message: impl Into<String>, offline: bool) {
        let message = message.into();
        let lane = self
            .lanes
            .entry(adapter_id.into())
            .or_insert_with(|| WorkIndexLane {
                adapter_id: adapter_id.into(),
                adapter_version: "unavailable".into(),
                origin: AdapterOrigin::OmegaNative,
                health: AdapterHealth::Loading,
                items: Vec::new(),
                resume_cursor: None,
                completeness: None,
                generated_at: None,
                pending: None,
            });
        lane.pending = None;
        lane.health = if offline {
            AdapterHealth::OfflineCached(message)
        } else {
            AdapterHealth::Error(message)
        };
    }

    pub fn apply_native_items(
        &mut self,
        adapter_id: &str,
        adapter_version: &str,
        items: Vec<WorkIndexItem>,
        generated_at: String,
        revision: u64,
    ) -> Result<(), WorkIndexError> {
        self.begin_refresh(adapter_id, adapter_version, AdapterOrigin::OmegaNative);
        let cursor = format!("cursor:{adapter_id}:{revision}");
        let completeness = Completeness {
            state: CompletenessState::Complete,
            cursor: Some(Some(WorkCursor::try_from(cursor)?)),
            gap_refs: Vec::new(),
        };
        self.apply_page(
            adapter_id,
            AdapterPage {
                requested_cursor: None,
                next_cursor: None,
                completeness,
                generated_at,
                items,
            },
        )?;
        Ok(())
    }

    pub fn apply_effect_result(
        &mut self,
        result: WorkIndexReadResult,
        requested_cursor: Option<String>,
    ) -> Result<bool, WorkIndexError> {
        result.validate()?;
        if requested_cursor.is_none() {
            self.begin_refresh(
                EFFECT_ADAPTER_ID,
                "omega-effectd.v2",
                AdapterOrigin::EffectService,
            );
        }
        let items = result
            .items
            .into_iter()
            .map(effect_item)
            .collect::<Result<Vec<_>, _>>()?;
        self.apply_page(
            EFFECT_ADAPTER_ID,
            AdapterPage {
                requested_cursor,
                next_cursor: result.next_cursor.flatten().map(|cursor| cursor.0),
                completeness: result.completeness,
                generated_at: result.generated_at.0,
                items,
            },
        )
    }

    pub fn projection(&self) -> WorkIndexProjection {
        let mut rows_by_ref: BTreeMap<String, (String, WorkIndexItem)> = BTreeMap::new();
        let mut conflicted_refs = BTreeSet::new();
        let mut conflicts = Vec::new();
        let mut lane_errors = Vec::new();
        let mut gap_refs = BTreeSet::new();
        let mut loading = false;
        let mut offline = false;
        let mut errors = false;
        let mut partial = false;

        for (adapter_id, lane) in &self.lanes {
            match &lane.health {
                AdapterHealth::Loading => loading = true,
                AdapterHealth::OfflineCached(message) => {
                    offline = true;
                    lane_errors.push((adapter_id.clone(), message.clone()));
                }
                AdapterHealth::Error(message) => {
                    errors = true;
                    lane_errors.push((adapter_id.clone(), message.clone()));
                }
                AdapterHealth::Ready => {}
            }
            if let Some(completeness) = &lane.completeness {
                partial |= !matches!(completeness.state, CompletenessState::Complete);
                gap_refs.extend(completeness.gap_refs.iter().map(|gap| gap.0.clone()));
            }
            for item in &lane.items {
                let work_ref = item.work_ref().to_string();
                if conflicted_refs.contains(&work_ref) {
                    continue;
                }
                match rows_by_ref.get_mut(&work_ref) {
                    None => {
                        rows_by_ref.insert(work_ref, (adapter_id.clone(), item.clone()));
                    }
                    Some((first_adapter_id, existing))
                        if existing.source_ref() == item.source_ref()
                            && existing.summary.revision.0 < item.summary.revision.0 =>
                    {
                        *first_adapter_id = adapter_id.clone();
                        *existing = item.clone();
                    }
                    Some((_first_adapter_id, existing)) if existing == item => {}
                    Some((first_adapter_id, _)) => {
                        conflicts.push(WorkIdentityConflict {
                            work_ref: work_ref.clone(),
                            first_adapter_id: first_adapter_id.clone(),
                            conflicting_adapter_id: adapter_id.clone(),
                        });
                        rows_by_ref.remove(&work_ref);
                        conflicted_refs.insert(work_ref);
                    }
                }
            }
        }

        let mut rows = rows_by_ref
            .into_values()
            .map(|(_, item)| item)
            .collect::<Vec<_>>();
        sort_rows(&mut rows);
        let admitted = self.admitted();
        let health = if !conflicts.is_empty() {
            WorkIndexHealth::Conflict
        } else if loading {
            WorkIndexHealth::Loading
        } else if offline {
            WorkIndexHealth::Offline
        } else if errors && rows.is_empty() {
            WorkIndexHealth::Error
        } else if errors || partial {
            WorkIndexHealth::Partial
        } else if rows.is_empty() {
            WorkIndexHealth::Empty
        } else {
            WorkIndexHealth::Ready
        };
        WorkIndexProjection {
            health,
            admitted,
            rows,
            conflicts,
            lane_errors,
            gap_refs: gap_refs.into_iter().collect(),
        }
    }

    pub fn query(&self, query: &WorkIndexQuery) -> Vec<WorkIndexItem> {
        let search = query
            .search
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_lowercase);
        let mut rows = self
            .projection()
            .rows
            .into_iter()
            .filter(|item| match query.view {
                WorkIndexView::Inbox => item.attention.requires_inbox_attention(),
                WorkIndexView::MyWork => !item.accountability.is_empty(),
            })
            .filter(|item| query.domains.is_empty() || query.domains.contains(&item.summary.domain))
            .filter(|item| query.states.is_empty() || query.states.contains(&item.summary.state))
            .filter(|item| query.attention.is_empty() || query.attention.contains(&item.attention))
            .filter(|item| {
                search.as_ref().is_none_or(|search| {
                    item.summary.title.0.to_lowercase().contains(search)
                        || item.work_ref().to_lowercase().contains(search)
                        || item.source_ref().to_lowercase().contains(search)
                        || item
                            .summary
                            .description
                            .as_ref()
                            .is_some_and(|description| {
                                description.0.to_lowercase().contains(search)
                            })
                })
            })
            .collect::<Vec<_>>();
        sort_rows(&mut rows);
        rows
    }

    pub fn item(&self, work_ref: &str) -> Option<WorkIndexItem> {
        self.projection()
            .rows
            .into_iter()
            .find(|item| item.work_ref() == work_ref)
    }

    pub fn select(&mut self, work_ref: Option<String>) -> bool {
        let next = work_ref.filter(|work_ref| self.item(work_ref).is_some());
        if self.selected_work_ref == next {
            return false;
        }
        self.selected_work_ref = next;
        true
    }

    pub fn selected_work_ref(&self) -> Option<&str> {
        self.selected_work_ref.as_deref()
    }

    pub fn admitted(&self) -> bool {
        self.lanes
            .values()
            .filter(|lane| !lane.items.is_empty())
            .map(|lane| lane.adapter_id.as_str())
            .collect::<BTreeSet<_>>()
            .len()
            >= 2
    }

    pub fn resume_cursor(&self, adapter_id: &str) -> Option<&str> {
        self.lanes
            .get(adapter_id)
            .and_then(|lane| lane.resume_cursor.as_deref())
    }

    pub fn persistable(&self) -> PersistedWorkIndex {
        PersistedWorkIndex {
            schema: WORK_INDEX_SCHEMA_V1.into(),
            lanes: self.lanes.values().cloned().collect(),
            selected_work_ref: self.selected_work_ref.clone(),
        }
    }

    pub fn restore(snapshot: PersistedWorkIndex) -> Result<Self, WorkIndexError> {
        if snapshot.schema != WORK_INDEX_SCHEMA_V1 {
            return Err(WorkIndexError::UnsupportedSchema(snapshot.schema));
        }
        let mut lanes = BTreeMap::new();
        let mut total_items = 0usize;
        for mut lane in snapshot.lanes {
            for item in &lane.items {
                item.validate_for_lane(&lane.adapter_id, &lane.adapter_version, lane.origin)?;
            }
            total_items = total_items.saturating_add(lane.items.len());
            if total_items > MAX_INDEX_ITEMS {
                return Err(WorkIndexError::TooManyItems);
            }
            lane.pending = None;
            lane.health = AdapterHealth::OfflineCached(
                "Showing the last qualified snapshot while this source reconnects.".into(),
            );
            lanes.insert(lane.adapter_id.clone(), lane);
        }
        let mut index = Self {
            lanes,
            selected_work_ref: snapshot.selected_work_ref,
        };
        index.reconcile_selection();
        Ok(index)
    }

    fn reconcile_selection(&mut self) {
        if self
            .selected_work_ref
            .as_deref()
            .is_some_and(|work_ref| self.item(work_ref).is_none())
        {
            self.selected_work_ref = None;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeThreadLifecycle {
    Running,
    WaitingForPerson,
    Failed,
    Completed,
    Canceled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeThreadRecord {
    pub thread_ref: String,
    pub title: String,
    pub updated_at: String,
    pub observed_at: String,
    pub revision: u64,
    pub archived: bool,
    pub lifecycle: NativeThreadLifecycle,
    pub assignee: Option<HumanAssignee>,
    pub agent_delegate: Option<AgentDelegate>,
}

pub fn adapt_thread(record: NativeThreadRecord) -> Result<WorkIndexItem, WorkIndexError> {
    let (state, hint) = if record.archived {
        (WorkState::Archived, AttentionHint::None)
    } else {
        match record.lifecycle {
            NativeThreadLifecycle::Running => (WorkState::Active, AttentionHint::None),
            NativeThreadLifecycle::WaitingForPerson => {
                (WorkState::Waiting, AttentionHint::Question)
            }
            NativeThreadLifecycle::Failed => (WorkState::Failed, AttentionHint::None),
            NativeThreadLifecycle::Completed => (WorkState::Completed, AttentionHint::None),
            NativeThreadLifecycle::Canceled => (WorkState::Canceled, AttentionHint::None),
        }
    };
    let source_ref = format!("thread:omega:{}", record.thread_ref);
    let summary = make_summary(SummaryInput {
        work_ref: format!("work:omega:thread:{}", record.thread_ref),
        title: record.title,
        domain: WorkDomain::General,
        work_class: WorkClass::Task,
        state,
        priority: WorkPriority::Normal,
        source_ref,
        source_kind: SourceAuthorityKind::OmegaNative,
        adapter_version: THREAD_ADAPTER_ID.into(),
        source_writable: true,
        revision: record.revision,
        updated_at: record.updated_at,
        observed_at: record.observed_at,
        assignee: record.assignee,
        agent_delegate: record.agent_delegate,
    })?;
    let mut accountability = BTreeSet::new();
    accountability.insert(AccountabilityKind::Owner);
    accountability.insert(AccountabilityKind::Participant);
    if summary.assignee.0.is_some() {
        accountability.insert(AccountabilityKind::Assignee);
    }
    if summary.agent_delegate.as_ref().is_some_and(Option::is_some) {
        accountability.insert(AccountabilityKind::DelegatedAgent);
    }
    let item = WorkIndexItem {
        attention: attention_for(&summary, hint)?,
        summary,
        accountability,
        source_entity: WorkSourceEntity::Thread {
            thread_ref: record.thread_ref,
        },
    };
    item.validate()?;
    Ok(item)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeForensicsPhase {
    Prepared,
    Admitting,
    WorkerReady,
    Running,
    Waiting,
    Settled,
    Cleaned,
    Refused,
    Failed,
    RecoveryRequired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeForensicsRecord {
    pub case_ref: String,
    pub repository_name: String,
    pub updated_at: String,
    pub observed_at: String,
    pub revision: u64,
    pub phase: NativeForensicsPhase,
    pub run_ref: Option<String>,
    pub child_run_refs: Vec<String>,
}

pub fn adapt_forensics(
    record: NativeForensicsRecord,
) -> Result<Vec<WorkIndexItem>, WorkIndexError> {
    let (state, hint) = match record.phase {
        NativeForensicsPhase::Prepared
        | NativeForensicsPhase::Admitting
        | NativeForensicsPhase::WorkerReady
        | NativeForensicsPhase::Running => (WorkState::Active, AttentionHint::None),
        NativeForensicsPhase::Waiting => (WorkState::Waiting, AttentionHint::None),
        NativeForensicsPhase::Settled | NativeForensicsPhase::Cleaned => {
            (WorkState::Completed, AttentionHint::None)
        }
        NativeForensicsPhase::Refused | NativeForensicsPhase::Failed => {
            (WorkState::Failed, AttentionHint::None)
        }
        NativeForensicsPhase::RecoveryRequired => (WorkState::Blocked, AttentionHint::Recoverable),
    };
    let case_summary = make_summary(SummaryInput {
        work_ref: format!("work:omega:forensics:{}", record.case_ref),
        title: format!("Security case · {}", record.repository_name),
        domain: WorkDomain::Security,
        work_class: WorkClass::Case,
        state: state.clone(),
        priority: WorkPriority::High,
        source_ref: format!("forensics:case:{}", record.case_ref),
        source_kind: SourceAuthorityKind::OmegaNative,
        adapter_version: FORENSICS_ADAPTER_ID.into(),
        source_writable: false,
        revision: record.revision,
        updated_at: record.updated_at.clone(),
        observed_at: record.observed_at.clone(),
        assignee: None,
        agent_delegate: None,
    })?;
    let mut accountability = BTreeSet::new();
    accountability.insert(AccountabilityKind::Owner);
    let case = WorkIndexItem {
        attention: attention_for(&case_summary, hint)?,
        summary: case_summary,
        accountability: accountability.clone(),
        source_entity: WorkSourceEntity::ForensicsCase {
            case_ref: record.case_ref.clone(),
        },
    };
    case.validate()?;
    let mut items = vec![case];
    let mut run_refs = record.child_run_refs;
    if let Some(run_ref) = record.run_ref
        && !run_refs.contains(&run_ref)
    {
        run_refs.push(run_ref);
    }
    for run_ref in run_refs {
        let run_summary = make_summary(SummaryInput {
            work_ref: format!("work:omega:forensics-run:{run_ref}"),
            title: format!("Security run · {}", record.repository_name),
            domain: WorkDomain::Security,
            work_class: WorkClass::Run,
            state: state.clone(),
            priority: WorkPriority::High,
            source_ref: run_ref.clone(),
            source_kind: SourceAuthorityKind::OmegaNative,
            adapter_version: FORENSICS_ADAPTER_ID.into(),
            source_writable: false,
            revision: record.revision,
            updated_at: record.updated_at.clone(),
            observed_at: record.observed_at.clone(),
            assignee: None,
            agent_delegate: None,
        })?;
        let run = WorkIndexItem {
            attention: attention_for(&run_summary, hint)?,
            summary: run_summary,
            accountability: accountability.clone(),
            source_entity: WorkSourceEntity::ForensicsRun {
                case_ref: record.case_ref.clone(),
                run_ref,
            },
        };
        run.validate()?;
        items.push(run);
    }
    Ok(items)
}

fn effect_item(summary: WorkSummary) -> Result<WorkIndexItem, WorkIndexError> {
    summary.validate()?;
    let mut accountability = BTreeSet::new();
    let locally_owned = summary.owner_ref.0 == LOCAL_OWNER_REF;
    if locally_owned {
        accountability.insert(AccountabilityKind::Owner);
    }
    let locally_assigned = summary
        .assignee
        .0
        .as_ref()
        .is_some_and(|assignee| assignee.principal_ref.0 == LOCAL_OWNER_REF);
    if locally_assigned {
        accountability.insert(AccountabilityKind::Assignee);
    }
    if (locally_owned || locally_assigned)
        && summary.agent_delegate.as_ref().is_some_and(Option::is_some)
    {
        accountability.insert(AccountabilityKind::DelegatedAgent);
    }
    let work_ref = summary.work_ref.0.clone();
    let item = WorkIndexItem {
        attention: attention_for(&summary, AttentionHint::None)?,
        summary,
        accountability,
        source_entity: WorkSourceEntity::EffectWork { work_ref },
    };
    item.validate()?;
    Ok(item)
}

struct SummaryInput {
    work_ref: String,
    title: String,
    domain: WorkDomain,
    work_class: WorkClass,
    state: WorkState,
    priority: WorkPriority,
    source_ref: String,
    source_kind: SourceAuthorityKind,
    adapter_version: String,
    source_writable: bool,
    revision: u64,
    updated_at: String,
    observed_at: String,
    assignee: Option<HumanAssignee>,
    agent_delegate: Option<AgentDelegate>,
}

fn make_summary(input: SummaryInput) -> Result<WorkSummary, WorkIndexError> {
    let updated_at = IsoTimestamp::try_from(input.updated_at)?;
    let observed_at = IsoTimestamp::try_from(input.observed_at)?;
    let revision = SafeInteger::try_from(input.revision)?;
    let owner_ref = PrincipalRef::try_from(LOCAL_OWNER_REF.to_string())?;
    let summary = WorkSummary {
        contract_version: ContractVersion::OpenagentsAllWorkBoundaryV1,
        work_ref: WorkRef::try_from(input.work_ref)?,
        title: ShortText::try_from(input.title)?,
        description: None,
        domain: input.domain,
        work_class: input.work_class,
        state: input.state,
        priority: input.priority,
        owner_ref,
        assignee: Nullable(input.assignee),
        agent_delegate: Some(input.agent_delegate),
        portfolio: None,
        source_authority: SourceAuthority {
            kind: input.source_kind,
            source_ref: SourceRef::try_from(input.source_ref)?,
            adapter_version: ShortText::try_from(input.adapter_version)?,
            writable: input.source_writable,
        },
        revision,
        updated_at: updated_at.clone(),
        freshness: Freshness {
            state: FreshnessState::Fresh,
            observed_at,
            source_updated_at: Some(Some(updated_at)),
        },
        completeness: Completeness {
            state: CompletenessState::Complete,
            cursor: None,
            gap_refs: Vec::new(),
        },
        redaction: RedactionMetadata {
            privacy_class: PrivacyClass::Private,
            redacted_field_count: SafeInteger(0),
            policy_ref: SourceRef::try_from("policy:omega:private-work-v1".to_string())?,
        },
    };
    summary.validate()?;
    Ok(summary)
}

fn attention_for(
    summary: &WorkSummary,
    hint: AttentionHint,
) -> Result<AttentionGroup, WorkIndexError> {
    if !matches!(summary.freshness.state, FreshnessState::Fresh) {
        return Ok(AttentionGroup::Stale);
    }
    match hint {
        AttentionHint::Question if matches!(summary.state, WorkState::Waiting) => {
            return Ok(AttentionGroup::Question);
        }
        AttentionHint::Recoverable
            if matches!(summary.state, WorkState::Blocked | WorkState::Failed) =>
        {
            return Ok(AttentionGroup::Recoverable);
        }
        AttentionHint::Question | AttentionHint::Recoverable => {
            return Err(WorkIndexError::InvalidContract(
                "attention hint is not supported by source state".into(),
            ));
        }
        AttentionHint::None => {}
    }
    Ok(match summary.state {
        WorkState::Triage => AttentionGroup::Triage,
        WorkState::Planned => AttentionGroup::Planned,
        WorkState::Active => AttentionGroup::Active,
        WorkState::Waiting => AttentionGroup::Waiting,
        WorkState::Blocked => AttentionGroup::Blocked,
        WorkState::Failed => AttentionGroup::Failed,
        WorkState::Completed => AttentionGroup::Completed,
        WorkState::Canceled => AttentionGroup::Canceled,
        WorkState::Archived => AttentionGroup::Archived,
    })
}

fn sort_rows(rows: &mut [WorkIndexItem]) {
    rows.sort_by(|left, right| {
        left.attention
            .rank()
            .cmp(&right.attention.rank())
            .then_with(|| {
                priority_rank(&left.summary.priority).cmp(&priority_rank(&right.summary.priority))
            })
            .then_with(|| right.summary.updated_at.0.cmp(&left.summary.updated_at.0))
            .then_with(|| left.work_ref().cmp(right.work_ref()))
    });
}

const fn priority_rank(priority: &WorkPriority) -> u8 {
    match priority {
        WorkPriority::Urgent => 0,
        WorkPriority::High => 1,
        WorkPriority::Normal => 2,
        WorkPriority::Low => 3,
        WorkPriority::None => 4,
    }
}

pub fn store_path(data_dir: &Path) -> PathBuf {
    data_dir.join(STORE_DIR).join(STORE_FILE)
}

pub fn write_snapshot(data_dir: &Path, index: &WorkIndex) -> Result<(), WorkIndexError> {
    let path = store_path(data_dir);
    let Some(parent) = path.parent() else {
        return Err(WorkIndexError::Persistence(
            "Work Index snapshot has no parent directory".into(),
        ));
    };
    std::fs::create_dir_all(parent)
        .map_err(|error| WorkIndexError::Persistence(error.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| WorkIndexError::Persistence(error.to_string()))?;
    }
    let bytes = serde_json::to_vec_pretty(&index.persistable())
        .map_err(|error| WorkIndexError::Persistence(error.to_string()))?;
    let temporary = path.with_extension(format!(
        "json.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| WorkIndexError::Persistence(error.to_string()))?;
        file.write_all(&bytes)
            .map_err(|error| WorkIndexError::Persistence(error.to_string()))?;
        file.sync_all()
            .map_err(|error| WorkIndexError::Persistence(error.to_string()))?;
        std::fs::rename(&temporary, &path)
            .map_err(|error| WorkIndexError::Persistence(error.to_string()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub fn read_snapshot(data_dir: &Path) -> Result<Option<WorkIndex>, WorkIndexError> {
    let path = store_path(data_dir);
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(WorkIndexError::Persistence(error.to_string())),
    };
    let snapshot: PersistedWorkIndex = serde_json::from_slice(&bytes)
        .map_err(|error| WorkIndexError::Persistence(error.to_string()))?;
    WorkIndex::restore(snapshot).map(Some)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use proptest::prelude::*;
    use tempfile::tempdir;

    use super::*;

    fn thread(index: u64, lifecycle: NativeThreadLifecycle) -> WorkIndexItem {
        adapt_thread(NativeThreadRecord {
            thread_ref: format!("thread-{index}"),
            title: format!("Thread {index}"),
            updated_at: "2026-08-02T12:00:00Z".into(),
            observed_at: "2026-08-02T12:00:01Z".into(),
            revision: index + 1,
            archived: false,
            lifecycle,
            assignee: None,
            agent_delegate: None,
        })
        .expect("valid thread")
    }

    fn forensics(phase: NativeForensicsPhase) -> Vec<WorkIndexItem> {
        adapt_forensics(NativeForensicsRecord {
            case_ref: "repository:omega".into(),
            repository_name: "Omega".into(),
            updated_at: "2026-08-02T12:00:00Z".into(),
            observed_at: "2026-08-02T12:00:01Z".into(),
            revision: 7,
            phase,
            run_ref: Some("run:forensics:omega:7".into()),
            child_run_refs: Vec::new(),
        })
        .expect("valid Forensics")
    }

    #[test]
    fn multiple_security_cases_and_child_runs_keep_separate_source_identity() {
        let first = adapt_forensics(NativeForensicsRecord {
            case_ref: "repository:one".into(),
            repository_name: "One".into(),
            updated_at: "2026-08-02T12:00:00Z".into(),
            observed_at: "2026-08-02T12:00:01Z".into(),
            revision: 3,
            phase: NativeForensicsPhase::Running,
            run_ref: Some("run:security:one:primary".into()),
            child_run_refs: vec![
                "run:security:one:entropy".into(),
                "run:security:one:matrix".into(),
            ],
        })
        .expect("first Security Work");
        let second = adapt_forensics(NativeForensicsRecord {
            case_ref: "repository:two".into(),
            repository_name: "Two".into(),
            updated_at: "2026-08-02T12:00:02Z".into(),
            observed_at: "2026-08-02T12:00:03Z".into(),
            revision: 8,
            phase: NativeForensicsPhase::Prepared,
            run_ref: None,
            child_run_refs: Vec::new(),
        })
        .expect("second Security Work");
        assert_eq!(first.len(), 4);
        assert_eq!(second.len(), 1);
        assert_ne!(first[0].work_ref(), second[0].work_ref());
        assert!(first.iter().all(|row| match &row.source_entity {
            WorkSourceEntity::ForensicsCase { case_ref }
            | WorkSourceEntity::ForensicsRun { case_ref, .. } => case_ref == "repository:one",
            _ => false,
        }));
        assert!(second[0].summary.title.0.starts_with("Security case"));
    }

    proptest! {
        #[test]
        fn child_run_projection_round_trips_every_unique_ref(
            suffixes in prop::collection::btree_set(1_u16..10_000, 1..33)
        ) {
            let child_run_refs = suffixes
                .iter()
                .map(|suffix| format!("run:security:property:{suffix}"))
                .collect::<Vec<_>>();
            let rows = adapt_forensics(NativeForensicsRecord {
                case_ref: "repository:property".into(),
                repository_name: "Property".into(),
                updated_at: "2026-08-02T12:00:00Z".into(),
                observed_at: "2026-08-02T12:00:01Z".into(),
                revision: 1,
                phase: NativeForensicsPhase::Running,
                run_ref: None,
                child_run_refs: child_run_refs.clone(),
            })
            .expect("property Security Work");
            let observed = rows
                .iter()
                .filter_map(|row| match &row.source_entity {
                    WorkSourceEntity::ForensicsRun { run_ref, .. } => Some(run_ref.clone()),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            prop_assert_eq!(observed, child_run_refs.into_iter().collect());
        }
    }

    fn apply_native(
        index: &mut WorkIndex,
        adapter: &str,
        items: Vec<WorkIndexItem>,
        revision: u64,
    ) {
        index
            .apply_native_items(
                adapter,
                adapter,
                items,
                "2026-08-02T12:00:00Z".into(),
                revision,
            )
            .expect("apply native lane");
    }

    #[test]
    fn two_native_authorities_admit_honest_inbox_and_my_work_rows() {
        let mut index = WorkIndex::default();
        apply_native(
            &mut index,
            THREAD_ADAPTER_ID,
            vec![thread(1, NativeThreadLifecycle::WaitingForPerson)],
            2,
        );
        apply_native(
            &mut index,
            FORENSICS_ADAPTER_ID,
            forensics(NativeForensicsPhase::RecoveryRequired),
            7,
        );

        let projection = index.projection();
        assert!(projection.admitted);
        assert_eq!(projection.health, WorkIndexHealth::Ready);
        assert_eq!(projection.rows.len(), 3);
        assert_eq!(projection.rows[0].attention, AttentionGroup::Question);
        assert_eq!(projection.rows[1].attention, AttentionGroup::Recoverable);
        assert_ne!(
            projection.rows[0].summary.domain,
            projection.rows[1].summary.domain
        );
        assert!(projection.rows.iter().any(|row| {
            matches!(row.source_entity, WorkSourceEntity::Thread { .. })
                && row.summary.source_authority.writable
        }));
        assert!(projection.rows.iter().all(|row| {
            !matches!(row.source_entity, WorkSourceEntity::ForensicsCase { .. })
                || !row.summary.source_authority.writable
        }));

        let inbox = index.query(&WorkIndexQuery {
            view: WorkIndexView::Inbox,
            ..WorkIndexQuery::default()
        });
        let my_work = index.query(&WorkIndexQuery {
            view: WorkIndexView::MyWork,
            ..WorkIndexQuery::default()
        });
        assert_eq!(inbox.len(), 3);
        assert_eq!(my_work.len(), 3);
        assert!(
            my_work
                .iter()
                .any(|item| matches!(item.source_entity, WorkSourceEntity::Thread { .. }))
        );
        assert!(
            my_work
                .iter()
                .any(|item| matches!(item.source_entity, WorkSourceEntity::ForensicsCase { .. }))
        );
    }

    #[test]
    fn failed_refresh_keeps_other_lanes_and_the_last_qualified_source_snapshot() {
        let mut index = WorkIndex::default();
        apply_native(
            &mut index,
            THREAD_ADAPTER_ID,
            vec![thread(1, NativeThreadLifecycle::Running)],
            2,
        );
        apply_native(
            &mut index,
            FORENSICS_ADAPTER_ID,
            forensics(NativeForensicsPhase::Running),
            7,
        );
        index.begin_refresh(
            THREAD_ADAPTER_ID,
            THREAD_ADAPTER_ID,
            AdapterOrigin::OmegaNative,
        );
        index.fail_refresh(THREAD_ADAPTER_ID, "thread database unavailable", true);
        let projection = index.projection();
        assert_eq!(projection.rows.len(), 3);
        assert_eq!(projection.health, WorkIndexHealth::Offline);
        assert_eq!(projection.lane_errors.len(), 1);
    }

    #[test]
    fn cursor_gap_and_conflicting_identity_are_visible_and_never_empty_success() {
        let mut index = WorkIndex::default();
        index.begin_refresh(
            THREAD_ADAPTER_ID,
            THREAD_ADAPTER_ID,
            AdapterOrigin::OmegaNative,
        );
        let mut duplicate = thread(1, NativeThreadLifecycle::Running);
        duplicate.summary.title =
            ShortText::try_from("Conflicting title".to_string()).expect("title");
        let error = index
            .apply_page(
                THREAD_ADAPTER_ID,
                AdapterPage {
                    requested_cursor: None,
                    next_cursor: None,
                    completeness: Completeness {
                        state: CompletenessState::Gap,
                        cursor: Some(None),
                        gap_refs: vec![
                            SourceRef::try_from("gap:thread:1".to_string()).expect("gap ref"),
                        ],
                    },
                    generated_at: "2026-08-02T12:00:00Z".into(),
                    items: vec![thread(1, NativeThreadLifecycle::Running), duplicate],
                },
            )
            .expect_err("conflicting source identity");
        assert!(matches!(error, WorkIndexError::ConflictingIdentity { .. }));
        index.fail_refresh(THREAD_ADAPTER_ID, error.to_string(), false);
        assert_eq!(index.projection().health, WorkIndexHealth::Error);
    }

    #[test]
    fn cross_authority_identity_conflict_is_quarantined() {
        let mut index = WorkIndex::default();
        let native = thread(1, NativeThreadLifecycle::Running);
        let mut effect_summary = native.summary.clone();
        effect_summary.source_authority.kind = SourceAuthorityKind::EffectService;
        effect_summary.source_authority.source_ref =
            SourceRef::try_from("full-auto:run:collision".to_string()).expect("effect source ref");
        effect_summary.source_authority.adapter_version =
            ShortText::try_from("full-auto-registry.v1".to_string()).expect("effect adapter");
        let effect = effect_item(effect_summary).expect("valid colliding Effect item");
        apply_native(&mut index, THREAD_ADAPTER_ID, vec![native], 2);
        index.begin_refresh(
            EFFECT_ADAPTER_ID,
            "omega-effectd.v2",
            AdapterOrigin::EffectService,
        );
        assert!(
            index
                .apply_page(
                    EFFECT_ADAPTER_ID,
                    AdapterPage {
                        requested_cursor: None,
                        next_cursor: None,
                        completeness: Completeness {
                            state: CompletenessState::Complete,
                            cursor: Some(None),
                            gap_refs: Vec::new(),
                        },
                        generated_at: "2026-08-02T12:00:02Z".into(),
                        items: vec![effect],
                    },
                )
                .expect("effect collision page")
        );
        let projection = index.projection();
        assert_eq!(projection.health, WorkIndexHealth::Conflict);
        assert!(projection.rows.is_empty());
        assert_eq!(projection.conflicts.len(), 1);
    }

    #[test]
    fn loading_gap_and_truncation_states_remain_explicit() {
        let mut index = WorkIndex::default();
        assert_eq!(index.projection().health, WorkIndexHealth::Empty);
        index.begin_refresh(
            THREAD_ADAPTER_ID,
            THREAD_ADAPTER_ID,
            AdapterOrigin::OmegaNative,
        );
        assert_eq!(index.projection().health, WorkIndexHealth::Loading);
        assert!(
            index
                .apply_page(
                    THREAD_ADAPTER_ID,
                    AdapterPage {
                        requested_cursor: None,
                        next_cursor: None,
                        completeness: Completeness {
                            state: CompletenessState::Gap,
                            cursor: Some(Some(
                                WorkCursor::try_from("cursor:thread:gap".to_string())
                                    .expect("gap cursor"),
                            )),
                            gap_refs: vec![
                                SourceRef::try_from("gap:thread:missing".to_string())
                                    .expect("gap ref"),
                            ],
                        },
                        generated_at: "2026-08-02T12:00:02Z".into(),
                        items: vec![thread(1, NativeThreadLifecycle::Running)],
                    },
                )
                .expect("qualified gap page")
        );
        let projection = index.projection();
        assert_eq!(projection.health, WorkIndexHealth::Partial);
        assert_eq!(projection.gap_refs, vec!["gap:thread:missing"]);

        index.begin_refresh(
            THREAD_ADAPTER_ID,
            THREAD_ADAPTER_ID,
            AdapterOrigin::OmegaNative,
        );
        assert!(
            index
                .apply_page(
                    THREAD_ADAPTER_ID,
                    AdapterPage {
                        requested_cursor: None,
                        next_cursor: None,
                        completeness: Completeness {
                            state: CompletenessState::Truncated,
                            cursor: Some(None),
                            gap_refs: Vec::new(),
                        },
                        generated_at: "2026-08-02T12:00:03Z".into(),
                        items: vec![thread(2, NativeThreadLifecycle::Running)],
                    },
                )
                .expect("qualified truncated page")
        );
        assert_eq!(index.projection().health, WorkIndexHealth::Partial);
    }

    #[test]
    fn offline_restart_restores_qualified_rows_selection_and_cursors() {
        let directory = tempdir().expect("temp dir");
        let mut index = WorkIndex::default();
        apply_native(
            &mut index,
            THREAD_ADAPTER_ID,
            vec![thread(1, NativeThreadLifecycle::Running)],
            2,
        );
        apply_native(
            &mut index,
            FORENSICS_ADAPTER_ID,
            forensics(NativeForensicsPhase::Running),
            7,
        );
        let selected = index.projection().rows[0].work_ref().to_string();
        assert!(index.select(Some(selected.clone())));
        write_snapshot(directory.path(), &index).expect("write snapshot");

        let restored = read_snapshot(directory.path())
            .expect("read snapshot")
            .expect("snapshot present");
        assert_eq!(restored.selected_work_ref(), Some(selected.as_str()));
        assert_eq!(restored.projection().health, WorkIndexHealth::Offline);
        assert!(restored.admitted());
        assert!(restored.resume_cursor(THREAD_ADAPTER_ID).is_some());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let path = store_path(directory.path());
            assert_eq!(
                std::fs::metadata(path)
                    .expect("snapshot metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn generated_effect_summary_crosses_the_index_without_fact_rewrites() {
        let mut summary: WorkSummary = serde_json::from_str(include_str!(
            "../../omega_effectd/all-work-contract/fixtures/valid/work-summary.json"
        ))
        .expect("generated Work summary fixture");
        summary.source_authority.kind = SourceAuthorityKind::EffectService;
        summary.freshness.state = FreshnessState::Stale;
        summary.owner_ref =
            PrincipalRef::try_from(LOCAL_OWNER_REF.to_string()).expect("local owner ref");
        summary
            .assignee
            .0
            .as_mut()
            .expect("fixture assignee")
            .principal_ref =
            PrincipalRef::try_from(LOCAL_OWNER_REF.to_string()).expect("local assignee ref");
        summary.validate().expect("adapted generated summary");
        let expected = summary.clone();
        let expected_cursor = summary.completeness.cursor.clone().expect("fixture cursor");
        let mut index = WorkIndex::default();
        assert!(
            index
                .apply_effect_result(
                    WorkIndexReadResult {
                        items: vec![summary],
                        next_cursor: Some(None),
                        completeness: Completeness {
                            state: CompletenessState::Complete,
                            cursor: Some(expected_cursor),
                            gap_refs: Vec::new(),
                        },
                        generated_at: IsoTimestamp::try_from("2026-08-02T12:00:02Z".to_string(),)
                            .expect("generated at"),
                    },
                    None,
                )
                .expect("generated effect page")
        );
        let item = index
            .item(expected.work_ref.0.as_str())
            .expect("indexed row");
        assert_eq!(item.summary, expected);
        assert_eq!(item.attention, AttentionGroup::Stale);
        assert!(item.accountability.contains(&AccountabilityKind::Owner));
        assert!(item.accountability.contains(&AccountabilityKind::Assignee));
        assert_eq!(
            index.begin_resume(EFFECT_ADAPTER_ID),
            Some("cursor:forensics:7".to_string())
        );
    }

    #[test]
    fn ten_thousand_rows_search_filter_group_and_selection_stay_deterministic() {
        let started = Instant::now();
        let mut index = WorkIndex::default();
        let rows = (0..MAX_INDEX_ITEMS as u64)
            .map(|position| {
                thread(
                    position,
                    if position % 7 == 0 {
                        NativeThreadLifecycle::WaitingForPerson
                    } else {
                        NativeThreadLifecycle::Running
                    },
                )
            })
            .collect();
        apply_native(&mut index, THREAD_ADAPTER_ID, rows, MAX_INDEX_ITEMS as u64);
        let matches = index.query(&WorkIndexQuery {
            view: WorkIndexView::MyWork,
            search: Some("Thread 9999".into()),
            attention: vec![AttentionGroup::Active],
            ..WorkIndexQuery::default()
        });
        assert_eq!(matches.len(), 1);
        let selected = matches[0].work_ref().to_string();
        assert!(index.select(Some(selected.clone())));
        assert_eq!(index.selected_work_ref(), Some(selected.as_str()));
        assert!(started.elapsed().as_secs_f32() < 2.0);
    }

    proptest! {
        #[test]
        fn repeated_identical_source_rows_are_idempotent(revision in 1u64..10_000) {
            let mut index = WorkIndex::default();
            let item = adapt_thread(NativeThreadRecord {
                thread_ref: "thread-property".into(),
                title: "Property thread".into(),
                updated_at: "2026-08-02T12:00:00Z".into(),
                observed_at: "2026-08-02T12:00:01Z".into(),
                revision,
                archived: false,
                lifecycle: NativeThreadLifecycle::Running,
                assignee: None,
                agent_delegate: None,
            }).expect("valid thread");
            apply_native(&mut index, THREAD_ADAPTER_ID, vec![item.clone(), item], revision);
            prop_assert_eq!(index.projection().rows.len(), 1);
        }
    }
}
