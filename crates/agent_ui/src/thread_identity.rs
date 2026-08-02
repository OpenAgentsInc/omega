use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use gpui::SharedString;
use omega_workbench_state::RepositoryBinding;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BranchIdentity {
    Branch(SharedString),
    Detached(SharedString),
    Unborn,
    NoGit,
}

impl BranchIdentity {
    pub fn from_git(branch: Option<&str>, head_commit: Option<&str>) -> Self {
        match (branch, head_commit) {
            (Some(branch), Some(_)) => Self::Branch(branch.to_string().into()),
            (Some(_), None) | (None, None) => Self::Unborn,
            (None, Some(commit)) => {
                Self::Detached(commit.chars().take(8).collect::<String>().into())
            }
        }
    }

    pub fn label(&self) -> SharedString {
        match self {
            Self::Branch(name) => name.clone(),
            Self::Detached(commit) => format!("Detached at {commit}").into(),
            Self::Unborn => "No commits".into(),
            Self::NoGit => "No Git".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GitIdentitySummary {
    pub dirty_files: usize,
    pub conflicts: usize,
    pub ahead: usize,
    pub behind: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityPhase {
    NoProject,
    Loading,
    Ready,
    Stale,
    Offline,
    Reconnecting,
    Missing,
    Error(SharedString),
    Inconsistent(SharedString),
}

impl IdentityPhase {
    pub fn label(&self) -> Option<SharedString> {
        match self {
            Self::NoProject | Self::Ready => None,
            Self::Loading => Some("Loading repository identity".into()),
            Self::Stale => Some("Repository identity may be stale".into()),
            Self::Offline => Some("Repository identity is offline".into()),
            Self::Reconnecting => Some("Reconnecting repository identity".into()),
            Self::Missing => Some("The selected worktree is missing".into()),
            Self::Error(error) => Some(error.clone()),
            Self::Inconsistent(error) => Some(error.clone()),
        }
    }

    fn preserves_last_known_identity(&self) -> bool {
        matches!(
            self,
            Self::Loading
                | Self::Stale
                | Self::Offline
                | Self::Reconnecting
                | Self::Error(_)
                | Self::Inconsistent(_)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadIdentityCandidate {
    pub binding: RepositoryBinding,
    pub git_repository_id: Option<u64>,
    pub project_name: SharedString,
    pub repository_name: SharedString,
    pub worktree_name: SharedString,
    pub worktree_abs_path: PathBuf,
    pub worktree_path: SharedString,
    pub remote_url: Option<SharedString>,
    pub head_commit: Option<SharedString>,
    pub branch: BranchIdentity,
    pub git: GitIdentitySummary,
    pub source_revision: u64,
}

impl ThreadIdentityCandidate {
    pub fn accessible_label(&self) -> SharedString {
        format!(
            "Project {}, repository {}, worktree {} at {}, {}",
            self.project_name,
            self.repository_name,
            self.worktree_name,
            self.worktree_path,
            self.branch.label()
        )
        .into()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadIdentityObservation {
    pub revision: u64,
    pub phase: IdentityPhase,
    pub candidates: Vec<ThreadIdentityCandidate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadIdentityState {
    pub observation_revision: u64,
    pub selection_revision: u64,
    pub phase: IdentityPhase,
    pub candidates: Vec<ThreadIdentityCandidate>,
    pub selected: Option<ThreadIdentityCandidate>,
}

impl ThreadIdentityState {
    pub fn binding(&self) -> Option<&RepositoryBinding> {
        if matches!(
            self.phase,
            IdentityPhase::NoProject | IdentityPhase::Missing
        ) {
            None
        } else {
            self.selected.as_ref().and_then(|selected| {
                if matches!(
                    self.phase,
                    IdentityPhase::Error(_) | IdentityPhase::Inconsistent(_)
                ) && !self
                    .candidates
                    .iter()
                    .any(|candidate| candidate.binding == selected.binding)
                {
                    None
                } else {
                    Some(&selected.binding)
                }
            })
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentitySyncEffect {
    Applied,
    OlderObservationIgnored,
    SelectionMissing,
}

#[derive(Default)]
pub struct ThreadIdentityProjection {
    active_thread_id: Option<String>,
    threads: BTreeMap<String, ThreadIdentityState>,
}

impl ThreadIdentityProjection {
    pub fn active_thread_id(&self) -> Option<&str> {
        self.active_thread_id.as_deref()
    }

    pub fn active(&self) -> Option<&ThreadIdentityState> {
        self.threads.get(self.active_thread_id.as_deref()?)
    }

    pub fn sync_active_thread(
        &mut self,
        thread_id: Option<String>,
        mut observation: ThreadIdentityObservation,
    ) -> IdentitySyncEffect {
        let Some(thread_id) = thread_id else {
            self.active_thread_id = None;
            return IdentitySyncEffect::Applied;
        };
        let mut bindings = BTreeSet::new();
        observation
            .candidates
            .retain(|candidate| bindings.insert(candidate.binding.clone()));

        let state = self.threads.entry(thread_id.clone()).or_insert_with(|| {
            let selected = observation.candidates.first().cloned();
            ThreadIdentityState {
                observation_revision: observation.revision,
                selection_revision: 0,
                phase: observation.phase.clone(),
                candidates: observation.candidates.clone(),
                selected,
            }
        });
        self.active_thread_id = Some(thread_id);

        if observation.revision < state.observation_revision {
            return IdentitySyncEffect::OlderObservationIgnored;
        }
        if observation.revision == state.observation_revision
            && observation.phase == state.phase
            && observation.candidates == state.candidates
        {
            return IdentitySyncEffect::Applied;
        }

        let previous_selected = state.selected.clone();
        let selected = previous_selected.as_ref().and_then(|previous| {
            observation
                .candidates
                .iter()
                .find(|candidate| candidate.binding == previous.binding)
                .or_else(|| {
                    observation.candidates.iter().find(|candidate| {
                        candidate.worktree_abs_path == previous.worktree_abs_path
                            && candidate.repository_name == previous.repository_name
                    })
                })
                .or_else(|| {
                    observation.candidates.iter().find(|candidate| {
                        candidate.worktree_abs_path == previous.worktree_abs_path
                            && (matches!(previous.branch, BranchIdentity::NoGit)
                                || previous.repository_name == "No Git"
                                || previous.binding.worktree_id == candidate.binding.worktree_id)
                    })
                })
                .cloned()
        });
        let selection_missing = previous_selected.is_some()
            && selected.is_none()
            && !observation.phase.preserves_last_known_identity();

        state.observation_revision = observation.revision;
        state.candidates = observation.candidates;
        if selection_missing {
            if matches!(state.phase, IdentityPhase::Missing) {
                return IdentitySyncEffect::Applied;
            }
            state.phase = IdentityPhase::Missing;
            state.selected = previous_selected;
            state.selection_revision = state.selection_revision.saturating_add(1);
            IdentitySyncEffect::SelectionMissing
        } else {
            state.phase = observation.phase;
            state.selected = selected.or_else(|| {
                if state.phase.preserves_last_known_identity() && previous_selected.is_some() {
                    previous_selected
                } else {
                    state.candidates.first().cloned()
                }
            });
            IdentitySyncEffect::Applied
        }
    }

    pub fn select(
        &mut self,
        expected_observation_revision: u64,
        binding: &RepositoryBinding,
    ) -> anyhow::Result<bool> {
        let thread_id = self
            .active_thread_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no active thread identity"))?;
        let state = self
            .threads
            .get_mut(thread_id)
            .ok_or_else(|| anyhow::anyhow!("active thread identity is missing"))?;
        if state.observation_revision != expected_observation_revision {
            anyhow::bail!(
                "identity observation changed from revision {expected_observation_revision} to {}",
                state.observation_revision
            );
        }
        let candidate = state
            .candidates
            .iter()
            .find(|candidate| &candidate.binding == binding)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("selected repository/worktree is unavailable"))?;
        if state.selected.as_ref() == Some(&candidate) {
            return Ok(false);
        }
        state.selected = Some(candidate);
        state.phase = IdentityPhase::Ready;
        state.selection_revision = state.selection_revision.saturating_add(1);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(repository: &str, worktree: &str) -> ThreadIdentityCandidate {
        ThreadIdentityCandidate {
            binding: RepositoryBinding::new(repository, worktree).expect("valid fixture binding"),
            git_repository_id: None,
            project_name: "Omega".into(),
            repository_name: repository.to_string().into(),
            worktree_name: worktree.to_string().into(),
            worktree_abs_path: PathBuf::from(format!("/work/{worktree}")),
            worktree_path: format!("/work/{worktree}").into(),
            remote_url: Some(format!("https://github.com/OpenAgentsInc/{repository}.git").into()),
            head_commit: Some("0123456789abcdef0123456789abcdef01234567".into()),
            branch: BranchIdentity::Branch("main".into()),
            git: GitIdentitySummary::default(),
            source_revision: 1,
        }
    }

    fn observation(
        revision: u64,
        phase: IdentityPhase,
        candidates: Vec<ThreadIdentityCandidate>,
    ) -> ThreadIdentityObservation {
        ThreadIdentityObservation {
            revision,
            phase,
            candidates,
        }
    }

    #[test]
    fn switching_threads_never_exposes_the_previous_identity() {
        let mut projection = ThreadIdentityProjection::default();
        projection.sync_active_thread(
            Some("thread-a".into()),
            observation(1, IdentityPhase::Ready, vec![candidate("repo-a", "tree-a")]),
        );
        projection.sync_active_thread(
            Some("thread-b".into()),
            observation(1, IdentityPhase::Loading, Vec::new()),
        );

        let state = projection.active().expect("active thread state");
        assert_eq!(projection.active_thread_id(), Some("thread-b"));
        assert_eq!(state.phase, IdentityPhase::Loading);
        assert!(state.selected.is_none());
    }

    #[test]
    fn removed_selection_is_missing_instead_of_silently_rebound() {
        let mut projection = ThreadIdentityProjection::default();
        let first = candidate("repo-a", "tree-a");
        let second = candidate("repo-b", "tree-b");
        projection.sync_active_thread(
            Some("thread".into()),
            observation(1, IdentityPhase::Ready, vec![first.clone(), second.clone()]),
        );
        projection
            .select(1, &second.binding)
            .expect("select second candidate");

        let effect = projection.sync_active_thread(
            Some("thread".into()),
            observation(2, IdentityPhase::Ready, vec![first]),
        );

        assert_eq!(effect, IdentitySyncEffect::SelectionMissing);
        let state = projection.active().expect("active state");
        assert_eq!(state.phase, IdentityPhase::Missing);
        assert_eq!(
            state.selected.as_ref().map(|candidate| &candidate.binding),
            Some(&second.binding)
        );
        assert_eq!(state.binding(), None);

        projection.sync_active_thread(
            Some("thread".into()),
            observation(
                3,
                IdentityPhase::Error("replacement failed".into()),
                vec![candidate("repo-a", "tree-a")],
            ),
        );
        let state = projection.active().expect("failed recovery state");
        assert_eq!(state.binding(), None);
        assert_eq!(
            state.selected.as_ref().map(|candidate| &candidate.binding),
            Some(&second.binding),
            "a failed recovery may retain the missing label but not revive its authority"
        );
    }

    #[test]
    fn refreshed_runtime_ids_preserve_selection_for_the_same_worktree() {
        let mut projection = ThreadIdentityProjection::default();
        let selected = candidate("repo", "tree");
        projection.sync_active_thread(
            Some("thread".into()),
            observation(1, IdentityPhase::Ready, vec![selected.clone()]),
        );

        let mut refreshed = selected;
        refreshed.binding = RepositoryBinding::new("refreshed-repository", "refreshed-worktree")
            .expect("valid refreshed binding");
        let effect = projection.sync_active_thread(
            Some("thread".into()),
            observation(2, IdentityPhase::Ready, vec![refreshed.clone()]),
        );

        assert_eq!(effect, IdentitySyncEffect::Applied);
        let state = projection.active().expect("active state");
        assert_eq!(state.phase, IdentityPhase::Ready);
        assert_eq!(state.selected, Some(refreshed.clone()));
        assert_eq!(state.binding(), Some(&refreshed.binding));
    }

    #[test]
    fn initial_no_git_fallback_upgrades_to_scanned_git_repository_without_missing_phase() {
        let mut projection = ThreadIdentityProjection::default();
        let mut fallback = candidate("project-worktree-0", "worktree-0");
        fallback.repository_name = "No Git".into();
        fallback.branch = BranchIdentity::NoGit;
        projection.sync_active_thread(
            Some("thread".into()),
            observation(1, IdentityPhase::Ready, vec![fallback]),
        );

        let mut scanned = candidate("git-repository-1234", "worktree-0");
        scanned.repository_name = "omega".into();
        let effect = projection.sync_active_thread(
            Some("thread".into()),
            observation(2, IdentityPhase::Ready, vec![scanned.clone()]),
        );

        assert_eq!(effect, IdentitySyncEffect::Applied);
        let state = projection.active().expect("active state");
        assert_eq!(state.phase, IdentityPhase::Ready);
        assert_eq!(state.selected, Some(scanned.clone()));
        assert_eq!(state.binding(), Some(&scanned.binding));
    }

    #[test]
    fn stale_observation_cannot_replace_a_newer_selection() {
        let mut projection = ThreadIdentityProjection::default();
        let first = candidate("repo-a", "tree-a");
        let second = candidate("repo-b", "tree-b");
        projection.sync_active_thread(
            Some("thread".into()),
            observation(4, IdentityPhase::Ready, vec![first, second.clone()]),
        );
        projection
            .select(4, &second.binding)
            .expect("select second candidate");

        let effect = projection.sync_active_thread(
            Some("thread".into()),
            observation(3, IdentityPhase::Ready, vec![candidate("repo-a", "tree-a")]),
        );

        assert_eq!(effect, IdentitySyncEffect::OlderObservationIgnored);
        assert_eq!(
            projection
                .active()
                .and_then(|state| state.selected.as_ref())
                .map(|candidate| &candidate.binding),
            Some(&second.binding)
        );
    }

    #[test]
    fn transient_phase_preserves_only_the_same_threads_last_known_identity() {
        let mut projection = ThreadIdentityProjection::default();
        let selected = candidate("repo", "tree");
        projection.sync_active_thread(
            Some("thread".into()),
            observation(1, IdentityPhase::Ready, vec![selected.clone()]),
        );
        projection.sync_active_thread(
            Some("thread".into()),
            observation(2, IdentityPhase::Offline, Vec::new()),
        );

        let state = projection.active().expect("active state");
        assert_eq!(state.phase, IdentityPhase::Offline);
        assert_eq!(state.selected.as_ref(), Some(&selected));
        assert_eq!(state.binding(), Some(&selected.binding));
    }

    #[test]
    fn first_offline_observation_can_establish_the_threads_identity() {
        let mut projection = ThreadIdentityProjection::default();
        projection.sync_active_thread(
            Some("thread-a".into()),
            observation(
                1,
                IdentityPhase::Offline,
                vec![candidate("repo-a", "tree-a")],
            ),
        );

        let state = projection.active().expect("active thread state");
        assert_eq!(
            state.selected.as_ref().map(|selected| &selected.binding),
            Some(&candidate("repo-a", "tree-a").binding)
        );
        assert_eq!(state.phase, IdentityPhase::Offline);
    }

    #[test]
    fn git_head_states_are_distinct_and_typed() {
        assert_eq!(
            BranchIdentity::from_git(Some("main"), Some("abcdef")),
            BranchIdentity::Branch("main".into())
        );
        assert_eq!(
            BranchIdentity::from_git(None, Some("1234567890")),
            BranchIdentity::Detached("12345678".into())
        );
        assert_eq!(
            BranchIdentity::from_git(Some("main"), None),
            BranchIdentity::Unborn
        );
        assert_eq!(BranchIdentity::from_git(None, None), BranchIdentity::Unborn);
        assert_ne!(BranchIdentity::Unborn, BranchIdentity::NoGit);
        assert_eq!(BranchIdentity::Unborn.label().as_ref(), "No commits");
    }
}
