use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;
use url::Url;

pub const PREFLIGHT_SCHEMA_V1: &str = "openagents.omega.forensics-preflight.v1";
pub const LAUNCH_INTENT_SCHEMA_V1: &str = "openagents.omega.forensics-launch-intent.v1";
pub const RUN_PROJECTION_SCHEMA_V1: &str = "openagents.omega.forensics-run.v1";
pub const WORKER_PLACEMENT_SCHEMA_V1: &str = "openagents.forensic_worker_placement.v1";
pub const WORKER_OBSERVATION_SCHEMA_V1: &str = "openagents.forensic_worker_observation.v1";
pub const MANAGED_TARGET_REF: &str = "target-ref://openagents/managed-sandbox/gce-forensic-v1";
pub const GCE_ADAPTER_REF: &str = "adapter.oa-codex-control.gce.v1";
pub const BROKER_NETWORK_POLICY_REF: &str =
    "network-policy-ref://openagents/managed-sandbox/broker-only-v1";
pub const COLDCARD_REPOSITORY_REF: &str = "repository-ref://coldcard/firmware";
pub const COLDCARD_CLONE_URL: &str = "https://github.com/Coldcard/firmware.git";
pub const COLDCARD_VULNERABLE_COMMIT: &str = "7abc9a4c680b5623fc8a64f70555dd2d3802e488";
pub const COLDCARD_FIXED_COMMIT: &str = "ca72463709f4e3f8964952039d5caf955f566a87";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdcardBenchmarkArm {
    Vulnerable,
    Incomplete,
    Fixed,
    Clean,
}

impl ColdcardBenchmarkArm {
    pub const ALL: [Self; 4] = [Self::Vulnerable, Self::Incomplete, Self::Fixed, Self::Clean];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Vulnerable => "Vulnerable",
            Self::Incomplete => "Incomplete",
            Self::Fixed => "Fixed",
            Self::Clean => "Clean",
        }
    }

    pub const fn profile_ref(self) -> &'static str {
        match self {
            Self::Vulnerable => "scan-profile-ref://coldcard/complete-vulnerable-v1",
            Self::Incomplete => "scan-profile-ref://coldcard/incomplete-v1",
            Self::Fixed => "scan-profile-ref://coldcard/fixed-v1",
            Self::Clean => "scan-profile-ref://coldcard/clean-control-v1",
        }
    }

    pub const fn commit(self) -> &'static str {
        match self {
            Self::Vulnerable | Self::Incomplete => COLDCARD_VULNERABLE_COMMIT,
            Self::Fixed | Self::Clean => COLDCARD_FIXED_COMMIT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Clean,
    Dirty,
    ExternallyPrepared,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyPolicy {
    PinnedRecursive,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryTargetProjection {
    pub repository_ref: String,
    pub display_name: String,
    pub clone_url: String,
    pub commit: String,
    pub source_state: SourceState,
    pub dependency_policy: DependencyPolicy,
    pub scan_profile_ref: String,
    pub benchmark_arm: Option<ColdcardBenchmarkArm>,
}

impl RepositoryTargetProjection {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        validate_ref("repository", &self.repository_ref)?;
        validate_ref("scan profile", &self.scan_profile_ref)?;
        validate_commit(&self.commit)?;
        validate_clone_url(&self.clone_url)?;
        if self.display_name.trim().is_empty() || self.display_name.len() > 128 {
            return Err(ForensicsError::InvalidTarget(
                "repository display name must contain 1 to 128 bytes".into(),
            ));
        }
        Ok(())
    }

    pub fn coldcard(arm: ColdcardBenchmarkArm) -> Self {
        Self {
            repository_ref: COLDCARD_REPOSITORY_REF.into(),
            display_name: "Coldcard firmware".into(),
            clone_url: COLDCARD_CLONE_URL.into(),
            commit: arm.commit().into(),
            source_state: SourceState::Clean,
            dependency_policy: DependencyPolicy::PinnedRecursive,
            scan_profile_ref: arm.profile_ref().into(),
            benchmark_arm: Some(arm),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedTargetClass {
    OpenagentsManaged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedProvider {
    GoogleCloud,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedIsolation {
    GceVm,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedWorkerProjection {
    pub target_ref: String,
    pub target_class: ManagedTargetClass,
    pub provider: ManagedProvider,
    pub adapter_ref: String,
    pub isolation: ManagedIsolation,
    pub region_ref: String,
    pub custody_ref: String,
    pub image_digest: String,
    pub profile_digest: String,
    pub network_policy_ref: String,
    pub lease_ref: String,
    pub lease_seconds: u32,
    pub capability_refs: Vec<String>,
}

impl ManagedWorkerProjection {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.target_ref != MANAGED_TARGET_REF
            || self.target_class != ManagedTargetClass::OpenagentsManaged
            || self.provider != ManagedProvider::GoogleCloud
            || self.adapter_ref != GCE_ADAPTER_REF
            || self.isolation != ManagedIsolation::GceVm
            || self.network_policy_ref != BROKER_NETWORK_POLICY_REF
        {
            return Err(ForensicsError::PlacementRefused);
        }
        validate_ref("region", &self.region_ref)?;
        validate_ref("custody", &self.custody_ref)?;
        validate_ref("lease", &self.lease_ref)?;
        validate_digest("image", &self.image_digest)?;
        validate_digest("profile", &self.profile_digest)?;
        if !(60..=3_600).contains(&self.lease_seconds) {
            return Err(ForensicsError::InvalidWorker(
                "worker lease must be between 60 and 3600 seconds".into(),
            ));
        }
        if self.capability_refs.is_empty() || self.capability_refs.len() > 64 {
            return Err(ForensicsError::InvalidWorker(
                "worker must expose 1 to 64 public capability refs".into(),
            ));
        }
        let mut unique = BTreeSet::new();
        for capability_ref in &self.capability_refs {
            validate_ref("capability", capability_ref)?;
            if !unique.insert(capability_ref) {
                return Err(ForensicsError::InvalidWorker(
                    "worker capability refs must be unique".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForensicBudgetProjection {
    pub model_ref: String,
    pub effort_ref: String,
    pub max_concurrency: u16,
    pub max_time_seconds: u32,
    pub max_tokens: u64,
    pub max_cost_micros: u64,
    pub max_artifact_bytes: u64,
    pub max_network_bytes: u64,
}

impl ForensicBudgetProjection {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        validate_ref("model", &self.model_ref)?;
        validate_ref("effort", &self.effort_ref)?;
        if self.max_concurrency == 0
            || self.max_time_seconds == 0
            || self.max_tokens == 0
            || self.max_cost_micros == 0
            || self.max_artifact_bytes == 0
        {
            return Err(ForensicsError::InvalidBudget);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Pending,
    Complete,
    Incomplete,
    Denied,
}

impl CoverageStatus {
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageClassification {
    Present,
    Missing,
    Excluded,
    Generated,
    Oversized,
    DependencyOwned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageSummaryProjection {
    pub manifest_ref: Option<String>,
    pub status: CoverageStatus,
    pub present: u32,
    pub missing: u32,
    pub excluded: u32,
    pub generated: u32,
    pub oversized: u32,
    pub dependency_owned: u32,
    pub reason_refs: Vec<String>,
}

impl CoverageSummaryProjection {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.status.is_terminal() != self.manifest_ref.is_some() {
            return Err(ForensicsError::InvalidCoverage(
                "terminal coverage requires a manifest ref and pending coverage forbids one".into(),
            ));
        }
        if let Some(manifest_ref) = &self.manifest_ref {
            validate_ref("coverage manifest", manifest_ref)?;
        }
        for reason_ref in &self.reason_refs {
            validate_ref("coverage reason", reason_ref)?;
        }
        if self.status == CoverageStatus::Complete
            && (self.missing > 0 || self.oversized > 0 || !self.reason_refs.is_empty())
        {
            return Err(ForensicsError::InvalidCoverage(
                "complete coverage cannot retain missing, oversized, or reason entries".into(),
            ));
        }
        if self.status == CoverageStatus::Incomplete
            && self.missing == 0
            && self.oversized == 0
            && self.excluded == 0
        {
            return Err(ForensicsError::InvalidCoverage(
                "incomplete coverage must retain a visible incomplete category".into(),
            ));
        }
        Ok(())
    }

    pub fn pending() -> Self {
        Self {
            manifest_ref: None,
            status: CoverageStatus::Pending,
            present: 0,
            missing: 0,
            excluded: 0,
            generated: 0,
            oversized: 0,
            dependency_owned: 0,
            reason_refs: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightReadiness {
    AwaitingCoverage,
    Ready,
    IncompleteResearch,
    Denied,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForensicsPreflightProjection {
    pub schema: String,
    pub preflight_ref: String,
    pub repository_binding_ref: String,
    pub target: RepositoryTargetProjection,
    pub worker: ManagedWorkerProjection,
    pub budget: ForensicBudgetProjection,
    pub coverage: CoverageSummaryProjection,
    pub incomplete_acknowledged: bool,
}

impl ForensicsPreflightProjection {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != PREFLIGHT_SCHEMA_V1 {
            return Err(ForensicsError::InvalidSchema);
        }
        validate_ref("preflight", &self.preflight_ref)?;
        validate_ref("repository binding", &self.repository_binding_ref)?;
        self.target.validate()?;
        self.worker.validate()?;
        self.budget.validate()?;
        self.coverage.validate()?;
        if self.coverage.status != CoverageStatus::Incomplete && self.incomplete_acknowledged {
            return Err(ForensicsError::InvalidCoverage(
                "only an incomplete research run can retain incomplete acknowledgment".into(),
            ));
        }
        Ok(())
    }

    pub const fn readiness(&self) -> PreflightReadiness {
        match self.coverage.status {
            CoverageStatus::Pending => PreflightReadiness::AwaitingCoverage,
            CoverageStatus::Complete => PreflightReadiness::Ready,
            CoverageStatus::Incomplete => PreflightReadiness::IncompleteResearch,
            CoverageStatus::Denied => PreflightReadiness::Denied,
        }
    }

    pub fn set_benchmark_arm(&mut self, arm: ColdcardBenchmarkArm) {
        self.target = RepositoryTargetProjection::coldcard(arm);
        self.coverage = CoverageSummaryProjection::pending();
        self.incomplete_acknowledged = false;
    }

    pub fn apply_terminal_coverage(
        &mut self,
        coverage: CoverageSummaryProjection,
    ) -> Result<(), ForensicsError> {
        coverage.validate()?;
        if !coverage.status.is_terminal() {
            return Err(ForensicsError::CoverageNotTerminal);
        }
        self.coverage = coverage;
        self.incomplete_acknowledged = false;
        Ok(())
    }

    pub fn acknowledge_incomplete(&mut self) -> Result<(), ForensicsError> {
        if self.coverage.status != CoverageStatus::Incomplete {
            return Err(ForensicsError::IncompleteAcknowledgmentRefused);
        }
        self.incomplete_acknowledged = true;
        Ok(())
    }

    pub fn request_launch(
        &self,
        action: ExplicitOperatorAction,
    ) -> Result<ForensicsLaunchIntent, ForensicsError> {
        self.validate()?;
        match self.readiness() {
            PreflightReadiness::AwaitingCoverage => {
                return Err(ForensicsError::CoverageNotTerminal);
            }
            PreflightReadiness::Denied => return Err(ForensicsError::PlacementRefused),
            PreflightReadiness::IncompleteResearch if !self.incomplete_acknowledged => {
                return Err(ForensicsError::IncompleteAcknowledgmentRefused);
            }
            PreflightReadiness::Ready | PreflightReadiness::IncompleteResearch => {}
        }
        validate_ref("operator action", &action.action_ref)?;
        Ok(ForensicsLaunchIntent {
            schema: LAUNCH_INTENT_SCHEMA_V1.into(),
            preflight_ref: self.preflight_ref.clone(),
            operator_action_ref: action.action_ref,
            coverage_status: self.coverage.status,
            incomplete: self.coverage.status == CoverageStatus::Incomplete,
            budget: self.budget.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplicitOperatorAction {
    pub action_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForensicsLaunchIntent {
    pub schema: String,
    pub preflight_ref: String,
    pub operator_action_ref: String,
    pub coverage_status: CoverageStatus,
    pub incomplete: bool,
    pub budget: ForensicBudgetProjection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerPlacementState {
    AdmissionRequested,
    Refused,
    Provisioning,
    WorkerReady,
    Running,
    Stopping,
    Deleting,
    Cleaned,
    RecoveryRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicWorkerPlacement {
    pub schema: String,
    pub placement_ref: String,
    pub owner_ref: String,
    pub tenant_ref: String,
    pub work_unit_ref: String,
    pub sandbox_ref: String,
    pub attachment_generation: u64,
    pub resource_generation: u64,
    pub target_class: String,
    pub provider: String,
    pub adapter_ref: String,
    pub isolation: String,
    pub region_ref: String,
    pub image_digest: String,
    pub profile_digest: String,
    pub network_policy_ref: String,
    pub lease_ref: String,
    pub budget_ref: String,
    pub capability_refs: Vec<String>,
    pub state: WorkerPlacementState,
    pub admission_receipt_ref: Option<String>,
    pub readiness_receipt_ref: Option<String>,
    pub stop_receipt_ref: Option<String>,
    pub deletion_receipt_ref: Option<String>,
    pub cleanup_receipt_ref: Option<String>,
    pub updated_at: String,
}

impl ForensicWorkerPlacement {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != WORKER_PLACEMENT_SCHEMA_V1
            || self.target_class != "openagents_managed"
            || self.provider != "google_cloud"
            || self.adapter_ref != GCE_ADAPTER_REF
            || self.isolation != "gce_vm"
            || self.network_policy_ref != BROKER_NETWORK_POLICY_REF
            || self.attachment_generation == 0
            || self.resource_generation == 0
        {
            return Err(ForensicsError::InvalidRun(
                "worker placement is outside the admitted generation-bound GCE contract".into(),
            ));
        }
        for value in [
            &self.placement_ref,
            &self.owner_ref,
            &self.tenant_ref,
            &self.work_unit_ref,
            &self.sandbox_ref,
            &self.region_ref,
            &self.lease_ref,
            &self.budget_ref,
        ] {
            validate_ref("worker placement", value)?;
        }
        validate_digest("worker image", &self.image_digest)?;
        validate_digest("worker profile", &self.profile_digest)?;
        if self.capability_refs.is_empty() || self.capability_refs.len() > 64 {
            return Err(ForensicsError::InvalidRun(
                "worker placement requires bounded capabilities".into(),
            ));
        }
        if matches!(
            self.state,
            WorkerPlacementState::WorkerReady | WorkerPlacementState::Running
        ) && (self.admission_receipt_ref.is_none() || self.readiness_receipt_ref.is_none())
        {
            return Err(ForensicsError::InvalidRun(
                "ready workers require admission and readiness receipts".into(),
            ));
        }
        if self.state == WorkerPlacementState::Cleaned
            && (self.deletion_receipt_ref.is_none() || self.cleanup_receipt_ref.is_none())
        {
            return Err(ForensicsError::InvalidRun(
                "cleaned workers require deletion and cleanup receipts".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicWorkerEvent {
    pub event_ref: String,
    pub kind: String,
    pub sequence: u64,
    pub resource_generation: u64,
    pub observed_at: String,
    pub turn_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicWorkerTurn {
    pub turn_ref: String,
    pub status: String,
    pub last_event_sequence: u64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub settled_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicWorkerObservation {
    pub schema: String,
    pub placement_ref: String,
    pub sandbox_ref: String,
    pub resource_generation: u64,
    pub lifecycle: String,
    pub cleanup_complete: bool,
    pub turn: Option<ForensicWorkerTurn>,
    pub events: Vec<ForensicWorkerEvent>,
    pub after_sequence: u64,
    pub next_sequence: u64,
    pub terminal_sequence: u64,
    pub has_more: bool,
    pub silence_is_terminal: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicsRunPhase {
    Prepared,
    Admitting,
    WorkerReady,
    Running,
    CancelRequested,
    Interrupting,
    Settled,
    Deleting,
    Cleaned,
    Refused,
    Failed,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicsFailureClass {
    Refused,
    Failed,
    RecoveryRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicsFailureProjection {
    pub class: ForensicsFailureClass,
    pub reason_ref: String,
    pub message: String,
    pub retryable: bool,
    pub observed_at: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicsRunTimestamps {
    pub admission_requested_at: Option<String>,
    pub worker_ready_at: Option<String>,
    pub run_started_at: Option<String>,
    pub cancel_requested_at: Option<String>,
    pub interrupt_observed_at: Option<String>,
    pub structurally_settled_at: Option<String>,
    pub deletion_requested_at: Option<String>,
    pub cleanup_observed_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicsRunProjection {
    pub schema: String,
    pub run_ref: String,
    pub phase: ForensicsRunPhase,
    pub placement: Option<ForensicWorkerPlacement>,
    pub event_cursor: u64,
    pub events: Vec<ForensicWorkerEvent>,
    pub timestamps: ForensicsRunTimestamps,
    pub failure: Option<ForensicsFailureProjection>,
}

impl ForensicsRunProjection {
    pub fn prepared(run_ref: String) -> Result<Self, ForensicsError> {
        validate_ref("forensic run", &run_ref)?;
        Ok(Self {
            schema: RUN_PROJECTION_SCHEMA_V1.into(),
            run_ref,
            phase: ForensicsRunPhase::Prepared,
            placement: None,
            event_cursor: 0,
            events: Vec::new(),
            timestamps: ForensicsRunTimestamps::default(),
            failure: None,
        })
    }

    pub fn mark_admitting(&mut self, requested_at: String) {
        self.phase = ForensicsRunPhase::Admitting;
        self.timestamps.admission_requested_at = Some(requested_at);
        self.failure = None;
    }

    pub fn apply_admission(
        &mut self,
        placement: ForensicWorkerPlacement,
    ) -> Result<(), ForensicsError> {
        placement.validate()?;
        if placement.work_unit_ref != self.run_ref || placement.owner_ref != placement.tenant_ref {
            return Err(ForensicsError::InvalidRun(
                "worker placement is not bound to this owner-scoped run".into(),
            ));
        }
        if let Some(current) = &self.placement
            && (current.placement_ref != placement.placement_ref
                || current.sandbox_ref != placement.sandbox_ref
                || current.attachment_generation != placement.attachment_generation
                || current.resource_generation != placement.resource_generation)
        {
            return Err(ForensicsError::DuplicateWorkerGeneration);
        }
        self.timestamps.worker_ready_at = Some(placement.updated_at.clone());
        self.phase = ForensicsRunPhase::WorkerReady;
        self.placement = Some(placement);
        Ok(())
    }

    pub fn mark_cancel_requested(&mut self, requested_at: String) {
        self.phase = ForensicsRunPhase::CancelRequested;
        self.timestamps.cancel_requested_at = Some(requested_at);
    }

    pub fn mark_deleting(&mut self, requested_at: String) {
        self.phase = ForensicsRunPhase::Deleting;
        self.timestamps.deletion_requested_at = Some(requested_at);
    }

    pub fn apply_cleaned_placement(
        &mut self,
        placement: ForensicWorkerPlacement,
    ) -> Result<(), ForensicsError> {
        placement.validate()?;
        let current = self
            .placement
            .as_ref()
            .ok_or_else(|| ForensicsError::InvalidRun("worker placement is absent".into()))?;
        if current.placement_ref != placement.placement_ref
            || current.sandbox_ref != placement.sandbox_ref
            || current.attachment_generation != placement.attachment_generation
            || current.resource_generation != placement.resource_generation
            || placement.state != WorkerPlacementState::Cleaned
        {
            return Err(ForensicsError::InvalidRun(
                "cleanup belongs to a different worker".into(),
            ));
        }
        self.timestamps.cleanup_observed_at = Some(placement.updated_at.clone());
        self.phase = ForensicsRunPhase::Cleaned;
        self.placement = Some(placement);
        Ok(())
    }

    pub fn apply_observation(
        &mut self,
        observation: ForensicWorkerObservation,
    ) -> Result<(), ForensicsError> {
        let placement = self
            .placement
            .as_ref()
            .ok_or_else(|| ForensicsError::InvalidRun("worker placement is absent".into()))?;
        if observation.schema != WORKER_OBSERVATION_SCHEMA_V1
            || observation.placement_ref != placement.placement_ref
            || observation.sandbox_ref != placement.sandbox_ref
            || observation.resource_generation != placement.resource_generation
            || observation.after_sequence != self.event_cursor
            || observation.next_sequence < observation.after_sequence
            || observation.silence_is_terminal
        {
            return Err(ForensicsError::InvalidObservation);
        }
        for (index, event) in observation.events.iter().enumerate() {
            if event.sequence != self.event_cursor + index as u64 + 1
                || event.resource_generation != placement.resource_generation
            {
                return Err(ForensicsError::InvalidObservation);
            }
        }
        if observation.next_sequence
            != observation
                .events
                .last()
                .map_or(observation.after_sequence, |event| event.sequence)
        {
            return Err(ForensicsError::InvalidObservation);
        }
        for event in &observation.events {
            match event.kind.as_str() {
                "GuestReady" => {
                    self.phase = ForensicsRunPhase::WorkerReady;
                    self.timestamps.worker_ready_at = Some(event.observed_at.clone());
                }
                "RuntimeStarted" => {
                    self.phase = ForensicsRunPhase::Running;
                    self.timestamps.run_started_at = Some(event.observed_at.clone());
                }
                "RuntimeInterruptRequested" => {
                    self.phase = ForensicsRunPhase::Interrupting;
                    self.timestamps.interrupt_observed_at = Some(event.observed_at.clone());
                }
                "RuntimeSettled" | "RuntimeInterrupted" => {
                    self.phase = ForensicsRunPhase::Settled;
                    self.timestamps.structurally_settled_at = Some(event.observed_at.clone());
                }
                "DeleteRequested" => self.phase = ForensicsRunPhase::Deleting,
                "CleanupObserved" if observation.cleanup_complete => {
                    self.phase = ForensicsRunPhase::Cleaned;
                    self.timestamps.cleanup_observed_at = Some(event.observed_at.clone());
                }
                "RuntimeFailed" | "OperationFailed" => self.phase = ForensicsRunPhase::Failed,
                "RecoveryMarked" => self.phase = ForensicsRunPhase::RecoveryRequired,
                _ => {}
            }
        }
        self.event_cursor = observation.next_sequence;
        self.events.extend(observation.events);
        if self.events.len() > 256 {
            self.events.drain(..self.events.len() - 256);
        }
        Ok(())
    }

    pub fn apply_failure(&mut self, failure: ForensicsFailureProjection) {
        self.phase = match failure.class {
            ForensicsFailureClass::Refused => ForensicsRunPhase::Refused,
            ForensicsFailureClass::Failed => ForensicsRunPhase::Failed,
            ForensicsFailureClass::RecoveryRequired => ForensicsRunPhase::RecoveryRequired,
        };
        self.failure = Some(failure);
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ForensicsError {
    #[error("the forensics schema is unsupported")]
    InvalidSchema,
    #[error("the repository target is invalid: {0}")]
    InvalidTarget(String),
    #[error("the managed worker projection is invalid: {0}")]
    InvalidWorker(String),
    #[error("only the admitted OpenAgents-managed Google Cloud target is selectable")]
    PlacementRefused,
    #[error("forensic budgets must be explicitly positive")]
    InvalidBudget,
    #[error("the coverage projection is invalid: {0}")]
    InvalidCoverage(String),
    #[error("coverage preflight is not terminal")]
    CoverageNotTerminal,
    #[error("the incomplete research state requires explicit acknowledgment")]
    IncompleteAcknowledgmentRefused,
    #[error("the forensic run projection is invalid: {0}")]
    InvalidRun(String),
    #[error("the forensic event page is not an exact ordered continuation")]
    InvalidObservation,
    #[error("an idempotent retry attempted to bind a duplicate worker generation")]
    DuplicateWorkerGeneration,
}

fn validate_ref(label: &str, value: &str) -> Result<(), ForensicsError> {
    if value.len() < 3
        || value.len() > 512
        || value.chars().any(char::is_whitespace)
        || value
            .chars()
            .any(|character| matches!(character, '?' | '&' | '='))
    {
        return Err(ForensicsError::InvalidTarget(format!(
            "{label} ref is not a bounded public ref"
        )));
    }
    Ok(())
}

fn validate_digest(label: &str, value: &str) -> Result<(), ForensicsError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(ForensicsError::InvalidWorker(format!(
            "{label} digest must use sha256"
        )));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ForensicsError::InvalidWorker(format!(
            "{label} digest must contain 64 hexadecimal characters"
        )));
    }
    Ok(())
}

fn validate_commit(value: &str) -> Result<(), ForensicsError> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ForensicsError::InvalidTarget(
            "commit must contain exactly 40 hexadecimal characters".into(),
        ));
    }
    Ok(())
}

fn validate_clone_url(value: &str) -> Result<(), ForensicsError> {
    let url = Url::parse(value)
        .map_err(|error| ForensicsError::InvalidTarget(format!("clone URL: {error}")))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ForensicsError::InvalidTarget(
            "clone URL must be public HTTPS without credentials, query, or fragment".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn worker() -> ManagedWorkerProjection {
        ManagedWorkerProjection {
            target_ref: MANAGED_TARGET_REF.into(),
            target_class: ManagedTargetClass::OpenagentsManaged,
            provider: ManagedProvider::GoogleCloud,
            adapter_ref: GCE_ADAPTER_REF.into(),
            isolation: ManagedIsolation::GceVm,
            region_ref: "region-ref://openagents/us-central1".into(),
            custody_ref: "custody-ref://openagents/operator-owned-v1".into(),
            image_digest: digest('a'),
            profile_digest: digest('b'),
            network_policy_ref: BROKER_NETWORK_POLICY_REF.into(),
            lease_ref: "lease-ref://openagents/forensics/coldcard-v1".into(),
            lease_seconds: 900,
            capability_refs: vec![
                "capability-ref://forensics/source-read".into(),
                "capability-ref://forensics/build-test".into(),
            ],
        }
    }

    fn budget() -> ForensicBudgetProjection {
        ForensicBudgetProjection {
            model_ref: "model-ref://openai/gpt-5.6".into(),
            effort_ref: "effort-ref://high".into(),
            max_concurrency: 2,
            max_time_seconds: 900,
            max_tokens: 100_000,
            max_cost_micros: 5_000_000,
            max_artifact_bytes: 10_000_000,
            max_network_bytes: 0,
        }
    }

    fn admitted_placement() -> ForensicWorkerPlacement {
        ForensicWorkerPlacement {
            schema: WORKER_PLACEMENT_SCHEMA_V1.into(),
            placement_ref: "placement.forensic.fixture".into(),
            owner_ref: "owner.forensic.fixture".into(),
            tenant_ref: "owner.forensic.fixture".into(),
            work_unit_ref: "run.forensic.fixture".into(),
            sandbox_ref: "sandbox.forensic.fixture".into(),
            attachment_generation: 1,
            resource_generation: 1,
            target_class: "openagents_managed".into(),
            provider: "google_cloud".into(),
            adapter_ref: GCE_ADAPTER_REF.into(),
            isolation: "gce_vm".into(),
            region_ref: "region.google-cloud.us-central1".into(),
            image_digest: digest('a'),
            profile_digest: digest('b'),
            network_policy_ref: BROKER_NETWORK_POLICY_REF.into(),
            lease_ref: "lease.forensic.fixture".into(),
            budget_ref: "budget.forensic.worker.initial.v1".into(),
            capability_refs: vec!["capability.forensic.fixture.agent_turn".into()],
            state: WorkerPlacementState::WorkerReady,
            admission_receipt_ref: Some("receipt.forensic.admission".into()),
            readiness_receipt_ref: Some("receipt.forensic.readiness".into()),
            stop_receipt_ref: None,
            deletion_receipt_ref: None,
            cleanup_receipt_ref: None,
            updated_at: "2026-08-01T10:00:00.000Z".into(),
        }
    }

    fn preflight(arm: ColdcardBenchmarkArm) -> ForensicsPreflightProjection {
        ForensicsPreflightProjection {
            schema: PREFLIGHT_SCHEMA_V1.into(),
            preflight_ref: "preflight-ref://omega/coldcard-v1".into(),
            repository_binding_ref: "repository-binding-ref://omega/current-worktree".into(),
            target: RepositoryTargetProjection::coldcard(arm),
            worker: worker(),
            budget: budget(),
            coverage: CoverageSummaryProjection::pending(),
            incomplete_acknowledged: false,
        }
    }

    #[test]
    fn every_coldcard_arm_is_selectable_without_internal_state_edits() {
        let mut projection = preflight(ColdcardBenchmarkArm::Vulnerable);
        for arm in ColdcardBenchmarkArm::ALL {
            projection.set_benchmark_arm(arm);
            assert_eq!(projection.target.benchmark_arm, Some(arm));
            assert_eq!(projection.target.commit, arm.commit());
            assert_eq!(projection.target.scan_profile_ref, arm.profile_ref());
            assert_eq!(projection.coverage.status, CoverageStatus::Pending);
        }
    }

    #[test]
    fn pending_coverage_cannot_create_a_launch_intent() {
        let projection = preflight(ColdcardBenchmarkArm::Vulnerable);
        assert_eq!(projection.readiness(), PreflightReadiness::AwaitingCoverage);
        assert_eq!(
            projection.request_launch(ExplicitOperatorAction {
                action_ref: "operator-action-ref://omega/start".into(),
            }),
            Err(ForensicsError::CoverageNotTerminal)
        );
    }

    #[test]
    fn idempotent_admission_cannot_bind_a_duplicate_generation() {
        let mut run =
            ForensicsRunProjection::prepared("run.forensic.fixture".into()).expect("valid run");
        let placement = admitted_placement();
        run.apply_admission(placement.clone())
            .expect("first admission");
        run.apply_admission(placement).expect("exact replay");
        let mut duplicate = admitted_placement();
        duplicate.resource_generation = 2;
        assert_eq!(
            run.apply_admission(duplicate),
            Err(ForensicsError::DuplicateWorkerGeneration)
        );
    }

    #[test]
    fn reconnect_requires_a_contiguous_cursor_and_silence_is_not_terminal() {
        let mut run =
            ForensicsRunProjection::prepared("run.forensic.fixture".into()).expect("valid run");
        run.apply_admission(admitted_placement())
            .expect("admission");
        let observation = ForensicWorkerObservation {
            schema: WORKER_OBSERVATION_SCHEMA_V1.into(),
            placement_ref: "placement.forensic.fixture".into(),
            sandbox_ref: "sandbox.forensic.fixture".into(),
            resource_generation: 1,
            lifecycle: "ready".into(),
            cleanup_complete: false,
            turn: None,
            events: vec![ForensicWorkerEvent {
                event_ref: "event.forensic.runtime-started".into(),
                kind: "RuntimeStarted".into(),
                sequence: 1,
                resource_generation: 1,
                observed_at: "2026-08-01T10:00:01.000Z".into(),
                turn_ref: Some("turn.forensic.fixture".into()),
            }],
            after_sequence: 0,
            next_sequence: 1,
            terminal_sequence: 1,
            has_more: false,
            silence_is_terminal: false,
        };
        run.apply_observation(observation).expect("ordered page");
        assert_eq!(run.phase, ForensicsRunPhase::Running);
        run.apply_observation(ForensicWorkerObservation {
            schema: WORKER_OBSERVATION_SCHEMA_V1.into(),
            placement_ref: "placement.forensic.fixture".into(),
            sandbox_ref: "sandbox.forensic.fixture".into(),
            resource_generation: 1,
            lifecycle: "ready".into(),
            cleanup_complete: false,
            turn: None,
            events: Vec::new(),
            after_sequence: 1,
            next_sequence: 1,
            terminal_sequence: 1,
            has_more: false,
            silence_is_terminal: false,
        })
        .expect("silent reconnect");
        assert_eq!(run.phase, ForensicsRunPhase::Running);
    }

    #[test]
    fn cancellation_tracks_interrupt_settlement_deletion_and_cleanup_separately() {
        let mut run =
            ForensicsRunProjection::prepared("run.forensic.fixture".into()).expect("valid run");
        run.apply_admission(admitted_placement())
            .expect("admission");
        run.mark_cancel_requested("2026-08-01T10:01:00.000Z".into());
        let events = [
            ("RuntimeInterruptRequested", "2026-08-01T10:01:01.000Z"),
            ("RuntimeInterrupted", "2026-08-01T10:01:02.000Z"),
            ("DeleteRequested", "2026-08-01T10:01:03.000Z"),
            ("CleanupObserved", "2026-08-01T10:01:04.000Z"),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (kind, observed_at))| ForensicWorkerEvent {
            event_ref: format!("event.forensic.{index}"),
            kind: kind.into(),
            sequence: index as u64 + 1,
            resource_generation: 1,
            observed_at: observed_at.into(),
            turn_ref: Some("turn.forensic.fixture".into()),
        })
        .collect::<Vec<_>>();
        run.apply_observation(ForensicWorkerObservation {
            schema: WORKER_OBSERVATION_SCHEMA_V1.into(),
            placement_ref: "placement.forensic.fixture".into(),
            sandbox_ref: "sandbox.forensic.fixture".into(),
            resource_generation: 1,
            lifecycle: "deleted".into(),
            cleanup_complete: true,
            turn: None,
            events,
            after_sequence: 0,
            next_sequence: 4,
            terminal_sequence: 4,
            has_more: false,
            silence_is_terminal: false,
        })
        .expect("cancellation page");
        assert_eq!(run.phase, ForensicsRunPhase::Cleaned);
        assert!(run.timestamps.cancel_requested_at.is_some());
        assert!(run.timestamps.interrupt_observed_at.is_some());
        assert!(run.timestamps.structurally_settled_at.is_some());
        assert!(run.timestamps.cleanup_observed_at.is_some());
    }

    #[test]
    fn incomplete_research_stays_incomplete_after_operator_acknowledgment() {
        let mut projection = preflight(ColdcardBenchmarkArm::Incomplete);
        projection
            .apply_terminal_coverage(CoverageSummaryProjection {
                manifest_ref: Some("coverage-manifest-ref://coldcard/incomplete-v1".into()),
                status: CoverageStatus::Incomplete,
                present: 99,
                missing: 4,
                excluded: 0,
                generated: 0,
                oversized: 0,
                dependency_owned: 4,
                reason_refs: vec!["coverage-reason-ref://missing-submodules".into()],
            })
            .expect("valid incomplete manifest");
        assert_eq!(
            projection.request_launch(ExplicitOperatorAction {
                action_ref: "operator-action-ref://omega/start".into(),
            }),
            Err(ForensicsError::IncompleteAcknowledgmentRefused)
        );
        projection
            .acknowledge_incomplete()
            .expect("operator can acknowledge incomplete research");
        let intent = projection
            .request_launch(ExplicitOperatorAction {
                action_ref: "operator-action-ref://omega/start".into(),
            })
            .expect("acknowledged incomplete research can create an intent");
        assert!(intent.incomplete);
        assert_eq!(intent.coverage_status, CoverageStatus::Incomplete);
    }

    #[test]
    fn substitute_placements_are_not_deserializable_or_valid() {
        let mut value = serde_json::to_value(worker()).expect("serialize worker");
        value["target_class"] = serde_json::json!("local");
        assert!(serde_json::from_value::<ManagedWorkerProjection>(value).is_err());

        for target_ref in [
            "target-ref://local",
            "target-ref://fake",
            "target-ref://byo",
            "target-ref://box-owned",
            "target-ref://pylon",
            "target-ref://remote-linux",
            "target-ref://foreign-cloud",
            "target-ref://fallback",
        ] {
            let mut candidate = worker();
            candidate.target_ref = target_ref.into();
            assert_eq!(candidate.validate(), Err(ForensicsError::PlacementRefused));
        }
    }

    #[test]
    fn renderer_projection_contains_no_provider_authority_or_secret_fields() {
        let mut run =
            ForensicsRunProjection::prepared("run.forensic.fixture".into()).expect("valid run");
        run.apply_admission(admitted_placement())
            .expect("admission");
        let json = serde_json::to_string(&(preflight(ColdcardBenchmarkArm::Fixed), run))
            .expect("serialize public projections");
        for forbidden in [
            "project_id",
            "instance_id",
            "control_token",
            "access_token",
            "credential",
            "service_account",
            "provider_client",
            "shell",
            "external_ip",
            "prompt",
            "source_bytes",
            "private_evidence",
            "finding_content",
        ] {
            assert!(!json.contains(forbidden), "projection leaked {forbidden}");
        }
    }

    #[test]
    fn explicit_positive_budgets_are_required() {
        let mut candidate = budget();
        candidate.max_tokens = 0;
        assert_eq!(candidate.validate(), Err(ForensicsError::InvalidBudget));
    }
}
