use editor::{Editor, EditorEvent};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, Render, SharedString, Subscription,
    TaskExt, Window,
};
use omega_forensics::{
    ColdcardBenchmarkArm, ColdcardClaimRung, ColdcardEvidenceWorkspaceProjection,
    ColdcardRungState, CoverageStatus, DEFAULT_ENTROPY_ANALYSIS_PROMPT, DependencyPolicy,
    EntropyCampaignComparison, EntropyCampaignPhase, EntropyCampaignProjection,
    EntropyDependencyAvailability, EntropyFileAnalysisOutput, EntropyFileState, EntropyFileTask,
    EntropyLimitation, EntropyProjectCatalog, EntropyPromptSnapshot, EntropyRunPhase,
    EntropyRunProjection, EntropySourceInspection, EntropySourceInspectionState,
    ExplicitOperatorAction, FORENSIC_FINDING_SCHEMA_V1, FORENSIC_HYPOTHESIS_SCHEMA_V1,
    ForensicBudgetState, ForensicEvidenceTier, ForensicExactness, ForensicLifecycleState,
    ForensicModelProvenance, ForensicPocIdentity, ForensicPriorWorkQuery,
    ForensicPriorWorkQueryMode, ForensicPriorWorkQueryResult, ForensicPromptIr,
    ForensicPromptWorkspace, ForensicPublicationGate, ForensicPublicationGateKind,
    ForensicPublicationGateProjection, ForensicPublicationGateState, ForensicReviewDecisionKind,
    ForensicReviewOutcome, ForensicSourceCatalog, ForensicSourceCitation, ForensicStatistic,
    ForensicToolEvent, ForensicToolEventStatus, ForensicToolJournal, ForensicWorkDisposition,
    ForensicWorkerObservation, ForensicWorkerPlacement, ForensicsFailureProjection,
    ForensicsLaunchIntent, ForensicsMatrixProjection, ForensicsPreflightProjection,
    ForensicsReviewProjection, ForensicsRunPhase, ForensicsRunProjection,
    IndependentVerifierEnvelope, PUBLICATION_GATE_SCHEMA_V1, PreflightReadiness, PromptChangeKind,
    PromptCompatibilityProfile, RepositoryTargetProjection, SourceState,
};
use omega_workbench_state::RepositoryBinding;
use sha2::{Digest, Sha256};
use ui::{
    Button, ButtonSize, ButtonStyle, Color, Icon, IconName, IconSize, Label, LabelSize, prelude::*,
    v_flex,
};

use crate::forensics_tool_bridge::{
    VisibleForensicToolCallState, ingest_visible_forensic_tool_call,
};
use crate::omega_status_cue::{OmegaStatus, omega_status_cue};
use crate::thread_identity::ThreadIdentityCandidate;

const PREPARE_ACTION_REF: &str = "operator-action-ref://omega/forensics/prepare-run";
const MAX_VISIBLE_ENTROPY_FILES: usize = 500;
const LIVE_FORENSIC_CONTROLS_ACCEPTED: bool = false;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColdcardCaseSelection {
    #[default]
    Overview,
    Rung(ColdcardClaimRung),
}

impl ColdcardCaseSelection {
    fn persisted_rung(self) -> Option<&'static str> {
        match self {
            Self::Overview => None,
            Self::Rung(ColdcardClaimRung::Source) => Some("source"),
            Self::Rung(ColdcardClaimRung::Artifact) => Some("artifact"),
            Self::Rung(ColdcardClaimRung::Generator) => Some("generator"),
            Self::Rung(ColdcardClaimRung::Exploitability) => Some("exploitability"),
            Self::Rung(ColdcardClaimRung::OwnedFixture) => Some("owned_fixture"),
            Self::Rung(ColdcardClaimRung::Fingerprint) => Some("fingerprint"),
            Self::Rung(ColdcardClaimRung::Entity) => Some("entity"),
            Self::Rung(ColdcardClaimRung::UnauthorizedMovement) => Some("unauthorized_movement"),
            Self::Rung(ColdcardClaimRung::Identity) => Some("identity"),
        }
    }

    fn from_persisted_rung(rung: Option<&str>) -> Self {
        match rung {
            Some("source") => Self::Rung(ColdcardClaimRung::Source),
            Some("artifact") => Self::Rung(ColdcardClaimRung::Artifact),
            Some("generator") => Self::Rung(ColdcardClaimRung::Generator),
            Some("exploitability") => Self::Rung(ColdcardClaimRung::Exploitability),
            Some("owned_fixture") => Self::Rung(ColdcardClaimRung::OwnedFixture),
            Some("fingerprint") => Self::Rung(ColdcardClaimRung::Fingerprint),
            Some("entity") => Self::Rung(ColdcardClaimRung::Entity),
            Some("unauthorized_movement") => Self::Rung(ColdcardClaimRung::UnauthorizedMovement),
            Some("identity") => Self::Rung(ColdcardClaimRung::Identity),
            _ => Self::Overview,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColdcardCaseReaderState {
    Loading,
    Empty,
    Invalid(SharedString),
    Stale(SharedString),
    Complete,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForensicsBenchView {
    #[default]
    Entropy,
    Case,
    Lifecycle,
    Evidence,
    Models,
    Publication,
}

impl ForensicsBenchView {
    const AVAILABLE: [Self; 6] = [
        Self::Entropy,
        Self::Case,
        Self::Lifecycle,
        Self::Evidence,
        Self::Models,
        Self::Publication,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Entropy => "Entropy",
            Self::Case => "Case",
            Self::Lifecycle => "Lifecycle",
            Self::Evidence => "Evidence",
            Self::Models => "Models",
            Self::Publication => "Publication",
        }
    }

    fn persisted(self) -> &'static str {
        match self {
            Self::Entropy => "entropy",
            Self::Case => "case",
            Self::Lifecycle => "lifecycle",
            Self::Evidence => "evidence",
            Self::Models => "models",
            Self::Publication => "publication",
        }
    }

    fn from_persisted(value: Option<&str>) -> Self {
        match value {
            Some("case") => Self::Case,
            Some("lifecycle") => Self::Lifecycle,
            Some("evidence") => Self::Evidence,
            Some("models") => Self::Models,
            Some("publication") => Self::Publication,
            _ => Self::Entropy,
        }
    }
}

const OMEGA_UI_MOCKS_ENV: &str = "OMEGA_UI_MOCKS";

fn forensics_fixture_views_enabled_for(
    test_support: bool,
    debug_assertions: bool,
    mock_value: Option<&str>,
) -> bool {
    test_support || (debug_assertions && mock_value == Some("1"))
}

fn forensics_fixture_views_enabled() -> bool {
    forensics_fixture_views_enabled_for(
        cfg!(any(test, feature = "test-support")),
        cfg!(debug_assertions),
        std::env::var(OMEGA_UI_MOCKS_ENV).ok().as_deref(),
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PublicationScene {
    #[default]
    PrivateBlocked,
    Denied,
    AwaitingReview,
    Rejected,
    Stale,
    EligibleNotAuthorized,
}

impl PublicationScene {
    const ALL: [Self; 6] = [
        Self::PrivateBlocked,
        Self::Denied,
        Self::AwaitingReview,
        Self::Rejected,
        Self::Stale,
        Self::EligibleNotAuthorized,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::PrivateBlocked => "Private · blocked",
            Self::Denied => "Denied",
            Self::AwaitingReview => "Awaiting review",
            Self::Rejected => "Rejected",
            Self::Stale => "Stale",
            Self::EligibleNotAuthorized => "Eligible · not authorized",
        }
    }

    fn persisted(self) -> &'static str {
        match self {
            Self::PrivateBlocked => "private_blocked",
            Self::Denied => "denied",
            Self::AwaitingReview => "awaiting_review",
            Self::Rejected => "rejected",
            Self::Stale => "stale",
            Self::EligibleNotAuthorized => "eligible_not_authorized",
        }
    }

    fn from_persisted(value: Option<&str>) -> Self {
        match value {
            Some("denied") => Self::Denied,
            Some("awaiting_review") => Self::AwaitingReview,
            Some("rejected") => Self::Rejected,
            Some("stale") => Self::Stale,
            Some("eligible_not_authorized") => Self::EligibleNotAuthorized,
            _ => Self::PrivateBlocked,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EvidenceSelection {
    #[default]
    Findings,
    Hypotheses,
    Limitations,
    Disputes,
    Reconciliation,
}

impl EvidenceSelection {
    const ALL: [Self; 5] = [
        Self::Findings,
        Self::Hypotheses,
        Self::Limitations,
        Self::Disputes,
        Self::Reconciliation,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Findings => "Findings",
            Self::Hypotheses => "Hypotheses",
            Self::Limitations => "Limitations",
            Self::Disputes => "Disputes",
            Self::Reconciliation => "Reconciliation",
        }
    }

    fn persisted(self) -> &'static str {
        match self {
            Self::Findings => "findings",
            Self::Hypotheses => "hypotheses",
            Self::Limitations => "limitations",
            Self::Disputes => "disputes",
            Self::Reconciliation => "reconciliation",
        }
    }

    fn from_persisted(value: Option<&str>) -> Self {
        match value {
            Some("hypotheses") => Self::Hypotheses,
            Some("limitations") => Self::Limitations,
            Some("disputes") => Self::Disputes,
            Some("reconciliation") => Self::Reconciliation,
            _ => Self::Findings,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LifecycleSelection {
    #[default]
    Summary,
    Target,
    Coverage,
    Profile,
    Runtime,
    Cleanup,
}

impl LifecycleSelection {
    const ALL: [Self; 6] = [
        Self::Summary,
        Self::Target,
        Self::Coverage,
        Self::Profile,
        Self::Runtime,
        Self::Cleanup,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Summary => "Lifecycle summary",
            Self::Target => "Target",
            Self::Coverage => "Coverage",
            Self::Profile => "Tool profile",
            Self::Runtime => "Runtime",
            Self::Cleanup => "Cleanup",
        }
    }

    fn persisted(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Target => "target",
            Self::Coverage => "coverage",
            Self::Profile => "profile",
            Self::Runtime => "runtime",
            Self::Cleanup => "cleanup",
        }
    }

    fn from_persisted(value: Option<&str>) -> Self {
        match value {
            Some("target") => Self::Target,
            Some("coverage") => Self::Coverage,
            Some("profile") => Self::Profile,
            Some("runtime") => Self::Runtime,
            Some("cleanup") => Self::Cleanup,
            _ => Self::Summary,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForensicsLifecycleScene {
    AwaitingProfile,
    AwaitingCoverage,
    Complete,
    Incomplete,
    Denied,
    IncompatibleTool,
    Running,
    Cancelled,
    RecoveryRequired,
    Cleaned,
    Stale,
}

impl ForensicsLifecycleScene {
    fn label(self) -> &'static str {
        match self {
            Self::AwaitingProfile => "Awaiting profile",
            Self::AwaitingCoverage => "Coverage pending",
            Self::Complete => "Preflight complete",
            Self::Incomplete => "Incomplete",
            Self::Denied => "Denied",
            Self::IncompatibleTool => "Tool incompatible",
            Self::Running => "Running",
            Self::Cancelled => "Cancelled",
            Self::RecoveryRequired => "Recovery required",
            Self::Cleaned => "Cleaned",
            Self::Stale => "Stale",
        }
    }

    /// The shared status channel for this lifecycle scene. The exact scene name
    /// stays the accessible context, so the cue reads as `Awaiting profile:
    /// Blocked` rather than losing which lifecycle state is meant.
    fn status(self) -> OmegaStatus {
        match self {
            Self::Complete | Self::Cleaned => OmegaStatus::Complete,
            Self::Running => OmegaStatus::Running,
            Self::AwaitingProfile | Self::AwaitingCoverage => OmegaStatus::Blocked,
            Self::Incomplete | Self::Cancelled | Self::Stale => OmegaStatus::Warning,
            Self::Denied | Self::IncompatibleTool | Self::RecoveryRequired => OmegaStatus::Failed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ForensicsLifecyclePresentation {
    scene: ForensicsLifecycleScene,
    blocker: &'static str,
    next_action: &'static str,
}

fn bundled_coldcard_evidence_workspace() -> anyhow::Result<ColdcardEvidenceWorkspaceProjection> {
    let workspace: ColdcardEvidenceWorkspaceProjection = serde_json::from_str(include_str!(
        "../../omega_forensics/fixtures/coldcard-evidence-workspace.v1.json"
    ))?;
    workspace.validate()?;
    Ok(workspace)
}

fn fixture_digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

fn bundled_coldcard_model_matrix() -> anyhow::Result<ForensicsMatrixProjection> {
    use omega_forensics::{
        ForensicDatasetSplit, ForensicMatrixArm, ForensicMatrixHardGates, ForensicMatrixOutcome,
        ForensicMatrixPopulation, ForensicMatrixRun, ForensicParetoStatus,
    };

    let prompt_digest = fixture_digest('1');
    let arm =
        |suffix: &str, model_family_ref: &str, role_ref: &str, model: char| ForensicMatrixArm {
            arm_ref: format!("arm.forensic.{suffix}"),
            model_family_ref: model_family_ref.into(),
            role_ref: role_ref.into(),
            prompt_digest: prompt_digest.clone(),
            model_digest: fixture_digest(model),
            effort_ref: "effort.high".into(),
            scope_ref: "scope.coldcard.entropy".into(),
            dependency_policy_ref: "dependency.pinned-recursive".into(),
            random_seed: 7,
            tool_surface_digest: fixture_digest('2'),
            analysis_mode_ref: "analysis.static-build-and-fixture".into(),
            worker_image_digest: fixture_digest('3'),
            worker_profile_digest: fixture_digest('4'),
            source_bundle_digest: fixture_digest(model),
            writable_disk_ref: format!("disk.forensic.{suffix}"),
            provider_session_ref: format!("provider-session.forensic.{suffix}"),
            auth_home_ref: format!("auth-home.forensic.{suffix}"),
            environment_ref: format!("environment.forensic.{suffix}"),
            worker_state_ref: format!("worker-state.forensic.{suffix}"),
        };
    let specialist = arm(
        "entropy-specialist",
        "model-family.openai.gpt-5",
        "role.forensic.entropy-specialist",
        '6',
    );
    let generalist = arm(
        "general-reviewer",
        "model-family.anthropic.claude",
        "role.forensic.general-reviewer",
        '7',
    );
    let clean_control = arm(
        "clean-control",
        "model-family.openai.gpt-5",
        "role.forensic.clean-control",
        '8',
    );
    let run = |run_ref: &str,
               run_digest_character: char,
               arm_ref: String,
               split,
               population,
               outcome,
               censored,
               censor_at_milliseconds,
               identification_milliseconds,
               identification_tokens,
               total_tokens,
               token_exactness,
               cost_micros,
               cost_exactness,
               findings: Vec<String>| ForensicMatrixRun {
        run_ref: run_ref.into(),
        run_digest: fixture_digest(run_digest_character),
        arm_ref,
        dataset_split: split,
        population,
        coverage_status: CoverageStatus::Complete,
        outcome,
        censored,
        censor_at_milliseconds,
        identification_milliseconds,
        identification_tokens,
        total_tokens,
        token_exactness,
        cost_micros,
        cost_exactness,
        causal_links_supported: if findings.is_empty() { 0 } else { 4 },
        causal_links_required: if findings.is_empty() { 0 } else { 4 },
        false_positive_count: 0,
        reviewer_active_seconds: Some(90),
        budget_compliant: true,
        cleanup_observed: true,
        qualified_finding_refs: findings,
        failure_refs: Vec::new(),
        event_refs: vec![format!("event.{run_ref}")],
        receipt_refs: vec![format!("receipt.{run_ref}")],
    };
    let specialist_run = run(
        "run.matrix.entropy-specialist",
        'c',
        specialist.arm_ref.clone(),
        ForensicDatasetSplit::Holdout,
        ForensicMatrixPopulation::Vulnerable,
        ForensicMatrixOutcome::Hit,
        false,
        None,
        Some(82_000),
        Some(18_400),
        Some(31_200),
        ForensicExactness::Exact,
        Some(740_000),
        ForensicExactness::Exact,
        vec![
            "finding.coldcard.entropy-fallback".into(),
            "finding.coldcard.provider-guard".into(),
        ],
    );
    let generalist_run = run(
        "run.matrix.general-reviewer",
        'd',
        generalist.arm_ref.clone(),
        ForensicDatasetSplit::Holdout,
        ForensicMatrixPopulation::Vulnerable,
        ForensicMatrixOutcome::Miss,
        true,
        Some(120_000),
        None,
        None,
        Some(28_000),
        ForensicExactness::Exact,
        Some(620_000),
        ForensicExactness::Estimated,
        Vec::new(),
    );
    let control_run = run(
        "run.matrix.clean-control",
        'e',
        clean_control.arm_ref.clone(),
        ForensicDatasetSplit::CleanHoldout,
        ForensicMatrixPopulation::CleanControl,
        ForensicMatrixOutcome::NotEligible,
        false,
        None,
        None,
        None,
        None,
        ForensicExactness::Unavailable,
        None,
        ForensicExactness::Unavailable,
        Vec::new(),
    );

    ForensicsMatrixProjection::rebuild(
        "matrix.forensic.coldcard.synthetic".into(),
        fixture_digest('9'),
        fixture_digest('a'),
        fixture_digest('b'),
        3,
        vec![specialist, generalist, clean_control],
        vec![specialist_run, generalist_run, control_run],
        ForensicMatrixHardGates {
            input_complete: true,
            isolation_complete: true,
            clean_control: true,
            evidence_quality: true,
            budget_compliant: true,
            cleanup_complete: true,
            hit_rate_not_regressed: true,
        },
        ForensicParetoStatus::Incomparable,
        false,
    )
    .map_err(Into::into)
}

fn bundled_publication_gate(
    scene: PublicationScene,
) -> anyhow::Result<ForensicPublicationGateProjection> {
    let state_for = |kind| match (scene, kind) {
        (PublicationScene::Denied, ForensicPublicationGateKind::Redaction) => {
            ForensicPublicationGateState::Denied
        }
        (PublicationScene::AwaitingReview, ForensicPublicationGateKind::IndependentReview) => {
            ForensicPublicationGateState::AwaitingReview
        }
        (PublicationScene::Rejected, ForensicPublicationGateKind::MaintainerDecision) => {
            ForensicPublicationGateState::Rejected
        }
        (PublicationScene::Stale, ForensicPublicationGateKind::DisclosureScope) => {
            ForensicPublicationGateState::Stale
        }
        (
            PublicationScene::EligibleNotAuthorized,
            ForensicPublicationGateKind::PublicationAuthority,
        ) => ForensicPublicationGateState::EligibleNotAuthorized,
        (PublicationScene::EligibleNotAuthorized, _) => ForensicPublicationGateState::Satisfied,
        _ => ForensicPublicationGateState::Blocked,
    };
    let gate = |kind, suffix: &str, blocker: &str, next_action: &str, evidence: Option<&str>| {
        ForensicPublicationGate {
            gate_ref: format!("gate.publication.{suffix}"),
            kind,
            state: state_for(kind),
            evidence_ref: evidence.map(str::to_string),
            blocker: blocker.into(),
            next_action: next_action.into(),
        }
    };
    let projection = ForensicPublicationGateProjection {
        schema: PUBLICATION_GATE_SCHEMA_V1.into(),
        case_ref: "case.coldcard.synthetic".into(),
        private: true,
        synthetic: true,
        operator_ready: scene == PublicationScene::EligibleNotAuthorized,
        maintainer_approved: scene == PublicationScene::EligibleNotAuthorized,
        publication_authorized: false,
        gates: vec![
            gate(
                ForensicPublicationGateKind::Redaction,
                "redaction",
                "Private identifiers and source excerpts require a redaction receipt",
                "Produce a bounded redaction receipt without changing the evidence record",
                None,
            ),
            gate(
                ForensicPublicationGateKind::IndependentReview,
                "independent-review",
                "No accepted independent-review decision is attached",
                "Request review from an admitted independent reviewer",
                Some("evidence.review.synthetic"),
            ),
            gate(
                ForensicPublicationGateKind::DisclosureScope,
                "disclosure-scope",
                "The public disclosure scope is absent or stale",
                "Record the exact claims, limitations, and evidence refs proposed for disclosure",
                Some("evidence.scope.synthetic"),
            ),
            gate(
                ForensicPublicationGateKind::MaintainerDecision,
                "maintainer-decision",
                "A maintainer has not admitted this disclosure",
                "Request a maintainer decision after evidence and review gates are satisfied",
                None,
            ),
            gate(
                ForensicPublicationGateKind::PublicationAuthority,
                "publication-authority",
                "No publication authority receipt exists",
                "Obtain publication authority outside this read-only UI",
                None,
            ),
        ],
    };
    projection.validate()?;
    Ok(projection)
}

pub(crate) fn entropy_campaign_checkout_root(
    campaign_ref: &str,
    product_ref: &str,
) -> std::path::PathBuf {
    let mut digest = Sha256::new();
    digest.update(campaign_ref.as_bytes());
    digest.update([0]);
    digest.update(product_ref.as_bytes());
    std::env::temp_dir()
        .join("omega-entropy-campaigns")
        .join(format!("{:x}", digest.finalize()))
}

pub(crate) struct EntropyRepositorySource {
    root: std::path::PathBuf,
    _guard: Option<std::sync::Arc<tempfile::TempDir>>,
}

impl EntropyRepositorySource {
    pub(crate) fn snapshot(root: std::path::PathBuf, guard: tempfile::TempDir) -> Self {
        Self {
            root,
            _guard: Some(std::sync::Arc::new(guard)),
        }
    }

    pub(crate) fn root(&self) -> &std::path::Path {
        &self.root
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EntropyFileFilter {
    #[default]
    All,
    Candidates,
    Failures,
    Incomplete,
}

impl EntropyFileFilter {
    const ALL: [Self; 4] = [
        Self::All,
        Self::Candidates,
        Self::Failures,
        Self::Incomplete,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All files",
            Self::Candidates => "Candidates",
            Self::Failures => "Failures",
            Self::Incomplete => "Incomplete",
        }
    }

    fn includes(self, state: EntropyFileState) -> bool {
        match self {
            Self::All => true,
            Self::Candidates => state == EntropyFileState::Candidate,
            Self::Failures => matches!(state, EntropyFileState::Failed | EntropyFileState::Skipped),
            Self::Incomplete => matches!(
                state,
                EntropyFileState::Queued | EntropyFileState::Reading | EntropyFileState::Cancelled
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForensicsRepositoryContext {
    pub display_name: SharedString,
    pub clone_url: Option<SharedString>,
    pub commit: Option<SharedString>,
    pub dirty_files: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForensicsWorkbenchSnapshot {
    pub binding: RepositoryBinding,
    pub bench_view: ForensicsBenchView,
    pub lifecycle_selection: LifecycleSelection,
    pub evidence_selection: EvidenceSelection,
    pub selected_model_run_ref: Option<String>,
    pub publication_scene: PublicationScene,
    pub selected_arm: ColdcardBenchmarkArm,
    pub readiness: Option<PreflightReadiness>,
    pub prepared_intent: Option<ForensicsLaunchIntent>,
    pub run: Option<ForensicsRunProjection>,
    pub review: Option<ForensicsReviewProjection>,
    pub prompt_workspace: ForensicPromptWorkspace,
    pub matrix: Option<ForensicsMatrixProjection>,
    pub coldcard_evidence: Option<ColdcardEvidenceWorkspaceProjection>,
    pub coldcard_case_selection: ColdcardCaseSelection,
    pub coldcard_case_reader_state: ColdcardCaseReaderState,
    pub prior_work: Option<ForensicPriorWorkQueryResult>,
    pub tool_journal: Option<ForensicToolJournal>,
    pub entropy_run: Option<EntropyRunProjection>,
    pub entropy_run_history: Vec<EntropyRunProjection>,
    pub entropy_source_inspection: Option<EntropySourceInspection>,
    pub entropy_prompt_draft: String,
    pub entropy_parent_prompt_ref: Option<String>,
    pub entropy_source_run_ref: Option<String>,
    pub entropy_prompt_snapshots: Vec<EntropyPromptSnapshot>,
    pub entropy_file_filter: EntropyFileFilter,
    pub selected_entropy_file: Option<String>,
    pub entropy_campaign: Option<EntropyCampaignProjection>,
    pub entropy_campaign_history: Vec<EntropyCampaignProjection>,
    pub selected_entropy_project: Option<String>,
    pub source_resolutions: std::collections::BTreeMap<String, ForensicSourceResolution>,
    pub status: SharedString,
    pub fixture_views_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForensicSourceResolution {
    Opening,
    Opened,
    Failed(String),
}

#[derive(Clone, Debug)]
pub enum ForensicsWorkbenchCommand {
    StartEntropy {
        prompt_snapshot: EntropyPromptSnapshot,
    },
    StartCatalogEntropy {
        prompt_snapshot: EntropyPromptSnapshot,
        project: omega_forensics::EntropyProjectRecord,
    },
    CancelEntropy,
    StartEntropyCampaign {
        prompt_snapshot: EntropyPromptSnapshot,
        catalog: EntropyProjectCatalog,
    },
    ContinueEntropyCampaign,
    Launch {
        run_ref: String,
        intent: ForensicsLaunchIntent,
        prompt_digest: String,
    },
    Refresh,
    RefreshPriorWork {
        query: ForensicPriorWorkQuery,
    },
    Cancel,
    Cleanup,
    OpenSource {
        citation: ForensicSourceCitation,
        repository_root: Option<std::path::PathBuf>,
    },
}

pub struct ForensicsWorkbenchSurface {
    focus_handle: FocusHandle,
    binding: RepositoryBinding,
    repository: ForensicsRepositoryContext,
    bench_view: ForensicsBenchView,
    lifecycle_selection: LifecycleSelection,
    evidence_selection: EvidenceSelection,
    selected_model_run_ref: Option<String>,
    publication_scene: PublicationScene,
    selected_arm: ColdcardBenchmarkArm,
    preflight: Option<ForensicsPreflightProjection>,
    prepared_intent: Option<ForensicsLaunchIntent>,
    run: Option<ForensicsRunProjection>,
    review: Option<ForensicsReviewProjection>,
    prompt_workspace: ForensicPromptWorkspace,
    matrix: Option<ForensicsMatrixProjection>,
    coldcard_evidence: Option<ColdcardEvidenceWorkspaceProjection>,
    coldcard_case_selection: ColdcardCaseSelection,
    coldcard_case_reader_state: ColdcardCaseReaderState,
    prior_work: Option<ForensicPriorWorkQueryResult>,
    tool_journal: Option<ForensicToolJournal>,
    entropy_run: Option<EntropyRunProjection>,
    entropy_run_history: Vec<EntropyRunProjection>,
    entropy_source_inspection: Option<EntropySourceInspection>,
    entropy_prompt_editor: Option<Entity<Editor>>,
    entropy_prompt_draft: String,
    entropy_parent_prompt_ref: Option<String>,
    entropy_source_run_ref: Option<String>,
    entropy_prompt_snapshots: Vec<EntropyPromptSnapshot>,
    entropy_file_filter: EntropyFileFilter,
    selected_entropy_file: Option<String>,
    entropy_catalog: EntropyProjectCatalog,
    entropy_campaign: Option<EntropyCampaignProjection>,
    entropy_campaign_history: Vec<EntropyCampaignProjection>,
    selected_entropy_project: Option<String>,
    entropy_campaign_roots: std::collections::BTreeMap<String, std::path::PathBuf>,
    entropy_repository_source: Option<EntropyRepositorySource>,
    _entropy_prompt_subscription: Option<Subscription>,
    source_resolutions: std::collections::BTreeMap<String, ForensicSourceResolution>,
    status: SharedString,
    fixture_views_enabled: bool,
}

impl ForensicsWorkbenchSurface {
    pub fn new(candidate: &ThreadIdentityCandidate, cx: &mut Context<Self>) -> Self {
        let fixture_views_enabled = forensics_fixture_views_enabled();
        let entropy_catalog = EntropyProjectCatalog::wallet_entropy_v2()
            .expect("the built-in entropy catalog must remain valid");
        let selected_entropy_project = entropy_catalog
            .projects
            .first()
            .map(|project| project.product_ref.clone());
        let (coldcard_evidence, coldcard_case_reader_state) = if fixture_views_enabled {
            match bundled_coldcard_evidence_workspace() {
                Ok(workspace) => (Some(workspace), ColdcardCaseReaderState::Complete),
                Err(error) => (
                    None,
                    ColdcardCaseReaderState::Invalid(
                        format!("Bundled Coldcard case is invalid: {error}").into(),
                    ),
                ),
            }
        } else {
            (None, ColdcardCaseReaderState::Empty)
        };
        Self {
            focus_handle: cx.focus_handle(),
            binding: candidate.binding.clone(),
            repository: ForensicsRepositoryContext {
                display_name: candidate.repository_name.clone(),
                clone_url: candidate.remote_url.clone(),
                commit: candidate.head_commit.clone(),
                dirty_files: candidate.git.dirty_files,
            },
            bench_view: ForensicsBenchView::Entropy,
            lifecycle_selection: LifecycleSelection::Summary,
            evidence_selection: EvidenceSelection::Findings,
            selected_model_run_ref: None,
            publication_scene: PublicationScene::PrivateBlocked,
            selected_arm: ColdcardBenchmarkArm::Vulnerable,
            preflight: None,
            prepared_intent: None,
            run: None,
            review: None,
            prompt_workspace: ForensicPromptWorkspace::new(
                omega_forensics::baseline_forensic_prompt(
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                )
                .expect("the built-in forensic prompt must remain valid"),
            )
            .expect("the built-in forensic prompt workspace must remain valid"),
            matrix: None,
            coldcard_evidence,
            coldcard_case_selection: ColdcardCaseSelection::Overview,
            coldcard_case_reader_state,
            prior_work: None,
            tool_journal: None,
            entropy_run: None,
            entropy_run_history: Vec::new(),
            entropy_source_inspection: None,
            entropy_prompt_editor: None,
            entropy_prompt_draft: DEFAULT_ENTROPY_ANALYSIS_PROMPT.into(),
            entropy_parent_prompt_ref: None,
            entropy_source_run_ref: None,
            entropy_prompt_snapshots: Vec::new(),
            entropy_file_filter: EntropyFileFilter::All,
            selected_entropy_file: None,
            entropy_catalog,
            entropy_campaign: None,
            entropy_campaign_history: Vec::new(),
            selected_entropy_project,
            entropy_campaign_roots: std::collections::BTreeMap::new(),
            entropy_repository_source: None,
            _entropy_prompt_subscription: None,
            source_resolutions: std::collections::BTreeMap::new(),
            status: "Awaiting OpenAgents managed profile".into(),
            fixture_views_enabled,
        }
    }

    pub fn new_with_window(
        candidate: &ThreadIdentityCandidate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let restored =
            crate::entropy_prompt_store::read(&candidate.binding, cx).unwrap_or_default();
        let mut this = Self::new(candidate, cx);
        this.entropy_prompt_draft = restored.draft_prompt;
        this.entropy_parent_prompt_ref = restored.parent_prompt_ref;
        this.entropy_source_run_ref = restored.source_run_ref;
        this.entropy_prompt_snapshots = restored.prompt_snapshots;
        this.coldcard_case_selection =
            ColdcardCaseSelection::from_persisted_rung(restored.coldcard_case_rung.as_deref());
        this.restore_bench_view(restored.bench_view.as_deref());
        this.lifecycle_selection =
            LifecycleSelection::from_persisted(restored.lifecycle_selection.as_deref());
        this.evidence_selection =
            EvidenceSelection::from_persisted(restored.evidence_selection.as_deref());
        this.selected_model_run_ref = restored.model_run_ref;
        this.publication_scene =
            PublicationScene::from_persisted(restored.publication_scene.as_deref());
        this.tool_journal = restored.tool_journal;
        let mut campaigns = restored.campaigns;
        if let Some(mut active_campaign) = campaigns.pop() {
            if matches!(
                active_campaign.phase,
                EntropyCampaignPhase::Ready
                    | EntropyCampaignPhase::Running
                    | EntropyCampaignPhase::Paused
            ) {
                let _ = active_campaign.cancel(
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                );
            }
            for project in &active_campaign.projects {
                if project.run.is_some() {
                    this.entropy_campaign_roots.insert(
                        project.product.product_ref.clone(),
                        entropy_campaign_checkout_root(
                            &active_campaign.binding.campaign_ref,
                            &project.product.product_ref,
                        ),
                    );
                }
            }
            this.entropy_campaign = Some(active_campaign);
        }
        this.entropy_campaign_history = campaigns;
        this.selected_entropy_file = restored
            .runs
            .last()
            .and_then(|run| {
                run.files
                    .iter()
                    .find(|file| file.state == EntropyFileState::Candidate)
            })
            .map(|file| file.path.clone());
        let mut runs = restored.runs;
        if let Some(mut active) = runs.pop() {
            if matches!(
                active.phase,
                EntropyRunPhase::Ready
                    | EntropyRunPhase::Running
                    | EntropyRunPhase::CancelRequested
            ) {
                let _ = active.cancel(
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                );
            }
            this.entropy_run = Some(active);
        }
        this.entropy_run_history = runs;
        this.entropy_source_inspection = restored.source_inspection.and_then(|inspection| {
            inspection
                .mark_stale(
                    inspection.generation.saturating_add(1),
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                )
                .ok()
        });
        let editor = cx.new(|cx| {
            let mut editor = Editor::auto_height(5, 12, window, cx);
            editor.set_text(this.entropy_prompt_draft.clone(), window, cx);
            editor.set_placeholder_text("Describe the entropy vulnerability analysis…", window, cx);
            editor.set_soft_wrap();
            editor
        });
        this._entropy_prompt_subscription =
            Some(cx.subscribe(&editor, |this, editor, event, cx| {
                if matches!(event, EditorEvent::Edited { .. }) {
                    this.entropy_prompt_draft = editor.read(cx).text(cx);
                    this.persist_entropy_state(cx);
                    cx.notify();
                }
            }));
        this.entropy_prompt_editor = Some(editor);
        this
    }

    pub fn binding(&self) -> &RepositoryBinding {
        &self.binding
    }

    pub fn request_entropy_run(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.entropy_campaign.as_ref().is_some_and(|campaign| {
                matches!(
                    campaign.phase,
                    EntropyCampaignPhase::Ready
                        | EntropyCampaignPhase::Running
                        | EntropyCampaignPhase::Paused
                )
            }),
            "finish or cancel the active entropy campaign before starting a repository run"
        );
        let selected_project_ref = self
            .selected_entropy_project
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("select a project before starting an entropy scan"))?;
        let project = self
            .entropy_catalog
            .projects
            .iter()
            .find(|project| project.product_ref == selected_project_ref)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("the selected project is not in the catalog"))?;
        anyhow::ensure!(
            project.repository_url.is_some() && project.pinned_revision.is_some(),
            "the selected project has no complete source pin"
        );
        let snapshot = EntropyPromptSnapshot::new(
            format!("prompt.omega.entropy.{}", uuid::Uuid::new_v4().simple()),
            self.entropy_parent_prompt_ref.clone(),
            self.entropy_source_run_ref.clone(),
            self.entropy_prompt_draft.clone(),
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        )?;
        if let Some(campaign) = self.entropy_campaign.take() {
            self.entropy_campaign_history.push(campaign);
        }
        self.entropy_campaign_roots.clear();
        self.entropy_prompt_snapshots.push(snapshot.clone());
        if let Some(previous) = self.entropy_run.take() {
            self.entropy_run_history.push(previous);
        }
        self.entropy_source_inspection = None;
        self.status = "Preparing an entropy file manifest…".into();
        cx.emit(ForensicsWorkbenchCommand::StartCatalogEntropy {
            prompt_snapshot: snapshot,
            project,
        });
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn request_entropy_campaign(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.entropy_run.as_ref().is_some_and(|run| {
                matches!(
                    run.phase,
                    EntropyRunPhase::Ready
                        | EntropyRunPhase::Running
                        | EntropyRunPhase::CancelRequested
                )
            }),
            "finish or cancel the active repository run before starting a campaign"
        );
        let snapshot = EntropyPromptSnapshot::new(
            format!("prompt.omega.entropy.{}", uuid::Uuid::new_v4().simple()),
            self.entropy_parent_prompt_ref.clone(),
            self.entropy_source_run_ref.clone(),
            self.entropy_prompt_draft.clone(),
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        )?;
        self.entropy_prompt_snapshots.push(snapshot.clone());
        self.entropy_repository_source = None;
        self.entropy_source_inspection = None;
        self.status = format!(
            "Preparing the {}-target entropy campaign…",
            self.entropy_catalog.projects.len()
        )
        .into();
        cx.emit(ForensicsWorkbenchCommand::StartEntropyCampaign {
            prompt_snapshot: snapshot,
            catalog: self.entropy_catalog.clone(),
        });
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn install_entropy_campaign(
        &mut self,
        campaign: EntropyCampaignProjection,
        cx: &mut Context<Self>,
    ) {
        if let Some(previous) = self.entropy_campaign.replace(campaign) {
            self.entropy_campaign_history.push(previous);
        }
        self.status = format!(
            "{}-target entropy campaign started",
            self.entropy_catalog.projects.len()
        )
        .into();
        self.persist_entropy_state(cx);
        cx.notify();
    }

    pub fn start_next_entropy_campaign_project(
        &mut self,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<Option<omega_forensics::EntropyProjectRecord>> {
        let campaign = self
            .entropy_campaign
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the entropy campaign is unavailable"))?;
        let next = campaign.start_next_project()?;
        if let Some(project) = &next {
            self.selected_entropy_project = Some(project.product_ref.clone());
            self.selected_entropy_file = None;
            self.status = format!(
                "Materializing {} at its pinned revision…",
                project.product_name
            )
            .into();
        }
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(next)
    }

    pub fn install_entropy_campaign_root(
        &mut self,
        product_ref: String,
        root: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.entropy_campaign_roots.insert(product_ref, root);
        cx.notify();
    }

    pub fn sync_entropy_campaign_project(
        &mut self,
        product_ref: &str,
        run: EntropyRunProjection,
        elapsed_milliseconds: Option<u64>,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let campaign = self
            .entropy_campaign
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the entropy campaign is unavailable"))?;
        campaign.update_project_run(
            product_ref,
            run,
            elapsed_milliseconds,
            omega_forensics::EntropyCampaignUsage::unavailable(),
        )?;
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn fail_entropy_campaign_project(
        &mut self,
        product_ref: &str,
        message: String,
        elapsed_milliseconds: Option<u64>,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let campaign = self
            .entropy_campaign
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the entropy campaign is unavailable"))?;
        campaign.record_provider_failure(product_ref, message, elapsed_milliseconds)?;
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn fail_entropy_campaign_source(
        &mut self,
        product_ref: &str,
        message: String,
        elapsed_milliseconds: Option<u64>,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let campaign = self
            .entropy_campaign
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the entropy campaign is unavailable"))?;
        campaign.record_source_failure(product_ref, message, elapsed_milliseconds)?;
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn pause_entropy_campaign(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        self.entropy_campaign
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the entropy campaign is unavailable"))?
            .pause()?;
        self.status = "Entropy campaign paused after the active repository".into();
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn resume_entropy_campaign(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        self.entropy_campaign
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the entropy campaign is unavailable"))?
            .resume()?;
        self.status = "Entropy campaign resumed".into();
        cx.emit(ForensicsWorkbenchCommand::ContinueEntropyCampaign);
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn cancel_entropy_campaign(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        if self.entropy_run.as_ref().is_some_and(|run| {
            matches!(
                run.phase,
                EntropyRunPhase::Ready
                    | EntropyRunPhase::Running
                    | EntropyRunPhase::CancelRequested
            )
        }) {
            self.cancel_entropy_run(cx)?;
        }
        self.entropy_campaign
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the entropy campaign is unavailable"))?
            .cancel(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true))?;
        self.status = "Entropy campaign cancelled; partial results retained".into();
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn select_entropy_project(&mut self, product_ref: String, cx: &mut Context<Self>) {
        self.selected_entropy_project = Some(product_ref);
        self.selected_entropy_file = None;
        cx.notify();
    }

    pub fn install_entropy_run(&mut self, run: EntropyRunProjection, cx: &mut Context<Self>) {
        let counts = run.counts();
        self.status = format!(
            "Entropy run ready · {} queued · {} skipped",
            counts.queued, counts.skipped
        )
        .into();
        if let Some(previous) = self.entropy_run.replace(run) {
            self.entropy_run_history.push(previous);
        }
        self.selected_entropy_file = None;
        self.persist_entropy_state(cx);
        cx.notify();
    }

    fn entropy_restore_state(&self) -> crate::entropy_prompt_store::EntropyForensicsRestoreState {
        let mut runs = self.entropy_run_history.clone();
        if let Some(run) = self.entropy_run.clone() {
            runs.push(run);
        }
        crate::entropy_prompt_store::EntropyForensicsRestoreState {
            draft_prompt: self.entropy_prompt_draft.clone(),
            parent_prompt_ref: self.entropy_parent_prompt_ref.clone(),
            source_run_ref: self.entropy_source_run_ref.clone(),
            prompt_snapshots: self.entropy_prompt_snapshots.clone(),
            runs,
            campaigns: {
                let mut campaigns = self.entropy_campaign_history.clone();
                if let Some(campaign) = self.entropy_campaign.clone() {
                    campaigns.push(campaign);
                }
                campaigns
            },
            source_inspection: self.entropy_source_inspection.clone(),
            coldcard_case_rung: self
                .coldcard_case_selection
                .persisted_rung()
                .map(str::to_string),
            bench_view: Some(self.bench_view.persisted().into()),
            lifecycle_selection: Some(self.lifecycle_selection.persisted().into()),
            evidence_selection: Some(self.evidence_selection.persisted().into()),
            model_run_ref: self.selected_model_run_ref.clone(),
            publication_scene: Some(self.publication_scene.persisted().into()),
            tool_journal: self.tool_journal.clone(),
        }
    }

    fn persist_entropy_state(&self, cx: &mut Context<Self>) {
        crate::entropy_prompt_store::write(self.binding.clone(), self.entropy_restore_state(), cx)
            .detach_and_log_err(cx);
    }

    fn set_entropy_prompt_text(
        &mut self,
        text: String,
        parent_prompt_ref: Option<String>,
        source_run_ref: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.entropy_prompt_draft = text.clone();
        self.entropy_parent_prompt_ref = parent_prompt_ref;
        self.entropy_source_run_ref = source_run_ref;
        if let Some(editor) = self.entropy_prompt_editor.clone() {
            editor.update(cx, |editor, cx| editor.set_text(text.clone(), window, cx));
        }
        self.persist_entropy_state(cx);
        cx.notify();
    }

    pub fn reset_entropy_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_entropy_prompt_text(
            DEFAULT_ENTROPY_ANALYSIS_PROMPT.into(),
            None,
            None,
            window,
            cx,
        );
    }

    pub fn copy_latest_entropy_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self
            .entropy_run
            .as_ref()
            .or_else(|| self.entropy_run_history.last())
        else {
            self.status = "No prior entropy run is available to copy".into();
            cx.notify();
            return;
        };
        self.set_entropy_prompt_text(
            run.binding.prompt_snapshot.text.clone(),
            Some(run.binding.prompt_snapshot.prompt_ref.clone()),
            Some(run.binding.run_ref.clone()),
            window,
            cx,
        );
    }

    pub fn start_next_entropy_file(
        &mut self,
        observed_at: String,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<Option<EntropyFileTask>> {
        let run = self
            .entropy_run
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the entropy repository run is unavailable"))?;
        let task = run.start_next_file(observed_at)?;
        if let Some(task) = &task {
            self.status = format!("Reading {}", task.file_path).into();
        } else {
            self.status = entropy_run_status(run).into();
        }
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(task)
    }

    pub fn apply_entropy_output(
        &mut self,
        output: EntropyFileAnalysisOutput,
        observed_at: String,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let run = self
            .entropy_run
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the entropy repository run is unavailable"))?;
        run.apply_output(output, observed_at)?;
        self.status = entropy_run_status(run).into();
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn fail_entropy_file(
        &mut self,
        limitation: EntropyLimitation,
        observed_at: String,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let run = self
            .entropy_run
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the entropy repository run is unavailable"))?;
        run.fail_reading_file(limitation, observed_at)?;
        self.status = entropy_run_status(run).into();
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn observe_entropy_cleanup(
        &mut self,
        receipt_ref: String,
        observed_at: String,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let run = self
            .entropy_run
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the entropy repository run is unavailable"))?;
        run.observe_cleanup(receipt_ref, observed_at)?;
        self.status = entropy_run_status(run).into();
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn cancel_entropy_run(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let run = self
            .entropy_run
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the entropy repository run is unavailable"))?;
        run.cancel(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true))?;
        self.status = entropy_run_status(run).into();
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn set_entropy_error(&mut self, error: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = error.into();
        cx.notify();
    }

    pub(crate) fn set_entropy_status(
        &mut self,
        status: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.status = status.into();
        cx.notify();
    }

    pub(crate) fn set_entropy_repository_source(
        &mut self,
        source: EntropyRepositorySource,
        cx: &mut Context<Self>,
    ) {
        self.entropy_repository_source = Some(source);
        cx.notify();
    }

    pub(crate) fn install_entropy_source_inspection(
        &mut self,
        inspection: EntropySourceInspection,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        inspection.validate()?;
        if let Some(current) = &self.entropy_source_inspection {
            if inspection.generation <= current.generation {
                anyhow::ensure!(
                    inspection == *current,
                    "source inspection generations must advance monotonically"
                );
                return Ok(());
            }
            if inspection.repository != current.repository
                || inspection.top_level_tree != current.top_level_tree
                || (current.state == EntropySourceInspectionState::Complete
                    && inspection.manifest_digest != current.manifest_digest)
            {
                let stale = current.mark_stale(inspection.generation, inspection.observed_at)?;
                self.status = "Source changed · prior inspection is stale".into();
                self.entropy_source_inspection = Some(stale);
                self.persist_entropy_state(cx);
                cx.notify();
                return Ok(());
            }
        }
        self.status = match inspection.state {
            EntropySourceInspectionState::Complete => "Source inspection complete".into(),
            EntropySourceInspectionState::Incomplete => {
                "Source inspection incomplete · partial analysis remains available".into()
            }
            EntropySourceInspectionState::Pending => "Source inspection pending".into(),
            EntropySourceInspectionState::Denied => "Source inspection denied".into(),
            EntropySourceInspectionState::Stale => "Source inspection stale".into(),
        };
        self.entropy_source_inspection = Some(inspection);
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn select_entropy_filter(&mut self, filter: EntropyFileFilter, cx: &mut Context<Self>) {
        self.entropy_file_filter = filter;
        cx.notify();
    }

    pub fn select_entropy_file(&mut self, path: String, cx: &mut Context<Self>) {
        self.selected_entropy_file = Some(path);
        cx.notify();
    }

    pub fn set_managed_preflight(
        &mut self,
        binding: &RepositoryBinding,
        projection: ForensicsPreflightProjection,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            binding == &self.binding,
            "the managed preflight belongs to a different repository binding"
        );
        projection.validate()?;
        let selected_arm = projection
            .target
            .benchmark_arm
            .unwrap_or(ColdcardBenchmarkArm::Vulnerable);
        self.selected_arm = selected_arm;
        self.prepared_intent = None;
        self.run = None;
        self.review = None;
        self.source_resolutions.clear();
        self.status = readiness_label(projection.readiness()).into();
        self.preflight = Some(projection);
        cx.notify();
        Ok(())
    }

    pub fn select_benchmark_arm(&mut self, arm: ColdcardBenchmarkArm, cx: &mut Context<Self>) {
        self.selected_arm = arm;
        self.prepared_intent = None;
        self.run = None;
        self.review = None;
        self.source_resolutions.clear();
        if let Some(preflight) = self.preflight.as_mut() {
            preflight.set_benchmark_arm(arm);
            self.status = "Coverage pending".into();
        }
        cx.notify();
    }

    pub fn acknowledge_incomplete(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let preflight = self
            .preflight
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the managed preflight is unavailable"))?;
        preflight.acknowledge_incomplete()?;
        self.status = "Incomplete research acknowledged".into();
        cx.notify();
        Ok(())
    }

    pub fn prepare_run(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let active_prompt = self.prompt_workspace.active();
        let supported_tools = vec!["tool.source.read".into(), "tool.dependency.inspect".into()];
        self.prompt_workspace
            .check_compatibility(&PromptCompatibilityProfile {
                prompt_artifact_ref: active_prompt.prompt_artifact_ref.clone(),
                finding_schema_ref: FORENSIC_FINDING_SCHEMA_V1.into(),
                hypothesis_schema_ref: FORENSIC_HYPOTHESIS_SCHEMA_V1.into(),
                admitted_tool_refs: supported_tools.clone(),
                runtime_tool_refs: supported_tools,
                compatibility_refs: vec!["compatibility.loupe.v1".into()],
            })?;
        let preflight = self
            .preflight
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("the managed preflight is unavailable"))?;
        let intent = preflight.request_launch(ExplicitOperatorAction {
            action_ref: PREPARE_ACTION_REF.into(),
        })?;
        self.prepared_intent = Some(intent);
        self.status = "Run prepared; no worker launched".into();
        cx.notify();
        Ok(())
    }

    pub fn launch_run(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let intent = self
            .prepared_intent
            .clone()
            .ok_or_else(|| anyhow::anyhow!("prepare the run before launching a worker"))?;
        let commit_prefix = self
            .preflight
            .as_ref()
            .and_then(|preflight| preflight.target.commit.get(..12))
            .ok_or_else(|| anyhow::anyhow!("the repository commit is unavailable"))?;
        let run_ref = format!(
            "run.omega.forensics.{commit_prefix}.{}",
            uuid::Uuid::new_v4().simple()
        );
        let prompt_digest = self.prompt_workspace.bind_run(run_ref.clone())?;
        self.run = Some(ForensicsRunProjection::prepared(run_ref.clone())?);
        self.status = "Launching one OpenAgents Cloud worker…".into();
        cx.emit(ForensicsWorkbenchCommand::Launch {
            run_ref,
            intent,
            prompt_digest,
        });
        cx.notify();
        Ok(())
    }

    pub fn mark_admitting(&mut self, requested_at: String, cx: &mut Context<Self>) {
        if let Some(run) = self.run.as_mut() {
            run.mark_admitting(requested_at);
            self.status = run_phase_label(run.phase).into();
            cx.notify();
        }
    }

    pub fn apply_admission(
        &mut self,
        placement: ForensicWorkerPlacement,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let run = self
            .run
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the forensic run is unavailable"))?;
        run.apply_admission(placement)?;
        self.status = run_phase_label(run.phase).into();
        cx.notify();
        Ok(())
    }

    pub fn apply_observation(
        &mut self,
        observation: ForensicWorkerObservation,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let run = self
            .run
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the forensic run is unavailable"))?;
        run.apply_observation(observation)?;
        self.status = run_phase_label(run.phase).into();
        cx.notify();
        Ok(())
    }

    pub fn mark_cancel_requested(&mut self, requested_at: String, cx: &mut Context<Self>) {
        if let Some(run) = self.run.as_mut() {
            run.mark_cancel_requested(requested_at);
            self.status = run_phase_label(run.phase).into();
            cx.notify();
        }
    }

    pub fn mark_deleting(&mut self, requested_at: String, cx: &mut Context<Self>) {
        if let Some(run) = self.run.as_mut() {
            run.mark_deleting(requested_at);
            self.status = run_phase_label(run.phase).into();
            cx.notify();
        }
    }

    pub fn apply_cleaned_placement(
        &mut self,
        placement: ForensicWorkerPlacement,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let run = self
            .run
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the forensic run is unavailable"))?;
        run.apply_cleaned_placement(placement)?;
        self.status = run_phase_label(run.phase).into();
        cx.notify();
        Ok(())
    }

    pub fn apply_failure(&mut self, failure: ForensicsFailureProjection, cx: &mut Context<Self>) {
        if let Some(run) = self.run.as_mut() {
            run.apply_failure(failure);
            self.status = run_phase_label(run.phase).into();
            cx.notify();
        }
    }

    pub fn set_review_projection(
        &mut self,
        review: ForensicsReviewProjection,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        review.validate()?;
        let expected_commit = self
            .preflight
            .as_ref()
            .map(|preflight| preflight.target.commit.as_str())
            .or(self.repository.commit.as_deref())
            .ok_or_else(|| anyhow::anyhow!("the pinned repository commit is unavailable"))?;
        anyhow::ensure!(
            review.commit == expected_commit,
            "the forensic review belongs to a different pinned source"
        );
        self.status = format!(
            "Review ready · {} findings · {} hypotheses",
            review.findings.len(),
            review.hypotheses.len()
        )
        .into();
        self.review = Some(review);
        self.source_resolutions.clear();
        cx.notify();
        Ok(())
    }

    pub fn set_matrix_projection(
        &mut self,
        matrix: ForensicsMatrixProjection,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        matrix.validate()?;
        self.status = format!(
            "Matrix ready · {} arms · {} retained runs",
            matrix.arms.len(),
            matrix.runs.len()
        )
        .into();
        self.matrix = Some(matrix);
        cx.notify();
        Ok(())
    }

    pub fn set_coldcard_evidence_projection(
        &mut self,
        projection: ColdcardEvidenceWorkspaceProjection,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        projection.validate()?;
        if let Some(run) = &self.run {
            anyhow::ensure!(
                projection.run_ref == run.run_ref,
                "the Coldcard evidence workspace belongs to a different run"
            );
        }
        self.status = format!(
            "Coldcard evidence ready · {} evidenced rungs · private boundary",
            projection
                .ladder
                .iter()
                .filter(|rung| rung.state != omega_forensics::ColdcardRungState::Missing)
                .count()
        )
        .into();
        self.coldcard_evidence = Some(projection);
        self.coldcard_case_reader_state = ColdcardCaseReaderState::Complete;
        self.coldcard_case_selection = ColdcardCaseSelection::Overview;
        cx.notify();
        Ok(())
    }

    pub fn select_coldcard_case(
        &mut self,
        selection: ColdcardCaseSelection,
        cx: &mut Context<Self>,
    ) {
        self.coldcard_case_selection = selection;
        self.persist_entropy_state(cx);
        cx.notify();
    }

    pub fn select_bench_view(&mut self, view: ForensicsBenchView, cx: &mut Context<Self>) {
        if !self.bench_view_available(view) {
            return;
        }
        self.bench_view = view;
        self.persist_entropy_state(cx);
        cx.notify();
    }

    fn restore_bench_view(&mut self, persisted: Option<&str>) {
        let restored = ForensicsBenchView::from_persisted(persisted);
        self.bench_view = if self.bench_view_available(restored) {
            restored
        } else {
            ForensicsBenchView::Entropy
        };
    }

    fn bench_view_available(&self, view: ForensicsBenchView) -> bool {
        match view {
            ForensicsBenchView::Entropy | ForensicsBenchView::Lifecycle => true,
            ForensicsBenchView::Case => self.coldcard_evidence.is_some(),
            ForensicsBenchView::Evidence => self.fixture_views_enabled || self.review.is_some(),
            ForensicsBenchView::Models => self.fixture_views_enabled || self.matrix.is_some(),
            ForensicsBenchView::Publication => self.fixture_views_enabled,
        }
    }

    pub fn select_lifecycle_stage(
        &mut self,
        selection: LifecycleSelection,
        cx: &mut Context<Self>,
    ) {
        self.lifecycle_selection = selection;
        self.persist_entropy_state(cx);
        cx.notify();
    }

    pub fn select_evidence_section(
        &mut self,
        selection: EvidenceSelection,
        cx: &mut Context<Self>,
    ) {
        self.evidence_selection = selection;
        self.persist_entropy_state(cx);
        cx.notify();
    }

    pub fn select_model_run(&mut self, run_ref: String, cx: &mut Context<Self>) {
        self.selected_model_run_ref = Some(run_ref);
        self.persist_entropy_state(cx);
        cx.notify();
    }

    pub fn select_publication_scene(&mut self, scene: PublicationScene, cx: &mut Context<Self>) {
        self.publication_scene = scene;
        self.persist_entropy_state(cx);
        cx.notify();
    }

    pub fn open_source(&mut self, citation: ForensicSourceCitation, cx: &mut Context<Self>) {
        let repository_root = self
            .entropy_repository_source
            .as_ref()
            .map(|source| source.root().to_path_buf())
            .or_else(|| {
                self.selected_entropy_project
                    .as_ref()
                    .and_then(|product_ref| self.entropy_campaign_roots.get(product_ref))
                    .cloned()
            });
        self.source_resolutions.insert(
            citation.source_ref.clone(),
            ForensicSourceResolution::Opening,
        );
        self.status = format!(
            "Resolving {} at line {}…",
            citation.path, citation.start_line
        )
        .into();
        cx.emit(ForensicsWorkbenchCommand::OpenSource {
            citation,
            repository_root,
        });
        cx.notify();
    }

    pub fn apply_source_resolution(
        &mut self,
        source_ref: String,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        let resolution = match result {
            Ok(()) => {
                self.status = "Opened exact pinned source".into();
                ForensicSourceResolution::Opened
            }
            Err(error) => {
                self.status = format!("Source resolution failed: {error}").into();
                ForensicSourceResolution::Failed(error)
            }
        };
        self.source_resolutions.insert(source_ref, resolution);
        cx.notify();
    }

    pub fn record_review_decision(
        &mut self,
        finding_ref: &str,
        decision: ForensicReviewDecisionKind,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let review = self
            .review
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the forensic review is unavailable"))?;
        let decision_label = match decision {
            ForensicReviewDecisionKind::Accept => "accepted",
            ForensicReviewDecisionKind::Correct => "marked for correction",
            ForensicReviewDecisionKind::Reject => "rejected",
        };
        review.append_decision(
            finding_ref,
            decision,
            format!("Omega operator {decision_label} this immutable finding."),
            "reviewer.omega.operator".into(),
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        )?;
        self.status = format!("Review decision appended · {decision_label}").into();
        cx.notify();
        Ok(())
    }

    pub fn request_fixture_independent_verification(
        &mut self,
        finding_ref: &str,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.fixture_views_enabled,
            "live verification requires the complete provider envelope"
        );
        let review = self
            .review
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the forensic review is unavailable"))?;
        let finding = review
            .findings
            .iter()
            .find(|finding| finding.finding_ref == finding_ref)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("the immutable finding is unavailable"))?;
        let poc_ref = finding
            .poc_ref
            .clone()
            .ok_or_else(|| anyhow::anyhow!("verification requires the original candidate PoC"))?;
        let evidence_refs = finding
            .source_refs
            .iter()
            .map(|source| source.source_ref.clone())
            .chain(
                finding
                    .evidence_receipts
                    .iter()
                    .map(|receipt| receipt.receipt_ref.clone()),
            )
            .collect::<Vec<_>>();
        let envelope = IndependentVerifierEnvelope {
            request_ref: format!("verification-request.{finding_ref}"),
            finding: finding.clone(),
            finding_digest: String::new(),
            assumptions: vec![
                "The fixture source and dependency receipts bind the displayed immutable finding."
                    .into(),
            ],
            occurrence_refs: vec![format!("occurrence.{finding_ref}.1")],
            root_cause_ref: finding.claim_ref.clone(),
            source_bundle_ref: format!("source-bundle.{}", review.run_ref),
            source_bundle_digest: fixture_digest('a'),
            coverage_manifest_ref: format!("coverage.{}", review.run_ref),
            coverage_manifest_digest: fixture_digest('b'),
            original_poc: ForensicPocIdentity {
                poc_ref,
                content_digest: fixture_digest('c'),
                supersedes_poc_ref: None,
            },
            discovery_actor_ref: format!("actor.discovery.{}", review.run_ref),
            prompt_digest: review.prompt_digest.clone(),
            prompt_lineage_refs: vec![format!("prompt-lineage.{}", review.run_ref)],
            model_provenance: ForensicModelProvenance {
                provider_ref: "provider.fixture".into(),
                model_ref: "model.fixture".into(),
                route_ref: "model-route.fixture".into(),
                configuration_digest: fixture_digest('d'),
            },
            tool_surface_refs: vec!["tool-surface.omega.forensics.discovery.v1".into()],
            evidence_refs,
            verifier_actor_ref: format!("actor.verifier.{}", review.run_ref),
            verifier_capability_refs: vec![
                "capability.omega.forensics.independent-verifier.v1".into(),
            ],
            vulnerable_revision_digest: fixture_digest('e'),
            fixed_revision_digest: fixture_digest('f'),
            requested_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            canonical_digest: String::new(),
        }
        .seal()?;
        review.request_independent_verification(envelope)?;
        self.status = "Independent verification requested · patch work remains locked".into();
        cx.notify();
        Ok(())
    }

    pub fn install_prior_work(
        &mut self,
        result: ForensicPriorWorkQueryResult,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        result.validate()?;
        self.status = format!(
            "Prior Work refreshed · {} authorized matches",
            result.matches.len()
        )
        .into();
        self.prior_work = Some(result);
        cx.notify();
        Ok(())
    }

    pub fn install_forensic_tool_journal(
        &mut self,
        journal: ForensicToolJournal,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        journal.validate()?;
        self.status = format!(
            "Live forensic tools connected · cursor {}",
            journal.event_cursor()
        )
        .into();
        self.tool_journal = Some(journal);
        self.persist_entropy_state(cx);
        cx.notify();
        Ok(())
    }

    pub fn ingest_visible_forensic_tool_call(
        &mut self,
        tool_name: &str,
        raw_input: &serde_json::Value,
        state: VisibleForensicToolCallState,
        sources: &ForensicSourceCatalog,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<Option<ForensicToolEvent>> {
        let journal = self
            .tool_journal
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("forensic tool journal is not installed"))?;
        let event =
            ingest_visible_forensic_tool_call(tool_name, raw_input, state, journal, sources)?;
        if let Some(event) = &event {
            self.status = match event.status {
                ForensicToolEventStatus::Accepted => format!(
                    "Live forensic tool accepted · {} · cursor {}",
                    event.tool.canonical_name(),
                    event.sequence
                ),
                ForensicToolEventStatus::Rejected => format!(
                    "Live forensic tool rejected · {} · cursor {}",
                    event.tool.canonical_name(),
                    event.sequence
                ),
            }
            .into();
            cx.notify();
            self.persist_entropy_state(cx);
        }
        Ok(event)
    }

    pub fn set_prior_work_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.status = format!("Prior Work search failed · {error}").into();
        cx.notify();
    }

    pub fn snapshot(&self) -> ForensicsWorkbenchSnapshot {
        ForensicsWorkbenchSnapshot {
            binding: self.binding.clone(),
            bench_view: self.bench_view,
            lifecycle_selection: self.lifecycle_selection,
            evidence_selection: self.evidence_selection,
            selected_model_run_ref: self.selected_model_run_ref.clone(),
            publication_scene: self.publication_scene,
            selected_arm: self.selected_arm,
            readiness: self
                .preflight
                .as_ref()
                .map(|preflight| preflight.readiness()),
            prepared_intent: self.prepared_intent.clone(),
            run: self.run.clone(),
            review: self.review.clone(),
            prompt_workspace: self.prompt_workspace.clone(),
            matrix: self.matrix.clone(),
            coldcard_evidence: self.coldcard_evidence.clone(),
            coldcard_case_selection: self.coldcard_case_selection,
            coldcard_case_reader_state: self.coldcard_case_reader_state.clone(),
            prior_work: self.prior_work.clone(),
            tool_journal: self.tool_journal.clone(),
            entropy_run: self.entropy_run.clone(),
            entropy_run_history: self.entropy_run_history.clone(),
            entropy_source_inspection: self.entropy_source_inspection.clone(),
            entropy_prompt_draft: self.entropy_prompt_draft.clone(),
            entropy_parent_prompt_ref: self.entropy_parent_prompt_ref.clone(),
            entropy_source_run_ref: self.entropy_source_run_ref.clone(),
            entropy_prompt_snapshots: self.entropy_prompt_snapshots.clone(),
            entropy_file_filter: self.entropy_file_filter,
            selected_entropy_file: self.selected_entropy_file.clone(),
            entropy_campaign: self.entropy_campaign.clone(),
            entropy_campaign_history: self.entropy_campaign_history.clone(),
            selected_entropy_project: self.selected_entropy_project.clone(),
            source_resolutions: self.source_resolutions.clone(),
            status: self.status.clone(),
            fixture_views_enabled: self.fixture_views_enabled,
        }
    }

    pub fn clone_prompt_candidate(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let candidate_ref = format!(
            "prompt.forensic.omega.candidate.{}",
            uuid::Uuid::new_v4().simple()
        );
        self.prompt_workspace.clone_active(
            candidate_ref,
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        )?;
        self.status = "Editing a save-as prompt candidate".into();
        cx.notify();
        Ok(())
    }

    pub fn update_prompt_draft(
        &mut self,
        prompt_ir: ForensicPromptIr,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        self.prompt_workspace.update_draft_ir(prompt_ir)?;
        self.status = "Prompt candidate edited; active prompt unchanged".into();
        cx.notify();
        Ok(())
    }

    pub fn save_prompt_candidate(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let candidate_ref = self.prompt_workspace.save_draft()?;
        self.status = format!("Saved immutable candidate · {candidate_ref}").into();
        cx.notify();
        Ok(())
    }

    pub fn activate_prompt_candidate(
        &mut self,
        candidate_ref: &str,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        self.prompt_workspace.activate(candidate_ref)?;
        self.prepared_intent = None;
        self.status = "Active prompt pointer changed; prepare the run again".into();
        cx.notify();
        Ok(())
    }

    fn lifecycle_presentation(&self) -> ForensicsLifecyclePresentation {
        if let Some(preflight) = self.preflight.as_ref() {
            let same_repository = self
                .repository
                .clone_url
                .as_deref()
                .is_some_and(|clone_url| clone_url == preflight.target.clone_url);
            if same_repository
                && self.repository.commit.as_deref().is_some_and(|commit| {
                    commit != preflight.target.commit
                        && !commit.starts_with(&preflight.target.commit)
                })
            {
                return ForensicsLifecyclePresentation {
                    scene: ForensicsLifecycleScene::Stale,
                    blocker: "The preflight commit no longer matches the selected project",
                    next_action: "Refresh preflight for the current commit",
                };
            }
            if !preflight
                .worker
                .capability_refs
                .iter()
                .any(|capability| capability.contains("forensics/source-read"))
            {
                return ForensicsLifecyclePresentation {
                    scene: ForensicsLifecycleScene::IncompatibleTool,
                    blocker: "The managed profile does not prove read-only forensic source access",
                    next_action: "Select an admitted compatible profile",
                };
            }
        }

        if let Some(run) = self.run.as_ref() {
            let failure_text = run
                .failure
                .as_ref()
                .map(|failure| {
                    format!("{} {}", failure.reason_ref, failure.message).to_ascii_lowercase()
                })
                .unwrap_or_default();
            if failure_text.contains("stale") {
                return ForensicsLifecyclePresentation {
                    scene: ForensicsLifecycleScene::Stale,
                    blocker: "The worker observation is older than the selected run generation",
                    next_action: "Refresh the run projection before acting",
                };
            }
            if failure_text.contains("incompatible") || failure_text.contains("tool-contract") {
                return ForensicsLifecyclePresentation {
                    scene: ForensicsLifecycleScene::IncompatibleTool,
                    blocker: "The worker tool contract is incompatible with this prompt profile",
                    next_action: "Select a compatible prompt and profile pair",
                };
            }
            return match run.phase {
                ForensicsRunPhase::Cleaned => ForensicsLifecyclePresentation {
                    scene: ForensicsLifecycleScene::Cleaned,
                    blocker: "No lifecycle blocker",
                    next_action: "Inspect retained private evidence",
                },
                ForensicsRunPhase::RecoveryRequired | ForensicsRunPhase::Failed => {
                    ForensicsLifecyclePresentation {
                        scene: ForensicsLifecycleScene::RecoveryRequired,
                        blocker: "The worker did not prove terminal cleanup",
                        next_action: "Recover the exact worker generation and verify cleanup",
                    }
                }
                ForensicsRunPhase::Refused => ForensicsLifecyclePresentation {
                    scene: ForensicsLifecycleScene::Denied,
                    blocker: "Managed placement was refused",
                    next_action: "Inspect the refusal receipt before retrying",
                },
                ForensicsRunPhase::CancelRequested
                | ForensicsRunPhase::Interrupting
                | ForensicsRunPhase::Settled
                    if run.timestamps.cancel_requested_at.is_some() =>
                {
                    ForensicsLifecyclePresentation {
                        scene: ForensicsLifecycleScene::Cancelled,
                        blocker: "Cancellation is retained until cleanup is proven",
                        next_action: "Verify deletion and zero-residue cleanup",
                    }
                }
                ForensicsRunPhase::Admitting
                | ForensicsRunPhase::WorkerReady
                | ForensicsRunPhase::Running
                | ForensicsRunPhase::CancelRequested
                | ForensicsRunPhase::Interrupting
                | ForensicsRunPhase::Settled
                | ForensicsRunPhase::Deleting => ForensicsLifecyclePresentation {
                    scene: ForensicsLifecycleScene::Running,
                    blocker: "Live run controls remain evidence-gated",
                    next_action: "Observe the host-owned lifecycle projection",
                },
                ForensicsRunPhase::Prepared => ForensicsLifecyclePresentation {
                    scene: ForensicsLifecycleScene::Complete,
                    blocker: "Worker launch is not accepted for this build",
                    next_action: "Wait for accepted live worker and source-delivery receipts",
                },
            };
        }

        match self
            .preflight
            .as_ref()
            .map(|preflight| preflight.readiness())
        {
            None => ForensicsLifecyclePresentation {
                scene: ForensicsLifecycleScene::AwaitingProfile,
                blocker: "No admitted OpenAgents managed profile is attached",
                next_action: "Attach an admitted managed profile",
            },
            Some(PreflightReadiness::AwaitingCoverage) => ForensicsLifecyclePresentation {
                scene: ForensicsLifecycleScene::AwaitingCoverage,
                blocker: "The coverage manifest is not terminal",
                next_action: "Wait for a complete, incomplete, or denied manifest",
            },
            Some(PreflightReadiness::Ready) => ForensicsLifecyclePresentation {
                scene: ForensicsLifecycleScene::Complete,
                blocker: "Live prepare and launch are not accepted for this build",
                next_action: "Inspect the preflight receipt and source boundary",
            },
            Some(PreflightReadiness::IncompleteResearch) => ForensicsLifecyclePresentation {
                scene: ForensicsLifecycleScene::Incomplete,
                blocker: "Coverage is incomplete and remains visible in every result",
                next_action: "Inspect missing and excluded source before acknowledgment",
            },
            Some(PreflightReadiness::Denied) => ForensicsLifecyclePresentation {
                scene: ForensicsLifecycleScene::Denied,
                blocker: "Coverage or placement policy denied the run",
                next_action: "Inspect the exact denial reason refs",
            },
        }
    }

    fn render_bench_navigation(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .id("omega.forensics.bench.navigation")
            .debug_selector(|| "omega.forensics.bench.navigation".into())
            .w_full()
            .gap_1()
            .p_1()
            .rounded(px(8.))
            .bg(cx.theme().colors().element_background)
            .role(gpui::Role::List)
            .aria_label("Forensics bench views")
            .children(
                ForensicsBenchView::AVAILABLE
                    .into_iter()
                    .filter(|view| self.bench_view_available(*view))
                    .enumerate()
                    .map(|(index, view)| {
                        let selected = self.bench_view == view;
                        div()
                            .id(("omega.forensics.bench.view", index))
                            .px_3()
                            .py_1p5()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .tab_index(0)
                            .role(gpui::Role::ListItem)
                            .aria_label(format!("{} forensics view", view.label()))
                            .aria_selected(selected)
                            .when(selected, |item| {
                                item.bg(cx.theme().colors().element_selected)
                            })
                            .hover(|item| item.bg(cx.theme().colors().element_hover))
                            .child(Label::new(view.label()).size(LabelSize::Small))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.select_bench_view(view, cx)),
                            )
                            .on_key_down(cx.listener(
                                move |this, event: &gpui::KeyDownEvent, _, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        this.select_bench_view(view, cx);
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                    }),
            )
            .into_any_element()
    }

    fn render_lifecycle_workspace(&self, cx: &mut Context<Self>) -> AnyElement {
        let presentation = self.lifecycle_presentation();
        let target = self.preflight.as_ref().map(|preflight| &preflight.target);
        let coverage = self.preflight.as_ref().map(|preflight| &preflight.coverage);
        let run = self.run.as_ref();
        let selected = self.lifecycle_selection;

        let list = v_flex()
            .id("omega.forensics.lifecycle.list")
            .w(px(260.))
            .flex_shrink_0()
            .gap_1()
            .p_2()
            .border_r_1()
            .border_color(cx.theme().colors().border_variant)
            .role(gpui::Role::List)
            .aria_label("Forensic lifecycle stages")
            .children(
                LifecycleSelection::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, stage)| {
                        let is_selected = selected == stage;
                        div()
                            .id(("omega.forensics.lifecycle.stage", index))
                            .px_3()
                            .py_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .tab_index(0)
                            .role(gpui::Role::ListItem)
                            .aria_label(stage.label())
                            .aria_selected(is_selected)
                            .when(is_selected, |row| {
                                row.bg(cx.theme().colors().element_selected)
                            })
                            .hover(|row| row.bg(cx.theme().colors().element_hover))
                            .child(
                                h_flex()
                                    .w_full()
                                    .justify_between()
                                    .child(Label::new(stage.label()).size(LabelSize::Small))
                                    .child(
                                        Icon::new(if stage == LifecycleSelection::Summary {
                                            IconName::Crosshair
                                        } else {
                                            IconName::ChevronRight
                                        })
                                        .size(IconSize::XSmall)
                                        .color(Color::Muted),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_lifecycle_stage(stage, cx)
                            }))
                            .on_key_down(cx.listener(
                                move |this, event: &gpui::KeyDownEvent, _, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        this.select_lifecycle_stage(stage, cx);
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                    }),
            );

        let detail = v_flex()
            .id("omega.forensics.lifecycle.detail")
            .min_w_0()
            .flex_1()
            .gap_3()
            .p_4()
            .role(gpui::Role::Group)
            .aria_label(format!("{} detail", selected.label()))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .child(Label::new(selected.label()).size(LabelSize::Large))
                    .child(omega_status_cue(
                        "omega.forensics.lifecycle.detail-status",
                        presentation.scene.status(),
                        presentation.scene.label(),
                    )),
            )
            .child(Self::render_fact("State", presentation.scene.label()))
            .when(selected == LifecycleSelection::Summary, |detail| {
                detail
                    .child(Self::render_fact("Blocker", presentation.blocker))
                    .child(Self::render_fact("Next action", presentation.next_action))
                    .child(Self::render_fact("Authority", "OpenAgents projections"))
            })
            .when(selected == LifecycleSelection::Target, |detail| {
                detail
                    .child(Self::render_fact(
                        "Repository",
                        target.map_or(self.repository.display_name.to_string(), |target| {
                            target.display_name.clone()
                        }),
                    ))
                    .child(Self::render_fact(
                        "Commit",
                        target.map_or_else(
                            || {
                                self.repository
                                    .commit
                                    .clone()
                                    .unwrap_or_else(|| "Unavailable".into())
                            },
                            |target| target.commit.clone().into(),
                        ),
                    ))
                    .child(Self::render_fact(
                        "Source",
                        target.map_or("Local project", |target| {
                            source_state_label(target.source_state)
                        }),
                    ))
            })
            .when(selected == LifecycleSelection::Coverage, |detail| {
                detail
                    .child(Self::render_fact(
                        "Manifest",
                        coverage
                            .and_then(|coverage| coverage.manifest_ref.clone())
                            .unwrap_or_else(|| "Unavailable".into()),
                    ))
                    .child(Self::render_fact(
                        "Present / missing",
                        coverage.map_or_else(
                            || "Unavailable".into(),
                            |coverage| format!("{} / {}", coverage.present, coverage.missing),
                        ),
                    ))
                    .child(Self::render_fact(
                        "Reason refs",
                        coverage.map_or_else(
                            || "Unavailable".into(),
                            |coverage| {
                                if coverage.reason_refs.is_empty() {
                                    "None".into()
                                } else {
                                    coverage.reason_refs.join(" · ")
                                }
                            },
                        ),
                    ))
            })
            .when(selected == LifecycleSelection::Profile, |detail| {
                detail
                    .child(Self::render_fact(
                        "Profile",
                        self.preflight.as_ref().map_or("Unavailable", |preflight| {
                            preflight.worker.profile_digest.as_str()
                        }),
                    ))
                    .child(Self::render_fact(
                        "Capabilities",
                        self.preflight.as_ref().map_or_else(
                            || "Unavailable".into(),
                            |preflight| preflight.worker.capability_refs.join(" · "),
                        ),
                    ))
                    .child(Self::render_fact("Network", "Broker only"))
            })
            .when(selected == LifecycleSelection::Runtime, |detail| {
                detail
                    .child(Self::render_fact(
                        "Run",
                        run.map_or("Not prepared", |run| run.run_ref.as_str()),
                    ))
                    .child(Self::render_fact(
                        "Phase",
                        run.map_or("Not started", |run| run_phase_label(run.phase)),
                    ))
                    .child(Self::render_fact(
                        "Failure",
                        run.and_then(|run| run.failure.as_ref())
                            .map_or("None", |failure| failure.message.as_str()),
                    ))
            })
            .when(selected == LifecycleSelection::Cleanup, |detail| {
                detail
                    .child(Self::render_fact(
                        "Deletion receipt",
                        run.and_then(|run| run.placement.as_ref())
                            .and_then(|placement| placement.deletion_receipt_ref.as_deref())
                            .unwrap_or("Unavailable"),
                    ))
                    .child(Self::render_fact(
                        "Cleanup receipt",
                        run.and_then(|run| run.placement.as_ref())
                            .and_then(|placement| placement.cleanup_receipt_ref.as_deref())
                            .unwrap_or("Unavailable"),
                    ))
                    .child(Self::render_fact(
                        "Residue",
                        if presentation.scene == ForensicsLifecycleScene::Cleaned {
                            "Zero residue observed"
                        } else {
                            "Not proven"
                        },
                    ))
            })
            .child(
                h_flex()
                    .gap_2()
                    .pt_2()
                    .child(
                        Button::new("omega.forensics.lifecycle.prepare", "Prepare run")
                            .size(ButtonSize::Compact)
                            .disabled(!LIVE_FORENSIC_CONTROLS_ACCEPTED),
                    )
                    .child(
                        Button::new("omega.forensics.lifecycle.launch", "Launch worker")
                            .size(ButtonSize::Compact)
                            .disabled(!LIVE_FORENSIC_CONTROLS_ACCEPTED),
                    )
                    .child(
                        Label::new("Live controls require accepted worker and source receipts")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );

        v_flex()
            .id("omega.forensics.lifecycle.workspace")
            .debug_selector(|| "omega.forensics.lifecycle.workspace".into())
            .w_full()
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().surface_background)
            .role(gpui::Role::Region)
            .aria_label("Forensic preflight and lifecycle workspace")
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(Label::new("Preflight and lifecycle").size(LabelSize::Small))
                    .child(omega_status_cue(
                        "omega.forensics.lifecycle.header-status",
                        presentation.scene.status(),
                        presentation.scene.label(),
                    )),
            )
            .child(h_flex().w_full().items_stretch().child(list).child(detail))
            .into_any_element()
    }

    fn evidence_section_count(&self, section: EvidenceSelection) -> usize {
        if let Some(journal) = self.tool_journal.as_ref()
            && !journal.events.is_empty()
        {
            return match section {
                EvidenceSelection::Findings => journal.findings.len(),
                EvidenceSelection::Hypotheses => journal.hypotheses.len(),
                EvidenceSelection::Limitations => journal.limitations.len(),
                EvidenceSelection::Disputes => journal
                    .events
                    .iter()
                    .filter(|event| event.status == ForensicToolEventStatus::Rejected)
                    .count(),
                EvidenceSelection::Reconciliation => {
                    journal.diff_applicability.len() + journal.executed_controls.len()
                }
            };
        }
        let Some(workspace) = self.coldcard_evidence.as_ref() else {
            return 0;
        };
        match section {
            EvidenceSelection::Findings => self.review.as_ref().map_or_else(
                || {
                    workspace
                        .ladder
                        .iter()
                        .filter(|rung| rung.state != ColdcardRungState::Missing)
                        .count()
                },
                |review| review.findings.len(),
            ),
            EvidenceSelection::Hypotheses => self.review.as_ref().map_or_else(
                || {
                    workspace
                        .ladder
                        .iter()
                        .filter(|rung| rung.state == ColdcardRungState::Provisional)
                        .count()
                },
                |review| review.hypotheses.len(),
            ),
            EvidenceSelection::Limitations => {
                workspace
                    .ladder
                    .iter()
                    .filter(|rung| rung.state == ColdcardRungState::Missing)
                    .count()
                    + usize::from(!workspace.scan.reportable)
            }
            EvidenceSelection::Disputes => {
                workspace.corrections.len()
                    + workspace
                        .graph_health
                        .iter()
                        .filter(|subject| !subject.complete)
                        .count()
            }
            EvidenceSelection::Reconciliation => workspace.reconciliation.len(),
        }
    }

    fn render_evidence_workspace(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.evidence_selection;
        let state_label = match &self.coldcard_case_reader_state {
            ColdcardCaseReaderState::Loading => "Loading",
            ColdcardCaseReaderState::Empty => "Empty",
            ColdcardCaseReaderState::Invalid(_) => "Invalid",
            ColdcardCaseReaderState::Stale(_) => "Stale",
            ColdcardCaseReaderState::Complete => "Complete",
        };
        let state_color = match &self.coldcard_case_reader_state {
            ColdcardCaseReaderState::Invalid(_) => Color::Error,
            ColdcardCaseReaderState::Stale(_) => Color::Warning,
            ColdcardCaseReaderState::Complete => Color::Success,
            ColdcardCaseReaderState::Loading | ColdcardCaseReaderState::Empty => Color::Muted,
        };

        let list =
            v_flex()
                .id("omega.forensics.evidence.list")
                .w(px(260.))
                .flex_shrink_0()
                .gap_1()
                .p_2()
                .border_r_1()
                .border_color(cx.theme().colors().border_variant)
                .role(gpui::Role::List)
                .aria_label("Forensic evidence queue")
                .children(EvidenceSelection::ALL.into_iter().enumerate().map(
                    |(index, section)| {
                        let is_selected = selected == section;
                        div()
                            .id(("omega.forensics.evidence.section", index))
                            .px_3()
                            .py_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .tab_index(0)
                            .role(gpui::Role::ListItem)
                            .aria_label(format!(
                                "{} evidence section, {} items",
                                section.label(),
                                self.evidence_section_count(section)
                            ))
                            .aria_selected(is_selected)
                            .when(is_selected, |row| {
                                row.bg(cx.theme().colors().element_selected)
                            })
                            .hover(|row| row.bg(cx.theme().colors().element_hover))
                            .child(
                                h_flex()
                                    .w_full()
                                    .justify_between()
                                    .child(Label::new(section.label()).size(LabelSize::Small))
                                    .child(
                                        Label::new(
                                            self.evidence_section_count(section).to_string(),
                                        )
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_evidence_section(section, cx)
                            }))
                            .on_key_down(cx.listener(
                                move |this, event: &gpui::KeyDownEvent, _, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        this.select_evidence_section(section, cx);
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                    },
                ));

        let mut detail = v_flex()
            .id("omega.forensics.evidence.detail")
            .min_w_0()
            .flex_1()
            .gap_3()
            .p_4()
            .role(gpui::Role::Group)
            .aria_label(format!("{} evidence detail", selected.label()))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .child(Label::new(selected.label()).size(LabelSize::Large))
                    .child(
                        Label::new(state_label)
                            .size(LabelSize::XSmall)
                            .color(state_color),
                    ),
            );

        if let Some(journal) = self.tool_journal.as_ref() {
            detail = detail.child(
                Label::new(format!(
                    "Live typed journal · cursor {} · {} accepted · {} rejected",
                    journal.event_cursor(),
                    journal
                        .events
                        .iter()
                        .filter(|event| event.status == ForensicToolEventStatus::Accepted)
                        .count(),
                    journal
                        .events
                        .iter()
                        .filter(|event| event.status == ForensicToolEventStatus::Rejected)
                        .count()
                ))
                .size(LabelSize::Small),
            );
            detail = match selected {
                EvidenceSelection::Findings => {
                    detail.children(journal.findings.iter().map(|finding| {
                        Self::render_fact(
                            format!("Finding · {}", finding.title),
                            format!(
                                "{} · {} · {}",
                                finding.claim_state,
                                finding.evidence_tier.label(),
                                finding.finding_ref
                            ),
                        )
                    }))
                }
                EvidenceSelection::Hypotheses => {
                    detail.children(journal.hypotheses.iter().map(|hypothesis| {
                        Self::render_fact(
                            format!("Hypothesis · {}", hypothesis.hypothesis_ref),
                            format!(
                                "{} · next {}",
                                hypothesis.suspected_mechanism, hypothesis.next_check
                            ),
                        )
                    }))
                }
                EvidenceSelection::Limitations => {
                    detail.children(journal.limitations.iter().map(|limitation| {
                        Self::render_fact(
                            format!("Limitation · {}", limitation.class_ref),
                            format!(
                                "{} · next {}",
                                limitation.message, limitation.required_next_check
                            ),
                        )
                    }))
                }
                EvidenceSelection::Disputes => detail.children(
                    journal
                        .events
                        .iter()
                        .filter(|event| event.status == ForensicToolEventStatus::Rejected)
                        .map(|event| {
                            Self::render_fact(
                                format!(
                                    "Rejected {} · event {}",
                                    event.tool.canonical_name(),
                                    event.sequence
                                ),
                                event
                                    .refusal_ref
                                    .as_deref()
                                    .unwrap_or("typed refusal missing"),
                            )
                        }),
                ),
                EvidenceSelection::Reconciliation => detail
                    .children(journal.diff_applicability.iter().map(|applicability| {
                        Self::render_fact(
                            format!("Diff applicability · {}", applicability.applicability_ref),
                            format!(
                                "applicable {} · artifact only · execution {} · test {}",
                                applicability.applicable,
                                applicability.executed,
                                applicability.test_outcome
                            ),
                        )
                    }))
                    .children(journal.executed_controls.iter().map(|control| {
                        Self::render_fact(
                            format!("Independent executed control · {}", control.control_ref),
                            format!(
                                "{} · {}",
                                control.receipt.outcome, control.receipt.receipt_ref
                            ),
                        )
                    })),
            };
        }

        let Some(workspace) = self.coldcard_evidence.as_ref() else {
            if self.tool_journal.is_none() {
                let message = match &self.coldcard_case_reader_state {
                    ColdcardCaseReaderState::Loading => {
                        "Loading the validated evidence projection…"
                    }
                    ColdcardCaseReaderState::Empty => "No evidence projection is available.",
                    ColdcardCaseReaderState::Invalid(error)
                    | ColdcardCaseReaderState::Stale(error) => error.as_ref(),
                    ColdcardCaseReaderState::Complete => "The evidence projection is unavailable.",
                };
                detail = detail.child(
                    Label::new(message)
                        .size(LabelSize::Small)
                        .color(state_color),
                );
            }
            return v_flex()
                .id("omega.forensics.evidence.workspace")
                .w_full()
                .rounded(px(10.))
                .border_1()
                .border_color(cx.theme().colors().border_variant)
                .bg(cx.theme().colors().surface_background)
                .role(gpui::Role::Region)
                .aria_label("Evidence queue and claim inspector")
                .child(h_flex().w_full().items_stretch().child(list).child(detail))
                .into_any_element();
        };

        detail = match selected {
            EvidenceSelection::Findings => {
                let source_buttons = self
                    .review
                    .iter()
                    .flat_map(|review| &review.findings)
                    .flat_map(|finding| &finding.source_refs)
                    .cloned()
                    .enumerate()
                    .map(|(index, citation)| {
                        let label = format!("{}:{}", citation.path, citation.start_line);
                        Button::new(("omega.forensics.evidence.source", index), label)
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Subtle)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_source(citation.clone(), cx)
                            }))
                    });
                detail
                    .child(Self::render_fact("Claim authority", &workspace.workspace_ref))
                    .child(Self::render_fact(
                        "Evidence ladder",
                        format!(
                            "{} evidenced · {} missing",
                            workspace
                                .ladder
                                .iter()
                                .filter(|rung| rung.state != ColdcardRungState::Missing)
                                .count(),
                            workspace
                                .ladder
                                .iter()
                                .filter(|rung| rung.state == ColdcardRungState::Missing)
                                .count()
                        ),
                    ))
                    .children(workspace.ladder.iter().filter(|rung| {
                        rung.state != ColdcardRungState::Missing
                    }).map(|rung| {
                        Self::render_fact(
                            rung.rung.label(),
                            format!(
                                "{} · {}",
                                coldcard_rung_state_label(rung.state),
                                rung.evidence_refs.join(" · ")
                            ),
                        )
                    }))
                    .child(Label::new("Generator trace").size(LabelSize::Small))
                    .children(workspace.trace.iter().map(|step| {
                        Self::render_fact(
                            format!("{}. {}", step.sequence, step.label),
                            format!(
                                "{} · {} · {}",
                                step.evidence_ref, step.rule_ref, step.verifier_state
                            ),
                        )
                    }))
                    .when_some(self.review.as_ref(), |detail, review| {
                        detail.children(review.findings.iter().map(|finding| {
                            Self::render_fact(
                                format!("Finding · {}", finding.title),
                                format!(
                                    "{} · {} · {}",
                                    finding.claim_state,
                                    finding.evidence_tier.label(),
                                    finding.claim_ref
                                ),
                            )
                        }))
                    })
                    .child(h_flex().gap_2().flex_wrap().children(source_buttons))
            }
            EvidenceSelection::Hypotheses => detail
                .children(self.review.iter().flat_map(|review| &review.hypotheses).map(
                    |hypothesis| {
                        v_flex()
                            .gap_1()
                            .child(Self::render_fact(
                                &hypothesis.hypothesis_ref,
                                &hypothesis.state,
                            ))
                            .child(
                                Label::new(hypothesis.suspected_mechanism.clone())
                                    .size(LabelSize::Small),
                            )
                            .child(Self::render_fact(
                                "Supporting evidence",
                                hypothesis.supporting_refs.join(" · "),
                            ))
                            .child(Self::render_fact(
                                "Missing evidence",
                                hypothesis.missing_evidence.join(" · "),
                            ))
                            .child(Self::render_fact("Next mechanical check", &hypothesis.next_check))
                    },
                ))
                .children(workspace.ladder.iter().filter(|rung| {
                    rung.state == ColdcardRungState::Provisional
                }).map(|rung| {
                    Self::render_fact(
                        format!("Provisional · {}", rung.rung.label()),
                        format!(
                            "{} · next: independent verification",
                            rung.assumptions.join(" · ")
                        ),
                    )
                })),
            EvidenceSelection::Limitations => detail
                .child(Self::render_fact(
                    "Privacy boundary",
                    if workspace.scan.reportable {
                        "Reportable"
                    } else {
                        "Private · non-reportable"
                    },
                ))
                .child(Self::render_fact(
                    "Historical scan",
                    format!(
                        "{} ranges · {}",
                        workspace.scan.ranges.len(),
                        workspace.scan.restart_state
                    ),
                ))
                .children(workspace.ladder.iter().filter(|rung| {
                    rung.state == ColdcardRungState::Missing
                }).map(|rung| {
                    Self::render_fact(
                        format!("Missing rung · {}", rung.rung.label()),
                        format!(
                            "{} · {}",
                            rung.assumptions.join(" · "),
                            rung.non_implications.join(" · ")
                        ),
                    )
                }))
                .child(Self::render_fact(
                    "Next mechanical check",
                    "Establish the next missing rung with independent evidence; do not infer across it",
                )),
            EvidenceSelection::Disputes => detail
                .child(Label::new("Corrections are append-only; prior claims remain inspectable.")
                    .size(LabelSize::Small)
                    .color(Color::Muted))
                .children(workspace.corrections.iter().map(|correction| {
                    Self::render_fact(
                        format!("Correction {} · {}", correction.sequence, correction.claim_ref),
                        format!(
                            "{} → {} · {} · evidence {}",
                            correction.prior_value,
                            correction.corrected_value,
                            correction.reason_ref,
                            correction.appended_evidence_refs.join(" · ")
                        ),
                    )
                }))
                .children(workspace.graph_health.iter().filter(|subject| !subject.complete).map(
                    |subject| {
                        Self::render_fact(
                            format!("Disputed provenance · {}", subject.subject_ref),
                            subject.missing_provenance_refs.join(" · "),
                        )
                    },
                )),
            EvidenceSelection::Reconciliation => detail
                .child(Self::render_fact(
                    "Graph health",
                    format!(
                        "{} complete · {} incomplete",
                        workspace.graph_health.iter().filter(|item| item.complete).count(),
                        workspace.graph_health.iter().filter(|item| !item.complete).count()
                    ),
                ))
                .children(workspace.reconciliation.iter().map(|item| {
                    Self::render_fact(
                        &item.metric_ref,
                        format!(
                            "{:?} · derived {} · published {} · refs {}",
                            item.status,
                            item.derived_value.as_deref().unwrap_or("Unavailable"),
                            item.published_value.as_deref().unwrap_or("Unavailable"),
                            item.source_refs.join(" · ")
                        ),
                    )
                })),
        };

        v_flex()
            .id("omega.forensics.evidence.workspace")
            .debug_selector(|| "omega.forensics.evidence.workspace".into())
            .w_full()
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().surface_background)
            .role(gpui::Role::Region)
            .aria_label("Evidence queue and claim inspector")
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(Label::new("Coldcard evidence queue").size(LabelSize::Small))
                    .child(
                        Label::new("Private · read only")
                            .size(LabelSize::XSmall)
                            .color(Color::Warning),
                    ),
            )
            .child(h_flex().w_full().items_stretch().child(list).child(detail))
            .into_any_element()
    }

    fn render_model_matrix_workspace(&self, cx: &mut Context<Self>) -> AnyElement {
        let using_fixture = self.matrix.is_none() && self.fixture_views_enabled;
        let matrix = self.matrix.clone().or_else(|| {
            using_fixture
                .then(bundled_coldcard_model_matrix)
                .and_then(Result::ok)
        });
        let Some(matrix) = matrix else {
            return v_flex()
                .id("omega.forensics.models.workspace")
                .w_full()
                .gap_2()
                .p_4()
                .rounded(px(10.))
                .border_1()
                .border_color(cx.theme().colors().border_variant)
                .bg(cx.theme().colors().surface_background)
                .role(gpui::Role::Region)
                .aria_label("Forensic model run matrix unavailable")
                .child(Label::new("Model run matrix unavailable").size(LabelSize::Large))
                .child(
                    Label::new("No validated matrix projection can be displayed.")
                        .size(LabelSize::Small)
                        .color(Color::Error),
                )
                .into_any_element();
        };
        let selected_run_ref = self
            .selected_model_run_ref
            .as_deref()
            .filter(|run_ref| matrix.runs.iter().any(|run| run.run_ref == *run_ref))
            .or_else(|| matrix.runs.first().map(|run| run.run_ref.as_str()));
        let selected_run = selected_run_ref
            .and_then(|run_ref| matrix.runs.iter().find(|run| run.run_ref == run_ref));
        let selected_arm =
            selected_run.and_then(|run| matrix.arms.iter().find(|arm| arm.arm_ref == run.arm_ref));

        let list =
            v_flex()
                .id("omega.forensics.models.list")
                .w(px(300.))
                .flex_shrink_0()
                .gap_1()
                .p_2()
                .border_r_1()
                .border_color(cx.theme().colors().border_variant)
                .role(gpui::Role::List)
                .aria_label("Forensic model runs")
                .children(matrix.runs.iter().enumerate().map(|(index, run)| {
                    let is_selected = selected_run_ref == Some(run.run_ref.as_str());
                    let arm = matrix.arms.iter().find(|arm| arm.arm_ref == run.arm_ref);
                    let run_ref = run.run_ref.clone();
                    let keyboard_run_ref = run_ref.clone();
                    div()
                        .id(("omega.forensics.models.run", index))
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .tab_index(0)
                        .role(gpui::Role::ListItem)
                        .aria_label(format!(
                            "{} model run, {}, {}",
                            arm.map_or("Unknown role", |arm| arm.role_ref.as_str()),
                            matrix_outcome_label(run.outcome),
                            if run.eligible_for_identification() {
                                "eligible"
                            } else {
                                "not eligible"
                            }
                        ))
                        .aria_selected(is_selected)
                        .when(is_selected, |row| {
                            row.bg(cx.theme().colors().element_selected)
                        })
                        .hover(|row| row.bg(cx.theme().colors().element_hover))
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    Label::new(arm.map_or("Unknown model family", |arm| {
                                        arm.model_family_ref.as_str()
                                    }))
                                    .size(LabelSize::Small),
                                )
                                .child(
                                    h_flex()
                                        .w_full()
                                        .justify_between()
                                        .gap_2()
                                        .child(
                                            Label::new(arm.map_or("Unknown role", |arm| {
                                                arm.role_ref.as_str()
                                            }))
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                        )
                                        .child(
                                            Label::new(matrix_outcome_label(run.outcome))
                                                .size(LabelSize::XSmall)
                                                .color(matrix_outcome_color(run.outcome)),
                                        ),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_model_run(run_ref.clone(), cx)
                        }))
                        .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                this.select_model_run(keyboard_run_ref.clone(), cx);
                                cx.stop_propagation();
                            }
                        }))
                }));

        let mut detail = v_flex()
            .id("omega.forensics.models.detail")
            .min_w_0()
            .flex_1()
            .gap_3()
            .p_4()
            .role(gpui::Role::Group)
            .aria_label("Selected forensic model run detail")
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .child(Label::new("Run scorecard").size(LabelSize::Large))
                    .child(
                        Label::new(if using_fixture {
                            "Synthetic fixture"
                        } else {
                            "Observed projection"
                        })
                        .size(LabelSize::XSmall)
                        .color(if using_fixture {
                            Color::Warning
                        } else {
                            Color::Success
                        }),
                    ),
            );
        if let (Some(run), Some(arm)) = (selected_run, selected_arm) {
            let unique_findings = matrix
                .finding_divergence
                .unique_finding_refs_by_arm
                .get(&run.arm_ref)
                .cloned()
                .unwrap_or_default();
            detail = detail
                .child(Self::render_fact("Model family", &arm.model_family_ref))
                .child(Self::render_fact("Role", &arm.role_ref))
                .child(Self::render_fact(
                    "Eligibility",
                    if run.eligible_for_identification() {
                        "Eligible for identification metrics"
                    } else {
                        "Not eligible for identification metrics"
                    },
                ))
                .child(Self::render_fact(
                    "Typed outcome",
                    matrix_outcome_label(run.outcome),
                ))
                .child(Self::render_fact(
                    "Censoring",
                    if run.censored {
                        format!(
                            "Right-censored at {} ms",
                            run.censor_at_milliseconds.unwrap_or_default()
                        )
                    } else {
                        "Not censored".into()
                    },
                ))
                .child(Self::render_fact("Prompt digest", &arm.prompt_digest))
                .child(Self::render_fact("Model digest", &arm.model_digest))
                .child(Self::render_fact(
                    "Tokens",
                    aggregate_truth_label(run.total_tokens, run.token_exactness, "tokens"),
                ))
                .child(Self::render_fact(
                    "Cost",
                    aggregate_truth_label(run.cost_micros, run.cost_exactness, "µUSD"),
                ))
                .child(Self::render_fact(
                    "Qualified findings",
                    if run.qualified_finding_refs.is_empty() {
                        "None".into()
                    } else {
                        run.qualified_finding_refs.join(" · ")
                    },
                ))
                .child(Self::render_fact(
                    "Unique disagreement",
                    if unique_findings.is_empty() {
                        "None".into()
                    } else {
                        unique_findings.join(" · ")
                    },
                ))
                .child(div().h_px().bg(cx.theme().colors().border_variant))
                .child(Label::new("Matched comparison").size(LabelSize::Small))
                .child(Self::render_fact(
                    "Dataset revision",
                    &matrix.dataset_revision_digest,
                ))
                .child(Self::render_fact(
                    "Metric definition",
                    &matrix.metric_definition_revision_digest,
                ))
                .child(Self::render_fact(
                    "Evaluator revision",
                    &matrix.evaluator_revision_digest,
                ))
                .child(Self::render_fact(
                    "Common findings",
                    if matrix.finding_divergence.common_finding_refs.is_empty() {
                        "None".into()
                    } else {
                        matrix.finding_divergence.common_finding_refs.join(" · ")
                    },
                ))
                .child(Self::render_fact(
                    "Promotion",
                    "Agreement is not truth; majority vote never promotes a claim",
                ))
                .child(
                    Label::new("Compared under frozen inputs")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                );
        } else {
            detail = detail.child(
                Label::new("The validated matrix contains no selectable runs.")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            );
        }

        v_flex()
            .id("omega.forensics.models.workspace")
            .debug_selector(|| "omega.forensics.models.workspace".into())
            .w_full()
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().surface_background)
            .role(gpui::Role::Region)
            .aria_label("Forensic model panel and run matrix")
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(Label::new("Model panel and run matrix").size(LabelSize::Small))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Label::new(format!(
                                    "{} arms · {} runs",
                                    matrix.arms.len(),
                                    matrix.runs.len(),
                                ))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            )
                            .child(omega_status_cue(
                                "omega.forensics.matrix.promotion-status",
                                if matrix.promoted {
                                    OmegaStatus::Ready
                                } else {
                                    OmegaStatus::Blocked
                                },
                                "Promotion",
                            )),
                    ),
            )
            .child(h_flex().w_full().items_stretch().child(list).child(detail))
            .into_any_element()
    }

    fn render_publication_gate_workspace(&self, cx: &mut Context<Self>) -> AnyElement {
        if !self.fixture_views_enabled {
            return v_flex()
                .id("omega.forensics.publication.source-required")
                .w_full()
                .gap_3()
                .p_4()
                .rounded(px(6.))
                .border_1()
                .border_color(cx.theme().colors().border_variant)
                .child(Label::new("Publication authority").size(LabelSize::Large))
                .child(omega_status_cue(
                    "omega.forensics.publication.source-status",
                    OmegaStatus::Blocked,
                    "Publication",
                ))
                .child(
                    Label::new("No source-owned publication gate projection is attached")
                        .size(LabelSize::Small),
                )
                .child(Self::render_fact(
                    "Authorization",
                    "A completed run, clean worker, relay state, or model agreement cannot authorize a claim",
                ))
                .child(
                    Label::new(
                        "Synthetic publication scenes are available only in explicit development and mock builds.",
                    )
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
                )
                .into_any_element();
        }
        let projection = bundled_publication_gate(self.publication_scene).ok();
        let scenes = v_flex()
            .id("omega.forensics.publication.scenes")
            .w(px(260.))
            .flex_shrink_0()
            .gap_1()
            .p_2()
            .border_r_1()
            .border_color(cx.theme().colors().border_variant)
            .role(gpui::Role::List)
            .aria_label("Publication gate scenes")
            .children(
                PublicationScene::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, scene)| {
                        let selected = self.publication_scene == scene;
                        div()
                            .id(("omega.forensics.publication.scene", index))
                            .px_3()
                            .py_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .tab_index(0)
                            .role(gpui::Role::ListItem)
                            .aria_label(format!("{} publication scene", scene.label()))
                            .aria_selected(selected)
                            .when(selected, |row| row.bg(cx.theme().colors().element_selected))
                            .hover(|row| row.bg(cx.theme().colors().element_hover))
                            .child(Label::new(scene.label()).size(LabelSize::Small))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_publication_scene(scene, cx)
                            }))
                            .on_key_down(cx.listener(
                                move |this, event: &gpui::KeyDownEvent, _, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        this.select_publication_scene(scene, cx);
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                    }),
            );

        let mut detail = v_flex()
            .id("omega.forensics.publication.detail")
            .min_w_0()
            .flex_1()
            .gap_3()
            .p_4()
            .role(gpui::Role::Group)
            .aria_label("Selected publication gate detail")
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .child(Label::new("Publication readiness").size(LabelSize::Large))
                    .child(omega_status_cue(
                        "omega.forensics.publication.detail-status",
                        OmegaStatus::Blocked,
                        "Publication",
                    )),
            )
            .child(
                Label::new("Reports authority receipts; never approves or publishes a case")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );
        if let Some(projection) = projection {
            detail = detail
                .child(
                    h_flex()
                        .w_full()
                        .gap_4()
                        .child(Self::render_fact(
                            "Operator readiness",
                            if projection.operator_ready {
                                "Ready"
                            } else {
                                "Blocked"
                            },
                        ))
                        .child(Self::render_fact(
                            "Maintainer decision",
                            if projection.maintainer_approved {
                                "Approved"
                            } else {
                                "Not approved"
                            },
                        ))
                        .child(Self::render_fact(
                            "Publication authority",
                            if projection.publication_authorized {
                                "Authorized"
                            } else {
                                "Not authorized"
                            },
                        )),
                )
                .children(projection.gates.iter().enumerate().map(|(index, gate)| {
                    v_flex()
                        .id(("omega.forensics.publication.gate", index))
                        .w_full()
                        .gap_1()
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(cx.theme().colors().border_variant)
                        .role(gpui::Role::Group)
                        .aria_label(format!(
                            "{} gate, {}",
                            publication_gate_kind_label(gate.kind),
                            publication_gate_state_label(gate.state)
                        ))
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .gap_3()
                                .child(
                                    Label::new(publication_gate_kind_label(gate.kind))
                                        .size(LabelSize::Small),
                                )
                                .child(
                                    Label::new(publication_gate_state_label(gate.state))
                                        .size(LabelSize::XSmall)
                                        .color(publication_gate_state_color(gate.state)),
                                ),
                        )
                        .child(Label::new(gate.blocker.clone()).size(LabelSize::XSmall))
                        .child(
                            Label::new(format!(
                                "Evidence · {}",
                                gate.evidence_ref.as_deref().unwrap_or("missing")
                            ))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                        )
                        .child(
                            Label::new(format!("Next · {}", gate.next_action))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                }));
        } else {
            detail = detail.child(
                Label::new("The publication gate projection is invalid or unavailable.")
                    .size(LabelSize::Small)
                    .color(Color::Error),
            );
        }

        v_flex()
            .id("omega.forensics.publication.workspace")
            .debug_selector(|| "omega.forensics.publication.workspace".into())
            .w_full()
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().surface_background)
            .role(gpui::Role::Region)
            .aria_label("Forensic publication gate")
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(Label::new("Publication gate").size(LabelSize::Small))
                    .child(
                        Label::new("Synthetic case · read only")
                            .size(LabelSize::XSmall)
                            .color(Color::Warning),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_stretch()
                    .child(scenes)
                    .child(detail),
            )
            .into_any_element()
    }

    fn render_workbench_header(
        &self,
        repository_name: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .justify_between()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Icon::new(IconName::Crosshair).size(IconSize::Medium))
                            .child(Label::new("Entropy forensics").size(LabelSize::Large)),
                    )
                    .child(
                        Label::new(
                            "Trace entropy sources, evidence, and lifecycle truth across pinned source.",
                        )
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .when(self.fixture_views_enabled, |header| {
                        header.child(
                            div()
                                .id("omega.forensics.dev-mocks")
                                .debug_selector(|| "omega.forensics.dev-mocks".into())
                                .role(gpui::Role::Status)
                                .aria_label("Development mock data")
                                .child(
                                    Label::new("DEV MOCKS")
                                        .size(LabelSize::XSmall)
                                        .color(Color::Warning),
                                ),
                        )
                    })
                    .child(
                        h_flex()
                            .max_w(px(320.))
                            .gap_2()
                            .px_3()
                            .py_1p5()
                            .rounded_full()
                            .bg(cx.theme().colors().element_background)
                            .child(Icon::new(IconName::Folder).size(IconSize::Small))
                            .child(div().truncate().child(repository_name)),
                    ),
            )
            .into_any_element()
    }

    fn render_fact(
        label: impl Into<SharedString>,
        value: impl Into<SharedString>,
    ) -> impl IntoElement {
        let label = label.into();
        let value = value.into();
        h_flex()
            .w_full()
            .gap_2()
            .justify_between()
            .child(
                Label::new(label)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(Label::new(value).size(LabelSize::XSmall).line_clamp(1))
    }

    fn render_coldcard_case_reader(&self, cx: &mut Context<Self>) -> AnyElement {
        let state_content = match &self.coldcard_case_reader_state {
            ColdcardCaseReaderState::Loading => Some((
                gpui::Role::Status,
                "Loading the bundled Coldcard case…".to_string(),
                Color::Muted,
            )),
            ColdcardCaseReaderState::Empty => Some((
                gpui::Role::Status,
                "No Coldcard case projection is available.".to_string(),
                Color::Muted,
            )),
            ColdcardCaseReaderState::Invalid(error) => {
                Some((gpui::Role::Alert, error.to_string(), Color::Error))
            }
            ColdcardCaseReaderState::Stale(reason) => Some((
                gpui::Role::Status,
                format!("Coldcard case is stale: {reason}"),
                Color::Warning,
            )),
            ColdcardCaseReaderState::Complete => None,
        };

        let shell = v_flex()
            .id("omega.forensics.coldcard.case-reader")
            .debug_selector(|| "omega.forensics.coldcard.case-reader".into())
            .w_full()
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().surface_background)
            .role(gpui::Role::Region)
            .aria_label("Read-only Coldcard case reader")
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_4()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Icon::new(IconName::Lock)
                                            .size(IconSize::Small)
                                            .color(Color::Muted),
                                    )
                                    .child(Label::new("Coldcard case").size(LabelSize::Small)),
                            )
                            .child(
                                Label::new("Synthetic evidence fixture · read-only")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Label::new("PRIVATE · NON-REPORTABLE")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Warning),
                            )
                            .child(
                                Button::new(
                                    "omega.forensics.coldcard.live",
                                    "Live run unavailable",
                                )
                                .size(ButtonSize::Compact)
                                .style(ButtonStyle::Subtle)
                                .disabled(true),
                            ),
                    ),
            );

        if let Some((role, message, color)) = state_content {
            return shell
                .child(
                    div()
                        .id("omega.forensics.coldcard.case.state")
                        .role(role)
                        .aria_label(message.clone())
                        .p_4()
                        .child(Label::new(message).size(LabelSize::Small).color(color)),
                )
                .into_any_element();
        }

        let Some(workspace) = self.coldcard_evidence.as_ref() else {
            return shell
                .child(
                    div()
                        .id("omega.forensics.coldcard.case.missing")
                        .role(gpui::Role::Alert)
                        .aria_label("The validated Coldcard case projection is missing")
                        .p_4()
                        .child(
                            Label::new("The validated Coldcard case projection is missing.")
                                .size(LabelSize::Small)
                                .color(Color::Error),
                        ),
                )
                .into_any_element();
        };

        let target = RepositoryTargetProjection::coldcard(ColdcardBenchmarkArm::Vulnerable);
        let evidenced = workspace
            .ladder
            .iter()
            .filter(|rung| rung.state != ColdcardRungState::Missing)
            .count();
        let missing = workspace.ladder.len().saturating_sub(evidenced);
        let incomplete_provenance = workspace
            .graph_health
            .iter()
            .filter(|health| !health.complete)
            .count();

        let overview_selected = self.coldcard_case_selection == ColdcardCaseSelection::Overview;
        let overview_row = div()
            .id("omega.forensics.coldcard.case.overview")
            .debug_selector(|| "omega.forensics.coldcard.case.overview".into())
            .w_full()
            .px_3()
            .py_2()
            .rounded(px(6.))
            .cursor_pointer()
            .role(gpui::Role::ListItem)
            .tab_index(0)
            .aria_label(format!(
                "Case overview, {evidenced} of {} evidence rungs complete",
                workspace.ladder.len()
            ))
            .aria_selected(overview_selected)
            .when(overview_selected, |row| {
                row.bg(cx.theme().colors().element_selected)
            })
            .hover(|row| row.bg(cx.theme().colors().element_hover))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .child(Label::new("Case overview").size(LabelSize::Small))
                    .child(
                        Label::new(format!("{evidenced}/{}", workspace.ladder.len()))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.select_coldcard_case(ColdcardCaseSelection::Overview, cx);
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.select_coldcard_case(ColdcardCaseSelection::Overview, cx);
                    cx.stop_propagation();
                }
            }));

        let ladder = v_flex()
            .id("omega.forensics.coldcard.case.ladder")
            .w(px(288.))
            .flex_shrink_0()
            .gap_1()
            .p_2()
            .border_r_1()
            .border_color(cx.theme().colors().border_variant)
            .role(gpui::Role::List)
            .aria_label("Coldcard evidence ladder")
            .child(overview_row)
            .children(workspace.ladder.iter().enumerate().map(|(index, rung)| {
                let rung_id = rung.rung;
                let selected = self.coldcard_case_selection == ColdcardCaseSelection::Rung(rung_id);
                let state_label = coldcard_rung_state_label(rung.state);
                div()
                    .id(("omega.forensics.coldcard.case.rung", index))
                    .debug_selector(move || {
                        format!("omega.forensics.coldcard.case.rung.{}", index + 1)
                    })
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .role(gpui::Role::ListItem)
                    .tab_index(0)
                    .aria_label(format!(
                        "Evidence rung {} of 9, {}, {state_label}",
                        index + 1,
                        rung.rung.label()
                    ))
                    .aria_selected(selected)
                    .when(selected, |row| row.bg(cx.theme().colors().element_selected))
                    .hover(|row| row.bg(cx.theme().colors().element_hover))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                Icon::new(if rung.state == ColdcardRungState::Missing {
                                    IconName::Dash
                                } else {
                                    IconName::Check
                                })
                                .size(IconSize::XSmall)
                                .color(coldcard_rung_state_color(rung.state)),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .child(Label::new(rung.rung.label()).size(LabelSize::Small)),
                            )
                            .child(
                                Label::new(format!("{:02}", index + 1))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_coldcard_case(ColdcardCaseSelection::Rung(rung_id), cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.select_coldcard_case(ColdcardCaseSelection::Rung(rung_id), cx);
                            cx.stop_propagation();
                        }
                    }))
            }));

        let detail = match self.coldcard_case_selection {
            ColdcardCaseSelection::Overview => v_flex()
                .gap_3()
                .child(
                    v_flex()
                        .gap_1()
                        .child(Label::new("Case overview").size(LabelSize::Large))
                        .child(
                            Label::new("Validated public-safe fixture for the evidence boundary")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        ),
                )
                .child(Self::render_fact(
                    "Execution",
                    "No worker launch · no live result asserted",
                ))
                .child(Self::render_fact("Target", target.display_name))
                .child(Self::render_fact("Repository", target.clone_url))
                .child(Self::render_fact("Pinned commit", target.commit))
                .child(Self::render_fact("Source", "Bundled synthetic fixture"))
                .child(Self::render_fact("Privacy", "Private · non-reportable"))
                .child(Self::render_fact(
                    "Highest supported rung",
                    "Program fingerprint · provisional",
                ))
                .child(Self::render_fact(
                    "Completeness",
                    format!("{evidenced} evidenced · {missing} missing"),
                ))
                .child(Self::render_fact(
                    "Provenance",
                    if incomplete_provenance == 0 {
                        "Complete".to_string()
                    } else {
                        format!("{incomplete_provenance} incomplete projection")
                    },
                ))
                .child(Self::render_fact(
                    "Workspace",
                    workspace.workspace_ref.clone(),
                ))
                .child(Self::render_fact("Fixture run", workspace.run_ref.clone()))
                .into_any_element(),
            ColdcardCaseSelection::Rung(rung_id) => {
                let rung = workspace.ladder.iter().find(|rung| rung.rung == rung_id);
                match rung {
                    Some(rung) => v_flex()
                        .gap_3()
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .gap_3()
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(Label::new(rung.rung.label()).size(LabelSize::Large))
                                        .child(
                                            Label::new(rung.verifier_state.clone())
                                                .size(LabelSize::Small)
                                                .color(Color::Muted),
                                        ),
                                )
                                .child(
                                    Label::new(coldcard_rung_state_label(rung.state))
                                        .size(LabelSize::XSmall)
                                        .color(coldcard_rung_state_color(rung.state)),
                                ),
                        )
                        .child(Self::render_fact("Time to rung", rung.time_to_rung.display_value()))
                        .child(Self::render_fact(
                            "Tokens to rung",
                            rung.tokens_to_rung.display_value(),
                        ))
                        .child(Self::render_fact(
                            "Evidence",
                            if rung.evidence_refs.is_empty() {
                                "Missing — not inferred downstream".to_string()
                            } else {
                                rung.evidence_refs.join(" · ")
                            },
                        ))
                        .child(Self::render_fact("Assumptions", rung.assumptions.join(" · ")))
                        .child(Self::render_fact(
                            "Does not imply",
                            rung.non_implications.join(" · "),
                        ))
                        .when(rung.state == ColdcardRungState::Missing, |this| {
                            this.child(
                                div()
                                    .id("omega.forensics.coldcard.case.missing-rung")
                                    .role(gpui::Role::Status)
                                    .aria_label("This missing rung blocks downstream attribution")
                                    .p_3()
                                    .rounded(px(8.))
                                    .bg(cx.theme().colors().element_background)
                                    .child(
                                        Label::new(
                                            "Missing evidence remains missing; no downstream inference is promoted into this rung",
                                        )
                                        .size(LabelSize::Small)
                                        .color(Color::Warning),
                                    ),
                            )
                        })
                        .into_any_element(),
                    None => div()
                        .id("omega.forensics.coldcard.case.unavailable-rung")
                        .role(gpui::Role::Alert)
                        .aria_label("Selected evidence rung is unavailable")
                        .child(
                            Label::new("Selected evidence rung is unavailable.")
                                .size(LabelSize::Small)
                                .color(Color::Error),
                        )
                        .into_any_element(),
                }
            }
        };

        shell
            .child(
                h_flex().w_full().items_stretch().child(ladder).child(
                    div()
                        .id("omega.forensics.coldcard.case.detail")
                        .min_w_0()
                        .flex_1()
                        .p_4()
                        .role(gpui::Role::Group)
                        .aria_label("Selected Coldcard case detail")
                        .child(detail),
                ),
            )
            .into_any_element()
    }

    fn render_prior_work_reader(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = ForensicPriorWorkQuery {
            query_ref: "query:omega:forensics-workbench:prior-work".into(),
            principal_ref: "principal:omega:local-owner".into(),
            organization_refs: vec!["organization:openagents".into()],
            include_public: true,
            mode: ForensicPriorWorkQueryMode::Semantic,
            exact_ref: None,
            text: Some(format!(
                "{} entropy security root cause",
                self.repository.display_name
            )),
            disposition_filter: ForensicWorkDisposition::ALL.into(),
            cursor: None,
            limit: 25,
        };
        let result = self.prior_work.clone();
        let result_is_none = result.is_none();
        v_flex()
            .id("omega.forensics.prior-work")
            .debug_selector(|| "omega.forensics.prior-work".into())
            .w_full()
            .gap_3()
            .p_4()
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().surface_background)
            .role(gpui::Role::Region)
            .aria_label("Prior forensic Work")
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(Label::new("Prior forensic Work").size(LabelSize::Small))
                            .child(
                                Label::new(
                                    "Exact occurrences and causal root causes across retained dispositions",
                                )
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            ),
                    )
                    .child(
                        Button::new("omega.forensics.prior-work.refresh", "Search prior Work")
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Subtle)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(ForensicsWorkbenchCommand::RefreshPriorWork {
                                    query: query.clone(),
                                });
                            })),
                    ),
            )
            .when(result_is_none, |this| {
                this.child(
                    Label::new("No prior-work query receipt is loaded.")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
            })
            .when_some(result, |this, result| {
                this.child(Self::render_fact(
                    "Completeness",
                    if result.receipt.authorized_population_complete {
                        "Complete"
                    } else {
                        "Partial"
                    },
                ))
                .child(Self::render_fact(
                    "Authorized population",
                    format!(
                        "{} searched · {} returned · {} losses",
                        result.receipt.searched_authorized_count,
                        result.receipt.returned_count,
                        result.receipt.loss_refs.len()
                    ),
                ))
                .child(Self::render_fact(
                    "Receipt",
                    result.receipt.receipt_ref.clone(),
                ))
                .children(result.matches.into_iter().map(|matched| {
                    v_flex()
                        .gap_1()
                        .p_3()
                        .rounded(px(8.))
                        .bg(cx.theme().colors().element_background)
                        .child(
                            Label::new(matched.record.root_cause.causal_mechanism)
                                .size(LabelSize::Small),
                        )
                        .child(
                            Label::new(format!(
                                "{} occurrences · {} Work refs · match {} bp",
                                matched.record.occurrences.len(),
                                matched.record.work_refs.len(),
                                matched.score_basis_points
                            ))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                        )
                }))
            })
            .into_any_element()
    }
}

impl EventEmitter<ForensicsWorkbenchCommand> for ForensicsWorkbenchSurface {}

impl Focusable for ForensicsWorkbenchSurface {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ForensicsWorkbenchSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.bench_view_available(self.bench_view) {
            self.bench_view = ForensicsBenchView::Entropy;
        }
        let target = self.preflight.as_ref().map(|preflight| &preflight.target);
        let repository_name = target
            .map(|target| SharedString::from(target.display_name.clone()))
            .unwrap_or_else(|| self.repository.display_name.clone());
        let clone_url = target
            .map(|target| SharedString::from(target.clone_url.clone()))
            .or_else(|| self.repository.clone_url.clone())
            .unwrap_or_else(|| "No public HTTPS remote".into());
        let commit = target
            .map(|target| SharedString::from(target.commit.clone()))
            .or_else(|| self.repository.commit.clone())
            .unwrap_or_else(|| "No commits".into());
        let source_state: SharedString = target
            .map(|target| source_state_label(target.source_state).into())
            .unwrap_or_else(|| {
                if self.repository.dirty_files == 0 {
                    "Clean".into()
                } else {
                    format!("Dirty · {} files", self.repository.dirty_files).into()
                }
            });
        let dependency_policy = target
            .map(|target| dependency_policy_label(target.dependency_policy))
            .unwrap_or("Pinned recursive");
        let readiness = self
            .preflight
            .as_ref()
            .map(|preflight| preflight.readiness());
        let coverage = self.preflight.as_ref().map(|preflight| &preflight.coverage);
        let can_prepare = LIVE_FORENSIC_CONTROLS_ACCEPTED
            && self.preflight.as_ref().is_some_and(|preflight| {
                matches!(preflight.readiness(), PreflightReadiness::Ready)
                    || (preflight.readiness() == PreflightReadiness::IncompleteResearch
                        && preflight.incomplete_acknowledged)
            });
        let needs_acknowledgment = self.preflight.as_ref().is_some_and(|preflight| {
            preflight.coverage.status == CoverageStatus::Incomplete
                && !preflight.incomplete_acknowledged
        });
        let run_phase = self.run.as_ref().map(|run| run.phase);
        let can_launch =
            LIVE_FORENSIC_CONTROLS_ACCEPTED && self.prepared_intent.is_some() && self.run.is_none();
        let can_refresh = LIVE_FORENSIC_CONTROLS_ACCEPTED
            && run_phase.is_some_and(|phase| {
                !matches!(
                    phase,
                    ForensicsRunPhase::Prepared
                        | ForensicsRunPhase::Admitting
                        | ForensicsRunPhase::Cleaned
                        | ForensicsRunPhase::Refused
                        | ForensicsRunPhase::Failed
                )
            });
        let can_cancel = LIVE_FORENSIC_CONTROLS_ACCEPTED
            && run_phase.is_some_and(|phase| matches!(phase, ForensicsRunPhase::Running));
        let can_cleanup = LIVE_FORENSIC_CONTROLS_ACCEPTED
            && run_phase.is_some_and(|phase| {
                matches!(
                    phase,
                    ForensicsRunPhase::WorkerReady
                        | ForensicsRunPhase::Settled
                        | ForensicsRunPhase::RecoveryRequired
                )
            });
        let review = self.review.clone();
        let fixture_views_enabled = self.fixture_views_enabled;
        let source_resolutions = self.source_resolutions.clone();
        let prompt_workspace = self.prompt_workspace.clone();
        let active_prompt = prompt_workspace.active().clone();
        let prompt_changes = prompt_workspace.semantic_diff().unwrap_or_default();
        let prompt_candidates = prompt_workspace.candidates().cloned().collect::<Vec<_>>();
        let matrix = self.matrix.clone();
        let entropy_run = self.entropy_run.clone();
        let entropy_source_inspection = self.entropy_source_inspection.clone();
        let entropy_catalog = self.entropy_catalog.clone();
        let entropy_campaign = self.entropy_campaign.clone();
        let selected_entropy_project = self.selected_entropy_project.clone();
        let selected_campaign_project = entropy_campaign.as_ref().and_then(|campaign| {
            selected_entropy_project
                .as_deref()
                .and_then(|product_ref| campaign.project(product_ref))
        });
        let entropy_run_workbench = if entropy_campaign.is_some() {
            selected_campaign_project.and_then(|project| project.run.clone())
        } else {
            entropy_run.clone()
        };
        let entropy_campaign_comparison = self
            .entropy_campaign_history
            .last()
            .zip(entropy_campaign.as_ref())
            .and_then(|(prior, current)| EntropyCampaignComparison::between(prior, current).ok());
        let entropy_filter = self.entropy_file_filter;
        let selected_entropy_file = self.selected_entropy_file.clone();
        let entropy_comparison = self
            .entropy_run_history
            .last()
            .zip(entropy_run.as_ref())
            .filter(|(prior, current)| prior.binding.repository == current.binding.repository)
            .map(|(prior, current)| compare_entropy_runs(prior, current));
        let entropy_prompt_editor = self.entropy_prompt_editor.clone();
        let entropy_prompt_draft = self.entropy_prompt_draft.clone();
        let entropy_parent_prompt_ref = self.entropy_parent_prompt_ref.clone();
        let entropy_source_run_ref = self.entropy_source_run_ref.clone();
        let next_prompt_digest = omega_forensics::entropy_prompt_digest(&entropy_prompt_draft).ok();
        let entropy_running = entropy_run.as_ref().is_some_and(|run| {
            matches!(
                run.phase,
                EntropyRunPhase::Ready
                    | EntropyRunPhase::Running
                    | EntropyRunPhase::CancelRequested
            )
        });
        let entropy_campaign_active = entropy_campaign.as_ref().is_some_and(|campaign| {
            matches!(
                campaign.phase,
                EntropyCampaignPhase::Ready
                    | EntropyCampaignPhase::Running
                    | EntropyCampaignPhase::Paused
            )
        });
        let visible_status = self.status.clone();
        let status_color = if visible_status.to_ascii_lowercase().contains("failed")
            || visible_status.to_ascii_lowercase().contains("error")
            || visible_status.to_ascii_lowercase().contains("configure")
        {
            Color::Error
        } else if entropy_running || entropy_campaign_active {
            Color::Accent
        } else {
            Color::Muted
        };
        let coldcard_case_reader = self.render_coldcard_case_reader(cx);
        let header = self.render_workbench_header(repository_name.clone(), cx);
        let navigation = self.render_bench_navigation(cx);

        if self.bench_view == ForensicsBenchView::Case {
            let prior_work = self.render_prior_work_reader(cx);
            return v_flex()
                .id("omega.forensics.workbench")
                .track_focus(&self.focus_handle)
                .tab_index(0)
                .role(gpui::Role::Group)
                .aria_label("Forensics case workspace")
                .size_full()
                .overflow_y_scroll()
                .p_6()
                .gap_4()
                .child(header)
                .child(navigation)
                .child(prior_work)
                .child(coldcard_case_reader)
                .into_any_element();
        }

        if self.bench_view == ForensicsBenchView::Lifecycle {
            let lifecycle = self.render_lifecycle_workspace(cx);
            return v_flex()
                .id("omega.forensics.workbench")
                .track_focus(&self.focus_handle)
                .tab_index(0)
                .role(gpui::Role::Group)
                .aria_label("Forensics lifecycle workspace")
                .size_full()
                .overflow_y_scroll()
                .p_6()
                .gap_4()
                .child(header)
                .child(navigation)
                .child(lifecycle)
                .into_any_element();
        }

        if self.bench_view == ForensicsBenchView::Evidence {
            let evidence = self.render_evidence_workspace(cx);
            return v_flex()
                .id("omega.forensics.workbench")
                .track_focus(&self.focus_handle)
                .tab_index(0)
                .role(gpui::Role::Group)
                .aria_label("Forensics evidence workspace")
                .size_full()
                .overflow_y_scroll()
                .p_6()
                .gap_4()
                .child(header)
                .child(navigation)
                .child(evidence)
                .into_any_element();
        }

        if self.bench_view == ForensicsBenchView::Models {
            let models = self.render_model_matrix_workspace(cx);
            return v_flex()
                .id("omega.forensics.workbench")
                .track_focus(&self.focus_handle)
                .tab_index(0)
                .role(gpui::Role::Group)
                .aria_label("Forensics model matrix workspace")
                .size_full()
                .overflow_y_scroll()
                .p_6()
                .gap_4()
                .child(header)
                .child(navigation)
                .child(models)
                .into_any_element();
        }

        if self.bench_view == ForensicsBenchView::Publication {
            let publication = self.render_publication_gate_workspace(cx);
            return v_flex()
                .id("omega.forensics.workbench")
                .track_focus(&self.focus_handle)
                .tab_index(0)
                .role(gpui::Role::Group)
                .aria_label("Forensics publication gate workspace")
                .size_full()
                .overflow_y_scroll()
                .p_6()
                .gap_4()
                .child(header)
                .child(navigation)
                .child(publication)
                .into_any_element();
        }

        v_flex()
            .id("omega.forensics.workbench")
            .debug_selector(|| "omega.forensics.workbench".to_string())
            .track_focus(&self.focus_handle)
            .tab_index(0)
            .role(gpui::Role::Group)
            .aria_label("Forensics preflight workbench")
            .size_full()
            .overflow_y_scroll()
            .p_6()
            .gap_4()
            .child(header)
            .child(navigation)
            .child(
                h_flex()
                    .id("omega.forensics.entropy.status")
                    .debug_selector(|| "omega.forensics.entropy.status".into())
                    .w_full()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(cx.theme().colors().border_variant)
                    .bg(cx.theme().colors().element_background)
                    .child(div().size(px(7.)).rounded_full().bg(status_color.color(cx)))
                    .child(
                        Label::new(visible_status)
                            .size(LabelSize::XSmall)
                            .color(status_color),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(cx.theme().colors().border_variant)
                    .bg(cx.theme().colors().surface_background)
                    .child(
                        h_flex()
                            .justify_between()
                            .child(Label::new("Repository scan").size(LabelSize::Small))
                            .child(
                                Label::new(source_state.clone())
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(
                        Label::new("Read-only traversal using the configured model")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Entropy prompt")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .when_some(entropy_prompt_editor, |this, editor| {
                        this.child(
                            div()
                                .h(px(128.))
                                .p_2()
                                .rounded(px(8.))
                                .border_1()
                                .border_color(cx.theme().colors().border_variant)
                                .bg(cx.theme().colors().background)
                                .child(editor),
                        )
                    })
                    .when(self.entropy_prompt_editor.is_none(), |this| {
                        this.child(Label::new(entropy_prompt_draft.clone()).size(LabelSize::XSmall))
                    })
                    .when_some(next_prompt_digest, |this, digest| {
                        this.child(Self::render_fact("Next prompt digest", digest))
                    })
                    .when_some(entropy_parent_prompt_ref, |this, prompt_ref| {
                        this.child(Self::render_fact("Parent prompt", prompt_ref))
                    })
                    .when_some(entropy_source_run_ref, |this, run_ref| {
                        this.child(Self::render_fact("Copied from run", run_ref))
                    })
                    .when_some(entropy_run, |this, run| {
                        let counts = run.counts();
                        this.child(Self::render_fact(
                            "Progress",
                            format!(
                                "{} queued · {} reading · {} analyzed · {} candidates",
                                counts.queued, counts.reading, counts.analyzed, counts.candidate
                            ),
                        ))
                        .child(Self::render_fact(
                            "Limitations",
                            format!("{} skipped · {} failed", counts.skipped, counts.failed),
                        ))
                        .child(Self::render_fact(
                            "Frozen prompt digest",
                            run.binding.prompt_digest.clone(),
                        ))
                        .child(Self::render_fact("Model route", run.binding.model_route_ref.clone()))
                        .child(Self::render_fact(
                            "Model parameters",
                            format!(
                                "temperature {} · thinking {}",
                                run.binding.model_parameters.temperature_millis,
                                run.binding.model_parameters.thinking_allowed
                            ),
                        ))
                    })
                    .child(
                        h_flex().flex_wrap()
                            .gap_1()
                            .child(
                                Button::new("omega.forensics.entropy.reset", "Reset prompt")
                                    .size(ButtonSize::Compact)
                                    .style(ButtonStyle::Subtle)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reset_entropy_prompt(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("omega.forensics.entropy.copy", "Use prior prompt")
                                    .size(ButtonSize::Compact)
                                    .style(ButtonStyle::Subtle)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.copy_latest_entropy_prompt(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("omega.forensics.entropy.start", "Run entropy scan")
                                    .size(ButtonSize::Compact)
                                    .debug_selector(|| "omega.forensics.entropy.start".into())
                                    .disabled(entropy_running || entropy_campaign_active || self.entropy_prompt_draft.trim().is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Err(error) = this.request_entropy_run(cx) {
                                            this.set_entropy_error(format!("Entropy prompt is invalid · {error}"), cx);
                                        }
                                    })),
                            )
                            .child(
                                Button::new(
                                    "omega.forensics.entropy.campaign.start",
                                    format!(
                                        "Run {}-target campaign",
                                        entropy_catalog.projects.len()
                                    ),
                                )
                                .size(ButtonSize::Compact)
                                .disabled(
                                    entropy_campaign_active
                                        || entropy_running
                                        || self.entropy_prompt_draft.trim().is_empty(),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Err(error) = this.request_entropy_campaign(cx) {
                                        this.set_entropy_error(
                                            format!("Entropy campaign prompt is invalid · {error}"),
                                            cx,
                                        );
                                    }
                                })),
                            )
                            .when(entropy_running, |this| {
                                this.child(
                                    Button::new(
                                        "omega.forensics.entropy.cancel",
                                        "Cancel entropy scan",
                                    )
                                    .size(ButtonSize::Compact)
                                    .style(ButtonStyle::Subtle)
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(ForensicsWorkbenchCommand::CancelEntropy);
                                    })),
                                )
                            }),
                    ),
            )
            .child(
                v_flex()
                    .id("omega.forensics.entropy.campaign")
                    .debug_selector(|| "omega.forensics.entropy.campaign".into())
                    .gap_2()
                    .p_4()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(cx.theme().colors().border_variant)
                    .bg(cx.theme().colors().surface_background)
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                Label::new(format!(
                                    "{}-target entropy campaign",
                                    entropy_catalog.projects.len()
                                ))
                                .size(LabelSize::Small),
                            )
                            .child(
                                Label::new(
                                    entropy_campaign
                                        .as_ref()
                                        .map_or("Not started", |campaign| {
                                            entropy_campaign_phase_label(campaign.phase)
                                        }),
                                )
                                .size(LabelSize::XSmall)
                                .color(entropy_campaign.as_ref().map_or(Color::Muted, |campaign| {
                                    entropy_campaign_phase_color(campaign.phase)
                                })),
                            ),
                    )
                    .child(
                        Label::new("One frozen prompt and source policy across pinned repositories")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(Self::render_fact(
                        "Missing source",
                        "Recorded as a limitation, never a clean result",
                    ))
                    .child(Self::render_fact(
                        "Catalog",
                        format!(
                            "{} · {} products",
                            entropy_catalog.catalog_ref,
                            entropy_catalog.projects.len()
                        ),
                    ))
                    .child(Self::render_fact(
                        "Catalog digest",
                        entropy_catalog.canonical_digest.clone(),
                    ))
                    .when_some(entropy_campaign.clone(), |this, campaign| {
                        this.child(Self::render_fact(
                            "Frozen prompt digest",
                            campaign.binding.prompt_digest,
                        ))
                        .child(Self::render_fact("Model route", campaign.binding.model_route_ref))
                        .child(Self::render_fact(
                            "File selection",
                            campaign.binding.file_selection_policy_ref,
                        ))
                    })
                    .when_some(entropy_campaign.clone(), |this, campaign| {
                        this.child(
                            h_flex()
                                .flex_wrap()
                                .gap_1()
                                .when(campaign.phase == EntropyCampaignPhase::Running, |this| {
                                    this.child(
                                        Button::new(
                                            "omega.forensics.entropy.campaign.pause",
                                            "Pause after this repo",
                                        )
                                        .size(ButtonSize::Compact)
                                        .style(ButtonStyle::Subtle)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Err(error) = this.pause_entropy_campaign(cx) {
                                                this.set_entropy_error(
                                                    format!("Campaign pause failed · {error}"),
                                                    cx,
                                                );
                                            }
                                        })),
                                    )
                                })
                                .when(campaign.phase == EntropyCampaignPhase::Paused, |this| {
                                    this.child(
                                        Button::new(
                                            "omega.forensics.entropy.campaign.resume",
                                            "Resume campaign",
                                        )
                                        .size(ButtonSize::Compact)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Err(error) = this.resume_entropy_campaign(cx) {
                                                this.set_entropy_error(
                                                    format!("Campaign resume failed · {error}"),
                                                    cx,
                                                );
                                            }
                                        })),
                                    )
                                })
                                .when(
                                    matches!(
                                        campaign.phase,
                                        EntropyCampaignPhase::Ready
                                            | EntropyCampaignPhase::Running
                                            | EntropyCampaignPhase::Paused
                                    ),
                                    |this| {
                                        this.child(
                                            Button::new(
                                                "omega.forensics.entropy.campaign.cancel",
                                                "Cancel campaign",
                                            )
                                            .size(ButtonSize::Compact)
                                            .style(ButtonStyle::Subtle)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if let Err(error) =
                                                    this.cancel_entropy_campaign(cx)
                                                {
                                                    this.set_entropy_error(
                                                        format!(
                                                            "Campaign cancellation failed · {error}"
                                                        ),
                                                        cx,
                                                    );
                                                }
                                            })),
                                        )
                                    },
                                ),
                        )
                    })
                    .child(
                        v_flex()
                            .id("omega.forensics.entropy.campaign.projects")
                            .max_h(px(360.))
                            .overflow_y_scroll()
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .children(entropy_catalog.projects.iter().enumerate().map(
                                |(index, product)| {
                                    let product_ref = product.product_ref.clone();
                                    let selected = selected_entropy_project.as_deref()
                                        == Some(product_ref.as_str());
                                    let campaign_row = entropy_campaign
                                        .as_ref()
                                        .and_then(|campaign| campaign.project(&product_ref));
                                    let status = campaign_row.map_or_else(
                                        || product.source_availability.label(),
                                        |row| row.phase.label(),
                                    );
                                    let progress = campaign_row.map_or_else(
                                        || "Not run".to_string(),
                                        |row| {
                                            format!(
                                                "{} files · {} candidates",
                                                row.files_analyzed(),
                                                row.candidate_count()
                                            )
                                        },
                                    );
                                    h_flex()
                                        .id(("omega-entropy-project", index))
                                        .debug_selector({
                                            let product_ref = product_ref.clone();
                                            move || format!(
                                                "omega.forensics.entropy.project.{product_ref}"
                                            )
                                        })
                                        .w_full()
                                        .px_2()
                                        .py_1()
                                        .gap_2()
                                        .cursor_pointer()
                                        .when(selected, |row| {
                                            row.bg(cx.theme().colors().element_selected)
                                        })
                                        .hover(|style| {
                                            style.bg(cx.theme().colors().ghost_element_hover)
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.select_entropy_project(product_ref.clone(), cx)
                                        }))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .flex_1()
                                                .truncate()
                                                .child(product.product_name.clone()),
                                        )
                                        .child(
                                            Label::new(progress)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        )
                                        .child(
                                            Label::new(status)
                                                .size(LabelSize::XSmall)
                                                .color(campaign_row.map_or(Color::Muted, |row| {
                                                    entropy_campaign_project_color(row.phase)
                                                })),
                                        )
                                },
                            )),
                    )
                    .when_some(
                        selected_entropy_project.as_deref().and_then(|product_ref| {
                            entropy_catalog
                                .projects
                                .iter()
                                .find(|product| product.product_ref == product_ref)
                                .cloned()
                        }),
                        |this, product| {
                            let campaign_row = entropy_campaign
                                .as_ref()
                                .and_then(|campaign| campaign.project(&product.product_ref));
                            let comparison_row = entropy_campaign_comparison
                                .as_ref()
                                .and_then(|comparison| {
                                    comparison.projects.iter().find(|row| {
                                        row.product_ref == product.product_ref
                                    })
                                });
                            this.child(
                                v_flex()
                                    .gap_1()
                                    .p_2()
                                    .border_1()
                                    .border_color(cx.theme().colors().border)
                                    .child(
                                        Label::new(product.product_name.clone())
                                            .size(LabelSize::Small),
                                    )
                                    .child(Self::render_fact(
                                        "Source",
                                        product.source_availability.label(),
                                    ))
                                    .child(Self::render_fact(
                                        "Repository",
                                        product
                                            .repository_url
                                            .clone()
                                            .unwrap_or_else(|| "Unavailable".into()),
                                    ))
                                    .child(Self::render_fact(
                                        "Revision",
                                        product
                                            .pinned_revision
                                            .clone()
                                            .unwrap_or_else(|| "Unavailable".into()),
                                    ))
                                    .child(Self::render_fact(
                                        "License / access",
                                        product.license_or_access_status.clone(),
                                    ))
                                    .child(Self::render_fact(
                                        "Dependencies",
                                        product.dependency_policy_ref.clone(),
                                    ))
                                    .child(Self::render_fact(
                                        "Analysis profile",
                                        product.analysis_profile_ref(),
                                    ))
                                    .child(Self::render_fact(
                                        "Limitations",
                                        campaign_row.map_or_else(
                                            || {
                                                if product.limitation_refs.is_empty() {
                                                    "None declared".into()
                                                } else {
                                                    product.limitation_refs.join(" · ")
                                                }
                                            },
                                            |row| {
                                                if row.limitation_refs.is_empty() {
                                                    "None declared".into()
                                                } else {
                                                    row.limitation_refs.join(" · ")
                                                }
                                            },
                                        ),
                                    ))
                                    .when_some(campaign_row, |this, row| {
                                        this.child(Self::render_fact(
                                            "Progress",
                                            format!(
                                                "{} files · {} candidates · {}",
                                                row.files_analyzed(),
                                                row.candidate_count(),
                                                row.phase.label()
                                            ),
                                        ))
                                        .child(Self::render_fact(
                                            "Elapsed",
                                            row.elapsed_milliseconds.map_or_else(
                                                || "Unavailable".into(),
                                                |elapsed| format!("{elapsed} ms"),
                                            ),
                                        ))
                                        .child(Self::render_fact(
                                            "Usage",
                                            row.usage.total_tokens.map_or_else(
                                                || "Unavailable · not inferred".into(),
                                                |tokens| format!("{tokens} exact tokens"),
                                            ),
                                        ))
                                    })
                                    .when_some(comparison_row, |this, row| {
                                        this.child(Self::render_fact(
                                            "Prompt A → B",
                                            format!(
                                                "{} gained · {} lost · {} changed · {} unchanged",
                                                row.gained, row.lost, row.changed, row.unchanged
                                            ),
                                        ))
                                        .child(Self::render_fact(
                                            "Exact run identities",
                                            format!(
                                                "{} → {}",
                                                row.run_a_ref
                                                    .clone()
                                                    .unwrap_or_else(|| "Unavailable".into()),
                                                row.run_b_ref
                                                    .clone()
                                                    .unwrap_or_else(|| "Unavailable".into())
                                            ),
                                        ))
                                    }),
                            )
                        },
                    ),
            )
            .when_some(entropy_source_inspection, |this, inspection| {
                let qualified_miss = inspection.qualified_miss_eligible();
                this.child(div().h_px().bg(cx.theme().colors().border)).child(
                    v_flex()
                        .gap_1()
                        .child(Label::new("Mechanical source inspection").size(LabelSize::Small))
                        .child(Self::render_fact(
                            "State",
                            match inspection.state {
                                EntropySourceInspectionState::Pending => "Pending",
                                EntropySourceInspectionState::Complete => "Complete",
                                EntropySourceInspectionState::Incomplete => "Incomplete",
                                EntropySourceInspectionState::Denied => "Denied",
                                EntropySourceInspectionState::Stale => "Changed · stale",
                            },
                        ))
                        .child(Self::render_fact("Generation", inspection.generation.to_string()))
                        .child(Self::render_fact(
                            "Expected commit",
                            inspection.repository.revision,
                        ))
                        .child(Self::render_fact(
                            "Observed commit",
                            inspection.observed_revision,
                        ))
                        .child(Self::render_fact("Top-level tree", inspection.top_level_tree))
                        .child(Self::render_fact("Manifest", inspection.manifest_ref))
                        .child(Self::render_fact("Manifest digest", inspection.manifest_digest))
                        .child(Self::render_fact(
                            "Path coverage",
                            format!(
                                "{} focal · {} contextual · {} reached · {} not reached",
                                inspection.focal_paths.len(),
                                inspection.contextual_paths.len(),
                                inspection.reached_paths.len(),
                                inspection.not_reached_paths.len()
                            ),
                        ))
                        .child(Self::render_fact(
                            "Dependency coverage",
                            format!("{} declared recursive paths", inspection.dependency_paths.len()),
                        ))
                        .when(!inspection.dependency_facts.is_empty(), |this| {
                            this.child(
                                v_flex()
                                    .gap_1()
                                    .children(inspection.dependency_facts.into_iter().enumerate().map(
                                        |(index, dependency)| {
                                            Self::render_fact(
                                                format!("Dependency {}", index + 1),
                                                format!(
                                                    "{} · {:?} · expected {} · observed {}{}",
                                                    dependency.path,
                                                    dependency.availability,
                                                    dependency.expected_revision.as_deref().unwrap_or("unavailable"),
                                                    dependency.observed_revision.as_deref().unwrap_or("unavailable"),
                                                    dependency.materialization_error.as_ref().map_or_else(
                                                        String::new,
                                                        |error| format!(" · {error}"),
                                                    ),
                                                ),
                                            )
                                        },
                                    )),
                            )
                        })
                        .child(Self::render_fact(
                            "Generated inputs",
                            format!(
                                "{} required · {} missing",
                                inspection.required_generated_input_paths.len(),
                                inspection.missing_generated_input_paths.len()
                            ),
                        ))
                        .child(Self::render_fact(
                            "Excluded source",
                            format!(
                                "{} excluded · {} required exclusions · {} oversized · {} dirty bytes excluded",
                                inspection.excluded_paths.len(),
                                inspection.required_excluded_paths.len(),
                                inspection.oversized_paths.len(),
                                inspection.dirty_excluded_paths.len()
                            ),
                        ))
                        .child(Self::render_fact(
                            "Qualified miss",
                            if qualified_miss { "Eligible" } else { "Blocked" },
                        ))
                        .when(!inspection.reason_refs.is_empty(), |this| {
                            this.child(Self::render_fact(
                                "Incomplete reasons",
                                inspection.reason_refs.join(" · "),
                            ))
                        }),
                )
            })
            .when_some(entropy_run_workbench, |this, run| {
                let counts = run.counts();
                let completed = counts.analyzed
                    + counts.candidate
                    + counts.skipped
                    + counts.failed
                    + counts.timed_out
                    + counts.refused
                    + counts.cancelled;
                let summary = run.summary.clone();
                let elapsed = entropy_elapsed_label(&run);
                let dependencies = run.manifest.dependencies.clone();
                let incomplete_dependencies = dependencies
                    .iter()
                    .filter(|dependency| {
                        dependency.availability != EntropyDependencyAvailability::Available
                    })
                    .count();
                let visible_files = run
                    .files
                    .iter()
                    .filter(|file| entropy_filter.includes(file.state))
                    .take(MAX_VISIBLE_ENTROPY_FILES)
                    .cloned()
                    .collect::<Vec<_>>();
                let selected = selected_entropy_file
                    .as_deref()
                    .and_then(|path| run.files.iter().find(|file| file.path == path))
                    .or_else(|| run.files.iter().find(|file| file.state == EntropyFileState::Candidate))
                    .or_else(|| run.files.first())
                    .cloned();
                let has_selected = selected.is_some();

                this.child(div().h_px().bg(cx.theme().colors().border))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(Label::new("Live entropy traversal").size(LabelSize::Small))
                            .child(Self::render_fact("Run state", entropy_run_phase_label(run.phase)))
                            .child(Self::render_fact(
                                "Summary",
                                format!(
                                    "{completed}/{} files · {} candidates · {} limitations",
                                    run.files.len(),
                                    counts.candidate,
                                    run.limitations.len()
                                ),
                            ))
                            .child(Self::render_fact(
                                "Canonical outcome",
                                format!(
                                    "{:?} · summary {}",
                                    summary.outcome, summary.canonical_digest
                                ),
                            ))
                            .child(Self::render_fact(
                                "Evidence-ranked schedule",
                                format!(
                                    "{} · {} scanner · {} tranches of up to {} · rank is triage evidence, not vulnerability truth",
                                    run.ranked_schedule.ranking_version,
                                    run.ranked_schedule.scanner_version,
                                    run.ranked_schedule.units.iter().map(|unit| unit.tranche).max().unwrap_or(0),
                                    run.ranked_schedule.tranche_size,
                                ),
                            ))
                            .child(Self::render_fact(
                                "Session denominator",
                                format!(
                                    "{} eligible · {} attempted · {} settled · {} failed · {} timed out · {} refused · {} cancelled",
                                    summary.source.eligible_focal_units,
                                    summary.sessions.attempted,
                                    summary.sessions.settled,
                                    summary.sessions.failed,
                                    summary.sessions.timed_out,
                                    summary.sessions.refused,
                                    summary.sessions.cancelled,
                                ),
                            ))
                            .child(Self::render_fact(
                                "Tool denominator",
                                format!(
                                    "{} requested · {} available · {} unavailable · {} denied · {} timed out · {} failed",
                                    summary.tools.requested,
                                    summary.tools.available,
                                    summary.tools.unavailable,
                                    summary.tools.denied,
                                    summary.tools.timed_out,
                                    summary.tools.failed,
                                ),
                            ))
                            .child(Self::render_fact(
                                "Output accounting",
                                format!(
                                    "{} findings · {} hypotheses · {} duplicates · {} limitations · {} malformed rejected",
                                    summary.outputs.findings,
                                    summary.outputs.hypotheses,
                                    summary.outputs.duplicates,
                                    summary.outputs.limitations,
                                    summary.outputs.rejected_malformed,
                                ),
                            ))
                            .child(Self::render_fact(
                                "Source accounting",
                                format!(
                                    "{} focal used · {} contextual · {} reached · {} excluded · {} oversized · {} never reached · {}/{} dependency trees reached",
                                    summary.source.focal_used,
                                    summary.source.contextual_read,
                                    summary.source.reached,
                                    summary.source.excluded,
                                    summary.source.oversized,
                                    summary.source.never_reached,
                                    summary.source.dependency_trees_reached,
                                    summary.source.dependency_trees_total,
                                ),
                            ))
                            .child(Self::render_fact(
                                "Cleanup",
                                format!(
                                    "{:?} · {}",
                                    summary.cleanup.state,
                                    summary
                                        .cleanup
                                        .receipt_ref
                                        .as_deref()
                                        .or(summary.cleanup.reason_ref.as_deref())
                                        .unwrap_or("receipt unavailable")
                                ),
                            ))
                            .child(Self::render_fact("Elapsed", elapsed))
                            .child(Self::render_fact(
                                "Recursive source",
                                if dependencies.is_empty() {
                                    "No submodules declared".to_string()
                                } else if incomplete_dependencies == 0 {
                                    format!("Complete · {} pinned dependencies", dependencies.len())
                                } else {
                                    format!(
                                        "Incomplete · {incomplete_dependencies}/{} dependencies unavailable or mismatched",
                                        dependencies.len()
                                    )
                                },
                            ))
                            .when(!dependencies.is_empty(), |this| {
                                this.child(
                                    v_flex()
                                        .id("omega.forensics.entropy.dependencies")
                                        .gap_1()
                                        .p_2()
                                        .border_1()
                                        .border_color(cx.theme().colors().border)
                                        .child(
                                            Label::new("Pinned dependency inventory")
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        )
                                        .children(dependencies.into_iter().enumerate().map(
                                            |(index, dependency)| {
                                                let available = dependency.availability
                                                    == EntropyDependencyAvailability::Available;
                                                v_flex()
                                                    .id(("omega-entropy-dependency", index))
                                                    .gap_1()
                                                    .child(
                                                        Label::new(dependency.path)
                                                            .size(LabelSize::XSmall),
                                                    )
                                                    .child(Self::render_fact(
                                                        "Status",
                                                        entropy_dependency_availability_label(
                                                            dependency.availability,
                                                        ),
                                                    ))
                                                    .child(Self::render_fact(
                                                        "Expected",
                                                        dependency.expected_revision.unwrap_or_else(
                                                            || "Unavailable".into(),
                                                        ),
                                                    ))
                                                    .child(Self::render_fact(
                                                        "Observed",
                                                        dependency.observed_revision.unwrap_or_else(
                                                            || "Unavailable".into(),
                                                        ),
                                                    ))
                                                    .when_some(
                                                        dependency.materialization_error,
                                                        |this, error| {
                                                            this.child(
                                                                Label::new(error)
                                                                    .size(LabelSize::XSmall)
                                                                    .color(Color::Warning),
                                                            )
                                                        },
                                                    )
                                                    .when(!available, |this| {
                                                        this.border_l_2().border_color(
                                                            cx.theme().status().warning_border,
                                                        ).pl_2()
                                                    })
                                            },
                                        )),
                                )
                            })
                            .child(Self::render_fact(
                                "Usage truth",
                                format!(
                                    "time {} · tokens {} · cost {} · network {}",
                                    entropy_usage_label(&summary.elapsed_milliseconds),
                                    entropy_usage_label(&summary.total_tokens),
                                    entropy_usage_label(&summary.cost_micros),
                                    entropy_usage_label(&summary.network_bytes),
                                ),
                            ))
                            .when_some(entropy_comparison, |this, comparison| {
                                this.child(Self::render_fact(
                                    "Prompt A → B",
                                    format!(
                                        "{} gained · {} lost · {} changed · {} unchanged",
                                        comparison.gained,
                                        comparison.lost,
                                        comparison.changed,
                                        comparison.unchanged
                                    ),
                                ))
                            })
                            .child(
                                h_flex().flex_wrap().gap_1().children(
                                    EntropyFileFilter::ALL.into_iter().enumerate().map(|(index, filter)| {
                                        Button::new(("omega.forensics.entropy.filter", index), filter.label())
                                            .size(ButtonSize::Compact)
                                            .style(if filter == entropy_filter {
                                                ButtonStyle::Tinted(ui::TintColor::Accent)
                                            } else {
                                                ButtonStyle::Subtle
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.select_entropy_filter(filter, cx)
                                            }))
                                    }),
                                ),
                            )
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_2()
                                    .child(
                                        v_flex()
                                            .id("omega.forensics.entropy.files")
                                            .debug_selector(|| "omega.forensics.entropy.files".into())
                                            .w_1_2()
                                            .max_h(px(420.))
                                            .overflow_y_scroll()
                                            .border_1()
                                            .border_color(cx.theme().colors().border)
                                            .when(visible_files.is_empty(), |this| {
                                                this.child(
                                                    div().p_2().child(
                                                        Label::new("No files match this filter")
                                                            .size(LabelSize::XSmall)
                                                            .color(Color::Muted),
                                                    ),
                                                )
                                            })
                                            .children(visible_files.into_iter().enumerate().map(|(index, file)| {
                                                let path = file.path.clone();
                                                let is_selected = selected_entropy_file.as_deref() == Some(path.as_str());
                                                h_flex()
                                                    .id(("omega-entropy-file", index))
                                                    .debug_selector({
                                                        let path = path.clone();
                                                        move || format!("omega.forensics.entropy.file.{path}")
                                                    })
                                                    .w_full()
                                                    .px_2()
                                                    .py_1()
                                                    .gap_2()
                                                    .cursor_pointer()
                                                    .when(is_selected, |row| row.bg(cx.theme().colors().element_selected))
                                                    .hover(|style| style.bg(cx.theme().colors().ghost_element_hover))
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.select_entropy_file(path.clone(), cx)
                                                    }))
                                                    .child(
                                                        Label::new(entropy_file_state_label(file.state))
                                                            .size(LabelSize::XSmall)
                                                            .color(entropy_file_state_color(file.state)),
                                                    )
                                                    .child(div().min_w_0().flex_1().truncate().child(file.path))
                                            })),
                                    )
                                    .child(
                                        v_flex()
                                            .id("omega.forensics.entropy.detail")
                                            .debug_selector(|| "omega.forensics.entropy.detail".into())
                                            .w_1_2()
                                            .min_h_32()
                                            .max_h(px(420.))
                                            .overflow_y_scroll()
                                            .border_1()
                                            .border_color(cx.theme().colors().border)
                                            .p_2()
                                            .gap_1()
                                            .when_some(selected, |this, file| {
                                                this.child(Label::new(file.path.clone()).size(LabelSize::Small))
                                                    .child(Self::render_fact("State", entropy_file_state_label(file.state)))
                                                    .children(file.observations.into_iter().enumerate().map(|(index, observation)| {
                                                        let citations = observation.source_refs.clone();
                                                        v_flex()
                                                            .id(("omega-entropy-observation", index))
                                                            .gap_1()
                                                            .child(Label::new(observation.title).size(LabelSize::XSmall))
                                                            .child(Self::render_fact("Mechanism", observation.suspected_mechanism))
                                                            .child(Self::render_fact("Confidence boundary", observation.confidence_boundary))
                                                            .children(citations.into_iter().enumerate().map(|(source_index, citation)| {
                                                                let label = format!("{}:{} · {}", citation.path, citation.start_line, citation.symbol.clone().unwrap_or_else(|| "source".into()));
                                                                Button::new(("omega-entropy-observation-source", index * 100 + source_index), label)
                                                                    .size(ButtonSize::Compact)
                                                                    .style(ButtonStyle::Subtle)
                                                                    .on_click(cx.listener(move |this, _, _, cx| this.open_source(citation.clone(), cx)))
                                                            }))
                                                    }))
                                                    .children(file.hypotheses.into_iter().enumerate().map(|(index, hypothesis)| {
                                                        let citations = hypothesis
                                                            .causal_links
                                                            .iter()
                                                            .flat_map(|link| link.source_refs.iter().cloned())
                                                            .collect::<Vec<_>>();
                                                        v_flex()
                                                            .id(("omega-entropy-hypothesis", index))
                                                            .gap_1()
                                                            .child(Label::new(hypothesis.title).size(LabelSize::XSmall))
                                                            .child(Self::render_fact("Mechanism", hypothesis.suspected_mechanism))
                                                            .child(Self::render_fact("Missing evidence", hypothesis.missing_evidence.join(" · ")))
                                                            .child(Self::render_fact("Next check", hypothesis.next_check))
                                                            .child(Self::render_fact("Confidence boundary", hypothesis.confidence_boundary))
                                                            .children(citations.into_iter().enumerate().map(|(source_index, citation)| {
                                                                let label = format!("{}:{} · {}", citation.path, citation.start_line, citation.symbol.clone().unwrap_or_else(|| "source".into()));
                                                                Button::new(("omega-entropy-hypothesis-source", index * 100 + source_index), label)
                                                                    .size(ButtonSize::Compact)
                                                                    .style(ButtonStyle::Subtle)
                                                                    .on_click(cx.listener(move |this, _, _, cx| this.open_source(citation.clone(), cx)))
                                                            }))
                                                    }))
                                                    .children(file.limitations.into_iter().map(|limitation| {
                                                        Label::new(format!("Limitation · {}", limitation.message))
                                                            .size(LabelSize::XSmall)
                                                            .color(Color::Warning)
                                                    }))
                                            })
                                            .when(!has_selected, |this| {
                                                this.child(Label::new("Select a file to inspect its result").size(LabelSize::XSmall).color(Color::Muted))
                                            }),
                                    ),
                            ),
                    )
            })
            .child(div().h_px().bg(cx.theme().colors().border))
            .child(
                v_flex()
                    .gap_1()
                    .child(Self::render_fact("Repository", repository_name))
                    .child(Self::render_fact("Remote", clone_url))
                    .child(Self::render_fact("Commit", commit))
                    .child(Self::render_fact("Source", source_state))
                    .child(Self::render_fact("Dependencies", dependency_policy))
                    .when_some(target, |this, target| {
                        this.child(Self::render_fact(
                            "Scan profile",
                            target.scan_profile_ref.clone(),
                        ))
                    }),
            )
            .child(div().h_px().bg(cx.theme().colors().border))
            .child(
                Label::new("Coldcard target")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(
                h_flex().flex_wrap().gap_1().children(
                    ColdcardBenchmarkArm::ALL
                        .into_iter()
                        .enumerate()
                        .map(|(index, arm)| {
                            Button::new(("omega.forensics.benchmark", index), arm.label())
                                .size(ButtonSize::Compact)
                                .style(if arm == self.selected_arm {
                                    ButtonStyle::Tinted(ui::TintColor::Accent)
                                } else {
                                    ButtonStyle::Subtle
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_benchmark_arm(arm, cx)
                                }))
                        }),
                ),
            )
            .child(div().h_px().bg(cx.theme().colors().border))
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        Label::new("Prompt artifacts")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(Self::render_fact(
                        "Active",
                        active_prompt.prompt_artifact_ref.clone(),
                    ))
                    .child(Self::render_fact(
                        "Digest",
                        active_prompt.canonical_digest.clone(),
                    ))
                    .child(Self::render_fact(
                        "Typed output",
                        "Finding + hypothesis schemas",
                    ))
                    .child(Self::render_fact("Authority", "External admitted profile"))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("omega.forensics.prompt.clone", "Clone active")
                                    .size(ButtonSize::Compact)
                                    .style(ButtonStyle::Subtle)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Err(error) = this.clone_prompt_candidate(cx) {
                                            this.status =
                                                format!("Prompt clone failed · {error}").into();
                                            cx.notify();
                                        }
                                    })),
                            )
                            .when(prompt_workspace.draft().is_some(), |this| {
                                this.child(
                                    Button::new("omega.forensics.prompt.save", "Save candidate")
                                        .size(ButtonSize::Compact)
                                        .style(ButtonStyle::Tinted(ui::TintColor::Accent))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Err(error) = this.save_prompt_candidate(cx) {
                                                this.status =
                                                    format!("Prompt save failed · {error}").into();
                                                cx.notify();
                                            }
                                        })),
                                )
                            }),
                    )
                    .when_some(prompt_workspace.draft(), |this, draft| {
                        this.child(Self::render_fact(
                            "Draft",
                            draft.prompt_artifact_ref.clone(),
                        ))
                        .child(Self::render_fact(
                            "Parent",
                            draft.parent_prompt_artifact_ref.clone().unwrap_or_default(),
                        ))
                        .child(
                            Label::new(if prompt_changes.is_empty() {
                                "No semantic changes".to_string()
                            } else {
                                prompt_changes
                                    .iter()
                                    .map(|change| {
                                        format!(
                                            "{} · {}",
                                            prompt_change_label(change.kind),
                                            change.field
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join("  |  ")
                            })
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                        )
                    })
                    .children(prompt_candidates.into_iter().enumerate().map(
                        |(index, candidate)| {
                            let candidate_ref = candidate.prompt_artifact_ref.clone();
                            Button::new(
                                ("omega.forensics.prompt.activate", index),
                                candidate.prompt_artifact_ref,
                            )
                            .size(ButtonSize::Compact)
                            .style(if candidate_ref == active_prompt.prompt_artifact_ref {
                                ButtonStyle::Tinted(ui::TintColor::Accent)
                            } else {
                                ButtonStyle::Subtle
                            })
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Err(error) =
                                        this.activate_prompt_candidate(&candidate_ref, cx)
                                    {
                                        this.status =
                                            format!("Prompt activation failed · {error}").into();
                                        cx.notify();
                                    }
                                },
                            ))
                        },
                    )),
            )
            .when_some(matrix, |this, matrix| {
                this.child(div().h_px().bg(cx.theme().colors().border))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new("Run matrix")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(Self::render_fact(
                                "Retained runs",
                                matrix.runs.len().to_string(),
                            ))
                            .child(Self::render_fact(
                                "Promotion",
                                if matrix.promoted {
                                    "Admitted"
                                } else {
                                    "Blocked / not requested"
                                },
                            ))
                            .child(Self::render_fact(
                                "Pareto frontier",
                                matrix.pareto_frontier_arm_refs.join(", "),
                            ))
                            .child(Self::render_fact(
                                "Common / divergent findings",
                                format!(
                                    "{} / {} arms",
                                    matrix.finding_divergence.common_finding_refs.len(),
                                    matrix
                                        .finding_divergence
                                        .unique_finding_refs_by_arm
                                        .values()
                                        .filter(|values| !values.is_empty())
                                        .count()
                                ),
                            ))
                            .child(Self::render_fact(
                                "Recall curves",
                                format!(
                                    "{} time points · {} token points",
                                    matrix.recall_time_curve.len(),
                                    matrix.recall_token_curve.len()
                                ),
                            ))
                            .children(matrix.rows.into_iter().map(|row| {
                                let hit_rate = row
                                    .hit_rate_basis_points
                                    .map(|value| format!("{}.{:02}%", value / 100, value % 100))
                                    .unwrap_or_else(|| "Not eligible".into());
                                v_flex()
                                    .gap_1()
                                    .p_2()
                                    .border_1()
                                    .border_color(cx.theme().colors().border)
                                    .child(
                                        Label::new(row.arm_ref)
                                            .size(LabelSize::XSmall)
                                            .color(Color::Accent),
                                    )
                                    .child(Self::render_fact(
                                        "Hit / miss / n",
                                        format!(
                                            "{} / {} / {} · {hit_rate}",
                                            row.hit_count, row.miss_count, row.sample_count
                                        ),
                                    ))
                                    .child(Self::render_fact(
                                        "Identification",
                                        format!(
                                            "p50 {} · tail {}",
                                            statistic_label(&row.p50_identification_milliseconds),
                                            statistic_label(&row.tail_identification_milliseconds)
                                        ),
                                    ))
                                    .child(Self::render_fact(
                                        "Tokens / cost",
                                        format!(
                                            "{} · {}",
                                            aggregate_truth_label(
                                                row.total_tokens,
                                                row.token_exactness,
                                                "tokens"
                                            ),
                                            aggregate_truth_label(
                                                row.total_cost_micros,
                                                row.cost_exactness,
                                                "µUSD"
                                            )
                                        ),
                                    ))
                                    .child(Self::render_fact(
                                        "Evidence / false positives",
                                        format!(
                                            "{} · {}",
                                            row.causal_coverage_basis_points
                                                .map(|value| format!("{}%", value / 100))
                                                .unwrap_or_else(|| "N/A".into()),
                                            row.false_positive_count
                                        ),
                                    ))
                                    .child(Self::render_fact(
                                        "Cleanup / provenance",
                                        format!(
                                            "{}/{} · {} events · {} receipts",
                                            row.cleanup_count,
                                            row.sample_count,
                                            row.event_refs.len(),
                                            row.receipt_refs.len()
                                        ),
                                    ))
                            })),
                    )
            })
            .child(div().h_px().bg(cx.theme().colors().border))
            .child(
                Label::new("Managed worker")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .when_some(self.preflight.as_ref(), |this, preflight| {
                let worker = &preflight.worker;
                this.child(
                    v_flex()
                        .gap_1()
                        .child(Self::render_fact("Supply", "OpenAgents Cloud"))
                        .child(Self::render_fact("Provider", "Google Cloud"))
                        .child(Self::render_fact("Isolation", "GCE VM"))
                        .child(Self::render_fact("Adapter", worker.adapter_ref.clone()))
                        .child(Self::render_fact("Region", worker.region_ref.clone()))
                        .child(Self::render_fact("Custody", worker.custody_ref.clone()))
                        .child(Self::render_fact("Image", worker.image_digest.clone()))
                        .child(Self::render_fact("Profile", worker.profile_digest.clone()))
                        .child(Self::render_fact("Network", "Broker only"))
                        .child(Self::render_fact("Lease", worker.lease_ref.clone()))
                        .child(Self::render_fact(
                            "Lease bound",
                            format!("{} s", worker.lease_seconds),
                        ))
                        .child(Self::render_fact(
                            "Capabilities",
                            worker.capability_refs.len().to_string(),
                        )),
                )
            })
            .when(self.preflight.is_none(), |this| {
                this.child(
                    Label::new("Awaiting an admitted OpenAgents managed GCE profile")
                        .size(LabelSize::Small)
                        .color(Color::Warning),
                )
            })
            .when_some(self.preflight.as_ref(), |this, preflight| {
                let budget = &preflight.budget;
                this.child(div().h_px().bg(cx.theme().colors().border))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new("Run bounds")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(Self::render_fact("Model", budget.model_ref.clone()))
                            .child(Self::render_fact("Effort", budget.effort_ref.clone()))
                            .child(Self::render_fact(
                                "Concurrency",
                                budget.max_concurrency.to_string(),
                            ))
                            .child(Self::render_fact(
                                "Time",
                                format!("{} s", budget.max_time_seconds),
                            ))
                            .child(Self::render_fact("Tokens", budget.max_tokens.to_string()))
                            .child(Self::render_fact(
                                "Cost",
                                format!("{} µUSD", budget.max_cost_micros),
                            ))
                            .child(Self::render_fact(
                                "Artifacts",
                                format!("{} B", budget.max_artifact_bytes),
                            ))
                            .child(Self::render_fact(
                                "Network",
                                format!("{} B", budget.max_network_bytes),
                            )),
                    )
            })
            .when_some(coverage, |this, coverage| {
                this.child(div().h_px().bg(cx.theme().colors().border))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new("Coverage")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(Self::render_fact(
                                "State",
                                readiness_label(
                                    readiness.unwrap_or(PreflightReadiness::AwaitingCoverage),
                                ),
                            ))
                            .child(Self::render_fact("Present", coverage.present.to_string()))
                            .child(Self::render_fact("Missing", coverage.missing.to_string()))
                            .child(Self::render_fact("Excluded", coverage.excluded.to_string()))
                            .child(Self::render_fact(
                                "Generated",
                                coverage.generated.to_string(),
                            ))
                            .child(Self::render_fact(
                                "Oversized",
                                coverage.oversized.to_string(),
                            ))
                            .child(Self::render_fact(
                                "Dependency-owned",
                                coverage.dependency_owned.to_string(),
                            )),
                    )
            })
            .when_some(review, |this, review| {
                let outcome = review_outcome_label(review.outcome);
                let cleanup = if review.cleanup_state == "observed_zero_residue" {
                    "Verified · zero residue"
                } else {
                    review.cleanup_state.as_str()
                };
                this.child(div().h_px().bg(cx.theme().colors().border))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new("Run review")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(Self::render_fact("Outcome", outcome))
                            .child(Self::render_fact("Prompt", review.prompt_digest.clone()))
                            .child(Self::render_fact(
                                "Budget",
                                budget_state_label(review.budget_state),
                            ))
                            .child(Self::render_fact(
                                "Coverage",
                                coverage_status_label(review.coverage_status),
                            ))
                            .child(Self::render_fact("Placement", review.placement_ref.clone()))
                            .child(Self::render_fact(
                                "Generation",
                                review.resource_generation.to_string(),
                            ))
                            .child(Self::render_fact("Cleanup", cleanup.to_string())),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new("Lifecycle")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .children(review.lifecycle.iter().map(|stage| {
                                let marker = lifecycle_marker(stage.state);
                                let timestamp =
                                    stage.observed_at.as_deref().unwrap_or("Not observed");
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Label::new(marker)
                                            .size(LabelSize::XSmall)
                                            .color(lifecycle_color(stage.state)),
                                    )
                                    .child(Label::new(stage.label.clone()).size(LabelSize::XSmall))
                                    .child(
                                        Label::new(timestamp.to_string())
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted)
                                            .line_clamp(1),
                                    )
                            })),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new("Identification metrics")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .children(review.metrics.iter().map(|metric| {
                                Self::render_fact(metric.label.clone(), metric.display_value())
                            })),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                Label::new(format!("Findings · {}", review.findings.len()))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .children(review.findings.into_iter().enumerate().map(
                                |(finding_index, finding)| {
                                    let finding_ref = finding.finding_ref.clone();
                                    let accept_ref = finding_ref.clone();
                                    let correct_ref = finding_ref.clone();
                                    let reject_ref = finding_ref.clone();
                                    let verification_ref = finding_ref.clone();
                                    let verification_case = review.verification_cases.iter().find(|case| case.envelope.finding.finding_ref == finding_ref).cloned();
                                    let can_request_verification = fixture_views_enabled && verification_case.is_none() && finding.poc_ref.is_some();
                                    v_flex()
                                        .id(("omega.forensics.finding", finding_index))
                                        .gap_2()
                                        .p_2()
                                        .border_1()
                                        .border_color(cx.theme().colors().border)
                                        .rounded_md()
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .child(
                                                    Label::new(format!(
                                                        "{} · Finding",
                                                        finding.severity
                                                    ))
                                                    .size(LabelSize::XSmall)
                                                    .color(Color::Error),
                                                )
                                                .child(
                                                    Label::new(finding.evidence_tier.label())
                                                        .size(LabelSize::XSmall)
                                                        .color(evidence_tier_color(
                                                            finding.evidence_tier,
                                                        )),
                                                )
                                                .child(
                                                    Label::new(finding.claim_state.clone())
                                                        .size(LabelSize::XSmall)
                                                        .color(Color::Muted),
                                                ),
                                        )
                                        .child(
                                            Label::new(finding.title.clone())
                                                .size(LabelSize::Small),
                                        )
                                        .child(
                                            Label::new(finding.impact.clone())
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted)
                                                .line_clamp(4),
                                        )
                                        .when_some(
                                            finding.duplicate_group_ref.clone(),
                                            |this, group| {
                                                this.child(Self::render_fact(
                                                    "Duplicate group",
                                                    group,
                                                ))
                                            },
                                        )
                                        .child(
                                            v_flex()
                                                .gap_1()
                                                .child(
                                                    Label::new("Causal path")
                                                        .size(LabelSize::XSmall)
                                                        .color(Color::Muted),
                                                )
                                                .children(finding.causal_path.iter().map(|link| {
                                                    Label::new(format!(
                                                        "{} {}. {}",
                                                        if link.supported { "✓" } else { "?" },
                                                        link.sequence,
                                                        link.proposition
                                                    ))
                                                    .size(LabelSize::XSmall)
                                                    .color(if link.supported {
                                                        Color::Success
                                                    } else {
                                                        Color::Warning
                                                    })
                                                })),
                                        )
                                        .child(Self::render_fact(
                                            "Independent verification",
                                            verification_case.as_ref().map_or_else(
                                                || "Not requested · remediation locked".to_string(),
                                                |case| format!("{:?} · {} receipts · remediation {}", case.state, case.evidence.len(), if case.remediation_enabled { "enabled" } else { "locked" }),
                                            ),
                                        ))
                                        .child(
                                            Button::new(("omega.forensics.request-verification", finding_index), "Request independent verification")
                                                .size(ButtonSize::Compact)
                                                .style(ButtonStyle::Subtle)
                                                .disabled(!can_request_verification)
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    if let Err(error) = this.request_fixture_independent_verification(&verification_ref, cx) {
                                                        this.status = error.to_string().into(); cx.notify();
                                                    }
                                                })),
                                        )
                                        .child(
                                            v_flex()
                                                .gap_1()
                                                .child(
                                                    Label::new("Evidence and verification")
                                                        .size(LabelSize::XSmall)
                                                        .color(Color::Muted),
                                                )
                                                .children(finding.evidence_receipts.iter().map(
                                                    |receipt| {
                                                        let verdict = receipt
                                                            .verifier_verdict
                                                            .as_deref()
                                                            .unwrap_or("not verified");
                                                        Label::new(format!(
                                                            "{} · {} · {} · {}",
                                                            receipt.evidence_tier.label(),
                                                            receipt.outcome,
                                                            verdict,
                                                            receipt
                                                                .artifact_ref
                                                                .as_deref()
                                                                .unwrap_or("no artifact")
                                                        ))
                                                        .size(LabelSize::XSmall)
                                                    },
                                                )),
                                        )
                                        .when_some(finding.poc_ref.clone(), |this, poc_ref| {
                                            this.child(Self::render_fact(
                                                "PoC / test diff",
                                                poc_ref,
                                            ))
                                        })
                                        .child(h_flex().flex_wrap().gap_1().children(
                                            finding.source_refs.into_iter().enumerate().map(
                                                |(source_index, citation)| {
                                                    let resolution = source_resolutions
                                                        .get(&citation.source_ref);
                                                    let label = match resolution {
                                                        Some(ForensicSourceResolution::Opening) => {
                                                            format!(
                                                                "Opening {}:{}…",
                                                                citation.path, citation.start_line
                                                            )
                                                        }
                                                        Some(ForensicSourceResolution::Opened) => {
                                                            format!(
                                                                "Opened {}:{}",
                                                                citation.path, citation.start_line
                                                            )
                                                        }
                                                        Some(ForensicSourceResolution::Failed(
                                                            _,
                                                        )) => format!(
                                                            "Resolution failed · {}:{}",
                                                            citation.path, citation.start_line
                                                        ),
                                                        None => format!(
                                                            "Open {}:{}",
                                                            citation.path, citation.start_line
                                                        ),
                                                    };
                                                    Button::new(
                                                        (
                                                            "omega.forensics.source",
                                                            finding_index * 512 + source_index,
                                                        ),
                                                        label,
                                                    )
                                                    .size(ButtonSize::Compact)
                                                    .style(ButtonStyle::Subtle)
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.open_source(citation.clone(), cx)
                                                    }))
                                                },
                                            ),
                                        ))
                                        .child(
                                            h_flex()
                                                .gap_1()
                                                .child(
                                                    Button::new(
                                                        ("omega.forensics.accept", finding_index),
                                                        "Accept",
                                                    )
                                                    .size(ButtonSize::Compact)
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        if let Err(error) = this
                                                            .record_review_decision(
                                                                &accept_ref,
                                                                ForensicReviewDecisionKind::Accept,
                                                                cx,
                                                            )
                                                        {
                                                            this.status = error.to_string().into();
                                                            cx.notify();
                                                        }
                                                    })),
                                                )
                                                .child(
                                                    Button::new(
                                                        ("omega.forensics.correct", finding_index),
                                                        "Correct",
                                                    )
                                                    .size(ButtonSize::Compact)
                                                    .style(ButtonStyle::Subtle)
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        if let Err(error) = this
                                                            .record_review_decision(
                                                                &correct_ref,
                                                                ForensicReviewDecisionKind::Correct,
                                                                cx,
                                                            )
                                                        {
                                                            this.status = error.to_string().into();
                                                            cx.notify();
                                                        }
                                                    })),
                                                )
                                                .child(
                                                    Button::new(
                                                        ("omega.forensics.reject", finding_index),
                                                        "Reject",
                                                    )
                                                    .size(ButtonSize::Compact)
                                                    .style(ButtonStyle::Subtle)
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        if let Err(error) = this
                                                            .record_review_decision(
                                                                &reject_ref,
                                                                ForensicReviewDecisionKind::Reject,
                                                                cx,
                                                            )
                                                        {
                                                            this.status = error.to_string().into();
                                                            cx.notify();
                                                        }
                                                    })),
                                                ),
                                        )
                                        .child(
                                            Label::new(format!(
                                                "{} append-only review decisions",
                                                review
                                                    .decisions
                                                    .iter()
                                                    .filter(|decision| decision.finding_ref
                                                        == finding_ref)
                                                    .count()
                                            ))
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                        )
                                        .children(
                                            review
                                                .decisions
                                                .iter()
                                                .filter(|decision| {
                                                    decision.finding_ref == finding_ref
                                                })
                                                .map(|decision| {
                                                    Label::new(format!(
                                                        "#{:02} · {:?} · {} · {}",
                                                        decision.sequence,
                                                        decision.decision,
                                                        decision.decided_at,
                                                        decision.reason
                                                    ))
                                                    .size(LabelSize::XSmall)
                                                    .color(Color::Muted)
                                                    .line_clamp(2)
                                                }),
                                        )
                                },
                            )),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                Label::new(format!(
                                    "Unverified hypotheses · {}",
                                    review.hypotheses.len()
                                ))
                                .size(LabelSize::XSmall)
                                .color(Color::Warning),
                            )
                            .children(review.hypotheses.into_iter().enumerate().map(
                                |(index, hypothesis)| {
                                    v_flex()
                                        .id(("omega.forensics.hypothesis", index))
                                        .gap_1()
                                        .p_2()
                                        .border_1()
                                        .border_color(cx.theme().colors().border)
                                        .rounded_md()
                                        .child(
                                            Label::new(format!(
                                                "Hypothesis · {}",
                                                hypothesis.state
                                            ))
                                            .size(LabelSize::XSmall)
                                            .color(Color::Warning),
                                        )
                                        .child(
                                            Label::new(hypothesis.suspected_mechanism)
                                                .size(LabelSize::Small),
                                        )
                                        .child(Self::render_fact(
                                            "Missing evidence",
                                            hypothesis.missing_evidence.join(" · "),
                                        ))
                                        .child(Self::render_fact(
                                            "Next check",
                                            hypothesis.next_check,
                                        ))
                                        .child(Self::render_fact(
                                            "If true",
                                            hypothesis.consequence_if_true,
                                        ))
                                },
                            )),
                    )
            })
            .child(div().h_px().bg(cx.theme().colors().border))
            .child(
                Label::new(self.status.clone())
                    .size(LabelSize::XSmall)
                    .color(if can_prepare {
                        Color::Success
                    } else {
                        Color::Muted
                    }),
            )
            .when(needs_acknowledgment, |this| {
                this.child(
                    Button::new(
                        "omega.forensics.acknowledge-incomplete",
                        "Acknowledge incomplete",
                    )
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Subtle)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Err(error) = this.acknowledge_incomplete(cx) {
                            this.status = error.to_string().into();
                            cx.notify();
                        }
                    })),
                )
            })
            .child(
                Button::new("omega.forensics.prepare-run", "Prepare run")
                    .size(ButtonSize::Compact)
                    .disabled(!can_prepare)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Err(error) = this.prepare_run(cx) {
                            this.status = error.to_string().into();
                            cx.notify();
                        }
                    })),
            )
            .when(can_launch, |this| {
                this.child(
                    Button::new("omega.forensics.launch-run", "Launch worker")
                        .size(ButtonSize::Compact)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Err(error) = this.launch_run(cx) {
                                this.status = error.to_string().into();
                                cx.notify();
                            }
                        })),
                )
            })
            .when(can_refresh, |this| {
                this.child(
                    Button::new("omega.forensics.refresh-run", "Refresh events")
                        .size(ButtonSize::Compact)
                        .style(ButtonStyle::Subtle)
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(ForensicsWorkbenchCommand::Refresh);
                        })),
                )
            })
            .when(can_cancel, |this| {
                this.child(
                    Button::new("omega.forensics.cancel-run", "Cancel and clean up")
                        .size(ButtonSize::Compact)
                        .style(ButtonStyle::Subtle)
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(ForensicsWorkbenchCommand::Cancel);
                        })),
                )
            })
            .when(can_cleanup, |this| {
                this.child(
                    Button::new("omega.forensics.cleanup-run", "Delete and verify cleanup")
                        .size(ButtonSize::Compact)
                        .style(ButtonStyle::Subtle)
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(ForensicsWorkbenchCommand::Cleanup);
                        })),
                )
            })
            .into_any_element()
    }
}

fn readiness_label(readiness: PreflightReadiness) -> &'static str {
    match readiness {
        PreflightReadiness::AwaitingCoverage => "Coverage pending",
        PreflightReadiness::Ready => "Ready",
        PreflightReadiness::IncompleteResearch => "Incomplete research",
        PreflightReadiness::Denied => "Denied",
    }
}

fn source_state_label(source_state: SourceState) -> &'static str {
    match source_state {
        SourceState::Clean => "Clean",
        SourceState::Dirty => "Dirty",
        SourceState::ExternallyPrepared => "Externally prepared",
    }
}

fn dependency_policy_label(dependency_policy: DependencyPolicy) -> &'static str {
    match dependency_policy {
        DependencyPolicy::PinnedRecursive => "Pinned recursive",
    }
}

fn entropy_dependency_availability_label(
    availability: EntropyDependencyAvailability,
) -> &'static str {
    match availability {
        EntropyDependencyAvailability::Available => "Available at pinned revision",
        EntropyDependencyAvailability::Missing => "Missing",
        EntropyDependencyAvailability::WrongRevision => "Wrong revision",
        EntropyDependencyAvailability::SourceUnavailable => "Source unavailable",
    }
}

fn run_phase_label(phase: ForensicsRunPhase) -> &'static str {
    match phase {
        ForensicsRunPhase::Prepared => "Run prepared",
        ForensicsRunPhase::Admitting => "Admitting one managed GCE worker",
        ForensicsRunPhase::WorkerReady => "Worker ready",
        ForensicsRunPhase::Running => "Forensic run active",
        ForensicsRunPhase::CancelRequested => "Cancellation requested",
        ForensicsRunPhase::Interrupting => "Interrupt observed; awaiting settlement",
        ForensicsRunPhase::Settled => "Runtime structurally settled",
        ForensicsRunPhase::Deleting => "Deleting worker and verifying cleanup",
        ForensicsRunPhase::Cleaned => "Cleanup verified; zero residue",
        ForensicsRunPhase::Refused => "Run refused",
        ForensicsRunPhase::Failed => "Run failed",
        ForensicsRunPhase::RecoveryRequired => "Recovery required",
    }
}

fn entropy_run_status(run: &EntropyRunProjection) -> String {
    let counts = run.counts();
    match run.phase {
        EntropyRunPhase::Ready | EntropyRunPhase::Running => format!(
            "Entropy analysis · {} queued · {} analyzed · {} candidates · {} failed",
            counts.queued, counts.analyzed, counts.candidate, counts.failed
        ),
        EntropyRunPhase::CancelRequested | EntropyRunPhase::Cancelled => format!(
            "Entropy analysis cancelled · {} files stopped",
            counts.cancelled
        ),
        EntropyRunPhase::Completed => format!(
            "Entropy analysis complete · {} analyzed · {} candidates",
            counts.analyzed, counts.candidate
        ),
        EntropyRunPhase::CompletedWithLimitations => format!(
            "Entropy analysis complete with limitations · {} candidates · {} skipped · {} failed",
            counts.candidate, counts.skipped, counts.failed
        ),
        EntropyRunPhase::AwaitingCleanup => {
            "Entropy outputs persisted · cleanup or recovery remains required".into()
        }
        EntropyRunPhase::Failed => format!(
            "Entropy analysis failed · {}/{} sessions failed",
            run.summary.sessions.failed, run.summary.sessions.attempted
        ),
        EntropyRunPhase::FailedWithPartialOutput => format!(
            "Entropy analysis failed with retained partial output · {} findings · {} hypotheses",
            run.summary.outputs.findings, run.summary.outputs.hypotheses
        ),
    }
}

fn entropy_run_phase_label(phase: EntropyRunPhase) -> &'static str {
    match phase {
        EntropyRunPhase::Ready => "Ready",
        EntropyRunPhase::Running => "Running",
        EntropyRunPhase::CancelRequested => "Cancellation requested",
        EntropyRunPhase::AwaitingCleanup => "Awaiting cleanup · recoverable",
        EntropyRunPhase::Completed => "Completed",
        EntropyRunPhase::CompletedWithLimitations => "Completed with limitations",
        EntropyRunPhase::Failed => "Failed",
        EntropyRunPhase::FailedWithPartialOutput => "Failed with partial output",
        EntropyRunPhase::Cancelled => "Cancelled",
    }
}

fn entropy_usage_label(value: &omega_forensics::EntropyUsageValue) -> String {
    match value.value {
        Some(amount) => format!("{amount} ({:?})", value.exactness),
        None => "Unavailable".into(),
    }
}

fn entropy_campaign_phase_label(phase: EntropyCampaignPhase) -> &'static str {
    match phase {
        EntropyCampaignPhase::Ready => "Ready",
        EntropyCampaignPhase::Running => "Running",
        EntropyCampaignPhase::Paused => "Paused",
        EntropyCampaignPhase::Completed => "Completed",
        EntropyCampaignPhase::CompletedWithLimitations => "Completed with limitations",
        EntropyCampaignPhase::Cancelled => "Cancelled",
    }
}

fn entropy_campaign_phase_color(phase: EntropyCampaignPhase) -> Color {
    match phase {
        EntropyCampaignPhase::Running => Color::Accent,
        EntropyCampaignPhase::Paused
        | EntropyCampaignPhase::CompletedWithLimitations
        | EntropyCampaignPhase::Cancelled => Color::Warning,
        EntropyCampaignPhase::Ready | EntropyCampaignPhase::Completed => Color::Muted,
    }
}

fn entropy_campaign_project_color(phase: omega_forensics::EntropyCampaignProjectPhase) -> Color {
    use omega_forensics::EntropyCampaignProjectPhase;
    match phase {
        EntropyCampaignProjectPhase::Running => Color::Accent,
        EntropyCampaignProjectPhase::ProviderFailed | EntropyCampaignProjectPhase::SourceFailed => {
            Color::Error
        }
        EntropyCampaignProjectPhase::CompletedWithLimitations
        | EntropyCampaignProjectPhase::SourceUnavailable
        | EntropyCampaignProjectPhase::InputIncomplete
        | EntropyCampaignProjectPhase::Cancelled => Color::Warning,
        EntropyCampaignProjectPhase::Queued | EntropyCampaignProjectPhase::Completed => {
            Color::Muted
        }
    }
}

fn entropy_file_state_label(state: EntropyFileState) -> &'static str {
    match state {
        EntropyFileState::Queued => "Queued",
        EntropyFileState::Reading => "Reading",
        EntropyFileState::Analyzed => "Analyzed",
        EntropyFileState::Candidate => "Candidate",
        EntropyFileState::Skipped => "Skipped",
        EntropyFileState::Failed => "Failed",
        EntropyFileState::TimedOut => "Timed out",
        EntropyFileState::Refused => "Refused",
        EntropyFileState::Cancelled => "Cancelled",
    }
}

fn entropy_file_state_color(state: EntropyFileState) -> Color {
    match state {
        EntropyFileState::Candidate => Color::Accent,
        EntropyFileState::Failed | EntropyFileState::Refused => Color::Error,
        EntropyFileState::Skipped | EntropyFileState::TimedOut | EntropyFileState::Cancelled => {
            Color::Warning
        }
        EntropyFileState::Reading => Color::Success,
        EntropyFileState::Queued | EntropyFileState::Analyzed => Color::Muted,
    }
}

fn entropy_elapsed_label(run: &EntropyRunProjection) -> String {
    let started = chrono::DateTime::parse_from_rfc3339(&run.binding.started_at).ok();
    let ended = run
        .events
        .last()
        .and_then(|event| chrono::DateTime::parse_from_rfc3339(&event.observed_at).ok());
    match started.zip(ended) {
        Some((started, ended)) => {
            let seconds = (ended - started).num_seconds().max(0);
            format!("{seconds}s")
        }
        None => "Pending first file event".into(),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct EntropyRunComparison {
    gained: usize,
    lost: usize,
    changed: usize,
    unchanged: usize,
}

fn compare_entropy_runs(
    prior: &EntropyRunProjection,
    current: &EntropyRunProjection,
) -> EntropyRunComparison {
    use std::collections::{BTreeMap, BTreeSet};

    fn candidates(run: &EntropyRunProjection) -> BTreeMap<&str, String> {
        run.files
            .iter()
            .filter(|file| file.state == EntropyFileState::Candidate)
            .map(|file| {
                let signature = format!("{:?}|{:?}", file.observations, file.hypotheses);
                (file.path.as_str(), signature)
            })
            .collect()
    }

    let prior = candidates(prior);
    let current = candidates(current);
    let paths = prior
        .keys()
        .chain(current.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut comparison = EntropyRunComparison::default();
    for path in paths {
        match (prior.get(path), current.get(path)) {
            (None, Some(_)) => comparison.gained += 1,
            (Some(_), None) => comparison.lost += 1,
            (Some(left), Some(right)) if left == right => comparison.unchanged += 1,
            (Some(_), Some(_)) => comparison.changed += 1,
            (None, None) => {}
        }
    }
    comparison
}

fn evidence_tier_color(tier: ForensicEvidenceTier) -> Color {
    match tier {
        ForensicEvidenceTier::Hypothesis => Color::Warning,
        ForensicEvidenceTier::SourceObserved => Color::Muted,
        ForensicEvidenceTier::ArtifactObserved => Color::Accent,
        ForensicEvidenceTier::Executed | ForensicEvidenceTier::IndependentlyVerified => {
            Color::Success
        }
    }
}

fn coldcard_rung_state_label(state: omega_forensics::ColdcardRungState) -> &'static str {
    match state {
        omega_forensics::ColdcardRungState::Missing => "Missing",
        omega_forensics::ColdcardRungState::Provisional => "Provisional",
        omega_forensics::ColdcardRungState::Qualified => "Qualified",
        omega_forensics::ColdcardRungState::IndependentlyVerified => "Independently verified",
    }
}

fn coldcard_rung_state_color(state: omega_forensics::ColdcardRungState) -> Color {
    match state {
        omega_forensics::ColdcardRungState::Missing => Color::Muted,
        omega_forensics::ColdcardRungState::Provisional => Color::Warning,
        omega_forensics::ColdcardRungState::Qualified => Color::Accent,
        omega_forensics::ColdcardRungState::IndependentlyVerified => Color::Success,
    }
}

fn lifecycle_marker(state: ForensicLifecycleState) -> &'static str {
    match state {
        ForensicLifecycleState::Pending => "○",
        ForensicLifecycleState::Active => "●",
        ForensicLifecycleState::Succeeded => "✓",
        ForensicLifecycleState::Failed => "×",
        ForensicLifecycleState::Cancelled => "−",
        ForensicLifecycleState::Censored => "◐",
    }
}

fn lifecycle_color(state: ForensicLifecycleState) -> Color {
    match state {
        ForensicLifecycleState::Pending => Color::Muted,
        ForensicLifecycleState::Active => Color::Accent,
        ForensicLifecycleState::Succeeded => Color::Success,
        ForensicLifecycleState::Failed => Color::Error,
        ForensicLifecycleState::Cancelled | ForensicLifecycleState::Censored => Color::Warning,
    }
}

fn review_outcome_label(outcome: ForensicReviewOutcome) -> &'static str {
    match outcome {
        ForensicReviewOutcome::Running => "Running",
        ForensicReviewOutcome::Completed => "Completed",
        ForensicReviewOutcome::CompletedIncomplete => "Completed · incomplete inputs",
        ForensicReviewOutcome::Missed => "Missed · budget retained",
        ForensicReviewOutcome::Cancelled => "Cancelled",
        ForensicReviewOutcome::Failed => "Failed",
        ForensicReviewOutcome::Censored => "Right-censored",
        ForensicReviewOutcome::CleanupFailed => "Cleanup failed",
    }
}

fn prompt_change_label(kind: PromptChangeKind) -> &'static str {
    match kind {
        PromptChangeKind::Section => "Section",
        PromptChangeKind::Example => "Example",
        PromptChangeKind::Schema => "Schema",
        PromptChangeKind::Tool => "Tool",
        PromptChangeKind::Parameter => "Parameter",
        PromptChangeKind::Policy => "Policy",
    }
}

fn statistic_label(statistic: &ForensicStatistic) -> String {
    match (statistic.status, statistic.value) {
        (omega_forensics::ForensicStatisticStatus::NotEstimable, _) => "not estimable".into(),
        (omega_forensics::ForensicStatisticStatus::Provisional, Some(value)) => {
            format!("{value} ms · provisional")
        }
        (_, Some(value)) => format!("{value} ms"),
        (_, None) => "unavailable".into(),
    }
}

fn publication_gate_kind_label(kind: ForensicPublicationGateKind) -> &'static str {
    match kind {
        ForensicPublicationGateKind::Redaction => "Redaction",
        ForensicPublicationGateKind::IndependentReview => "Independent review",
        ForensicPublicationGateKind::DisclosureScope => "Disclosure scope",
        ForensicPublicationGateKind::MaintainerDecision => "Maintainer decision",
        ForensicPublicationGateKind::PublicationAuthority => "Publication authority",
    }
}

fn publication_gate_state_label(state: ForensicPublicationGateState) -> &'static str {
    match state {
        ForensicPublicationGateState::Satisfied => "Satisfied",
        ForensicPublicationGateState::Blocked => "Blocked",
        ForensicPublicationGateState::Denied => "Denied",
        ForensicPublicationGateState::AwaitingReview => "Awaiting review",
        ForensicPublicationGateState::Rejected => "Rejected",
        ForensicPublicationGateState::Stale => "Stale",
        ForensicPublicationGateState::EligibleNotAuthorized => "Eligible · not authorized",
    }
}

fn publication_gate_state_color(state: ForensicPublicationGateState) -> Color {
    match state {
        ForensicPublicationGateState::Satisfied => Color::Success,
        ForensicPublicationGateState::AwaitingReview => Color::Muted,
        ForensicPublicationGateState::Stale
        | ForensicPublicationGateState::EligibleNotAuthorized => Color::Warning,
        ForensicPublicationGateState::Blocked
        | ForensicPublicationGateState::Denied
        | ForensicPublicationGateState::Rejected => Color::Error,
    }
}

fn aggregate_truth_label(value: Option<u64>, exactness: ForensicExactness, unit: &str) -> String {
    match (value, exactness) {
        (None, ForensicExactness::Unavailable) => "unavailable".into(),
        (Some(value), ForensicExactness::Estimated) => format!("≈ {value} {unit}"),
        (Some(value), ForensicExactness::UpperBound) => format!("≤ {value} {unit}"),
        (Some(value), ForensicExactness::Exact) => format!("{value} {unit}"),
        _ => "invalid metric truth".into(),
    }
}

fn matrix_outcome_label(outcome: omega_forensics::ForensicMatrixOutcome) -> &'static str {
    use omega_forensics::ForensicMatrixOutcome;
    match outcome {
        ForensicMatrixOutcome::Hit => "Hit",
        ForensicMatrixOutcome::Miss => "Miss",
        ForensicMatrixOutcome::NotEligible => "Not eligible",
        ForensicMatrixOutcome::Failed => "Failed",
        ForensicMatrixOutcome::Cancelled => "Cancelled",
    }
}

fn matrix_outcome_color(outcome: omega_forensics::ForensicMatrixOutcome) -> Color {
    use omega_forensics::ForensicMatrixOutcome;
    match outcome {
        ForensicMatrixOutcome::Hit => Color::Success,
        ForensicMatrixOutcome::Miss | ForensicMatrixOutcome::NotEligible => Color::Muted,
        ForensicMatrixOutcome::Failed => Color::Error,
        ForensicMatrixOutcome::Cancelled => Color::Warning,
    }
}

fn budget_state_label(state: ForensicBudgetState) -> &'static str {
    match state {
        ForensicBudgetState::WithinBudget => "Within budget",
        ForensicBudgetState::Exhausted => "Exhausted",
        ForensicBudgetState::Unmeasurable => "Unmeasurable",
        ForensicBudgetState::Refused => "Refused",
    }
}

fn coverage_status_label(status: CoverageStatus) -> &'static str {
    match status {
        CoverageStatus::Pending => "Pending",
        CoverageStatus::Complete => "Complete",
        CoverageStatus::Incomplete => "Incomplete",
        CoverageStatus::Denied => "Denied",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread_identity::{BranchIdentity, GitIdentitySummary};
    use omega_forensics::{
        BROKER_NETWORK_POLICY_REF, CoverageSummaryProjection, FORENSIC_TOOL_VERSION_V1,
        ForensicBudgetProjection, ForensicCausalLink, ForensicEvidenceReceiptProjection,
        ForensicExactness, ForensicFindingProjection, ForensicHypothesisProjection,
        ForensicLifecycleStage, ForensicMetricTruth, ForensicToolActorRole, ForensicToolCall,
        ForensicToolCallBinding, ForensicToolPayload, ForensicWorkerPlacement, GCE_ADAPTER_REF,
        MANAGED_TARGET_REF, ManagedIsolation, ManagedProvider, ManagedTargetClass,
        ManagedWorkerProjection, PREFLIGHT_SCHEMA_V1, REVIEW_PROJECTION_SCHEMA_V1,
        RepositoryTargetProjection, WORKER_PLACEMENT_SCHEMA_V1, WorkerPlacementState,
    };
    use std::path::PathBuf;

    #[test]
    fn fixture_views_require_test_support_or_explicit_debug_mock_gate() {
        for (test_support, debug, value, expected) in [
            (true, false, None, true),
            (true, false, Some("0"), true),
            (false, true, Some("1"), true),
            (false, true, Some("0"), false),
            (false, true, Some("true"), false),
            (false, true, None, false),
            (false, false, Some("1"), false),
            (false, false, None, false),
        ] {
            assert_eq!(
                forensics_fixture_views_enabled_for(test_support, debug, value),
                expected,
                "unexpected gate for test_support={test_support}, debug={debug}, value={value:?}"
            );
        }
    }

    #[gpui::test]
    fn completed_typed_tool_call_updates_live_workbench_before_turn_completion(
        cx: &mut gpui::TestAppContext,
    ) {
        crate::test_support::init_test(cx);
        let binding = ForensicToolCallBinding {
            run_ref: "run:live:1".into(),
            task_ref: "task:live:1".into(),
            actor_ref: "actor:discovery:1".into(),
            actor_role: ForensicToolActorRole::Discovery,
            audience_ref: "audience:private:owner".into(),
            source_bundle_ref: "source-bundle:live:1".into(),
            source_bundle_digest: digest('a'),
            coverage_generation: 1,
            prompt_digest: digest('b'),
            model_route_ref: "model-route:live:1".into(),
            tool_version: FORENSIC_TOOL_VERSION_V1.into(),
            budget_ref: "budget:live:1".into(),
            expected_event_cursor: 0,
        };
        let journal =
            ForensicToolJournal::new(binding.clone(), "actor:verifier:1".into()).expect("journal");
        let sources = ForensicSourceCatalog {
            audience_ref: binding.audience_ref.clone(),
            source_bundle_ref: binding.source_bundle_ref.clone(),
            source_bundle_digest: binding.source_bundle_digest.clone(),
            coverage_generation: binding.coverage_generation,
            missing_dependency_paths: Vec::new(),
            files: Vec::new(),
        };
        let call = ForensicToolCall {
            call_ref: "call:live:1".into(),
            idempotency_ref: "idempotency:live:1".into(),
            binding,
            payload: ForensicToolPayload::QueryPriorForensicWork {
                query_ref: "query:live:1".into(),
            },
            observed_at: "2026-08-03T23:30:00Z".into(),
        };
        let raw_input = serde_json::to_value(call).expect("typed call json");
        let repository_binding = RepositoryBinding::new("repo", "worktree").expect("binding");
        let surface =
            cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(repository_binding), cx));
        surface.update(cx, |surface, cx| {
            surface
                .install_forensic_tool_journal(journal, cx)
                .expect("install journal");
            let event = surface
                .ingest_visible_forensic_tool_call(
                    "query_prior_forensic_work",
                    &raw_input,
                    VisibleForensicToolCallState::Completed,
                    &sources,
                    cx,
                )
                .expect("ingest call")
                .expect("typed event");
            assert_eq!(event.sequence, 1);
            assert_eq!(surface.snapshot().tool_journal.unwrap().event_cursor(), 1);
        });
    }

    #[gpui::test]
    fn production_and_mock_navigation_expose_exact_accessible_destinations(
        cx: &mut gpui::TestAppContext,
    ) {
        crate::test_support::init_test(cx);
        let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
        let candidate = candidate(binding);
        let (surface, cx) = cx.add_window_view(|window, cx| {
            ForensicsWorkbenchSurface::new_with_window(&candidate, window, cx)
        });
        cx.set_debug_accessibility_active(true);

        surface.update(cx, |surface, cx| {
            surface.fixture_views_enabled = false;
            surface.coldcard_evidence = None;
            surface.review = None;
            surface.matrix = None;
            surface.restore_bench_view(Some("publication"));
            cx.notify();
        });
        cx.run_until_parked();
        let production_tree = cx
            .debug_render_snapshot()
            .accessibility_tree_json()
            .expect("production accessibility tree")
            .to_string();
        for label in ["Entropy forensics view", "Lifecycle forensics view"] {
            assert!(production_tree.contains(label), "missing {label}");
        }
        for label in [
            "Case forensics view",
            "Evidence forensics view",
            "Models forensics view",
            "Publication forensics view",
            "Development mock data",
        ] {
            assert!(
                !production_tree.contains(label),
                "production exposed {label}"
            );
        }
        assert_eq!(
            surface.read_with(cx, |surface, _| surface.bench_view),
            ForensicsBenchView::Entropy,
            "a persisted fixture-only route must normalize to Entropy"
        );

        surface.update(cx, |surface, cx| {
            surface.fixture_views_enabled = true;
            surface.coldcard_evidence = Some(
                bundled_coldcard_evidence_workspace().expect("valid bundled fixture workspace"),
            );
            cx.notify();
        });
        cx.run_until_parked();
        let mock_tree = cx
            .debug_render_snapshot()
            .accessibility_tree_json()
            .expect("mock accessibility tree")
            .to_string();
        for label in [
            "Entropy forensics view",
            "Case forensics view",
            "Lifecycle forensics view",
            "Evidence forensics view",
            "Models forensics view",
            "Publication forensics view",
            "Development mock data",
        ] {
            assert!(mock_tree.contains(label), "mock mode missing {label}");
        }
    }

    #[gpui::test]
    fn entropy_run_button_emits_the_selected_catalog_project(cx: &mut gpui::TestAppContext) {
        crate::test_support::init_test(cx);
        let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
        let candidate = candidate(binding);
        let (surface, cx) = cx.add_window_view(|window, cx| {
            ForensicsWorkbenchSurface::new_with_window(&candidate, window, cx)
        });
        let started_project = std::rc::Rc::new(std::cell::RefCell::new(None));
        let _subscription = surface.update(cx, |_, cx| {
            let started_project = started_project.clone();
            cx.subscribe(&cx.entity(), move |_, _, event, _| {
                if let ForensicsWorkbenchCommand::StartCatalogEntropy { project, .. } = event {
                    *started_project.borrow_mut() = Some(project.product_ref.clone());
                }
            })
        });

        cx.simulate_click_selector("omega.forensics.entropy.project.product.bitkey")
            .expect("the Bitkey catalog row must accept pointer input");
        cx.simulate_click_selector("omega.forensics.entropy.start")
            .expect("Run entropy scan must accept pointer input");
        cx.run_until_parked();

        assert_eq!(
            started_project.borrow().as_deref(),
            Some("product.bitkey"),
            "Run entropy scan must dispatch the selected catalog project"
        );
        assert_eq!(
            surface.read_with(cx, |surface, _| surface.snapshot().status),
            "Preparing an entropy file manifest…"
        );
    }

    fn candidate(binding: RepositoryBinding) -> ThreadIdentityCandidate {
        ThreadIdentityCandidate {
            binding,
            git_repository_id: Some(1),
            project_name: "Omega".into(),
            repository_name: "omega".into(),
            worktree_name: "omega".into(),
            worktree_abs_path: PathBuf::from("/work/omega"),
            worktree_path: "/work/omega".into(),
            remote_url: Some("https://github.com/OpenAgentsInc/omega.git".into()),
            head_commit: Some("0123456789abcdef0123456789abcdef01234567".into()),
            branch: BranchIdentity::Branch("main".into()),
            git: GitIdentitySummary::default(),
            source_revision: 1,
        }
    }

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn coldcard_evidence_workspace() -> ColdcardEvidenceWorkspaceProjection {
        serde_json::from_str(include_str!(
            "../../omega_forensics/fixtures/coldcard-evidence-workspace.v1.json"
        ))
        .expect("valid Coldcard evidence fixture JSON")
    }

    fn complete_preflight() -> ForensicsPreflightProjection {
        ForensicsPreflightProjection {
            schema: PREFLIGHT_SCHEMA_V1.into(),
            preflight_ref: "preflight-ref://omega/coldcard-v1".into(),
            repository_binding_ref: "repository-binding-ref://omega/current-worktree".into(),
            target: RepositoryTargetProjection {
                source_state: SourceState::Clean,
                dependency_policy: DependencyPolicy::PinnedRecursive,
                ..RepositoryTargetProjection::coldcard(ColdcardBenchmarkArm::Vulnerable)
            },
            worker: ManagedWorkerProjection {
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
                capability_refs: vec!["capability-ref://forensics/source-read".into()],
            },
            budget: ForensicBudgetProjection {
                model_ref: "model-ref://openai/gpt-5.6".into(),
                effort_ref: "effort-ref://high".into(),
                max_concurrency: 2,
                max_time_seconds: 900,
                max_tokens: 100_000,
                max_cost_micros: 5_000_000,
                max_artifact_bytes: 10_000_000,
                max_network_bytes: 0,
            },
            coverage: CoverageSummaryProjection {
                manifest_ref: Some("coverage-manifest-ref://coldcard/complete-v1".into()),
                status: CoverageStatus::Complete,
                present: 103,
                missing: 0,
                excluded: 0,
                generated: 3,
                oversized: 0,
                dependency_owned: 4,
                reason_refs: Vec::new(),
            },
            incomplete_acknowledged: false,
        }
    }

    fn admitted_placement(run_ref: &str) -> ForensicWorkerPlacement {
        ForensicWorkerPlacement {
            schema: WORKER_PLACEMENT_SCHEMA_V1.into(),
            placement_ref: format!("placement.{run_ref}"),
            owner_ref: "owner.forensic.fixture".into(),
            tenant_ref: "owner.forensic.fixture".into(),
            work_unit_ref: run_ref.into(),
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
            run_ref: "run.forensic.coldcard.fixture".into(),
            prompt_digest:
                "sha256:e59c827a678c1f3867ac410b7af729587e7700ac6fec1830b370a77b2c9e8610".into(),
            repository_ref: "repository-ref://coldcard/firmware".into(),
            commit: omega_forensics::COLDCARD_VULNERABLE_COMMIT.into(),
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
                    commit: omega_forensics::COLDCARD_VULNERABLE_COMMIT.into(),
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
                suspected_mechanism: "A second source may share the same state.".into(),
                supporting_refs: vec!["source.coldcard.shared.utils.42".into()],
                missing_evidence: vec!["Executed cross-device reproduction".into()],
                next_check: "Run the trace against two owned fixtures.".into(),
                consequence_if_true: "More devices could share recoverable state.".into(),
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
                    metric_ref: "metric.tokens-to-qualified-identification".into(),
                    label: "Tokens to qualified identification".into(),
                    unit: "tokens".into(),
                    value: None,
                    exactness: ForensicExactness::Unavailable,
                    unavailable_reason_ref: Some("reason.provider-usage-unavailable".into()),
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
            verification_cases: Vec::new(),
        }
    }

    #[gpui::test]
    fn benchmark_arms_are_operator_selectable_without_a_managed_profile(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            for arm in ColdcardBenchmarkArm::ALL {
                surface.update(cx, |surface, cx| surface.select_benchmark_arm(arm, cx));
                assert_eq!(surface.read(cx).snapshot().selected_arm, arm);
                assert_eq!(surface.read(cx).snapshot().readiness, None);
            }
        });
    }

    #[gpui::test]
    fn lifecycle_projection_covers_every_named_ui_scene(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            assert_eq!(
                surface.read(cx).lifecycle_presentation().scene,
                ForensicsLifecycleScene::AwaitingProfile
            );

            let mut preflight = complete_preflight();
            preflight.coverage = CoverageSummaryProjection::pending();
            surface.update(cx, |surface, _| surface.preflight = Some(preflight));
            assert_eq!(
                surface.read(cx).lifecycle_presentation().scene,
                ForensicsLifecycleScene::AwaitingCoverage
            );

            surface.update(cx, |surface, _| {
                surface.preflight = Some(complete_preflight())
            });
            assert_eq!(
                surface.read(cx).lifecycle_presentation().scene,
                ForensicsLifecycleScene::Complete
            );

            let mut preflight = complete_preflight();
            preflight.coverage.status = CoverageStatus::Incomplete;
            preflight.coverage.missing = 2;
            surface.update(cx, |surface, _| surface.preflight = Some(preflight));
            assert_eq!(
                surface.read(cx).lifecycle_presentation().scene,
                ForensicsLifecycleScene::Incomplete
            );

            let mut preflight = complete_preflight();
            preflight.coverage.status = CoverageStatus::Denied;
            preflight.coverage.present = 0;
            preflight.coverage.reason_refs = vec!["reason.policy-denied".into()];
            surface.update(cx, |surface, _| surface.preflight = Some(preflight));
            assert_eq!(
                surface.read(cx).lifecycle_presentation().scene,
                ForensicsLifecycleScene::Denied
            );

            let mut preflight = complete_preflight();
            preflight.worker.capability_refs = vec!["capability.forensics.metadata".into()];
            surface.update(cx, |surface, _| surface.preflight = Some(preflight));
            assert_eq!(
                surface.read(cx).lifecycle_presentation().scene,
                ForensicsLifecycleScene::IncompatibleTool
            );

            surface.update(cx, |surface, _| {
                surface.preflight = Some(complete_preflight());
                let mut run = ForensicsRunProjection::prepared("run.lifecycle.fixture".into())
                    .expect("run projection");
                run.phase = ForensicsRunPhase::Running;
                surface.run = Some(run);
            });
            assert_eq!(
                surface.read(cx).lifecycle_presentation().scene,
                ForensicsLifecycleScene::Running
            );

            surface.update(cx, |surface, _| {
                let run = surface.run.as_mut().expect("run");
                run.phase = ForensicsRunPhase::Settled;
                run.timestamps.cancel_requested_at = Some("2026-08-02T16:00:00Z".into());
            });
            assert_eq!(
                surface.read(cx).lifecycle_presentation().scene,
                ForensicsLifecycleScene::Cancelled
            );

            surface.update(cx, |surface, _| {
                surface.run.as_mut().expect("run").phase = ForensicsRunPhase::RecoveryRequired;
            });
            assert_eq!(
                surface.read(cx).lifecycle_presentation().scene,
                ForensicsLifecycleScene::RecoveryRequired
            );

            surface.update(cx, |surface, _| {
                surface.run.as_mut().expect("run").phase = ForensicsRunPhase::Cleaned;
            });
            assert_eq!(
                surface.read(cx).lifecycle_presentation().scene,
                ForensicsLifecycleScene::Cleaned
            );

            surface.update(cx, |surface, _| {
                surface.run = None;
                let preflight = complete_preflight();
                surface.repository.clone_url = Some(preflight.target.clone_url.clone().into());
                surface.repository.commit = Some("ffffffffffffffffffffffffffffffffffffffff".into());
                surface.preflight = Some(preflight);
            });
            assert_eq!(
                surface.read(cx).lifecycle_presentation().scene,
                ForensicsLifecycleScene::Stale
            );
        });
    }

    #[gpui::test]
    fn bench_and_lifecycle_selection_are_presentation_only(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            let before = surface.read(cx).snapshot();
            surface.update(cx, |surface, cx| {
                surface.select_bench_view(ForensicsBenchView::Lifecycle, cx);
                surface.select_lifecycle_stage(LifecycleSelection::Cleanup, cx);
            });
            let after = surface.read(cx).snapshot();
            assert_eq!(after.bench_view, ForensicsBenchView::Lifecycle);
            assert_eq!(after.lifecycle_selection, LifecycleSelection::Cleanup);
            assert_eq!(after.prepared_intent, before.prepared_intent);
            assert_eq!(after.run, before.run);
            assert_eq!(after.review, before.review);
            assert!(!LIVE_FORENSIC_CONTROLS_ACCEPTED);
        });
    }

    #[gpui::test]
    fn evidence_queue_keeps_claim_classes_and_private_boundaries_visible(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            let surface = surface.read(cx);
            let workspace = surface
                .coldcard_evidence
                .as_ref()
                .expect("bundled evidence workspace");

            assert_eq!(
                surface.evidence_section_count(EvidenceSelection::Findings),
                6
            );
            assert_eq!(
                surface.evidence_section_count(EvidenceSelection::Hypotheses),
                2
            );
            assert_eq!(
                surface.evidence_section_count(EvidenceSelection::Limitations),
                4
            );
            assert_eq!(
                surface.evidence_section_count(EvidenceSelection::Disputes),
                2
            );
            assert_eq!(
                surface.evidence_section_count(EvidenceSelection::Reconciliation),
                2
            );
            assert!(!workspace.scan.reportable);
            assert!(workspace.reconciliation.iter().any(|item| {
                item.status == omega_forensics::ColdcardReconciliationStatus::Unavailable
                    && item.derived_value.is_none()
                    && item.published_value.is_none()
            }));
            assert!(
                workspace
                    .ladder
                    .iter()
                    .filter(|rung| rung.state == ColdcardRungState::Missing)
                    .all(|rung| !rung.non_implications.is_empty())
            );
        });
    }

    #[gpui::test]
    fn evidence_selection_is_presentation_only(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            let before = surface.read(cx).snapshot();
            surface.update(cx, |surface, cx| {
                surface.select_bench_view(ForensicsBenchView::Evidence, cx);
                surface.select_evidence_section(EvidenceSelection::Disputes, cx);
            });
            let after = surface.read(cx).snapshot();
            assert_eq!(after.bench_view, ForensicsBenchView::Evidence);
            assert_eq!(after.evidence_selection, EvidenceSelection::Disputes);
            assert_eq!(after.coldcard_evidence, before.coldcard_evidence);
            assert_eq!(after.review, before.review);
            assert_eq!(after.run, before.run);
            assert_eq!(
                surface
                    .read(cx)
                    .entropy_restore_state()
                    .evidence_selection
                    .as_deref(),
                Some("disputes")
            );
        });
    }

    #[gpui::test]
    fn entropy_campaign_keeps_every_target_and_selected_project_state(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            let catalog = EntropyProjectCatalog::wallet_entropy_v2().expect("wallet catalog");
            let prompt = EntropyPromptSnapshot::new(
                "prompt.entropy.campaign.fixture".into(),
                None,
                None,
                "Inspect entropy only.".into(),
                "2026-08-02T08:15:00Z".into(),
            )
            .expect("prompt");
            let campaign_binding = omega_forensics::EntropyCampaignBinding {
                campaign_ref: "campaign.entropy.fixture".into(),
                catalog_ref: catalog.catalog_ref.clone(),
                catalog_digest: catalog.canonical_digest.clone(),
                prompt_digest: prompt.canonical_digest.clone(),
                prompt_snapshot: prompt,
                model_route_ref: "model-route.fixture.kimi".into(),
                model_parameters: omega_forensics::EntropyModelParameters {
                    temperature_millis: 0,
                    thinking_allowed: true,
                    reasoning_effort_ref: None,
                },
                tool_surface_refs: vec!["tool.omega.project.read".into()],
                file_selection_policy_ref: omega_forensics::ENTROPY_FILE_SELECTION_POLICY_REF_V1
                    .into(),
                started_at: "2026-08-02T08:15:00Z".into(),
            };
            let mut campaign = EntropyCampaignProjection::new(campaign_binding, catalog)
                .expect("campaign projection");
            campaign.start().expect("start campaign");
            surface.update(cx, |surface, cx| {
                surface.install_entropy_campaign(campaign, cx);
                surface.select_entropy_project("product.samourai-wallet".into(), cx);
            });
            let snapshot = surface.read(cx).snapshot();
            assert_eq!(
                snapshot
                    .entropy_campaign
                    .as_ref()
                    .expect("campaign")
                    .projects
                    .len(),
                17
            );
            assert_eq!(
                snapshot.selected_entropy_project.as_deref(),
                Some("product.samourai-wallet")
            );
        });
    }

    #[gpui::test]
    fn preflight_is_bound_and_only_an_explicit_action_prepares_a_run(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let foreign = RepositoryBinding::new("other-repo", "worktree").expect("valid binding");
            let surface =
                cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding.clone()), cx));
            assert!(
                surface
                    .update(cx, |surface, cx| surface.set_managed_preflight(
                        &foreign,
                        complete_preflight(),
                        cx
                    ))
                    .is_err()
            );
            surface
                .update(cx, |surface, cx| {
                    surface.set_managed_preflight(&binding, complete_preflight(), cx)
                })
                .expect("matching managed preflight");
            assert_eq!(surface.read(cx).snapshot().prepared_intent, None);
            surface
                .update(cx, |surface, cx| surface.prepare_run(cx))
                .expect("explicit operator action prepares a run");
            let snapshot = surface.read(cx).snapshot();
            assert_eq!(snapshot.readiness, Some(PreflightReadiness::Ready));
            assert_eq!(
                snapshot
                    .prepared_intent
                    .as_ref()
                    .map(|intent| intent.operator_action_ref.as_str()),
                Some(PREPARE_ACTION_REF)
            );
        });
    }

    #[gpui::test]
    fn explicit_launch_projects_the_host_owned_worker_lifecycle(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface =
                cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding.clone()), cx));
            surface
                .update(cx, |surface, cx| {
                    surface.set_managed_preflight(&binding, complete_preflight(), cx)
                })
                .expect("managed preflight");
            surface
                .update(cx, |surface, cx| surface.prepare_run(cx))
                .expect("prepare run");
            surface
                .update(cx, |surface, cx| surface.launch_run(cx))
                .expect("launch intent");
            assert_eq!(
                surface
                    .read(cx)
                    .snapshot()
                    .run
                    .as_ref()
                    .map(|run| run.phase),
                Some(ForensicsRunPhase::Prepared)
            );
            surface.update(cx, |surface, cx| {
                surface.mark_admitting("2026-08-01T09:59:59.000Z".into(), cx)
            });
            let run_ref = surface
                .read(cx)
                .snapshot()
                .run
                .as_ref()
                .map(|run| run.run_ref.clone())
                .expect("prepared run ref");
            surface
                .update(cx, |surface, cx| {
                    surface.apply_admission(admitted_placement(&run_ref), cx)
                })
                .expect("admission projection");
            let snapshot = surface.read(cx).snapshot();
            assert_eq!(
                snapshot.run.as_ref().map(|run| run.phase),
                Some(ForensicsRunPhase::WorkerReady)
            );
            assert_eq!(snapshot.status.as_ref(), "Worker ready");
        });
    }

    #[gpui::test]
    fn review_keeps_findings_hypotheses_usage_resolution_and_decisions_distinct(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface =
                cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding.clone()), cx));
            surface
                .update(cx, |surface, cx| {
                    surface.set_managed_preflight(&binding, complete_preflight(), cx)
                })
                .expect("managed preflight");
            surface
                .update(cx, |surface, cx| {
                    surface.set_review_projection(review_projection(), cx)
                })
                .expect("review projection");
            let original_finding = surface
                .read(cx)
                .snapshot()
                .review
                .as_ref()
                .and_then(|review| review.findings.first())
                .cloned()
                .expect("review finding");
            surface
                .update(cx, |surface, cx| {
                    surface.record_review_decision(
                        &original_finding.finding_ref,
                        ForensicReviewDecisionKind::Correct,
                        cx,
                    )
                })
                .expect("append review decision");
            surface.update(cx, |surface, cx| {
                surface.apply_source_resolution(
                    original_finding.source_refs[0].source_ref.clone(),
                    Err("pinned file is absent".into()),
                    cx,
                )
            });
            let snapshot = surface.read(cx).snapshot();
            let review = snapshot.review.expect("review");
            assert_eq!(review.findings[0], original_finding);
            assert_eq!(review.hypotheses[0].state, "unverified");
            assert_eq!(review.metrics[1].display_value(), "Unavailable");
            assert_eq!(review.decisions.len(), 1);
            assert!(matches!(
                snapshot.source_resolutions.get("source.coldcard.shared.utils.42"),
                Some(ForensicSourceResolution::Failed(reason)) if reason == "pinned file is absent"
            ));
        });
    }

    #[gpui::test]
    fn fixture_verification_request_preserves_the_finding_and_keeps_remediation_locked(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface =
                cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding.clone()), cx));
            surface.update(cx, |surface, _| surface.fixture_views_enabled = true);
            surface
                .update(cx, |surface, cx| {
                    surface.set_managed_preflight(&binding, complete_preflight(), cx)
                })
                .expect("managed preflight");
            surface
                .update(cx, |surface, cx| {
                    surface.set_review_projection(review_projection(), cx)
                })
                .expect("review projection");

            let original_finding = surface
                .read(cx)
                .review
                .as_ref()
                .and_then(|review| review.findings.first())
                .cloned()
                .expect("fixture finding");
            surface
                .update(cx, |surface, cx| {
                    surface
                        .request_fixture_independent_verification(&original_finding.finding_ref, cx)
                })
                .expect("independent verification request");

            let snapshot = surface.read(cx).snapshot();
            let review = snapshot.review.expect("review");
            assert_eq!(review.findings[0], original_finding);
            assert_eq!(review.verification_cases.len(), 1);
            assert_eq!(
                review.verification_cases[0].envelope.finding,
                original_finding
            );
            assert!(!review.verification_cases[0].remediation_enabled);
            assert_eq!(
                snapshot.status.as_ref(),
                "Independent verification requested · patch work remains locked"
            );
        });
    }

    #[gpui::test]
    fn prompt_editor_saves_a_new_candidate_without_mutating_the_active_artifact(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            let original = surface
                .read(cx)
                .snapshot()
                .prompt_workspace
                .active()
                .clone();
            surface
                .update(cx, |surface, cx| surface.clone_prompt_candidate(cx))
                .expect("clone prompt");
            let mut prompt_ir = surface
                .read(cx)
                .snapshot()
                .prompt_workspace
                .draft()
                .expect("draft")
                .prompt_ir
                .clone();
            prompt_ir.evidence_requirements.push(
                "Reproduce the Coldcard fallback entropy path from its pinned fixture.".into(),
            );
            surface
                .update(cx, |surface, cx| surface.update_prompt_draft(prompt_ir, cx))
                .expect("edit structured prompt");
            assert_eq!(
                surface.read(cx).snapshot().prompt_workspace.active(),
                &original
            );
            surface
                .update(cx, |surface, cx| surface.save_prompt_candidate(cx))
                .expect("save candidate");
            let snapshot = surface.read(cx).snapshot();
            assert_eq!(snapshot.prompt_workspace.candidates().count(), 2);
            assert_eq!(snapshot.prompt_workspace.active(), &original);
        });
    }

    #[gpui::test]
    fn matrix_projection_keeps_retained_run_scorecards_in_workbench_state(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            let arm = omega_forensics::ForensicMatrixArm {
                arm_ref: "arm.forensic.candidate".into(),
                model_family_ref: "model-family.openai.gpt-5".into(),
                role_ref: "role.forensic.entropy-specialist".into(),
                prompt_digest: digest('a'),
                model_digest: digest('b'),
                effort_ref: "effort.high".into(),
                scope_ref: "scope.entropy".into(),
                dependency_policy_ref: "dependency.pinned".into(),
                random_seed: 7,
                tool_surface_digest: digest('c'),
                analysis_mode_ref: "analysis.static-and-build".into(),
                worker_image_digest: digest('d'),
                worker_profile_digest: digest('e'),
                source_bundle_digest: digest('f'),
                writable_disk_ref: "disk.candidate".into(),
                provider_session_ref: "provider-session.candidate".into(),
                auth_home_ref: "auth-home.candidate".into(),
                environment_ref: "environment.candidate".into(),
                worker_state_ref: "worker-state.candidate".into(),
            };
            let run = omega_forensics::ForensicMatrixRun {
                run_ref: "run.matrix.candidate".into(),
                run_digest: digest('1'),
                arm_ref: arm.arm_ref.clone(),
                dataset_split: omega_forensics::ForensicDatasetSplit::Holdout,
                population: omega_forensics::ForensicMatrixPopulation::Vulnerable,
                coverage_status: CoverageStatus::Complete,
                outcome: omega_forensics::ForensicMatrixOutcome::Hit,
                censored: false,
                censor_at_milliseconds: None,
                identification_milliseconds: Some(8_000),
                identification_tokens: Some(2_000),
                total_tokens: Some(4_000),
                token_exactness: ForensicExactness::Exact,
                cost_micros: Some(200_000),
                cost_exactness: ForensicExactness::Exact,
                causal_links_supported: 4,
                causal_links_required: 4,
                false_positive_count: 0,
                reviewer_active_seconds: Some(90),
                budget_compliant: true,
                cleanup_observed: true,
                qualified_finding_refs: vec!["finding.coldcard.entropy-fallback".into()],
                failure_refs: Vec::new(),
                event_refs: vec!["event.matrix.finding".into()],
                receipt_refs: vec!["receipt.matrix.cleanup".into()],
            };
            let matrix = ForensicsMatrixProjection::rebuild(
                "matrix.forensic.workbench".into(),
                digest('2'),
                digest('3'),
                digest('4'),
                3,
                vec![arm],
                vec![run],
                omega_forensics::ForensicMatrixHardGates {
                    input_complete: true,
                    isolation_complete: true,
                    clean_control: true,
                    evidence_quality: true,
                    budget_compliant: true,
                    cleanup_complete: true,
                    hit_rate_not_regressed: true,
                },
                omega_forensics::ForensicParetoStatus::NonDominated,
                false,
            )
            .expect("matrix projection");
            surface
                .update(cx, |surface, cx| surface.set_matrix_projection(matrix, cx))
                .expect("set matrix");
            let snapshot = surface.read(cx).snapshot();
            let matrix = snapshot.matrix.expect("matrix state");
            assert_eq!(matrix.rows[0].run_refs, vec!["run.matrix.candidate"]);
            assert_eq!(matrix.rows[0].hit_count, 1);
        });
    }

    #[test]
    fn bundled_model_matrix_preserves_roles_outcomes_and_unavailable_truth() {
        let matrix = bundled_coldcard_model_matrix().expect("valid bundled model matrix");
        matrix.validate().expect("validated model matrix");
        assert_eq!(matrix.arms.len(), 3);
        assert_eq!(matrix.runs.len(), 3);
        assert!(
            matrix
                .arms
                .iter()
                .all(|arm| !arm.model_family_ref.is_empty() && !arm.role_ref.is_empty())
        );
        assert!(
            matrix
                .arms
                .windows(2)
                .all(|arms| arms[0].prompt_digest == arms[1].prompt_digest)
        );
        assert!(matrix.runs.iter().any(|run| {
            run.outcome == omega_forensics::ForensicMatrixOutcome::Miss && run.censored
        }));
        assert!(matrix.runs.iter().any(|run| {
            run.outcome == omega_forensics::ForensicMatrixOutcome::NotEligible
                && run.total_tokens.is_none()
                && run.token_exactness == ForensicExactness::Unavailable
                && run.cost_micros.is_none()
                && run.cost_exactness == ForensicExactness::Unavailable
        }));
        assert!(!matrix.promoted);
    }

    #[gpui::test]
    fn model_run_selection_is_presentation_only(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            let before = surface.read(cx).snapshot();
            surface.update(cx, |surface, cx| {
                surface.select_bench_view(ForensicsBenchView::Models, cx);
                surface.select_model_run("run.matrix.general-reviewer".into(), cx);
            });
            let after = surface.read(cx).snapshot();
            assert_eq!(after.bench_view, ForensicsBenchView::Models);
            assert_eq!(
                after.selected_model_run_ref.as_deref(),
                Some("run.matrix.general-reviewer")
            );
            assert_eq!(after.matrix, before.matrix);
            assert_eq!(after.review, before.review);
            assert_eq!(after.run, before.run);
            assert_eq!(
                surface
                    .read(cx)
                    .entropy_restore_state()
                    .model_run_ref
                    .as_deref(),
                Some("run.matrix.general-reviewer")
            );
        });
    }

    #[test]
    fn bundled_publication_scenes_are_private_blocked_and_separate_authorities() {
        for scene in PublicationScene::ALL {
            let projection = bundled_publication_gate(scene).expect("valid publication fixture");
            assert!(projection.private);
            assert!(projection.synthetic);
            assert!(!projection.publication_authorized);
            assert_eq!(projection.gates.len(), 5);
            assert!(
                projection
                    .gates
                    .iter()
                    .any(|gate| gate.kind == ForensicPublicationGateKind::PublicationAuthority)
            );
        }
    }

    #[gpui::test]
    fn publication_scene_selection_is_presentation_only(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            let before = surface.read(cx).snapshot();
            surface.update(cx, |surface, cx| {
                surface.select_bench_view(ForensicsBenchView::Publication, cx);
                surface.select_publication_scene(PublicationScene::Stale, cx);
            });
            let after = surface.read(cx).snapshot();
            assert_eq!(after.bench_view, ForensicsBenchView::Publication);
            assert_eq!(after.publication_scene, PublicationScene::Stale);
            assert_eq!(after.matrix, before.matrix);
            assert_eq!(after.review, before.review);
            assert_eq!(after.run, before.run);
            assert_eq!(
                surface
                    .read(cx)
                    .entropy_restore_state()
                    .publication_scene
                    .as_deref(),
                Some("stale")
            );
        });
    }

    #[gpui::test]
    fn coldcard_views_keep_missing_rungs_private_ids_and_original_corrections(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            surface
                .update(cx, |surface, cx| {
                    surface.set_coldcard_evidence_projection(coldcard_evidence_workspace(), cx)
                })
                .expect("valid private Coldcard evidence projection");
            let snapshot = surface.read(cx).snapshot();
            let workspace = snapshot.coldcard_evidence.expect("Coldcard evidence state");
            assert_eq!(workspace.ladder.len(), 9);
            assert_eq!(
                workspace.ladder[6].state,
                omega_forensics::ColdcardRungState::Missing
            );
            assert!(!workspace.scan.reportable);
            assert_eq!(workspace.scan.public_transaction_refs.len(), 1);
            assert_eq!(workspace.corrections[0].prior_value, "5 candidate clusters");
            assert_eq!(
                workspace.corrections[0].corrected_value,
                "4 candidate clusters"
            );
            assert_eq!(
                snapshot.status.as_ref(),
                "Coldcard evidence ready · 6 evidenced rungs · private boundary"
            );
        });
    }

    #[gpui::test]
    fn coldcard_case_reader_loads_the_validated_fixture_without_live_authority(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));

            let initial = surface.read(cx).snapshot();
            assert_eq!(
                initial.coldcard_case_reader_state,
                ColdcardCaseReaderState::Complete
            );
            assert_eq!(
                initial.coldcard_case_selection,
                ColdcardCaseSelection::Overview
            );
            let fixture = initial
                .coldcard_evidence
                .as_ref()
                .expect("bundled validated Coldcard case");
            assert_eq!(fixture.ladder.len(), ColdcardClaimRung::ALL.len());
            assert_eq!(
                fixture
                    .ladder
                    .iter()
                    .filter(|rung| rung.state == ColdcardRungState::Missing)
                    .count(),
                3
            );
            assert!(!fixture.scan.reportable);
            assert!(initial.prepared_intent.is_none());
            assert!(initial.run.is_none());
            assert!(initial.review.is_none());

            surface.update(cx, |surface, cx| {
                surface.select_coldcard_case(
                    ColdcardCaseSelection::Rung(ColdcardClaimRung::Entity),
                    cx,
                );
            });
            let selected = surface.read(cx).snapshot();
            assert_eq!(
                selected.coldcard_case_selection,
                ColdcardCaseSelection::Rung(ColdcardClaimRung::Entity)
            );
            assert_eq!(selected.coldcard_evidence, initial.coldcard_evidence);
            assert!(selected.prepared_intent.is_none());
            assert!(selected.run.is_none());
            assert!(selected.review.is_none());
        });
    }

    #[gpui::test]
    fn coldcard_case_reader_preserves_deterministic_non_authoritative_scenes(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            let scenes = [
                ColdcardCaseReaderState::Loading,
                ColdcardCaseReaderState::Empty,
                ColdcardCaseReaderState::Invalid("fixture validation failed".into()),
                ColdcardCaseReaderState::Stale("fixture revision changed".into()),
                ColdcardCaseReaderState::Complete,
            ];

            for scene in scenes {
                surface.update(cx, |surface, cx| {
                    surface.coldcard_case_reader_state = scene.clone();
                    cx.notify();
                });
                let snapshot = surface.read(cx).snapshot();
                assert_eq!(snapshot.coldcard_case_reader_state, scene);
                assert!(snapshot.prepared_intent.is_none());
                assert!(snapshot.run.is_none());
                assert!(snapshot.review.is_none());
            }
        });
    }

    #[gpui::test]
    fn source_inspection_generations_project_complete_incomplete_and_stale_truth(
        cx: &mut gpui::TestAppContext,
    ) {
        let root = tempfile::tempdir().expect("temporary repository");
        std::fs::write(root.path().join("rng.c"), "int rng_get(void);\n").expect("source fixture");
        let repository = omega_forensics::EntropyRepositoryBinding {
            repository_ref: "repository.omega.fixture".into(),
            display_name: "Omega fixture".into(),
            revision: "0123456789abcdef0123456789abcdef01234567".into(),
        };
        let manifest = omega_forensics::EntropyManifest::build(
            root.path(),
            "manifest.omega.source-inspection.fixture".into(),
            repository,
            Vec::new(),
            1_024,
        )
        .expect("source manifest");
        let inspection = |generation, tree: &str| {
            omega_forensics::EntropySourceInspection::from_manifest(
                &manifest,
                omega_forensics::EntropySourceInspectionInput {
                    inspection_ref: "inspection.omega.fixture".into(),
                    generation,
                    observed_revision: "0123456789abcdef0123456789abcdef01234567".into(),
                    top_level_tree: tree.into(),
                    focal_paths: vec!["rng.c".into()],
                    reached_paths: Vec::new(),
                    required_generated_input_paths: Vec::new(),
                    missing_generated_input_paths: Vec::new(),
                    required_excluded_paths: Vec::new(),
                    dirty_excluded_paths: Vec::new(),
                    observed_at: format!("2026-08-03T20:00:0{generation}Z"),
                },
            )
            .expect("source inspection")
        };

        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            surface
                .update(cx, |surface, cx| {
                    surface.install_entropy_source_inspection(
                        inspection(1, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                        cx,
                    )
                })
                .expect("complete inspection");
            assert_eq!(
                surface
                    .read(cx)
                    .snapshot()
                    .entropy_source_inspection
                    .as_ref()
                    .map(|inspection| inspection.state),
                Some(EntropySourceInspectionState::Complete)
            );

            surface
                .update(cx, |surface, cx| {
                    surface.install_entropy_source_inspection(
                        inspection(2, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                        cx,
                    )
                })
                .expect("changed tree becomes stale");
            let stale = surface
                .read(cx)
                .snapshot()
                .entropy_source_inspection
                .expect("stale inspection");
            assert_eq!(stale.state, EntropySourceInspectionState::Stale);
            assert!(!stale.qualified_miss_eligible());
        });
    }

    #[gpui::test]
    fn entropy_run_exposes_ordered_progress_and_cancellation(cx: &mut gpui::TestAppContext) {
        let root = tempfile::tempdir().expect("temporary repository");
        std::fs::write(root.path().join("a.c"), "int rng_get(void);\n").expect("first source");
        std::fs::write(root.path().join("b.py"), "seed = random(32)\n").expect("second source");
        let repository = omega_forensics::EntropyRepositoryBinding {
            repository_ref: "repository.omega.fixture".into(),
            display_name: "Omega fixture".into(),
            revision: "0123456789abcdef0123456789abcdef01234567".into(),
        };
        let manifest = omega_forensics::EntropyManifest::build(
            root.path(),
            "manifest.omega.entropy.fixture".into(),
            repository.clone(),
            Vec::new(),
            1_024,
        )
        .expect("valid entropy manifest");
        let run = omega_forensics::EntropyRunProjection::new(
            {
                let prompt_snapshot = omega_forensics::EntropyPromptSnapshot::new(
                    "prompt.omega.entropy.fixture".into(),
                    None,
                    None,
                    omega_forensics::DEFAULT_ENTROPY_ANALYSIS_PROMPT.into(),
                    "2026-08-02T18:19:00Z".into(),
                )
                .expect("valid prompt snapshot");
                omega_forensics::EntropyRunBinding {
                    run_ref: "run.omega.entropy.fixture".into(),
                    repository,
                    manifest_ref: manifest.manifest_ref.clone(),
                    manifest_digest: manifest.canonical_digest.clone(),
                    prompt_digest: prompt_snapshot.canonical_digest.clone(),
                    prompt_snapshot,
                    model_route_ref: "model.omega.fixture".into(),
                    model_parameters: omega_forensics::EntropyModelParameters {
                        temperature_millis: 0,
                        thinking_allowed: true,
                        reasoning_effort_ref: None,
                    },
                    tool_surface_refs: vec!["tool.omega.project.read".into()],
                    started_at: "2026-08-02T18:20:00Z".into(),
                }
            },
            manifest,
        )
        .expect("valid entropy run");
        let mut prior = run.clone();
        let mut current = run.clone();
        prior.files[0].state = EntropyFileState::Candidate;
        current.files[0].state = EntropyFileState::Candidate;
        current.files[1].state = EntropyFileState::Candidate;
        assert_eq!(
            compare_entropy_runs(&prior, &current),
            EntropyRunComparison {
                gained: 1,
                lost: 0,
                changed: 0,
                unchanged: 1,
            }
        );

        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            surface.update(cx, |surface, cx| surface.install_entropy_run(run, cx));
            surface.update(cx, |surface, cx| {
                surface.select_entropy_filter(EntropyFileFilter::Incomplete, cx);
                surface.select_entropy_file("b.py".into(), cx);
            });
            let dashboard = surface.read(cx).snapshot();
            assert_eq!(dashboard.entropy_file_filter, EntropyFileFilter::Incomplete);
            assert_eq!(dashboard.selected_entropy_file.as_deref(), Some("b.py"));
            surface.update(cx, |surface, cx| {
                surface.entropy_prompt_draft = "A changed prompt for the next run.".into();
                cx.notify();
            });
            assert_eq!(
                surface
                    .read(cx)
                    .snapshot()
                    .entropy_run
                    .as_ref()
                    .expect("bound run")
                    .binding
                    .prompt_snapshot
                    .text,
                omega_forensics::DEFAULT_ENTROPY_ANALYSIS_PROMPT
            );
            let first = surface
                .update(cx, |surface, cx| {
                    surface.start_next_entropy_file("2026-08-02T18:20:01Z".into(), cx)
                })
                .expect("first file should start")
                .expect("first file task");
            assert_eq!(first.file_path, "a.c");
            surface
                .update(cx, |surface, cx| surface.cancel_entropy_run(cx))
                .expect("cancellation should be explicit");
            let snapshot = surface.read(cx).snapshot();
            let entropy = snapshot.entropy_run.expect("entropy state");
            assert_eq!(entropy.phase, omega_forensics::EntropyRunPhase::Cancelled);
            assert_eq!(entropy.counts().cancelled, 2);
        });
    }
}
