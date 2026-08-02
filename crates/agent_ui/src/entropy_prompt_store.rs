use anyhow::{Context as _, Result};
use db::kvp::KeyValueStore;
use gpui::{App, AppContext as _, Task};
use omega_forensics::{
    DEFAULT_ENTROPY_ANALYSIS_PROMPT, EntropyCampaignProjection, EntropyPromptSnapshot,
    EntropyRunProjection,
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
        }
    }
}

impl EntropyForensicsRestoreState {
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
    let state: EntropyForensicsRestoreState = serde_json::from_str(&raw).log_err()?;
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
        };
        state.validate().expect("valid restore state");
        let encoded = serde_json::to_string(&state).expect("encode restore state");
        let decoded: EntropyForensicsRestoreState =
            serde_json::from_str(&encoded).expect("decode restore state");
        assert_eq!(decoded, state);
    }
}
