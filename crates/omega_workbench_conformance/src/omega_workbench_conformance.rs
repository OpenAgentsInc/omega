use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

pub const TRACE_SCHEMA_V1: &str = "openagents.omega.workbench-conformance.v1";
pub const MAX_TRACE_STEPS: usize = 100_000;
pub const MAX_THREADS: usize = 4_096;
pub const MAX_PENDING_LOADS: usize = 16_384;
pub const MAX_IDENTIFIER_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThreadId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepositoryId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorktreeId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceId {
    Files,
    Search,
    Review,
    Git,
    Terminal,
    Plan,
}

impl SurfaceId {
    pub const FALLBACK_PRIORITY: [Self; 6] = [
        Self::Files,
        Self::Search,
        Self::Review,
        Self::Git,
        Self::Terminal,
        Self::Plan,
    ];

    pub fn requires_binding(self) -> bool {
        matches!(
            self,
            Self::Files | Self::Search | Self::Review | Self::Git | Self::Terminal
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionPhase {
    Online,
    Offline,
    Reconnecting,
    StaleProjection,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub repository_id: RepositoryId,
    pub worktree_id: WorktreeId,
}

impl Binding {
    pub fn new(repository_id: impl Into<String>, worktree_id: impl Into<String>) -> Self {
        Self {
            repository_id: RepositoryId(repository_id.into()),
            worktree_id: WorktreeId(worktree_id.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadSeed {
    pub generation: u64,
    pub binding: Option<Binding>,
    pub available_surfaces: BTreeSet<SurfaceId>,
    pub requested_surface: Option<SurfaceId>,
    pub dock_visible: bool,
    pub artifact_revision: u64,
    pub event_revision: u64,
}

impl ThreadSeed {
    pub fn new(
        generation: u64,
        binding: Option<Binding>,
        available_surfaces: impl IntoIterator<Item = SurfaceId>,
    ) -> Self {
        Self {
            generation,
            binding,
            available_surfaces: available_surfaces.into_iter().collect(),
            requested_surface: None,
            dock_visible: false,
            artifact_revision: 0,
            event_revision: 0,
        }
    }

    fn into_state(self) -> ThreadState {
        ThreadState {
            generation: self.generation,
            binding: self.binding,
            available_surfaces: self.available_surfaces,
            requested_surface: self.requested_surface,
            effective_surface: None,
            dock_visible: self.dock_visible,
            focus_owner: None,
            artifact_revision: self.artifact_revision,
            event_revision: self.event_revision,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadState {
    pub generation: u64,
    pub binding: Option<Binding>,
    pub available_surfaces: BTreeSet<SurfaceId>,
    pub requested_surface: Option<SurfaceId>,
    pub effective_surface: Option<SurfaceId>,
    pub dock_visible: bool,
    pub focus_owner: Option<SurfaceId>,
    pub artifact_revision: u64,
    pub event_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingLoad {
    pub thread_id: ThreadId,
    pub surface: SurfaceId,
    pub generation: u64,
    pub binding: Option<Binding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedSelection {
    pub revision: u64,
    pub thread_id: ThreadId,
    pub generation: u64,
    pub binding: Option<Binding>,
    pub requested_surface: Option<SurfaceId>,
    pub dock_visible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisibleProjection {
    pub thread_id: ThreadId,
    pub generation: u64,
    pub binding: Option<Binding>,
    pub requested_surface: Option<SurfaceId>,
    pub effective_surface: Option<SurfaceId>,
    pub dock_visible: bool,
    pub focus_owner: Option<SurfaceId>,
    pub artifact_revision: u64,
    pub event_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkbenchState {
    pub projection_revision: u64,
    pub persistence_revision: u64,
    pub connection: ConnectionPhase,
    pub active_thread: Option<ThreadId>,
    pub threads: BTreeMap<ThreadId, ThreadState>,
    pub pending_loads: BTreeMap<RequestId, PendingLoad>,
    pub persisted_selection: Option<PersistedSelection>,
    pub restore_pending: bool,
    pub visible_projection: Option<VisibleProjection>,
}

impl WorkbenchState {
    pub fn empty() -> Self {
        Self {
            projection_revision: 0,
            persistence_revision: 0,
            connection: ConnectionPhase::Online,
            active_thread: None,
            threads: BTreeMap::new(),
            pending_loads: BTreeMap::new(),
            persisted_selection: None,
            restore_pending: false,
            visible_projection: None,
        }
    }

    pub fn expected_visible_projection(&self) -> Option<VisibleProjection> {
        let thread_id = self.active_thread.as_ref()?;
        let thread = self.threads.get(thread_id)?;
        Some(VisibleProjection {
            thread_id: thread_id.clone(),
            generation: thread.generation,
            binding: thread.binding.clone(),
            requested_surface: thread.requested_surface,
            effective_surface: thread.effective_surface,
            dock_visible: thread.dock_visible,
            focus_owner: thread.focus_owner,
            artifact_revision: thread.artifact_revision,
            event_revision: thread.event_revision,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotThread {
    pub thread_id: ThreadId,
    #[serde(flatten)]
    pub seed: ThreadSeed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionSnapshot {
    pub revision: u64,
    pub persistence_revision: u64,
    pub active_thread: Option<ThreadId>,
    pub threads: Vec<SnapshotThread>,
    pub persisted_selection: Option<PersistedSelection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    OpenThread,
    CloseThread,
    SwitchThread,
    RequestSurface,
    CloseSurface,
    CollapseDock,
    ExpandDock,
    BindRepository,
    ChangeWorktree,
    RemoveBinding,
    ChangeBinding,
    BeginSurfaceLoad,
    CompleteSurfaceLoad,
    FailSurfaceLoad,
    Disconnect,
    Reconnect,
    ReceiveProjectionSnapshot,
    PersistSelection,
    AdoptPersistedSelection,
    ColdStart,
    RestoreSelection,
    InvalidateCapability,
    DispatchSurfaceCommand,
}

impl ActionKind {
    pub const ALL: [Self; 23] = [
        Self::OpenThread,
        Self::CloseThread,
        Self::SwitchThread,
        Self::RequestSurface,
        Self::CloseSurface,
        Self::CollapseDock,
        Self::ExpandDock,
        Self::BindRepository,
        Self::ChangeWorktree,
        Self::RemoveBinding,
        Self::ChangeBinding,
        Self::BeginSurfaceLoad,
        Self::CompleteSurfaceLoad,
        Self::FailSurfaceLoad,
        Self::Disconnect,
        Self::Reconnect,
        Self::ReceiveProjectionSnapshot,
        Self::PersistSelection,
        Self::AdoptPersistedSelection,
        Self::ColdStart,
        Self::RestoreSelection,
        Self::InvalidateCapability,
        Self::DispatchSurfaceCommand,
    ];

    pub fn wire_name(self) -> &'static str {
        match self {
            Self::OpenThread => "open_thread",
            Self::CloseThread => "close_thread",
            Self::SwitchThread => "switch_thread",
            Self::RequestSurface => "request_surface",
            Self::CloseSurface => "close_surface",
            Self::CollapseDock => "collapse_dock",
            Self::ExpandDock => "expand_dock",
            Self::BindRepository => "bind_repository",
            Self::ChangeWorktree => "change_worktree",
            Self::RemoveBinding => "remove_binding",
            Self::ChangeBinding => "change_binding",
            Self::BeginSurfaceLoad => "begin_surface_load",
            Self::CompleteSurfaceLoad => "complete_surface_load",
            Self::FailSurfaceLoad => "fail_surface_load",
            Self::Disconnect => "disconnect",
            Self::Reconnect => "reconnect",
            Self::ReceiveProjectionSnapshot => "receive_projection_snapshot",
            Self::PersistSelection => "persist_selection",
            Self::AdoptPersistedSelection => "adopt_persisted_selection",
            Self::ColdStart => "cold_start",
            Self::RestoreSelection => "restore_selection",
            Self::InvalidateCapability => "invalidate_capability",
            Self::DispatchSurfaceCommand => "dispatch_surface_command",
        }
    }

    pub fn from_wire_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|action| action.wire_name() == name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transition {
    OpenThread {
        thread_id: ThreadId,
        seed: ThreadSeed,
    },
    CloseThread {
        thread_id: ThreadId,
    },
    SwitchThread {
        thread_id: ThreadId,
    },
    RequestSurface {
        thread_id: ThreadId,
        surface: SurfaceId,
    },
    CloseSurface {
        thread_id: ThreadId,
    },
    CollapseDock {
        thread_id: ThreadId,
    },
    ExpandDock {
        thread_id: ThreadId,
    },
    BindRepository {
        thread_id: ThreadId,
        generation: u64,
        binding: Binding,
        available_surfaces: BTreeSet<SurfaceId>,
    },
    ChangeWorktree {
        thread_id: ThreadId,
        generation: u64,
        worktree_id: WorktreeId,
        available_surfaces: BTreeSet<SurfaceId>,
    },
    RemoveBinding {
        thread_id: ThreadId,
        generation: u64,
        available_surfaces: BTreeSet<SurfaceId>,
    },
    ChangeBinding {
        thread_id: ThreadId,
        generation: u64,
        binding: Option<Binding>,
        available_surfaces: BTreeSet<SurfaceId>,
    },
    BeginSurfaceLoad {
        request_id: RequestId,
        thread_id: ThreadId,
        surface: SurfaceId,
        generation: u64,
        binding: Option<Binding>,
    },
    CompleteSurfaceLoad {
        request_id: RequestId,
        thread_id: ThreadId,
        surface: SurfaceId,
        generation: u64,
        binding: Option<Binding>,
    },
    FailSurfaceLoad {
        request_id: RequestId,
        thread_id: ThreadId,
        surface: SurfaceId,
        generation: u64,
        binding: Option<Binding>,
    },
    Disconnect,
    Reconnect,
    ReceiveProjectionSnapshot {
        snapshot: ProjectionSnapshot,
    },
    PersistSelection {
        revision: u64,
    },
    AdoptPersistedSelection {
        selection: PersistedSelection,
    },
    ColdStart,
    RestoreSelection,
    InvalidateCapability {
        thread_id: ThreadId,
        generation: u64,
        surface: SurfaceId,
    },
    DispatchSurfaceCommand {
        thread_id: ThreadId,
        surface: SurfaceId,
        generation: u64,
        binding: Option<Binding>,
    },
}

impl Transition {
    pub fn kind(&self) -> ActionKind {
        match self {
            Self::OpenThread { .. } => ActionKind::OpenThread,
            Self::CloseThread { .. } => ActionKind::CloseThread,
            Self::SwitchThread { .. } => ActionKind::SwitchThread,
            Self::RequestSurface { .. } => ActionKind::RequestSurface,
            Self::CloseSurface { .. } => ActionKind::CloseSurface,
            Self::CollapseDock { .. } => ActionKind::CollapseDock,
            Self::ExpandDock { .. } => ActionKind::ExpandDock,
            Self::BindRepository { .. } => ActionKind::BindRepository,
            Self::ChangeWorktree { .. } => ActionKind::ChangeWorktree,
            Self::RemoveBinding { .. } => ActionKind::RemoveBinding,
            Self::ChangeBinding { .. } => ActionKind::ChangeBinding,
            Self::BeginSurfaceLoad { .. } => ActionKind::BeginSurfaceLoad,
            Self::CompleteSurfaceLoad { .. } => ActionKind::CompleteSurfaceLoad,
            Self::FailSurfaceLoad { .. } => ActionKind::FailSurfaceLoad,
            Self::Disconnect => ActionKind::Disconnect,
            Self::Reconnect => ActionKind::Reconnect,
            Self::ReceiveProjectionSnapshot { .. } => ActionKind::ReceiveProjectionSnapshot,
            Self::PersistSelection { .. } => ActionKind::PersistSelection,
            Self::AdoptPersistedSelection { .. } => ActionKind::AdoptPersistedSelection,
            Self::ColdStart => ActionKind::ColdStart,
            Self::RestoreSelection => ActionKind::RestoreSelection,
            Self::InvalidateCapability { .. } => ActionKind::InvalidateCapability,
            Self::DispatchSurfaceCommand { .. } => ActionKind::DispatchSurfaceCommand,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceStep {
    pub sequence: u64,
    #[serde(flatten)]
    pub transition: Transition,
    pub observed_effect: TransitionEffect,
    pub observed_state: WorkbenchState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionEffect {
    Applied,
    StaleCompletionIgnored,
    OlderRevisionIgnored,
    DeterministicFallback,
    Rejected { code: RejectCode },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectCode {
    UnknownThread,
    DuplicateThread,
    InactiveThread,
    UnknownRequest,
    DuplicateRequest,
    UnavailableSurface,
    InvalidConnectionPhase,
    NoActiveSelection,
    NoPersistedSelection,
    CommandBindingMismatch,
    StaleGeneration,
    RequestContextMismatch,
    RevisionOverflow,
    InvalidIdentifier,
    InvalidBinding,
    InvalidSnapshot,
    AlreadyBound,
    AlreadyUnbound,
    CapabilityAlreadyUnavailable,
    RestoreNotPending,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConformanceTrace {
    pub schema: String,
    pub required_actions: BTreeSet<ActionKind>,
    pub initial_state: WorkbenchState,
    pub steps: Vec<TraceStep>,
}

impl ConformanceTrace {
    pub fn new(initial_state: WorkbenchState) -> Self {
        Self {
            schema: TRACE_SCHEMA_V1.to_string(),
            required_actions: BTreeSet::new(),
            initial_state,
            steps: Vec::new(),
        }
    }

    pub fn require(mut self, actions: impl IntoIterator<Item = ActionKind>) -> Self {
        self.required_actions.extend(actions);
        self
    }

    pub fn push(&mut self, transition: Transition, observed_state: WorkbenchState) {
        self.push_with_effect(transition, TransitionEffect::Applied, observed_state);
    }

    pub fn push_with_effect(
        &mut self,
        transition: Transition,
        observed_effect: TransitionEffect,
        observed_state: WorkbenchState,
    ) {
        self.steps.push(TraceStep {
            sequence: self.steps.len() as u64 + 1,
            transition,
            observed_effect,
            observed_state,
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransitionOutcome {
    pub effect: TransitionEffect,
    pub state: WorkbenchState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConformanceReport {
    pub steps_checked: usize,
    pub seen_actions: BTreeSet<ActionKind>,
    pub final_state: WorkbenchState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    MalformedJson,
    MalformedTrace,
    UnsupportedSchema,
    UnknownCriticalAction,
    TraceLimitExceeded,
    InvalidIdentifier,
    MissingRequiredAction,
    SequenceMismatch,
    IllegalTransition,
    StaleGeneration,
    StaleBinding,
    RevisionRollback,
    CrossThreadState,
    EffectMismatch,
    StateMismatch,
    CoverageBreach,
    InvariantViolation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConformanceError {
    pub code: ErrorCode,
    pub reject_code: Option<RejectCode>,
    pub step_index: Option<usize>,
    pub detail: String,
}

impl ConformanceError {
    fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            reject_code: None,
            step_index: None,
            detail: detail.into(),
        }
    }

    fn rejected(code: RejectCode, detail: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::IllegalTransition,
            reject_code: Some(code),
            step_index: None,
            detail: detail.into(),
        }
    }

    fn at_step(mut self, step_index: usize) -> Self {
        self.step_index = Some(step_index);
        self
    }
}

impl fmt::Display for ConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(step_index) = self.step_index {
            write!(
                formatter,
                "{:?} at trace step {}: {}",
                self.code,
                step_index + 1,
                self.detail
            )
        } else {
            write!(formatter, "{:?}: {}", self.code, self.detail)
        }
    }
}

impl Error for ConformanceError {}

pub fn check_json(input: &[u8]) -> Result<ConformanceReport, ConformanceError> {
    let value: serde_json::Value = serde_json::from_slice(input).map_err(|error| {
        ConformanceError::new(ErrorCode::MalformedJson, format!("invalid JSON: {error}"))
    })?;
    preflight_wire_actions(&value)?;
    let trace: ConformanceTrace = serde_json::from_value(value).map_err(|error| {
        ConformanceError::new(
            ErrorCode::MalformedTrace,
            format!("invalid v1 trace payload: {error}"),
        )
    })?;
    check_trace(&trace)
}

pub fn check_trace(trace: &ConformanceTrace) -> Result<ConformanceReport, ConformanceError> {
    if trace.schema != TRACE_SCHEMA_V1 {
        return Err(ConformanceError::new(
            ErrorCode::UnsupportedSchema,
            format!("unsupported trace schema {:?}", trace.schema),
        ));
    }
    if trace.steps.len() > MAX_TRACE_STEPS {
        return Err(ConformanceError::new(
            ErrorCode::TraceLimitExceeded,
            format!(
                "trace has {} steps, maximum is {MAX_TRACE_STEPS}",
                trace.steps.len()
            ),
        ));
    }
    if trace.steps.is_empty() {
        return Err(ConformanceError::new(
            ErrorCode::CoverageBreach,
            "a conformance trace must contain at least one critical transition",
        ));
    }
    validate_state(&trace.initial_state)?;

    let mut model = trace.initial_state.clone();
    let mut seen_actions = BTreeSet::new();
    for (step_index, step) in trace.steps.iter().enumerate() {
        let expected_sequence = step_index as u64 + 1;
        if step.sequence != expected_sequence {
            return Err(ConformanceError::new(
                ErrorCode::SequenceMismatch,
                format!(
                    "expected sequence {expected_sequence}, observed {}",
                    step.sequence
                ),
            )
            .at_step(step_index));
        }
        let effect = apply_transition(&mut model, &step.transition)
            .map_err(|error| error.at_step(step_index))?;
        validate_state(&model).map_err(|error| error.at_step(step_index))?;
        validate_state(&step.observed_state).map_err(|error| error.at_step(step_index))?;
        if step.observed_effect != effect {
            return Err(ConformanceError::new(
                ErrorCode::EffectMismatch,
                format!(
                    "observed effect {:?} does not equal independent effect {effect:?}",
                    step.observed_effect
                ),
            )
            .at_step(step_index));
        }
        if step.observed_state != model {
            return Err(ConformanceError::new(
                ErrorCode::StateMismatch,
                "observed reducer state does not equal the independent transition result",
            )
            .at_step(step_index));
        }
        seen_actions.insert(step.transition.kind());
    }

    let missing: Vec<_> = trace
        .required_actions
        .difference(&seen_actions)
        .map(|action| action.wire_name())
        .collect();
    if !missing.is_empty() {
        return Err(ConformanceError::new(
            ErrorCode::MissingRequiredAction,
            format!(
                "trace did not cover required actions: {}",
                missing.join(", ")
            ),
        ));
    }

    Ok(ConformanceReport {
        steps_checked: trace.steps.len(),
        seen_actions,
        final_state: model,
    })
}

pub fn replay_transition(
    state: &WorkbenchState,
    transition: &Transition,
) -> Result<TransitionOutcome, ConformanceError> {
    validate_state(state)?;
    let mut next = state.clone();
    let effect = apply_transition(&mut next, transition)?;
    validate_state(&next)?;
    Ok(TransitionOutcome {
        effect,
        state: next,
    })
}

fn preflight_wire_actions(value: &serde_json::Value) -> Result<(), ConformanceError> {
    let object = value.as_object().ok_or_else(|| {
        ConformanceError::new(
            ErrorCode::MalformedTrace,
            "trace root must be a JSON object",
        )
    })?;
    let schema = object
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ConformanceError::new(ErrorCode::MalformedTrace, "trace schema must be a string")
        })?;
    if schema != TRACE_SCHEMA_V1 {
        return Err(ConformanceError::new(
            ErrorCode::UnsupportedSchema,
            format!("unsupported trace schema {schema:?}"),
        ));
    }

    if let Some(required_actions) = object.get("required_actions") {
        let required_actions = required_actions.as_array().ok_or_else(|| {
            ConformanceError::new(
                ErrorCode::MalformedTrace,
                "required_actions must be an array",
            )
        })?;
        for required_action in required_actions {
            let name = required_action.as_str().ok_or_else(|| {
                ConformanceError::new(
                    ErrorCode::MalformedTrace,
                    "required action names must be strings",
                )
            })?;
            if ActionKind::from_wire_name(name).is_none() {
                return Err(ConformanceError::new(
                    ErrorCode::UnknownCriticalAction,
                    format!("unknown required action {name:?}"),
                ));
            }
        }
    }

    let steps = object
        .get("steps")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            ConformanceError::new(ErrorCode::MalformedTrace, "steps must be an array")
        })?;
    if steps.len() > MAX_TRACE_STEPS {
        return Err(ConformanceError::new(
            ErrorCode::TraceLimitExceeded,
            format!("trace has {} steps", steps.len()),
        ));
    }
    for (step_index, step) in steps.iter().enumerate() {
        let kind = step
            .as_object()
            .and_then(|step| step.get("kind"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ConformanceError::new(
                    ErrorCode::MalformedTrace,
                    "every step must contain a string kind",
                )
                .at_step(step_index)
            })?;
        if ActionKind::from_wire_name(kind).is_none() {
            return Err(ConformanceError::new(
                ErrorCode::UnknownCriticalAction,
                format!("unknown critical action {kind:?}"),
            )
            .at_step(step_index));
        }
    }
    Ok(())
}

fn apply_transition(
    state: &mut WorkbenchState,
    transition: &Transition,
) -> Result<TransitionEffect, ConformanceError> {
    let mut candidate = state.clone();
    match try_apply_transition(&mut candidate, transition) {
        Ok(effect) => {
            *state = candidate;
            Ok(effect)
        }
        Err(error) => match error.reject_code {
            Some(code) => Ok(TransitionEffect::Rejected { code }),
            None => Err(error),
        },
    }
}

fn try_apply_transition(
    state: &mut WorkbenchState,
    transition: &Transition,
) -> Result<TransitionEffect, ConformanceError> {
    let effect = match transition {
        Transition::OpenThread { thread_id, seed } => {
            validate_transition_identifier(validate_thread_id(thread_id))?;
            if state.threads.contains_key(thread_id) {
                return rejected(
                    RejectCode::DuplicateThread,
                    format!("thread {:?} is already open", thread_id.0),
                );
            }
            let mut thread = seed.clone().into_state();
            reconcile_selection(&mut thread).map_err(as_invalid_binding)?;
            state.threads.insert(thread_id.clone(), thread);
            if state.active_thread.is_none() {
                state.active_thread = Some(thread_id.clone());
            }
            TransitionEffect::Applied
        }
        Transition::CloseThread { thread_id } => {
            validate_transition_identifier(validate_thread_id(thread_id))?;
            if state.threads.remove(thread_id).is_none() {
                return rejected(
                    RejectCode::UnknownThread,
                    format!("cannot close unknown thread {:?}", thread_id.0),
                );
            }
            state
                .pending_loads
                .retain(|_, pending| pending.thread_id != *thread_id);
            if state.active_thread.as_ref() == Some(thread_id) {
                state.active_thread = state.threads.keys().next().cloned();
                TransitionEffect::DeterministicFallback
            } else {
                TransitionEffect::Applied
            }
        }
        Transition::SwitchThread { thread_id } => {
            require_thread(state, thread_id)?;
            state.active_thread = Some(thread_id.clone());
            TransitionEffect::Applied
        }
        Transition::RequestSurface { thread_id, surface } => {
            require_active_thread(state, thread_id)?;
            if *surface != SurfaceId::Plan {
                require_online(state)?;
            }
            let thread = require_thread_mut(state, thread_id)?;
            if !thread.available_surfaces.contains(surface) {
                return rejected(
                    RejectCode::UnavailableSurface,
                    format!(
                        "surface {surface:?} is unavailable for thread {:?}",
                        thread_id.0
                    ),
                );
            }
            thread.requested_surface = Some(*surface);
            thread.effective_surface = Some(*surface);
            thread.dock_visible = true;
            TransitionEffect::Applied
        }
        Transition::CloseSurface { thread_id } => {
            require_active_thread(state, thread_id)?;
            let thread = require_thread_mut(state, thread_id)?;
            thread.requested_surface = None;
            thread.effective_surface = None;
            thread.dock_visible = false;
            TransitionEffect::Applied
        }
        Transition::CollapseDock { thread_id } => {
            require_active_thread(state, thread_id)?;
            let thread = require_thread_mut(state, thread_id)?;
            thread.dock_visible = false;
            TransitionEffect::Applied
        }
        Transition::ExpandDock { thread_id } => {
            require_active_thread(state, thread_id)?;
            if require_thread(state, thread_id)?.effective_surface != Some(SurfaceId::Plan) {
                require_online(state)?;
            }
            let thread = require_thread_mut(state, thread_id)?;
            reconcile_selection(thread)?;
            if thread.effective_surface.is_some() {
                thread.dock_visible = true;
                TransitionEffect::Applied
            } else {
                thread.dock_visible = false;
                TransitionEffect::DeterministicFallback
            }
        }
        Transition::BindRepository {
            thread_id,
            generation,
            binding,
            available_surfaces,
        } => {
            validate_transition_identifier(validate_binding(binding))?;
            let thread = require_thread_mut(state, thread_id)?;
            require_generation(thread, *generation)?;
            if thread.binding.is_some() {
                return rejected(
                    RejectCode::AlreadyBound,
                    format!("thread {:?} is already bound", thread_id.0),
                );
            }
            thread.generation = next_generation(*generation)?;
            thread.binding = Some(binding.clone());
            thread.available_surfaces = available_surfaces.clone();
            let previous_selection = thread.effective_surface;
            reconcile_selection(thread).map_err(as_invalid_binding)?;
            if thread.effective_surface != previous_selection {
                TransitionEffect::DeterministicFallback
            } else {
                TransitionEffect::Applied
            }
        }
        Transition::ChangeWorktree {
            thread_id,
            generation,
            worktree_id,
            available_surfaces,
        } => {
            validate_transition_identifier(validate_identifier("worktree", &worktree_id.0))?;
            let thread = require_thread_mut(state, thread_id)?;
            require_generation(thread, *generation)?;
            let binding = thread.binding.as_mut().ok_or_else(|| {
                ConformanceError::rejected(
                    RejectCode::AlreadyUnbound,
                    format!("thread {:?} has no repository binding", thread_id.0),
                )
            })?;
            binding.worktree_id = worktree_id.clone();
            thread.generation = next_generation(*generation)?;
            thread.available_surfaces = available_surfaces.clone();
            let previous_selection = thread.effective_surface;
            reconcile_selection(thread).map_err(as_invalid_binding)?;
            if thread.effective_surface != previous_selection {
                TransitionEffect::DeterministicFallback
            } else {
                TransitionEffect::Applied
            }
        }
        Transition::RemoveBinding {
            thread_id,
            generation,
            available_surfaces,
        } => {
            let thread = require_thread_mut(state, thread_id)?;
            require_generation(thread, *generation)?;
            if thread.binding.is_none() {
                return rejected(
                    RejectCode::AlreadyUnbound,
                    format!("thread {:?} is already unbound", thread_id.0),
                );
            }
            thread.generation = next_generation(*generation)?;
            thread.binding = None;
            thread.available_surfaces = available_surfaces.clone();
            let previous_selection = thread.effective_surface;
            reconcile_selection(thread).map_err(as_invalid_binding)?;
            if thread.effective_surface != previous_selection {
                TransitionEffect::DeterministicFallback
            } else {
                TransitionEffect::Applied
            }
        }
        Transition::ChangeBinding {
            thread_id,
            generation,
            binding,
            available_surfaces,
        } => {
            if let Some(binding) = binding {
                validate_transition_identifier(validate_binding(binding))?;
            }
            let thread = require_thread_mut(state, thread_id)?;
            require_generation(thread, *generation)?;
            let previous_selection = thread.effective_surface;
            thread.binding = binding.clone();
            thread.generation = next_generation(*generation)?;
            thread.available_surfaces = available_surfaces.clone();
            reconcile_selection(thread).map_err(as_invalid_binding)?;
            if thread.effective_surface != previous_selection {
                TransitionEffect::DeterministicFallback
            } else {
                TransitionEffect::Applied
            }
        }
        Transition::BeginSurfaceLoad {
            request_id,
            thread_id,
            surface,
            generation,
            binding,
        } => {
            validate_transition_identifier(validate_request_id(request_id))?;
            if state.pending_loads.contains_key(request_id) {
                return rejected(
                    RejectCode::DuplicateRequest,
                    format!("surface load request {:?} already exists", request_id.0),
                );
            }
            validate_load_context(state, thread_id, *surface, *generation, binding)?;
            state.pending_loads.insert(
                request_id.clone(),
                PendingLoad {
                    thread_id: thread_id.clone(),
                    surface: *surface,
                    generation: *generation,
                    binding: binding.clone(),
                },
            );
            TransitionEffect::Applied
        }
        Transition::CompleteSurfaceLoad {
            request_id,
            thread_id,
            surface,
            generation,
            binding,
        } => finish_surface_load(state, request_id, thread_id, *surface, *generation, binding)?,
        Transition::FailSurfaceLoad {
            request_id,
            thread_id,
            surface,
            generation,
            binding,
        } => finish_surface_load(state, request_id, thread_id, *surface, *generation, binding)?,
        Transition::Disconnect => {
            if !matches!(
                state.connection,
                ConnectionPhase::Online | ConnectionPhase::StaleProjection
            ) {
                return rejected(
                    RejectCode::InvalidConnectionPhase,
                    format!(
                        "disconnect requires online or stale state, observed {:?}",
                        state.connection
                    ),
                );
            }
            state.connection = ConnectionPhase::Offline;
            TransitionEffect::Applied
        }
        Transition::Reconnect => {
            if state.connection != ConnectionPhase::Offline {
                return rejected(
                    RejectCode::InvalidConnectionPhase,
                    format!(
                        "reconnect requires offline state, observed {:?}",
                        state.connection
                    ),
                );
            }
            state.connection = ConnectionPhase::Reconnecting;
            TransitionEffect::Applied
        }
        Transition::ReceiveProjectionSnapshot { snapshot } => apply_snapshot(state, snapshot)?,
        Transition::PersistSelection { revision } => {
            if *revision <= state.persistence_revision {
                TransitionEffect::OlderRevisionIgnored
            } else {
                let thread_id = state.active_thread.clone().ok_or_else(|| {
                    ConformanceError::rejected(
                        RejectCode::NoActiveSelection,
                        "cannot persist without an active thread",
                    )
                })?;
                let thread = require_thread(state, &thread_id)?;
                let generation = thread.generation;
                let binding = thread.binding.clone();
                let requested_surface = thread.requested_surface;
                let dock_visible = thread.dock_visible;
                state.persistence_revision = *revision;
                state.persisted_selection = Some(PersistedSelection {
                    revision: *revision,
                    thread_id,
                    generation,
                    binding,
                    requested_surface,
                    dock_visible,
                });
                TransitionEffect::Applied
            }
        }
        Transition::AdoptPersistedSelection { selection } => {
            validate_transition_identifier(validate_thread_id(&selection.thread_id))?;
            if let Some(binding) = &selection.binding {
                validate_transition_identifier(validate_binding(binding))?;
            }
            if selection.dock_visible && selection.requested_surface.is_none() {
                return rejected(
                    RejectCode::InvalidBinding,
                    "persisted open dock has no requested surface",
                );
            }
            // A durable record adopts at the higher of its own revision and
            // this session's persistence revision, so adoption never installs
            // a selection whose revision disagrees with the state revision.
            let revision = selection.revision.max(1).max(state.persistence_revision);
            if state
                .persisted_selection
                .as_ref()
                .is_some_and(|current| current.revision >= revision)
            {
                TransitionEffect::OlderRevisionIgnored
            } else {
                state.persistence_revision = revision;
                state.persisted_selection = Some(PersistedSelection {
                    revision,
                    ..selection.clone()
                });
                TransitionEffect::Applied
            }
        }
        Transition::ColdStart => {
            state.active_thread = None;
            state.pending_loads.clear();
            for thread in state.threads.values_mut() {
                thread.dock_visible = false;
                thread.focus_owner = None;
            }
            state.restore_pending = state.persisted_selection.is_some();
            TransitionEffect::Applied
        }
        Transition::RestoreSelection => restore_selection(state)?,
        Transition::InvalidateCapability {
            thread_id,
            generation,
            surface,
        } => {
            let thread = require_thread_mut(state, thread_id)?;
            require_generation(thread, *generation)?;
            if !thread.available_surfaces.remove(surface) {
                return rejected(
                    RejectCode::CapabilityAlreadyUnavailable,
                    format!(
                        "cannot invalidate unavailable surface {surface:?} for thread {:?}",
                        thread_id.0
                    ),
                );
            }
            let previous_selection = thread.effective_surface;
            thread.generation = next_generation(*generation)?;
            reconcile_selection(thread).map_err(as_invalid_binding)?;
            if thread.effective_surface != previous_selection {
                TransitionEffect::DeterministicFallback
            } else {
                TransitionEffect::Applied
            }
        }
        Transition::DispatchSurfaceCommand {
            thread_id,
            surface,
            generation,
            binding,
        } => {
            require_online(state)?;
            require_active_thread(state, thread_id)?;
            let thread = require_thread(state, thread_id)?;
            require_generation(thread, *generation)?;
            if thread.binding != *binding {
                return rejected(
                    RejectCode::CommandBindingMismatch,
                    format!(
                        "command binding does not match active thread {:?}",
                        thread_id.0
                    ),
                );
            }
            if !thread.dock_visible
                || thread.effective_surface != Some(*surface)
                || thread.focus_owner != Some(*surface)
            {
                return rejected(
                    RejectCode::UnavailableSurface,
                    format!(
                        "surface {surface:?} is not the visible focused owner for thread {:?}",
                        thread_id.0
                    ),
                );
            }
            TransitionEffect::Applied
        }
    };
    normalize_state(state)?;
    Ok(effect)
}

fn apply_snapshot(
    state: &mut WorkbenchState,
    snapshot: &ProjectionSnapshot,
) -> Result<TransitionEffect, ConformanceError> {
    if !matches!(
        state.connection,
        ConnectionPhase::Reconnecting | ConnectionPhase::StaleProjection
    ) {
        return rejected(
            RejectCode::InvalidConnectionPhase,
            format!(
                "projection snapshot requires reconnecting or stale state, observed {:?}",
                state.connection
            ),
        );
    }
    validate_snapshot_shape(snapshot)?;
    if snapshot.revision <= state.projection_revision
        || snapshot.persistence_revision < state.persistence_revision
    {
        state.connection = ConnectionPhase::StaleProjection;
        return Ok(TransitionEffect::OlderRevisionIgnored);
    }

    let mut threads = BTreeMap::new();
    let mut used_fallback = false;
    for snapshot_thread in &snapshot.threads {
        let mut thread = snapshot_thread.seed.clone().into_state();
        let requested_surface = thread.requested_surface;
        reconcile_selection(&mut thread)?;
        used_fallback |= requested_surface != thread.effective_surface;
        if threads
            .insert(snapshot_thread.thread_id.clone(), thread)
            .is_some()
        {
            return rejected(
                RejectCode::InvalidSnapshot,
                format!(
                    "snapshot contains duplicate thread {:?}",
                    snapshot_thread.thread_id.0
                ),
            );
        }
    }
    if let Some(active_thread) = &snapshot.active_thread
        && !threads.contains_key(active_thread)
    {
        return rejected(
            RejectCode::InvalidSnapshot,
            format!("snapshot activates unknown thread {:?}", active_thread.0),
        );
    }

    state.projection_revision = snapshot.revision;
    state.persistence_revision = snapshot.persistence_revision;
    state.active_thread = snapshot.active_thread.clone();
    state.threads = threads;
    state.pending_loads.clear();
    state.persisted_selection = snapshot.persisted_selection.clone();
    state.restore_pending = false;
    state.connection = ConnectionPhase::Online;
    Ok(if used_fallback {
        TransitionEffect::DeterministicFallback
    } else {
        TransitionEffect::Applied
    })
}

fn restore_selection(state: &mut WorkbenchState) -> Result<TransitionEffect, ConformanceError> {
    if !state.restore_pending {
        return rejected(
            RejectCode::RestoreNotPending,
            "restore_selection requires a preceding cold_start",
        );
    }
    require_online(state)?;

    let persisted = state.persisted_selection.clone();
    if persisted.is_none() {
        return rejected(
            RejectCode::NoPersistedSelection,
            "restore_selection has no persisted selection",
        );
    }
    let valid_persisted = persisted.as_ref().is_some_and(|selection| {
        state
            .threads
            .get(&selection.thread_id)
            .is_some_and(|thread| {
                thread.generation == selection.generation
                    && thread.binding == selection.binding
                    && selection
                        .requested_surface
                        .is_none_or(|surface| thread.available_surfaces.contains(&surface))
            })
    });

    let effect = if valid_persisted {
        if let Some(selection) = persisted {
            state.active_thread = Some(selection.thread_id.clone());
            if let Some(thread) = state.threads.get_mut(&selection.thread_id) {
                thread.requested_surface = selection.requested_surface;
                reconcile_selection(thread)?;
                thread.dock_visible = selection.dock_visible && thread.effective_surface.is_some();
            }
        }
        TransitionEffect::Applied
    } else {
        if state
            .active_thread
            .as_ref()
            .is_none_or(|thread_id| !state.threads.contains_key(thread_id))
        {
            state.active_thread = state.threads.keys().next().cloned();
        }
        if let Some(thread_id) = state.active_thread.clone()
            && let Some(thread) = state.threads.get_mut(&thread_id)
        {
            thread.requested_surface = fallback_surface(&thread.available_surfaces);
            reconcile_selection(thread)?;
            thread.dock_visible = persisted
                .as_ref()
                .is_some_and(|selection| selection.dock_visible)
                && thread.effective_surface.is_some();
        }
        TransitionEffect::DeterministicFallback
    };
    state.restore_pending = false;
    normalize_state(state)?;

    state.persisted_selection = match state.active_thread.clone() {
        Some(thread_id) => {
            let thread = require_thread(state, &thread_id)?;
            Some(PersistedSelection {
                revision: state.persistence_revision,
                thread_id,
                generation: thread.generation,
                binding: thread.binding.clone(),
                requested_surface: thread.requested_surface,
                dock_visible: thread.dock_visible,
            })
        }
        None => None,
    };
    Ok(effect)
}

fn finish_surface_load(
    state: &mut WorkbenchState,
    request_id: &RequestId,
    thread_id: &ThreadId,
    surface: SurfaceId,
    generation: u64,
    binding: &Option<Binding>,
) -> Result<TransitionEffect, ConformanceError> {
    validate_transition_identifier(validate_request_id(request_id))?;
    validate_transition_identifier(validate_thread_id(thread_id))?;
    if let Some(binding) = binding {
        validate_binding(binding).map_err(as_invalid_binding)?;
    }

    let pending = state
        .pending_loads
        .get(request_id)
        .cloned()
        .ok_or_else(|| {
            ConformanceError::rejected(
                RejectCode::UnknownRequest,
                format!("surface load request {:?} does not exist", request_id.0),
            )
        })?;
    if pending.thread_id != *thread_id
        || pending.surface != surface
        || pending.generation != generation
        || pending.binding != *binding
    {
        return rejected(
            RejectCode::RequestContextMismatch,
            format!(
                "surface load completion does not match request {:?}",
                request_id.0
            ),
        );
    }

    let current = state.threads.get(thread_id);
    let is_stale = current.is_none_or(|thread| {
        thread.generation != generation
            || thread.binding != *binding
            || !thread.available_surfaces.contains(&surface)
    });
    state.pending_loads.remove(request_id);
    if is_stale {
        return Ok(TransitionEffect::StaleCompletionIgnored);
    }
    Ok(TransitionEffect::Applied)
}

fn validate_snapshot_shape(snapshot: &ProjectionSnapshot) -> Result<(), ConformanceError> {
    if snapshot.threads.len() > MAX_THREADS {
        return rejected(
            RejectCode::InvalidSnapshot,
            format!("snapshot has {} threads", snapshot.threads.len()),
        );
    }
    if snapshot
        .persisted_selection
        .as_ref()
        .is_some_and(|selection| selection.revision != snapshot.persistence_revision)
    {
        return rejected(
            RejectCode::InvalidSnapshot,
            "snapshot persistence revision disagrees with its selection",
        );
    }
    if let Some(active_thread) = &snapshot.active_thread {
        validate_snapshot_identifier(validate_thread_id(active_thread))?;
    }
    if let Some(selection) = &snapshot.persisted_selection {
        validate_snapshot_identifier(validate_thread_id(&selection.thread_id))?;
        if let Some(binding) = &selection.binding {
            validate_snapshot_identifier(validate_binding(binding))?;
        }
    }

    let mut thread_ids = BTreeSet::new();
    for snapshot_thread in &snapshot.threads {
        validate_snapshot_identifier(validate_thread_id(&snapshot_thread.thread_id))?;
        if !thread_ids.insert(snapshot_thread.thread_id.clone()) {
            return rejected(
                RejectCode::InvalidSnapshot,
                format!(
                    "snapshot contains duplicate thread {:?}",
                    snapshot_thread.thread_id.0
                ),
            );
        }
        if let Some(binding) = &snapshot_thread.seed.binding {
            validate_snapshot_identifier(validate_binding(binding))?;
        }
        validate_binding_availability(
            snapshot_thread.seed.binding.as_ref(),
            &snapshot_thread.seed.available_surfaces,
        )
        .map_err(|error| ConformanceError::rejected(RejectCode::InvalidSnapshot, error.detail))?;
    }
    if let Some(active_thread) = &snapshot.active_thread
        && !thread_ids.contains(active_thread)
    {
        return rejected(
            RejectCode::InvalidSnapshot,
            format!("snapshot activates unknown thread {:?}", active_thread.0),
        );
    }
    Ok(())
}

fn validate_load_context(
    state: &WorkbenchState,
    thread_id: &ThreadId,
    surface: SurfaceId,
    generation: u64,
    binding: &Option<Binding>,
) -> Result<(), ConformanceError> {
    let thread = require_thread(state, thread_id)?;
    require_generation(thread, generation)?;
    if thread.binding != *binding {
        return rejected(
            RejectCode::InvalidBinding,
            format!(
                "surface load binding does not match thread {:?}",
                thread_id.0
            ),
        );
    }
    if !thread.available_surfaces.contains(&surface) {
        return rejected(
            RejectCode::UnavailableSurface,
            format!(
                "surface {surface:?} is unavailable for thread {:?}",
                thread_id.0
            ),
        );
    }
    Ok(())
}

fn normalize_state(state: &mut WorkbenchState) -> Result<(), ConformanceError> {
    for thread in state.threads.values_mut() {
        thread.focus_owner = None;
    }
    if state.connection == ConnectionPhase::Online
        && let Some(thread_id) = state.active_thread.as_ref()
        && let Some(thread) = state.threads.get_mut(thread_id)
        && thread.dock_visible
    {
        thread.focus_owner = thread.effective_surface;
    }
    state.visible_projection = state.expected_visible_projection();
    Ok(())
}

fn reconcile_selection(thread: &mut ThreadState) -> Result<(), ConformanceError> {
    validate_binding_availability(thread.binding.as_ref(), &thread.available_surfaces)?;
    let effective = match thread.requested_surface {
        Some(requested) if thread.available_surfaces.contains(&requested) => Some(requested),
        Some(_) => fallback_surface(&thread.available_surfaces),
        None => None,
    };
    thread.effective_surface = effective;
    if effective.is_none() {
        thread.dock_visible = false;
    }
    thread.focus_owner = None;
    Ok(())
}

fn fallback_surface(available_surfaces: &BTreeSet<SurfaceId>) -> Option<SurfaceId> {
    SurfaceId::FALLBACK_PRIORITY
        .into_iter()
        .find(|surface| available_surfaces.contains(surface))
}

fn validate_state(state: &WorkbenchState) -> Result<(), ConformanceError> {
    if state.threads.len() > MAX_THREADS {
        return Err(ConformanceError::new(
            ErrorCode::TraceLimitExceeded,
            format!("state has {} threads", state.threads.len()),
        ));
    }
    if state.pending_loads.len() > MAX_PENDING_LOADS {
        return Err(ConformanceError::new(
            ErrorCode::TraceLimitExceeded,
            format!("state has {} pending loads", state.pending_loads.len()),
        ));
    }
    if let Some(active_thread) = &state.active_thread {
        validate_thread_id(active_thread)?;
        if !state.threads.contains_key(active_thread) {
            return Err(ConformanceError::new(
                ErrorCode::CrossThreadState,
                format!("active thread {:?} is not open", active_thread.0),
            ));
        }
    }

    let mut focus_owners = 0usize;
    for (thread_id, thread) in &state.threads {
        validate_thread_id(thread_id)?;
        if let Some(binding) = &thread.binding {
            validate_binding(binding)?;
        }
        validate_binding_availability(thread.binding.as_ref(), &thread.available_surfaces)?;
        if thread
            .effective_surface
            .is_some_and(|surface| !thread.available_surfaces.contains(&surface))
        {
            return Err(ConformanceError::new(
                ErrorCode::InvariantViolation,
                format!(
                    "thread {:?} has an unavailable effective surface",
                    thread_id.0
                ),
            ));
        }
        let expected_effective = match thread.requested_surface {
            Some(requested) if thread.available_surfaces.contains(&requested) => Some(requested),
            Some(_) => fallback_surface(&thread.available_surfaces),
            None => None,
        };
        if thread.effective_surface != expected_effective {
            return Err(ConformanceError::new(
                ErrorCode::InvariantViolation,
                format!(
                    "thread {:?} effective surface is not the deterministic projection",
                    thread_id.0
                ),
            ));
        }
        if thread.dock_visible && thread.effective_surface.is_none() {
            return Err(ConformanceError::new(
                ErrorCode::InvariantViolation,
                format!("thread {:?} has an empty visible dock", thread_id.0),
            ));
        }
        if let Some(focus_owner) = thread.focus_owner {
            focus_owners += 1;
            if state.active_thread.as_ref() != Some(thread_id)
                || state.connection != ConnectionPhase::Online
                || !thread.dock_visible
                || thread.effective_surface != Some(focus_owner)
            {
                return Err(ConformanceError::new(
                    ErrorCode::CrossThreadState,
                    format!(
                        "thread {:?} owns focus while hidden, stale, or inactive",
                        thread_id.0
                    ),
                ));
            }
        }
    }
    if focus_owners > 1 {
        return Err(ConformanceError::new(
            ErrorCode::CrossThreadState,
            "more than one thread owns primary workbench focus",
        ));
    }

    for (request_id, pending) in &state.pending_loads {
        validate_request_id(request_id)?;
        validate_thread_id(&pending.thread_id)?;
        if let Some(binding) = &pending.binding {
            validate_binding(binding)?;
        }
        if !state.threads.contains_key(&pending.thread_id) {
            return Err(ConformanceError::new(
                ErrorCode::CrossThreadState,
                format!(
                    "surface load request {:?} targets a closed thread",
                    request_id.0
                ),
            ));
        }
    }
    if let Some(selection) = &state.persisted_selection {
        validate_thread_id(&selection.thread_id)?;
        if selection.revision != state.persistence_revision {
            return Err(ConformanceError::new(
                ErrorCode::InvariantViolation,
                "persisted selection revision disagrees with state revision",
            ));
        }
        if let Some(binding) = &selection.binding {
            validate_binding(binding)?;
        }
    }

    let expected_visible = state.expected_visible_projection();
    if state.visible_projection != expected_visible {
        let cross_thread = state
            .visible_projection
            .as_ref()
            .zip(state.active_thread.as_ref())
            .is_some_and(|(visible, active)| &visible.thread_id != active);
        return Err(ConformanceError::new(
            if cross_thread {
                ErrorCode::CrossThreadState
            } else {
                ErrorCode::InvariantViolation
            },
            "visible projection does not match the active thread projection",
        ));
    }
    Ok(())
}

fn validate_binding_availability(
    binding: Option<&Binding>,
    available_surfaces: &BTreeSet<SurfaceId>,
) -> Result<(), ConformanceError> {
    if binding.is_none()
        && let Some(surface) = available_surfaces
            .iter()
            .find(|surface| surface.requires_binding())
    {
        return Err(ConformanceError::new(
            ErrorCode::StaleBinding,
            format!("unbound thread exposes binding-dependent surface {surface:?}"),
        ));
    }
    Ok(())
}

fn validate_thread_id(thread_id: &ThreadId) -> Result<(), ConformanceError> {
    validate_identifier("thread", &thread_id.0)
}

fn validate_request_id(request_id: &RequestId) -> Result<(), ConformanceError> {
    validate_identifier("request", &request_id.0)
}

fn validate_binding(binding: &Binding) -> Result<(), ConformanceError> {
    validate_identifier("repository", &binding.repository_id.0)?;
    validate_identifier("worktree", &binding.worktree_id.0)
}

fn validate_identifier(kind: &str, value: &str) -> Result<(), ConformanceError> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ConformanceError::new(
            ErrorCode::InvalidIdentifier,
            format!(
                "{kind} identifier must be 1..={MAX_IDENTIFIER_BYTES} ASCII letters, digits, '.', '_', or '-'"
            ),
        ));
    }
    Ok(())
}

fn require_online(state: &WorkbenchState) -> Result<(), ConformanceError> {
    if state.connection != ConnectionPhase::Online {
        return rejected(
            RejectCode::InvalidConnectionPhase,
            format!(
                "action requires online projection, observed {:?}",
                state.connection
            ),
        );
    }
    Ok(())
}

fn require_active_thread(
    state: &WorkbenchState,
    thread_id: &ThreadId,
) -> Result<(), ConformanceError> {
    validate_transition_identifier(validate_thread_id(thread_id))?;
    if state.active_thread.as_ref() != Some(thread_id) {
        return rejected(
            RejectCode::InactiveThread,
            format!(
                "action targets thread {:?}, active thread is {:?}",
                thread_id.0,
                state.active_thread.as_ref().map(|thread| &thread.0)
            ),
        );
    }
    Ok(())
}

fn require_thread<'a>(
    state: &'a WorkbenchState,
    thread_id: &ThreadId,
) -> Result<&'a ThreadState, ConformanceError> {
    validate_transition_identifier(validate_thread_id(thread_id))?;
    state.threads.get(thread_id).ok_or_else(|| {
        ConformanceError::rejected(
            RejectCode::UnknownThread,
            format!("unknown thread {:?}", thread_id.0),
        )
    })
}

fn require_thread_mut<'a>(
    state: &'a mut WorkbenchState,
    thread_id: &ThreadId,
) -> Result<&'a mut ThreadState, ConformanceError> {
    validate_transition_identifier(validate_thread_id(thread_id))?;
    state.threads.get_mut(thread_id).ok_or_else(|| {
        ConformanceError::rejected(
            RejectCode::UnknownThread,
            format!("unknown thread {:?}", thread_id.0),
        )
    })
}

fn require_generation(thread: &ThreadState, generation: u64) -> Result<(), ConformanceError> {
    if thread.generation != generation {
        return rejected(
            RejectCode::StaleGeneration,
            format!(
                "action generation {generation} does not match {}",
                thread.generation
            ),
        );
    }
    Ok(())
}

fn next_generation(generation: u64) -> Result<u64, ConformanceError> {
    generation.checked_add(1).ok_or_else(|| {
        ConformanceError::rejected(
            RejectCode::RevisionOverflow,
            "thread generation cannot advance beyond u64::MAX",
        )
    })
}

fn validate_transition_identifier(
    result: Result<(), ConformanceError>,
) -> Result<(), ConformanceError> {
    result.map_err(|error| ConformanceError::rejected(RejectCode::InvalidIdentifier, error.detail))
}

fn validate_snapshot_identifier(
    result: Result<(), ConformanceError>,
) -> Result<(), ConformanceError> {
    result.map_err(|error| ConformanceError::rejected(RejectCode::InvalidSnapshot, error.detail))
}

fn as_invalid_binding(error: ConformanceError) -> ConformanceError {
    ConformanceError::rejected(RejectCode::InvalidBinding, error.detail)
}

fn rejected<T>(code: RejectCode, detail: impl Into<String>) -> Result<T, ConformanceError> {
    Err(ConformanceError::rejected(code, detail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn thread(value: &str) -> ThreadId {
        ThreadId(value.to_string())
    }

    fn request(value: &str) -> RequestId {
        RequestId(value.to_string())
    }

    fn surfaces(values: impl IntoIterator<Item = SurfaceId>) -> BTreeSet<SurfaceId> {
        values.into_iter().collect()
    }

    fn bound_seed(generation: u64, worktree_id: &str) -> ThreadSeed {
        ThreadSeed::new(
            generation,
            Some(Binding::new("repository-a", worktree_id)),
            [
                SurfaceId::Files,
                SurfaceId::Search,
                SurfaceId::Review,
                SurfaceId::Git,
                SurfaceId::Terminal,
                SurfaceId::Plan,
            ],
        )
    }

    fn push_valid(
        trace: &mut ConformanceTrace,
        state: &mut WorkbenchState,
        transition: Transition,
    ) {
        match replay_transition(state, &transition) {
            Ok(next) => {
                *state = next.state;
                trace.push_with_effect(transition, next.effect, state.clone());
            }
            Err(error) => panic!("test attempted to append an invalid transition: {error}"),
        }
    }

    fn assert_error<T>(result: Result<T, ConformanceError>, code: ErrorCode) -> ConformanceError {
        match result {
            Ok(_) => panic!("expected conformance error {code:?}"),
            Err(error) => {
                assert_eq!(error.code, code, "unexpected error: {error}");
                error
            }
        }
    }

    fn replay_ok(state: &WorkbenchState, transition: &Transition) -> TransitionOutcome {
        match replay_transition(state, transition) {
            Ok(outcome) => outcome,
            Err(error) => panic!("test transition could not be replayed: {error}"),
        }
    }

    fn one_bound_thread_state() -> WorkbenchState {
        let thread_id = thread("thread-a");
        let mut thread_state = bound_seed(0, "worktree-a").into_state();
        thread_state.requested_surface = Some(SurfaceId::Files);
        thread_state.effective_surface = Some(SurfaceId::Files);
        let mut state = WorkbenchState::empty();
        state.active_thread = Some(thread_id.clone());
        state.threads.insert(thread_id, thread_state);
        if let Err(error) = normalize_state(&mut state) {
            panic!("valid test state failed to normalize: {error}");
        }
        state
    }

    #[test]
    fn clean_load_command_and_persistence_trace_is_accepted() {
        let thread_id = thread("thread-a");
        let binding = Binding::new("repository-a", "worktree-a");
        let request_id = request("load-a");
        let mut trace = ConformanceTrace::new(WorkbenchState::empty()).require([
            ActionKind::OpenThread,
            ActionKind::RequestSurface,
            ActionKind::BeginSurfaceLoad,
            ActionKind::CompleteSurfaceLoad,
            ActionKind::DispatchSurfaceCommand,
            ActionKind::PersistSelection,
        ]);
        let mut state = trace.initial_state.clone();

        push_valid(
            &mut trace,
            &mut state,
            Transition::OpenThread {
                thread_id: thread_id.clone(),
                seed: bound_seed(0, "worktree-a"),
            },
        );
        push_valid(
            &mut trace,
            &mut state,
            Transition::RequestSurface {
                thread_id: thread_id.clone(),
                surface: SurfaceId::Terminal,
            },
        );
        push_valid(
            &mut trace,
            &mut state,
            Transition::BeginSurfaceLoad {
                request_id: request_id.clone(),
                thread_id: thread_id.clone(),
                surface: SurfaceId::Terminal,
                generation: 0,
                binding: Some(binding.clone()),
            },
        );
        push_valid(
            &mut trace,
            &mut state,
            Transition::CompleteSurfaceLoad {
                request_id,
                thread_id: thread_id.clone(),
                surface: SurfaceId::Terminal,
                generation: 0,
                binding: Some(binding.clone()),
            },
        );
        push_valid(
            &mut trace,
            &mut state,
            Transition::DispatchSurfaceCommand {
                thread_id,
                surface: SurfaceId::Terminal,
                generation: 0,
                binding: Some(binding),
            },
        );
        push_valid(
            &mut trace,
            &mut state,
            Transition::PersistSelection { revision: 1 },
        );

        let report = match check_trace(&trace) {
            Ok(report) => report,
            Err(error) => panic!("clean trace was rejected: {error}"),
        };
        assert_eq!(report.steps_checked, 6);
        assert_eq!(report.final_state.persistence_revision, 1);
    }

    #[test]
    fn valid_cold_restore_round_trips_selection() {
        let thread_id = thread("thread-a");
        let mut initial = one_bound_thread_state();
        if let Some(thread) = initial.threads.get_mut(&thread_id) {
            thread.requested_surface = Some(SurfaceId::Git);
            thread.effective_surface = Some(SurfaceId::Git);
            thread.dock_visible = true;
        }
        if let Err(error) = normalize_state(&mut initial) {
            panic!("valid test state failed to normalize: {error}");
        }
        initial.persistence_revision = 7;
        initial.persisted_selection = Some(PersistedSelection {
            revision: 7,
            thread_id: thread_id.clone(),
            generation: 0,
            binding: Some(Binding::new("repository-a", "worktree-a")),
            requested_surface: Some(SurfaceId::Git),
            dock_visible: true,
        });

        let mut trace = ConformanceTrace::new(initial.clone())
            .require([ActionKind::ColdStart, ActionKind::RestoreSelection]);
        let mut state = initial;
        push_valid(&mut trace, &mut state, Transition::ColdStart);
        push_valid(&mut trace, &mut state, Transition::RestoreSelection);

        let report = match check_trace(&trace) {
            Ok(report) => report,
            Err(error) => panic!("valid restore trace was rejected: {error}"),
        };
        let restored = report.final_state.threads.get(&thread_id).map(|thread| {
            (
                thread.requested_surface,
                thread.effective_surface,
                thread.dock_visible,
            )
        });
        assert_eq!(
            restored,
            Some((Some(SurfaceId::Git), Some(SurfaceId::Git), true))
        );
        assert!(!report.final_state.restore_pending);
    }

    #[test]
    fn adopting_a_durable_record_reconciles_a_fresh_persistence_revision() {
        // Regression companion for the production defect where a disk record
        // at revision 2 met a fresh projection at revision 0 and the mismatch
        // was installed permanently instead of being reconciled.
        let thread_id = thread("thread-a");
        let initial = one_bound_thread_state();
        assert_eq!(initial.persistence_revision, 0);

        let mut trace = ConformanceTrace::new(initial.clone()).require([
            ActionKind::AdoptPersistedSelection,
            ActionKind::ColdStart,
            ActionKind::RestoreSelection,
        ]);
        let mut state = initial;
        push_valid(
            &mut trace,
            &mut state,
            Transition::AdoptPersistedSelection {
                selection: PersistedSelection {
                    revision: 2,
                    thread_id: thread_id.clone(),
                    generation: 0,
                    binding: Some(Binding::new("repository-a", "worktree-a")),
                    requested_surface: Some(SurfaceId::Git),
                    dock_visible: true,
                },
            },
        );
        push_valid(&mut trace, &mut state, Transition::ColdStart);
        push_valid(&mut trace, &mut state, Transition::RestoreSelection);

        let report = match check_trace(&trace) {
            Ok(report) => report,
            Err(error) => panic!("adoption trace was rejected: {error}"),
        };
        assert_eq!(report.final_state.persistence_revision, 2);
        let restored = report.final_state.threads.get(&thread_id).map(|thread| {
            (
                thread.requested_surface,
                thread.effective_surface,
                thread.dock_visible,
            )
        });
        assert_eq!(
            restored,
            Some((Some(SurfaceId::Git), Some(SurfaceId::Git), true))
        );
        assert!(!report.final_state.restore_pending);
    }

    #[test]
    fn invalid_restore_uses_fixed_fallback_and_repairs_persistence() {
        let thread_id = thread("thread-a");
        let mut initial = WorkbenchState::empty();
        let mut current = bound_seed(1, "worktree-b").into_state();
        current.available_surfaces =
            surfaces([SurfaceId::Plan, SurfaceId::Search, SurfaceId::Files]);
        current.requested_surface = Some(SurfaceId::Search);
        current.effective_surface = Some(SurfaceId::Search);
        initial.active_thread = Some(thread_id.clone());
        initial.threads.insert(thread_id.clone(), current);
        initial.persistence_revision = 3;
        initial.persisted_selection = Some(PersistedSelection {
            revision: 3,
            thread_id: thread_id.clone(),
            generation: 0,
            binding: Some(Binding::new("repository-a", "removed-worktree")),
            requested_surface: Some(SurfaceId::Terminal),
            dock_visible: true,
        });
        if let Err(error) = normalize_state(&mut initial) {
            panic!("invalid restore test state failed to normalize: {error}");
        }
        let mut state = initial.clone();
        let mut trace = ConformanceTrace::new(initial);
        push_valid(&mut trace, &mut state, Transition::ColdStart);
        push_valid(&mut trace, &mut state, Transition::RestoreSelection);

        let report = match check_trace(&trace) {
            Ok(report) => report,
            Err(error) => panic!("invalid restore did not converge: {error}"),
        };
        let active = report.final_state.threads.get(&thread_id);
        assert_eq!(
            active.and_then(|thread| thread.effective_surface),
            Some(SurfaceId::Files)
        );
        assert_eq!(
            report
                .final_state
                .persisted_selection
                .as_ref()
                .and_then(|selection| selection.requested_surface),
            Some(SurfaceId::Files)
        );
    }

    #[test]
    fn switching_threads_never_reuses_visible_projection_or_focus() {
        let first_id = thread("thread-a");
        let second_id = thread("thread-b");
        let mut state = one_bound_thread_state();
        let mut second = bound_seed(4, "worktree-b").into_state();
        second.requested_surface = Some(SurfaceId::Review);
        second.effective_surface = Some(SurfaceId::Review);
        second.dock_visible = true;
        state.threads.insert(second_id.clone(), second);
        if let Some(first) = state.threads.get_mut(&first_id) {
            first.dock_visible = true;
        }
        if let Err(error) = normalize_state(&mut state) {
            panic!("valid two-thread state failed to normalize: {error}");
        }
        let mut trace = ConformanceTrace::new(state.clone());
        push_valid(
            &mut trace,
            &mut state,
            Transition::SwitchThread {
                thread_id: second_id.clone(),
            },
        );

        let report = match check_trace(&trace) {
            Ok(report) => report,
            Err(error) => panic!("clean thread switch was rejected: {error}"),
        };
        let visible = report.final_state.visible_projection.as_ref();
        assert_eq!(visible.map(|visible| &visible.thread_id), Some(&second_id));
        assert_eq!(
            visible.map(|visible| visible.effective_surface),
            Some(Some(SurfaceId::Review))
        );
        assert_eq!(
            report
                .final_state
                .threads
                .get(&first_id)
                .and_then(|thread| thread.focus_owner),
            None
        );
    }

    #[test]
    fn malformed_and_unknown_critical_actions_are_rejected() {
        assert_error(check_json(br#"{"schema":"#), ErrorCode::MalformedJson);

        let unknown = serde_json::json!({
            "schema": TRACE_SCHEMA_V1,
            "required_actions": [],
            "initial_state": WorkbenchState::empty(),
            "steps": [{
                "sequence": 1,
                "kind": "teleport_surface",
                "observed_state": WorkbenchState::empty()
            }]
        });
        let encoded = match serde_json::to_vec(&unknown) {
            Ok(encoded) => encoded,
            Err(error) => panic!("failed to encode test JSON: {error}"),
        };
        assert_error(check_json(&encoded), ErrorCode::UnknownCriticalAction);
    }

    #[test]
    fn rejected_transition_requires_exact_effect_and_unchanged_state() {
        let mut trace = ConformanceTrace::new(WorkbenchState::empty());
        trace.push_with_effect(
            Transition::RequestSurface {
                thread_id: thread("missing"),
                surface: SurfaceId::Files,
            },
            TransitionEffect::Rejected {
                code: RejectCode::InactiveThread,
            },
            WorkbenchState::empty(),
        );
        assert!(check_trace(&trace).is_ok());

        trace.steps[0].observed_effect = TransitionEffect::Applied;
        assert_error(check_trace(&trace), ErrorCode::EffectMismatch);
    }

    #[test]
    fn stale_completion_is_ignored_and_binding_mismatch_is_rejected() {
        let thread_id = thread("thread-a");
        let request_id = request("load-a");
        let binding = Binding::new("repository-a", "worktree-a");
        let mut trace = ConformanceTrace::new(one_bound_thread_state());
        let mut state = trace.initial_state.clone();
        push_valid(
            &mut trace,
            &mut state,
            Transition::BeginSurfaceLoad {
                request_id: request_id.clone(),
                thread_id: thread_id.clone(),
                surface: SurfaceId::Files,
                generation: 0,
                binding: Some(binding.clone()),
            },
        );
        push_valid(
            &mut trace,
            &mut state,
            Transition::ChangeWorktree {
                thread_id: thread_id.clone(),
                generation: 0,
                worktree_id: WorktreeId("worktree-b".to_string()),
                available_surfaces: bound_seed(1, "worktree-b").available_surfaces,
            },
        );
        push_valid(
            &mut trace,
            &mut state,
            Transition::CompleteSurfaceLoad {
                request_id,
                thread_id,
                surface: SurfaceId::Files,
                generation: 0,
                binding: Some(binding),
            },
        );
        assert_eq!(
            trace.steps.last().map(|step| step.observed_effect),
            Some(TransitionEffect::StaleCompletionIgnored)
        );
        let report = match check_trace(&trace) {
            Ok(report) => report,
            Err(error) => panic!("stale completion did not converge: {error}"),
        };
        assert!(report.final_state.pending_loads.is_empty());

        let mut binding_trace = ConformanceTrace::new(one_bound_thread_state());
        binding_trace.push_with_effect(
            Transition::BeginSurfaceLoad {
                request_id: request("load-b"),
                thread_id: thread("thread-a"),
                surface: SurfaceId::Files,
                generation: 0,
                binding: Some(Binding::new("repository-a", "other-worktree")),
            },
            TransitionEffect::Rejected {
                code: RejectCode::InvalidBinding,
            },
            binding_trace.initial_state.clone(),
        );
        assert!(check_trace(&binding_trace).is_ok());
    }

    #[test]
    fn cross_thread_observation_is_rejected_before_generic_mismatch() {
        let first_id = thread("thread-a");
        let second_id = thread("thread-b");
        let mut initial = one_bound_thread_state();
        initial
            .threads
            .insert(second_id.clone(), bound_seed(0, "worktree-b").into_state());
        if let Err(error) = normalize_state(&mut initial) {
            panic!("valid two-thread state failed to normalize: {error}");
        }
        let mut expected = match replay_transition(
            &initial,
            &Transition::SwitchThread {
                thread_id: second_id,
            },
        ) {
            Ok(expected) => expected,
            Err(error) => panic!("valid switch failed: {error}"),
        };
        if let Some(visible) = expected.state.visible_projection.as_mut() {
            visible.thread_id = first_id;
        }
        let mut trace = ConformanceTrace::new(initial);
        trace.push_with_effect(
            Transition::SwitchThread {
                thread_id: thread("thread-b"),
            },
            expected.effect,
            expected.state,
        );
        assert_error(check_trace(&trace), ErrorCode::CrossThreadState);
    }

    #[test]
    fn older_revisions_are_ignored_without_rollback() {
        let mut persistence_state = one_bound_thread_state();
        persistence_state.persistence_revision = 4;
        persistence_state.persisted_selection = Some(PersistedSelection {
            revision: 4,
            thread_id: thread("thread-a"),
            generation: 0,
            binding: Some(Binding::new("repository-a", "worktree-a")),
            requested_surface: Some(SurfaceId::Files),
            dock_visible: false,
        });
        let mut persistence_trace = ConformanceTrace::new(persistence_state.clone());
        persistence_trace.push_with_effect(
            Transition::PersistSelection { revision: 3 },
            TransitionEffect::OlderRevisionIgnored,
            persistence_state,
        );
        assert!(check_trace(&persistence_trace).is_ok());

        let mut reconnecting = WorkbenchState::empty();
        reconnecting.projection_revision = 9;
        reconnecting.connection = ConnectionPhase::Reconnecting;
        let mut projection_trace = ConformanceTrace::new(reconnecting.clone());
        reconnecting.connection = ConnectionPhase::StaleProjection;
        projection_trace.push_with_effect(
            Transition::ReceiveProjectionSnapshot {
                snapshot: ProjectionSnapshot {
                    revision: 8,
                    persistence_revision: 0,
                    active_thread: None,
                    threads: Vec::new(),
                    persisted_selection: None,
                },
            },
            TransitionEffect::OlderRevisionIgnored,
            reconnecting,
        );
        let report = match check_trace(&projection_trace) {
            Ok(report) => report,
            Err(error) => panic!("older snapshot was not ignored: {error}"),
        };
        assert_eq!(
            report.final_state.connection,
            ConnectionPhase::StaleProjection
        );
        assert_eq!(report.final_state.projection_revision, 9);
    }

    #[test]
    fn missing_required_action_is_a_coverage_failure() {
        let mut trace = ConformanceTrace::new(one_bound_thread_state())
            .require([ActionKind::ColdStart, ActionKind::RestoreSelection]);
        let observed = trace.initial_state.clone();
        trace.push(
            Transition::SwitchThread {
                thread_id: thread("thread-a"),
            },
            observed,
        );
        let error = assert_error(check_trace(&trace), ErrorCode::MissingRequiredAction);
        assert!(error.detail.contains("cold_start"));
        assert!(error.detail.contains("restore_selection"));
    }

    #[test]
    fn state_mismatch_is_rejected_even_when_observed_state_is_valid() {
        let initial = one_bound_thread_state();
        let mut wrong_but_valid = initial.clone();
        wrong_but_valid.connection = ConnectionPhase::Offline;
        if let Err(error) = normalize_state(&mut wrong_but_valid) {
            panic!("valid alternate state failed to normalize: {error}");
        }
        let mut trace = ConformanceTrace::new(initial);
        trace.push(
            Transition::SwitchThread {
                thread_id: thread("thread-a"),
            },
            wrong_but_valid,
        );
        assert_error(check_trace(&trace), ErrorCode::StateMismatch);
    }

    #[test]
    fn hidden_or_stale_surface_cannot_receive_commands() {
        let initial = one_bound_thread_state();
        let mut hidden_trace = ConformanceTrace::new(initial.clone());
        hidden_trace.push_with_effect(
            Transition::DispatchSurfaceCommand {
                thread_id: thread("thread-a"),
                surface: SurfaceId::Files,
                generation: 0,
                binding: Some(Binding::new("repository-a", "worktree-a")),
            },
            TransitionEffect::Rejected {
                code: RejectCode::UnavailableSurface,
            },
            initial.clone(),
        );
        assert!(check_trace(&hidden_trace).is_ok());

        let mut offline = initial;
        offline.connection = ConnectionPhase::Offline;
        if let Err(error) = normalize_state(&mut offline) {
            panic!("offline test state failed to normalize: {error}");
        }
        let mut offline_trace = ConformanceTrace::new(offline.clone());
        offline_trace.push_with_effect(
            Transition::DispatchSurfaceCommand {
                thread_id: thread("thread-a"),
                surface: SurfaceId::Files,
                generation: 0,
                binding: Some(Binding::new("repository-a", "worktree-a")),
            },
            TransitionEffect::Rejected {
                code: RejectCode::InvalidConnectionPhase,
            },
            offline,
        );
        assert!(check_trace(&offline_trace).is_ok());
    }

    #[test]
    fn offline_plan_trace_is_accepted_while_repository_surface_is_rejected() {
        let thread_id = thread("thread-a");
        let mut state = one_bound_thread_state();
        state.connection = ConnectionPhase::Offline;
        let mut trace = ConformanceTrace::new(state.clone());

        push_valid(
            &mut trace,
            &mut state,
            Transition::RequestSurface {
                thread_id: thread_id.clone(),
                surface: SurfaceId::Plan,
            },
        );
        push_valid(
            &mut trace,
            &mut state,
            Transition::CollapseDock {
                thread_id: thread_id.clone(),
            },
        );
        push_valid(
            &mut trace,
            &mut state,
            Transition::ExpandDock {
                thread_id: thread_id.clone(),
            },
        );
        trace.push_with_effect(
            Transition::RequestSurface {
                thread_id,
                surface: SurfaceId::Git,
            },
            TransitionEffect::Rejected {
                code: RejectCode::InvalidConnectionPhase,
            },
            state,
        );

        assert!(check_trace(&trace).is_ok());
    }

    #[test]
    fn removing_binding_forces_git_and_terminal_to_deterministic_fallback() {
        let thread_id = thread("thread-a");
        let mut initial = one_bound_thread_state();
        if let Some(thread) = initial.threads.get_mut(&thread_id) {
            thread.requested_surface = Some(SurfaceId::Terminal);
            thread.effective_surface = Some(SurfaceId::Terminal);
            thread.dock_visible = true;
        }
        if let Err(error) = normalize_state(&mut initial) {
            panic!("binding removal state failed to normalize: {error}");
        }
        let transition = Transition::RemoveBinding {
            thread_id: thread_id.clone(),
            generation: 0,
            available_surfaces: surfaces([SurfaceId::Plan]),
        };
        let next = match replay_transition(&initial, &transition) {
            Ok(next) => next,
            Err(error) => panic!("valid binding removal was rejected: {error}"),
        };
        assert_eq!(next.effect, TransitionEffect::DeterministicFallback);
        let thread = next.state.threads.get(&thread_id);
        assert_eq!(
            thread.and_then(|thread| thread.effective_surface),
            Some(SurfaceId::Plan)
        );
        assert_eq!(
            thread.and_then(|thread| thread.focus_owner),
            Some(SurfaceId::Plan)
        );
    }

    #[test]
    fn changing_repository_and_worktree_is_one_generation_checked_transition() {
        let thread_id = thread("thread-a");
        let initial = one_bound_thread_state();
        let replacement = Binding::new("repository-b", "worktree-b");
        let transition = Transition::ChangeBinding {
            thread_id: thread_id.clone(),
            generation: 0,
            binding: Some(replacement.clone()),
            available_surfaces: surfaces([SurfaceId::Files, SurfaceId::Plan]),
        };

        let next = replay_transition(&initial, &transition)
            .unwrap_or_else(|error| panic!("valid atomic binding change was rejected: {error}"));
        let projected = next
            .state
            .threads
            .get(&thread_id)
            .expect("thread remains projected");
        assert_eq!(projected.binding, Some(replacement));
        assert_eq!(projected.generation, 1);
        assert_eq!(projected.effective_surface, Some(SurfaceId::Files));
    }

    #[test]
    fn snapshot_duplicate_threads_and_unbound_capabilities_are_rejected() {
        let mut reconnecting = WorkbenchState::empty();
        reconnecting.connection = ConnectionPhase::Reconnecting;
        let duplicate = SnapshotThread {
            thread_id: thread("thread-a"),
            seed: bound_seed(0, "worktree-a"),
        };
        let result = replay_transition(
            &reconnecting,
            &Transition::ReceiveProjectionSnapshot {
                snapshot: ProjectionSnapshot {
                    revision: 1,
                    persistence_revision: 0,
                    active_thread: Some(thread("thread-a")),
                    threads: vec![duplicate.clone(), duplicate],
                    persisted_selection: None,
                },
            },
        );
        let result = match result {
            Ok(result) => result,
            Err(error) => panic!("snapshot rejection was not modeled: {error}"),
        };
        assert_eq!(
            result.effect,
            TransitionEffect::Rejected {
                code: RejectCode::InvalidSnapshot
            }
        );
        assert_eq!(result.state, reconnecting);

        let invalid = Transition::OpenThread {
            thread_id: thread("thread-b"),
            seed: ThreadSeed::new(0, None, [SurfaceId::Git, SurfaceId::Plan]),
        };
        let result = match replay_transition(&WorkbenchState::empty(), &invalid) {
            Ok(result) => result,
            Err(error) => panic!("binding rejection was not modeled: {error}"),
        };
        assert_eq!(
            result.effect,
            TransitionEffect::Rejected {
                code: RejectCode::InvalidBinding
            }
        );
    }

    #[test]
    fn unavailable_request_is_rejected_without_changing_selection() {
        let thread_id = thread("thread-a");
        let mut initial = one_bound_thread_state();
        if let Some(thread) = initial.threads.get_mut(&thread_id) {
            thread.available_surfaces = surfaces([SurfaceId::Files, SurfaceId::Plan]);
        }
        if let Err(error) = normalize_state(&mut initial) {
            panic!("fallback test state failed to normalize: {error}");
        }
        let transition = Transition::RequestSurface {
            thread_id: thread_id.clone(),
            surface: SurfaceId::Terminal,
        };
        let outcome = replay_transition(&initial, &transition)
            .unwrap_or_else(|error| panic!("unavailable request replay failed: {error}"));
        assert_eq!(
            outcome.effect,
            TransitionEffect::Rejected {
                code: RejectCode::UnavailableSurface
            }
        );
        assert_eq!(outcome.state, initial);
    }

    #[test]
    fn close_active_thread_clears_its_loads_and_selects_next_thread() {
        let first_id = thread("thread-a");
        let second_id = thread("thread-b");
        let mut state = one_bound_thread_state();
        state = replay_ok(
            &state,
            &Transition::BeginSurfaceLoad {
                request_id: request("load-a"),
                thread_id: first_id.clone(),
                surface: SurfaceId::Files,
                generation: 0,
                binding: Some(Binding::new("repository-a", "worktree-a")),
            },
        )
        .state;
        state = replay_ok(
            &state,
            &Transition::OpenThread {
                thread_id: second_id.clone(),
                seed: bound_seed(0, "worktree-b"),
            },
        )
        .state;
        assert_eq!(state.active_thread, Some(first_id.clone()));

        let closed = replay_ok(
            &state,
            &Transition::CloseThread {
                thread_id: first_id,
            },
        );
        assert_eq!(closed.effect, TransitionEffect::DeterministicFallback);
        assert_eq!(closed.state.active_thread, Some(second_id));
        assert!(closed.state.pending_loads.is_empty());
    }

    #[test]
    fn cold_start_retains_thread_selection_but_clears_global_ownership() {
        let thread_id = thread("thread-a");
        let mut initial = one_bound_thread_state();
        if let Some(thread) = initial.threads.get_mut(&thread_id) {
            thread.requested_surface = Some(SurfaceId::Git);
            thread.effective_surface = Some(SurfaceId::Git);
            thread.dock_visible = true;
        }
        initial.persistence_revision = 1;
        initial.persisted_selection = Some(PersistedSelection {
            revision: 1,
            thread_id: thread_id.clone(),
            generation: 0,
            binding: Some(Binding::new("repository-a", "worktree-a")),
            requested_surface: Some(SurfaceId::Git),
            dock_visible: true,
        });
        if let Err(error) = normalize_state(&mut initial) {
            panic!("cold-start test state failed to normalize: {error}");
        }

        let cold = replay_ok(&initial, &Transition::ColdStart);
        let retained = cold.state.threads.get(&thread_id);
        assert_eq!(
            retained.and_then(|thread| thread.requested_surface),
            Some(SurfaceId::Git)
        );
        assert_eq!(
            retained.and_then(|thread| thread.effective_surface),
            Some(SurfaceId::Git)
        );
        assert_eq!(retained.and_then(|thread| thread.focus_owner), None);
        assert_eq!(retained.map(|thread| thread.dock_visible), Some(false));
        assert_eq!(cold.state.active_thread, None);
        assert!(cold.state.restore_pending);
    }

    #[test]
    fn newer_snapshot_supersedes_pending_cold_restore() {
        let thread_id = thread("thread-a");
        let mut initial = one_bound_thread_state();
        initial.persistence_revision = 1;
        initial.persisted_selection = Some(PersistedSelection {
            revision: 1,
            thread_id: thread_id.clone(),
            generation: 0,
            binding: Some(Binding::new("repository-a", "worktree-a")),
            requested_surface: Some(SurfaceId::Files),
            dock_visible: false,
        });

        let mut state = replay_ok(&initial, &Transition::ColdStart).state;
        assert!(state.restore_pending);
        state = replay_ok(&state, &Transition::Disconnect).state;
        state = replay_ok(&state, &Transition::Reconnect).state;

        let mut snapshot_seed = bound_seed(0, "worktree-a");
        snapshot_seed.requested_surface = Some(SurfaceId::Files);
        let snapshot = replay_ok(
            &state,
            &Transition::ReceiveProjectionSnapshot {
                snapshot: ProjectionSnapshot {
                    revision: 1,
                    persistence_revision: 1,
                    active_thread: Some(thread_id.clone()),
                    threads: vec![SnapshotThread {
                        thread_id: thread_id.clone(),
                        seed: snapshot_seed,
                    }],
                    persisted_selection: initial.persisted_selection,
                },
            },
        );
        assert_eq!(snapshot.effect, TransitionEffect::Applied);
        assert_eq!(snapshot.state.connection, ConnectionPhase::Online);
        assert_eq!(snapshot.state.active_thread, Some(thread_id));
        assert!(!snapshot.state.restore_pending);
    }

    #[test]
    fn request_context_mismatch_and_revision_overflow_are_transactional_rejections() {
        let thread_id = thread("thread-a");
        let request_id = request("load-a");
        let binding = Some(Binding::new("repository-a", "worktree-a"));
        let pending = replay_ok(
            &one_bound_thread_state(),
            &Transition::BeginSurfaceLoad {
                request_id: request_id.clone(),
                thread_id: thread_id.clone(),
                surface: SurfaceId::Files,
                generation: 0,
                binding: binding.clone(),
            },
        )
        .state;
        let mismatch = replay_ok(
            &pending,
            &Transition::CompleteSurfaceLoad {
                request_id,
                thread_id: thread_id.clone(),
                surface: SurfaceId::Search,
                generation: 0,
                binding,
            },
        );
        assert_eq!(
            mismatch.effect,
            TransitionEffect::Rejected {
                code: RejectCode::RequestContextMismatch
            }
        );
        assert_eq!(mismatch.state, pending);

        let mut overflow = pending;
        if let Some(thread) = overflow.threads.get_mut(&thread_id) {
            thread.generation = u64::MAX;
        }
        overflow.visible_projection = overflow.expected_visible_projection();
        let rejected = replay_ok(
            &overflow,
            &Transition::ChangeWorktree {
                thread_id,
                generation: u64::MAX,
                worktree_id: WorktreeId("worktree-b".into()),
                available_surfaces: surfaces(SurfaceId::FALLBACK_PRIORITY),
            },
        );
        assert_eq!(
            rejected.effect,
            TransitionEffect::Rejected {
                code: RejectCode::RevisionOverflow
            }
        );
        assert_eq!(rejected.state, overflow);
    }

    #[test]
    fn invalid_identifiers_and_phase_transitions_are_closed_rejections() {
        let initial = WorkbenchState::empty();
        let invalid_identifier = replay_ok(
            &initial,
            &Transition::OpenThread {
                thread_id: thread("bad/thread"),
                seed: ThreadSeed::new(0, None, [SurfaceId::Plan]),
            },
        );
        assert_eq!(
            invalid_identifier.effect,
            TransitionEffect::Rejected {
                code: RejectCode::InvalidIdentifier
            }
        );
        assert_eq!(invalid_identifier.state, initial);

        let reconnect = replay_ok(&initial, &Transition::Reconnect);
        assert_eq!(
            reconnect.effect,
            TransitionEffect::Rejected {
                code: RejectCode::InvalidConnectionPhase
            }
        );
        let snapshot = replay_ok(
            &initial,
            &Transition::ReceiveProjectionSnapshot {
                snapshot: ProjectionSnapshot {
                    revision: 1,
                    persistence_revision: 0,
                    active_thread: None,
                    threads: Vec::new(),
                    persisted_selection: None,
                },
            },
        );
        assert_eq!(
            snapshot.effect,
            TransitionEffect::Rejected {
                code: RejectCode::InvalidConnectionPhase
            }
        );
    }

    #[test]
    fn empty_trace_and_unknown_fields_fail_closed() {
        assert_error(
            check_trace(&ConformanceTrace::new(WorkbenchState::empty())),
            ErrorCode::CoverageBreach,
        );

        let mut trace = ConformanceTrace::new(one_bound_thread_state());
        let observed = trace.initial_state.clone();
        trace.push(
            Transition::SwitchThread {
                thread_id: thread("thread-a"),
            },
            observed,
        );
        let mut value = match serde_json::to_value(trace) {
            Ok(value) => value,
            Err(error) => panic!("failed to serialize test trace: {error}"),
        };
        if let Some(step) = value
            .get_mut("steps")
            .and_then(serde_json::Value::as_array_mut)
            .and_then(|steps| steps.first_mut())
            .and_then(serde_json::Value::as_object_mut)
        {
            step.insert("unexpected_payload".to_string(), serde_json::json!(true));
        }
        let encoded = match serde_json::to_vec(&value) {
            Ok(encoded) => encoded,
            Err(error) => panic!("failed to encode test trace: {error}"),
        };
        assert_error(check_json(&encoded), ErrorCode::MalformedTrace);
    }

    #[test]
    fn action_wire_names_are_unique_and_round_trip() {
        let names: BTreeSet<_> = ActionKind::ALL
            .into_iter()
            .map(ActionKind::wire_name)
            .collect();
        assert_eq!(names.len(), ActionKind::ALL.len());
        for action in ActionKind::ALL {
            assert_eq!(ActionKind::from_wire_name(action.wire_name()), Some(action));
        }
    }

    #[test]
    fn serialized_trace_contains_only_logical_projection_payload() {
        let mut trace = ConformanceTrace::new(one_bound_thread_state());
        let observed = trace.initial_state.clone();
        trace.push(
            Transition::SwitchThread {
                thread_id: thread("thread-a"),
            },
            observed,
        );
        let encoded = match serde_json::to_string(&trace) {
            Ok(encoded) => encoded,
            Err(error) => panic!("failed to serialize trace: {error}"),
        };
        for forbidden in [
            "message_content",
            "tool_output",
            "source_code",
            "credential",
            "absolute_path",
        ] {
            assert!(
                !encoded.contains(forbidden),
                "trace leaked field {forbidden}"
            );
        }
        let report = match check_json(encoded.as_bytes()) {
            Ok(report) => report,
            Err(error) => panic!("serialized trace failed round-trip: {error}"),
        };
        assert_eq!(report.steps_checked, 1);
    }

    #[derive(Clone, Copy, Debug)]
    enum GeneratedAction {
        Switch(u8),
        Request(SurfaceId),
        Collapse,
        Expand,
        Command(SurfaceId),
    }

    fn surface_strategy() -> impl Strategy<Value = SurfaceId> {
        prop_oneof![
            Just(SurfaceId::Files),
            Just(SurfaceId::Search),
            Just(SurfaceId::Review),
            Just(SurfaceId::Git),
            Just(SurfaceId::Terminal),
            Just(SurfaceId::Plan),
        ]
    }

    fn action_strategy() -> impl Strategy<Value = GeneratedAction> {
        prop_oneof![
            (0u8..5).prop_map(GeneratedAction::Switch),
            surface_strategy().prop_map(GeneratedAction::Request),
            Just(GeneratedAction::Collapse),
            Just(GeneratedAction::Expand),
            surface_strategy().prop_map(GeneratedAction::Command),
        ]
    }

    fn generated_initial_state() -> WorkbenchState {
        let mut state = WorkbenchState::empty();
        for index in 0..3 {
            let thread_id = thread(&format!("thread-{index}"));
            let mut thread_state = bound_seed(index, &format!("worktree-{index}")).into_state();
            thread_state.requested_surface = Some(SurfaceId::Files);
            thread_state.effective_surface = Some(SurfaceId::Files);
            state.threads.insert(thread_id, thread_state);
        }
        state.active_thread = Some(thread("thread-0"));
        if let Err(error) = normalize_state(&mut state) {
            panic!("generated initial state failed to normalize: {error}");
        }
        state
    }

    proptest! {
        #[test]
        fn arbitrary_json_never_panics_and_is_deterministic(
            input in prop::collection::vec(any::<u8>(), 0..4096)
        ) {
            let first = catch_unwind(AssertUnwindSafe(|| check_json(&input)));
            let second = catch_unwind(AssertUnwindSafe(|| check_json(&input)));
            match (first, second) {
                (Ok(first), Ok(second)) => prop_assert_eq!(first, second),
                _ => prop_assert!(false, "checker panicked on arbitrary JSON"),
            }
        }

        #[test]
        fn arbitrary_typed_action_sequences_never_panic_and_are_deterministic(
            actions in prop::collection::vec(action_strategy(), 0..128)
        ) {
            let initial = generated_initial_state();
            let mut trace = ConformanceTrace::new(initial.clone());
            for (index, action) in actions.into_iter().enumerate() {
                let transition = match action {
                    GeneratedAction::Switch(thread_index) => Transition::SwitchThread {
                        thread_id: thread(&format!("thread-{thread_index}")),
                    },
                    GeneratedAction::Request(surface) => Transition::RequestSurface {
                        thread_id: thread("thread-0"),
                        surface,
                    },
                    GeneratedAction::Collapse => Transition::CollapseDock {
                        thread_id: thread("thread-0"),
                    },
                    GeneratedAction::Expand => Transition::ExpandDock {
                        thread_id: thread("thread-0"),
                    },
                    GeneratedAction::Command(surface) => Transition::DispatchSurfaceCommand {
                        thread_id: thread("thread-0"),
                        surface,
                        generation: 0,
                        binding: Some(Binding::new("repository-a", "worktree-0")),
                    },
                };
                trace.steps.push(TraceStep {
                    sequence: index as u64 + 1,
                    transition,
                    observed_effect: TransitionEffect::Applied,
                    observed_state: initial.clone(),
                });
            }
            let first = catch_unwind(AssertUnwindSafe(|| check_trace(&trace)));
            let second = catch_unwind(AssertUnwindSafe(|| check_trace(&trace)));
            match (first, second) {
                (Ok(first), Ok(second)) => prop_assert_eq!(first, second),
                _ => prop_assert!(false, "checker panicked on typed actions"),
            }
        }

        #[test]
        fn valid_generated_thread_switches_are_accepted(
            switches in prop::collection::vec(0usize..3, 1..128)
        ) {
            let initial = generated_initial_state();
            let mut expected = initial.clone();
            let mut trace = ConformanceTrace::new(initial);
            for thread_index in switches {
                let thread_id = thread(&format!("thread-{thread_index}"));
                expected.active_thread = Some(thread_id.clone());
                for thread in expected.threads.values_mut() {
                    thread.focus_owner = None;
                }
                if let Some(active) = expected.threads.get_mut(&thread_id)
                    && active.dock_visible
                {
                    active.focus_owner = active.effective_surface;
                }
                expected.visible_projection = expected.expected_visible_projection();
                trace.push(Transition::SwitchThread { thread_id }, expected.clone());
            }
            prop_assert!(check_trace(&trace).is_ok());
        }

        #[test]
        fn old_generation_completion_is_always_ignored_after_invalidation(
            generation in 0u64..u64::MAX,
            surface in surface_strategy()
        ) {
            let thread_id = thread("thread-a");
            let request_id = request("load-a");
            let binding = Binding::new("repository-a", "worktree-a");
            let mut initial = WorkbenchState::empty();
            let mut seed = bound_seed(generation, "worktree-a");
            seed.requested_surface = Some(surface);
            let mut thread_state = seed.into_state();
            thread_state.effective_surface = Some(surface);
            initial.active_thread = Some(thread_id.clone());
            initial.threads.insert(thread_id.clone(), thread_state);
            if let Err(error) = normalize_state(&mut initial) {
                panic!("property initial state failed to normalize: {error}");
            }
            let mut trace = ConformanceTrace::new(initial);
            let mut state = trace.initial_state.clone();
            push_valid(
                &mut trace,
                &mut state,
                Transition::BeginSurfaceLoad {
                    request_id: request_id.clone(),
                    thread_id: thread_id.clone(),
                    surface,
                    generation,
                    binding: Some(binding.clone()),
                },
            );
            push_valid(
                &mut trace,
                &mut state,
                Transition::InvalidateCapability {
                    thread_id: thread_id.clone(),
                    generation,
                    surface,
                },
            );
            push_valid(
                &mut trace,
                &mut state,
                Transition::CompleteSurfaceLoad {
                    request_id,
                    thread_id,
                    surface,
                    generation,
                    binding: Some(binding),
                },
            );
            prop_assert_eq!(
                trace.steps.last().map(|step| step.observed_effect),
                Some(TransitionEffect::StaleCompletionIgnored)
            );
            let result = check_trace(&trace);
            prop_assert!(result.is_ok());
            if let Ok(report) = result {
                prop_assert!(report.final_state.pending_loads.is_empty());
            }
        }
    }
}
