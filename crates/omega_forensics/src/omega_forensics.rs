use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use url::Url;

mod entropy_campaign;
mod entropy_repository;

pub use entropy_campaign::*;
pub use entropy_repository::*;

pub const PREFLIGHT_SCHEMA_V1: &str = "openagents.omega.forensics-preflight.v1";
pub const LAUNCH_INTENT_SCHEMA_V1: &str = "openagents.omega.forensics-launch-intent.v1";
pub const RUN_PROJECTION_SCHEMA_V1: &str = "openagents.omega.forensics-run.v1";
pub const WORKER_PLACEMENT_SCHEMA_V1: &str = "openagents.forensic_worker_placement.v1";
pub const WORKER_OBSERVATION_SCHEMA_V1: &str = "openagents.forensic_worker_observation.v1";
pub const REVIEW_PROJECTION_SCHEMA_V1: &str = "openagents.omega.forensics-review.v1";
pub const MATRIX_PROJECTION_SCHEMA_V1: &str = "openagents.omega.forensics-matrix.v1";
pub const PUBLICATION_GATE_SCHEMA_V1: &str = "openagents.omega.forensics-publication-gate.v1";
pub const COLDCARD_EVIDENCE_WORKSPACE_SCHEMA_V1: &str =
    "openagents.omega.coldcard-evidence-workspace.v1";
pub const FORENSIC_PROMPT_ARTIFACT_SCHEMA_V1: &str = "openagents.forensic_prompt_artifact.v1";
pub const FORENSIC_FINDING_SCHEMA_V1: &str = "openagents.forensic_finding.v1";
pub const FORENSIC_HYPOTHESIS_SCHEMA_V1: &str = "openagents.forensic_hypothesis.v1";
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicPromptIr {
    pub role: String,
    pub threat_model: String,
    pub vulnerability_classes: Vec<String>,
    pub security_invariants: Vec<String>,
    pub evidence_requirements: Vec<String>,
    pub dependency_exploration_policy: String,
    pub uncertainty_policy: String,
    pub tool_policy_refs: Vec<String>,
    pub finding_schema_ref: String,
    pub hypothesis_schema_ref: String,
    pub poc_policy: String,
    pub severity_policy: String,
    pub context_policy: String,
    pub budget_policy_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicPromptArtifact {
    pub schema: String,
    pub prompt_artifact_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_prompt_artifact_ref: Option<String>,
    pub prompt_ir: ForensicPromptIr,
    pub example_refs: Vec<String>,
    pub parameter_refs: Vec<String>,
    pub canonical_digest: String,
    pub dataset_revision_ref: String,
    pub compatibility_refs: Vec<String>,
    pub created_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptDigestInput<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_prompt_artifact_ref: Option<&'a str>,
    prompt_ir: &'a ForensicPromptIr,
    example_refs: &'a [String],
    parameter_refs: &'a [String],
    dataset_revision_ref: &'a str,
    compatibility_refs: &'a [String],
}

impl ForensicPromptArtifact {
    pub fn create(
        prompt_artifact_ref: String,
        parent_prompt_artifact_ref: Option<String>,
        prompt_ir: ForensicPromptIr,
        example_refs: Vec<String>,
        parameter_refs: Vec<String>,
        dataset_revision_ref: String,
        compatibility_refs: Vec<String>,
        created_at: String,
    ) -> Result<Self, ForensicsError> {
        let mut artifact = Self {
            schema: FORENSIC_PROMPT_ARTIFACT_SCHEMA_V1.into(),
            prompt_artifact_ref,
            parent_prompt_artifact_ref,
            prompt_ir,
            example_refs,
            parameter_refs,
            canonical_digest: String::new(),
            dataset_revision_ref,
            compatibility_refs,
            created_at,
        };
        artifact.canonical_digest = artifact.computed_digest()?;
        artifact.validate()?;
        Ok(artifact)
    }

    pub fn computed_digest(&self) -> Result<String, ForensicsError> {
        forensic_sha256_digest(&PromptDigestInput {
            parent_prompt_artifact_ref: self.parent_prompt_artifact_ref.as_deref(),
            prompt_ir: &self.prompt_ir,
            example_refs: &self.example_refs,
            parameter_refs: &self.parameter_refs,
            dataset_revision_ref: &self.dataset_revision_ref,
            compatibility_refs: &self.compatibility_refs,
        })
    }

    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != FORENSIC_PROMPT_ARTIFACT_SCHEMA_V1 {
            return Err(ForensicsError::InvalidSchema);
        }
        validate_ref("prompt artifact", &self.prompt_artifact_ref)?;
        if let Some(parent) = &self.parent_prompt_artifact_ref {
            validate_ref("parent prompt artifact", parent)?;
        }
        validate_prompt_ir(&self.prompt_ir)?;
        validate_bounded_refs("example", &self.example_refs, 64)?;
        validate_bounded_refs("parameter", &self.parameter_refs, 64)?;
        validate_ref("dataset revision", &self.dataset_revision_ref)?;
        validate_bounded_refs("compatibility", &self.compatibility_refs, 64)?;
        validate_digest("prompt canonical", &self.canonical_digest)?;
        if self.canonical_digest != self.computed_digest()? {
            return Err(ForensicsError::InvalidPrompt(
                "canonical digest does not bind structured content and lineage".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PromptChangeKind {
    Section,
    Example,
    Schema,
    Tool,
    Parameter,
    Policy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptSemanticChange {
    pub kind: PromptChangeKind,
    pub field: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptCompatibilityProfile {
    pub prompt_artifact_ref: String,
    pub finding_schema_ref: String,
    pub hypothesis_schema_ref: String,
    pub admitted_tool_refs: Vec<String>,
    pub runtime_tool_refs: Vec<String>,
    pub compatibility_refs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForensicPromptWorkspace {
    candidates: BTreeMap<String, ForensicPromptArtifact>,
    active_prompt_artifact_ref: String,
    draft: Option<ForensicPromptArtifact>,
    run_prompt_digests: BTreeMap<String, String>,
}

fn prompt_semantic_diff(
    parent: &ForensicPromptArtifact,
    candidate: &ForensicPromptArtifact,
) -> Vec<PromptSemanticChange> {
    let mut changes = BTreeSet::new();
    let parent_ir = &parent.prompt_ir;
    let candidate_ir = &candidate.prompt_ir;
    for (different, kind, field) in [
        (
            parent_ir.role != candidate_ir.role,
            PromptChangeKind::Section,
            "role",
        ),
        (
            parent_ir.threat_model != candidate_ir.threat_model,
            PromptChangeKind::Section,
            "threatModel",
        ),
        (
            parent_ir.vulnerability_classes != candidate_ir.vulnerability_classes,
            PromptChangeKind::Section,
            "vulnerabilityClasses",
        ),
        (
            parent_ir.security_invariants != candidate_ir.security_invariants,
            PromptChangeKind::Section,
            "securityInvariants",
        ),
        (
            parent_ir.evidence_requirements != candidate_ir.evidence_requirements,
            PromptChangeKind::Section,
            "evidenceRequirements",
        ),
        (
            parent_ir.finding_schema_ref != candidate_ir.finding_schema_ref,
            PromptChangeKind::Schema,
            "findingSchemaRef",
        ),
        (
            parent_ir.hypothesis_schema_ref != candidate_ir.hypothesis_schema_ref,
            PromptChangeKind::Schema,
            "hypothesisSchemaRef",
        ),
        (
            parent_ir.tool_policy_refs != candidate_ir.tool_policy_refs,
            PromptChangeKind::Tool,
            "toolPolicyRefs",
        ),
        (
            parent_ir.dependency_exploration_policy != candidate_ir.dependency_exploration_policy,
            PromptChangeKind::Policy,
            "dependencyExplorationPolicy",
        ),
        (
            parent_ir.uncertainty_policy != candidate_ir.uncertainty_policy,
            PromptChangeKind::Policy,
            "uncertaintyPolicy",
        ),
        (
            parent_ir.poc_policy != candidate_ir.poc_policy,
            PromptChangeKind::Policy,
            "pocPolicy",
        ),
        (
            parent_ir.severity_policy != candidate_ir.severity_policy,
            PromptChangeKind::Policy,
            "severityPolicy",
        ),
        (
            parent_ir.context_policy != candidate_ir.context_policy,
            PromptChangeKind::Policy,
            "contextPolicy",
        ),
        (
            parent_ir.budget_policy_ref != candidate_ir.budget_policy_ref,
            PromptChangeKind::Policy,
            "budgetPolicyRef",
        ),
        (
            parent.example_refs != candidate.example_refs,
            PromptChangeKind::Example,
            "exampleRefs",
        ),
        (
            parent.parameter_refs != candidate.parameter_refs,
            PromptChangeKind::Parameter,
            "parameterRefs",
        ),
    ] {
        if different {
            changes.insert((kind, field));
        }
    }
    changes
        .into_iter()
        .map(|(kind, field)| PromptSemanticChange { kind, field })
        .collect()
}

fn validate_prompt_ir(prompt: &ForensicPromptIr) -> Result<(), ForensicsError> {
    for (label, value) in [
        ("role", &prompt.role),
        ("threat model", &prompt.threat_model),
        (
            "dependency exploration policy",
            &prompt.dependency_exploration_policy,
        ),
        ("uncertainty policy", &prompt.uncertainty_policy),
        ("PoC policy", &prompt.poc_policy),
        ("severity policy", &prompt.severity_policy),
        ("context policy", &prompt.context_policy),
    ] {
        if value.trim().is_empty() || value.len() > 16_384 {
            return Err(ForensicsError::InvalidPrompt(format!(
                "{label} must contain 1 to 16384 bytes"
            )));
        }
    }
    for (label, values) in [
        ("vulnerability class", &prompt.vulnerability_classes),
        ("security invariant", &prompt.security_invariants),
        ("evidence requirement", &prompt.evidence_requirements),
    ] {
        if values.len() > 64
            || values
                .iter()
                .any(|value| value.trim().is_empty() || value.len() > 512)
        {
            return Err(ForensicsError::InvalidPrompt(format!(
                "invalid {label} list"
            )));
        }
    }
    validate_bounded_refs("tool policy", &prompt.tool_policy_refs, 64)?;
    validate_ref("finding schema", &prompt.finding_schema_ref)?;
    validate_ref("hypothesis schema", &prompt.hypothesis_schema_ref)?;
    validate_ref("budget policy", &prompt.budget_policy_ref)?;
    Ok(())
}

fn forensic_sha256_digest<Value: Serialize>(value: &Value) -> Result<String, ForensicsError> {
    let value = serde_json::to_value(value)
        .map_err(|error| ForensicsError::InvalidPrompt(format!("cannot encode prompt: {error}")))?;
    let canonical = forensic_canonical_json(&value)?;
    Ok(format!("sha256:{:x}", Sha256::digest(canonical.as_bytes())))
}

fn forensic_canonical_json(value: &serde_json::Value) -> Result<String, ForensicsError> {
    Ok(match value {
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => serde_json::to_string(value).map_err(|error| {
            ForensicsError::InvalidPrompt(format!("cannot encode prompt string: {error}"))
        })?,
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(forensic_canonical_json)
                .collect::<Result<Vec<_>, _>>()?
                .join(",")
        ),
        serde_json::Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| {
                        let encoded_key = serde_json::to_string(key).map_err(|error| {
                            ForensicsError::InvalidPrompt(format!(
                                "cannot encode prompt key: {error}"
                            ))
                        })?;
                        Ok(format!(
                            "{encoded_key}:{}",
                            forensic_canonical_json(&values[key])?
                        ))
                    })
                    .collect::<Result<Vec<_>, ForensicsError>>()?
                    .join(",")
            )
        }
    })
}

impl ForensicPromptWorkspace {
    pub fn new(active: ForensicPromptArtifact) -> Result<Self, ForensicsError> {
        active.validate()?;
        let active_prompt_artifact_ref = active.prompt_artifact_ref.clone();
        Ok(Self {
            candidates: BTreeMap::from([(active_prompt_artifact_ref.clone(), active)]),
            active_prompt_artifact_ref,
            draft: None,
            run_prompt_digests: BTreeMap::new(),
        })
    }

    pub fn active(&self) -> &ForensicPromptArtifact {
        &self.candidates[&self.active_prompt_artifact_ref]
    }

    pub fn draft(&self) -> Option<&ForensicPromptArtifact> {
        self.draft.as_ref()
    }

    pub fn candidates(&self) -> impl Iterator<Item = &ForensicPromptArtifact> {
        self.candidates.values()
    }

    pub fn clone_active(
        &mut self,
        candidate_ref: String,
        created_at: String,
    ) -> Result<(), ForensicsError> {
        if self.candidates.contains_key(&candidate_ref) {
            return Err(ForensicsError::InvalidPrompt(
                "candidate ref already exists".into(),
            ));
        }
        validate_ref("prompt candidate", &candidate_ref)?;
        let active = self.active();
        self.draft = Some(ForensicPromptArtifact::create(
            candidate_ref,
            Some(active.prompt_artifact_ref.clone()),
            active.prompt_ir.clone(),
            active.example_refs.clone(),
            active.parameter_refs.clone(),
            active.dataset_revision_ref.clone(),
            active.compatibility_refs.clone(),
            created_at,
        )?);
        Ok(())
    }

    pub fn update_draft_ir(&mut self, prompt_ir: ForensicPromptIr) -> Result<(), ForensicsError> {
        validate_prompt_ir(&prompt_ir)?;
        let draft = self.draft.as_mut().ok_or(ForensicsError::NoPromptDraft)?;
        draft.prompt_ir = prompt_ir;
        draft.canonical_digest = draft.computed_digest()?;
        Ok(())
    }

    pub fn update_draft_inputs(
        &mut self,
        example_refs: Vec<String>,
        parameter_refs: Vec<String>,
        dataset_revision_ref: String,
        compatibility_refs: Vec<String>,
    ) -> Result<(), ForensicsError> {
        validate_bounded_refs("example", &example_refs, 64)?;
        validate_bounded_refs("parameter", &parameter_refs, 64)?;
        validate_ref("dataset revision", &dataset_revision_ref)?;
        validate_bounded_refs("compatibility", &compatibility_refs, 64)?;
        let draft = self.draft.as_mut().ok_or(ForensicsError::NoPromptDraft)?;
        draft.example_refs = example_refs;
        draft.parameter_refs = parameter_refs;
        draft.dataset_revision_ref = dataset_revision_ref;
        draft.compatibility_refs = compatibility_refs;
        draft.canonical_digest = draft.computed_digest()?;
        Ok(())
    }

    pub fn save_draft(&mut self) -> Result<String, ForensicsError> {
        let draft = self.draft.take().ok_or(ForensicsError::NoPromptDraft)?;
        draft.validate()?;
        let candidate_ref = draft.prompt_artifact_ref.clone();
        self.candidates.insert(candidate_ref.clone(), draft);
        Ok(candidate_ref)
    }

    pub fn activate(&mut self, candidate_ref: &str) -> Result<(), ForensicsError> {
        if !self.candidates.contains_key(candidate_ref) {
            return Err(ForensicsError::InvalidPrompt(
                "unknown prompt candidate".into(),
            ));
        }
        self.active_prompt_artifact_ref = candidate_ref.into();
        Ok(())
    }

    pub fn bind_run(&mut self, run_ref: String) -> Result<String, ForensicsError> {
        validate_ref("forensic run", &run_ref)?;
        let digest = self.active().canonical_digest.clone();
        self.run_prompt_digests.insert(run_ref, digest.clone());
        Ok(digest)
    }

    pub fn run_prompt_digest(&self, run_ref: &str) -> Option<&str> {
        self.run_prompt_digests.get(run_ref).map(String::as_str)
    }

    pub fn semantic_diff(&self) -> Result<Vec<PromptSemanticChange>, ForensicsError> {
        let draft = self.draft.as_ref().ok_or(ForensicsError::NoPromptDraft)?;
        Ok(prompt_semantic_diff(self.active(), draft))
    }

    pub fn check_compatibility(
        &self,
        profile: &PromptCompatibilityProfile,
    ) -> Result<(), ForensicsError> {
        let active = self.active();
        if profile.prompt_artifact_ref != active.prompt_artifact_ref
            || profile.finding_schema_ref != active.prompt_ir.finding_schema_ref
            || profile.hypothesis_schema_ref != active.prompt_ir.hypothesis_schema_ref
            || !active
                .compatibility_refs
                .iter()
                .all(|value| profile.compatibility_refs.contains(value))
            || !active.prompt_ir.tool_policy_refs.iter().all(|value| {
                profile.admitted_tool_refs.contains(value)
                    && profile.runtime_tool_refs.contains(value)
            })
        {
            return Err(ForensicsError::IncompatiblePrompt);
        }
        Ok(())
    }
}

pub fn baseline_forensic_prompt(
    created_at: String,
) -> Result<ForensicPromptArtifact, ForensicsError> {
    ForensicPromptArtifact::create(
        "prompt.forensic.omega.baseline.v1".into(),
        None,
        ForensicPromptIr {
            role: "Find security-relevant invariant violations and preserve uncertainty.".into(),
            threat_model: "Trace attacker-controlled and entropy-sensitive inputs across dependency boundaries.".into(),
            vulnerability_classes: vec!["entropy downgrade".into(), "trust-boundary violation".into()],
            security_invariants: vec!["Security claims require source-grounded causal evidence.".into()],
            evidence_requirements: vec!["Cite exact source locations and every causal link.".into()],
            dependency_exploration_policy: "Inspect every mounted dependency needed to complete a causal path.".into(),
            uncertainty_policy: "Use a typed hypothesis when evidence is incomplete.".into(),
            tool_policy_refs: vec!["tool.source.read".into(), "tool.dependency.inspect".into()],
            finding_schema_ref: FORENSIC_FINDING_SCHEMA_V1.into(),
            hypothesis_schema_ref: FORENSIC_HYPOTHESIS_SCHEMA_V1.into(),
            poc_policy: "Prefer deterministic, fixture-bound reproduction.".into(),
            severity_policy: "Severity follows demonstrated impact.".into(),
            context_policy: "Prioritize high-risk paths without changing admitted authority.".into(),
            budget_policy_ref: "budget.admitted.forensic.v1".into(),
        },
        vec!["example.typed.finding.v1".into()],
        vec!["parameter.reasoning.high".into()],
        "dataset.omega.forensics.v1".into(),
        vec!["compatibility.loupe.v1".into()],
        created_at,
    )
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicEvidenceTier {
    Hypothesis,
    SourceObserved,
    ArtifactObserved,
    Executed,
    IndependentlyVerified,
}

impl ForensicEvidenceTier {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hypothesis => "Hypothesis",
            Self::SourceObserved => "Source observed",
            Self::ArtifactObserved => "Artifact observed",
            Self::Executed => "Executed",
            Self::IndependentlyVerified => "Independently verified",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicExactness {
    Exact,
    Estimated,
    UpperBound,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicMetricTruth {
    pub metric_ref: String,
    pub label: String,
    pub unit: String,
    pub value: Option<u64>,
    pub exactness: ForensicExactness,
    pub unavailable_reason_ref: Option<String>,
    pub source_event_refs: Vec<String>,
    pub source_receipt_refs: Vec<String>,
}

impl ForensicMetricTruth {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        validate_ref("metric", &self.metric_ref)?;
        if self.label.trim().is_empty() || self.label.len() > 128 || self.unit.len() > 64 {
            return Err(ForensicsError::InvalidReview(
                "metric labels and units must be bounded".into(),
            ));
        }
        let unavailable = self.exactness == ForensicExactness::Unavailable;
        if unavailable != self.value.is_none()
            || unavailable != self.unavailable_reason_ref.is_some()
        {
            return Err(ForensicsError::InvalidReview(
                "unavailable metrics require a reason and no numeric zero".into(),
            ));
        }
        validate_bounded_refs("metric event", &self.source_event_refs, 256)?;
        validate_bounded_refs("metric receipt", &self.source_receipt_refs, 256)?;
        if let Some(reason_ref) = &self.unavailable_reason_ref {
            validate_ref("metric unavailable reason", reason_ref)?;
        }
        Ok(())
    }

    pub fn display_value(&self) -> String {
        match (self.value, self.exactness) {
            (Some(value), ForensicExactness::Exact) => format!("{value} {}", self.unit),
            (Some(value), ForensicExactness::Estimated) => {
                format!("≈ {value} {} · estimated", self.unit)
            }
            (Some(value), ForensicExactness::UpperBound) => {
                format!("≤ {value} {} · upper bound", self.unit)
            }
            (None, ForensicExactness::Unavailable) => "Unavailable".into(),
            _ => "Invalid metric truth".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicSourceCitation {
    pub source_ref: String,
    pub path: String,
    pub symbol: Option<String>,
    pub start_line: u32,
    pub end_line: u32,
    pub commit: String,
}

impl ForensicSourceCitation {
    pub fn validate(&self, expected_commit: &str) -> Result<(), ForensicsError> {
        validate_ref("source", &self.source_ref)?;
        validate_commit(&self.commit)?;
        if self.commit != expected_commit
            || self.path.is_empty()
            || self.path.len() > 1_024
            || self.path.starts_with('/')
            || self
                .path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || self.start_line == 0
            || self.end_line < self.start_line
            || self.end_line - self.start_line > 10_000
            || self
                .symbol
                .as_ref()
                .is_some_and(|symbol| symbol.trim().is_empty() || symbol.len() > 256)
        {
            return Err(ForensicsError::InvalidReview(
                "source citations must bind a bounded relative path and exact pinned commit".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicCausalLink {
    pub sequence: u32,
    pub proposition: String,
    pub evidence_refs: Vec<String>,
    pub supported: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicEvidenceReceiptProjection {
    pub receipt_ref: String,
    pub evidence_tier: ForensicEvidenceTier,
    pub outcome: String,
    pub artifact_ref: Option<String>,
    pub verifier_verdict: Option<String>,
    pub observed_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicFindingProjection {
    pub finding_ref: String,
    pub claim_ref: String,
    pub title: String,
    pub impact: String,
    pub severity: String,
    pub claim_state: String,
    pub evidence_tier: ForensicEvidenceTier,
    pub duplicate_group_ref: Option<String>,
    pub source_refs: Vec<ForensicSourceCitation>,
    pub causal_path: Vec<ForensicCausalLink>,
    pub evidence_receipts: Vec<ForensicEvidenceReceiptProjection>,
    pub poc_ref: Option<String>,
    pub submitted_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicHypothesisProjection {
    pub hypothesis_ref: String,
    pub suspected_mechanism: String,
    pub supporting_refs: Vec<String>,
    pub missing_evidence: Vec<String>,
    pub next_check: String,
    pub consequence_if_true: String,
    pub state: String,
    pub submitted_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicReviewOutcome {
    Running,
    Completed,
    CompletedIncomplete,
    Missed,
    Cancelled,
    Failed,
    Censored,
    CleanupFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicBudgetState {
    WithinBudget,
    Exhausted,
    Unmeasurable,
    Refused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicLifecycleState {
    Pending,
    Active,
    Succeeded,
    Failed,
    Cancelled,
    Censored,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicLifecycleStage {
    pub stage_ref: String,
    pub label: String,
    pub state: ForensicLifecycleState,
    pub observed_at: Option<String>,
    pub receipt_ref: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicReviewDecisionKind {
    Accept,
    Correct,
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicPublicationGateKind {
    Redaction,
    IndependentReview,
    DisclosureScope,
    MaintainerDecision,
    PublicationAuthority,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicPublicationGateState {
    Satisfied,
    Blocked,
    Denied,
    AwaitingReview,
    Rejected,
    Stale,
    EligibleNotAuthorized,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicPublicationGate {
    pub gate_ref: String,
    pub kind: ForensicPublicationGateKind,
    pub state: ForensicPublicationGateState,
    pub evidence_ref: Option<String>,
    pub blocker: String,
    pub next_action: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicPublicationGateProjection {
    pub schema: String,
    pub case_ref: String,
    pub private: bool,
    pub synthetic: bool,
    pub operator_ready: bool,
    pub maintainer_approved: bool,
    pub publication_authorized: bool,
    pub gates: Vec<ForensicPublicationGate>,
}

impl ForensicPublicationGateProjection {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != PUBLICATION_GATE_SCHEMA_V1 {
            return Err(ForensicsError::InvalidSchema);
        }
        validate_ref("publication case", &self.case_ref)?;
        if self.gates.len() != 5 {
            return Err(ForensicsError::InvalidPublicationGate(
                "all five publication gates must be present".into(),
            ));
        }
        let mut kinds = BTreeSet::new();
        for gate in &self.gates {
            validate_ref("publication gate", &gate.gate_ref)?;
            if let Some(evidence_ref) = &gate.evidence_ref {
                validate_ref("publication evidence", evidence_ref)?;
            }
            if gate.blocker.trim().is_empty()
                || gate.next_action.trim().is_empty()
                || !kinds.insert(gate.kind)
            {
                return Err(ForensicsError::InvalidPublicationGate(
                    "publication gates require unique kinds, blockers, and next actions".into(),
                ));
            }
        }
        let all_satisfied = self
            .gates
            .iter()
            .all(|gate| gate.state == ForensicPublicationGateState::Satisfied);
        if self.synthetic && (!self.private || self.publication_authorized)
            || self.publication_authorized
                && (!all_satisfied || !self.maintainer_approved || !self.operator_ready)
            || !self.private && !self.publication_authorized
        {
            return Err(ForensicsError::InvalidPublicationGate(
                "privacy, synthetic evidence, and publication authority disagree".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicReviewDecision {
    pub decision_ref: String,
    pub sequence: u32,
    pub finding_ref: String,
    pub decision: ForensicReviewDecisionKind,
    pub reason: String,
    pub reviewer_ref: String,
    pub decided_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicsReviewProjection {
    pub schema: String,
    pub review_ref: String,
    pub run_ref: String,
    pub prompt_digest: String,
    pub repository_ref: String,
    pub commit: String,
    pub coverage_status: CoverageStatus,
    pub outcome: ForensicReviewOutcome,
    pub budget_state: ForensicBudgetState,
    pub findings: Vec<ForensicFindingProjection>,
    pub hypotheses: Vec<ForensicHypothesisProjection>,
    pub metrics: Vec<ForensicMetricTruth>,
    pub lifecycle: Vec<ForensicLifecycleStage>,
    pub placement_ref: String,
    pub sandbox_ref: String,
    pub resource_generation: u64,
    pub cleanup_state: String,
    pub cleanup_receipt_ref: Option<String>,
    pub decisions: Vec<ForensicReviewDecision>,
}

impl ForensicsReviewProjection {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != REVIEW_PROJECTION_SCHEMA_V1 {
            return Err(ForensicsError::InvalidSchema);
        }
        for value in [
            &self.review_ref,
            &self.run_ref,
            &self.repository_ref,
            &self.placement_ref,
            &self.sandbox_ref,
        ] {
            validate_ref("review", value)?;
        }
        validate_commit(&self.commit)?;
        validate_digest("review prompt", &self.prompt_digest)?;
        if self.resource_generation == 0
            || self.findings.len() > 1_024
            || self.hypotheses.len() > 1_024
            || self.metrics.len() > 512
        {
            return Err(ForensicsError::InvalidReview(
                "review collections or worker generation are invalid".into(),
            ));
        }
        if self.coverage_status == CoverageStatus::Incomplete
            && self.outcome == ForensicReviewOutcome::Completed
        {
            return Err(ForensicsError::InvalidReview(
                "incomplete inputs cannot render as a complete run".into(),
            ));
        }
        let cleanup_failed = self.outcome == ForensicReviewOutcome::CleanupFailed;
        let cleanup_observed = self.cleanup_state == "observed_zero_residue";
        if cleanup_observed != self.cleanup_receipt_ref.is_some()
            || (cleanup_failed && cleanup_observed)
            || (matches!(
                self.outcome,
                ForensicReviewOutcome::Completed
                    | ForensicReviewOutcome::CompletedIncomplete
                    | ForensicReviewOutcome::Cancelled
            ) && !cleanup_observed)
            || (self.outcome == ForensicReviewOutcome::Completed
                && self.coverage_status != CoverageStatus::Complete)
            || (self.outcome == ForensicReviewOutcome::CompletedIncomplete
                && self.coverage_status != CoverageStatus::Incomplete)
        {
            return Err(ForensicsError::InvalidReview(
                "cleanup truth and its receipt disagree".into(),
            ));
        }
        let mut item_refs = BTreeSet::new();
        for finding in &self.findings {
            validate_finding(finding, &self.commit)?;
            if !item_refs.insert(finding.finding_ref.as_str()) {
                return Err(ForensicsError::InvalidReview(
                    "duplicate review item ref".into(),
                ));
            }
        }
        for hypothesis in &self.hypotheses {
            validate_hypothesis(hypothesis)?;
            if !item_refs.insert(hypothesis.hypothesis_ref.as_str()) {
                return Err(ForensicsError::InvalidReview(
                    "duplicate review item ref".into(),
                ));
            }
        }
        let mut metric_refs = BTreeSet::new();
        for metric in &self.metrics {
            metric.validate()?;
            if !metric_refs.insert(metric.metric_ref.as_str()) {
                return Err(ForensicsError::InvalidReview(
                    "review metrics must have unique refs".into(),
                ));
            }
        }
        validate_lifecycle(&self.lifecycle)?;
        validate_decisions(&self.decisions, &self.findings)?;
        Ok(())
    }

    pub fn append_decision(
        &mut self,
        finding_ref: &str,
        decision: ForensicReviewDecisionKind,
        reason: String,
        reviewer_ref: String,
        decided_at: String,
    ) -> Result<(), ForensicsError> {
        if !self
            .findings
            .iter()
            .any(|finding| finding.finding_ref == finding_ref)
        {
            return Err(ForensicsError::InvalidReview(
                "review decision targets an unknown finding".into(),
            ));
        }
        validate_ref("reviewer", &reviewer_ref)?;
        if reason.trim().is_empty() || reason.len() > 8_000 {
            return Err(ForensicsError::InvalidReview(
                "review decision reason must be bounded".into(),
            ));
        }
        let sequence = u32::try_from(self.decisions.len() + 1)
            .map_err(|_| ForensicsError::InvalidReview("too many review decisions".into()))?;
        self.decisions.push(ForensicReviewDecision {
            decision_ref: format!("decision.{finding_ref}.{sequence}"),
            sequence,
            finding_ref: finding_ref.into(),
            decision,
            reason,
            reviewer_ref,
            decided_at,
        });
        Ok(())
    }
}

fn validate_finding(
    finding: &ForensicFindingProjection,
    expected_commit: &str,
) -> Result<(), ForensicsError> {
    validate_ref("finding", &finding.finding_ref)?;
    validate_ref("claim", &finding.claim_ref)?;
    if finding.title.trim().is_empty()
        || finding.title.len() > 512
        || finding.impact.trim().is_empty()
        || finding.impact.len() > 8_000
        || finding.source_refs.is_empty()
        || finding.source_refs.len() > 256
        || finding.causal_path.is_empty()
        || finding.causal_path.len() > 128
        || finding.evidence_receipts.len() > 256
    {
        return Err(ForensicsError::InvalidReview(
            "finding content or evidence bounds are invalid".into(),
        ));
    }
    for source in &finding.source_refs {
        source.validate(expected_commit)?;
    }
    for (index, link) in finding.causal_path.iter().enumerate() {
        if link.sequence as usize != index + 1
            || link.proposition.trim().is_empty()
            || link.proposition.len() > 512
            || link.evidence_refs.is_empty()
        {
            return Err(ForensicsError::InvalidReview(
                "causal paths must be ordered and evidenced".into(),
            ));
        }
        validate_bounded_refs("causal evidence", &link.evidence_refs, 64)?;
    }
    for receipt in &finding.evidence_receipts {
        validate_ref("evidence receipt", &receipt.receipt_ref)?;
        if let Some(artifact_ref) = &receipt.artifact_ref {
            validate_ref("evidence artifact", artifact_ref)?;
        }
        if receipt.outcome.trim().is_empty() {
            return Err(ForensicsError::InvalidReview(
                "evidence outcome is absent".into(),
            ));
        }
    }
    if matches!(
        finding.evidence_tier,
        ForensicEvidenceTier::Executed | ForensicEvidenceTier::IndependentlyVerified
    ) && !finding
        .evidence_receipts
        .iter()
        .any(|receipt| receipt.outcome == "succeeded" && receipt.artifact_ref.is_some())
    {
        return Err(ForensicsError::InvalidReview(
            "executed evidence requires a successful artifact receipt".into(),
        ));
    }
    Ok(())
}

fn validate_hypothesis(hypothesis: &ForensicHypothesisProjection) -> Result<(), ForensicsError> {
    validate_ref("hypothesis", &hypothesis.hypothesis_ref)?;
    if hypothesis.suspected_mechanism.trim().is_empty()
        || hypothesis.suspected_mechanism.len() > 8_000
        || hypothesis.missing_evidence.is_empty()
        || hypothesis.missing_evidence.len() > 128
        || hypothesis
            .missing_evidence
            .iter()
            .any(|value| value.trim().is_empty() || value.len() > 512)
        || hypothesis.next_check.trim().is_empty()
        || hypothesis.next_check.len() > 8_000
        || hypothesis.consequence_if_true.trim().is_empty()
        || hypothesis.consequence_if_true.len() > 8_000
    {
        return Err(ForensicsError::InvalidReview(
            "hypotheses require missing evidence, next check, and consequence".into(),
        ));
    }
    validate_bounded_refs("hypothesis support", &hypothesis.supporting_refs, 256)
}

fn validate_lifecycle(stages: &[ForensicLifecycleStage]) -> Result<(), ForensicsError> {
    if stages.is_empty() || stages.len() > 32 {
        return Err(ForensicsError::InvalidReview(
            "lifecycle waterfall must contain 1 to 32 stages".into(),
        ));
    }
    let mut refs = BTreeSet::new();
    for stage in stages {
        validate_ref("lifecycle stage", &stage.stage_ref)?;
        if !refs.insert(stage.stage_ref.as_str()) || stage.label.trim().is_empty() {
            return Err(ForensicsError::InvalidReview(
                "lifecycle stages must be unique and labelled".into(),
            ));
        }
        let observed = stage.observed_at.is_some();
        if (stage.state == ForensicLifecycleState::Pending && observed)
            || (stage.state != ForensicLifecycleState::Pending && !observed)
        {
            return Err(ForensicsError::InvalidReview(
                "lifecycle state and observation timestamp disagree".into(),
            ));
        }
    }
    Ok(())
}

fn validate_decisions(
    decisions: &[ForensicReviewDecision],
    findings: &[ForensicFindingProjection],
) -> Result<(), ForensicsError> {
    if decisions.len() > 4_096 {
        return Err(ForensicsError::InvalidReview(
            "too many review decisions".into(),
        ));
    }
    let finding_refs = findings
        .iter()
        .map(|finding| finding.finding_ref.as_str())
        .collect::<BTreeSet<_>>();
    let mut decision_refs = BTreeSet::new();
    for (index, decision) in decisions.iter().enumerate() {
        if decision.sequence as usize != index + 1
            || !finding_refs.contains(decision.finding_ref.as_str())
            || decision.reason.trim().is_empty()
            || decision.reason.len() > 8_000
        {
            return Err(ForensicsError::InvalidReview(
                "review decisions must append in order against an immutable finding".into(),
            ));
        }
        validate_ref("review decision", &decision.decision_ref)?;
        validate_ref("reviewer", &decision.reviewer_ref)?;
        if !decision_refs.insert(decision.decision_ref.as_str()) {
            return Err(ForensicsError::InvalidReview(
                "review decision refs must be unique".into(),
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicDatasetSplit {
    Train,
    Development,
    Holdout,
    CleanHoldout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicMatrixPopulation {
    Incomplete,
    Vulnerable,
    StructuralVariant,
    FixedControl,
    CleanControl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicMatrixOutcome {
    Hit,
    Miss,
    NotEligible,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicMatrixArm {
    pub arm_ref: String,
    pub model_family_ref: String,
    pub role_ref: String,
    pub prompt_digest: String,
    pub model_digest: String,
    pub effort_ref: String,
    pub scope_ref: String,
    pub dependency_policy_ref: String,
    pub random_seed: u64,
    pub tool_surface_digest: String,
    pub analysis_mode_ref: String,
    pub worker_image_digest: String,
    pub worker_profile_digest: String,
    pub source_bundle_digest: String,
    pub writable_disk_ref: String,
    pub provider_session_ref: String,
    pub auth_home_ref: String,
    pub environment_ref: String,
    pub worker_state_ref: String,
}

impl ForensicMatrixArm {
    fn validate(&self) -> Result<(), ForensicsError> {
        for value in [
            &self.prompt_digest,
            &self.model_digest,
            &self.tool_surface_digest,
            &self.worker_image_digest,
            &self.worker_profile_digest,
            &self.source_bundle_digest,
        ] {
            validate_digest("matrix comparison", value)?;
        }
        for value in [
            &self.arm_ref,
            &self.model_family_ref,
            &self.role_ref,
            &self.effort_ref,
            &self.scope_ref,
            &self.dependency_policy_ref,
            &self.analysis_mode_ref,
            &self.writable_disk_ref,
            &self.provider_session_ref,
            &self.auth_home_ref,
            &self.environment_ref,
            &self.worker_state_ref,
        ] {
            validate_ref("matrix arm", value)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicMatrixRun {
    pub run_ref: String,
    pub run_digest: String,
    pub arm_ref: String,
    pub dataset_split: ForensicDatasetSplit,
    pub population: ForensicMatrixPopulation,
    pub coverage_status: CoverageStatus,
    pub outcome: ForensicMatrixOutcome,
    pub censored: bool,
    pub censor_at_milliseconds: Option<u64>,
    pub identification_milliseconds: Option<u64>,
    pub identification_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub token_exactness: ForensicExactness,
    pub cost_micros: Option<u64>,
    pub cost_exactness: ForensicExactness,
    pub causal_links_supported: u32,
    pub causal_links_required: u32,
    pub false_positive_count: u32,
    pub reviewer_active_seconds: Option<u64>,
    pub budget_compliant: bool,
    pub cleanup_observed: bool,
    pub qualified_finding_refs: Vec<String>,
    pub failure_refs: Vec<String>,
    pub event_refs: Vec<String>,
    pub receipt_refs: Vec<String>,
}

impl ForensicMatrixRun {
    pub fn eligible_for_identification(&self) -> bool {
        self.coverage_status == CoverageStatus::Complete
            && matches!(
                self.population,
                ForensicMatrixPopulation::Vulnerable | ForensicMatrixPopulation::StructuralVariant
            )
            && matches!(
                self.dataset_split,
                ForensicDatasetSplit::Development | ForensicDatasetSplit::Holdout
            )
    }

    fn validate(&self) -> Result<(), ForensicsError> {
        validate_ref("matrix run", &self.run_ref)?;
        validate_ref("matrix arm", &self.arm_ref)?;
        validate_digest("matrix run", &self.run_digest)?;
        validate_bounded_refs("matrix failure", &self.failure_refs, 256)?;
        validate_bounded_refs(
            "matrix qualified finding",
            &self.qualified_finding_refs,
            1_024,
        )?;
        validate_bounded_refs("matrix event", &self.event_refs, 4_096)?;
        validate_bounded_refs("matrix receipt", &self.receipt_refs, 4_096)?;
        if self.event_refs.is_empty() || self.receipt_refs.is_empty() {
            return Err(ForensicsError::InvalidMatrix(
                "every row requires event and receipt drill-down refs".into(),
            ));
        }
        if self.population == ForensicMatrixPopulation::Incomplete
            && self.coverage_status == CoverageStatus::Complete
        {
            return Err(ForensicsError::InvalidMatrix(
                "incomplete populations cannot claim complete coverage".into(),
            ));
        }
        if self.population == ForensicMatrixPopulation::CleanControl
            && self.dataset_split != ForensicDatasetSplit::CleanHoldout
        {
            return Err(ForensicsError::InvalidMatrix(
                "clean controls must remain in clean holdout".into(),
            ));
        }
        let is_hit = self.outcome == ForensicMatrixOutcome::Hit;
        let is_miss = self.outcome == ForensicMatrixOutcome::Miss;
        if is_hit
            && (!self.eligible_for_identification()
                || self.censored
                || self.identification_milliseconds.is_none()
                || self.identification_tokens.is_none()
                || self.qualified_finding_refs.is_empty())
        {
            return Err(ForensicsError::InvalidMatrix(
                "hits require eligible, uncensored identification observations".into(),
            ));
        }
        if is_miss
            && (!self.eligible_for_identification()
                || !self.censored
                || self.censor_at_milliseconds.is_none())
        {
            return Err(ForensicsError::InvalidMatrix(
                "misses remain eligible right-censored observations".into(),
            ));
        }
        if self.censored != self.censor_at_milliseconds.is_some()
            || self.censor_at_milliseconds == Some(0)
        {
            return Err(ForensicsError::InvalidMatrix(
                "censoring requires a nonzero declared boundary".into(),
            ));
        }
        if !is_hit
            && (self.identification_milliseconds.is_some() || self.identification_tokens.is_some())
        {
            return Err(ForensicsError::InvalidMatrix(
                "non-hits cannot invent identification values".into(),
            ));
        }
        if self.causal_links_supported > self.causal_links_required {
            return Err(ForensicsError::InvalidMatrix(
                "supported causal links exceed the frozen requirement".into(),
            ));
        }
        for (label, value, exactness) in [
            ("tokens", self.total_tokens, self.token_exactness),
            ("cost", self.cost_micros, self.cost_exactness),
        ] {
            if (value.is_none()) != (exactness == ForensicExactness::Unavailable) {
                return Err(ForensicsError::InvalidMatrix(format!(
                    "{label} availability and exactness disagree"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicStatisticStatus {
    Exact,
    Provisional,
    NotEstimable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicStatistic {
    pub status: ForensicStatisticStatus,
    pub value: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicConfidenceInterval {
    pub lower_basis_points: u32,
    pub upper_basis_points: u32,
    pub provisional: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicMatrixRow {
    pub arm_ref: String,
    pub sample_count: usize,
    pub eligible_count: usize,
    pub hit_count: usize,
    pub miss_count: usize,
    pub censor_count: usize,
    pub hit_rate_basis_points: Option<u32>,
    pub hit_rate_interval: Option<ForensicConfidenceInterval>,
    pub identification_observations_milliseconds: Vec<Option<u64>>,
    pub p50_identification_milliseconds: ForensicStatistic,
    pub tail_identification_milliseconds: ForensicStatistic,
    pub identification_token_observations: Vec<Option<u64>>,
    pub total_tokens: Option<u64>,
    pub token_exactness: ForensicExactness,
    pub total_cost_micros: Option<u64>,
    pub cost_exactness: ForensicExactness,
    pub causal_coverage_basis_points: Option<u32>,
    pub false_positive_count: u32,
    pub reviewer_active_seconds: u64,
    pub cleanup_count: usize,
    pub run_refs: Vec<String>,
    pub event_refs: Vec<String>,
    pub receipt_refs: Vec<String>,
}

impl ForensicMatrixRow {
    fn from_runs(
        arm_ref: String,
        runs: &[&ForensicMatrixRun],
        registered_sample_size: usize,
    ) -> Self {
        let eligible = runs
            .iter()
            .copied()
            .filter(|run| run.eligible_for_identification())
            .collect::<Vec<_>>();
        let hits = eligible
            .iter()
            .copied()
            .filter(|run| run.outcome == ForensicMatrixOutcome::Hit)
            .collect::<Vec<_>>();
        let miss_count = eligible
            .iter()
            .filter(|run| run.outcome == ForensicMatrixOutcome::Miss)
            .count();
        let censor_count = runs.iter().filter(|run| run.censored).count();
        let hit_rate_basis_points =
            (!eligible.is_empty()).then(|| ((hits.len() * 10_000) / eligible.len()) as u32);
        let hit_rate_interval = (!eligible.is_empty()).then(|| {
            wilson_interval(
                hits.len(),
                eligible.len(),
                eligible.len() < registered_sample_size,
            )
        });
        let identification_observations_milliseconds = eligible
            .iter()
            .map(|run| run.identification_milliseconds)
            .collect::<Vec<_>>();
        let identification_token_observations = eligible
            .iter()
            .map(|run| run.identification_tokens)
            .collect::<Vec<_>>();
        let mut durations = hits
            .iter()
            .filter_map(|run| run.identification_milliseconds)
            .collect::<Vec<_>>();
        durations.sort_unstable();
        let p50 = durations
            .get(durations.len().saturating_sub(1) / 2)
            .copied();
        let sample_status = if eligible.len() < registered_sample_size {
            ForensicStatisticStatus::Provisional
        } else {
            ForensicStatisticStatus::Exact
        };
        let p50_identification_milliseconds = if p50.is_some() && miss_count * 2 < eligible.len() {
            ForensicStatistic {
                status: sample_status,
                value: p50,
            }
        } else {
            ForensicStatistic {
                status: ForensicStatisticStatus::NotEstimable,
                value: None,
            }
        };
        let tail_identification_milliseconds =
            if eligible.len() >= registered_sample_size && durations.len() == eligible.len() {
                ForensicStatistic {
                    status: ForensicStatisticStatus::Exact,
                    value: durations.last().copied(),
                }
            } else {
                ForensicStatistic {
                    status: ForensicStatisticStatus::NotEstimable,
                    value: None,
                }
            };
        let supported = runs
            .iter()
            .map(|run| u64::from(run.causal_links_supported))
            .sum::<u64>();
        let required = runs
            .iter()
            .map(|run| u64::from(run.causal_links_required))
            .sum::<u64>();
        let (total_tokens, token_exactness) = aggregate_matrix_truth(
            runs.iter()
                .map(|run| (run.total_tokens, run.token_exactness)),
        );
        let (total_cost_micros, cost_exactness) =
            aggregate_matrix_truth(runs.iter().map(|run| (run.cost_micros, run.cost_exactness)));
        Self {
            arm_ref,
            sample_count: runs.len(),
            eligible_count: eligible.len(),
            hit_count: hits.len(),
            miss_count,
            censor_count,
            hit_rate_basis_points,
            hit_rate_interval,
            identification_observations_milliseconds,
            p50_identification_milliseconds,
            tail_identification_milliseconds,
            identification_token_observations,
            total_tokens,
            token_exactness,
            total_cost_micros,
            cost_exactness,
            causal_coverage_basis_points: (required > 0)
                .then(|| ((supported * 10_000) / required) as u32),
            false_positive_count: runs.iter().map(|run| run.false_positive_count).sum(),
            reviewer_active_seconds: runs
                .iter()
                .filter_map(|run| run.reviewer_active_seconds)
                .sum(),
            cleanup_count: runs.iter().filter(|run| run.cleanup_observed).count(),
            run_refs: runs.iter().map(|run| run.run_ref.clone()).collect(),
            event_refs: runs
                .iter()
                .flat_map(|run| run.event_refs.iter().cloned())
                .collect(),
            receipt_refs: runs
                .iter()
                .flat_map(|run| run.receipt_refs.iter().cloned())
                .collect(),
        }
    }
}

fn aggregate_matrix_truth(
    values: impl Iterator<Item = (Option<u64>, ForensicExactness)>,
) -> (Option<u64>, ForensicExactness) {
    let mut total = 0_u64;
    let mut exactness = ForensicExactness::Exact;
    for (value, value_exactness) in values {
        let Some(value) = value else {
            return (None, ForensicExactness::Unavailable);
        };
        total = total.saturating_add(value);
        exactness = match (exactness, value_exactness) {
            (_, ForensicExactness::UpperBound) | (ForensicExactness::UpperBound, _) => {
                ForensicExactness::UpperBound
            }
            (_, ForensicExactness::Estimated) | (ForensicExactness::Estimated, _) => {
                ForensicExactness::Estimated
            }
            _ => ForensicExactness::Exact,
        };
    }
    (Some(total), exactness)
}

fn wilson_interval(hits: usize, samples: usize, provisional: bool) -> ForensicConfidenceInterval {
    let samples = samples as f64;
    let proportion = hits as f64 / samples;
    let z = 1.959_963_984_540_054_f64;
    let z_squared = z * z;
    let denominator = 1.0 + z_squared / samples;
    let center = (proportion + z_squared / (2.0 * samples)) / denominator;
    let margin = z
        * ((proportion * (1.0 - proportion) / samples + z_squared / (4.0 * samples * samples))
            .sqrt())
        / denominator;
    ForensicConfidenceInterval {
        lower_basis_points: ((center - margin).clamp(0.0, 1.0) * 10_000.0).round() as u32,
        upper_basis_points: ((center + margin).clamp(0.0, 1.0) * 10_000.0).round() as u32,
        provisional,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicMatrixHardGates {
    pub input_complete: bool,
    pub isolation_complete: bool,
    pub clean_control: bool,
    pub evidence_quality: bool,
    pub budget_compliant: bool,
    pub cleanup_complete: bool,
    pub hit_rate_not_regressed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicFindingDivergence {
    pub common_finding_refs: Vec<String>,
    pub unique_finding_refs_by_arm: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicRecallPoint {
    pub budget: u64,
    pub qualified_count: usize,
    pub eligible_count: usize,
    pub recall_basis_points: u32,
}

impl ForensicMatrixHardGates {
    pub fn all_passed(&self) -> bool {
        self.input_complete
            && self.isolation_complete
            && self.clean_control
            && self.evidence_quality
            && self.budget_compliant
            && self.cleanup_complete
            && self.hit_rate_not_regressed
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicParetoStatus {
    Dominates,
    NonDominated,
    Dominated,
    Incomparable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicsMatrixProjection {
    pub schema: String,
    pub matrix_ref: String,
    pub dataset_revision_digest: String,
    pub metric_definition_revision_digest: String,
    pub evaluator_revision_digest: String,
    pub registered_sample_size: usize,
    pub arms: Vec<ForensicMatrixArm>,
    pub runs: Vec<ForensicMatrixRun>,
    pub rows: Vec<ForensicMatrixRow>,
    pub finding_divergence: ForensicFindingDivergence,
    pub recall_time_curve: Vec<ForensicRecallPoint>,
    pub recall_token_curve: Vec<ForensicRecallPoint>,
    pub pareto_frontier_arm_refs: Vec<String>,
    pub hard_gates: ForensicMatrixHardGates,
    pub pareto_status: ForensicParetoStatus,
    pub promoted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdcardClaimRung {
    Source,
    Artifact,
    Generator,
    Exploitability,
    OwnedFixture,
    Fingerprint,
    Entity,
    UnauthorizedMovement,
    Identity,
}

impl ColdcardClaimRung {
    pub const ALL: [Self; 9] = [
        Self::Source,
        Self::Artifact,
        Self::Generator,
        Self::Exploitability,
        Self::OwnedFixture,
        Self::Fingerprint,
        Self::Entity,
        Self::UnauthorizedMovement,
        Self::Identity,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Source => "Source flaw",
            Self::Artifact => "Artifact reality",
            Self::Generator => "Generator behavior",
            Self::Exploitability => "Exploitability",
            Self::OwnedFixture => "Owned fixture",
            Self::Fingerprint => "Program fingerprint",
            Self::Entity => "Entity grouping",
            Self::UnauthorizedMovement => "Unauthorized movement",
            Self::Identity => "Identity attribution",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdcardRungState {
    Missing,
    Provisional,
    Qualified,
    IndependentlyVerified,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColdcardEvidenceRungProjection {
    pub rung: ColdcardClaimRung,
    pub state: ColdcardRungState,
    pub time_to_rung: ForensicMetricTruth,
    pub tokens_to_rung: ForensicMetricTruth,
    pub evidence_refs: Vec<String>,
    pub assumptions: Vec<String>,
    pub verifier_state: String,
    pub non_implications: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdcardTraceKind {
    Source,
    Artifact,
    Generator,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColdcardTraceStepProjection {
    pub sequence: u32,
    pub kind: ColdcardTraceKind,
    pub label: String,
    pub evidence_ref: String,
    pub rule_ref: String,
    pub verifier_state: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdcardAssumptionKind {
    Uid,
    Timer,
    CallTrace,
    Firmware,
    Hardware,
}

impl ColdcardAssumptionKind {
    pub const ALL: [Self; 5] = [
        Self::Uid,
        Self::Timer,
        Self::CallTrace,
        Self::Firmware,
        Self::Hardware,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Uid => "UID",
            Self::Timer => "Timer",
            Self::CallTrace => "Call trace",
            Self::Firmware => "Firmware",
            Self::Hardware => "Hardware",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColdcardAssumptionDiffProjection {
    pub kind: ColdcardAssumptionKind,
    pub baseline: String,
    pub selected: String,
    pub lower_bound_bits: u32,
    pub upper_bound_bits: u32,
    pub evidence_refs: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicControlState {
    Passed,
    Failed,
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColdcardScanRangeProjection {
    pub range_ref: String,
    pub start_height: u64,
    pub end_height: u64,
    pub completed_height: Option<u64>,
    pub checkpoint_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColdcardCandidateStageProjection {
    pub stage_ref: String,
    pub label: String,
    pub count: u64,
    pub source_receipt_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColdcardBaseRateProjection {
    pub stratum_ref: String,
    pub matches_per_million: u64,
    pub exactness: ForensicExactness,
    pub receipt_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColdcardPrivateScanProjection {
    pub boundary_ref: String,
    pub reportable: bool,
    pub ranges: Vec<ColdcardScanRangeProjection>,
    pub transactions_per_second: Option<u64>,
    pub throughput_exactness: ForensicExactness,
    pub restart_state: String,
    pub positive_control: ForensicControlState,
    pub negative_control: ForensicControlState,
    pub base_rates: Vec<ColdcardBaseRateProjection>,
    pub candidate_funnel: Vec<ColdcardCandidateStageProjection>,
    pub missing_data_refs: Vec<String>,
    pub public_transaction_refs: Vec<String>,
    pub candidate_cluster_refs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColdcardGraphHealthProjection {
    pub subject_ref: String,
    pub complete: bool,
    pub source_refs: Vec<String>,
    pub rule_refs: Vec<String>,
    pub missing_provenance_refs: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ColdcardReconciliationStatus {
    Match,
    Drift,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColdcardReconciliationProjection {
    pub metric_ref: String,
    pub status: ColdcardReconciliationStatus,
    pub derived_value: Option<String>,
    pub published_value: Option<String>,
    pub precision_ref: String,
    pub source_refs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColdcardClaimCorrectionProjection {
    pub sequence: u32,
    pub claim_ref: String,
    pub prior_value: String,
    pub corrected_value: String,
    pub reason_ref: String,
    pub appended_evidence_refs: Vec<String>,
    pub affected_projection_refs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColdcardEvidenceWorkspaceProjection {
    pub schema: String,
    pub workspace_ref: String,
    pub run_ref: String,
    pub ladder: Vec<ColdcardEvidenceRungProjection>,
    pub trace: Vec<ColdcardTraceStepProjection>,
    pub assumption_diffs: Vec<ColdcardAssumptionDiffProjection>,
    pub scan: ColdcardPrivateScanProjection,
    pub graph_health: Vec<ColdcardGraphHealthProjection>,
    pub reconciliation: Vec<ColdcardReconciliationProjection>,
    pub corrections: Vec<ColdcardClaimCorrectionProjection>,
}

impl ColdcardEvidenceWorkspaceProjection {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != COLDCARD_EVIDENCE_WORKSPACE_SCHEMA_V1 {
            return Err(ForensicsError::InvalidSchema);
        }
        validate_ref("Coldcard evidence workspace", &self.workspace_ref)?;
        validate_ref("Coldcard evidence run", &self.run_ref)?;
        let serialized = serde_json::to_string(self).map_err(|error| {
            ForensicsError::InvalidColdcardEvidence(format!(
                "the evidence workspace cannot be inspected for secrets: {error}"
            ))
        })?;
        validate_no_secret_markers(&serialized)?;
        if self.trace.len() > 256
            || self.graph_health.len() > 10_000
            || self.reconciliation.len() > 10_000
            || self.corrections.len() > 10_000
        {
            return Err(ForensicsError::InvalidColdcardEvidence(
                "evidence view collections exceed their render bounds".into(),
            ));
        }
        let observed_rungs = self.ladder.iter().map(|rung| rung.rung).collect::<Vec<_>>();
        if observed_rungs != ColdcardClaimRung::ALL {
            return Err(ForensicsError::InvalidColdcardEvidence(
                "the evidence ladder must preserve all nine ordered rungs, including missing rungs"
                    .into(),
            ));
        }
        for rung in &self.ladder {
            rung.time_to_rung.validate()?;
            rung.tokens_to_rung.validate()?;
            validate_bounded_refs("rung evidence", &rung.evidence_refs, 256)?;
            validate_safe_texts(&rung.assumptions)?;
            validate_safe_text(&rung.verifier_state)?;
            validate_safe_texts(&rung.non_implications)?;
            if rung.non_implications.is_empty()
                || (rung.state == ColdcardRungState::Missing && !rung.evidence_refs.is_empty())
                || (rung.state != ColdcardRungState::Missing && rung.evidence_refs.is_empty())
            {
                return Err(ForensicsError::InvalidColdcardEvidence(
                    "rung state, evidence, and non-implications disagree".into(),
                ));
            }
        }
        validate_dense_sequences(self.trace.iter().map(|step| step.sequence))?;
        for step in &self.trace {
            validate_ref("trace evidence", &step.evidence_ref)?;
            validate_ref("trace rule", &step.rule_ref)?;
            validate_safe_text(&step.label)?;
            validate_safe_text(&step.verifier_state)?;
        }
        let observed_assumptions = self
            .assumption_diffs
            .iter()
            .map(|assumption| assumption.kind)
            .collect::<BTreeSet<_>>();
        if observed_assumptions != ColdcardAssumptionKind::ALL.into_iter().collect() {
            return Err(ForensicsError::InvalidColdcardEvidence(
                "UID, timer, call-trace, firmware, and hardware assumptions must remain visible"
                    .into(),
            ));
        }
        for assumption in &self.assumption_diffs {
            if assumption.lower_bound_bits > assumption.upper_bound_bits {
                return Err(ForensicsError::InvalidColdcardEvidence(
                    "entropy sensitivity bounds are inverted".into(),
                ));
            }
            validate_safe_text(&assumption.baseline)?;
            validate_safe_text(&assumption.selected)?;
            validate_bounded_refs("assumption evidence", &assumption.evidence_refs, 64)?;
        }
        self.scan.validate()?;
        for health in &self.graph_health {
            validate_ref("graph subject", &health.subject_ref)?;
            validate_bounded_refs("graph source", &health.source_refs, 256)?;
            validate_bounded_refs("graph rule", &health.rule_refs, 256)?;
            validate_bounded_refs("missing provenance", &health.missing_provenance_refs, 256)?;
            if health.complete
                != (!health.source_refs.is_empty()
                    && !health.rule_refs.is_empty()
                    && health.missing_provenance_refs.is_empty())
            {
                return Err(ForensicsError::InvalidColdcardEvidence(
                    "graph health must fail for every provenance gap".into(),
                ));
            }
        }
        for item in &self.reconciliation {
            validate_ref("reconciliation metric", &item.metric_ref)?;
            validate_ref("reconciliation precision", &item.precision_ref)?;
            validate_bounded_refs("reconciliation source", &item.source_refs, 64)?;
            if let Some(value) = &item.derived_value {
                validate_safe_text(value)?;
            }
            if let Some(value) = &item.published_value {
                validate_safe_text(value)?;
            }
            if item.status == ColdcardReconciliationStatus::Unavailable
                && (item.derived_value.is_some() || item.published_value.is_some())
            {
                return Err(ForensicsError::InvalidColdcardEvidence(
                    "unavailable reconciliation cannot invent a comparable value".into(),
                ));
            }
            if item.status != ColdcardReconciliationStatus::Unavailable
                && (item.derived_value.is_none() || item.published_value.is_none())
            {
                return Err(ForensicsError::InvalidColdcardEvidence(
                    "matched or drifting reconciliation must retain both original values".into(),
                ));
            }
        }
        validate_dense_sequences(
            self.corrections
                .iter()
                .map(|correction| correction.sequence),
        )?;
        for correction in &self.corrections {
            validate_ref("corrected claim", &correction.claim_ref)?;
            validate_ref("correction reason", &correction.reason_ref)?;
            validate_safe_text(&correction.prior_value)?;
            validate_safe_text(&correction.corrected_value)?;
            validate_bounded_refs(
                "appended correction evidence",
                &correction.appended_evidence_refs,
                256,
            )?;
            validate_bounded_refs(
                "affected correction projection",
                &correction.affected_projection_refs,
                256,
            )?;
            if correction.appended_evidence_refs.is_empty()
                || correction.affected_projection_refs.is_empty()
            {
                return Err(ForensicsError::InvalidColdcardEvidence(
                    "corrections must append evidence and identify affected projections".into(),
                ));
            }
        }
        Ok(())
    }
}

impl ColdcardPrivateScanProjection {
    fn validate(&self) -> Result<(), ForensicsError> {
        if self.boundary_ref != "boundary.omega.private-forensic-run.v1" || self.reportable {
            return Err(ForensicsError::InvalidColdcardEvidence(
                "transaction and cluster evidence must remain private and non-reportable".into(),
            ));
        }
        if (self.transactions_per_second.is_none())
            != (self.throughput_exactness == ForensicExactness::Unavailable)
        {
            return Err(ForensicsError::InvalidColdcardEvidence(
                "scan throughput value and exactness disagree".into(),
            ));
        }
        if self.ranges.is_empty() || self.candidate_funnel.is_empty() {
            return Err(ForensicsError::InvalidColdcardEvidence(
                "scan ranges and candidate funnel must remain visible".into(),
            ));
        }
        if self.ranges.len() > 10_000
            || self.candidate_funnel.len() > 256
            || self.base_rates.len() > 10_000
            || self.missing_data_refs.len() > 10_000
            || self.public_transaction_refs.len() > 100_000
            || self.candidate_cluster_refs.len() > 100_000
        {
            return Err(ForensicsError::InvalidColdcardEvidence(
                "private scan collections exceed their display bounds".into(),
            ));
        }
        for range in &self.ranges {
            validate_ref("scan range", &range.range_ref)?;
            if range.start_height > range.end_height
                || range
                    .completed_height
                    .is_some_and(|height| height < range.start_height || height > range.end_height)
                || (range.completed_height.is_some() != range.checkpoint_ref.is_some())
            {
                return Err(ForensicsError::InvalidColdcardEvidence(
                    "scan range, progress, and checkpoint disagree".into(),
                ));
            }
        }
        let mut previous_count = None;
        for stage in &self.candidate_funnel {
            validate_ref("candidate stage", &stage.stage_ref)?;
            validate_ref("candidate receipt", &stage.source_receipt_ref)?;
            validate_safe_text(&stage.label)?;
            if previous_count.is_some_and(|count| stage.count > count) {
                return Err(ForensicsError::InvalidColdcardEvidence(
                    "candidate funnel counts cannot increase".into(),
                ));
            }
            previous_count = Some(stage.count);
        }
        for rate in &self.base_rates {
            validate_ref("base-rate stratum", &rate.stratum_ref)?;
            validate_ref("base-rate receipt", &rate.receipt_ref)?;
            if rate.exactness == ForensicExactness::Unavailable {
                return Err(ForensicsError::InvalidColdcardEvidence(
                    "unavailable base rates cannot carry a numeric zero".into(),
                ));
            }
        }
        validate_safe_text(&self.restart_state)?;
        validate_bounded_refs("scan missing data", &self.missing_data_refs, 256)?;
        validate_bounded_refs(
            "private transaction",
            &self.public_transaction_refs,
            100_000,
        )?;
        validate_bounded_refs(
            "private candidate cluster",
            &self.candidate_cluster_refs,
            100_000,
        )?;
        Ok(())
    }
}

fn validate_dense_sequences(values: impl Iterator<Item = u32>) -> Result<(), ForensicsError> {
    for (index, sequence) in values.enumerate() {
        if sequence as usize != index + 1 {
            return Err(ForensicsError::InvalidColdcardEvidence(
                "append-only sequences must be dense and ordered".into(),
            ));
        }
    }
    Ok(())
}

fn validate_safe_texts(values: &[String]) -> Result<(), ForensicsError> {
    for value in values {
        validate_safe_text(value)?;
    }
    Ok(())
}

fn validate_safe_text(value: &str) -> Result<(), ForensicsError> {
    let normalized = value.to_ascii_lowercase();
    let words = value.split_whitespace().collect::<Vec<_>>();
    let looks_like_mnemonic = matches!(words.len(), 12 | 15 | 18 | 21 | 24)
        && words
            .iter()
            .all(|word| word.bytes().all(|byte| byte.is_ascii_lowercase()));
    let forbidden = [
        "xprv",
        "private-key:",
        "node-cookie:",
        "rpc-credential:",
        "password=",
        "seed-phrase:",
    ];
    if value.trim().is_empty()
        || value.len() > 2_048
        || looks_like_mnemonic
        || forbidden.iter().any(|marker| normalized.contains(marker))
    {
        return Err(ForensicsError::InvalidColdcardEvidence(
            "secret-bearing or unbounded evidence cannot be rendered or exported".into(),
        ));
    }
    Ok(())
}

fn validate_no_secret_markers(value: &str) -> Result<(), ForensicsError> {
    let normalized = value.to_ascii_lowercase();
    for marker in [
        "xprv",
        "private-key:",
        "node-cookie:",
        "rpc-credential:",
        "password=",
        "seed-phrase:",
    ] {
        if normalized.contains(marker) {
            return Err(ForensicsError::InvalidColdcardEvidence(
                "secret-bearing evidence cannot be rendered or exported".into(),
            ));
        }
    }
    Ok(())
}

impl ForensicsMatrixProjection {
    pub fn rebuild(
        matrix_ref: String,
        dataset_revision_digest: String,
        metric_definition_revision_digest: String,
        evaluator_revision_digest: String,
        registered_sample_size: usize,
        arms: Vec<ForensicMatrixArm>,
        runs: Vec<ForensicMatrixRun>,
        hard_gates: ForensicMatrixHardGates,
        pareto_status: ForensicParetoStatus,
        request_promotion: bool,
    ) -> Result<Self, ForensicsError> {
        if registered_sample_size < 3 || arms.is_empty() || runs.is_empty() {
            return Err(ForensicsError::InvalidMatrix(
                "matrix requires arms, runs, and a registered sample size of at least three".into(),
            ));
        }
        validate_ref("matrix", &matrix_ref)?;
        for digest in [
            &dataset_revision_digest,
            &metric_definition_revision_digest,
            &evaluator_revision_digest,
        ] {
            validate_digest("matrix revision", digest)?;
        }
        let mut arm_refs = BTreeSet::new();
        let mut isolation_refs = BTreeSet::new();
        for arm in &arms {
            arm.validate()?;
            if !arm_refs.insert(arm.arm_ref.as_str()) {
                return Err(ForensicsError::InvalidMatrix("duplicate matrix arm".into()));
            }
            for isolation_ref in [
                &arm.writable_disk_ref,
                &arm.provider_session_ref,
                &arm.auth_home_ref,
                &arm.environment_ref,
                &arm.worker_state_ref,
                &arm.source_bundle_digest,
            ] {
                if !isolation_refs.insert(isolation_ref.as_str()) {
                    return Err(ForensicsError::InvalidMatrix(
                        "matrix arms cannot share writable or hidden worker state".into(),
                    ));
                }
            }
        }
        let mut run_digests = BTreeSet::new();
        for run in &runs {
            run.validate()?;
            if !arm_refs.contains(run.arm_ref.as_str()) || !run_digests.insert(&run.run_digest) {
                return Err(ForensicsError::InvalidMatrix(
                    "runs must bind unique digests to declared arms".into(),
                ));
            }
        }
        let rows = arms
            .iter()
            .map(|arm| {
                let arm_runs = runs
                    .iter()
                    .filter(|run| run.arm_ref == arm.arm_ref)
                    .collect::<Vec<_>>();
                ForensicMatrixRow::from_runs(arm.arm_ref.clone(), &arm_runs, registered_sample_size)
            })
            .collect::<Vec<_>>();
        let finding_divergence = finding_divergence(&arms, &runs);
        let eligible_runs = runs
            .iter()
            .filter(|run| run.eligible_for_identification())
            .collect::<Vec<_>>();
        let recall_time_curve = recall_curve(&eligible_runs, |run| run.identification_milliseconds);
        let recall_token_curve = recall_curve(&eligible_runs, |run| run.identification_tokens);
        let pareto_frontier_arm_refs = pareto_frontier(&rows);
        let observed_quality = runs.iter().all(|run| {
            run.false_positive_count == 0
                && run.cleanup_observed
                && run.budget_compliant
                && (run.outcome != ForensicMatrixOutcome::Hit
                    || run.causal_links_supported == run.causal_links_required)
        });
        let promotion_safe = hard_gates.all_passed()
            && observed_quality
            && matches!(
                pareto_status,
                ForensicParetoStatus::Dominates | ForensicParetoStatus::NonDominated
            );
        let projection = Self {
            schema: MATRIX_PROJECTION_SCHEMA_V1.into(),
            matrix_ref,
            dataset_revision_digest,
            metric_definition_revision_digest,
            evaluator_revision_digest,
            registered_sample_size,
            arms,
            runs,
            rows,
            finding_divergence,
            recall_time_curve,
            recall_token_curve,
            pareto_frontier_arm_refs,
            hard_gates,
            pareto_status,
            promoted: request_promotion && promotion_safe,
        };
        projection.validate()?;
        Ok(projection)
    }

    pub fn validate(&self) -> Result<(), ForensicsError> {
        let observed_quality = self.runs.iter().all(|run| {
            run.false_positive_count == 0
                && run.cleanup_observed
                && run.budget_compliant
                && (run.outcome != ForensicMatrixOutcome::Hit
                    || run.causal_links_supported == run.causal_links_required)
        });
        if self.schema != MATRIX_PROJECTION_SCHEMA_V1
            || self.rows.len() != self.arms.len()
            || self.pareto_frontier_arm_refs.is_empty()
            || (self.promoted
                && (!self.hard_gates.all_passed()
                    || !observed_quality
                    || !matches!(
                        self.pareto_status,
                        ForensicParetoStatus::Dominates | ForensicParetoStatus::NonDominated
                    )))
        {
            return Err(ForensicsError::InvalidMatrix(
                "matrix projection or promotion state is inconsistent".into(),
            ));
        }
        Ok(())
    }
}

fn finding_divergence(
    arms: &[ForensicMatrixArm],
    runs: &[ForensicMatrixRun],
) -> ForensicFindingDivergence {
    let by_arm = arms
        .iter()
        .filter_map(|arm| {
            let eligible_runs = runs
                .iter()
                .filter(|run| run.arm_ref == arm.arm_ref && run.eligible_for_identification())
                .collect::<Vec<_>>();
            (!eligible_runs.is_empty()).then(|| {
                (
                    arm.arm_ref.clone(),
                    eligible_runs
                        .into_iter()
                        .flat_map(|run| run.qualified_finding_refs.iter().cloned())
                        .collect::<BTreeSet<_>>(),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let common_finding_refs = by_arm
        .values()
        .cloned()
        .reduce(|left, right| left.intersection(&right).cloned().collect())
        .unwrap_or_default()
        .into_iter()
        .collect();
    let unique_finding_refs_by_arm = by_arm
        .iter()
        .map(|(arm_ref, findings)| {
            let other_findings = by_arm
                .iter()
                .filter(|(other_ref, _)| *other_ref != arm_ref)
                .flat_map(|(_, values)| values.iter().cloned())
                .collect::<BTreeSet<_>>();
            (
                arm_ref.clone(),
                findings.difference(&other_findings).cloned().collect(),
            )
        })
        .collect();
    ForensicFindingDivergence {
        common_finding_refs,
        unique_finding_refs_by_arm,
    }
}

fn recall_curve(
    eligible_runs: &[&ForensicMatrixRun],
    observation: impl Fn(&ForensicMatrixRun) -> Option<u64>,
) -> Vec<ForensicRecallPoint> {
    let mut budgets = eligible_runs
        .iter()
        .filter_map(|run| observation(run))
        .collect::<Vec<_>>();
    budgets.sort_unstable();
    budgets.dedup();
    budgets
        .into_iter()
        .map(|budget| {
            let qualified_count = eligible_runs
                .iter()
                .filter(|run| observation(run).is_some_and(|value| value <= budget))
                .count();
            ForensicRecallPoint {
                budget,
                qualified_count,
                eligible_count: eligible_runs.len(),
                recall_basis_points: ((qualified_count * 10_000) / eligible_runs.len()) as u32,
            }
        })
        .collect()
}

fn pareto_frontier(rows: &[ForensicMatrixRow]) -> Vec<String> {
    rows.iter()
        .filter(|candidate| candidate.hit_rate_basis_points.is_some())
        .filter(|candidate| {
            !rows.iter().any(|other| {
                if other.arm_ref == candidate.arm_ref || other.hit_rate_basis_points.is_none() {
                    return false;
                }
                let candidate_hit = candidate.hit_rate_basis_points.unwrap_or_default();
                let other_hit = other.hit_rate_basis_points.unwrap_or_default();
                let candidate_causal = candidate.causal_coverage_basis_points.unwrap_or_default();
                let other_causal = other.causal_coverage_basis_points.unwrap_or_default();
                let candidate_time = candidate
                    .p50_identification_milliseconds
                    .value
                    .unwrap_or(u64::MAX);
                let other_time = other
                    .p50_identification_milliseconds
                    .value
                    .unwrap_or(u64::MAX);
                let no_worse = other_hit >= candidate_hit
                    && other_causal >= candidate_causal
                    && other_time <= candidate_time
                    && other.total_tokens.unwrap_or(u64::MAX)
                        <= candidate.total_tokens.unwrap_or(u64::MAX)
                    && other.total_cost_micros.unwrap_or(u64::MAX)
                        <= candidate.total_cost_micros.unwrap_or(u64::MAX)
                    && other.false_positive_count <= candidate.false_positive_count
                    && other.reviewer_active_seconds <= candidate.reviewer_active_seconds;
                let strictly_better = other_hit > candidate_hit
                    || other_causal > candidate_causal
                    || other_time < candidate_time
                    || other.total_tokens.unwrap_or(u64::MAX)
                        < candidate.total_tokens.unwrap_or(u64::MAX)
                    || other.total_cost_micros.unwrap_or(u64::MAX)
                        < candidate.total_cost_micros.unwrap_or(u64::MAX)
                    || other.false_positive_count < candidate.false_positive_count
                    || other.reviewer_active_seconds < candidate.reviewer_active_seconds;
                no_worse && strictly_better
            })
        })
        .map(|row| row.arm_ref.clone())
        .collect()
}

fn validate_bounded_refs(
    label: &str,
    refs: &[String],
    maximum: usize,
) -> Result<(), ForensicsError> {
    if refs.len() > maximum {
        return Err(ForensicsError::InvalidReview(format!(
            "{label} refs exceed the bound"
        )));
    }
    for value in refs {
        validate_ref(label, value)?;
    }
    Ok(())
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
    #[error("the forensic review projection is invalid: {0}")]
    InvalidReview(String),
    #[error("the forensic prompt artifact is invalid: {0}")]
    InvalidPrompt(String),
    #[error("clone an active prompt before editing or saving")]
    NoPromptDraft,
    #[error("the prompt schema, tool surface, or profile is incompatible")]
    IncompatiblePrompt,
    #[error("the forensic run matrix is invalid: {0}")]
    InvalidMatrix(String),
    #[error("the forensic publication gate is invalid: {0}")]
    InvalidPublicationGate(String),
    #[error("the Coldcard evidence workspace is invalid: {0}")]
    InvalidColdcardEvidence(String),
    #[error("the entropy repository run is invalid: {0}")]
    InvalidEntropyRun(String),
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

    fn review_projection() -> ForensicsReviewProjection {
        ForensicsReviewProjection {
            schema: REVIEW_PROJECTION_SCHEMA_V1.into(),
            review_ref: "review.forensic.coldcard.fixture".into(),
            run_ref: "run.forensic.fixture".into(),
            prompt_digest:
                "sha256:e59c827a678c1f3867ac410b7af729587e7700ac6fec1830b370a77b2c9e8610".into(),
            repository_ref: COLDCARD_REPOSITORY_REF.into(),
            commit: COLDCARD_VULNERABLE_COMMIT.into(),
            coverage_status: CoverageStatus::Complete,
            outcome: ForensicReviewOutcome::Completed,
            budget_state: ForensicBudgetState::WithinBudget,
            findings: vec![ForensicFindingProjection {
                finding_ref: "finding.coldcard.rng-fallback".into(),
                claim_ref: "claim.coldcard.source-flaw".into(),
                title: "Fallback entropy can repeat wallet secrets".into(),
                impact: "A repeated fallback state can reproduce generated wallet material.".into(),
                severity: "critical".into(),
                claim_state: "qualified".into(),
                evidence_tier: ForensicEvidenceTier::Executed,
                duplicate_group_ref: Some("duplicate-group.coldcard.rng-fallback".into()),
                source_refs: vec![ForensicSourceCitation {
                    source_ref: "source.coldcard.shared.utils.42".into(),
                    path: "shared/utils.py".into(),
                    symbol: Some("get_random_bytes".into()),
                    start_line: 42,
                    end_line: 57,
                    commit: COLDCARD_VULNERABLE_COMMIT.into(),
                }],
                causal_path: vec![ForensicCausalLink {
                    sequence: 1,
                    proposition: "The fallback admits insufficient entropy.".into(),
                    evidence_refs: vec!["evidence.coldcard.source".into()],
                    supported: true,
                }],
                evidence_receipts: vec![ForensicEvidenceReceiptProjection {
                    receipt_ref: "receipt.coldcard.execution".into(),
                    evidence_tier: ForensicEvidenceTier::Executed,
                    outcome: "succeeded".into(),
                    artifact_ref: Some("artifact.coldcard.poc".into()),
                    verifier_verdict: Some("confirmed".into()),
                    observed_at: "2026-08-01T10:03:00.000Z".into(),
                }],
                poc_ref: Some("artifact.coldcard.poc".into()),
                submitted_at: "2026-08-01T10:02:00.000Z".into(),
            }],
            hypotheses: vec![ForensicHypothesisProjection {
                hypothesis_ref: "hypothesis.coldcard.entropy-source".into(),
                suspected_mechanism: "A second entropy source may share the same state.".into(),
                supporting_refs: vec!["source.coldcard.shared.utils.42".into()],
                missing_evidence: vec!["Executed cross-device reproduction".into()],
                next_check: "Run the generator trace against two owned fixtures.".into(),
                consequence_if_true: "More devices could share a recoverable state.".into(),
                state: "unverified".into(),
                submitted_at: "2026-08-01T10:02:30.000Z".into(),
            }],
            metrics: vec![
                ForensicMetricTruth {
                    metric_ref: "metric.time-to-qualified-identification".into(),
                    label: "Time to qualified identification".into(),
                    unit: "ms".into(),
                    value: Some(120_000),
                    exactness: ForensicExactness::Exact,
                    unavailable_reason_ref: None,
                    source_event_refs: vec!["event.finding.coldcard".into()],
                    source_receipt_refs: Vec::new(),
                },
                ForensicMetricTruth {
                    metric_ref: "metric.cost-to-qualified-identification".into(),
                    label: "Cost to qualified identification".into(),
                    unit: "µUSD".into(),
                    value: None,
                    exactness: ForensicExactness::Unavailable,
                    unavailable_reason_ref: Some("reason.provider-cost-unavailable".into()),
                    source_event_refs: Vec::new(),
                    source_receipt_refs: Vec::new(),
                },
            ],
            lifecycle: vec![
                ForensicLifecycleStage {
                    stage_ref: "stage.request-admitted".into(),
                    label: "Request admitted".into(),
                    state: ForensicLifecycleState::Succeeded,
                    observed_at: Some("2026-08-01T10:00:00.000Z".into()),
                    receipt_ref: Some("receipt.request-admitted".into()),
                },
                ForensicLifecycleStage {
                    stage_ref: "stage.cleanup-observed".into(),
                    label: "Cleanup observed".into(),
                    state: ForensicLifecycleState::Succeeded,
                    observed_at: Some("2026-08-01T10:04:00.000Z".into()),
                    receipt_ref: Some("receipt.cleanup-observed".into()),
                },
            ],
            placement_ref: "placement.forensic.fixture".into(),
            sandbox_ref: "sandbox.forensic.fixture".into(),
            resource_generation: 1,
            cleanup_state: "observed_zero_residue".into(),
            cleanup_receipt_ref: Some("receipt.cleanup-observed".into()),
            decisions: Vec::new(),
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
        let json = serde_json::to_string(&(
            preflight(ColdcardBenchmarkArm::Fixed),
            run,
            review_projection(),
        ))
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
            "compiled_prompt",
            "prompt_text",
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

    #[test]
    fn review_projection_preserves_distinct_claim_evidence_metric_and_cleanup_truth() {
        let review = review_projection();
        review.validate().expect("valid review projection");
        assert_eq!(
            review.findings[0].evidence_tier,
            ForensicEvidenceTier::Executed
        );
        assert_eq!(review.hypotheses[0].state, "unverified");
        assert_eq!(review.metrics[1].display_value(), "Unavailable");
        assert_eq!(review.cleanup_state, "observed_zero_residue");
    }

    #[test]
    fn unavailable_metrics_cannot_render_as_zero_and_incomplete_runs_cannot_render_complete() {
        let mut review = review_projection();
        review.metrics[1].value = Some(0);
        assert!(matches!(
            review.validate(),
            Err(ForensicsError::InvalidReview(_))
        ));

        let mut review = review_projection();
        review.coverage_status = CoverageStatus::Incomplete;
        assert!(matches!(
            review.validate(),
            Err(ForensicsError::InvalidReview(_))
        ));
    }

    #[test]
    fn review_decisions_append_without_mutating_the_original_finding() {
        let mut review = review_projection();
        let finding = review.findings[0].clone();
        review
            .append_decision(
                &finding.finding_ref,
                ForensicReviewDecisionKind::Correct,
                "Narrow the impact to affected fallback builds.".into(),
                "reviewer.omega.operator".into(),
                "2026-08-01T10:05:00.000Z".into(),
            )
            .expect("append decision");
        review.validate().expect("review remains valid");
        assert_eq!(review.findings[0], finding);
        assert_eq!(review.decisions.len(), 1);
        assert_eq!(review.decisions[0].sequence, 1);
    }

    #[test]
    fn citations_cannot_escape_or_drift_from_the_pinned_source() {
        let mut review = review_projection();
        review.findings[0].source_refs[0].path = "../private/key".into();
        assert!(matches!(
            review.validate(),
            Err(ForensicsError::InvalidReview(_))
        ));

        let mut review = review_projection();
        review.findings[0].source_refs[0].commit = COLDCARD_FIXED_COMMIT.into();
        assert!(matches!(
            review.validate(),
            Err(ForensicsError::InvalidReview(_))
        ));
    }

    #[test]
    fn cancelled_missed_failed_censored_and_cleanup_failed_runs_remain_reviewable() {
        for outcome in [
            ForensicReviewOutcome::Cancelled,
            ForensicReviewOutcome::Missed,
            ForensicReviewOutcome::Failed,
            ForensicReviewOutcome::Censored,
            ForensicReviewOutcome::CleanupFailed,
        ] {
            let mut review = review_projection();
            review.outcome = outcome;
            if outcome == ForensicReviewOutcome::CleanupFailed {
                review.cleanup_state = "failed".into();
                review.cleanup_receipt_ref = None;
                review.lifecycle[1].state = ForensicLifecycleState::Failed;
            }
            review.validate().expect("terminal run remains reviewable");
        }
    }

    #[test]
    fn prompt_digest_matches_the_openagents_canonical_contract() {
        let artifact =
            baseline_forensic_prompt("2026-08-01T10:00:00.000Z".into()).expect("baseline prompt");
        assert_eq!(
            artifact.canonical_digest,
            "sha256:e59c827a678c1f3867ac410b7af729587e7700ac6fec1830b370a77b2c9e8610"
        );
        artifact.validate().expect("canonical artifact");
    }

    #[test]
    fn prompt_edits_are_save_as_candidates_with_lineage_and_pointer_only_reverts() {
        let active =
            baseline_forensic_prompt("2026-08-01T10:00:00.000Z".into()).expect("baseline prompt");
        let active_digest = active.canonical_digest.clone();
        let mut workspace = ForensicPromptWorkspace::new(active).expect("prompt workspace");
        workspace
            .clone_active(
                "prompt.forensic.omega.candidate.v2".into(),
                "2026-08-01T10:01:00.000Z".into(),
            )
            .expect("clone active");
        let mut prompt_ir = workspace.draft().expect("draft").prompt_ir.clone();
        prompt_ir.uncertainty_policy = "Retain unsupported claims only as typed hypotheses.".into();
        workspace.update_draft_ir(prompt_ir).expect("edit draft");
        let candidate_ref = workspace.save_draft().expect("save candidate");
        assert_eq!(workspace.active().canonical_digest, active_digest);
        workspace
            .activate(&candidate_ref)
            .expect("activate candidate");
        let candidate_digest = workspace.active().canonical_digest.clone();
        assert_ne!(candidate_digest, active_digest);
        workspace
            .bind_run("run.prompt-candidate".into())
            .expect("bind run");
        workspace
            .activate("prompt.forensic.omega.baseline.v1")
            .expect("revert pointer");
        assert_eq!(workspace.candidates().count(), 2);
        assert_eq!(
            workspace.run_prompt_digest("run.prompt-candidate"),
            Some(candidate_digest.as_str())
        );
    }

    #[test]
    fn semantic_diff_classifies_sections_examples_schemas_tools_parameters_and_policies() {
        let active =
            baseline_forensic_prompt("2026-08-01T10:00:00.000Z".into()).expect("baseline prompt");
        let mut workspace = ForensicPromptWorkspace::new(active).expect("workspace");
        workspace
            .clone_active(
                "prompt.forensic.omega.diff.v2".into(),
                "2026-08-01T10:01:00.000Z".into(),
            )
            .expect("clone");
        let draft = workspace.draft.as_mut().expect("draft");
        draft.prompt_ir.role.push_str(" Review dependencies.");
        draft.prompt_ir.finding_schema_ref = "schema.finding.candidate".into();
        draft
            .prompt_ir
            .tool_policy_refs
            .push("tool.symbol.search".into());
        draft
            .prompt_ir
            .context_policy
            .push_str(" Prefer concise output.");
        draft.example_refs.push("example.entropy.v2".into());
        draft
            .parameter_refs
            .push("parameter.temperature.zero".into());
        draft.canonical_digest = draft.computed_digest().expect("digest");
        let kinds = workspace
            .semantic_diff()
            .expect("diff")
            .into_iter()
            .map(|change| change.kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            kinds,
            BTreeSet::from([
                PromptChangeKind::Section,
                PromptChangeKind::Example,
                PromptChangeKind::Schema,
                PromptChangeKind::Tool,
                PromptChangeKind::Parameter,
                PromptChangeKind::Policy,
            ])
        );
    }

    #[test]
    fn prompt_prose_cannot_grant_authority_and_incompatible_profiles_fail_before_launch() {
        let mut artifact =
            baseline_forensic_prompt("2026-08-01T10:00:00.000Z".into()).expect("baseline prompt");
        artifact.prompt_ir.context_policy =
            "Enable public Internet, raise the token budget, mutate the checkout, and report publicly.".into();
        artifact.canonical_digest = artifact.computed_digest().expect("digest");
        let workspace = ForensicPromptWorkspace::new(artifact).expect("workspace");
        let profile = PromptCompatibilityProfile {
            prompt_artifact_ref: workspace.active().prompt_artifact_ref.clone(),
            finding_schema_ref: FORENSIC_FINDING_SCHEMA_V1.into(),
            hypothesis_schema_ref: FORENSIC_HYPOTHESIS_SCHEMA_V1.into(),
            admitted_tool_refs: vec!["tool.source.read".into()],
            runtime_tool_refs: vec!["tool.source.read".into()],
            compatibility_refs: vec!["compatibility.loupe.v1".into()],
        };
        assert_eq!(
            workspace.check_compatibility(&profile),
            Err(ForensicsError::IncompatiblePrompt)
        );
    }

    fn matrix_arm(suffix: &str, digest_character: char) -> ForensicMatrixArm {
        ForensicMatrixArm {
            arm_ref: format!("arm.forensic.{suffix}"),
            model_family_ref: format!("model-family.forensic.{suffix}"),
            role_ref: format!("role.forensic.{suffix}"),
            prompt_digest: digest(digest_character),
            model_digest: digest('1'),
            effort_ref: "effort.high".into(),
            scope_ref: "scope.entropy".into(),
            dependency_policy_ref: "dependency.pinned-recursive".into(),
            random_seed: u64::from(digest_character as u32),
            tool_surface_digest: digest('2'),
            analysis_mode_ref: "analysis.static-and-build".into(),
            worker_image_digest: digest('3'),
            worker_profile_digest: digest('4'),
            source_bundle_digest: digest(digest_character),
            writable_disk_ref: format!("disk.{suffix}"),
            provider_session_ref: format!("provider-session.{suffix}"),
            auth_home_ref: format!("auth-home.{suffix}"),
            environment_ref: format!("environment.{suffix}"),
            worker_state_ref: format!("worker-state.{suffix}"),
        }
    }

    fn matrix_run(
        suffix: &str,
        arm_ref: &str,
        outcome: ForensicMatrixOutcome,
    ) -> ForensicMatrixRun {
        let hit = outcome == ForensicMatrixOutcome::Hit;
        let miss = outcome == ForensicMatrixOutcome::Miss;
        ForensicMatrixRun {
            run_ref: format!("run.matrix.{suffix}"),
            run_digest: digest(match suffix {
                "hit-a" => '5',
                "hit-b" => '6',
                "miss" => '7',
                _ => '8',
            }),
            arm_ref: arm_ref.into(),
            dataset_split: if outcome == ForensicMatrixOutcome::NotEligible {
                ForensicDatasetSplit::Development
            } else {
                ForensicDatasetSplit::Holdout
            },
            population: if outcome == ForensicMatrixOutcome::NotEligible {
                ForensicMatrixPopulation::Incomplete
            } else {
                ForensicMatrixPopulation::Vulnerable
            },
            coverage_status: if outcome == ForensicMatrixOutcome::NotEligible {
                CoverageStatus::Incomplete
            } else {
                CoverageStatus::Complete
            },
            outcome,
            censored: miss,
            censor_at_milliseconds: miss.then_some(60_000),
            identification_milliseconds: hit.then_some(if suffix == "hit-a" {
                8_000
            } else {
                12_000
            }),
            identification_tokens: hit.then_some(if suffix == "hit-a" { 2_000 } else { 3_000 }),
            total_tokens: Some(if miss { 10_000 } else { 4_000 }),
            token_exactness: ForensicExactness::Exact,
            cost_micros: Some(if miss { 500_000 } else { 200_000 }),
            cost_exactness: ForensicExactness::Exact,
            causal_links_supported: if hit { 4 } else { 0 },
            causal_links_required: if hit { 4 } else { 0 },
            false_positive_count: 0,
            reviewer_active_seconds: hit.then_some(90),
            budget_compliant: true,
            cleanup_observed: true,
            qualified_finding_refs: if hit {
                vec!["finding.coldcard.entropy-fallback".into()]
            } else {
                Vec::new()
            },
            failure_refs: if miss {
                vec![format!("failure.{suffix}")]
            } else {
                Vec::new()
            },
            event_refs: vec![format!("event.{suffix}")],
            receipt_refs: vec![format!("receipt.{suffix}")],
        }
    }

    fn matrix_gates() -> ForensicMatrixHardGates {
        ForensicMatrixHardGates {
            input_complete: true,
            isolation_complete: true,
            clean_control: true,
            evidence_quality: true,
            budget_compliant: true,
            cleanup_complete: true,
            hit_rate_not_regressed: true,
        }
    }

    fn matrix_projection(
        request_promotion: bool,
    ) -> Result<ForensicsMatrixProjection, ForensicsError> {
        let candidate = matrix_arm("candidate", 'a');
        let incomplete = matrix_arm("incomplete", 'b');
        ForensicsMatrixProjection::rebuild(
            "matrix.forensic.fixture".into(),
            digest('c'),
            digest('d'),
            digest('e'),
            10,
            vec![candidate.clone(), incomplete.clone()],
            vec![
                matrix_run("hit-a", &candidate.arm_ref, ForensicMatrixOutcome::Hit),
                matrix_run("hit-b", &candidate.arm_ref, ForensicMatrixOutcome::Hit),
                matrix_run("miss", &candidate.arm_ref, ForensicMatrixOutcome::Miss),
                matrix_run(
                    "incomplete",
                    &incomplete.arm_ref,
                    ForensicMatrixOutcome::NotEligible,
                ),
            ],
            matrix_gates(),
            ForensicParetoStatus::NonDominated,
            request_promotion,
        )
    }

    #[test]
    fn matrix_retains_misses_censoring_small_samples_and_population_separation() {
        let matrix = matrix_projection(false).expect("matrix");
        let candidate = &matrix.rows[0];
        assert_eq!((candidate.hit_count, candidate.miss_count), (2, 1));
        assert_eq!(candidate.identification_observations_milliseconds.len(), 3);
        assert_eq!(candidate.identification_observations_milliseconds[2], None);
        assert_eq!(
            candidate.p50_identification_milliseconds.status,
            ForensicStatisticStatus::Provisional
        );
        assert_eq!(
            candidate.tail_identification_milliseconds.status,
            ForensicStatisticStatus::NotEstimable
        );
        assert_eq!(matrix.rows[1].eligible_count, 0);
        assert_eq!(matrix.rows[1].sample_count, 1);
        assert_eq!(candidate.event_refs.len(), 3);
        assert_eq!(candidate.receipt_refs.len(), 3);
        assert_eq!(matrix.recall_time_curve.len(), 2);
        assert_eq!(matrix.recall_time_curve[1].recall_basis_points, 6_666);
        assert_eq!(
            matrix.finding_divergence.common_finding_refs,
            vec!["finding.coldcard.entropy-fallback"]
        );
        assert_eq!(
            matrix.pareto_frontier_arm_refs,
            vec!["arm.forensic.candidate"]
        );
    }

    #[test]
    fn matrix_rejects_shared_writable_or_hidden_state_between_arms() {
        let first = matrix_arm("first", 'a');
        let mut second = matrix_arm("second", 'b');
        second.auth_home_ref = first.auth_home_ref.clone();
        let result = ForensicsMatrixProjection::rebuild(
            "matrix.forensic.isolation".into(),
            digest('c'),
            digest('d'),
            digest('e'),
            3,
            vec![first.clone(), second],
            vec![matrix_run(
                "hit-a",
                &first.arm_ref,
                ForensicMatrixOutcome::Hit,
            )],
            matrix_gates(),
            ForensicParetoStatus::NonDominated,
            false,
        );
        assert!(matches!(result, Err(ForensicsError::InvalidMatrix(_))));
    }

    #[test]
    fn quality_cleanup_budget_or_hard_gate_regressions_block_promotion() {
        assert!(matrix_projection(true).expect("safe matrix").promoted);

        let candidate = matrix_arm("candidate", 'a');
        let mut run = matrix_run("hit-a", &candidate.arm_ref, ForensicMatrixOutcome::Hit);
        run.false_positive_count = 1;
        let matrix = ForensicsMatrixProjection::rebuild(
            "matrix.forensic.blocked".into(),
            digest('c'),
            digest('d'),
            digest('e'),
            3,
            vec![candidate],
            vec![run],
            matrix_gates(),
            ForensicParetoStatus::Dominates,
            true,
        )
        .expect("truthful blocked matrix");
        assert!(!matrix.promoted);
    }

    #[test]
    fn unavailable_matrix_usage_never_becomes_numeric_zero() {
        let candidate = matrix_arm("candidate", 'a');
        let mut run = matrix_run("hit-a", &candidate.arm_ref, ForensicMatrixOutcome::Hit);
        run.total_tokens = None;
        run.token_exactness = ForensicExactness::Unavailable;
        run.cost_micros = None;
        run.cost_exactness = ForensicExactness::Unavailable;
        let matrix = ForensicsMatrixProjection::rebuild(
            "matrix.forensic.unavailable".into(),
            digest('c'),
            digest('d'),
            digest('e'),
            3,
            vec![candidate],
            vec![run],
            matrix_gates(),
            ForensicParetoStatus::NonDominated,
            false,
        )
        .expect("matrix with unavailable usage");
        assert_eq!(matrix.rows[0].total_tokens, None);
        assert_eq!(
            matrix.rows[0].token_exactness,
            ForensicExactness::Unavailable
        );
        assert_eq!(matrix.rows[0].total_cost_micros, None);
    }

    fn coldcard_evidence_workspace() -> ColdcardEvidenceWorkspaceProjection {
        serde_json::from_str(include_str!(
            "../fixtures/coldcard-evidence-workspace.v1.json"
        ))
        .expect("valid Coldcard evidence fixture JSON")
    }

    #[test]
    fn coldcard_workspace_preserves_missing_rungs_and_distinct_evidence_views() {
        let workspace = coldcard_evidence_workspace();
        workspace.validate().expect("valid Coldcard workspace");
        assert_eq!(workspace.ladder.len(), 9);
        assert_eq!(workspace.ladder[6].state, ColdcardRungState::Missing);
        assert_eq!(workspace.ladder[6].time_to_rung.value, None);
        assert_eq!(workspace.assumption_diffs.len(), 5);
        assert_eq!(
            workspace.scan.positive_control,
            ForensicControlState::Passed
        );
        assert_eq!(
            workspace.scan.negative_control,
            ForensicControlState::Passed
        );
        assert!(!workspace.graph_health[1].complete);
        assert_eq!(
            workspace.reconciliation[0].derived_value.as_deref(),
            Some("1000000")
        );
        assert_eq!(
            workspace.reconciliation[0].published_value.as_deref(),
            Some("1.0 million")
        );
    }

    #[test]
    fn coldcard_workspace_rejects_inferred_rungs_and_public_scan_evidence() {
        let mut missing_rung = coldcard_evidence_workspace();
        missing_rung.ladder.remove(6);
        assert!(matches!(
            missing_rung.validate(),
            Err(ForensicsError::InvalidColdcardEvidence(_))
        ));

        let mut reportable = coldcard_evidence_workspace();
        reportable.scan.reportable = true;
        assert!(matches!(
            reportable.validate(),
            Err(ForensicsError::InvalidColdcardEvidence(_))
        ));
    }

    #[test]
    fn coldcard_workspace_fails_closed_on_provenance_gaps_and_secret_material() {
        let mut false_green = coldcard_evidence_workspace();
        false_green.graph_health[1].complete = true;
        assert!(matches!(
            false_green.validate(),
            Err(ForensicsError::InvalidColdcardEvidence(_))
        ));

        let mut secret = coldcard_evidence_workspace();
        secret.corrections[0].corrected_value = "xprv9s21ZrQH143K3fixture-secret-material".into();
        assert!(matches!(
            secret.validate(),
            Err(ForensicsError::InvalidColdcardEvidence(_))
        ));

        let mut mnemonic = coldcard_evidence_workspace();
        mnemonic.corrections[0].corrected_value =
            "abandon ability able about above absent absorb abstract absurd abuse access accident"
                .into();
        assert!(matches!(
            mnemonic.validate(),
            Err(ForensicsError::InvalidColdcardEvidence(_))
        ));
    }

    #[test]
    fn publication_gate_requires_all_authorities_and_fails_closed_for_synthetic_cases() {
        let kinds = [
            ForensicPublicationGateKind::Redaction,
            ForensicPublicationGateKind::IndependentReview,
            ForensicPublicationGateKind::DisclosureScope,
            ForensicPublicationGateKind::MaintainerDecision,
            ForensicPublicationGateKind::PublicationAuthority,
        ];
        let mut projection = ForensicPublicationGateProjection {
            schema: PUBLICATION_GATE_SCHEMA_V1.into(),
            case_ref: "case.coldcard.synthetic".into(),
            private: true,
            synthetic: true,
            operator_ready: false,
            maintainer_approved: false,
            publication_authorized: false,
            gates: kinds
                .into_iter()
                .enumerate()
                .map(|(index, kind)| ForensicPublicationGate {
                    gate_ref: format!("gate.publication.{index}"),
                    kind,
                    state: ForensicPublicationGateState::Blocked,
                    evidence_ref: None,
                    blocker: "Missing authority receipt".into(),
                    next_action: "Obtain the exact external receipt".into(),
                })
                .collect(),
        };
        projection.validate().expect("private blocked projection");
        projection.publication_authorized = true;
        assert!(matches!(
            projection.validate(),
            Err(ForensicsError::InvalidPublicationGate(_))
        ));
    }
}
