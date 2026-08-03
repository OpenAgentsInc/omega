use anyhow::{Context as _, Result};
use db::kvp::KeyValueStore;
use gpui::{App, AppContext as _, Task};
use omega_forensics::{
    DEFAULT_ENTROPY_ANALYSIS_PROMPT, EntropyCampaignProjection, EntropyPromptSnapshot,
    EntropyRunProjection, EntropySourceInspection,
};
use omega_workbench_state::RepositoryBinding;
use serde::{Deserialize, Serialize};
use util::ResultExt as _;

use crate::account_scope::AccountScope;

const NAMESPACE: &str = "omega_entropy_forensics_v1";
const MAX_PROMPT_SNAPSHOTS: usize = 64;
const MAX_RESTORED_RUNS: usize = 16;
const MAX_RESTORED_CAMPAIGNS: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyForensicsRestoreState {
    pub draft_prompt: String,
    pub parent_prompt_ref: Option<String>,
    pub source_run_ref: Option<String>,
    pub prompt_snapshots: Vec<EntropyPromptSnapshot>,
    pub runs: Vec<EntropyRunProjection>,
    #[serde(default)]
    pub campaigns: Vec<EntropyCampaignProjection>,
    #[serde(default)]
    pub source_inspection: Option<EntropySourceInspection>,
    #[serde(default)]
    pub coldcard_case_rung: Option<String>,
    #[serde(default)]
    pub bench_view: Option<String>,
    #[serde(default)]
    pub lifecycle_selection: Option<String>,
    #[serde(default)]
    pub evidence_selection: Option<String>,
    #[serde(default)]
    pub model_run_ref: Option<String>,
    #[serde(default)]
    pub publication_scene: Option<String>,
}

impl Default for EntropyForensicsRestoreState {
    fn default() -> Self {
        Self {
            draft_prompt: DEFAULT_ENTROPY_ANALYSIS_PROMPT.into(),
            parent_prompt_ref: None,
            source_run_ref: None,
            prompt_snapshots: Vec::new(),
            runs: Vec::new(),
            campaigns: Vec::new(),
            source_inspection: None,
            coldcard_case_rung: None,
            bench_view: None,
            lifecycle_selection: None,
            evidence_selection: None,
            model_run_ref: None,
            publication_scene: None,
        }
    }
}

impl EntropyForensicsRestoreState {
    fn migrate_legacy_accounting(&mut self) -> Result<()> {
        for run in &mut self.runs {
            run.migrate_legacy_accounting()?;
        }
        for campaign in &mut self.campaigns {
            let mut campaign_migrated = false;
            for project in &mut campaign.projects {
                let Some(run) = project.run.as_mut() else {
                    continue;
                };
                if run.migrate_legacy_accounting()? {
                    campaign_migrated = true;
                    project.phase = match run.phase {
                        omega_forensics::EntropyRunPhase::Cancelled => {
                            omega_forensics::EntropyCampaignProjectPhase::Cancelled
                        }
                        omega_forensics::EntropyRunPhase::Failed => {
                            omega_forensics::EntropyCampaignProjectPhase::ProviderFailed
                        }
                        omega_forensics::EntropyRunPhase::Completed => {
                            omega_forensics::EntropyCampaignProjectPhase::Completed
                        }
                        _ => omega_forensics::EntropyCampaignProjectPhase::CompletedWithLimitations,
                    };
                }
            }
            if campaign_migrated
                && campaign.phase == omega_forensics::EntropyCampaignPhase::Completed
            {
                campaign.phase = omega_forensics::EntropyCampaignPhase::CompletedWithLimitations;
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        omega_forensics::entropy_prompt_digest(&self.draft_prompt)?;
        if self.prompt_snapshots.len() > MAX_PROMPT_SNAPSHOTS {
            anyhow::bail!("entropy prompt restore exceeds the snapshot bound");
        }
        if self.runs.len() > MAX_RESTORED_RUNS {
            anyhow::bail!("entropy prompt restore exceeds the run bound");
        }
        if self.campaigns.len() > MAX_RESTORED_CAMPAIGNS {
            anyhow::bail!("entropy prompt restore exceeds the campaign bound");
        }
        for snapshot in &self.prompt_snapshots {
            snapshot.validate()?;
        }
        for run in &self.runs {
            run.validate()?;
        }
        for campaign in &self.campaigns {
            campaign.validate()?;
        }
        if let Some(inspection) = &self.source_inspection {
            inspection.validate()?;
        }
        if self.coldcard_case_rung.as_deref().is_some_and(|rung| {
            !matches!(
                rung,
                "source"
                    | "artifact"
                    | "generator"
                    | "exploitability"
                    | "owned_fixture"
                    | "fingerprint"
                    | "entity"
                    | "unauthorized_movement"
                    | "identity"
            )
        }) {
            anyhow::bail!("Coldcard case restore contains an unknown evidence rung");
        }
        if self.bench_view.as_deref().is_some_and(|view| {
            !matches!(
                view,
                "entropy" | "case" | "lifecycle" | "evidence" | "models" | "publication"
            )
        }) {
            anyhow::bail!("forensics restore contains an unknown bench view");
        }
        if self
            .lifecycle_selection
            .as_deref()
            .is_some_and(|selection| {
                !matches!(
                    selection,
                    "summary" | "target" | "coverage" | "profile" | "runtime" | "cleanup"
                )
            })
        {
            anyhow::bail!("forensics restore contains an unknown lifecycle selection");
        }
        if self.evidence_selection.as_deref().is_some_and(|selection| {
            !matches!(
                selection,
                "findings" | "hypotheses" | "limitations" | "disputes" | "reconciliation"
            )
        }) {
            anyhow::bail!("forensics restore contains an unknown evidence selection");
        }
        if self
            .model_run_ref
            .as_deref()
            .is_some_and(|run_ref| run_ref.len() > 256 || !run_ref.starts_with("run."))
        {
            anyhow::bail!("forensics restore contains an invalid model run ref");
        }
        if self.publication_scene.as_deref().is_some_and(|scene| {
            !matches!(
                scene,
                "private_blocked"
                    | "denied"
                    | "awaiting_review"
                    | "rejected"
                    | "stale"
                    | "eligible_not_authorized"
            )
        }) {
            anyhow::bail!("forensics restore contains an unknown publication scene");
        }
        if let Some(parent_prompt_ref) = &self.parent_prompt_ref
            && !self
                .prompt_snapshots
                .iter()
                .any(|snapshot| &snapshot.prompt_ref == parent_prompt_ref)
        {
            anyhow::bail!("entropy draft parent is absent from restored snapshots");
        }
        if let Some(source_run_ref) = &self.source_run_ref
            && !self
                .runs
                .iter()
                .any(|run| &run.binding.run_ref == source_run_ref)
        {
            anyhow::bail!("entropy draft source run is absent from restored runs");
        }
        Ok(())
    }

    pub fn bounded(mut self) -> Self {
        if self.prompt_snapshots.len() > MAX_PROMPT_SNAPSHOTS {
            self.prompt_snapshots
                .drain(..self.prompt_snapshots.len() - MAX_PROMPT_SNAPSHOTS);
        }
        if self.runs.len() > MAX_RESTORED_RUNS {
            self.runs.drain(..self.runs.len() - MAX_RESTORED_RUNS);
        }
        if self.campaigns.len() > MAX_RESTORED_CAMPAIGNS {
            self.campaigns
                .drain(..self.campaigns.len() - MAX_RESTORED_CAMPAIGNS);
        }
        self
    }
}

pub fn read(binding: &RepositoryBinding, cx: &App) -> Option<EntropyForensicsRestoreState> {
    let scope = AccountScope::observed();
    let store = KeyValueStore::global(cx);
    let raw = store
        .scoped(&scope.namespace(NAMESPACE))
        .read(&scope.profile_key(&binding_key(binding)))
        .log_err()
        .flatten()?;
    let mut state: EntropyForensicsRestoreState = serde_json::from_str(&raw).log_err()?;
    state.migrate_legacy_accounting().log_err()?;
    state.validate().log_err()?;
    Some(state)
}

pub fn write(
    binding: RepositoryBinding,
    state: EntropyForensicsRestoreState,
    cx: &App,
) -> Task<Result<()>> {
    let scope = AccountScope::observed();
    let store = KeyValueStore::global(cx);
    let namespace = scope.namespace(NAMESPACE);
    let key = scope.profile_key(&binding_key(&binding));
    let state = state.bounded();
    let payload = match state
        .validate()
        .and_then(|()| serde_json::to_string(&state).context("encoding entropy prompt restore"))
    {
        Ok(payload) => payload,
        Err(error) => return Task::ready(Err(error)),
    };
    cx.background_spawn(async move {
        scope.ensure_current()?;
        store.scoped(&namespace).write(key, payload).await?;
        scope.ensure_current()?;
        Ok(())
    })
}

fn binding_key(binding: &RepositoryBinding) -> String {
    format!("{}:{}", binding.repository_id, binding.worktree_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_state_round_trip_preserves_prompt_lineage() {
        let parent = EntropyPromptSnapshot::new(
            "prompt.entropy.parent".into(),
            None,
            None,
            "Inspect entropy sources.".into(),
            "2026-08-02T19:00:00Z".into(),
        )
        .expect("parent prompt");
        let state = EntropyForensicsRestoreState {
            draft_prompt: "Inspect entropy sources and fallback generators.".into(),
            parent_prompt_ref: Some(parent.prompt_ref.clone()),
            source_run_ref: None,
            prompt_snapshots: vec![parent],
            runs: Vec::new(),
            campaigns: Vec::new(),
            source_inspection: None,
            coldcard_case_rung: Some("entity".into()),
            bench_view: Some("lifecycle".into()),
            lifecycle_selection: Some("cleanup".into()),
            evidence_selection: Some("disputes".into()),
            model_run_ref: Some("run.matrix.fixture".into()),
            publication_scene: Some("stale".into()),
        };
        state.validate().expect("valid restore state");
        let encoded = serde_json::to_string(&state).expect("encode restore state");
        let decoded: EntropyForensicsRestoreState =
            serde_json::from_str(&encoded).expect("decode restore state");
        assert_eq!(decoded, state);
    }
}
