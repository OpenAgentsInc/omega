use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use gpui::{App, Global, SharedString};
use parking_lot::Mutex;
use remote::{RemoteConnectionOptions, remote_connection_identity};

use acp_thread::{AcpThread, ThreadStatus, ThreadTerminalStatus};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SupervisedThreadLifecycle {
    Running,
    WaitingForPerson,
    Failed,
    #[default]
    Completed,
    Cancelled,
}

impl SupervisedThreadLifecycle {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::WaitingForPerson => "Waiting for you",
            Self::Failed => "Failed",
            Self::Completed => "Completed",
            Self::Cancelled => "Cancelled",
        }
    }

    /// One-word status for UI chrome (tooltips). `OMEGA-DELTA-0189` law 3:
    /// statuses are colors/icons, never multi-word labels.
    pub const fn status_word(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::WaitingForPerson => "Waiting",
            Self::Failed => "Failed",
            Self::Completed => "Completed",
            Self::Cancelled => "Cancelled",
        }
    }

    pub const fn token(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingForPerson => "waiting_for_person",
            Self::Failed => "failed",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "running" => Some(Self::Running),
            "waiting_for_person" => Some(Self::WaitingForPerson),
            "failed" => Some(Self::Failed),
            "completed" => Some(Self::Completed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Failed | Self::Completed | Self::Cancelled)
    }

    pub const fn durable_terminal(self) -> Self {
        if self.is_terminal() {
            self
        } else {
            Self::Failed
        }
    }

    pub const fn terminal_status(self) -> ThreadTerminalStatus {
        match self.durable_terminal() {
            Self::Failed => ThreadTerminalStatus::Failed,
            Self::Completed => ThreadTerminalStatus::Completed,
            Self::Cancelled => ThreadTerminalStatus::Cancelled,
            Self::Running | Self::WaitingForPerson => ThreadTerminalStatus::Failed,
        }
    }
}

pub fn lifecycle_for_thread(thread: &AcpThread) -> SupervisedThreadLifecycle {
    if thread.is_waiting_for_confirmation() {
        SupervisedThreadLifecycle::WaitingForPerson
    } else if thread.status() == ThreadStatus::Generating {
        SupervisedThreadLifecycle::Running
    } else {
        match thread.terminal_status() {
            ThreadTerminalStatus::Failed => SupervisedThreadLifecycle::Failed,
            ThreadTerminalStatus::Completed => SupervisedThreadLifecycle::Completed,
            ThreadTerminalStatus::Cancelled => SupervisedThreadLifecycle::Cancelled,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadSupervisionSnapshot {
    pub thread_key: String,
    pub title: SharedString,
    pub executor: SharedString,
    pub lifecycle: SupervisedThreadLifecycle,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct WorktreeScope {
    remote_identity: Option<String>,
    path: PathBuf,
}

impl WorktreeScope {
    fn overlaps(&self, other: &Self) -> bool {
        self.remote_identity == other.remote_identity
            && (self.path.starts_with(&other.path) || other.path.starts_with(&self.path))
    }
}

/// The one path-overlap rule. `OMEGA-DELTA-0214` resolves occupancy before a
/// session starts and `OMEGA-DELTA-0181` claims it for the turn; both compare
/// roots through this, so a second rule cannot drift away from the first.
fn scopes_for(
    work_dirs: impl IntoIterator<Item = PathBuf>,
    remote_connection: Option<&RemoteConnectionOptions>,
) -> Vec<WorktreeScope> {
    let remote_identity = remote_connection
        .map(remote_connection_identity)
        .map(|identity| identity.persistence_key());
    work_dirs
        .into_iter()
        .map(|path| WorktreeScope {
            remote_identity: remote_identity.clone(),
            path: if remote_identity.is_none() {
                std::fs::canonicalize(&path)
                    .map(|path| normalize_path(&path))
                    .unwrap_or_else(|_| normalize_path(&path))
            } else {
                normalize_path(&path)
            },
        })
        .collect()
}

#[derive(Clone, Debug)]
struct WorktreeClaim {
    thread: ThreadSupervisionSnapshot,
    scopes: Vec<WorktreeScope>,
}

#[derive(Default)]
struct SupervisionState {
    next_claim_id: u64,
    snapshots: HashMap<String, ThreadSupervisionSnapshot>,
    claims: HashMap<u64, WorktreeClaim>,
    /// Every thread's bound roots, independent of whether it is mid-turn.
    /// `OMEGA-DELTA-0214` needs occupancy *before* the first send, when no
    /// turn-scoped claim exists yet, so binding outlives the claim.
    bindings: HashMap<String, Vec<WorktreeScope>>,
}

#[derive(Clone, Default)]
pub struct AgentSupervision {
    state: Arc<Mutex<SupervisionState>>,
}

impl Global for AgentSupervision {}

impl AgentSupervision {
    pub fn global(cx: &mut App) -> Self {
        if !cx.has_global::<Self>() {
            cx.set_global(Self::default());
        }
        cx.global::<Self>().clone()
    }

    pub fn set_snapshot(&self, snapshot: ThreadSupervisionSnapshot) {
        self.state
            .lock()
            .snapshots
            .insert(snapshot.thread_key.clone(), snapshot);
    }

    pub fn snapshot(&self, thread_key: &str) -> Option<ThreadSupervisionSnapshot> {
        self.state.lock().snapshots.get(thread_key).cloned()
    }

    pub fn remove_snapshot(&self, thread_key: &str) {
        let mut state = self.state.lock();
        state.snapshots.remove(thread_key);
        state.bindings.remove(thread_key);
    }

    /// Records which roots a thread is bound to, whether or not it is running.
    ///
    /// `OMEGA-DELTA-0214`. A turn-scoped claim only exists between send and
    /// turn end, so it cannot answer "is this root occupied?" at the moment a
    /// thread is created — which is the only moment at which the answer can
    /// still be acted on for an agent that fixes its cwd when the session
    /// starts.
    pub fn bind_roots(
        &self,
        thread_key: &str,
        work_dirs: impl IntoIterator<Item = PathBuf>,
        remote_connection: Option<&RemoteConnectionOptions>,
    ) {
        let scopes = scopes_for(work_dirs, remote_connection);
        let mut state = self.state.lock();
        if scopes.is_empty() {
            state.bindings.remove(thread_key);
        } else {
            state.bindings.insert(thread_key.to_string(), scopes);
        }
    }

    /// The live thread already occupying any of `work_dirs`, if there is one.
    ///
    /// `OMEGA-DELTA-0214`. "Live" is a held turn claim or a non-terminal
    /// lifecycle: a thread that is running, or waiting on a person mid-turn,
    /// is a write-capable occupant. A finished thread is not, so ordinary
    /// sequential work never provisions anything.
    pub fn occupant_for(
        &self,
        thread_key: &str,
        work_dirs: impl IntoIterator<Item = PathBuf>,
        remote_connection: Option<&RemoteConnectionOptions>,
    ) -> Option<WorktreeCollision> {
        let scopes = scopes_for(work_dirs, remote_connection);
        if scopes.is_empty() {
            return None;
        }
        let state = self.state.lock();

        let claimed = state.claims.values().filter_map(|claim| {
            (claim.thread.thread_key != thread_key).then(|| (&claim.thread, &claim.scopes))
        });
        let bound = state.bindings.iter().filter_map(|(key, bound_scopes)| {
            if key == thread_key {
                return None;
            }
            let snapshot = state.snapshots.get(key)?;
            (!snapshot.lifecycle.is_terminal()).then_some((snapshot, bound_scopes))
        });

        for (occupant, occupied_scopes) in claimed.chain(bound) {
            for occupied in occupied_scopes {
                if let Some(requested) = scopes.iter().find(|scope| scope.overlaps(occupied)) {
                    return Some(WorktreeCollision {
                        requested_path: requested.path.clone(),
                        occupied_path: occupied.path.clone(),
                        occupant: occupant.clone(),
                    });
                }
            }
        }
        None
    }

    pub fn claim(
        &self,
        thread: ThreadSupervisionSnapshot,
        work_dirs: impl IntoIterator<Item = PathBuf>,
        remote_connection: Option<&RemoteConnectionOptions>,
        allow_collision: bool,
    ) -> Result<WorktreeClaimToken, Box<WorktreeCollision>> {
        let scopes = scopes_for(work_dirs, remote_connection);

        let mut state = self.state.lock();
        if !allow_collision
            && let Some((existing, occupied_scope, requested_scope)) =
                state.claims.values().find_map(|existing| {
                    if existing.thread.thread_key == thread.thread_key {
                        return None;
                    }
                    existing.scopes.iter().find_map(|existing_scope| {
                        scopes
                            .iter()
                            .find(|scope| scope.overlaps(existing_scope))
                            .map(|scope| (existing, existing_scope.clone(), scope.clone()))
                    })
                })
        {
            return Err(Box::new(WorktreeCollision {
                requested_path: requested_scope.path,
                occupied_path: occupied_scope.path,
                occupant: existing.thread.clone(),
            }));
        }

        state.next_claim_id = state.next_claim_id.saturating_add(1);
        let claim_id = state.next_claim_id;
        if !scopes.is_empty() {
            state
                .bindings
                .insert(thread.thread_key.clone(), scopes.clone());
        }
        state.claims.insert(
            claim_id,
            WorktreeClaim {
                thread: thread.clone(),
                scopes,
            },
        );
        state.snapshots.insert(thread.thread_key.clone(), thread);
        Ok(WorktreeClaimToken {
            claim_id,
            state: self.state.clone(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeCollision {
    pub requested_path: PathBuf,
    pub occupied_path: PathBuf,
    pub occupant: ThreadSupervisionSnapshot,
}

pub struct WorktreeClaimToken {
    claim_id: u64,
    state: Arc<Mutex<SupervisionState>>,
}

impl Drop for WorktreeClaimToken {
    fn drop(&mut self) {
        self.state.lock().claims.remove(&self.claim_id);
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(thread_key: &str) -> ThreadSupervisionSnapshot {
        ThreadSupervisionSnapshot {
            thread_key: thread_key.into(),
            title: format!("Thread {thread_key}").into(),
            executor: "Codex".into(),
            lifecycle: SupervisedThreadLifecycle::Running,
        }
    }

    #[test]
    fn claims_conflict_only_on_the_same_normalized_scope() {
        let supervision = AgentSupervision::default();
        let first = supervision
            .claim(
                snapshot("a"),
                [PathBuf::from("/repo/./src/..")],
                None,
                false,
            )
            .expect("first claim");

        let collision =
            match supervision.claim(snapshot("b"), [PathBuf::from("/repo")], None, false) {
                Ok(_) => panic!("same normalized path must collide"),
                Err(collision) => collision,
            };
        assert_eq!(collision.requested_path, PathBuf::from("/repo"));
        assert_eq!(collision.occupied_path, PathBuf::from("/repo"));
        assert_eq!(collision.occupant.thread_key, "a");

        supervision
            .claim(snapshot("c"), [PathBuf::from("/other")], None, false)
            .expect("different path");
        drop(first);
        supervision
            .claim(snapshot("b"), [PathBuf::from("/repo")], None, false)
            .expect("released path");
    }

    /// `OMEGA-DELTA-0214`. Occupancy has to be answerable at thread creation,
    /// when the occupying thread holds no turn claim.
    #[test]
    fn a_bound_live_thread_occupies_its_root_without_holding_a_claim() {
        let supervision = AgentSupervision::default();
        supervision.set_snapshot(snapshot("a"));
        supervision.bind_roots("a", [PathBuf::from("/repo")], None);

        let collision = supervision
            .occupant_for("b", [PathBuf::from("/repo/crates")], None)
            .expect("a live thread bound to the parent root occupies it");
        assert_eq!(collision.occupant.thread_key, "a");
        assert_eq!(collision.occupied_path, PathBuf::from("/repo"));
        assert_eq!(collision.requested_path, PathBuf::from("/repo/crates"));

        assert!(
            supervision
                .occupant_for("a", [PathBuf::from("/repo")], None)
                .is_none(),
            "a thread never collides with itself"
        );
        assert!(
            supervision
                .occupant_for("b", [PathBuf::from("/elsewhere")], None)
                .is_none(),
            "a disjoint root is not occupied"
        );
    }

    /// `OMEGA-DELTA-0214`. Sequential work must not provision anything: once a
    /// thread is finished it is no longer a write-capable occupant.
    #[test]
    fn a_terminal_thread_does_not_occupy_its_root() {
        let supervision = AgentSupervision::default();
        for lifecycle in [
            SupervisedThreadLifecycle::Completed,
            SupervisedThreadLifecycle::Failed,
            SupervisedThreadLifecycle::Cancelled,
        ] {
            supervision.set_snapshot(ThreadSupervisionSnapshot {
                lifecycle,
                ..snapshot("a")
            });
            supervision.bind_roots("a", [PathBuf::from("/repo")], None);
            assert!(
                supervision
                    .occupant_for("b", [PathBuf::from("/repo")], None)
                    .is_none(),
                "{lifecycle:?} must not occupy a root"
            );
        }

        for lifecycle in [
            SupervisedThreadLifecycle::Running,
            SupervisedThreadLifecycle::WaitingForPerson,
        ] {
            supervision.set_snapshot(ThreadSupervisionSnapshot {
                lifecycle,
                ..snapshot("a")
            });
            supervision.bind_roots("a", [PathBuf::from("/repo")], None);
            assert!(
                supervision
                    .occupant_for("b", [PathBuf::from("/repo")], None)
                    .is_some(),
                "{lifecycle:?} must occupy a root"
            );
        }
    }

    /// `OMEGA-DELTA-0214`. A binding must not outlive its thread, or a closed
    /// thread would isolate every later thread forever.
    #[test]
    fn removing_a_snapshot_releases_its_binding() {
        let supervision = AgentSupervision::default();
        supervision.set_snapshot(snapshot("a"));
        supervision.bind_roots("a", [PathBuf::from("/repo")], None);
        assert!(
            supervision
                .occupant_for("b", [PathBuf::from("/repo")], None)
                .is_some()
        );
        supervision.remove_snapshot("a");
        assert!(
            supervision
                .occupant_for("b", [PathBuf::from("/repo")], None)
                .is_none()
        );
    }

    #[test]
    fn explicit_override_is_tokenized_per_claim() {
        let supervision = AgentSupervision::default();
        let first = supervision
            .claim(snapshot("a"), [PathBuf::from("/repo")], None, false)
            .expect("first claim");
        let second = supervision
            .claim(snapshot("b"), [PathBuf::from("/repo")], None, true)
            .expect("explicit override");

        drop(first);
        assert!(
            supervision
                .claim(snapshot("c"), [PathBuf::from("/repo")], None, false)
                .is_err(),
            "releasing one override must not release the other"
        );
        drop(second);
        supervision
            .claim(snapshot("c"), [PathBuf::from("/repo")], None, false)
            .expect("all claims released");
    }

    #[test]
    fn any_overlap_in_multi_root_claims_conflicts() {
        let supervision = AgentSupervision::default();
        let _first = supervision
            .claim(
                snapshot("a"),
                [PathBuf::from("/repo-a"), PathBuf::from("/repo-b")],
                None,
                false,
            )
            .expect("first multi-root claim");
        assert!(
            supervision
                .claim(
                    snapshot("b"),
                    [PathBuf::from("/repo-c"), PathBuf::from("/repo-b")],
                    None,
                    false,
                )
                .is_err()
        );
    }

    #[test]
    fn ancestor_and_descendant_roots_overlap_but_siblings_do_not() {
        let supervision = AgentSupervision::default();
        let _first = supervision
            .claim(snapshot("a"), [PathBuf::from("/repo")], None, false)
            .expect("first claim");

        let collision = match supervision.claim(
            snapshot("b"),
            [PathBuf::from("/repo/subcrate")],
            None,
            false,
        ) {
            Ok(_) => panic!("a descendant root must collide with its ancestor"),
            Err(collision) => collision,
        };
        assert_eq!(collision.occupied_path, PathBuf::from("/repo"));
        assert_eq!(collision.requested_path, PathBuf::from("/repo/subcrate"));

        supervision
            .claim(snapshot("c"), [PathBuf::from("/repo-a")], None, false)
            .expect("component-aware siblings must remain separate");
    }

    #[test]
    fn same_remote_path_on_different_hosts_does_not_collide() {
        let supervision = AgentSupervision::default();
        let first_remote = RemoteConnectionOptions::Ssh(remote::SshConnectionOptions {
            host: "first.example.com".into(),
            ..Default::default()
        });
        let second_remote = RemoteConnectionOptions::Ssh(remote::SshConnectionOptions {
            host: "second.example.com".into(),
            ..Default::default()
        });
        let _first = supervision
            .claim(
                snapshot("a"),
                [PathBuf::from("/repo")],
                Some(&first_remote),
                false,
            )
            .expect("first remote claim");
        supervision
            .claim(
                snapshot("b"),
                [PathBuf::from("/repo")],
                Some(&second_remote),
                false,
            )
            .expect("different remote host");
    }

    #[test]
    fn nested_remote_roots_on_the_same_host_collide() {
        let supervision = AgentSupervision::default();
        let remote = RemoteConnectionOptions::Ssh(remote::SshConnectionOptions {
            host: "example.com".into(),
            ..Default::default()
        });
        let _first = supervision
            .claim(
                snapshot("a"),
                [PathBuf::from("/repo")],
                Some(&remote),
                false,
            )
            .expect("first remote claim");
        assert!(
            supervision
                .claim(
                    snapshot("b"),
                    [PathBuf::from("/repo/subcrate")],
                    Some(&remote),
                    false,
                )
                .is_err(),
            "nested roots on one remote host must collide"
        );
    }

    #[cfg(unix)]
    #[test]
    fn local_symlink_aliases_resolve_to_one_claim() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory");
        let real = directory.path().join("real");
        let alias = directory.path().join("alias");
        std::fs::create_dir(&real).expect("real directory");
        symlink(&real, &alias).expect("symlink alias");

        let supervision = AgentSupervision::default();
        let _first = supervision
            .claim(snapshot("a"), [real], None, false)
            .expect("real path claim");
        assert!(
            supervision
                .claim(snapshot("b"), [alias], None, false)
                .is_err()
        );
    }
}
