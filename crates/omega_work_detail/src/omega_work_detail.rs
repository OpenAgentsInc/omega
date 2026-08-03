use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use omega_effectd::all_work_contract::{
    AgentActivityRef, AgentSessionRef, ContractValidate, EventRef, EvidenceRef, FreshnessState,
    IntentRef, IsoTimestamp, IssueProjection, LongText, OwnerDispositionRef, ReceiptRef, RunRef,
    SafeInteger, SessionRef, ShortText, SourceRef, ThreadRef, VerificationRef, WorkPriority,
    WorkRef, WorkSnapshot, WorkState,
};
use omega_work_index::{WorkIndexItem, WorkSourceEntity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const WORK_DETAIL_JOURNAL_SCHEMA_V1: &str = "openagents.omega.work-detail-journal.v1";
pub const MAX_WORK_BLOCKS: usize = 64;
pub const MAX_WORK_HISTORY_ROWS: usize = 10_000;
pub const MAX_WORK_INTENT_RECORDS: usize = 4_096;
const JOURNAL_DIR: &str = "work-detail-v1";
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Error)]
pub enum WorkDetailError {
    #[error("invalid All Work contract: {0}")]
    Contract(#[from] omega_effectd::all_work_contract::ContractValidationError),
    #[error("Issue projection does not share the Work identity, revision, and state")]
    IssueIdentityMismatch,
    #[error("Work detail source does not match its generated summary")]
    SourceMismatch,
    #[error("Work detail has more than {MAX_WORK_BLOCKS} Blocks")]
    TooManyBlocks,
    #[error("Work detail journal has more than {MAX_WORK_INTENT_RECORDS} Intents")]
    TooManyIntents,
    #[error("Work detail Block does not belong to this Work")]
    BlockIdentityMismatch,
    #[error("Work detail journal does not belong to this Work and source")]
    JournalIdentityMismatch,
    #[error("unsupported Work detail journal schema {0:?}")]
    UnsupportedJournalSchema(String),
    #[error("idempotency key is already bound to a different Work Intent")]
    IdempotencyConflict,
    #[error("canonical Work Event does not match a pending Intent")]
    EventIntentMismatch,
    #[error("canonical Work Event does not continue the current revision")]
    EventRevisionMismatch,
    #[error("source Work revision regressed")]
    RevisionRegression,
    #[error("Work detail journal I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Work detail journal encoding failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkPresentation {
    #[default]
    Work,
    Issue,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkBlockKind {
    Conversation,
    Editor,
    Diff,
    Plan,
    Terminal,
    Review,
    Preview,
    Log,
    Metric,
    Guide,
    Artifact,
    Receipt,
}

impl WorkBlockKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Conversation => "Conversation",
            Self::Editor => "Editor",
            Self::Diff => "Diff",
            Self::Plan => "Plan",
            Self::Terminal => "Terminal",
            Self::Review => "Review",
            Self::Preview => "Preview",
            Self::Log => "Log",
            Self::Metric => "Metric",
            Self::Guide => "Guide",
            Self::Artifact => "Artifact",
            Self::Receipt => "Receipt",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkBlock {
    pub block_ref: SourceRef,
    pub work_ref: WorkRef,
    pub kind: WorkBlockKind,
    pub title: ShortText,
    pub source_ref: SourceRef,
    pub available: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkActivityKind {
    Thread,
    Session,
    AgentSession,
    AgentActivity,
    Run,
    Intent,
    Event,
    Receipt,
    Evidence,
    Verification,
    OwnerDisposition,
    Gap,
}

impl WorkActivityKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Thread => "Thread",
            Self::Session => "Session",
            Self::AgentSession => "Agent Session",
            Self::AgentActivity => "Agent Activity",
            Self::Run => "Run",
            Self::Intent => "Intent",
            Self::Event => "Event",
            Self::Receipt => "Receipt",
            Self::Evidence => "Evidence",
            Self::Verification => "Verification",
            Self::OwnerDisposition => "Owner Disposition",
            Self::Gap => "Gap",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkActivityRow {
    pub sequence: usize,
    pub kind: WorkActivityKind,
    pub reference: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedWorkHistory {
    pub rows: Vec<WorkActivityRow>,
    pub truncated: bool,
    pub omitted: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkMutationKind {
    Title,
    Description,
    State,
    Priority,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkMutationOperation {
    SetTitle { title: ShortText },
    SetDescription { description: Option<LongText> },
    SetState { state: WorkState },
    SetPriority { priority: WorkPriority },
}

impl WorkMutationOperation {
    pub const fn kind(&self) -> WorkMutationKind {
        match self {
            Self::SetTitle { .. } => WorkMutationKind::Title,
            Self::SetDescription { .. } => WorkMutationKind::Description,
            Self::SetState { .. } => WorkMutationKind::State,
            Self::SetPriority { .. } => WorkMutationKind::Priority,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkMutationCapability {
    pub adapter_version: ShortText,
    pub source_ref: SourceRef,
    pub generation: SafeInteger,
    pub operations: BTreeSet<WorkMutationKind>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkIntent {
    pub intent_ref: IntentRef,
    pub work_ref: WorkRef,
    pub actor_ref: SourceRef,
    pub source_ref: SourceRef,
    pub expected_revision: SafeInteger,
    pub target_generation: SafeInteger,
    pub idempotency_key: ShortText,
    pub submitted_at: IsoTimestamp,
    pub operation: WorkMutationOperation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkCanonicalEvent {
    pub event_ref: EventRef,
    pub intent_ref: IntentRef,
    pub work_ref: WorkRef,
    pub source_ref: SourceRef,
    pub previous_revision: SafeInteger,
    pub revision: SafeInteger,
    pub admitted_at: IsoTimestamp,
    pub operation: WorkMutationOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkIntentRejection {
    ReadOnlySource,
    OperationUnsupported,
    AuthorityUnavailable,
    SourceRejected,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkIntentOutcome {
    Pending,
    Accepted {
        event_ref: EventRef,
        revision: SafeInteger,
    },
    Rejected {
        reason: WorkIntentRejection,
        detail: ShortText,
    },
    Offline,
    Conflict {
        current_revision: SafeInteger,
    },
    StaleGeneration {
        current_generation: SafeInteger,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkIntentRecord {
    pub intent: WorkIntent,
    pub outcome: WorkIntentOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubmitIntentDisposition {
    Submitted,
    Reconciled(WorkIntentOutcome),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkDetailSourceState {
    Loading,
    Ready,
    Offline,
    Error(String),
    Conflict(String),
}

#[derive(Clone, Debug, Default)]
pub struct SnapshotLinks {
    pub issue_identifier: Option<ShortText>,
    pub thread_refs: Vec<ThreadRef>,
    pub session_refs: Vec<SessionRef>,
    pub agent_session_refs: Vec<AgentSessionRef>,
    pub agent_activity_refs: Vec<AgentActivityRef>,
    pub run_refs: Vec<RunRef>,
    pub intent_refs: Vec<IntentRef>,
    pub event_refs: Vec<EventRef>,
    pub receipt_refs: Vec<ReceiptRef>,
    pub evidence_refs: Vec<EvidenceRef>,
    pub verification_refs: Vec<VerificationRef>,
    pub owner_disposition_refs: Vec<OwnerDispositionRef>,
}

pub fn snapshot_from_index_item(
    item: &WorkIndexItem,
    links: SnapshotLinks,
) -> Result<WorkSnapshot, WorkDetailError> {
    let summary = item.summary.clone();
    let issue = links.issue_identifier.map(|identifier| {
        Some(IssueProjection {
            work_ref: summary.work_ref.clone(),
            identifier,
            state: summary.state.clone(),
            revision: summary.revision,
        })
    });
    let snapshot = WorkSnapshot {
        summary,
        issue,
        relations: Vec::new(),
        thread_refs: links.thread_refs,
        session_refs: links.session_refs,
        agent_session_refs: links.agent_session_refs,
        agent_activity_refs: links.agent_activity_refs,
        run_refs: links.run_refs,
        intent_refs: links.intent_refs,
        event_refs: links.event_refs,
        receipt_refs: links.receipt_refs,
        evidence_refs: links.evidence_refs,
        verification_refs: links.verification_refs,
        owner_disposition_refs: links.owner_disposition_refs,
    };
    validate_snapshot_identity(&snapshot)?;
    Ok(snapshot)
}

pub fn default_blocks(item: &WorkIndexItem) -> Result<Vec<WorkBlock>, WorkDetailError> {
    let work_ref = item.summary.work_ref.clone();
    let source_ref = item.summary.source_authority.source_ref.clone();
    let mut kinds = Vec::new();
    match &item.source_entity {
        WorkSourceEntity::Thread { .. } => {
            kinds.extend([WorkBlockKind::Conversation, WorkBlockKind::Log]);
        }
        WorkSourceEntity::ForensicsCase { .. } | WorkSourceEntity::ForensicsRun { .. } => {
            kinds.extend([
                WorkBlockKind::Review,
                WorkBlockKind::Log,
                WorkBlockKind::Artifact,
                WorkBlockKind::Receipt,
            ]);
        }
        WorkSourceEntity::EffectWork { .. } => {
            kinds.extend([
                WorkBlockKind::Plan,
                WorkBlockKind::Log,
                WorkBlockKind::Artifact,
                WorkBlockKind::Receipt,
            ]);
        }
    }
    kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            Ok(WorkBlock {
                block_ref: SourceRef::try_from(format!(
                    "block:omega:{}:{}",
                    block_kind_key(kind),
                    index + 1
                ))?,
                work_ref: work_ref.clone(),
                kind,
                title: ShortText::try_from(kind.label().to_string())?,
                source_ref: source_ref.clone(),
                available: true,
            })
        })
        .collect()
}

fn block_kind_key(kind: WorkBlockKind) -> &'static str {
    match kind {
        WorkBlockKind::Conversation => "conversation",
        WorkBlockKind::Editor => "editor",
        WorkBlockKind::Diff => "diff",
        WorkBlockKind::Plan => "plan",
        WorkBlockKind::Terminal => "terminal",
        WorkBlockKind::Review => "review",
        WorkBlockKind::Preview => "preview",
        WorkBlockKind::Log => "log",
        WorkBlockKind::Metric => "metric",
        WorkBlockKind::Guide => "guide",
        WorkBlockKind::Artifact => "artifact",
        WorkBlockKind::Receipt => "receipt",
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkDetailJournal {
    pub schema: String,
    pub work_ref: WorkRef,
    pub source_ref: SourceRef,
    pub presentation: WorkPresentation,
    pub selected_block_ref: Option<SourceRef>,
    pub records: Vec<WorkIntentRecord>,
}

#[derive(Clone, Debug)]
pub struct WorkDetail {
    snapshot: WorkSnapshot,
    blocks: Vec<WorkBlock>,
    presentation: WorkPresentation,
    selected_block_ref: Option<SourceRef>,
    source_state: WorkDetailSourceState,
    capability: Option<WorkMutationCapability>,
    records: Vec<WorkIntentRecord>,
    idempotency: HashMap<String, usize>,
}

impl WorkDetail {
    pub fn new(
        snapshot: WorkSnapshot,
        blocks: Vec<WorkBlock>,
        capability: Option<WorkMutationCapability>,
        journal: Option<WorkDetailJournal>,
    ) -> Result<Self, WorkDetailError> {
        validate_snapshot_identity(&snapshot)?;
        if blocks.len() > MAX_WORK_BLOCKS {
            return Err(WorkDetailError::TooManyBlocks);
        }
        if blocks
            .iter()
            .any(|block| block.work_ref != snapshot.summary.work_ref)
        {
            return Err(WorkDetailError::BlockIdentityMismatch);
        }
        if let Some(capability) = capability.as_ref()
            && (!snapshot.summary.source_authority.writable
                || capability.source_ref != snapshot.summary.source_authority.source_ref
                || capability.adapter_version != snapshot.summary.source_authority.adapter_version)
        {
            return Err(WorkDetailError::SourceMismatch);
        }

        let source_state = if matches!(
            snapshot.summary.freshness.state,
            FreshnessState::OfflineCached
        ) {
            WorkDetailSourceState::Offline
        } else {
            WorkDetailSourceState::Ready
        };
        let (presentation, selected_block_ref, records) = match journal {
            Some(journal) => {
                validate_journal(&journal, &snapshot)?;
                (
                    journal.presentation,
                    journal.selected_block_ref,
                    journal.records,
                )
            }
            None => (WorkPresentation::Work, None, Vec::new()),
        };
        let selected_block_ref = selected_block_ref
            .filter(|selected| blocks.iter().any(|block| &block.block_ref == selected));
        let mut idempotency = HashMap::new();
        for (index, record) in records.iter().enumerate() {
            if idempotency
                .insert(record.intent.idempotency_key.0.clone(), index)
                .is_some()
            {
                return Err(WorkDetailError::IdempotencyConflict);
            }
        }
        Ok(Self {
            snapshot,
            blocks,
            presentation,
            selected_block_ref,
            source_state,
            capability,
            records,
            idempotency,
        })
    }

    pub fn snapshot(&self) -> &WorkSnapshot {
        &self.snapshot
    }

    pub fn blocks(&self) -> &[WorkBlock] {
        &self.blocks
    }

    pub fn presentation(&self) -> WorkPresentation {
        self.presentation
    }

    pub fn set_presentation(&mut self, presentation: WorkPresentation) {
        self.presentation = presentation;
    }

    pub fn selected_block_ref(&self) -> Option<&SourceRef> {
        self.selected_block_ref.as_ref()
    }

    pub fn select_block(&mut self, block_ref: &SourceRef) -> bool {
        if !self
            .blocks
            .iter()
            .any(|block| &block.block_ref == block_ref && block.available)
        {
            return false;
        }
        if self.selected_block_ref.as_ref() == Some(block_ref) {
            return false;
        }
        self.selected_block_ref = Some(block_ref.clone());
        true
    }

    pub fn selected_block(&self) -> Option<&WorkBlock> {
        self.selected_block_ref
            .as_ref()
            .and_then(|selected| {
                self.blocks
                    .iter()
                    .find(|block| &block.block_ref == selected)
            })
            .or_else(|| self.blocks.first())
    }

    pub fn source_state(&self) -> &WorkDetailSourceState {
        &self.source_state
    }

    pub fn set_source_state(&mut self, state: WorkDetailSourceState) {
        self.source_state = state;
    }

    pub fn capability(&self) -> Option<&WorkMutationCapability> {
        self.capability.as_ref()
    }

    pub fn can_mutate(&self, kind: WorkMutationKind) -> bool {
        matches!(self.source_state, WorkDetailSourceState::Ready)
            && self
                .capability
                .as_ref()
                .is_some_and(|capability| capability.operations.contains(&kind))
    }

    pub fn records(&self) -> &[WorkIntentRecord] {
        &self.records
    }

    pub fn intent_outcome(&self, intent_ref: &IntentRef) -> Option<&WorkIntentOutcome> {
        self.records
            .iter()
            .find(|record| &record.intent.intent_ref == intent_ref)
            .map(|record| &record.outcome)
    }

    pub fn submit_intent(
        &mut self,
        intent: WorkIntent,
        online: bool,
    ) -> Result<SubmitIntentDisposition, WorkDetailError> {
        validate_intent(&intent)?;
        if intent.work_ref != self.snapshot.summary.work_ref
            || intent.source_ref != self.snapshot.summary.source_authority.source_ref
        {
            return Err(WorkDetailError::SourceMismatch);
        }
        if let Some(index) = self.idempotency.get(&intent.idempotency_key.0).copied() {
            let existing = &self.records[index];
            if same_intent_request(&existing.intent, &intent) {
                return Ok(SubmitIntentDisposition::Reconciled(
                    existing.outcome.clone(),
                ));
            }
            return Err(WorkDetailError::IdempotencyConflict);
        }

        let outcome = if !online {
            WorkIntentOutcome::Offline
        } else if intent.expected_revision != self.snapshot.summary.revision {
            WorkIntentOutcome::Conflict {
                current_revision: self.snapshot.summary.revision,
            }
        } else if let Some(capability) = self.capability.as_ref() {
            if intent.target_generation != capability.generation {
                WorkIntentOutcome::StaleGeneration {
                    current_generation: capability.generation,
                }
            } else if !self.snapshot.summary.source_authority.writable {
                rejection(
                    WorkIntentRejection::ReadOnlySource,
                    "The source authority is read-only.",
                )?
            } else if !capability.operations.contains(&intent.operation.kind()) {
                rejection(
                    WorkIntentRejection::OperationUnsupported,
                    "The source does not admit this Work operation.",
                )?
            } else {
                WorkIntentOutcome::Pending
            }
        } else {
            rejection(
                WorkIntentRejection::AuthorityUnavailable,
                "No mutation authority is available for this Work.",
            )?
        };
        if self.records.len() >= MAX_WORK_INTENT_RECORDS {
            return Err(WorkDetailError::TooManyIntents);
        }
        let submitted = matches!(outcome, WorkIntentOutcome::Pending);
        let index = self.records.len();
        self.idempotency
            .insert(intent.idempotency_key.0.clone(), index);
        self.records.push(WorkIntentRecord { intent, outcome });
        if submitted {
            Ok(SubmitIntentDisposition::Submitted)
        } else {
            Ok(SubmitIntentDisposition::Reconciled(
                self.records[index].outcome.clone(),
            ))
        }
    }

    pub fn admit_event(&mut self, event: WorkCanonicalEvent) -> Result<(), WorkDetailError> {
        validate_event(&event)?;
        let Some(index) = self
            .records
            .iter()
            .position(|record| record.intent.intent_ref == event.intent_ref)
        else {
            return Err(WorkDetailError::EventIntentMismatch);
        };
        let record = &self.records[index];
        if !matches!(record.outcome, WorkIntentOutcome::Pending)
            || record.intent.work_ref != event.work_ref
            || record.intent.source_ref != event.source_ref
            || record.intent.operation != event.operation
            || event.work_ref != self.snapshot.summary.work_ref
            || event.source_ref != self.snapshot.summary.source_authority.source_ref
        {
            return Err(WorkDetailError::EventIntentMismatch);
        }
        let current_revision = self.snapshot.summary.revision.0;
        if event.previous_revision.0 != current_revision
            || event.revision.0 != current_revision.saturating_add(1)
        {
            return Err(WorkDetailError::EventRevisionMismatch);
        }

        apply_operation(&mut self.snapshot, &event.operation);
        self.snapshot.summary.revision = event.revision;
        self.snapshot.summary.updated_at = event.admitted_at.clone();
        self.snapshot.summary.freshness.state = FreshnessState::Fresh;
        self.snapshot.summary.freshness.observed_at = event.admitted_at.clone();
        self.snapshot.summary.freshness.source_updated_at = Some(Some(event.admitted_at.clone()));
        if let Some(Some(issue)) = self.snapshot.issue.as_mut() {
            issue.state = self.snapshot.summary.state.clone();
            issue.revision = event.revision;
        }
        if !self.snapshot.intent_refs.contains(&event.intent_ref) {
            if self.snapshot.intent_refs.len() >= MAX_WORK_INTENT_RECORDS {
                return Err(WorkDetailError::TooManyIntents);
            }
            self.snapshot.intent_refs.push(event.intent_ref.clone());
        }
        if !self.snapshot.event_refs.contains(&event.event_ref) {
            if self.snapshot.event_refs.len() >= MAX_WORK_INTENT_RECORDS {
                return Err(WorkDetailError::TooManyIntents);
            }
            self.snapshot.event_refs.push(event.event_ref.clone());
        }
        self.records[index].outcome = WorkIntentOutcome::Accepted {
            event_ref: event.event_ref,
            revision: event.revision,
        };
        Ok(())
    }

    pub fn reject_intent(
        &mut self,
        intent_ref: &IntentRef,
        detail: ShortText,
    ) -> Result<(), WorkDetailError> {
        self.resolve_intent(
            intent_ref,
            WorkIntentOutcome::Rejected {
                reason: WorkIntentRejection::SourceRejected,
                detail,
            },
        )
    }

    pub fn resolve_intent(
        &mut self,
        intent_ref: &IntentRef,
        outcome: WorkIntentOutcome,
    ) -> Result<(), WorkDetailError> {
        if matches!(
            outcome,
            WorkIntentOutcome::Pending | WorkIntentOutcome::Accepted { .. }
        ) {
            return Err(WorkDetailError::EventIntentMismatch);
        }
        let Some(record) = self
            .records
            .iter_mut()
            .find(|record| &record.intent.intent_ref == intent_ref)
        else {
            return Err(WorkDetailError::EventIntentMismatch);
        };
        if !matches!(record.outcome, WorkIntentOutcome::Pending) {
            return Err(WorkDetailError::EventIntentMismatch);
        }
        record.outcome = outcome;
        Ok(())
    }

    pub fn reconcile_source_snapshot(
        &mut self,
        snapshot: WorkSnapshot,
    ) -> Result<(), WorkDetailError> {
        validate_snapshot_identity(&snapshot)?;
        if snapshot.summary.work_ref != self.snapshot.summary.work_ref
            || snapshot.summary.source_authority.source_ref
                != self.snapshot.summary.source_authority.source_ref
        {
            return Err(WorkDetailError::SourceMismatch);
        }
        if snapshot.summary.revision.0 < self.snapshot.summary.revision.0 {
            return Err(WorkDetailError::RevisionRegression);
        }
        self.snapshot = snapshot;
        Ok(())
    }

    pub fn history(&self, maximum: usize) -> BoundedWorkHistory {
        let mut rows = Vec::new();
        let mut seen = HashSet::new();
        append_refs(
            &mut rows,
            &mut seen,
            WorkActivityKind::Thread,
            self.snapshot
                .thread_refs
                .iter()
                .map(|value| value.0.as_str()),
        );
        append_refs(
            &mut rows,
            &mut seen,
            WorkActivityKind::Session,
            self.snapshot
                .session_refs
                .iter()
                .map(|value| value.0.as_str()),
        );
        append_refs(
            &mut rows,
            &mut seen,
            WorkActivityKind::AgentSession,
            self.snapshot
                .agent_session_refs
                .iter()
                .map(|value| value.0.as_str()),
        );
        append_refs(
            &mut rows,
            &mut seen,
            WorkActivityKind::AgentActivity,
            self.snapshot
                .agent_activity_refs
                .iter()
                .map(|value| value.0.as_str()),
        );
        append_refs(
            &mut rows,
            &mut seen,
            WorkActivityKind::Run,
            self.snapshot.run_refs.iter().map(|value| value.0.as_str()),
        );
        append_refs(
            &mut rows,
            &mut seen,
            WorkActivityKind::Intent,
            self.snapshot
                .intent_refs
                .iter()
                .map(|value| value.0.as_str()),
        );
        append_refs(
            &mut rows,
            &mut seen,
            WorkActivityKind::Event,
            self.snapshot
                .event_refs
                .iter()
                .map(|value| value.0.as_str()),
        );
        append_refs(
            &mut rows,
            &mut seen,
            WorkActivityKind::Receipt,
            self.snapshot
                .receipt_refs
                .iter()
                .map(|value| value.0.as_str()),
        );
        append_refs(
            &mut rows,
            &mut seen,
            WorkActivityKind::Evidence,
            self.snapshot
                .evidence_refs
                .iter()
                .map(|value| value.0.as_str()),
        );
        append_refs(
            &mut rows,
            &mut seen,
            WorkActivityKind::Verification,
            self.snapshot
                .verification_refs
                .iter()
                .map(|value| value.0.as_str()),
        );
        append_refs(
            &mut rows,
            &mut seen,
            WorkActivityKind::OwnerDisposition,
            self.snapshot
                .owner_disposition_refs
                .iter()
                .map(|value| value.0.as_str()),
        );
        for record in &self.records {
            append_ref(
                &mut rows,
                &mut seen,
                WorkActivityKind::Intent,
                &record.intent.intent_ref.0,
            );
            if let WorkIntentOutcome::Accepted { event_ref, .. } = &record.outcome {
                append_ref(&mut rows, &mut seen, WorkActivityKind::Event, &event_ref.0);
            }
        }
        append_refs(
            &mut rows,
            &mut seen,
            WorkActivityKind::Gap,
            self.snapshot
                .summary
                .completeness
                .gap_refs
                .iter()
                .map(|value| value.0.as_str()),
        );
        let total = rows.len();
        let maximum = maximum.min(MAX_WORK_HISTORY_ROWS);
        rows.truncate(maximum);
        BoundedWorkHistory {
            truncated: total > rows.len(),
            omitted: total.saturating_sub(rows.len()),
            rows,
        }
    }

    pub fn journal(&self) -> WorkDetailJournal {
        WorkDetailJournal {
            schema: WORK_DETAIL_JOURNAL_SCHEMA_V1.to_string(),
            work_ref: self.snapshot.summary.work_ref.clone(),
            source_ref: self.snapshot.summary.source_authority.source_ref.clone(),
            presentation: self.presentation,
            selected_block_ref: self.selected_block_ref.clone(),
            records: self.records.clone(),
        }
    }
}

fn rejection(
    reason: WorkIntentRejection,
    detail: &str,
) -> Result<WorkIntentOutcome, WorkDetailError> {
    Ok(WorkIntentOutcome::Rejected {
        reason,
        detail: ShortText::try_from(detail.to_string())?,
    })
}

fn apply_operation(snapshot: &mut WorkSnapshot, operation: &WorkMutationOperation) {
    match operation {
        WorkMutationOperation::SetTitle { title } => snapshot.summary.title = title.clone(),
        WorkMutationOperation::SetDescription { description } => {
            snapshot.summary.description = description.clone();
        }
        WorkMutationOperation::SetState { state } => snapshot.summary.state = state.clone(),
        WorkMutationOperation::SetPriority { priority } => {
            snapshot.summary.priority = priority.clone();
        }
    }
}

fn same_intent_request(left: &WorkIntent, right: &WorkIntent) -> bool {
    left.work_ref == right.work_ref
        && left.actor_ref == right.actor_ref
        && left.source_ref == right.source_ref
        && left.expected_revision == right.expected_revision
        && left.target_generation == right.target_generation
        && left.idempotency_key == right.idempotency_key
        && left.operation == right.operation
}

fn validate_intent(intent: &WorkIntent) -> Result<(), WorkDetailError> {
    intent.intent_ref.validate()?;
    intent.work_ref.validate()?;
    intent.actor_ref.validate()?;
    intent.source_ref.validate()?;
    intent.expected_revision.validate()?;
    intent.target_generation.validate()?;
    intent.idempotency_key.validate()?;
    intent.submitted_at.validate()?;
    validate_operation(&intent.operation)
}

fn validate_event(event: &WorkCanonicalEvent) -> Result<(), WorkDetailError> {
    event.event_ref.validate()?;
    event.intent_ref.validate()?;
    event.work_ref.validate()?;
    event.source_ref.validate()?;
    event.previous_revision.validate()?;
    event.revision.validate()?;
    event.admitted_at.validate()?;
    validate_operation(&event.operation)
}

fn validate_operation(operation: &WorkMutationOperation) -> Result<(), WorkDetailError> {
    match operation {
        WorkMutationOperation::SetTitle { title } => title.validate()?,
        WorkMutationOperation::SetDescription { description } => description.validate()?,
        WorkMutationOperation::SetState { state } => state.validate()?,
        WorkMutationOperation::SetPriority { priority } => priority.validate()?,
    }
    Ok(())
}

fn validate_snapshot_identity(snapshot: &WorkSnapshot) -> Result<(), WorkDetailError> {
    snapshot.validate()?;
    if let Some(Some(issue)) = snapshot.issue.as_ref()
        && (issue.work_ref != snapshot.summary.work_ref
            || issue.revision != snapshot.summary.revision
            || issue.state != snapshot.summary.state)
    {
        return Err(WorkDetailError::IssueIdentityMismatch);
    }
    Ok(())
}

fn validate_journal(
    journal: &WorkDetailJournal,
    snapshot: &WorkSnapshot,
) -> Result<(), WorkDetailError> {
    if journal.schema != WORK_DETAIL_JOURNAL_SCHEMA_V1 {
        return Err(WorkDetailError::UnsupportedJournalSchema(
            journal.schema.clone(),
        ));
    }
    if journal.work_ref != snapshot.summary.work_ref
        || journal.source_ref != snapshot.summary.source_authority.source_ref
    {
        return Err(WorkDetailError::JournalIdentityMismatch);
    }
    if journal.records.len() > MAX_WORK_INTENT_RECORDS {
        return Err(WorkDetailError::TooManyIntents);
    }
    for record in &journal.records {
        validate_intent(&record.intent)?;
        if record.intent.work_ref != journal.work_ref
            || record.intent.source_ref != journal.source_ref
        {
            return Err(WorkDetailError::JournalIdentityMismatch);
        }
        if let WorkIntentOutcome::Accepted { revision, .. } = &record.outcome
            && revision.0 > snapshot.summary.revision.0
        {
            return Err(WorkDetailError::RevisionRegression);
        }
    }
    Ok(())
}

fn append_refs<'a>(
    rows: &mut Vec<WorkActivityRow>,
    seen: &mut HashSet<String>,
    kind: WorkActivityKind,
    refs: impl IntoIterator<Item = &'a str>,
) {
    for reference in refs {
        append_ref(rows, seen, kind, reference);
    }
}

fn append_ref(
    rows: &mut Vec<WorkActivityRow>,
    seen: &mut HashSet<String>,
    kind: WorkActivityKind,
    reference: &str,
) {
    let key = format!("{kind:?}:{reference}");
    if !seen.insert(key) {
        return;
    }
    rows.push(WorkActivityRow {
        sequence: rows.len().saturating_add(1),
        kind,
        reference: reference.to_string(),
        label: format!("{} · {reference}", kind.label()),
    });
}

fn journal_path(data_dir: &Path, work_ref: &WorkRef, source_ref: &SourceRef) -> PathBuf {
    let mut digest = Sha256::new();
    digest.update(work_ref.0.as_bytes());
    digest.update([0]);
    digest.update(source_ref.0.as_bytes());
    data_dir
        .join(JOURNAL_DIR)
        .join(format!("{:x}.json", digest.finalize()))
}

pub fn write_journal(data_dir: &Path, detail: &WorkDetail) -> Result<PathBuf, WorkDetailError> {
    let journal = detail.journal();
    write_journal_value(data_dir, &journal)
}

pub fn write_journal_value(
    data_dir: &Path,
    journal: &WorkDetailJournal,
) -> Result<PathBuf, WorkDetailError> {
    let path = journal_path(data_dir, &journal.work_ref, &journal.source_ref);
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::other("Work detail journal has no parent directory").into());
    };
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let suffix = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(".journal-{}-{suffix}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&journal)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temp_path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temp_path, &path)?;
        Ok::<(), std::io::Error>(())
    })();
    if result.is_err()
        && let Err(error) = fs::remove_file(&temp_path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(error.into());
    }
    result?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

pub fn read_journal(
    data_dir: &Path,
    work_ref: &WorkRef,
    source_ref: &SourceRef,
) -> Result<Option<WorkDetailJournal>, WorkDetailError> {
    let path = journal_path(data_dir, work_ref, source_ref);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let journal: WorkDetailJournal = serde_json::from_slice(&bytes)?;
    if journal.work_ref != *work_ref || journal.source_ref != *source_ref {
        return Err(WorkDetailError::JournalIdentityMismatch);
    }
    Ok(Some(journal))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use omega_effectd::all_work_contract::{
        Completeness, CompletenessState, ContractVersion, Freshness, HumanAssignee, Nullable,
        PrivacyClass, RedactionMetadata, SourceAuthority, SourceAuthorityKind, WorkClass,
        WorkDomain,
    };
    use omega_work_index::{AccountabilityKind, AttentionGroup};
    use proptest::prelude::*;
    use tempfile::tempdir;

    use super::*;

    fn text(value: &str) -> ShortText {
        ShortText::try_from(value.to_string()).expect("valid short text")
    }

    fn source(value: &str) -> SourceRef {
        SourceRef::try_from(value.to_string()).expect("valid source ref")
    }

    fn work(value: &str) -> WorkRef {
        WorkRef::try_from(value.to_string()).expect("valid Work ref")
    }

    fn timestamp(value: &str) -> IsoTimestamp {
        IsoTimestamp::try_from(value.to_string()).expect("valid timestamp")
    }

    fn item(writable: bool) -> WorkIndexItem {
        WorkIndexItem {
            summary: omega_effectd::all_work_contract::WorkSummary {
                contract_version: ContractVersion::OpenagentsAllWorkBoundaryV1,
                work_ref: work("work:omega:thread:abc"),
                title: text("Real thread Work"),
                description: Some(
                    LongText::try_from("Durable source description".to_string())
                        .expect("valid description"),
                ),
                domain: WorkDomain::General,
                work_class: WorkClass::Task,
                state: WorkState::Active,
                priority: WorkPriority::Normal,
                owner_ref: omega_effectd::all_work_contract::PrincipalRef::try_from(
                    "principal:omega:local-owner".to_string(),
                )
                .expect("valid owner"),
                assignee: Nullable(Some(HumanAssignee {
                    kind: omega_effectd::all_work_contract::AssigneeKind::Human,
                    principal_ref: omega_effectd::all_work_contract::PrincipalRef::try_from(
                        "principal:omega:local-owner".to_string(),
                    )
                    .expect("valid assignee"),
                })),
                agent_delegate: Some(None),
                portfolio: None,
                source_authority: SourceAuthority {
                    kind: SourceAuthorityKind::OmegaNative,
                    source_ref: source("thread:omega:abc"),
                    adapter_version: text("omega.thread-metadata.v1"),
                    writable,
                },
                revision: SafeInteger(7),
                updated_at: timestamp("2026-08-02T12:00:00Z"),
                freshness: Freshness {
                    state: FreshnessState::Fresh,
                    observed_at: timestamp("2026-08-02T12:00:01Z"),
                    source_updated_at: Some(Some(timestamp("2026-08-02T12:00:00Z"))),
                },
                completeness: Completeness {
                    state: CompletenessState::Complete,
                    cursor: None,
                    gap_refs: Vec::new(),
                },
                redaction: RedactionMetadata {
                    privacy_class: PrivacyClass::Private,
                    redacted_field_count: SafeInteger(0),
                    policy_ref: source("policy:omega:work-private-v1"),
                },
            },
            accountability: BTreeSet::from([
                AccountabilityKind::Owner,
                AccountabilityKind::Assignee,
            ]),
            attention: AttentionGroup::Active,
            source_entity: WorkSourceEntity::Thread {
                thread_ref: "abc".to_string(),
            },
        }
    }

    fn detail(writable: bool) -> WorkDetail {
        let item = item(writable);
        let snapshot = snapshot_from_index_item(
            &item,
            SnapshotLinks {
                issue_identifier: Some(text("OMEGA-ABC")),
                thread_refs: vec![
                    ThreadRef::try_from("thread:abc".to_string()).expect("valid Thread ref"),
                ],
                ..SnapshotLinks::default()
            },
        )
        .expect("valid snapshot");
        let capability = writable.then(|| WorkMutationCapability {
            adapter_version: text("omega.thread-metadata.v1"),
            source_ref: source("thread:omega:abc"),
            generation: SafeInteger(3),
            operations: BTreeSet::from([WorkMutationKind::Title]),
        });
        WorkDetail::new(
            snapshot,
            default_blocks(&item).expect("default Blocks"),
            capability,
            None,
        )
        .expect("valid detail")
    }

    fn intent(idempotency: &str, expected_revision: u64, generation: u64) -> WorkIntent {
        WorkIntent {
            intent_ref: IntentRef::try_from(format!("intent:omega:{idempotency}"))
                .expect("valid Intent ref"),
            work_ref: work("work:omega:thread:abc"),
            actor_ref: source("principal:omega:local-owner"),
            source_ref: source("thread:omega:abc"),
            expected_revision: SafeInteger(expected_revision),
            target_generation: SafeInteger(generation),
            idempotency_key: text(idempotency),
            submitted_at: timestamp("2026-08-02T12:00:02Z"),
            operation: WorkMutationOperation::SetTitle {
                title: text("Renamed through Work"),
            },
        }
    }

    fn event(intent: &WorkIntent) -> WorkCanonicalEvent {
        WorkCanonicalEvent {
            event_ref: EventRef::try_from("event:omega:title:8".to_string())
                .expect("valid Event ref"),
            intent_ref: intent.intent_ref.clone(),
            work_ref: intent.work_ref.clone(),
            source_ref: intent.source_ref.clone(),
            previous_revision: SafeInteger(7),
            revision: SafeInteger(8),
            admitted_at: timestamp("2026-08-02T12:00:03Z"),
            operation: intent.operation.clone(),
        }
    }

    #[test]
    fn work_and_issue_share_identity_revision_state_and_history() {
        let detail = detail(true);
        let issue = detail
            .snapshot()
            .issue
            .as_ref()
            .and_then(Option::as_ref)
            .expect("Issue projection");
        assert_eq!(issue.work_ref, detail.snapshot().summary.work_ref);
        assert_eq!(issue.revision, detail.snapshot().summary.revision);
        assert_eq!(issue.state, detail.snapshot().summary.state);
        assert_eq!(detail.history(MAX_WORK_HISTORY_ROWS).rows.len(), 1);
    }

    #[test]
    fn optimistic_intent_never_changes_confirmed_state_before_event_admission() {
        let mut detail = detail(true);
        let intent = intent("idempotency-title-1", 7, 3);
        assert_eq!(
            detail.submit_intent(intent.clone(), true).expect("submit"),
            SubmitIntentDisposition::Submitted
        );
        assert_eq!(detail.snapshot().summary.title.0, "Real thread Work");
        assert!(matches!(
            detail.intent_outcome(&intent.intent_ref),
            Some(WorkIntentOutcome::Pending)
        ));

        detail.admit_event(event(&intent)).expect("admit Event");
        assert_eq!(detail.snapshot().summary.title.0, "Renamed through Work");
        assert_eq!(detail.snapshot().summary.revision.0, 8);
        let issue = detail
            .snapshot()
            .issue
            .as_ref()
            .and_then(Option::as_ref)
            .expect("Issue projection");
        assert_eq!(issue.revision.0, 8);
        assert_eq!(detail.snapshot().intent_refs, vec![intent.intent_ref]);
        assert_eq!(detail.snapshot().event_refs.len(), 1);
    }

    #[test]
    fn idempotency_replay_returns_the_same_pending_outcome_without_duplicate() {
        let mut detail = detail(true);
        let mut replay = intent("idempotency-title-2", 7, 3);
        detail
            .submit_intent(replay.clone(), true)
            .expect("first submission");
        replay.intent_ref =
            IntentRef::try_from("intent:omega:replay".to_string()).expect("valid replay ref");
        replay.submitted_at = timestamp("2026-08-02T12:00:04Z");
        assert_eq!(
            detail.submit_intent(replay, true).expect("replay"),
            SubmitIntentDisposition::Reconciled(WorkIntentOutcome::Pending)
        );
        assert_eq!(detail.records().len(), 1);
    }

    #[test]
    fn rejected_stale_offline_and_conflict_outcomes_do_not_mutate_work() {
        let mut read_only = detail(false);
        let outcome = read_only
            .submit_intent(intent("read-only", 7, 3), true)
            .expect("read-only outcome");
        assert!(matches!(
            outcome,
            SubmitIntentDisposition::Reconciled(WorkIntentOutcome::Rejected { .. })
        ));

        let mut offline = detail(true);
        assert_eq!(
            offline
                .submit_intent(intent("offline", 7, 3), false)
                .expect("offline outcome"),
            SubmitIntentDisposition::Reconciled(WorkIntentOutcome::Offline)
        );

        let mut conflict = detail(true);
        assert_eq!(
            conflict
                .submit_intent(intent("conflict", 6, 3), true)
                .expect("conflict outcome"),
            SubmitIntentDisposition::Reconciled(WorkIntentOutcome::Conflict {
                current_revision: SafeInteger(7)
            })
        );

        let mut stale = detail(true);
        assert_eq!(
            stale
                .submit_intent(intent("stale", 7, 2), true)
                .expect("stale outcome"),
            SubmitIntentDisposition::Reconciled(WorkIntentOutcome::StaleGeneration {
                current_generation: SafeInteger(3)
            })
        );
        for detail in [&read_only, &offline, &conflict, &stale] {
            assert_eq!(detail.snapshot().summary.title.0, "Real thread Work");
            assert_eq!(detail.snapshot().summary.revision.0, 7);
        }
    }

    #[test]
    fn wrong_event_and_revision_fail_without_confirming_state() {
        let mut detail = detail(true);
        let intent = intent("wrong-event", 7, 3);
        detail
            .submit_intent(intent.clone(), true)
            .expect("pending intent");
        let mut wrong = event(&intent);
        wrong.revision = SafeInteger(9);
        assert!(matches!(
            detail.admit_event(wrong),
            Err(WorkDetailError::EventRevisionMismatch)
        ));
        assert_eq!(detail.snapshot().summary.title.0, "Real thread Work");
    }

    #[test]
    fn journal_round_trip_restores_view_selection_and_idempotency_without_copying_authority() {
        let directory = tempdir().expect("tempdir");
        let mut detail = detail(true);
        detail.set_presentation(WorkPresentation::Issue);
        let block_ref = detail.blocks()[1].block_ref.clone();
        assert!(detail.select_block(&block_ref));
        let intent = intent("journal", 7, 3);
        detail
            .submit_intent(intent.clone(), true)
            .expect("pending intent");
        detail.admit_event(event(&intent)).expect("admitted Event");
        let path = write_journal(directory.path(), &detail).expect("write journal");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&path)
                    .expect("journal metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let journal = read_journal(
            directory.path(),
            &detail.snapshot().summary.work_ref,
            &detail.snapshot().summary.source_authority.source_ref,
        )
        .expect("read journal")
        .expect("journal present");
        assert_eq!(journal.presentation, WorkPresentation::Issue);
        assert_eq!(journal.selected_block_ref, Some(block_ref));
        assert_eq!(journal.records.len(), 1);
        let encoded = serde_json::to_string(&journal).expect("encode journal");
        assert!(!encoded.contains("\"summary\""));
        assert!(!encoded.contains("sourceAuthority"));
    }

    #[test]
    fn history_is_deduplicated_bounded_and_reports_truncation_and_gaps() {
        let mut detail = detail(true);
        detail.snapshot.intent_refs = (0..4_096)
            .map(|index| {
                IntentRef::try_from(format!("intent:history:{index}")).expect("valid Intent ref")
            })
            .collect();
        detail.snapshot.event_refs = (0..4_096)
            .map(|index| {
                EventRef::try_from(format!("event:history:{index}")).expect("valid Event ref")
            })
            .collect();
        detail.snapshot.evidence_refs = (0..4_096)
            .map(|index| {
                EvidenceRef::try_from(format!("evidence:history:{index}"))
                    .expect("valid Evidence ref")
            })
            .collect();
        detail.snapshot.summary.completeness.gap_refs = vec![source("event:missing:1")];
        let started = Instant::now();
        let history = detail.history(MAX_WORK_HISTORY_ROWS);
        assert_eq!(history.rows.len(), MAX_WORK_HISTORY_ROWS);
        assert!(history.truncated);
        assert!(history.omitted > 0);
        assert!(started.elapsed().as_secs_f32() < 2.0);
    }

    proptest! {
        #[test]
        fn accepted_title_event_replay_is_deterministic(title in "[A-Za-z][A-Za-z0-9 ]{0,40}") {
            let mut left = detail(true);
            let mut right = detail(true);
            let mut request = intent("property", 7, 3);
            request.operation = WorkMutationOperation::SetTitle { title: text(&title) };
            let mut admitted = event(&request);
            admitted.operation = request.operation.clone();
            left.submit_intent(request.clone(), true).expect("left submit");
            right.submit_intent(request, true).expect("right submit");
            left.admit_event(admitted.clone()).expect("left event");
            right.admit_event(admitted).expect("right event");
            prop_assert_eq!(left.snapshot(), right.snapshot());
            prop_assert_eq!(left.records(), right.records());
        }
    }
}
