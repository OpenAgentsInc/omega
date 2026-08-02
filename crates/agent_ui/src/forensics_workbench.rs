use editor::{Editor, EditorEvent};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, Render, SharedString, Subscription,
    TaskExt, Window,
};
use omega_forensics::{
    ColdcardBenchmarkArm, ColdcardEvidenceWorkspaceProjection, CoverageStatus,
    DEFAULT_ENTROPY_ANALYSIS_PROMPT, DependencyPolicy, EntropyCampaignComparison,
    EntropyCampaignPhase, EntropyCampaignProjection, EntropyFileAnalysisOutput, EntropyFileState,
    EntropyFileTask, EntropyLimitation, EntropyProjectCatalog, EntropyPromptSnapshot,
    EntropyRunPhase, EntropyRunProjection, ExplicitOperatorAction, FORENSIC_FINDING_SCHEMA_V1,
    FORENSIC_HYPOTHESIS_SCHEMA_V1, ForensicBudgetState, ForensicEvidenceTier, ForensicExactness,
    ForensicLifecycleState, ForensicPromptIr, ForensicPromptWorkspace, ForensicReviewDecisionKind,
    ForensicReviewOutcome, ForensicSourceCitation, ForensicStatistic, ForensicWorkerObservation,
    ForensicWorkerPlacement, ForensicsFailureProjection, ForensicsLaunchIntent,
    ForensicsMatrixProjection, ForensicsPreflightProjection, ForensicsReviewProjection,
    ForensicsRunPhase, ForensicsRunProjection, PreflightReadiness, PromptChangeKind,
    PromptCompatibilityProfile, SourceState,
};
use omega_workbench_state::RepositoryBinding;
use sha2::{Digest, Sha256};
use ui::{
    Button, ButtonSize, ButtonStyle, Color, Icon, IconName, IconSize, Label, LabelSize, prelude::*,
    v_flex,
};

use crate::thread_identity::ThreadIdentityCandidate;

const PREPARE_ACTION_REF: &str = "operator-action-ref://omega/forensics/prepare-run";
const MAX_VISIBLE_ENTROPY_FILES: usize = 500;

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
    pub selected_arm: ColdcardBenchmarkArm,
    pub readiness: Option<PreflightReadiness>,
    pub prepared_intent: Option<ForensicsLaunchIntent>,
    pub run: Option<ForensicsRunProjection>,
    pub review: Option<ForensicsReviewProjection>,
    pub prompt_workspace: ForensicPromptWorkspace,
    pub matrix: Option<ForensicsMatrixProjection>,
    pub coldcard_evidence: Option<ColdcardEvidenceWorkspaceProjection>,
    pub entropy_run: Option<EntropyRunProjection>,
    pub entropy_run_history: Vec<EntropyRunProjection>,
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
    selected_arm: ColdcardBenchmarkArm,
    preflight: Option<ForensicsPreflightProjection>,
    prepared_intent: Option<ForensicsLaunchIntent>,
    run: Option<ForensicsRunProjection>,
    review: Option<ForensicsReviewProjection>,
    prompt_workspace: ForensicPromptWorkspace,
    matrix: Option<ForensicsMatrixProjection>,
    coldcard_evidence: Option<ColdcardEvidenceWorkspaceProjection>,
    entropy_run: Option<EntropyRunProjection>,
    entropy_run_history: Vec<EntropyRunProjection>,
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
    _entropy_prompt_subscription: Option<Subscription>,
    source_resolutions: std::collections::BTreeMap<String, ForensicSourceResolution>,
    status: SharedString,
}

impl ForensicsWorkbenchSurface {
    pub fn new(candidate: &ThreadIdentityCandidate, cx: &mut Context<Self>) -> Self {
        let entropy_catalog = EntropyProjectCatalog::wallet_entropy_v1()
            .expect("the built-in 15-project entropy catalog must remain valid");
        let selected_entropy_project = entropy_catalog
            .projects
            .first()
            .map(|project| project.product_ref.clone());
        Self {
            focus_handle: cx.focus_handle(),
            binding: candidate.binding.clone(),
            repository: ForensicsRepositoryContext {
                display_name: candidate.repository_name.clone(),
                clone_url: candidate.remote_url.clone(),
                commit: candidate.head_commit.clone(),
                dirty_files: candidate.git.dirty_files,
            },
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
            coldcard_evidence: None,
            entropy_run: None,
            entropy_run_history: Vec::new(),
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
            _entropy_prompt_subscription: None,
            source_resolutions: std::collections::BTreeMap::new(),
            status: "Awaiting OpenAgents managed profile".into(),
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
        self.entropy_prompt_snapshots.push(snapshot.clone());
        if let Some(previous) = self.entropy_run.take() {
            self.entropy_run_history.push(previous);
        }
        self.status = "Preparing an entropy file manifest…".into();
        cx.emit(ForensicsWorkbenchCommand::StartEntropy {
            prompt_snapshot: snapshot,
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
        self.status = "Preparing the 15-project entropy campaign…".into();
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
        self.status = "15-project entropy campaign started".into();
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
        self.coldcard_evidence = None;
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
        self.coldcard_evidence = None;
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
        cx.notify();
        Ok(())
    }

    pub fn open_source(&mut self, citation: ForensicSourceCitation, cx: &mut Context<Self>) {
        let repository_root = self
            .selected_entropy_project
            .as_ref()
            .and_then(|product_ref| self.entropy_campaign_roots.get(product_ref))
            .cloned();
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

    pub fn snapshot(&self) -> ForensicsWorkbenchSnapshot {
        ForensicsWorkbenchSnapshot {
            binding: self.binding.clone(),
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
            entropy_run: self.entropy_run.clone(),
            entropy_run_history: self.entropy_run_history.clone(),
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
}

impl EventEmitter<ForensicsWorkbenchCommand> for ForensicsWorkbenchSurface {}

impl Focusable for ForensicsWorkbenchSurface {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ForensicsWorkbenchSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .unwrap_or_else(|| "Unborn".into());
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
        let can_prepare = self.preflight.as_ref().is_some_and(|preflight| {
            matches!(preflight.readiness(), PreflightReadiness::Ready)
                || (preflight.readiness() == PreflightReadiness::IncompleteResearch
                    && preflight.incomplete_acknowledged)
        });
        let needs_acknowledgment = self.preflight.as_ref().is_some_and(|preflight| {
            preflight.coverage.status == CoverageStatus::Incomplete
                && !preflight.incomplete_acknowledged
        });
        let run_phase = self.run.as_ref().map(|run| run.phase);
        let can_launch = self.prepared_intent.is_some() && self.run.is_none();
        let can_refresh = run_phase.is_some_and(|phase| {
            !matches!(
                phase,
                ForensicsRunPhase::Prepared
                    | ForensicsRunPhase::Admitting
                    | ForensicsRunPhase::Cleaned
                    | ForensicsRunPhase::Refused
                    | ForensicsRunPhase::Failed
            )
        });
        let can_cancel = run_phase.is_some_and(|phase| matches!(phase, ForensicsRunPhase::Running));
        let can_cleanup = run_phase.is_some_and(|phase| {
            matches!(
                phase,
                ForensicsRunPhase::WorkerReady
                    | ForensicsRunPhase::Settled
                    | ForensicsRunPhase::RecoveryRequired
            )
        });
        let review = self.review.clone();
        let source_resolutions = self.source_resolutions.clone();
        let prompt_workspace = self.prompt_workspace.clone();
        let active_prompt = prompt_workspace.active().clone();
        let prompt_changes = prompt_workspace.semantic_diff().unwrap_or_default();
        let prompt_candidates = prompt_workspace.candidates().cloned().collect::<Vec<_>>();
        let matrix = self.matrix.clone();
        let coldcard_evidence = self.coldcard_evidence.clone();
        let entropy_run = self.entropy_run.clone();
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

        v_flex()
            .id("omega.forensics.workbench")
            .debug_selector(|| "omega.forensics.workbench".to_string())
            .track_focus(&self.focus_handle)
            .tab_index(0)
            .role(gpui::Role::Group)
            .aria_label("Forensics preflight workbench")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(Icon::new(IconName::Crosshair).size(IconSize::Small))
                    .child(Label::new("Forensics").size(LabelSize::Small)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        Label::new("Entropy repository run")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(
                            "Read-only file traversal using the configured model. Source and tool limitations remain visible.",
                        )
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    )
                    .child(
                        Label::new("Entropy prompt")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .when_some(entropy_prompt_editor, |this, editor| {
                        this.child(div().min_h_24().p_1().border_1().border_color(cx.theme().colors().border).child(editor))
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
                                    "Run 15-project campaign",
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
            .child(div().h_px().bg(cx.theme().colors().border))
            .child(
                v_flex()
                    .id("omega.forensics.entropy.campaign")
                    .debug_selector(|| "omega.forensics.entropy.campaign".into())
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                Label::new("15-project entropy campaign").size(LabelSize::Small),
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
                        Label::new(
                            "One frozen prompt and source policy across exact repository pins. A missing or partial source remains a limitation, never a clean result.",
                        )
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    )
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
                                        product.license_or_access_status,
                                    ))
                                    .child(Self::render_fact(
                                        "Dependencies",
                                        product.dependency_policy_ref,
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
            .when_some(entropy_run_workbench, |this, run| {
                let counts = run.counts();
                let completed = counts.analyzed + counts.candidate + counts.skipped + counts.failed + counts.cancelled;
                let elapsed = entropy_elapsed_label(&run);
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
                            .child(Self::render_fact("Elapsed", elapsed))
                            .child(Self::render_fact(
                                "Usage exactness",
                                "Unavailable · selected model route did not report exact usage",
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
                Label::new("Coldcard benchmark")
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
            .when_some(coldcard_evidence, |this, workspace| {
                this.child(div().h_px().bg(cx.theme().colors().border))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(
                                        Label::new("Coldcard reproduction chain")
                                            .size(LabelSize::Small),
                                    )
                                    .child(
                                        Label::new("PRIVATE RUN · NON-REPORTABLE")
                                            .size(LabelSize::XSmall)
                                            .color(Color::Warning),
                                    ),
                            )
                            .child(
                                Label::new("Evidence ladder")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .children(workspace.ladder.into_iter().map(|rung| {
                                v_flex()
                                    .gap_1()
                                    .p_2()
                                    .border_1()
                                    .border_color(cx.theme().colors().border)
                                    .rounded_md()
                                    .child(
                                        h_flex()
                                            .justify_between()
                                            .child(
                                                Label::new(rung.rung.label())
                                                    .size(LabelSize::Small),
                                            )
                                            .child(
                                                Label::new(coldcard_rung_state_label(rung.state))
                                                    .size(LabelSize::XSmall)
                                                    .color(coldcard_rung_state_color(rung.state)),
                                            ),
                                    )
                                    .child(Self::render_fact(
                                        "Time",
                                        rung.time_to_rung.display_value(),
                                    ))
                                    .child(Self::render_fact(
                                        "Tokens",
                                        rung.tokens_to_rung.display_value(),
                                    ))
                                    .child(Self::render_fact("Verifier", rung.verifier_state))
                                    .child(Self::render_fact(
                                        "Evidence",
                                        if rung.evidence_refs.is_empty() {
                                            "Missing — not inferred downstream".to_string()
                                        } else {
                                            rung.evidence_refs.join(" · ")
                                        },
                                    ))
                                    .child(Self::render_fact(
                                        "Assumptions",
                                        rung.assumptions.join(" · "),
                                    ))
                                    .child(Self::render_fact(
                                        "Does not imply",
                                        rung.non_implications.join(" · "),
                                    ))
                            }))
                            .child(
                                Label::new("Source → artifact → generator trace")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .children(workspace.trace.into_iter().map(|step| {
                                Self::render_fact(
                                    format!("{:02} · {}", step.sequence, step.label),
                                    format!("{} · {}", step.verifier_state, step.evidence_ref),
                                )
                            }))
                            .child(
                                Label::new("Entropy assumptions and sensitivity")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .children(workspace.assumption_diffs.into_iter().map(|assumption| {
                                Self::render_fact(
                                    assumption.kind.label(),
                                    format!(
                                        "{} → {} · {}–{} bits",
                                        assumption.baseline,
                                        assumption.selected,
                                        assumption.lower_bound_bits,
                                        assumption.upper_bound_bits
                                    ),
                                )
                            }))
                            .child(
                                Label::new("Historical-chain scan")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(Self::render_fact(
                                "Boundary",
                                workspace.scan.boundary_ref.clone(),
                            ))
                            .child(Self::render_fact(
                                "Restart",
                                workspace.scan.restart_state.clone(),
                            ))
                            .child(Self::render_fact(
                                "Throughput",
                                aggregate_truth_label(
                                    workspace.scan.transactions_per_second,
                                    workspace.scan.throughput_exactness,
                                    "tx/s",
                                ),
                            ))
                            .child(Self::render_fact(
                                "Controls",
                                format!(
                                    "positive {} · negative {}",
                                    control_state_label(workspace.scan.positive_control),
                                    control_state_label(workspace.scan.negative_control)
                                ),
                            ))
                            .children(workspace.scan.ranges.into_iter().map(|range| {
                                Self::render_fact(
                                    format!("Range {}–{}", range.start_height, range.end_height),
                                    match (range.completed_height, range.checkpoint_ref) {
                                        (Some(height), Some(checkpoint)) => {
                                            format!("checkpoint {height} · {checkpoint}")
                                        }
                                        _ => "Not started".into(),
                                    },
                                )
                            }))
                            .children(workspace.scan.candidate_funnel.into_iter().map(|stage| {
                                Self::render_fact(
                                    stage.label,
                                    format!("{} · {}", stage.count, stage.source_receipt_ref),
                                )
                            }))
                            .children(workspace.scan.base_rates.into_iter().map(|rate| {
                                Self::render_fact(
                                    "False matches / million",
                                    format!("{} · {}", rate.matches_per_million, rate.stratum_ref),
                                )
                            }))
                            .when(!workspace.scan.missing_data_refs.is_empty(), |this| {
                                this.child(
                                    Label::new(format!(
                                        "Missing-data failure · {}",
                                        workspace.scan.missing_data_refs.join(" · ")
                                    ))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Error),
                                )
                            })
                            .child(Self::render_fact(
                                "Private transactions",
                                format!(
                                    "{} retained inside run",
                                    workspace.scan.public_transaction_refs.len()
                                ),
                            ))
                            .child(Self::render_fact(
                                "Candidate clusters",
                                format!(
                                    "{} non-reportable",
                                    workspace.scan.candidate_cluster_refs.len()
                                ),
                            ))
                            .child(
                                Label::new("Provenance graph health")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .children(workspace.graph_health.into_iter().map(|health| {
                                Self::render_fact(
                                    health.subject_ref,
                                    if health.complete {
                                        "Complete provenance".into()
                                    } else {
                                        format!(
                                            "Missing · {}",
                                            health.missing_provenance_refs.join(" · ")
                                        )
                                    },
                                )
                            }))
                            .child(
                                Label::new("Reconciliation ledger")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .children(workspace.reconciliation.into_iter().map(|item| {
                                Self::render_fact(
                                    item.metric_ref,
                                    format!(
                                        "{} · derived {} · published {}",
                                        reconciliation_status_label(item.status),
                                        item.derived_value.unwrap_or_else(|| "unavailable".into()),
                                        item.published_value
                                            .unwrap_or_else(|| "unavailable".into())
                                    ),
                                )
                            }))
                            .child(
                                Label::new("Append-only correction history")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .children(workspace.corrections.into_iter().map(|correction| {
                                Self::render_fact(
                                    format!(
                                        "#{:02} · {}",
                                        correction.sequence, correction.claim_ref
                                    ),
                                    format!(
                                        "{} → {} · {} affected",
                                        correction.prior_value,
                                        correction.corrected_value,
                                        correction.affected_projection_refs.len()
                                    ),
                                )
                            })),
                    )
            })
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
    }
}

fn entropy_run_phase_label(phase: EntropyRunPhase) -> &'static str {
    match phase {
        EntropyRunPhase::Ready => "Ready",
        EntropyRunPhase::Running => "Running",
        EntropyRunPhase::CancelRequested => "Cancellation requested",
        EntropyRunPhase::Completed => "Completed",
        EntropyRunPhase::CompletedWithLimitations => "Completed with limitations",
        EntropyRunPhase::Cancelled => "Cancelled",
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
        EntropyFileState::Cancelled => "Cancelled",
    }
}

fn entropy_file_state_color(state: EntropyFileState) -> Color {
    match state {
        EntropyFileState::Candidate => Color::Accent,
        EntropyFileState::Failed => Color::Error,
        EntropyFileState::Skipped | EntropyFileState::Cancelled => Color::Warning,
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

fn control_state_label(state: omega_forensics::ForensicControlState) -> &'static str {
    match state {
        omega_forensics::ForensicControlState::Passed => "passed",
        omega_forensics::ForensicControlState::Failed => "failed",
        omega_forensics::ForensicControlState::Missing => "missing",
    }
}

fn reconciliation_status_label(
    status: omega_forensics::ColdcardReconciliationStatus,
) -> &'static str {
    match status {
        omega_forensics::ColdcardReconciliationStatus::Match => "MATCH",
        omega_forensics::ColdcardReconciliationStatus::Drift => "DRIFT",
        omega_forensics::ColdcardReconciliationStatus::Unavailable => "UNAVAILABLE",
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

fn aggregate_truth_label(value: Option<u64>, exactness: ForensicExactness, unit: &str) -> String {
    match (value, exactness) {
        (None, ForensicExactness::Unavailable) => "unavailable".into(),
        (Some(value), ForensicExactness::Estimated) => format!("≈ {value} {unit}"),
        (Some(value), ForensicExactness::UpperBound) => format!("≤ {value} {unit}"),
        (Some(value), ForensicExactness::Exact) => format!("{value} {unit}"),
        _ => "invalid metric truth".into(),
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
        BROKER_NETWORK_POLICY_REF, CoverageSummaryProjection, ForensicBudgetProjection,
        ForensicCausalLink, ForensicEvidenceReceiptProjection, ForensicExactness,
        ForensicFindingProjection, ForensicHypothesisProjection, ForensicLifecycleStage,
        ForensicMetricTruth, ForensicWorkerPlacement, GCE_ADAPTER_REF, MANAGED_TARGET_REF,
        ManagedIsolation, ManagedProvider, ManagedTargetClass, ManagedWorkerProjection,
        PREFLIGHT_SCHEMA_V1, REVIEW_PROJECTION_SCHEMA_V1, RepositoryTargetProjection,
        WORKER_PLACEMENT_SCHEMA_V1, WorkerPlacementState,
    };
    use std::path::PathBuf;

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
    fn entropy_campaign_keeps_all_fifteen_rows_and_selected_project_state(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let binding = RepositoryBinding::new("repo", "worktree").expect("valid binding");
            let surface = cx.new(|cx| ForensicsWorkbenchSurface::new(&candidate(binding), cx));
            let catalog = EntropyProjectCatalog::wallet_entropy_v1().expect("wallet catalog");
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
                15
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
