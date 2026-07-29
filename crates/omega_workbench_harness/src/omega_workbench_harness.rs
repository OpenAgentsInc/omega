use anyhow::{Context as _, Result, anyhow, bail};
#[cfg(feature = "gpui-support")]
use gpui::{
    AnyWindowHandle, Bounds, DebugRenderSnapshot, Pixels, VisualTestAppContext, VisualTestContext,
};
use image::{Rgba, RgbaImage};
use omega_workbench_conformance as conformance;
use omega_workbench_state as projection;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
#[cfg(feature = "gpui-support")]
use std::collections::BTreeMap;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub const PROOF_RECEIPT_SCHEMA: &str = "openagents.omega.workbench-proof.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenePhase {
    Recording,
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelPolicy {
    pub minimum_match: f64,
    pub channel_tolerance: u8,
    pub rationale: &'static str,
}

impl PixelPolicy {
    pub const fn new(minimum_match: f64, channel_tolerance: u8, rationale: &'static str) -> Self {
        Self {
            minimum_match,
            channel_tolerance,
            rationale,
        }
    }

    pub fn validate(self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.minimum_match) {
            bail!(
                "pixel match threshold must be between zero and one, got {}",
                self.minimum_match
            );
        }
        if self.rationale.trim().is_empty() {
            bail!("pixel policy rationale must not be empty");
        }
        Ok(())
    }
}

pub const APPLE_SILICON_METAL_POLICY: PixelPolicy = PixelPolicy::new(
    0.99,
    2,
    "Apple Silicon Metal baselines tolerate minor font and color rounding variance",
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewportFixture {
    pub width: u32,
    pub height: u32,
    pub scale_milli: u32,
}

impl ViewportFixture {
    pub const fn new(width: u32, height: u32, scale_milli: u32) -> Self {
        Self {
            width,
            height,
            scale_milli,
        }
    }

    fn validate(self) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            bail!("scene viewport dimensions must be non-zero");
        }
        if self.scale_milli == 0 {
            bail!("scene scale must be non-zero");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkSurfaceId {
    Files,
    Search,
    Review,
    Git,
    Terminal,
    Plan,
}

impl WorkSurfaceId {
    pub const ALL: [Self; 6] = [
        Self::Files,
        Self::Search,
        Self::Review,
        Self::Git,
        Self::Terminal,
        Self::Plan,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Search => "search",
            Self::Review => "review",
            Self::Git => "git",
            Self::Terminal => "terminal",
            Self::Plan => "plan",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Search => "Search",
            Self::Review => "Review",
            Self::Git => "Git",
            Self::Terminal => "Terminal",
            Self::Plan => "Plan",
        }
    }

    pub const fn rail_selector(self) -> &'static str {
        match self {
            Self::Files => "omega.workbench.control.rail.files",
            Self::Search => "omega.workbench.control.rail.search",
            Self::Review => "omega.workbench.control.rail.review",
            Self::Git => "omega.workbench.control.rail.git",
            Self::Terminal => "omega.workbench.control.rail.terminal",
            Self::Plan => "omega.workbench.control.rail.plan",
        }
    }

    pub const fn surface_selector(self) -> &'static str {
        match self {
            Self::Files => "omega.workbench.surface.files",
            Self::Search => "omega.workbench.surface.search",
            Self::Review => "omega.workbench.surface.review",
            Self::Git => "omega.workbench.surface.git",
            Self::Terminal => "omega.workbench.surface.terminal",
            Self::Plan => "omega.workbench.surface.plan",
        }
    }

    pub const fn badge_selector(self) -> &'static str {
        match self {
            Self::Files => "omega.workbench.badge.files",
            Self::Search => "omega.workbench.badge.search",
            Self::Review => "omega.workbench.badge.review",
            Self::Git => "omega.workbench.badge.git",
            Self::Terminal => "omega.workbench.badge.terminal",
            Self::Plan => "omega.workbench.badge.plan",
        }
    }

    pub const fn requires_binding(self) -> bool {
        matches!(
            self,
            Self::Files | Self::Search | Self::Review | Self::Git | Self::Terminal
        )
    }
}

pub const DOCK_OPEN_CONTROL: &str = "omega.workbench.control.dock.open";
pub const DOCK_COLLAPSE_CONTROL: &str = "omega.workbench.control.dock.collapse";
pub const DOCK_RESIZE_CONTROL: &str = "omega.workbench.control.dock.resize";

pub trait WorkbenchInteractionBackend {
    fn activate_selector(&mut self, selector: &str) -> Result<()>;

    fn restart(&mut self) -> Result<()>;
}

pub struct WorkbenchInteractionDriver<Backend> {
    backend: Backend,
}

impl<Backend: WorkbenchInteractionBackend> WorkbenchInteractionDriver<Backend> {
    pub fn new(backend: Backend) -> Self {
        Self { backend }
    }

    pub fn select_rail_item(&mut self, surface: WorkSurfaceId) -> Result<()> {
        self.backend.activate_selector(surface.rail_selector())
    }

    pub fn open_dock(&mut self) -> Result<()> {
        self.backend.activate_selector(DOCK_OPEN_CONTROL)
    }

    pub fn collapse_dock(&mut self) -> Result<()> {
        self.backend.activate_selector(DOCK_COLLAPSE_CONTROL)
    }

    pub fn switch_thread(&mut self, thread_id: &str) -> Result<()> {
        self.backend.activate_selector(&control_selector(
            "omega.workbench.control.thread",
            thread_id,
        )?)
    }

    pub fn change_worktree(&mut self, worktree_id: &str) -> Result<()> {
        self.backend.activate_selector(&control_selector(
            "omega.workbench.control.worktree",
            worktree_id,
        )?)
    }

    pub fn focus_surface(&mut self, surface: WorkSurfaceId) -> Result<()> {
        self.backend.activate_selector(surface.surface_selector())
    }

    pub fn restart(&mut self) -> Result<()> {
        self.backend.restart()
    }

    pub fn into_backend(self) -> Backend {
        self.backend
    }
}

fn control_selector(prefix: &str, fixture_id: &str) -> Result<String> {
    if fixture_id.is_empty()
        || !fixture_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!(
            "fixture ID {fixture_id:?} must use only ASCII letters, numbers, hyphens, or underscores"
        );
    }
    Ok(format!("{prefix}.{fixture_id}"))
}

#[cfg(feature = "gpui-support")]
impl WorkbenchInteractionBackend for VisualTestContext {
    fn activate_selector(&mut self, selector: &str) -> Result<()> {
        self.simulate_click_selector(selector)
    }

    fn restart(&mut self) -> Result<()> {
        bail!("a cold restart requires the external proof command and a second process")
    }
}

#[cfg(feature = "gpui-support")]
pub struct MetalInteractionBackend<'a> {
    context: &'a mut VisualTestAppContext,
    window: AnyWindowHandle,
}

#[cfg(feature = "gpui-support")]
impl<'a> MetalInteractionBackend<'a> {
    pub fn new(context: &'a mut VisualTestAppContext, window: AnyWindowHandle) -> Self {
        Self { context, window }
    }
}

#[cfg(feature = "gpui-support")]
impl WorkbenchInteractionBackend for MetalInteractionBackend<'_> {
    fn activate_selector(&mut self, selector: &str) -> Result<()> {
        self.context
            .simulate_click_selector(self.window, selector)?;
        self.context.run_until_parked();
        Ok(())
    }

    fn restart(&mut self) -> Result<()> {
        bail!("a cold restart requires the external proof command and a second process")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectivityFixture {
    Online,
    Offline,
    Reconnecting,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "message")]
pub enum ContentStateFixture {
    Empty,
    Loading,
    Ready,
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeFixture {
    Dark,
    Light,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadFixture {
    pub id: String,
    pub project_id: Option<String>,
    pub repository_id: Option<String>,
    pub worktree_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFixture {
    pub id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeGitStateFixture {
    NoGit,
    Unborn,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeFixture {
    pub id: String,
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_state: Option<WorktreeGitStateFixture>,
    pub dirty_files: u32,
    pub conflicts: u32,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryFixture {
    pub id: String,
    pub project_id: String,
    pub worktrees: Vec<WorktreeFixture>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRoleFixture {
    User,
    Assistant,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStateFixture {
    Complete,
    Streaming,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageFixture {
    pub id: String,
    pub thread_id: String,
    pub role: MessageRoleFixture,
    pub state: MessageStateFixture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStateFixture {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallFixture {
    pub id: String,
    pub thread_id: String,
    pub state: ToolCallStateFixture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStateFixture {
    Pending,
    InProgress,
    Completed,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStepFixture {
    pub id: String,
    pub thread_id: String,
    pub state: PlanStepStateFixture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKindFixture {
    File,
    Diff,
    Command,
    Plan,
    Url,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactFixture {
    pub id: String,
    pub thread_id: String,
    pub worktree_id: Option<String>,
    pub kind: ArtifactKindFixture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKindFixture {
    Message,
    ToolCall,
    Artifact,
    Repository,
    Connectivity,
    Persistence,
    RouteDecision,
    ExecutorDisclosure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventFixture {
    pub id: String,
    pub thread_id: String,
    pub revision: u64,
    pub kind: EventKindFixture,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceFixture {
    pub id: WorkSurfaceId,
    pub available: bool,
    pub badge: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkbenchBindingFixture {
    pub repository_id: String,
    pub worktree_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewCheckpointFixture {
    pub action_log_entity_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewBindingFixture {
    pub thread_id: String,
    pub session_id: String,
    pub repository_id: String,
    pub worktree_id: String,
    pub checkpoint: ReviewCheckpointFixture,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "message")]
pub enum ReviewLifecycleFixture {
    Unbound,
    Loading,
    Empty,
    Ready,
    Streaming,
    AllReviewed,
    Offline,
    UnavailableCheckpoint,
    UnsupportedBinary,
    Invalidated,
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFileStatusFixture {
    Added,
    Modified,
    Deleted,
    Renamed,
    Binary,
    Conflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewHunkStatusFixture {
    Pending,
    Kept,
    Rejected,
    Conflict,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewHunkFixture {
    pub id: String,
    pub start_row: u32,
    pub start_column: u32,
    pub end_row: u32,
    pub end_column: u32,
    pub status: ReviewHunkStatusFixture,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFileFixture {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: ReviewFileStatusFixture,
    pub hunks: Vec<ReviewHunkFixture>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFocusFixture {
    Surface,
    FileList,
    Diff,
    Toolbar,
    Editor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMutationKindFixture {
    KeepHunk,
    RejectHunk,
    KeepAll,
    RejectAll,
    OpenInEditor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewMutationFixture {
    pub kind: ReviewMutationKindFixture,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hunk_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resulting_contents: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSessionFixture {
    pub binding: ReviewBindingFixture,
    pub lifecycle: ReviewLifecycleFixture,
    pub files: Vec<ReviewFileFixture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_hunk_id: Option<String>,
    pub focus: ReviewFocusFixture,
    pub mutations: Vec<ReviewMutationFixture>,
    pub pending_operation_count: u32,
    pub ignored_stale_completion_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitBindingFixture {
    pub thread_id: String,
    pub repository_id: String,
    pub worktree_id: String,
    pub repository_entity_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "message")]
pub enum GitLifecycleFixture {
    Unbound,
    Loading,
    Ready,
    Offline,
    Reconnecting,
    RepositoryRemoved,
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum GitBranchFixture {
    Branch {
        name: String,
        ahead: u32,
        behind: u32,
    },
    Detached {
        head: String,
    },
    Unborn {
        name: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitFileStatusFixture {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitStagingStateFixture {
    Unstaged,
    Staged,
    PartiallyStaged,
    Conflict,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatusEntryFixture {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: GitFileStatusFixture,
    pub staging: GitStagingStateFixture,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatusCountsFixture {
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub conflicts: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitOperationKindFixture {
    Stage,
    Unstage,
    Discard,
    OpenDiff,
    Commit,
    CheckoutBranch,
    Pull,
    Push,
    SwitchRepository,
    Refresh,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "target", content = "value")]
pub enum GitMutationTargetFixture {
    None,
    Path(String),
    Branch(String),
    CommitMessage(String),
    Binding {
        repository_id: String,
        worktree_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "result", content = "message")]
pub enum GitMutationResultFixture {
    Succeeded,
    Pending,
    Cancelled,
    Rejected,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitMutationFixture {
    pub kind: GitOperationKindFixture,
    pub target: GitMutationTargetFixture,
    pub result: GitMutationResultFixture,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitPendingOperationFixture {
    pub id: String,
    pub kind: GitOperationKindFixture,
    pub target: GitMutationTargetFixture,
    pub confirmation_required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitFocusFixture {
    Surface,
    StatusList,
    StatusEntry,
    CommitEditor,
    BranchMenu,
    Toolbar,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSnapshotFixture {
    pub binding: GitBindingFixture,
    pub lifecycle: GitLifecycleFixture,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<GitBranchFixture>,
    pub status_entries: Vec<GitStatusEntryFixture>,
    pub status_counts: GitStatusCountsFixture,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_operation: Option<GitPendingOperationFixture>,
    pub badge_count: u32,
    pub requested_mutations: Vec<GitMutationFixture>,
    pub ignored_stale_refresh_count: u32,
    pub focus: GitFocusFixture,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalBindingFixture {
    pub thread_id: String,
    pub repository_id: String,
    pub worktree_id: String,
    pub worktree_abs_path: String,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOwnerFixture {
    pub thread_id: String,
    pub repository_id: String,
    pub worktree_id: String,
    pub worktree_abs_path: String,
    pub initial_cwd: String,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "value")]
pub enum TerminalProcessLifecycleFixture {
    Starting,
    Running { process_id: u32 },
    Exited { exit_code: i32 },
    FailedToSpawn(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalProcessFixture {
    pub terminal_id: String,
    pub item_id: String,
    pub title: String,
    pub owner: TerminalOwnerFixture,
    pub current_cwd: String,
    pub lifecycle: TerminalProcessLifecycleFixture,
    pub input_bytes: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalSplitAxisFixture {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "node")]
pub enum TerminalPaneLayoutFixture {
    Pane {
        pane_id: String,
    },
    Split {
        axis: TerminalSplitAxisFixture,
        children: Vec<TerminalPaneLayoutFixture>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalPaneFixture {
    pub pane_id: String,
    pub terminal_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_terminal_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "result", content = "value")]
pub enum TerminalSpawnResultFixture {
    Pending,
    Started { terminal_id: String },
    IgnoredStale,
    RejectedForeignBinding,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSpawnFixture {
    pub request_id: String,
    pub binding: TerminalBindingFixture,
    pub requested_cwd: String,
    pub result: TerminalSpawnResultFixture,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "message")]
pub enum TerminalLifecycleFixture {
    Empty,
    Ready,
    Starting,
    Offline,
    Reconnecting,
    WorktreeRemoved,
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "target", content = "id")]
pub enum TerminalFocusFixture {
    Surface,
    Pane(String),
    Terminal(String),
    Search,
    NewTerminal,
    Transcript,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSnapshotFixture {
    pub creation_binding: TerminalBindingFixture,
    pub panel_entity_id: String,
    pub lifecycle: TerminalLifecycleFixture,
    pub panes: Vec<TerminalPaneFixture>,
    pub pane_layout: TerminalPaneLayoutFixture,
    pub processes: Vec<TerminalProcessFixture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_terminal_id: Option<String>,
    pub requested_spawns: Vec<TerminalSpawnFixture>,
    pub running_badge_count: u32,
    pub implicit_spawn_count: u32,
    pub ignored_stale_completion_count: u32,
    pub rejected_foreign_spawn_count: u32,
    pub focus: TerminalFocusFixture,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadWorkbenchFixture {
    pub thread_id: String,
    pub generation: u64,
    pub binding: Option<WorkbenchBindingFixture>,
    pub requested_surface: Option<WorkSurfaceId>,
    pub effective_surface: Option<WorkSurfaceId>,
    pub dock_open: bool,
    pub surfaces: Vec<SurfaceFixture>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedSceneFixture {
    pub requested_surface: Option<WorkSurfaceId>,
    pub dock_open: bool,
    pub revision: u64,
    pub mutations_before_restart: Vec<SceneMutation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mutation")]
pub enum SceneMutation {
    SetActiveThread { thread_id: String },
    SetActiveSurface { surface: Option<WorkSurfaceId> },
    SetDockOpen { open: bool },
    SetConnectivity { connectivity: ConnectivityFixture },
    CompleteMessage { message_id: String },
    CompleteToolCall { tool_call_id: String },
    AdvanceRevision { revision: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkbenchScene {
    pub name: String,
    pub fixture_version: u32,
    pub viewport: ViewportFixture,
    pub theme: ThemeFixture,
    pub fake_time_ms: u64,
    pub connectivity: ConnectivityFixture,
    pub content_state: ContentStateFixture,
    pub threads: Vec<ThreadFixture>,
    pub active_thread_id: Option<String>,
    pub project: Option<ProjectFixture>,
    pub repositories: Vec<RepositoryFixture>,
    pub messages: Vec<MessageFixture>,
    pub tool_calls: Vec<ToolCallFixture>,
    pub plan_steps: Vec<PlanStepFixture>,
    pub artifacts: Vec<ArtifactFixture>,
    pub events: Vec<EventFixture>,
    pub surfaces: Vec<SurfaceFixture>,
    pub active_surface: Option<WorkSurfaceId>,
    pub dock_open: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thread_workbenches: Vec<ThreadWorkbenchFixture>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_sessions: Vec<ReviewSessionFixture>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub git_snapshots: Vec<GitSnapshotFixture>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terminal_snapshots: Vec<TerminalSnapshotFixture>,
    pub persisted: Option<PersistedSceneFixture>,
}

impl WorkbenchScene {
    pub fn empty(name: impl Into<String>, viewport: ViewportFixture) -> Self {
        Self {
            name: name.into(),
            fixture_version: 1,
            viewport,
            theme: ThemeFixture::Dark,
            fake_time_ms: 0,
            connectivity: ConnectivityFixture::Online,
            content_state: ContentStateFixture::Empty,
            threads: Vec::new(),
            active_thread_id: None,
            project: None,
            repositories: Vec::new(),
            messages: Vec::new(),
            tool_calls: Vec::new(),
            plan_steps: Vec::new(),
            artifacts: Vec::new(),
            events: Vec::new(),
            surfaces: WorkSurfaceId::ALL
                .into_iter()
                .map(|id| SurfaceFixture {
                    id,
                    available: false,
                    badge: None,
                })
                .collect(),
            active_surface: None,
            dock_open: false,
            thread_workbenches: Vec::new(),
            review_sessions: Vec::new(),
            git_snapshots: Vec::new(),
            terminal_snapshots: Vec::new(),
            persisted: None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("scene name must not be empty");
        }
        if !matches!(self.fixture_version, 1 | 2) {
            bail!("unsupported scene fixture version {}", self.fixture_version);
        }
        self.viewport.validate()?;

        unique_ids(
            "thread",
            self.threads.iter().map(|thread| thread.id.as_str()),
        )?;
        unique_ids(
            "repository",
            self.repositories
                .iter()
                .map(|repository| repository.id.as_str()),
        )?;
        unique_ids(
            "worktree",
            self.repositories
                .iter()
                .flat_map(|repository| repository.worktrees.iter())
                .map(|worktree| worktree.id.as_str()),
        )?;
        unique_ids(
            "message",
            self.messages.iter().map(|message| message.id.as_str()),
        )?;
        unique_ids(
            "tool call",
            self.tool_calls
                .iter()
                .map(|tool_call| tool_call.id.as_str()),
        )?;
        unique_ids(
            "plan step",
            self.plan_steps.iter().map(|step| step.id.as_str()),
        )?;
        unique_ids(
            "artifact",
            self.artifacts.iter().map(|artifact| artifact.id.as_str()),
        )?;
        unique_ids("event", self.events.iter().map(|event| event.id.as_str()))?;

        let surface_ids: BTreeSet<_> = self.surfaces.iter().map(|surface| surface.id).collect();
        if surface_ids.len() != self.surfaces.len() {
            bail!("scene contains duplicate surface fixtures");
        }
        if surface_ids != WorkSurfaceId::ALL.into_iter().collect() {
            bail!("scene must describe every native work surface");
        }

        if let ContentStateFixture::Error(message) = &self.content_state
            && message.trim().is_empty()
        {
            bail!("error content state must contain a message");
        }

        if let Some(active_thread_id) = &self.active_thread_id
            && !self
                .threads
                .iter()
                .any(|thread| &thread.id == active_thread_id)
        {
            bail!("active thread {active_thread_id:?} is not present in the scene");
        }

        for (kind, thread_id) in self
            .messages
            .iter()
            .map(|value| ("message", value.thread_id.as_str()))
            .chain(
                self.tool_calls
                    .iter()
                    .map(|value| ("tool call", value.thread_id.as_str())),
            )
            .chain(
                self.plan_steps
                    .iter()
                    .map(|value| ("plan step", value.thread_id.as_str())),
            )
            .chain(
                self.artifacts
                    .iter()
                    .map(|value| ("artifact", value.thread_id.as_str())),
            )
            .chain(
                self.events
                    .iter()
                    .map(|value| ("event", value.thread_id.as_str())),
            )
        {
            if !self.threads.iter().any(|thread| thread.id == thread_id) {
                bail!("{kind} references missing thread {thread_id:?}");
            }
        }

        if let Some(active_surface) = self.active_surface {
            let surface = self
                .surfaces
                .iter()
                .find(|surface| surface.id == active_surface)
                .ok_or_else(|| anyhow!("active surface {active_surface:?} has no fixture"))?;
            if !surface.available {
                bail!("active surface {active_surface:?} is unavailable");
            }
        } else if self.dock_open {
            bail!("an open dock must have an active surface");
        }

        match &self.project {
            Some(project) => {
                if project.id.trim().is_empty() || project.display_name.trim().is_empty() {
                    bail!("project ID and display name must not be empty");
                }
                for repository in &self.repositories {
                    if repository.project_id != project.id {
                        bail!(
                            "repository {:?} references missing project {:?}",
                            repository.id,
                            repository.project_id
                        );
                    }
                    if repository.worktrees.is_empty() {
                        bail!("repository {:?} has no worktree fixtures", repository.id);
                    }
                    for worktree in &repository.worktrees {
                        if worktree
                            .branch
                            .as_deref()
                            .is_some_and(|branch| branch.trim().is_empty())
                        {
                            bail!("worktree {:?} has an empty branch name", worktree.id);
                        }
                    }
                }
            }
            None if !self.repositories.is_empty() => {
                bail!("repository fixtures require a project fixture");
            }
            None => {}
        }

        for thread in &self.threads {
            match (
                thread.project_id.as_deref(),
                thread.repository_id.as_deref(),
                thread.worktree_id.as_deref(),
            ) {
                (None, None, None) => {}
                (Some(project_id), Some(repository_id), Some(worktree_id)) => {
                    if self.project.as_ref().map(|project| project.id.as_str()) != Some(project_id)
                    {
                        bail!(
                            "thread {:?} references missing project {project_id:?}",
                            thread.id
                        );
                    }
                    let repository = self
                        .repositories
                        .iter()
                        .find(|repository| repository.id == repository_id)
                        .ok_or_else(|| {
                            anyhow!(
                                "thread {:?} references missing repository {repository_id:?}",
                                thread.id
                            )
                        })?;
                    if !repository
                        .worktrees
                        .iter()
                        .any(|worktree| worktree.id == worktree_id)
                    {
                        bail!(
                            "thread {:?} references missing worktree {worktree_id:?}",
                            thread.id
                        );
                    }
                }
                _ => bail!(
                    "thread {:?} must specify project, repository, and worktree together",
                    thread.id
                ),
            }
        }
        self.validate_thread_workbenches()?;
        self.validate_review_sessions()?;
        self.validate_git_snapshots()?;
        self.validate_terminal_snapshots()?;

        for artifact in &self.artifacts {
            if let Some(worktree_id) = &artifact.worktree_id
                && !self
                    .repositories
                    .iter()
                    .flat_map(|repository| &repository.worktrees)
                    .any(|worktree| worktree.id == *worktree_id)
            {
                bail!(
                    "artifact {:?} references missing worktree {worktree_id:?}",
                    artifact.id
                );
            }
        }

        if let Some(persisted) = &self.persisted {
            if persisted.dock_open && persisted.requested_surface.is_none() {
                bail!("persisted open dock must have a requested surface");
            }
            if let Some(surface) = persisted.requested_surface
                && !self
                    .surfaces
                    .iter()
                    .any(|fixture| fixture.id == surface && fixture.available)
            {
                bail!("persisted state references unavailable surface {surface:?}");
            }
            for mutation in &persisted.mutations_before_restart {
                self.validate_mutation(mutation)?;
            }
        }

        Ok(())
    }

    pub fn active_thread_workbench(&self) -> Option<&ThreadWorkbenchFixture> {
        let active_thread_id = self.active_thread_id.as_deref()?;
        self.thread_workbenches
            .iter()
            .find(|workbench| workbench.thread_id == active_thread_id)
    }

    fn validate_thread_workbenches(&self) -> Result<()> {
        match self.fixture_version {
            1 => {
                if !self.thread_workbenches.is_empty() {
                    bail!("version 1 scenes cannot contain per-thread workbench fixtures");
                }
            }
            2 => {
                unique_ids(
                    "thread workbench",
                    self.thread_workbenches
                        .iter()
                        .map(|workbench| workbench.thread_id.as_str()),
                )?;
                if self.thread_workbenches.len() != self.threads.len() {
                    bail!("version 2 scenes must contain exactly one workbench fixture per thread");
                }

                for thread in &self.threads {
                    let workbench = self
                        .thread_workbenches
                        .iter()
                        .find(|workbench| workbench.thread_id == thread.id)
                        .ok_or_else(|| {
                            anyhow!(
                                "version 2 scene has no workbench fixture for thread {:?}",
                                thread.id
                            )
                        })?;
                    validate_surface_fixtures(
                        &format!("thread {:?} workbench", thread.id),
                        &workbench.surfaces,
                    )?;

                    match (
                        &workbench.binding,
                        thread.repository_id.as_deref(),
                        thread.worktree_id.as_deref(),
                    ) {
                        (None, None, None) => {}
                        (Some(binding), Some(repository_id), Some(worktree_id))
                            if binding.repository_id == repository_id
                                && binding.worktree_id == worktree_id => {}
                        _ => bail!(
                            "thread {:?} workbench binding does not match its thread fixture",
                            thread.id
                        ),
                    }

                    if workbench.binding.is_none()
                        && workbench
                            .surfaces
                            .iter()
                            .any(|surface| surface.available && surface.id.requires_binding())
                    {
                        bail!(
                            "unbound thread {:?} advertises a repository-bound surface",
                            thread.id
                        );
                    }

                    let expected_surface =
                        deterministic_surface(workbench.requested_surface, &workbench.surfaces);
                    if workbench.effective_surface != expected_surface {
                        bail!(
                            "thread {:?} effective surface {:?} does not match deterministic projection {expected_surface:?}",
                            thread.id,
                            workbench.effective_surface
                        );
                    }
                    if workbench.dock_open && workbench.effective_surface.is_none() {
                        bail!(
                            "thread {:?} has an open dock without an effective surface",
                            thread.id
                        );
                    }
                }

                match self.active_thread_workbench() {
                    Some(workbench) => {
                        for surface_id in WorkSurfaceId::ALL {
                            let visible_surface = surface_fixture(&self.surfaces, surface_id)?;
                            let thread_surface = surface_fixture(&workbench.surfaces, surface_id)?;
                            if visible_surface != thread_surface {
                                bail!(
                                    "visible surface {surface_id:?} does not match active thread {:?}",
                                    workbench.thread_id
                                );
                            }
                        }
                        if self.active_surface != workbench.effective_surface {
                            bail!(
                                "visible active surface does not match active thread {:?}",
                                workbench.thread_id
                            );
                        }
                        if self.dock_open != workbench.dock_open {
                            bail!(
                                "visible dock state does not match active thread {:?}",
                                workbench.thread_id
                            );
                        }
                    }
                    None if self.active_thread_id.is_some() => {
                        bail!("active thread has no version 2 workbench fixture");
                    }
                    None if self.active_surface.is_some() || self.dock_open => {
                        bail!(
                            "version 2 scene without an active thread has visible workbench state"
                        );
                    }
                    None => {}
                }
            }
            version => bail!("unsupported scene fixture version {version}"),
        }
        Ok(())
    }

    fn validate_review_sessions(&self) -> Result<()> {
        unique_ids(
            "review session thread",
            self.review_sessions
                .iter()
                .map(|review| review.binding.thread_id.as_str()),
        )?;
        unique_ids(
            "review session",
            self.review_sessions
                .iter()
                .map(|review| review.binding.session_id.as_str()),
        )?;

        for review in &self.review_sessions {
            let binding = &review.binding;
            let thread = self
                .threads
                .iter()
                .find(|thread| thread.id == binding.thread_id)
                .ok_or_else(|| {
                    anyhow!(
                        "review session references missing thread {:?}",
                        binding.thread_id
                    )
                })?;
            if thread.repository_id.as_deref() != Some(binding.repository_id.as_str())
                || thread.worktree_id.as_deref() != Some(binding.worktree_id.as_str())
            {
                bail!(
                    "review session for thread {:?} does not match its repository/worktree binding",
                    binding.thread_id
                );
            }
            if binding.session_id.trim().is_empty()
                || binding.checkpoint.action_log_entity_id.trim().is_empty()
            {
                bail!("review session and checkpoint IDs must not be empty");
            }
            if let ReviewLifecycleFixture::Error(message) = &review.lifecycle
                && message.trim().is_empty()
            {
                bail!("review error lifecycle must contain a message");
            }

            unique_ids(
                "review file",
                review.files.iter().map(|file| file.path.as_str()),
            )?;
            let mut hunk_ids = BTreeSet::new();
            for file in &review.files {
                if file.path.trim().is_empty()
                    || file
                        .old_path
                        .as_deref()
                        .is_some_and(|path| path.trim().is_empty())
                {
                    bail!("review file paths must not be empty");
                }
                match file.status {
                    ReviewFileStatusFixture::Renamed if file.old_path.is_none() => {
                        bail!("renamed review file {:?} has no old path", file.path);
                    }
                    ReviewFileStatusFixture::Renamed => {}
                    _ if file.old_path.is_some() => {
                        bail!("non-renamed review file {:?} has an old path", file.path);
                    }
                    _ => {}
                }
                for hunk in &file.hunks {
                    if hunk.id.trim().is_empty() || !hunk_ids.insert(hunk.id.as_str()) {
                        bail!("review hunk IDs must be non-empty and unique within a session");
                    }
                    if (hunk.start_row, hunk.start_column) > (hunk.end_row, hunk.end_column) {
                        bail!("review hunk {:?} has a reversed buffer range", hunk.id);
                    }
                }
            }

            match (
                review.selected_file_path.as_deref(),
                review.selected_hunk_id.as_deref(),
            ) {
                (None, None) => {}
                (Some(file_path), selected_hunk_id) => {
                    let file = review
                        .files
                        .iter()
                        .find(|file| file.path == file_path)
                        .ok_or_else(|| {
                            anyhow!("review selection references missing file {file_path:?}")
                        })?;
                    if let Some(hunk_id) = selected_hunk_id
                        && !file.hunks.iter().any(|hunk| hunk.id == hunk_id)
                    {
                        bail!(
                            "review selection references hunk {hunk_id:?} outside file {file_path:?}"
                        );
                    }
                }
                (None, Some(hunk_id)) => {
                    bail!("review hunk {hunk_id:?} is selected without a selected file");
                }
            }

            if matches!(
                review.lifecycle,
                ReviewLifecycleFixture::Empty
                    | ReviewLifecycleFixture::Loading
                    | ReviewLifecycleFixture::Offline
                    | ReviewLifecycleFixture::UnavailableCheckpoint
                    | ReviewLifecycleFixture::UnsupportedBinary
                    | ReviewLifecycleFixture::Invalidated
                    | ReviewLifecycleFixture::Error(_)
            ) && !review.files.is_empty()
            {
                bail!(
                    "review lifecycle {:?} cannot expose file fixtures",
                    review.lifecycle
                );
            }

            for mutation in &review.mutations {
                match mutation.kind {
                    ReviewMutationKindFixture::KeepHunk
                    | ReviewMutationKindFixture::RejectHunk
                    | ReviewMutationKindFixture::OpenInEditor => {
                        let file_path = mutation.file_path.as_deref().ok_or_else(|| {
                            anyhow!("review {:?} mutation has no file path", mutation.kind)
                        })?;
                        let Some(file) = review.files.iter().find(|file| file.path == file_path)
                        else {
                            if matches!(review.lifecycle, ReviewLifecycleFixture::AllReviewed)
                                && !matches!(mutation.kind, ReviewMutationKindFixture::OpenInEditor)
                            {
                                continue;
                            }
                            bail!(
                                "review {:?} mutation references missing file {file_path:?}",
                                mutation.kind
                            );
                        };
                        let hunk_id = mutation.hunk_id.as_deref().ok_or_else(|| {
                            anyhow!("review {:?} mutation has no hunk ID", mutation.kind)
                        })?;
                        if !file.hunks.iter().any(|hunk| hunk.id == hunk_id) {
                            bail!(
                                "review {:?} mutation references missing hunk {hunk_id:?}",
                                mutation.kind
                            );
                        }
                    }
                    ReviewMutationKindFixture::KeepAll | ReviewMutationKindFixture::RejectAll => {
                        if mutation.file_path.is_some() || mutation.hunk_id.is_some() {
                            bail!(
                                "review {:?} mutation must not target a single hunk",
                                mutation.kind
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn active_review_session(&self) -> Option<&ReviewSessionFixture> {
        let active_thread_id = self.active_thread_id.as_deref()?;
        self.review_sessions
            .iter()
            .find(|review| review.binding.thread_id == active_thread_id)
    }

    fn validate_git_snapshots(&self) -> Result<()> {
        unique_ids(
            "Git snapshot thread",
            self.git_snapshots
                .iter()
                .map(|snapshot| snapshot.binding.thread_id.as_str()),
        )?;

        for snapshot in &self.git_snapshots {
            let binding = &snapshot.binding;
            let thread = self
                .threads
                .iter()
                .find(|thread| thread.id == binding.thread_id)
                .ok_or_else(|| {
                    anyhow!(
                        "Git snapshot references missing thread {:?}",
                        binding.thread_id
                    )
                })?;
            if thread.repository_id.as_deref() != Some(binding.repository_id.as_str())
                || thread.worktree_id.as_deref() != Some(binding.worktree_id.as_str())
            {
                bail!(
                    "Git snapshot for thread {:?} does not match its repository/worktree binding",
                    binding.thread_id
                );
            }
            if binding.repository_entity_id.trim().is_empty() {
                bail!("Git repository entity ID must not be empty");
            }

            let workbench = self
                .thread_workbenches
                .iter()
                .find(|workbench| workbench.thread_id == binding.thread_id)
                .ok_or_else(|| {
                    anyhow!(
                        "Git snapshot for thread {:?} has no workbench projection",
                        binding.thread_id
                    )
                })?;
            if workbench.generation != binding.generation
                || workbench.binding.as_ref()
                    != Some(&WorkbenchBindingFixture {
                        repository_id: binding.repository_id.clone(),
                        worktree_id: binding.worktree_id.clone(),
                    })
            {
                bail!(
                    "Git snapshot for thread {:?} does not match its workbench generation/binding",
                    binding.thread_id
                );
            }

            let repository = self
                .repositories
                .iter()
                .find(|repository| repository.id == binding.repository_id)
                .ok_or_else(|| {
                    anyhow!(
                        "Git snapshot references missing repository {:?}",
                        binding.repository_id
                    )
                })?;
            let worktree = repository
                .worktrees
                .iter()
                .find(|worktree| worktree.id == binding.worktree_id)
                .ok_or_else(|| {
                    anyhow!(
                        "Git snapshot references missing worktree {:?}",
                        binding.worktree_id
                    )
                })?;

            if let GitLifecycleFixture::Error(message) = &snapshot.lifecycle
                && message.trim().is_empty()
            {
                bail!("Git error lifecycle must contain a message");
            }
            if matches!(
                snapshot.lifecycle,
                GitLifecycleFixture::Unbound
                    | GitLifecycleFixture::Loading
                    | GitLifecycleFixture::RepositoryRemoved
                    | GitLifecycleFixture::Error(_)
            ) && (!snapshot.status_entries.is_empty()
                || snapshot.selected_path.is_some()
                || snapshot.pending_operation.is_some())
            {
                bail!(
                    "Git lifecycle {:?} cannot expose status, selection, or pending operation state",
                    snapshot.lifecycle
                );
            }
            if matches!(
                snapshot.lifecycle,
                GitLifecycleFixture::Unbound | GitLifecycleFixture::RepositoryRemoved
            ) && snapshot.branch.is_some()
            {
                bail!(
                    "Git lifecycle {:?} cannot expose branch state",
                    snapshot.lifecycle
                );
            }
            if matches!(
                snapshot.lifecycle,
                GitLifecycleFixture::Ready
                    | GitLifecycleFixture::Offline
                    | GitLifecycleFixture::Reconnecting
            ) && snapshot.branch.is_none()
            {
                bail!(
                    "Git lifecycle {:?} requires branch state",
                    snapshot.lifecycle
                );
            }

            unique_ids(
                "Git status path",
                snapshot
                    .status_entries
                    .iter()
                    .map(|entry| entry.path.as_str()),
            )?;
            if snapshot.status_entries.windows(2).any(|entries| {
                entries
                    .first()
                    .zip(entries.get(1))
                    .is_some_and(|(left, right)| left.path > right.path)
            }) {
                bail!("Git status entries must be ordered by path");
            }
            for entry in &snapshot.status_entries {
                if entry.path.trim().is_empty()
                    || entry
                        .old_path
                        .as_deref()
                        .is_some_and(|path| path.trim().is_empty())
                {
                    bail!("Git status paths must not be empty");
                }
                match entry.status {
                    GitFileStatusFixture::Renamed if entry.old_path.is_none() => {
                        bail!("renamed Git status {:?} has no old path", entry.path);
                    }
                    GitFileStatusFixture::Renamed => {}
                    _ if entry.old_path.is_some() => {
                        bail!("non-renamed Git status {:?} has an old path", entry.path);
                    }
                    _ => {}
                }
                if entry.status == GitFileStatusFixture::Conflicted
                    && entry.staging != GitStagingStateFixture::Conflict
                {
                    bail!(
                        "conflicted Git status {:?} must use conflict staging state",
                        entry.path
                    );
                }
                if entry.staging == GitStagingStateFixture::Conflict
                    && entry.status != GitFileStatusFixture::Conflicted
                {
                    bail!(
                        "conflict staging state {:?} must use conflicted file status",
                        entry.path
                    );
                }
            }

            let expected_counts = git_status_counts(&snapshot.status_entries)?;
            if snapshot.status_counts != expected_counts {
                bail!(
                    "Git status counts {:?} do not match entries {expected_counts:?}",
                    snapshot.status_counts
                );
            }
            if u32::try_from(snapshot.status_entries.len()).ok() != Some(snapshot.badge_count) {
                bail!(
                    "Git badge count {} does not match {} unique status paths",
                    snapshot.badge_count,
                    snapshot.status_entries.len()
                );
            }
            if worktree.dirty_files != snapshot.badge_count
                || worktree.conflicts != snapshot.status_counts.conflicts
            {
                bail!(
                    "Git snapshot counts do not match worktree {:?}",
                    binding.worktree_id
                );
            }

            validate_git_branch(&snapshot.branch, worktree)?;

            if let Some(selected_path) = &snapshot.selected_path
                && !snapshot
                    .status_entries
                    .iter()
                    .any(|entry| entry.path == *selected_path)
            {
                bail!("Git selection references missing status path {selected_path:?}");
            }

            for mutation in &snapshot.requested_mutations {
                validate_git_mutation(mutation)?;
            }
            if let Some(pending) = &snapshot.pending_operation {
                if pending.id.trim().is_empty() {
                    bail!("pending Git operation ID must not be empty");
                }
                validate_git_target(&pending.target)?;
                if !snapshot.requested_mutations.iter().any(|mutation| {
                    mutation.kind == pending.kind
                        && mutation.target == pending.target
                        && mutation.result == GitMutationResultFixture::Pending
                }) {
                    bail!(
                        "pending Git operation {:?} has no matching requested mutation",
                        pending.id
                    );
                }
            }

            let expected_badge = (snapshot.badge_count > 0).then_some(snapshot.badge_count);
            let git_surface = surface_fixture(&workbench.surfaces, WorkSurfaceId::Git)?;
            if git_surface.badge != expected_badge {
                bail!(
                    "Git rail badge {:?} disagrees with typed snapshot badge {expected_badge:?}",
                    git_surface.badge
                );
            }
        }

        Ok(())
    }

    pub fn active_git_snapshot(&self) -> Option<&GitSnapshotFixture> {
        let active_thread_id = self.active_thread_id.as_deref()?;
        self.git_snapshots
            .iter()
            .find(|snapshot| snapshot.binding.thread_id == active_thread_id)
    }

    fn validate_terminal_snapshots(&self) -> Result<()> {
        unique_ids(
            "Terminal snapshot thread",
            self.terminal_snapshots
                .iter()
                .map(|snapshot| snapshot.creation_binding.thread_id.as_str()),
        )?;

        let panel_entity_ids = self
            .terminal_snapshots
            .iter()
            .map(|snapshot| snapshot.panel_entity_id.as_str())
            .collect::<BTreeSet<_>>();
        if panel_entity_ids.len() > 1 {
            bail!("Terminal snapshots must retain one workspace panel entity");
        }

        for snapshot in &self.terminal_snapshots {
            let binding = &snapshot.creation_binding;
            validate_terminal_binding(binding)?;
            if snapshot.panel_entity_id.trim().is_empty() {
                bail!("Terminal panel entity ID must not be empty");
            }
            let thread = self
                .threads
                .iter()
                .find(|thread| thread.id == binding.thread_id)
                .ok_or_else(|| {
                    anyhow!(
                        "Terminal snapshot references missing thread {:?}",
                        binding.thread_id
                    )
                })?;
            if thread.repository_id.as_deref() != Some(binding.repository_id.as_str())
                || thread.worktree_id.as_deref() != Some(binding.worktree_id.as_str())
            {
                bail!(
                    "Terminal snapshot for thread {:?} does not match its repository/worktree binding",
                    binding.thread_id
                );
            }
            let workbench = self
                .thread_workbenches
                .iter()
                .find(|workbench| workbench.thread_id == binding.thread_id)
                .ok_or_else(|| {
                    anyhow!(
                        "Terminal snapshot for thread {:?} has no workbench projection",
                        binding.thread_id
                    )
                })?;
            if workbench.generation != binding.generation
                || workbench.binding.as_ref()
                    != Some(&WorkbenchBindingFixture {
                        repository_id: binding.repository_id.clone(),
                        worktree_id: binding.worktree_id.clone(),
                    })
            {
                bail!(
                    "Terminal snapshot for thread {:?} does not match its workbench generation/binding",
                    binding.thread_id
                );
            }

            if let TerminalLifecycleFixture::Error(message) = &snapshot.lifecycle
                && message.trim().is_empty()
            {
                bail!("Terminal error lifecycle must contain a message");
            }
            if snapshot.implicit_spawn_count != 0 {
                bail!("selecting or reopening Terminal must not implicitly spawn a process");
            }

            unique_ids(
                "Terminal process",
                snapshot
                    .processes
                    .iter()
                    .map(|process| process.terminal_id.as_str()),
            )?;
            unique_ids(
                "Terminal item",
                snapshot
                    .processes
                    .iter()
                    .map(|process| process.item_id.as_str()),
            )?;
            for process in &snapshot.processes {
                validate_terminal_process(process, self)?;
            }

            unique_ids(
                "Terminal pane",
                snapshot.panes.iter().map(|pane| pane.pane_id.as_str()),
            )?;
            let process_ids = snapshot
                .processes
                .iter()
                .map(|process| process.terminal_id.as_str())
                .collect::<BTreeSet<_>>();
            let mut pane_process_ids = BTreeSet::new();
            for pane in &snapshot.panes {
                if pane.pane_id.trim().is_empty() {
                    bail!("Terminal pane ID must not be empty");
                }
                for terminal_id in &pane.terminal_ids {
                    if !process_ids.contains(terminal_id.as_str()) {
                        bail!(
                            "Terminal pane {:?} references missing process {terminal_id:?}",
                            pane.pane_id
                        );
                    }
                    if !pane_process_ids.insert(terminal_id.as_str()) {
                        bail!("Terminal process {terminal_id:?} appears in multiple panes");
                    }
                }
                if let Some(active_terminal_id) = &pane.active_terminal_id
                    && !pane
                        .terminal_ids
                        .iter()
                        .any(|terminal_id| terminal_id == active_terminal_id)
                {
                    bail!(
                        "Terminal pane {:?} activates missing process {active_terminal_id:?}",
                        pane.pane_id
                    );
                }
            }
            if pane_process_ids != process_ids {
                bail!("Terminal panes must contain every process exactly once");
            }

            let mut layout_pane_ids = Vec::new();
            collect_terminal_layout_panes(&snapshot.pane_layout, &mut layout_pane_ids)?;
            let layout_pane_ids = layout_pane_ids.into_iter().collect::<BTreeSet<_>>();
            let pane_ids = snapshot
                .panes
                .iter()
                .map(|pane| pane.pane_id.as_str())
                .collect::<BTreeSet<_>>();
            if layout_pane_ids != pane_ids {
                bail!("Terminal pane layout must contain every pane exactly once");
            }

            if let Some(selected_terminal_id) = &snapshot.selected_terminal_id {
                if !process_ids.contains(selected_terminal_id.as_str()) {
                    bail!("Terminal selection references missing process {selected_terminal_id:?}");
                }
                if !snapshot
                    .panes
                    .iter()
                    .any(|pane| pane.active_terminal_id.as_ref() == Some(selected_terminal_id))
                {
                    bail!(
                        "selected Terminal process {selected_terminal_id:?} is not active in its pane"
                    );
                }
            } else if !snapshot.processes.is_empty() {
                bail!("non-empty Terminal snapshot must select a process");
            }
            validate_terminal_focus(&snapshot.focus, snapshot)?;

            unique_ids(
                "Terminal spawn request",
                snapshot
                    .requested_spawns
                    .iter()
                    .map(|spawn| spawn.request_id.as_str()),
            )?;
            for spawn in &snapshot.requested_spawns {
                validate_terminal_spawn(spawn, snapshot)?;
            }
            let ignored_stale_count = u32::try_from(
                snapshot
                    .requested_spawns
                    .iter()
                    .filter(|spawn| {
                        matches!(spawn.result, TerminalSpawnResultFixture::IgnoredStale)
                    })
                    .count(),
            )
            .context("Terminal stale spawn count overflow")?;
            if snapshot.ignored_stale_completion_count != ignored_stale_count {
                bail!("Terminal ignored stale completion count does not match spawn records");
            }
            let rejected_foreign_count = u32::try_from(
                snapshot
                    .requested_spawns
                    .iter()
                    .filter(|spawn| {
                        matches!(
                            spawn.result,
                            TerminalSpawnResultFixture::RejectedForeignBinding
                        )
                    })
                    .count(),
            )
            .context("Terminal rejected foreign spawn count overflow")?;
            if snapshot.rejected_foreign_spawn_count != rejected_foreign_count {
                bail!("Terminal rejected foreign spawn count does not match spawn records");
            }

            let running_count = u32::try_from(
                snapshot
                    .processes
                    .iter()
                    .filter(|process| {
                        matches!(
                            process.lifecycle,
                            TerminalProcessLifecycleFixture::Starting
                                | TerminalProcessLifecycleFixture::Running { .. }
                        )
                    })
                    .count(),
            )
            .context("Terminal running badge count overflow")?;
            if snapshot.running_badge_count != running_count {
                bail!(
                    "Terminal running badge {} does not match {running_count} live processes",
                    snapshot.running_badge_count
                );
            }
            let expected_badge = (running_count > 0).then_some(running_count);
            let terminal_surface = surface_fixture(&workbench.surfaces, WorkSurfaceId::Terminal)?;
            if terminal_surface.badge != expected_badge {
                bail!(
                    "Terminal rail badge {:?} disagrees with typed snapshot badge {expected_badge:?}",
                    terminal_surface.badge
                );
            }

            match snapshot.lifecycle {
                TerminalLifecycleFixture::Empty
                    if !snapshot.processes.is_empty() || !snapshot.requested_spawns.is_empty() =>
                {
                    bail!("empty Terminal lifecycle cannot expose processes or spawn requests");
                }
                TerminalLifecycleFixture::Starting
                    if !snapshot.requested_spawns.iter().any(|spawn| {
                        matches!(spawn.result, TerminalSpawnResultFixture::Pending)
                    }) && !snapshot.processes.iter().any(|process| {
                        matches!(process.lifecycle, TerminalProcessLifecycleFixture::Starting)
                    }) =>
                {
                    bail!("starting Terminal lifecycle requires pending spawn state");
                }
                _ => {}
            }
        }

        Ok(())
    }

    pub fn active_terminal_snapshot(&self) -> Option<&TerminalSnapshotFixture> {
        let active_thread_id = self.active_thread_id.as_deref()?;
        self.terminal_snapshots
            .iter()
            .find(|snapshot| snapshot.creation_binding.thread_id == active_thread_id)
    }

    fn validate_mutation(&self, mutation: &SceneMutation) -> Result<()> {
        match mutation {
            SceneMutation::SetActiveThread { thread_id } => {
                if !self.threads.iter().any(|thread| thread.id == *thread_id) {
                    bail!("restart mutation references missing thread {thread_id:?}");
                }
            }
            SceneMutation::CompleteMessage { message_id } => {
                if !self
                    .messages
                    .iter()
                    .any(|message| message.id == *message_id)
                {
                    bail!("restart mutation references missing message {message_id:?}");
                }
            }
            SceneMutation::CompleteToolCall { tool_call_id } => {
                if !self
                    .tool_calls
                    .iter()
                    .any(|tool_call| tool_call.id == *tool_call_id)
                {
                    bail!("restart mutation references missing tool call {tool_call_id:?}");
                }
            }
            SceneMutation::SetActiveSurface {
                surface: Some(surface),
            } => {
                if !self
                    .surfaces
                    .iter()
                    .any(|fixture| fixture.id == *surface && fixture.available)
                {
                    bail!("restart mutation references unavailable surface {surface:?}");
                }
            }
            SceneMutation::SetActiveSurface { surface: None }
            | SceneMutation::SetDockOpen { .. }
            | SceneMutation::SetConnectivity { .. }
            | SceneMutation::AdvanceRevision { .. } => {}
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String> {
        self.validate()?;
        let encoded = serde_json::to_vec(self).context("serializing workbench scene")?;
        Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
    }
}

fn validate_surface_fixtures(kind: &str, surfaces: &[SurfaceFixture]) -> Result<()> {
    let surface_ids: BTreeSet<_> = surfaces.iter().map(|surface| surface.id).collect();
    if surface_ids.len() != surfaces.len() {
        bail!("{kind} contains duplicate surface fixtures");
    }
    if surface_ids != WorkSurfaceId::ALL.into_iter().collect() {
        bail!("{kind} must describe every native work surface");
    }
    Ok(())
}

fn surface_fixture(
    surfaces: &[SurfaceFixture],
    surface_id: WorkSurfaceId,
) -> Result<&SurfaceFixture> {
    surfaces
        .iter()
        .find(|surface| surface.id == surface_id)
        .ok_or_else(|| anyhow!("surface {surface_id:?} has no fixture"))
}

fn deterministic_surface(
    requested_surface: Option<WorkSurfaceId>,
    surfaces: &[SurfaceFixture],
) -> Option<WorkSurfaceId> {
    match requested_surface {
        None => None,
        Some(requested_surface)
            if surfaces
                .iter()
                .any(|surface| surface.id == requested_surface && surface.available) =>
        {
            Some(requested_surface)
        }
        Some(_) => WorkSurfaceId::ALL.into_iter().find(|surface_id| {
            surfaces
                .iter()
                .any(|surface| surface.id == *surface_id && surface.available)
        }),
    }
}

fn unique_ids<'a>(kind: &str, ids: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if id.trim().is_empty() {
            bail!("{kind} ID must not be empty");
        }
        if !seen.insert(id) {
            bail!("duplicate {kind} ID {id:?}");
        }
    }
    Ok(())
}

fn git_status_counts(entries: &[GitStatusEntryFixture]) -> Result<GitStatusCountsFixture> {
    let mut counts = GitStatusCountsFixture::default();
    for entry in entries {
        match entry.staging {
            GitStagingStateFixture::Unstaged => {
                if entry.status == GitFileStatusFixture::Untracked {
                    counts.untracked = counts
                        .untracked
                        .checked_add(1)
                        .context("Git untracked count overflow")?;
                } else {
                    counts.unstaged = counts
                        .unstaged
                        .checked_add(1)
                        .context("Git unstaged count overflow")?;
                }
            }
            GitStagingStateFixture::Staged => {
                if entry.status == GitFileStatusFixture::Untracked {
                    bail!("untracked Git status {:?} cannot be staged", entry.path);
                }
                counts.staged = counts
                    .staged
                    .checked_add(1)
                    .context("Git staged count overflow")?;
            }
            GitStagingStateFixture::PartiallyStaged => {
                if entry.status == GitFileStatusFixture::Untracked {
                    bail!(
                        "untracked Git status {:?} cannot be partially staged",
                        entry.path
                    );
                }
                counts.staged = counts
                    .staged
                    .checked_add(1)
                    .context("Git staged count overflow")?;
                counts.unstaged = counts
                    .unstaged
                    .checked_add(1)
                    .context("Git unstaged count overflow")?;
            }
            GitStagingStateFixture::Conflict => {
                counts.conflicts = counts
                    .conflicts
                    .checked_add(1)
                    .context("Git conflict count overflow")?;
            }
        }
    }
    Ok(counts)
}

fn validate_git_branch(
    branch: &Option<GitBranchFixture>,
    worktree: &WorktreeFixture,
) -> Result<()> {
    match branch {
        Some(GitBranchFixture::Branch {
            name,
            ahead,
            behind,
        }) => {
            if name.trim().is_empty() || worktree.branch.as_deref() != Some(name.as_str()) {
                bail!("Git branch does not match its worktree branch");
            }
            if worktree.ahead != *ahead || worktree.behind != *behind {
                bail!("Git branch ahead/behind counts do not match its worktree");
            }
            if worktree.git_state == Some(WorktreeGitStateFixture::Unborn) {
                bail!("unborn worktree cannot expose an established Git branch");
            }
        }
        Some(GitBranchFixture::Detached { head }) => {
            if head.trim().is_empty() || worktree.branch.is_some() {
                bail!("detached Git head must be non-empty and have no worktree branch");
            }
        }
        Some(GitBranchFixture::Unborn { name }) => {
            if name.trim().is_empty()
                || worktree.branch.as_deref() != Some(name.as_str())
                || worktree.git_state != Some(WorktreeGitStateFixture::Unborn)
            {
                bail!("unborn Git branch does not match its worktree");
            }
        }
        None => {}
    }
    Ok(())
}

fn validate_git_target(target: &GitMutationTargetFixture) -> Result<()> {
    match target {
        GitMutationTargetFixture::None => {}
        GitMutationTargetFixture::Path(value)
        | GitMutationTargetFixture::Branch(value)
        | GitMutationTargetFixture::CommitMessage(value) => {
            if value.trim().is_empty() {
                bail!("Git mutation target must not be empty");
            }
        }
        GitMutationTargetFixture::Binding {
            repository_id,
            worktree_id,
        } => {
            if repository_id.trim().is_empty() || worktree_id.trim().is_empty() {
                bail!("Git mutation binding target must not be empty");
            }
        }
    }
    Ok(())
}

fn validate_git_mutation(mutation: &GitMutationFixture) -> Result<()> {
    validate_git_target(&mutation.target)?;
    let target_matches_kind = matches!(
        (&mutation.kind, &mutation.target),
        (
            GitOperationKindFixture::Stage
                | GitOperationKindFixture::Unstage
                | GitOperationKindFixture::Discard
                | GitOperationKindFixture::OpenDiff,
            GitMutationTargetFixture::Path(_)
        ) | (
            GitOperationKindFixture::Commit,
            GitMutationTargetFixture::CommitMessage(_)
        ) | (
            GitOperationKindFixture::CheckoutBranch,
            GitMutationTargetFixture::Branch(_)
        ) | (
            GitOperationKindFixture::SwitchRepository,
            GitMutationTargetFixture::Binding { .. }
        ) | (
            GitOperationKindFixture::Pull
                | GitOperationKindFixture::Push
                | GitOperationKindFixture::Refresh,
            GitMutationTargetFixture::None
        )
    );
    if !target_matches_kind {
        bail!(
            "Git {:?} mutation has incompatible target {:?}",
            mutation.kind,
            mutation.target
        );
    }
    if let GitMutationResultFixture::Failed(message) = &mutation.result
        && message.trim().is_empty()
    {
        bail!("failed Git mutation must contain a message");
    }
    Ok(())
}

fn validate_terminal_binding(binding: &TerminalBindingFixture) -> Result<()> {
    if binding.thread_id.trim().is_empty()
        || binding.repository_id.trim().is_empty()
        || binding.worktree_id.trim().is_empty()
        || binding.worktree_abs_path.trim().is_empty()
    {
        bail!("Terminal binding identifiers and worktree path must not be empty");
    }
    if !Path::new(&binding.worktree_abs_path).is_absolute() {
        bail!("Terminal worktree path must be absolute");
    }
    Ok(())
}

fn validate_terminal_process(
    process: &TerminalProcessFixture,
    scene: &WorkbenchScene,
) -> Result<()> {
    if process.terminal_id.trim().is_empty()
        || process.item_id.trim().is_empty()
        || process.title.trim().is_empty()
    {
        bail!("Terminal process, item, and title values must not be empty");
    }
    let owner = &process.owner;
    if owner.thread_id.trim().is_empty()
        || owner.repository_id.trim().is_empty()
        || owner.worktree_id.trim().is_empty()
        || owner.worktree_abs_path.trim().is_empty()
        || owner.initial_cwd.trim().is_empty()
        || process.current_cwd.trim().is_empty()
    {
        bail!("Terminal ownership and cwd values must not be empty");
    }
    if !Path::new(&owner.worktree_abs_path).is_absolute()
        || !Path::new(&owner.initial_cwd).is_absolute()
        || !Path::new(&process.current_cwd).is_absolute()
    {
        bail!("Terminal ownership and cwd paths must be absolute");
    }
    if !Path::new(&owner.initial_cwd).starts_with(&owner.worktree_abs_path) {
        bail!("Terminal initial cwd must remain inside its owning worktree");
    }
    let owner_thread = scene
        .threads
        .iter()
        .find(|thread| thread.id == owner.thread_id)
        .ok_or_else(|| {
            anyhow!(
                "Terminal owner references missing thread {:?}",
                owner.thread_id
            )
        })?;
    if owner_thread.repository_id.as_deref() != Some(owner.repository_id.as_str())
        || owner_thread.worktree_id.as_deref() != Some(owner.worktree_id.as_str())
    {
        bail!(
            "Terminal owner {:?} does not match its repository/worktree binding",
            owner.thread_id
        );
    }
    let owner_snapshot = scene
        .terminal_snapshots
        .iter()
        .find(|snapshot| snapshot.creation_binding.thread_id == owner.thread_id)
        .ok_or_else(|| {
            anyhow!(
                "Terminal owner {:?} has no typed creation binding",
                owner.thread_id
            )
        })?;
    let owner_binding = &owner_snapshot.creation_binding;
    if owner.repository_id != owner_binding.repository_id
        || owner.worktree_id != owner_binding.worktree_id
        || owner.worktree_abs_path != owner_binding.worktree_abs_path
        || owner.generation != owner_binding.generation
    {
        bail!(
            "Terminal process {:?} was relabeled away from its immutable owner",
            process.terminal_id
        );
    }
    match &process.lifecycle {
        TerminalProcessLifecycleFixture::Running { process_id } if *process_id == 0 => {
            bail!("running Terminal process ID must be non-zero");
        }
        TerminalProcessLifecycleFixture::FailedToSpawn(message) if message.trim().is_empty() => {
            bail!("failed Terminal process must contain a message");
        }
        _ => {}
    }
    Ok(())
}

fn collect_terminal_layout_panes<'a>(
    layout: &'a TerminalPaneLayoutFixture,
    pane_ids: &mut Vec<&'a str>,
) -> Result<()> {
    match layout {
        TerminalPaneLayoutFixture::Pane { pane_id } => {
            if pane_id.trim().is_empty() || pane_ids.contains(&pane_id.as_str()) {
                bail!("Terminal layout pane IDs must be non-empty and unique");
            }
            pane_ids.push(pane_id);
        }
        TerminalPaneLayoutFixture::Split { children, .. } => {
            if children.len() < 2 {
                bail!("Terminal split layout must contain at least two children");
            }
            for child in children {
                collect_terminal_layout_panes(child, pane_ids)?;
            }
        }
    }
    Ok(())
}

fn validate_terminal_focus(
    focus: &TerminalFocusFixture,
    snapshot: &TerminalSnapshotFixture,
) -> Result<()> {
    match focus {
        TerminalFocusFixture::Pane(pane_id)
            if !snapshot.panes.iter().any(|pane| pane.pane_id == *pane_id) =>
        {
            bail!("Terminal focus references missing pane {pane_id:?}");
        }
        TerminalFocusFixture::Terminal(terminal_id)
            if !snapshot
                .processes
                .iter()
                .any(|process| process.terminal_id == *terminal_id) =>
        {
            bail!("Terminal focus references missing process {terminal_id:?}");
        }
        _ => {}
    }
    Ok(())
}

fn validate_terminal_spawn(
    spawn: &TerminalSpawnFixture,
    snapshot: &TerminalSnapshotFixture,
) -> Result<()> {
    if spawn.request_id.trim().is_empty() || spawn.requested_cwd.trim().is_empty() {
        bail!("Terminal spawn request ID and cwd must not be empty");
    }
    validate_terminal_binding(&spawn.binding)?;
    if !Path::new(&spawn.requested_cwd).is_absolute()
        || !Path::new(&spawn.requested_cwd).starts_with(&spawn.binding.worktree_abs_path)
    {
        bail!("Terminal spawn cwd must remain inside the requested worktree");
    }
    let matches_creation_binding = spawn.binding == snapshot.creation_binding;
    match &spawn.result {
        TerminalSpawnResultFixture::Pending | TerminalSpawnResultFixture::Failed(_)
            if !matches_creation_binding =>
        {
            bail!("live Terminal spawn request does not match the current creation binding");
        }
        TerminalSpawnResultFixture::IgnoredStale if matches_creation_binding => {
            bail!("ignored Terminal spawn completion is not stale");
        }
        TerminalSpawnResultFixture::RejectedForeignBinding if matches_creation_binding => {
            bail!("rejected Terminal spawn request is not foreign");
        }
        TerminalSpawnResultFixture::Started { terminal_id } => {
            let process = snapshot
                .processes
                .iter()
                .find(|process| process.terminal_id == *terminal_id)
                .ok_or_else(|| {
                    anyhow!("Terminal spawn references missing process {terminal_id:?}")
                })?;
            if process.owner.thread_id != spawn.binding.thread_id
                || process.owner.repository_id != spawn.binding.repository_id
                || process.owner.worktree_id != spawn.binding.worktree_id
                || process.owner.worktree_abs_path != spawn.binding.worktree_abs_path
                || process.owner.generation != spawn.binding.generation
                || process.owner.initial_cwd != spawn.requested_cwd
            {
                bail!("started Terminal process does not retain its spawn owner");
            }
        }
        TerminalSpawnResultFixture::Failed(message) if message.trim().is_empty() => {
            bail!("failed Terminal spawn must contain a message");
        }
        _ => {}
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneSpec {
    pub name: &'static str,
    pub phase: ScenePhase,
    pub viewport: ViewportFixture,
    pub fixture_version: u32,
    pub pixel_policy: PixelPolicy,
    pub regions: &'static [CaptureRegionSpec],
}

impl SceneSpec {
    pub fn fixture(self) -> WorkbenchScene {
        let mut scene = WorkbenchScene::empty(self.name, self.viewport);
        scene.fixture_version = self.fixture_version;
        scene
    }
}

pub const WORKBENCH_TERMINAL_PIXEL_SCENES: [&str; 19] = [
    "omega_workbench_terminal_empty",
    "omega_workbench_terminal_starting",
    "omega_workbench_terminal_running",
    "omega_workbench_terminal_typed_input",
    "omega_workbench_terminal_multiple_tabs",
    "omega_workbench_terminal_split",
    "omega_workbench_terminal_narrow",
    "omega_workbench_terminal_exited",
    "omega_workbench_terminal_failed_to_spawn",
    "omega_workbench_terminal_hidden_running",
    "omega_workbench_terminal_collapse_reopen",
    "omega_workbench_terminal_focus_return",
    "omega_workbench_terminal_worktree_removed",
    "omega_workbench_terminal_offline",
    "omega_workbench_terminal_reconnecting",
    "omega_workbench_terminal_thread_switch",
    "omega_workbench_terminal_stale_spawn",
    "omega_workbench_terminal_foreign_spawn_rejected",
    "omega_workbench_terminal_error",
];

pub const WORKBENCH_SHELL_PIXEL_SCENES: [&str; 66] = [
    "omega_workbench_shell_default",
    "omega_workbench_shell_active_dock",
    "omega_workbench_shell_focus_visible",
    "omega_workbench_shell_typed_badge",
    "omega_workbench_shell_unavailable_no_project",
    "omega_workbench_shell_narrow",
    "omega_workbench_shell_collapsed_after_open",
    "omega_workbench_files_wide",
    "omega_workbench_files_narrow",
    "omega_workbench_files_multi_root",
    "omega_workbench_files_empty",
    "omega_workbench_files_loading",
    "omega_workbench_files_error",
    "omega_workbench_files_stale_filesystem_completion",
    "omega_workbench_search_empty",
    "omega_workbench_search_populated",
    "omega_workbench_search_no_results",
    "omega_workbench_search_invalid_regex",
    "omega_workbench_search_loading",
    "omega_workbench_search_narrow",
    "omega_workbench_search_focused_result",
    "omega_workbench_search_error",
    "omega_workbench_review_empty",
    "omega_workbench_review_multi_file",
    "omega_workbench_review_selected_hunk",
    "omega_workbench_review_streaming_update",
    "omega_workbench_review_rename_delete",
    "omega_workbench_review_conflict",
    "omega_workbench_review_all_reviewed",
    "omega_workbench_review_narrow",
    "omega_workbench_review_error",
    "omega_workbench_git_clean",
    "omega_workbench_git_dirty",
    "omega_workbench_git_staged",
    "omega_workbench_git_conflict",
    "omega_workbench_git_detached",
    "omega_workbench_git_unborn",
    "omega_workbench_git_pending",
    "omega_workbench_git_multi_repository",
    "omega_workbench_git_repository_removed",
    "omega_workbench_git_offline",
    "omega_workbench_git_reconnect",
    "omega_workbench_git_error",
    "omega_workbench_terminal_empty",
    "omega_workbench_terminal_starting",
    "omega_workbench_terminal_running",
    "omega_workbench_terminal_typed_input",
    "omega_workbench_terminal_multiple_tabs",
    "omega_workbench_terminal_split",
    "omega_workbench_terminal_narrow",
    "omega_workbench_terminal_exited",
    "omega_workbench_terminal_failed_to_spawn",
    "omega_workbench_terminal_hidden_running",
    "omega_workbench_terminal_collapse_reopen",
    "omega_workbench_terminal_focus_return",
    "omega_workbench_terminal_worktree_removed",
    "omega_workbench_terminal_offline",
    "omega_workbench_terminal_reconnecting",
    "omega_workbench_terminal_thread_switch",
    "omega_workbench_terminal_stale_spawn",
    "omega_workbench_terminal_foreign_spawn_rejected",
    "omega_workbench_terminal_error",
    "omega_workbench_identity_clean",
    "omega_workbench_identity_dirty_conflict",
    "omega_workbench_identity_long_narrow",
    "omega_workbench_identity_offline_error",
];

pub const WORKBENCH_SHELL_REGIONS: &[CaptureRegionSpec] = &[CaptureRegionSpec::selector_union(
    "rail-dock",
    &["omega.workbench.activity-rail", "omega.workbench.dock"],
    8,
)];

pub const WORKBENCH_IDENTITY_REGIONS: &[CaptureRegionSpec] = &[CaptureRegionSpec::selector_union(
    "thread-identity",
    &["omega.workbench.thread-identity"],
    8,
)];

pub const WORKBENCH_FILES_REGIONS: &[CaptureRegionSpec] = &[CaptureRegionSpec::selector_union(
    "files-surface",
    &["omega.workbench.surface.files"],
    8,
)];

pub const WORKBENCH_SEARCH_REGIONS: &[CaptureRegionSpec] = &[CaptureRegionSpec::selector_union(
    "search-surface",
    &["omega.workbench.surface.search"],
    8,
)];

pub const WORKBENCH_REVIEW_REGIONS: &[CaptureRegionSpec] = &[CaptureRegionSpec::selector_union(
    "review-surface",
    &["omega.workbench.surface.review"],
    8,
)];

pub const WORKBENCH_GIT_REGIONS: &[CaptureRegionSpec] = &[CaptureRegionSpec::selector_union(
    "git-surface",
    &["omega.workbench.surface.git"],
    8,
)];

pub const WORKBENCH_TERMINAL_REGIONS: &[CaptureRegionSpec] = &[CaptureRegionSpec::selector_union(
    "terminal-surface",
    &["omega.workbench.surface.terminal"],
    8,
)];

pub const HERMETIC_SCENES: &[SceneSpec] = &[
    SceneSpec {
        name: "omega_workbench_shell_default",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SHELL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_shell_active_dock",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SHELL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_shell_focus_visible",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SHELL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_shell_typed_badge",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SHELL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_shell_unavailable_no_project",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SHELL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_shell_narrow",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(909, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SHELL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_shell_collapsed_after_open",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SHELL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_files_wide",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_FILES_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_files_narrow",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(910, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_FILES_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_files_multi_root",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_FILES_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_files_empty",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_FILES_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_files_loading",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_FILES_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_files_error",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_FILES_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_files_stale_filesystem_completion",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_FILES_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_search_empty",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SEARCH_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_search_populated",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SEARCH_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_search_no_results",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SEARCH_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_search_invalid_regex",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SEARCH_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_search_loading",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SEARCH_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_search_narrow",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(910, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SEARCH_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_search_focused_result",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SEARCH_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_search_error",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_SEARCH_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_review_empty",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_REVIEW_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_review_multi_file",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_REVIEW_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_review_selected_hunk",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_REVIEW_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_review_streaming_update",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_REVIEW_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_review_rename_delete",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_REVIEW_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_review_conflict",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_REVIEW_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_review_all_reviewed",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_REVIEW_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_review_narrow",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(910, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_REVIEW_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_review_error",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_REVIEW_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_git_clean",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_GIT_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_git_dirty",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_GIT_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_git_staged",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_GIT_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_git_conflict",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_GIT_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_git_detached",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_GIT_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_git_unborn",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_GIT_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_git_pending",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_GIT_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_git_multi_repository",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_GIT_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_git_repository_removed",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_GIT_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_git_offline",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_GIT_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_git_reconnect",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_GIT_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_git_error",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_GIT_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_empty",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_starting",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_running",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_typed_input",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_multiple_tabs",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_split",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_narrow",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(910, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_exited",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_failed_to_spawn",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_hidden_running",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        // Dock is collapsed: no terminal-surface selector is on screen.
        // Whole-window pixels still prove the rail badge and transcript.
        regions: &[],
    },
    SceneSpec {
        name: "omega_workbench_terminal_collapse_reopen",
        // In-process SelectPlan → SelectTerminal proves collapse/reopen without
        // a cold process restart. Restart phase is reserved for durable
        // executor-disclosure scenes that write a handoff file.
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_focus_return",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_worktree_removed",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_offline",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_reconnecting",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_thread_switch",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_stale_spawn",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_foreign_spawn_rejected",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_terminal_error",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 2,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_TERMINAL_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_identity_clean",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_IDENTITY_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_identity_dirty_conflict",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_IDENTITY_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_identity_long_narrow",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(909, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_IDENTITY_REGIONS,
    },
    SceneSpec {
        name: "omega_workbench_identity_offline_error",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(1200, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: WORKBENCH_IDENTITY_REGIONS,
    },
    SceneSpec {
        name: "omega_front_door_no_project",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(900, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: &[],
    },
    SceneSpec {
        name: "omega_front_door_typing",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(900, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: &[],
    },
    SceneSpec {
        name: "omega_executor_disclosure_native",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(900, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: &[],
    },
    SceneSpec {
        name: "omega_route_pin_honoured",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(900, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: &[],
    },
    SceneSpec {
        name: "omega_route_pin_not_honoured",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(900, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: &[],
    },
    SceneSpec {
        name: "omega_executor_disclosure_external_acp",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(900, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: &[],
    },
    SceneSpec {
        name: "omega_executor_disclosure_engine_lane",
        phase: ScenePhase::Recording,
        viewport: ViewportFixture::new(900, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: &[],
    },
    SceneSpec {
        name: "omega_executor_disclosure_external_acp_after_restart",
        phase: ScenePhase::Restart,
        viewport: ViewportFixture::new(900, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: &[],
    },
    SceneSpec {
        name: "omega_executor_disclosure_engine_lane_after_restart",
        phase: ScenePhase::Restart,
        viewport: ViewportFixture::new(900, 720, 2000),
        fixture_version: 1,
        pixel_policy: APPLE_SILICON_METAL_POLICY,
        regions: &[],
    },
];

pub fn scene_spec(name: &str) -> Option<&'static SceneSpec> {
    HERMETIC_SCENES.iter().find(|scene| scene.name == name)
}

pub fn workbench_review_scene(name: &str) -> Result<WorkbenchScene> {
    let spec = scene_spec(name).ok_or_else(|| anyhow!("unknown workbench scene {name:?}"))?;
    if !name.starts_with("omega_workbench_review_") {
        bail!("{name:?} is not a Review workbench scene");
    }

    let mut scene = spec.fixture();
    scene.content_state = ContentStateFixture::Ready;
    scene.project = Some(ProjectFixture {
        id: "visual-project".into(),
        display_name: "Omega".into(),
    });
    scene.repositories.push(RepositoryFixture {
        id: "visual-repository".into(),
        project_id: "visual-project".into(),
        worktrees: vec![
            WorktreeFixture {
                id: "alpha-worktree".into(),
                branch: Some("alpha-review".into()),
                git_state: None,
                dirty_files: 1,
                conflicts: 0,
                ahead: 1,
                behind: 0,
            },
            WorktreeFixture {
                id: "beta-worktree".into(),
                branch: Some("beta-review".into()),
                git_state: None,
                dirty_files: if name == "omega_workbench_review_empty" {
                    0
                } else {
                    2
                },
                conflicts: u32::from(name == "omega_workbench_review_conflict"),
                ahead: 2,
                behind: 0,
            },
        ],
    });
    scene.threads = vec![
        ThreadFixture {
            id: "active-thread".into(),
            project_id: Some("visual-project".into()),
            repository_id: Some("visual-repository".into()),
            worktree_id: Some("beta-worktree".into()),
        },
        ThreadFixture {
            id: "foreign-thread".into(),
            project_id: Some("visual-project".into()),
            repository_id: Some("visual-repository".into()),
            worktree_id: Some("alpha-worktree".into()),
        },
    ];
    scene.active_thread_id = Some("active-thread".into());

    for surface in &mut scene.surfaces {
        surface.available = true;
        if surface.id == WorkSurfaceId::Git {
            surface.badge = match name {
                "omega_workbench_review_rename_delete" => Some(3),
                _ => None,
            };
        }
    }
    scene.active_surface = Some(WorkSurfaceId::Review);
    scene.dock_open = true;
    scene.thread_workbenches = vec![
        ThreadWorkbenchFixture {
            thread_id: "active-thread".into(),
            generation: 7,
            binding: Some(WorkbenchBindingFixture {
                repository_id: "visual-repository".into(),
                worktree_id: "beta-worktree".into(),
            }),
            requested_surface: Some(WorkSurfaceId::Review),
            effective_surface: Some(WorkSurfaceId::Review),
            dock_open: true,
            surfaces: scene.surfaces.clone(),
        },
        ThreadWorkbenchFixture {
            thread_id: "foreign-thread".into(),
            generation: 3,
            binding: Some(WorkbenchBindingFixture {
                repository_id: "visual-repository".into(),
                worktree_id: "alpha-worktree".into(),
            }),
            requested_surface: Some(WorkSurfaceId::Review),
            effective_surface: Some(WorkSurfaceId::Review),
            dock_open: false,
            surfaces: scene.surfaces.clone(),
        },
    ];

    let active_binding = ReviewBindingFixture {
        thread_id: "active-thread".into(),
        session_id: "beta-session".into(),
        repository_id: "visual-repository".into(),
        worktree_id: "beta-worktree".into(),
        checkpoint: ReviewCheckpointFixture {
            action_log_entity_id: "beta-action-log".into(),
            generation: 7,
        },
    };
    let active_review = review_fixture_for_scene(name, active_binding)?;
    let foreign_review = ReviewSessionFixture {
        binding: ReviewBindingFixture {
            thread_id: "foreign-thread".into(),
            session_id: "alpha-session".into(),
            repository_id: "visual-repository".into(),
            worktree_id: "alpha-worktree".into(),
            checkpoint: ReviewCheckpointFixture {
                action_log_entity_id: "alpha-action-log".into(),
                generation: 3,
            },
        },
        lifecycle: ReviewLifecycleFixture::Ready,
        files: vec![ReviewFileFixture {
            path: "src/foreign_thread_only.rs".into(),
            old_path: None,
            status: ReviewFileStatusFixture::Modified,
            hunks: vec![review_hunk(
                "foreign-hunk",
                0,
                0,
                1,
                0,
                ReviewHunkStatusFixture::Pending,
            )],
        }],
        selected_file_path: Some("src/foreign_thread_only.rs".into()),
        selected_hunk_id: Some("foreign-hunk".into()),
        focus: ReviewFocusFixture::Diff,
        mutations: Vec::new(),
        pending_operation_count: 0,
        ignored_stale_completion_count: 0,
    };
    scene.review_sessions = vec![active_review, foreign_review];
    scene.validate()?;
    Ok(scene)
}

fn review_fixture_for_scene(
    name: &str,
    binding: ReviewBindingFixture,
) -> Result<ReviewSessionFixture> {
    let standard_files = || {
        vec![
            ReviewFileFixture {
                path: "src/main.rs".into(),
                old_path: None,
                status: ReviewFileStatusFixture::Modified,
                hunks: vec![
                    review_hunk("main-imports", 0, 0, 1, 0, ReviewHunkStatusFixture::Pending),
                    review_hunk("main-body", 20, 0, 21, 0, ReviewHunkStatusFixture::Pending),
                ],
            },
            ReviewFileFixture {
                path: "src/settings.rs".into(),
                old_path: None,
                status: ReviewFileStatusFixture::Added,
                hunks: vec![review_hunk(
                    "settings-new",
                    0,
                    0,
                    1,
                    0,
                    ReviewHunkStatusFixture::Pending,
                )],
            },
        ]
    };

    let mut fixture = ReviewSessionFixture {
        binding,
        lifecycle: ReviewLifecycleFixture::Ready,
        files: standard_files(),
        selected_file_path: Some("src/main.rs".into()),
        selected_hunk_id: Some("main-imports".into()),
        focus: ReviewFocusFixture::Diff,
        mutations: Vec::new(),
        pending_operation_count: 0,
        ignored_stale_completion_count: 0,
    };

    match name {
        "omega_workbench_review_empty" => {
            fixture.lifecycle = ReviewLifecycleFixture::Empty;
            fixture.files.clear();
            fixture.selected_file_path = None;
            fixture.selected_hunk_id = None;
            fixture.focus = ReviewFocusFixture::Surface;
        }
        "omega_workbench_review_multi_file" | "omega_workbench_review_narrow" => {}
        "omega_workbench_review_selected_hunk" => {
            fixture.selected_hunk_id = Some("main-body".into());
        }
        "omega_workbench_review_streaming_update" => {
            fixture.lifecycle = ReviewLifecycleFixture::Streaming;
            fixture.files[0].hunks.push(review_hunk(
                "main-streamed",
                35,
                0,
                36,
                0,
                ReviewHunkStatusFixture::Pending,
            ));
            fixture.selected_hunk_id = Some("main-body".into());
            fixture.pending_operation_count = 1;
            fixture.ignored_stale_completion_count = 1;
        }
        "omega_workbench_review_rename_delete" => {
            fixture.files = vec![
                ReviewFileFixture {
                    path: "src/current_name.rs".into(),
                    old_path: Some("src/previous_name.rs".into()),
                    status: ReviewFileStatusFixture::Renamed,
                    hunks: vec![review_hunk(
                        "rename-edit",
                        1,
                        0,
                        2,
                        0,
                        ReviewHunkStatusFixture::Pending,
                    )],
                },
                ReviewFileFixture {
                    path: "src/obsolete.rs".into(),
                    old_path: None,
                    status: ReviewFileStatusFixture::Deleted,
                    hunks: vec![review_hunk(
                        "delete-file",
                        0,
                        0,
                        0,
                        0,
                        ReviewHunkStatusFixture::Pending,
                    )],
                },
            ];
            fixture.selected_file_path = Some("src/current_name.rs".into());
            fixture.selected_hunk_id = Some("rename-edit".into());
        }
        "omega_workbench_review_conflict" => {
            fixture.files = vec![ReviewFileFixture {
                path: "src/conflicted.rs".into(),
                old_path: None,
                status: ReviewFileStatusFixture::Conflict,
                hunks: vec![review_hunk(
                    "conflict-hunk",
                    1,
                    0,
                    2,
                    0,
                    ReviewHunkStatusFixture::Conflict,
                )],
            }];
            fixture.selected_file_path = Some("src/conflicted.rs".into());
            fixture.selected_hunk_id = Some("conflict-hunk".into());
        }
        "omega_workbench_review_all_reviewed" => {
            fixture.lifecycle = ReviewLifecycleFixture::AllReviewed;
            fixture.files.clear();
            fixture.selected_file_path = None;
            fixture.selected_hunk_id = None;
            fixture.focus = ReviewFocusFixture::Surface;
            fixture.mutations = vec![
                ReviewMutationFixture {
                    kind: ReviewMutationKindFixture::KeepHunk,
                    file_path: Some("src/main.rs".into()),
                    hunk_id: Some("main-imports".into()),
                    resulting_contents: Some("use omega::review;\n".into()),
                },
                ReviewMutationFixture {
                    kind: ReviewMutationKindFixture::RejectHunk,
                    file_path: Some("src/main.rs".into()),
                    hunk_id: Some("main-body".into()),
                    resulting_contents: Some("const REVIEW_MODE: bool = false;\n".into()),
                },
                ReviewMutationFixture {
                    kind: ReviewMutationKindFixture::KeepHunk,
                    file_path: Some("src/settings.rs".into()),
                    hunk_id: Some("settings-new".into()),
                    resulting_contents: Some("pub const REVIEW_ENABLED: bool = true;\n".into()),
                },
            ];
        }
        "omega_workbench_review_error" => {
            fixture.lifecycle =
                ReviewLifecycleFixture::Error("Could not load this checkpoint".into());
            fixture.files.clear();
            fixture.selected_file_path = None;
            fixture.selected_hunk_id = None;
            fixture.focus = ReviewFocusFixture::Surface;
        }
        _ => bail!("unknown Review workbench scene {name:?}"),
    }
    Ok(fixture)
}

fn review_hunk(
    id: &str,
    start_row: u32,
    start_column: u32,
    end_row: u32,
    end_column: u32,
    status: ReviewHunkStatusFixture,
) -> ReviewHunkFixture {
    ReviewHunkFixture {
        id: id.into(),
        start_row,
        start_column,
        end_row,
        end_column,
        status,
    }
}

pub fn workbench_git_scene(name: &str) -> Result<WorkbenchScene> {
    let spec = scene_spec(name).ok_or_else(|| anyhow!("unknown workbench scene {name:?}"))?;
    if !name.starts_with("omega_workbench_git_") {
        bail!("{name:?} is not a Git workbench scene");
    }

    let active_snapshot = git_fixture_for_scene(
        name,
        GitBindingFixture {
            thread_id: "active-thread".into(),
            repository_id: "visual-repository-beta".into(),
            worktree_id: "beta-worktree".into(),
            repository_entity_id: "beta-repository-entity".into(),
            generation: 11,
        },
    )?;
    let foreign_entries = vec![GitStatusEntryFixture {
        path: "foreign/alpha_only.rs".into(),
        old_path: None,
        status: GitFileStatusFixture::Modified,
        staging: GitStagingStateFixture::Unstaged,
    }];
    let foreign_snapshot = GitSnapshotFixture {
        binding: GitBindingFixture {
            thread_id: "foreign-thread".into(),
            repository_id: "visual-repository-alpha".into(),
            worktree_id: "alpha-worktree".into(),
            repository_entity_id: "alpha-repository-entity".into(),
            generation: 4,
        },
        lifecycle: GitLifecycleFixture::Ready,
        branch: Some(GitBranchFixture::Branch {
            name: "alpha-work".into(),
            ahead: 1,
            behind: 0,
        }),
        status_counts: git_status_counts(&foreign_entries)?,
        status_entries: foreign_entries,
        selected_path: Some("foreign/alpha_only.rs".into()),
        pending_operation: None,
        badge_count: 1,
        requested_mutations: Vec::new(),
        ignored_stale_refresh_count: 0,
        focus: GitFocusFixture::StatusEntry,
    };

    let mut scene = spec.fixture();
    scene.content_state = ContentStateFixture::Ready;
    scene.connectivity = ConnectivityFixture::Online;
    scene.project = Some(ProjectFixture {
        id: "visual-project".into(),
        display_name: "Omega".into(),
    });
    scene.repositories = vec![
        RepositoryFixture {
            id: "visual-repository-alpha".into(),
            project_id: "visual-project".into(),
            worktrees: vec![WorktreeFixture {
                id: "alpha-worktree".into(),
                branch: Some("alpha-work".into()),
                git_state: None,
                dirty_files: foreign_snapshot.badge_count,
                conflicts: foreign_snapshot.status_counts.conflicts,
                ahead: 1,
                behind: 0,
            }],
        },
        RepositoryFixture {
            id: "visual-repository-beta".into(),
            project_id: "visual-project".into(),
            worktrees: vec![git_worktree_for_snapshot(&active_snapshot)?],
        },
    ];
    scene.threads = vec![
        ThreadFixture {
            id: "active-thread".into(),
            project_id: Some("visual-project".into()),
            repository_id: Some("visual-repository-beta".into()),
            worktree_id: Some("beta-worktree".into()),
        },
        ThreadFixture {
            id: "foreign-thread".into(),
            project_id: Some("visual-project".into()),
            repository_id: Some("visual-repository-alpha".into()),
            worktree_id: Some("alpha-worktree".into()),
        },
    ];
    scene.active_thread_id = Some("active-thread".into());

    for surface in &mut scene.surfaces {
        surface.available = true;
        if surface.id == WorkSurfaceId::Git {
            surface.badge =
                (active_snapshot.badge_count > 0).then_some(active_snapshot.badge_count);
        }
    }
    scene.active_surface = Some(WorkSurfaceId::Git);
    scene.dock_open = true;

    let active_surfaces = scene.surfaces.clone();
    let mut foreign_surfaces = scene.surfaces.clone();
    let foreign_git_surface = foreign_surfaces
        .iter_mut()
        .find(|surface| surface.id == WorkSurfaceId::Git)
        .context("Git surface fixture is missing")?;
    foreign_git_surface.badge = Some(foreign_snapshot.badge_count);
    scene.thread_workbenches = vec![
        ThreadWorkbenchFixture {
            thread_id: "active-thread".into(),
            generation: active_snapshot.binding.generation,
            binding: Some(WorkbenchBindingFixture {
                repository_id: active_snapshot.binding.repository_id.clone(),
                worktree_id: active_snapshot.binding.worktree_id.clone(),
            }),
            requested_surface: Some(WorkSurfaceId::Git),
            effective_surface: Some(WorkSurfaceId::Git),
            dock_open: true,
            surfaces: active_surfaces,
        },
        ThreadWorkbenchFixture {
            thread_id: "foreign-thread".into(),
            generation: foreign_snapshot.binding.generation,
            binding: Some(WorkbenchBindingFixture {
                repository_id: foreign_snapshot.binding.repository_id.clone(),
                worktree_id: foreign_snapshot.binding.worktree_id.clone(),
            }),
            requested_surface: Some(WorkSurfaceId::Git),
            effective_surface: Some(WorkSurfaceId::Git),
            dock_open: false,
            surfaces: foreign_surfaces,
        },
    ];
    scene.git_snapshots = vec![active_snapshot, foreign_snapshot];
    scene.validate()?;
    Ok(scene)
}

fn git_fixture_for_scene(name: &str, binding: GitBindingFixture) -> Result<GitSnapshotFixture> {
    let mut fixture = GitSnapshotFixture {
        binding,
        lifecycle: GitLifecycleFixture::Ready,
        branch: Some(GitBranchFixture::Branch {
            name: "codex/git-surface".into(),
            ahead: 0,
            behind: 0,
        }),
        status_entries: Vec::new(),
        status_counts: GitStatusCountsFixture::default(),
        selected_path: None,
        pending_operation: None,
        badge_count: 0,
        requested_mutations: Vec::new(),
        ignored_stale_refresh_count: 0,
        focus: GitFocusFixture::Surface,
    };

    match name {
        "omega_workbench_git_clean" => {}
        "omega_workbench_git_dirty" => {
            fixture.branch = Some(GitBranchFixture::Branch {
                name: "codex/git-surface".into(),
                ahead: 2,
                behind: 1,
            });
            fixture.status_entries = vec![
                git_status(
                    "README.md",
                    GitFileStatusFixture::Modified,
                    GitStagingStateFixture::Unstaged,
                ),
                git_status(
                    "src/new.rs",
                    GitFileStatusFixture::Untracked,
                    GitStagingStateFixture::Unstaged,
                ),
            ];
            fixture.selected_path = Some("README.md".into());
            fixture.focus = GitFocusFixture::StatusEntry;
            fixture.requested_mutations.push(GitMutationFixture {
                kind: GitOperationKindFixture::OpenDiff,
                target: GitMutationTargetFixture::Path("README.md".into()),
                result: GitMutationResultFixture::Succeeded,
            });
        }
        "omega_workbench_git_staged" => {
            fixture.status_entries = vec![git_status(
                "src/main.rs",
                GitFileStatusFixture::Modified,
                GitStagingStateFixture::Staged,
            )];
            fixture.selected_path = Some("src/main.rs".into());
            fixture.focus = GitFocusFixture::StatusList;
            fixture.requested_mutations.push(GitMutationFixture {
                kind: GitOperationKindFixture::Stage,
                target: GitMutationTargetFixture::Path("src/main.rs".into()),
                result: GitMutationResultFixture::Succeeded,
            });
        }
        "omega_workbench_git_conflict" => {
            fixture.status_entries = vec![git_status(
                "src/conflicted.rs",
                GitFileStatusFixture::Conflicted,
                GitStagingStateFixture::Conflict,
            )];
            fixture.selected_path = Some("src/conflicted.rs".into());
            fixture.focus = GitFocusFixture::StatusEntry;
            fixture.requested_mutations.push(GitMutationFixture {
                kind: GitOperationKindFixture::Discard,
                target: GitMutationTargetFixture::Path("src/conflicted.rs".into()),
                result: GitMutationResultFixture::Cancelled,
            });
        }
        "omega_workbench_git_detached" => {
            fixture.branch = Some(GitBranchFixture::Detached {
                head: "daac524".into(),
            });
            fixture.status_entries = vec![git_status(
                "src/detached.rs",
                GitFileStatusFixture::Modified,
                GitStagingStateFixture::Unstaged,
            )];
            fixture.selected_path = Some("src/detached.rs".into());
            fixture.focus = GitFocusFixture::BranchMenu;
        }
        "omega_workbench_git_unborn" => {
            fixture.branch = Some(GitBranchFixture::Unborn {
                name: "omega/initial".into(),
            });
            fixture.status_entries = vec![git_status(
                "README.md",
                GitFileStatusFixture::Untracked,
                GitStagingStateFixture::Unstaged,
            )];
            fixture.selected_path = Some("README.md".into());
            fixture.focus = GitFocusFixture::CommitEditor;
        }
        "omega_workbench_git_pending" => {
            fixture.status_entries = vec![git_status(
                "src/main.rs",
                GitFileStatusFixture::Modified,
                GitStagingStateFixture::Staged,
            )];
            fixture.selected_path = Some("src/main.rs".into());
            fixture.focus = GitFocusFixture::CommitEditor;
            let target = GitMutationTargetFixture::CommitMessage("Mount native Git surface".into());
            fixture.pending_operation = Some(GitPendingOperationFixture {
                id: "commit-operation".into(),
                kind: GitOperationKindFixture::Commit,
                target: target.clone(),
                confirmation_required: false,
            });
            fixture.requested_mutations.push(GitMutationFixture {
                kind: GitOperationKindFixture::Commit,
                target,
                result: GitMutationResultFixture::Pending,
            });
        }
        "omega_workbench_git_multi_repository" => {
            fixture.status_entries = vec![git_status(
                "src/beta.rs",
                GitFileStatusFixture::Modified,
                GitStagingStateFixture::Unstaged,
            )];
            fixture.selected_path = Some("src/beta.rs".into());
            fixture.focus = GitFocusFixture::StatusEntry;
            fixture.requested_mutations.push(GitMutationFixture {
                kind: GitOperationKindFixture::SwitchRepository,
                target: GitMutationTargetFixture::Binding {
                    repository_id: "visual-repository-beta".into(),
                    worktree_id: "beta-worktree".into(),
                },
                result: GitMutationResultFixture::Succeeded,
            });
        }
        "omega_workbench_git_repository_removed" => {
            fixture.lifecycle = GitLifecycleFixture::RepositoryRemoved;
            fixture.branch = None;
            fixture.ignored_stale_refresh_count = 1;
            fixture.requested_mutations.push(GitMutationFixture {
                kind: GitOperationKindFixture::Refresh,
                target: GitMutationTargetFixture::None,
                result: GitMutationResultFixture::Rejected,
            });
        }
        "omega_workbench_git_offline" => {
            fixture.lifecycle = GitLifecycleFixture::Offline;
            fixture.status_entries = vec![git_status(
                "src/offline.rs",
                GitFileStatusFixture::Modified,
                GitStagingStateFixture::Unstaged,
            )];
            fixture.selected_path = Some("src/offline.rs".into());
            fixture.focus = GitFocusFixture::Toolbar;
            fixture.requested_mutations.push(GitMutationFixture {
                kind: GitOperationKindFixture::Pull,
                target: GitMutationTargetFixture::None,
                result: GitMutationResultFixture::Rejected,
            });
        }
        "omega_workbench_git_reconnect" => {
            fixture.lifecycle = GitLifecycleFixture::Reconnecting;
            fixture.branch = Some(GitBranchFixture::Branch {
                name: "codex/git-surface".into(),
                ahead: 3,
                behind: 1,
            });
            fixture.status_entries = vec![git_status(
                "src/reconnected.rs",
                GitFileStatusFixture::Modified,
                GitStagingStateFixture::Unstaged,
            )];
            fixture.selected_path = Some("src/reconnected.rs".into());
            fixture.focus = GitFocusFixture::StatusList;
            fixture.ignored_stale_refresh_count = 1;
            fixture.requested_mutations.push(GitMutationFixture {
                kind: GitOperationKindFixture::Refresh,
                target: GitMutationTargetFixture::None,
                result: GitMutationResultFixture::Succeeded,
            });
        }
        "omega_workbench_git_error" => {
            fixture.lifecycle =
                GitLifecycleFixture::Error("Could not refresh repository status".into());
            fixture.requested_mutations.push(GitMutationFixture {
                kind: GitOperationKindFixture::Commit,
                target: GitMutationTargetFixture::CommitMessage("Mount native Git surface".into()),
                result: GitMutationResultFixture::Failed("pre-commit hook failed".into()),
            });
        }
        _ => bail!("unknown Git workbench scene {name:?}"),
    }

    fixture.status_counts = git_status_counts(&fixture.status_entries)?;
    fixture.badge_count = u32::try_from(fixture.status_entries.len())
        .context("Git fixture contains more status entries than the badge can represent")?;
    Ok(fixture)
}

fn git_worktree_for_snapshot(snapshot: &GitSnapshotFixture) -> Result<WorktreeFixture> {
    let (branch, git_state, ahead, behind) = match &snapshot.branch {
        Some(GitBranchFixture::Branch {
            name,
            ahead,
            behind,
        }) => (Some(name.clone()), None, *ahead, *behind),
        Some(GitBranchFixture::Detached { .. }) => (None, None, 0, 0),
        Some(GitBranchFixture::Unborn { name }) => (
            Some(name.clone()),
            Some(WorktreeGitStateFixture::Unborn),
            0,
            0,
        ),
        None => (Some("codex/git-surface".into()), None, 0, 0),
    };
    Ok(WorktreeFixture {
        id: snapshot.binding.worktree_id.clone(),
        branch,
        git_state,
        dirty_files: snapshot.badge_count,
        conflicts: snapshot.status_counts.conflicts,
        ahead,
        behind,
    })
}

fn git_status(
    path: &str,
    status: GitFileStatusFixture,
    staging: GitStagingStateFixture,
) -> GitStatusEntryFixture {
    GitStatusEntryFixture {
        path: path.into(),
        old_path: None,
        status,
        staging,
    }
}

pub fn workbench_terminal_scene(name: &str) -> Result<WorkbenchScene> {
    let spec = scene_spec(name).ok_or_else(|| anyhow!("unknown workbench scene {name:?}"))?;
    if !name.starts_with("omega_workbench_terminal_") {
        bail!("{name:?} is not a Terminal workbench scene");
    }

    let active_binding = TerminalBindingFixture {
        thread_id: "active-thread".into(),
        repository_id: "visual-repository-beta".into(),
        worktree_id: "beta-worktree".into(),
        worktree_abs_path: "/workspace/beta".into(),
        generation: 11,
    };
    let foreign_binding = TerminalBindingFixture {
        thread_id: "foreign-thread".into(),
        repository_id: "visual-repository-alpha".into(),
        worktree_id: "alpha-worktree".into(),
        worktree_abs_path: "/workspace/alpha".into(),
        generation: 4,
    };
    let active_snapshot = terminal_fixture_for_scene(
        name,
        active_binding.clone(),
        &foreign_binding,
        "workspace-terminal-panel",
    )?;
    let foreign_process = terminal_process(
        "terminal-alpha",
        "terminal-item-alpha",
        "alpha shell",
        terminal_owner(&foreign_binding, "/workspace/alpha"),
        TerminalProcessLifecycleFixture::Running { process_id: 4104 },
    );
    let foreign_snapshot = TerminalSnapshotFixture {
        creation_binding: foreign_binding.clone(),
        panel_entity_id: "workspace-terminal-panel".into(),
        lifecycle: TerminalLifecycleFixture::Ready,
        panes: vec![TerminalPaneFixture {
            pane_id: "foreign-pane".into(),
            terminal_ids: vec![foreign_process.terminal_id.clone()],
            active_terminal_id: Some(foreign_process.terminal_id.clone()),
        }],
        pane_layout: TerminalPaneLayoutFixture::Pane {
            pane_id: "foreign-pane".into(),
        },
        processes: vec![foreign_process.clone()],
        selected_terminal_id: Some(foreign_process.terminal_id.clone()),
        requested_spawns: Vec::new(),
        running_badge_count: 1,
        implicit_spawn_count: 0,
        ignored_stale_completion_count: 0,
        rejected_foreign_spawn_count: 0,
        focus: TerminalFocusFixture::Terminal(foreign_process.terminal_id),
    };

    let mut scene = spec.fixture();
    scene.content_state = ContentStateFixture::Ready;
    scene.connectivity = match name {
        "omega_workbench_terminal_offline" => ConnectivityFixture::Offline,
        "omega_workbench_terminal_reconnecting" => ConnectivityFixture::Reconnecting,
        _ => ConnectivityFixture::Online,
    };
    scene.project = Some(ProjectFixture {
        id: "visual-project".into(),
        display_name: "Omega".into(),
    });
    scene.repositories = vec![
        RepositoryFixture {
            id: foreign_binding.repository_id.clone(),
            project_id: "visual-project".into(),
            worktrees: vec![WorktreeFixture {
                id: foreign_binding.worktree_id.clone(),
                branch: Some("alpha-work".into()),
                git_state: None,
                dirty_files: 0,
                conflicts: 0,
                ahead: 0,
                behind: 0,
            }],
        },
        RepositoryFixture {
            id: active_binding.repository_id.clone(),
            project_id: "visual-project".into(),
            worktrees: vec![WorktreeFixture {
                id: active_binding.worktree_id.clone(),
                branch: Some("codex/terminal-surface".into()),
                git_state: None,
                dirty_files: 0,
                conflicts: 0,
                ahead: 0,
                behind: 0,
            }],
        },
    ];
    scene.threads = vec![
        ThreadFixture {
            id: active_binding.thread_id.clone(),
            project_id: Some("visual-project".into()),
            repository_id: Some(active_binding.repository_id.clone()),
            worktree_id: Some(active_binding.worktree_id.clone()),
        },
        ThreadFixture {
            id: foreign_binding.thread_id.clone(),
            project_id: Some("visual-project".into()),
            repository_id: Some(foreign_binding.repository_id.clone()),
            worktree_id: Some(foreign_binding.worktree_id.clone()),
        },
    ];
    scene.active_thread_id = Some(active_binding.thread_id.clone());
    for surface in &mut scene.surfaces {
        surface.available = true;
        if surface.id == WorkSurfaceId::Terminal {
            surface.badge = (active_snapshot.running_badge_count > 0)
                .then_some(active_snapshot.running_badge_count);
        }
    }
    scene.active_surface = Some(WorkSurfaceId::Terminal);
    scene.dock_open = name != "omega_workbench_terminal_hidden_running";
    let active_surfaces = scene.surfaces.clone();
    let mut foreign_surfaces = scene.surfaces.clone();
    surface_fixture_mut(&mut foreign_surfaces, WorkSurfaceId::Terminal)?.badge =
        Some(foreign_snapshot.running_badge_count);
    scene.thread_workbenches = vec![
        ThreadWorkbenchFixture {
            thread_id: active_binding.thread_id.clone(),
            generation: active_binding.generation,
            binding: Some(WorkbenchBindingFixture {
                repository_id: active_binding.repository_id.clone(),
                worktree_id: active_binding.worktree_id.clone(),
            }),
            requested_surface: Some(WorkSurfaceId::Terminal),
            effective_surface: Some(WorkSurfaceId::Terminal),
            dock_open: scene.dock_open,
            surfaces: active_surfaces,
        },
        ThreadWorkbenchFixture {
            thread_id: foreign_binding.thread_id.clone(),
            generation: foreign_binding.generation,
            binding: Some(WorkbenchBindingFixture {
                repository_id: foreign_binding.repository_id.clone(),
                worktree_id: foreign_binding.worktree_id.clone(),
            }),
            requested_surface: Some(WorkSurfaceId::Terminal),
            effective_surface: Some(WorkSurfaceId::Terminal),
            dock_open: false,
            surfaces: foreign_surfaces,
        },
    ];
    scene.terminal_snapshots = vec![active_snapshot, foreign_snapshot];
    if name == "omega_workbench_terminal_collapse_reopen" {
        scene.persisted = Some(PersistedSceneFixture {
            requested_surface: Some(WorkSurfaceId::Terminal),
            dock_open: true,
            revision: 9,
            mutations_before_restart: vec![
                SceneMutation::SetActiveSurface {
                    surface: Some(WorkSurfaceId::Terminal),
                },
                SceneMutation::SetDockOpen { open: false },
                SceneMutation::SetDockOpen { open: true },
            ],
        });
    }
    scene.validate()?;
    Ok(scene)
}

fn terminal_fixture_for_scene(
    name: &str,
    binding: TerminalBindingFixture,
    foreign_binding: &TerminalBindingFixture,
    panel_entity_id: &str,
) -> Result<TerminalSnapshotFixture> {
    let active_owner = terminal_owner(&binding, "/workspace/beta");
    let running_process = || {
        terminal_process(
            "terminal-beta",
            "terminal-item-beta",
            "beta shell",
            active_owner.clone(),
            TerminalProcessLifecycleFixture::Running { process_id: 1117 },
        )
    };
    let mut snapshot = TerminalSnapshotFixture {
        creation_binding: binding.clone(),
        panel_entity_id: panel_entity_id.into(),
        lifecycle: TerminalLifecycleFixture::Ready,
        panes: vec![TerminalPaneFixture {
            pane_id: "beta-pane".into(),
            terminal_ids: vec!["terminal-beta".into()],
            active_terminal_id: Some("terminal-beta".into()),
        }],
        pane_layout: TerminalPaneLayoutFixture::Pane {
            pane_id: "beta-pane".into(),
        },
        processes: vec![running_process()],
        selected_terminal_id: Some("terminal-beta".into()),
        requested_spawns: vec![TerminalSpawnFixture {
            request_id: "spawn-beta".into(),
            binding: binding.clone(),
            requested_cwd: "/workspace/beta".into(),
            result: TerminalSpawnResultFixture::Started {
                terminal_id: "terminal-beta".into(),
            },
        }],
        running_badge_count: 1,
        implicit_spawn_count: 0,
        ignored_stale_completion_count: 0,
        rejected_foreign_spawn_count: 0,
        focus: TerminalFocusFixture::Terminal("terminal-beta".into()),
    };

    match name {
        "omega_workbench_terminal_empty" => {
            snapshot.lifecycle = TerminalLifecycleFixture::Empty;
            snapshot.panes[0].terminal_ids.clear();
            snapshot.panes[0].active_terminal_id = None;
            snapshot.processes.clear();
            snapshot.selected_terminal_id = None;
            snapshot.requested_spawns.clear();
            snapshot.running_badge_count = 0;
            snapshot.focus = TerminalFocusFixture::NewTerminal;
        }
        "omega_workbench_terminal_starting" => {
            snapshot.lifecycle = TerminalLifecycleFixture::Starting;
            snapshot.processes[0].lifecycle = TerminalProcessLifecycleFixture::Starting;
            snapshot.requested_spawns[0].result = TerminalSpawnResultFixture::Pending;
        }
        "omega_workbench_terminal_running"
        | "omega_workbench_terminal_hidden_running"
        | "omega_workbench_terminal_collapse_reopen"
        | "omega_workbench_terminal_narrow" => {}
        "omega_workbench_terminal_typed_input" => {
            snapshot.processes[0].input_bytes =
                vec![b"cargo test -p omega_workbench_harness\r".to_vec()];
        }
        "omega_workbench_terminal_multiple_tabs" => {
            let second = terminal_process(
                "terminal-beta-tests",
                "terminal-item-beta-tests",
                "tests",
                active_owner,
                TerminalProcessLifecycleFixture::Running { process_id: 1129 },
            );
            snapshot.panes[0]
                .terminal_ids
                .push(second.terminal_id.clone());
            snapshot.panes[0].active_terminal_id = Some(second.terminal_id.clone());
            snapshot.selected_terminal_id = Some(second.terminal_id.clone());
            snapshot.processes.push(second.clone());
            snapshot.requested_spawns.push(TerminalSpawnFixture {
                request_id: "spawn-beta-tests".into(),
                binding,
                requested_cwd: "/workspace/beta".into(),
                result: TerminalSpawnResultFixture::Started {
                    terminal_id: second.terminal_id,
                },
            });
            snapshot.running_badge_count = 2;
        }
        "omega_workbench_terminal_split" => {
            let split = terminal_process(
                "terminal-beta-split",
                "terminal-item-beta-split",
                "split",
                active_owner,
                TerminalProcessLifecycleFixture::Running { process_id: 1133 },
            );
            snapshot.panes.push(TerminalPaneFixture {
                pane_id: "beta-split-pane".into(),
                terminal_ids: vec![split.terminal_id.clone()],
                active_terminal_id: Some(split.terminal_id.clone()),
            });
            snapshot.pane_layout = TerminalPaneLayoutFixture::Split {
                axis: TerminalSplitAxisFixture::Horizontal,
                children: vec![
                    TerminalPaneLayoutFixture::Pane {
                        pane_id: "beta-pane".into(),
                    },
                    TerminalPaneLayoutFixture::Pane {
                        pane_id: "beta-split-pane".into(),
                    },
                ],
            };
            snapshot.selected_terminal_id = Some(split.terminal_id.clone());
            snapshot.processes.push(split.clone());
            snapshot.requested_spawns.push(TerminalSpawnFixture {
                request_id: "spawn-beta-split".into(),
                binding,
                requested_cwd: "/workspace/beta".into(),
                result: TerminalSpawnResultFixture::Started {
                    terminal_id: split.terminal_id,
                },
            });
            snapshot.running_badge_count = 2;
        }
        "omega_workbench_terminal_exited" => {
            snapshot.processes[0].lifecycle =
                TerminalProcessLifecycleFixture::Exited { exit_code: 0 };
            snapshot.running_badge_count = 0;
            snapshot.focus = TerminalFocusFixture::Surface;
        }
        "omega_workbench_terminal_failed_to_spawn" => {
            snapshot.processes[0].lifecycle = TerminalProcessLifecycleFixture::FailedToSpawn(
                "configured shell was not found".into(),
            );
            snapshot.requested_spawns[0].result =
                TerminalSpawnResultFixture::Failed("configured shell was not found".into());
            snapshot.running_badge_count = 0;
            snapshot.focus = TerminalFocusFixture::Surface;
        }
        "omega_workbench_terminal_focus_return" => {
            snapshot.focus = TerminalFocusFixture::Transcript;
        }
        "omega_workbench_terminal_worktree_removed" => {
            snapshot.lifecycle = TerminalLifecycleFixture::WorktreeRemoved;
            snapshot.focus = TerminalFocusFixture::Surface;
        }
        "omega_workbench_terminal_offline" => {
            snapshot.lifecycle = TerminalLifecycleFixture::Offline;
            snapshot.focus = TerminalFocusFixture::Surface;
        }
        "omega_workbench_terminal_reconnecting" => {
            snapshot.lifecycle = TerminalLifecycleFixture::Reconnecting;
            snapshot.focus = TerminalFocusFixture::Surface;
        }
        "omega_workbench_terminal_thread_switch" => {
            let foreign = terminal_process(
                "terminal-alpha-visible",
                "terminal-item-alpha-visible",
                "alpha shell",
                terminal_owner(foreign_binding, "/workspace/alpha"),
                TerminalProcessLifecycleFixture::Running { process_id: 4105 },
            );
            snapshot.panes[0]
                .terminal_ids
                .push(foreign.terminal_id.clone());
            snapshot.panes[0].active_terminal_id = Some(foreign.terminal_id.clone());
            snapshot.selected_terminal_id = Some(foreign.terminal_id.clone());
            snapshot.processes.push(foreign);
            snapshot.running_badge_count = 2;
        }
        "omega_workbench_terminal_stale_spawn" => {
            let mut stale_binding = binding;
            stale_binding.generation = stale_binding.generation.saturating_sub(1);
            snapshot.requested_spawns.push(TerminalSpawnFixture {
                request_id: "stale-spawn".into(),
                binding: stale_binding,
                requested_cwd: "/workspace/beta".into(),
                result: TerminalSpawnResultFixture::IgnoredStale,
            });
            snapshot.ignored_stale_completion_count = 1;
        }
        "omega_workbench_terminal_foreign_spawn_rejected" => {
            snapshot.requested_spawns.push(TerminalSpawnFixture {
                request_id: "foreign-spawn".into(),
                binding: foreign_binding.clone(),
                requested_cwd: "/workspace/alpha".into(),
                result: TerminalSpawnResultFixture::RejectedForeignBinding,
            });
            snapshot.rejected_foreign_spawn_count = 1;
        }
        "omega_workbench_terminal_error" => {
            snapshot.lifecycle =
                TerminalLifecycleFixture::Error("terminal state could not be restored".into());
            snapshot.focus = TerminalFocusFixture::Surface;
        }
        _ => bail!("unknown Terminal workbench scene {name:?}"),
    }
    Ok(snapshot)
}

fn terminal_owner(binding: &TerminalBindingFixture, initial_cwd: &str) -> TerminalOwnerFixture {
    TerminalOwnerFixture {
        thread_id: binding.thread_id.clone(),
        repository_id: binding.repository_id.clone(),
        worktree_id: binding.worktree_id.clone(),
        worktree_abs_path: binding.worktree_abs_path.clone(),
        initial_cwd: initial_cwd.into(),
        generation: binding.generation,
    }
}

fn terminal_process(
    terminal_id: &str,
    item_id: &str,
    title: &str,
    owner: TerminalOwnerFixture,
    lifecycle: TerminalProcessLifecycleFixture,
) -> TerminalProcessFixture {
    TerminalProcessFixture {
        terminal_id: terminal_id.into(),
        item_id: item_id.into(),
        title: title.into(),
        current_cwd: owner.initial_cwd.clone(),
        owner,
        lifecycle,
        input_bytes: Vec::new(),
    }
}

fn surface_fixture_mut(
    fixtures: &mut [SurfaceFixture],
    surface_id: WorkSurfaceId,
) -> Result<&mut SurfaceFixture> {
    fixtures
        .iter_mut()
        .find(|fixture| fixture.id == surface_id)
        .ok_or_else(|| anyhow!("missing {surface_id:?} surface fixture"))
}

pub fn validate_scene_catalog() -> Result<()> {
    let mut names = BTreeSet::new();
    for scene in HERMETIC_SCENES {
        if !names.insert(scene.name) {
            bail!("duplicate workbench scene {:?}", scene.name);
        }
        scene.pixel_policy.validate()?;
        scene.fixture().validate()?;
        let mut region_names = BTreeSet::new();
        for region in scene.regions {
            region.validate()?;
            if !region_names.insert(region.name) {
                bail!(
                    "scene {:?} contains duplicate capture region {:?}",
                    scene.name,
                    region.name
                );
            }
        }
    }
    Ok(())
}

pub fn select_scenes(
    requested: Option<&str>,
    shard_index: Option<usize>,
    shard_count: Option<usize>,
) -> Result<Vec<&'static SceneSpec>> {
    validate_scene_catalog()?;
    let selected: Vec<_> = match requested {
        Some(name) => {
            vec![scene_spec(name).ok_or_else(|| anyhow!("unknown workbench scene {name:?}"))?]
        }
        None => HERMETIC_SCENES.iter().collect(),
    };

    match (shard_index, shard_count) {
        (None, None) => Ok(selected),
        (Some(index), Some(count)) => {
            if count == 0 {
                bail!("shard count must be greater than zero");
            }
            if index >= count {
                bail!("shard index {index} is outside shard count {count}");
            }
            let shard: Vec<_> = selected
                .into_iter()
                .enumerate()
                .filter_map(|(position, scene)| (position % count == index).then_some(scene))
                .collect();
            if shard.is_empty() {
                bail!("workbench scene shard {index}/{count} is empty");
            }
            Ok(shard)
        }
        _ => bail!("shard index and shard count must be provided together"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofLane {
    Semantic,
    Pixel,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: Option<String>,
}

impl ProofCheck {
    pub fn passed(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: CheckStatus::Passed,
            detail: None,
        }
    }

    pub fn failed(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: CheckStatus::Failed,
            detail: Some(detail.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelStatus {
    Passed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PixelProof {
    pub status: PixelStatus,
    pub minimum_match: f64,
    pub channel_tolerance: u8,
    pub policy_rationale: String,
    pub match_percentage: Option<f64>,
    pub different_pixels: Option<u32>,
    pub total_pixels: Option<u32>,
    pub baseline: PathBuf,
    pub current: PathBuf,
    pub diff: Option<PathBuf>,
    pub regions: Vec<RegionPixelProof>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionPixelProof {
    pub name: String,
    pub status: PixelStatus,
    pub match_percentage: Option<f64>,
    pub different_pixels: Option<u32>,
    pub total_pixels: Option<u32>,
    pub baseline: PathBuf,
    pub current: PathBuf,
    pub diff: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofOutcome {
    Passed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProofReceipt {
    pub schema: String,
    pub scene: String,
    pub fixture_digest: String,
    pub seed: u64,
    pub lane: ProofLane,
    pub viewport: ViewportFixture,
    pub semantic_checks: Vec<ProofCheck>,
    pub pixel: Option<PixelProof>,
    pub outcome: ProofOutcome,
}

impl ProofReceipt {
    pub fn new(scene: &WorkbenchScene, seed: u64, lane: ProofLane) -> Result<Self> {
        Ok(Self {
            schema: PROOF_RECEIPT_SCHEMA.to_string(),
            scene: scene.name.clone(),
            fixture_digest: scene.digest()?,
            seed,
            lane,
            viewport: scene.viewport,
            semantic_checks: Vec::new(),
            pixel: None,
            outcome: ProofOutcome::Passed,
        })
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != PROOF_RECEIPT_SCHEMA {
            bail!("unknown proof receipt schema {:?}", self.schema);
        }
        if self.scene.trim().is_empty() || !self.fixture_digest.starts_with("sha256:") {
            bail!("proof receipt must name a scene and a SHA-256 fixture digest");
        }
        self.viewport.validate()?;
        if self.semantic_checks.is_empty() {
            bail!("proof receipt has zero semantic checks");
        }
        if self
            .semantic_checks
            .iter()
            .any(|check| check.name.trim().is_empty())
        {
            bail!("proof receipt contains an unnamed semantic check");
        }
        let semantic_failure = self
            .semantic_checks
            .iter()
            .any(|check| check.status == CheckStatus::Failed);
        if self.lane == ProofLane::Pixel && self.pixel.is_none() {
            bail!("pixel proof receipt has no pixel result");
        }
        if self.lane == ProofLane::Semantic && self.pixel.is_some() {
            bail!("semantic proof receipt unexpectedly contains pixel evidence");
        }
        let pixel_failure = self.pixel.as_ref().is_some_and(|pixel| {
            pixel.status == PixelStatus::Failed
                || pixel
                    .regions
                    .iter()
                    .any(|region| region.status == PixelStatus::Failed)
        });
        if (semantic_failure || pixel_failure) != (self.outcome == ProofOutcome::Failed) {
            bail!("proof receipt outcome disagrees with semantic or pixel checks");
        }
        if let Some(pixel) = &self.pixel {
            if !(0.0..=1.0).contains(&pixel.minimum_match)
                || pixel.policy_rationale.trim().is_empty()
            {
                bail!("pixel receipt has an invalid policy");
            }
            if pixel
                .match_percentage
                .is_some_and(|percentage| !(0.0..=1.0).contains(&percentage))
            {
                bail!("pixel receipt has an invalid match percentage");
            }
            match (pixel.different_pixels, pixel.total_pixels) {
                (Some(different), Some(total)) if different <= total => {}
                (None, None) => {}
                _ => bail!("pixel receipt has inconsistent changed and total pixel counts"),
            }
            if pixel.status == PixelStatus::Passed
                && !pixel
                    .match_percentage
                    .is_some_and(|percentage| percentage >= pixel.minimum_match)
            {
                bail!("passing pixel receipt does not meet its match threshold");
            }
            let mut region_names = BTreeSet::new();
            for region in &pixel.regions {
                if region.name.trim().is_empty() || !region_names.insert(region.name.as_str()) {
                    bail!("pixel receipt contains an empty or duplicate region name");
                }
                if region
                    .match_percentage
                    .is_some_and(|percentage| !(0.0..=1.0).contains(&percentage))
                {
                    bail!(
                        "pixel region {:?} has an invalid match percentage",
                        region.name
                    );
                }
                match (region.different_pixels, region.total_pixels) {
                    (Some(different), Some(total)) if different <= total => {}
                    (None, None) => {}
                    _ => bail!(
                        "pixel region {:?} has inconsistent changed and total counts",
                        region.name
                    ),
                }
                if region.status == PixelStatus::Passed
                    && !region
                        .match_percentage
                        .is_some_and(|percentage| percentage >= pixel.minimum_match)
                {
                    bail!(
                        "passing pixel region {:?} does not meet its match threshold",
                        region.name
                    );
                }
            }
            for path in [&pixel.baseline, &pixel.current]
                .into_iter()
                .chain(pixel.diff.iter())
                .chain(pixel.regions.iter().flat_map(|region| {
                    [&region.baseline, &region.current]
                        .into_iter()
                        .chain(region.diff.iter())
                }))
            {
                validate_artifact_path(path)?;
            }
        }
        Ok(())
    }

    pub fn write_json(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating proof receipt folder {}", parent.display()))?;
        }
        let encoded = serde_json::to_vec_pretty(self).context("encoding proof receipt")?;
        fs::write(path, encoded)
            .with_context(|| format!("writing proof receipt {}", path.display()))
    }
}

fn validate_artifact_path(path: &Path) -> Result<()> {
    use std::path::Component;

    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!(
            "proof artifact path must be output-root-relative: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(feature = "gpui-support")]
pub struct SemanticProbe<'a> {
    snapshot: &'a DebugRenderSnapshot,
    checks: Vec<ProofCheck>,
}

#[cfg(feature = "gpui-support")]
impl<'a> SemanticProbe<'a> {
    pub fn new(snapshot: &'a DebugRenderSnapshot) -> Self {
        Self {
            snapshot,
            checks: Vec::new(),
        }
    }

    pub fn require_unique(&mut self, selector: &str) -> Result<Bounds<Pixels>> {
        let count = self.snapshot.selector_count(selector);
        if count != 1 {
            let detail = format!("expected one rendered target, found {count}");
            self.checks
                .push(ProofCheck::failed(format!("unique:{selector}"), &detail));
            bail!("{selector:?}: {detail}");
        }
        let bounds = self
            .snapshot
            .bounds(selector)
            .ok_or_else(|| anyhow!("{selector:?}: selector count had no bounds"))?;
        self.checks
            .push(ProofCheck::passed(format!("unique:{selector}")));
        Ok(bounds)
    }

    pub fn require_absent(&mut self, selector: &str) -> Result<()> {
        let count = self.snapshot.selector_count(selector);
        if count != 0 {
            let detail = format!("expected no rendered target, found {count}");
            self.checks
                .push(ProofCheck::failed(format!("absent:{selector}"), &detail));
            bail!("{selector:?}: {detail}");
        }
        self.checks
            .push(ProofCheck::passed(format!("absent:{selector}")));
        Ok(())
    }

    pub fn require_visible(&mut self, selector: &str) -> Result<Bounds<Pixels>> {
        let bounds = self.require_unique(selector)?;
        let occurrence = self
            .snapshot
            .occurrences(selector)
            .first()
            .ok_or_else(|| anyhow!("{selector:?}: selector count had no occurrence"))?;
        if !matches!(
            occurrence.visibility,
            gpui::DebugVisibility::Visible | gpui::DebugVisibility::PartiallyClipped
        ) {
            let detail = format!("target visibility is {:?}", occurrence.visibility);
            self.checks
                .push(ProofCheck::failed(format!("visible:{selector}"), &detail));
            bail!("{selector:?}: {detail}");
        }
        self.checks
            .push(ProofCheck::passed(format!("visible:{selector}")));
        Ok(bounds)
    }

    pub fn require_fully_visible(&mut self, selector: &str) -> Result<Bounds<Pixels>> {
        let bounds = self.require_unique(selector)?;
        let occurrence = self
            .snapshot
            .occurrences(selector)
            .first()
            .ok_or_else(|| anyhow!("{selector:?}: selector count had no occurrence"))?;
        if occurrence.visibility != gpui::DebugVisibility::Visible {
            let detail = format!("target visibility is {:?}", occurrence.visibility);
            self.checks.push(ProofCheck::failed(
                format!("fully-visible:{selector}"),
                &detail,
            ));
            bail!("{selector:?}: {detail}");
        }
        self.checks
            .push(ProofCheck::passed(format!("fully-visible:{selector}")));
        Ok(bounds)
    }

    pub fn require_interactive(&mut self, selector: &str) -> Result<Bounds<Pixels>> {
        let bounds = self.require_visible(selector)?;
        let occurrence = self
            .snapshot
            .occurrences(selector)
            .first()
            .ok_or_else(|| anyhow!("{selector:?}: selector count had no occurrence"))?;
        if !occurrence.hit_testable || !occurrence.focusable {
            let detail = format!(
                "expected hit-testable and focusable target, got hit_testable={} focusable={}",
                occurrence.hit_testable, occurrence.focusable
            );
            self.checks.push(ProofCheck::failed(
                format!("interactive:{selector}"),
                &detail,
            ));
            bail!("{selector:?}: {detail}");
        }
        self.checks
            .push(ProofCheck::passed(format!("interactive:{selector}")));
        Ok(bounds)
    }

    pub fn require_focus(&mut self, selector: &str, focused: bool) -> Result<()> {
        self.require_unique(selector)?;
        let occurrence = self
            .snapshot
            .occurrences(selector)
            .first()
            .ok_or_else(|| anyhow!("{selector:?}: selector count had no occurrence"))?;
        if occurrence.focused != focused {
            let detail = format!(
                "expected focused={focused}, rendered focused={}",
                occurrence.focused
            );
            self.checks
                .push(ProofCheck::failed(format!("focus:{selector}"), &detail));
            bail!("{selector:?}: {detail}");
        }
        self.checks
            .push(ProofCheck::passed(format!("focus:{selector}")));
        Ok(())
    }

    pub fn require_contains_focus(&mut self, selector: &str, contains_focus: bool) -> Result<()> {
        self.require_unique(selector)?;
        let occurrence = self
            .snapshot
            .occurrences(selector)
            .first()
            .ok_or_else(|| anyhow!("{selector:?}: selector count had no occurrence"))?;
        if occurrence.contains_focus != contains_focus {
            let detail = format!(
                "expected contains_focus={contains_focus}, rendered contains_focus={}",
                occurrence.contains_focus
            );
            self.checks.push(ProofCheck::failed(
                format!("contains-focus:{selector}"),
                &detail,
            ));
            bail!("{selector:?}: {detail}");
        }
        self.checks
            .push(ProofCheck::passed(format!("contains-focus:{selector}")));
        Ok(())
    }

    pub fn require_not_hit_testable(&mut self, selector: &str) -> Result<()> {
        self.require_visible(selector)?;
        let occurrence = self
            .snapshot
            .occurrences(selector)
            .first()
            .ok_or_else(|| anyhow!("{selector:?}: selector count had no occurrence"))?;
        if occurrence.hit_testable {
            let detail = "expected target not to receive pointer input";
            self.checks.push(ProofCheck::failed(
                format!("not-hit-testable:{selector}"),
                detail,
            ));
            bail!("{selector:?}: {detail}");
        }
        self.checks
            .push(ProofCheck::passed(format!("not-hit-testable:{selector}")));
        Ok(())
    }

    pub fn require_hit_testable(&mut self, selector: &str) -> Result<Bounds<Pixels>> {
        let bounds = self.require_visible(selector)?;
        let occurrence = self
            .snapshot
            .occurrences(selector)
            .first()
            .ok_or_else(|| anyhow!("{selector:?}: selector count had no occurrence"))?;
        if !occurrence.hit_testable {
            let detail = "expected target to receive pointer input";
            self.checks.push(ProofCheck::failed(
                format!("hit-testable:{selector}"),
                detail,
            ));
            bail!("{selector:?}: {detail}");
        }
        self.checks
            .push(ProofCheck::passed(format!("hit-testable:{selector}")));
        Ok(bounds)
    }

    pub fn require_inside(&mut self, child: &str, parent: &str) -> Result<()> {
        let child_bounds = self.require_unique(child)?;
        let parent_bounds = self.require_unique(parent)?;
        let inside = child_bounds.left() >= parent_bounds.left()
            && child_bounds.top() >= parent_bounds.top()
            && child_bounds.right() <= parent_bounds.right()
            && child_bounds.bottom() <= parent_bounds.bottom();
        if !inside {
            let detail = format!(
                "child bounds {child_bounds:?} are outside parent bounds {parent_bounds:?}"
            );
            self.checks.push(ProofCheck::failed(
                format!("inside:{child}:{parent}"),
                &detail,
            ));
            bail!("{detail}");
        }
        self.checks
            .push(ProofCheck::passed(format!("inside:{child}:{parent}")));
        Ok(())
    }

    pub fn require_disjoint(&mut self, first: &str, second: &str) -> Result<()> {
        let first_bounds = self.require_unique(first)?;
        let second_bounds = self.require_unique(second)?;
        let overlaps = first_bounds.left() < second_bounds.right()
            && first_bounds.right() > second_bounds.left()
            && first_bounds.top() < second_bounds.bottom()
            && first_bounds.bottom() > second_bounds.top();
        if overlaps {
            let detail = format!("bounds overlap: {first_bounds:?} and {second_bounds:?}");
            self.checks.push(ProofCheck::failed(
                format!("disjoint:{first}:{second}"),
                &detail,
            ));
            bail!("{detail}");
        }
        self.checks
            .push(ProofCheck::passed(format!("disjoint:{first}:{second}")));
        Ok(())
    }

    pub fn require_accessible(&mut self, element_id: &str, role: &str, label: &str) -> Result<()> {
        let tree = self
            .snapshot
            .accessibility_tree_json()
            .ok_or_else(|| anyhow!("accessibility tree was not active"))?;
        let value: serde_json::Value =
            serde_json::from_str(tree).context("parsing accessibility tree")?;
        let nodes = value
            .get("nodes")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| anyhow!("accessibility tree has no nodes object"))?;
        let matching: Vec<_> = nodes
            .values()
            .filter(|node| {
                node.get("element_id").and_then(|value| value.as_str()) == Some(element_id)
            })
            .collect();
        if matching.len() != 1 {
            let detail = format!(
                "expected one accessibility node for {element_id:?}, found {}",
                matching.len()
            );
            self.checks.push(ProofCheck::failed(
                format!("accessible:{element_id}"),
                &detail,
            ));
            bail!("{detail}");
        }
        let aria = matching[0]
            .get("aria")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| anyhow!("accessibility node {element_id:?} has no aria object"))?;
        let actual_role = aria
            .get("role")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let actual_label = aria
            .get("label")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if actual_role != role || actual_label != label {
            let detail = format!(
                "expected role {role:?} and label {label:?}, got {actual_role:?} and {actual_label:?}"
            );
            self.checks.push(ProofCheck::failed(
                format!("accessible:{element_id}"),
                &detail,
            ));
            bail!("{detail}");
        }
        self.checks
            .push(ProofCheck::passed(format!("accessible:{element_id}")));
        Ok(())
    }

    pub fn require_accessibility_property(
        &mut self,
        element_id: &str,
        property: &str,
        expected: serde_json::Value,
    ) -> Result<()> {
        let tree = self
            .snapshot
            .accessibility_tree_json()
            .ok_or_else(|| anyhow!("accessibility tree was not active"))?;
        let value: serde_json::Value =
            serde_json::from_str(tree).context("parsing accessibility tree")?;
        let nodes = value
            .get("nodes")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| anyhow!("accessibility tree has no nodes object"))?;
        let matching: Vec<_> = nodes
            .values()
            .filter(|node| {
                node.get("element_id").and_then(serde_json::Value::as_str) == Some(element_id)
            })
            .collect();
        if matching.len() != 1 {
            bail!(
                "expected one accessibility node for {element_id:?}, found {}",
                matching.len()
            );
        }
        let actual = matching[0]
            .get("aria")
            .and_then(|aria| aria.get(property))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        if actual != expected {
            let detail =
                format!("expected accessibility property {property:?}={expected}, got {actual}");
            self.checks.push(ProofCheck::failed(
                format!("accessibility-property:{element_id}:{property}"),
                &detail,
            ));
            bail!("{element_id:?}: {detail}");
        }
        self.checks.push(ProofCheck::passed(format!(
            "accessibility-property:{element_id}:{property}"
        )));
        Ok(())
    }

    pub fn into_checks(self) -> Vec<ProofCheck> {
        self.checks
    }
}

#[cfg(feature = "gpui-support")]
pub fn prove_workbench_shell(
    scene: &WorkbenchScene,
    snapshot: &DebugRenderSnapshot,
) -> Result<Vec<ProofCheck>> {
    scene.validate()?;
    let mut probe = SemanticProbe::new(snapshot);
    probe.require_fully_visible("omega.workbench.activity-rail")?;
    probe.require_accessible("omega.workbench.activity-rail", "Toolbar", "Work surfaces")?;
    probe.require_accessibility_property(
        "omega.workbench.activity-rail",
        "orientation",
        serde_json::Value::String("Vertical".into()),
    )?;

    for surface in &scene.surfaces {
        let selector = surface.id.rail_selector();
        probe.require_accessible(selector, "Button", surface.id.label())?;
        probe.require_accessibility_property(
            selector,
            "disabled",
            serde_json::Value::Bool(!surface.available),
        )?;
        probe.require_accessibility_property(
            selector,
            "expanded",
            serde_json::Value::Bool(
                surface.available && scene.dock_open && scene.active_surface == Some(surface.id),
            ),
        )?;
        if surface.available {
            probe.require_interactive(selector)?;
        } else {
            probe.require_visible(selector)?;
        }
        if surface.badge.is_some() {
            probe.require_visible(surface.id.badge_selector())?;
        } else {
            probe.require_absent(surface.id.badge_selector())?;
        }
    }

    if scene.dock_open {
        let surface = scene
            .active_surface
            .context("an open dock requires an active surface")?;
        probe.require_visible("omega.workbench.dock")?;
        probe.require_visible(surface.surface_selector())?;
        probe.require_inside(surface.surface_selector(), "omega.workbench.dock")?;
        probe.require_accessible(DOCK_COLLAPSE_CONTROL, "Button", "Collapse work surface")?;
        probe.require_interactive(DOCK_COLLAPSE_CONTROL)?;
        probe.require_inside(DOCK_COLLAPSE_CONTROL, "omega.workbench.dock")?;
        probe.require_accessible(DOCK_RESIZE_CONTROL, "Splitter", "Resize work surface")?;
        probe.require_accessibility_property(
            DOCK_RESIZE_CONTROL,
            "orientation",
            serde_json::Value::String("Vertical".into()),
        )?;
        probe.require_hit_testable(DOCK_RESIZE_CONTROL)?;
        probe.require_inside(DOCK_RESIZE_CONTROL, "omega.workbench.dock")?;
    } else {
        probe.require_absent("omega.workbench.dock")?;
    }
    Ok(probe.into_checks())
}

pub fn prove_review_surface(
    scene: &WorkbenchScene,
    actual: &ReviewSessionFixture,
) -> Result<Vec<ProofCheck>> {
    scene.validate()?;
    let expected = scene
        .active_review_session()
        .context("Review proof scene has no active review session")?;
    let mut checks = Vec::new();

    require_review_match(
        "review-binding-identity",
        &expected.binding,
        &actual.binding,
        &mut checks,
    )?;
    require_review_match(
        "review-lifecycle",
        &expected.lifecycle,
        &actual.lifecycle,
        &mut checks,
    )?;
    require_review_match(
        "review-file-count",
        &expected.files.len(),
        &actual.files.len(),
        &mut checks,
    )?;
    let expected_hunk_count = expected
        .files
        .iter()
        .map(|file| file.hunks.len())
        .sum::<usize>();
    let actual_hunk_count = actual
        .files
        .iter()
        .map(|file| file.hunks.len())
        .sum::<usize>();
    require_review_match(
        "review-hunk-count",
        &expected_hunk_count,
        &actual_hunk_count,
        &mut checks,
    )?;
    require_review_match(
        "review-ordered-file-hunk-status",
        &expected.files,
        &actual.files,
        &mut checks,
    )?;
    require_review_match(
        "review-selected-file",
        &expected.selected_file_path,
        &actual.selected_file_path,
        &mut checks,
    )?;
    require_review_match(
        "review-selected-hunk",
        &expected.selected_hunk_id,
        &actual.selected_hunk_id,
        &mut checks,
    )?;
    require_review_match(
        "review-focus-owner",
        &expected.focus,
        &actual.focus,
        &mut checks,
    )?;
    require_review_match(
        "review-filesystem-mutations",
        &expected.mutations,
        &actual.mutations,
        &mut checks,
    )?;
    require_review_match(
        "review-pending-operation-count",
        &expected.pending_operation_count,
        &actual.pending_operation_count,
        &mut checks,
    )?;
    require_review_match(
        "review-ignored-stale-completion-count",
        &expected.ignored_stale_completion_count,
        &actual.ignored_stale_completion_count,
        &mut checks,
    )?;

    let expected_paths = expected
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    let foreign_paths = scene
        .review_sessions
        .iter()
        .filter(|review| review.binding.thread_id != expected.binding.thread_id)
        .flat_map(|review| review.files.iter())
        .map(|file| file.path.as_str())
        .filter(|path| !expected_paths.contains(path))
        .collect::<BTreeSet<_>>();
    let leaked_paths = actual
        .files
        .iter()
        .map(|file| file.path.as_str())
        .filter(|path| foreign_paths.contains(path))
        .collect::<Vec<_>>();
    if !leaked_paths.is_empty() {
        let detail = format!("active Review snapshot leaked foreign-thread paths {leaked_paths:?}");
        checks.push(ProofCheck::failed(
            "review-no-foreign-thread-files",
            &detail,
        ));
        bail!("{detail}");
    }
    checks.push(ProofCheck::passed("review-no-foreign-thread-files"));

    Ok(checks)
}

fn require_review_match<T>(
    name: &str,
    expected: &T,
    actual: &T,
    checks: &mut Vec<ProofCheck>,
) -> Result<()>
where
    T: std::fmt::Debug + PartialEq,
{
    if expected != actual {
        let detail = format!("expected {expected:?}, got {actual:?}");
        checks.push(ProofCheck::failed(name, &detail));
        bail!("{name}: {detail}");
    }
    checks.push(ProofCheck::passed(name));
    Ok(())
}

pub fn prove_git_surface(
    scene: &WorkbenchScene,
    actual: &GitSnapshotFixture,
) -> Result<Vec<ProofCheck>> {
    scene.validate()?;
    let expected = scene
        .active_git_snapshot()
        .context("Git proof scene has no active Git snapshot")?;
    let mut checks = Vec::new();

    require_git_match(
        "git-binding-identity",
        &expected.binding,
        &actual.binding,
        &mut checks,
    )?;
    require_git_match(
        "git-lifecycle",
        &expected.lifecycle,
        &actual.lifecycle,
        &mut checks,
    )?;
    require_git_match(
        "git-branch-state",
        &expected.branch,
        &actual.branch,
        &mut checks,
    )?;

    let expected_paths = expected
        .status_entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();
    let foreign_paths = scene
        .git_snapshots
        .iter()
        .filter(|snapshot| snapshot.binding.repository_id != expected.binding.repository_id)
        .flat_map(|snapshot| snapshot.status_entries.iter())
        .map(|entry| entry.path.as_str())
        .filter(|path| !expected_paths.contains(path))
        .collect::<BTreeSet<_>>();
    let leaked_paths = actual
        .status_entries
        .iter()
        .map(|entry| entry.path.as_str())
        .filter(|path| foreign_paths.contains(path))
        .collect::<Vec<_>>();
    if !leaked_paths.is_empty() {
        let detail =
            format!("active Git snapshot leaked foreign-repository paths {leaked_paths:?}");
        checks.push(ProofCheck::failed(
            "git-no-foreign-repository-status",
            &detail,
        ));
        bail!("{detail}");
    }
    checks.push(ProofCheck::passed("git-no-foreign-repository-status"));

    require_git_match(
        "git-ordered-status-staging",
        &expected.status_entries,
        &actual.status_entries,
        &mut checks,
    )?;
    require_git_match(
        "git-status-counts",
        &expected.status_counts,
        &actual.status_counts,
        &mut checks,
    )?;
    require_git_match(
        "git-selected-path",
        &expected.selected_path,
        &actual.selected_path,
        &mut checks,
    )?;
    require_git_match(
        "git-pending-operation",
        &expected.pending_operation,
        &actual.pending_operation,
        &mut checks,
    )?;
    require_git_match(
        "git-badge-count",
        &expected.badge_count,
        &actual.badge_count,
        &mut checks,
    )?;
    require_git_match(
        "git-requested-mutations-results",
        &expected.requested_mutations,
        &actual.requested_mutations,
        &mut checks,
    )?;
    require_git_match(
        "git-ignored-stale-refresh-count",
        &expected.ignored_stale_refresh_count,
        &actual.ignored_stale_refresh_count,
        &mut checks,
    )?;
    require_git_match(
        "git-focus-owner",
        &expected.focus,
        &actual.focus,
        &mut checks,
    )?;

    let active_workbench = scene
        .active_thread_workbench()
        .context("Git proof scene has no active workbench projection")?;
    let rail_badge = surface_fixture(&active_workbench.surfaces, WorkSurfaceId::Git)?.badge;
    let expected_badge = (actual.badge_count > 0).then_some(actual.badge_count);
    require_git_match(
        "git-badge-agreement",
        &expected_badge,
        &rail_badge,
        &mut checks,
    )?;

    Ok(checks)
}

fn require_git_match<T>(
    name: &str,
    expected: &T,
    actual: &T,
    checks: &mut Vec<ProofCheck>,
) -> Result<()>
where
    T: std::fmt::Debug + PartialEq,
{
    if expected != actual {
        let detail = format!("expected {expected:?}, got {actual:?}");
        checks.push(ProofCheck::failed(name, &detail));
        bail!("{name}: {detail}");
    }
    checks.push(ProofCheck::passed(name));
    Ok(())
}

pub fn prove_terminal_surface(
    scene: &WorkbenchScene,
    actual: &TerminalSnapshotFixture,
) -> Result<Vec<ProofCheck>> {
    scene.validate()?;
    let expected = scene
        .active_terminal_snapshot()
        .context("Terminal proof scene has no active Terminal snapshot")?;
    let mut checks = Vec::new();

    require_terminal_match(
        "terminal-creation-binding",
        &expected.creation_binding,
        &actual.creation_binding,
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-panel-entity",
        &expected.panel_entity_id,
        &actual.panel_entity_id,
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-lifecycle",
        &expected.lifecycle,
        &actual.lifecycle,
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-pane-layout",
        &expected.pane_layout,
        &actual.pane_layout,
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-pane-tabs-selection",
        &(&expected.panes, &expected.selected_terminal_id),
        &(&actual.panes, &actual.selected_terminal_id),
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-process-identities",
        &expected
            .processes
            .iter()
            .map(|process| (&process.terminal_id, &process.item_id, &process.title))
            .collect::<Vec<_>>(),
        &actual
            .processes
            .iter()
            .map(|process| (&process.terminal_id, &process.item_id, &process.title))
            .collect::<Vec<_>>(),
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-immutable-owners",
        &expected
            .processes
            .iter()
            .map(|process| &process.owner)
            .collect::<Vec<_>>(),
        &actual
            .processes
            .iter()
            .map(|process| &process.owner)
            .collect::<Vec<_>>(),
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-cwd-identity",
        &expected
            .processes
            .iter()
            .map(|process| &process.current_cwd)
            .collect::<Vec<_>>(),
        &actual
            .processes
            .iter()
            .map(|process| &process.current_cwd)
            .collect::<Vec<_>>(),
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-process-lifecycles",
        &expected
            .processes
            .iter()
            .map(|process| &process.lifecycle)
            .collect::<Vec<_>>(),
        &actual
            .processes
            .iter()
            .map(|process| &process.lifecycle)
            .collect::<Vec<_>>(),
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-input-bytes",
        &expected
            .processes
            .iter()
            .map(|process| &process.input_bytes)
            .collect::<Vec<_>>(),
        &actual
            .processes
            .iter()
            .map(|process| &process.input_bytes)
            .collect::<Vec<_>>(),
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-spawn-results",
        &expected.requested_spawns,
        &actual.requested_spawns,
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-running-badge",
        &expected.running_badge_count,
        &actual.running_badge_count,
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-no-implicit-spawn",
        &expected.implicit_spawn_count,
        &actual.implicit_spawn_count,
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-ignored-stale-completion-count",
        &expected.ignored_stale_completion_count,
        &actual.ignored_stale_completion_count,
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-rejected-foreign-spawn-count",
        &expected.rejected_foreign_spawn_count,
        &actual.rejected_foreign_spawn_count,
        &mut checks,
    )?;
    require_terminal_match(
        "terminal-focus-owner",
        &expected.focus,
        &actual.focus,
        &mut checks,
    )?;

    let active_workbench = scene
        .active_thread_workbench()
        .context("Terminal proof scene has no active workbench projection")?;
    let rail_badge = surface_fixture(&active_workbench.surfaces, WorkSurfaceId::Terminal)?.badge;
    let actual_badge = (actual.running_badge_count > 0).then_some(actual.running_badge_count);
    require_terminal_match(
        "terminal-badge-agreement",
        &rail_badge,
        &actual_badge,
        &mut checks,
    )?;

    let expected_foreign_owners = expected
        .processes
        .iter()
        .filter(|process| process.owner.thread_id != expected.creation_binding.thread_id)
        .map(|process| (&process.terminal_id, &process.owner))
        .collect::<Vec<_>>();
    let actual_foreign_owners = actual
        .processes
        .iter()
        .filter(|process| process.owner.thread_id != expected.creation_binding.thread_id)
        .map(|process| (&process.terminal_id, &process.owner))
        .collect::<Vec<_>>();
    require_terminal_match(
        "terminal-no-owner-relabel",
        &expected_foreign_owners,
        &actual_foreign_owners,
        &mut checks,
    )?;

    Ok(checks)
}

fn require_terminal_match<T>(
    name: &str,
    expected: &T,
    actual: &T,
    checks: &mut Vec<ProofCheck>,
) -> Result<()>
where
    T: std::fmt::Debug + PartialEq,
{
    if expected != actual {
        let detail = format!("expected {expected:?}, got {actual:?}");
        checks.push(ProofCheck::failed(name, &detail));
        bail!("{name}: {detail}");
    }
    checks.push(ProofCheck::passed(name));
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureRegion {
    pub name: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureRegionSpec {
    pub name: &'static str,
    pub source: CaptureRegionSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureRegionSource {
    Pixels {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    SelectorUnion {
        selectors: &'static [&'static str],
        padding: u32,
    },
}

impl CaptureRegionSpec {
    pub const fn pixels(name: &'static str, x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            name,
            source: CaptureRegionSource::Pixels {
                x,
                y,
                width,
                height,
            },
        }
    }

    pub const fn selector_union(
        name: &'static str,
        selectors: &'static [&'static str],
        padding: u32,
    ) -> Self {
        Self {
            name,
            source: CaptureRegionSource::SelectorUnion { selectors, padding },
        }
    }

    fn validate(self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("capture region name must not be empty");
        }
        match self.source {
            CaptureRegionSource::Pixels { width, height, .. } => {
                if width == 0 || height == 0 {
                    bail!("fixed capture region dimensions must be non-zero");
                }
            }
            CaptureRegionSource::SelectorUnion { selectors, .. } => {
                if selectors.is_empty() {
                    bail!("selector capture region must name at least one selector");
                }
                let mut unique_selectors = BTreeSet::new();
                for selector in selectors {
                    if selector.trim().is_empty() || !unique_selectors.insert(selector) {
                        bail!("selector capture region contains an empty or duplicate selector");
                    }
                }
            }
        }
        Ok(())
    }

    #[cfg(feature = "gpui-support")]
    pub fn resolve(
        self,
        snapshot: &DebugRenderSnapshot,
        viewport: ViewportFixture,
        image_width: u32,
        image_height: u32,
    ) -> Result<CaptureRegion> {
        self.validate()?;
        match self.source {
            CaptureRegionSource::Pixels {
                x,
                y,
                width,
                height,
            } => Ok(CaptureRegion {
                name: self.name.to_string(),
                x,
                y,
                width,
                height,
            }),
            CaptureRegionSource::SelectorUnion { selectors, padding } => {
                let logical_bounds = selectors.iter().filter_map(|selector| {
                    snapshot.bounds(selector).map(|bounds| {
                        (
                            bounds.origin.x.as_f32(),
                            bounds.origin.y.as_f32(),
                            bounds.size.width.as_f32(),
                            bounds.size.height.as_f32(),
                        )
                    })
                });
                resolve_selector_region(
                    self.name,
                    logical_bounds,
                    padding,
                    viewport,
                    image_width,
                    image_height,
                )
            }
        }
    }
}

fn resolve_selector_region(
    name: &str,
    logical_bounds: impl IntoIterator<Item = (f32, f32, f32, f32)>,
    padding: u32,
    viewport: ViewportFixture,
    image_width: u32,
    image_height: u32,
) -> Result<CaptureRegion> {
    if name.trim().is_empty() {
        bail!("capture region name must not be empty");
    }
    viewport.validate()?;
    if image_width == 0 || image_height == 0 {
        bail!("capture image dimensions must be non-zero");
    }

    let mut union: Option<(f32, f32, f32, f32)> = None;
    for (x, y, width, height) in logical_bounds {
        if !x.is_finite()
            || !y.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
        {
            bail!("capture selector bounds must be finite and non-zero");
        }
        let right = x + width;
        let bottom = y + height;
        if !right.is_finite() || !bottom.is_finite() {
            bail!("capture selector bounds must not overflow");
        }
        union = Some(match union {
            Some((left, top, union_right, union_bottom)) => (
                left.min(x),
                top.min(y),
                union_right.max(right),
                union_bottom.max(bottom),
            ),
            None => (x, y, right, bottom),
        });
    }
    let (left, top, right, bottom) =
        union.ok_or_else(|| anyhow!("capture region {name:?} has no rendered selector bounds"))?;
    let scale = viewport.scale_milli as f64 / 1000.0;
    let padding = (padding as f64 * scale).ceil();
    let left = ((left as f64 * scale).floor() - padding)
        .max(0.0)
        .min(image_width as f64) as u32;
    let top = ((top as f64 * scale).floor() - padding)
        .max(0.0)
        .min(image_height as f64) as u32;
    let right = ((right as f64 * scale).ceil() + padding)
        .max(left as f64)
        .min(image_width as f64) as u32;
    let bottom = ((bottom as f64 * scale).ceil() + padding)
        .max(top as f64)
        .min(image_height as f64) as u32;
    let width = right.saturating_sub(left);
    let height = bottom.saturating_sub(top);
    if width == 0 || height == 0 {
        bail!("capture region {name:?} is empty after clamping");
    }
    Ok(CaptureRegion {
        name: name.to_string(),
        x: left,
        y: top,
        width,
        height,
    })
}

impl CaptureRegion {
    pub fn crop(&self, image: &RgbaImage) -> Result<RgbaImage> {
        if self.name.trim().is_empty() {
            bail!("capture region name must not be empty");
        }
        if self.width == 0 || self.height == 0 {
            bail!("capture region dimensions must be non-zero");
        }
        let right = self
            .x
            .checked_add(self.width)
            .ok_or_else(|| anyhow!("capture region horizontal extent overflowed"))?;
        let bottom = self
            .y
            .checked_add(self.height)
            .ok_or_else(|| anyhow!("capture region vertical extent overflowed"))?;
        if right > image.width() || bottom > image.height() {
            bail!(
                "capture region {:?} exceeds image dimensions {}x{}",
                self.name,
                image.width(),
                image.height()
            );
        }
        Ok(image::imageops::crop_imm(image, self.x, self.y, self.width, self.height).to_image())
    }
}

pub struct ImageComparison {
    pub match_percentage: f64,
    pub diff_image: RgbaImage,
    pub different_pixels: u32,
    pub total_pixels: u32,
}

pub fn compare_images(
    actual: &RgbaImage,
    expected: &RgbaImage,
    channel_tolerance: u8,
) -> ImageComparison {
    let width = actual.width().max(expected.width());
    let height = actual.height().max(expected.height());
    let total_pixels = width.saturating_mul(height);
    let mut diff_image = RgbaImage::new(width, height);
    let mut matching_pixels = 0u32;

    for y in 0..height {
        for x in 0..width {
            let actual_pixel = if x < actual.width() && y < actual.height() {
                *actual.get_pixel(x, y)
            } else {
                Rgba([0, 0, 0, 0])
            };
            let expected_pixel = if x < expected.width() && y < expected.height() {
                *expected.get_pixel(x, y)
            } else {
                Rgba([0, 0, 0, 0])
            };
            if pixels_are_similar(&actual_pixel, &expected_pixel, channel_tolerance) {
                matching_pixels += 1;
                diff_image.put_pixel(x, y, Rgba([0, 255, 0, 64]));
            } else {
                diff_image.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
    }

    let match_percentage = if total_pixels == 0 {
        1.0
    } else {
        matching_pixels as f64 / total_pixels as f64
    };
    ImageComparison {
        match_percentage,
        diff_image,
        different_pixels: total_pixels.saturating_sub(matching_pixels),
        total_pixels,
    }
}

fn pixels_are_similar(first: &Rgba<u8>, second: &Rgba<u8>, tolerance: u8) -> bool {
    first
        .0
        .iter()
        .zip(second.0.iter())
        .all(|(first, second)| first.abs_diff(*second) <= tolerance)
}

#[cfg(feature = "gpui-support")]
pub fn normalized_accessibility_nodes(
    snapshot: &DebugRenderSnapshot,
) -> Result<BTreeMap<String, serde_json::Value>> {
    let Some(tree) = snapshot.accessibility_tree_json() else {
        return Ok(BTreeMap::new());
    };
    let value: serde_json::Value =
        serde_json::from_str(tree).context("parsing accessibility tree")?;
    let Some(nodes) = value.get("nodes").and_then(serde_json::Value::as_object) else {
        return Ok(BTreeMap::new());
    };
    let mut normalized = BTreeMap::new();
    for node in nodes.values() {
        let Some(element_id) = node.get("element_id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(aria) = node.get("aria") else {
            continue;
        };
        if normalized
            .insert(element_id.to_string(), aria.clone())
            .is_some()
        {
            bail!("duplicate accessibility element ID {element_id:?}");
        }
    }
    Ok(normalized)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkbenchConsistencyScenario {
    ThreadSwitch,
    WorktreeChange,
    StaleCompletion,
    Reconnect,
    ValidRestore,
    InvalidRestoreFallback,
}

impl WorkbenchConsistencyScenario {
    pub const ALL: [Self; 6] = [
        Self::ThreadSwitch,
        Self::WorktreeChange,
        Self::StaleCompletion,
        Self::Reconnect,
        Self::ValidRestore,
        Self::InvalidRestoreFallback,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ThreadSwitch => "thread_switch",
            Self::WorktreeChange => "worktree_change",
            Self::StaleCompletion => "stale_completion",
            Self::Reconnect => "reconnect",
            Self::ValidRestore => "valid_restore",
            Self::InvalidRestoreFallback => "invalid_restore_fallback",
        }
    }
}

pub fn workbench_consistency_trace(
    scenario: WorkbenchConsistencyScenario,
) -> Result<conformance::ConformanceTrace> {
    let mut recorder = WorkbenchTransitionRecorder::new();
    let repository_surfaces = projection::WorkSurface::FALLBACK_ORDER.to_vec();

    match scenario {
        WorkbenchConsistencyScenario::ThreadSwitch => {
            recorder.require([
                conformance::ActionKind::OpenThread,
                conformance::ActionKind::RequestSurface,
                conformance::ActionKind::SwitchThread,
            ]);
            recorder.apply(open_bound_thread(
                "thread-a",
                "worktree-a",
                &repository_surfaces,
            ));
            recorder.apply(projection::ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: projection::WorkSurface::Git,
            });
            recorder.apply(open_bound_thread(
                "thread-b",
                "worktree-b",
                &repository_surfaces,
            ));
            recorder.apply(projection::ProjectionTransition::SwitchThread {
                thread_id: "thread-b".into(),
            });
            recorder.apply(projection::ProjectionTransition::RequestSurface {
                thread_id: "thread-b".into(),
                surface: projection::WorkSurface::Terminal,
            });
        }
        WorkbenchConsistencyScenario::WorktreeChange => {
            recorder.require([
                conformance::ActionKind::OpenThread,
                conformance::ActionKind::RequestSurface,
                conformance::ActionKind::ChangeWorktree,
            ]);
            recorder.apply(open_bound_thread(
                "thread-a",
                "worktree-a",
                &repository_surfaces,
            ));
            recorder.apply(projection::ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: projection::WorkSurface::Git,
            });
            recorder.apply(projection::ProjectionTransition::ChangeWorktree {
                thread_id: "thread-a".into(),
                generation: 0,
                worktree_id: "worktree-b".into(),
                available_surfaces: repository_surfaces,
            });
        }
        WorkbenchConsistencyScenario::StaleCompletion => {
            recorder.require([
                conformance::ActionKind::OpenThread,
                conformance::ActionKind::BeginSurfaceLoad,
                conformance::ActionKind::ChangeWorktree,
                conformance::ActionKind::CompleteSurfaceLoad,
            ]);
            let old_binding = production_binding("worktree-a");
            let new_binding = production_binding("worktree-b");
            recorder.apply(open_bound_thread(
                "thread-a",
                "worktree-a",
                &repository_surfaces,
            ));
            recorder.apply(projection::ProjectionTransition::BeginSurfaceLoad {
                request_id: "load-old".into(),
                thread_id: "thread-a".into(),
                surface: projection::WorkSurface::Git,
                generation: 0,
                binding: Some(old_binding.clone()),
            });
            recorder.apply(projection::ProjectionTransition::ChangeWorktree {
                thread_id: "thread-a".into(),
                generation: 0,
                worktree_id: "worktree-b".into(),
                available_surfaces: repository_surfaces,
            });
            recorder.apply(projection::ProjectionTransition::CompleteSurfaceLoad {
                request_id: "load-old".into(),
                thread_id: "thread-a".into(),
                surface: projection::WorkSurface::Git,
                generation: 0,
                binding: Some(old_binding),
            });
            recorder.apply(projection::ProjectionTransition::BeginSurfaceLoad {
                request_id: "load-current".into(),
                thread_id: "thread-a".into(),
                surface: projection::WorkSurface::Git,
                generation: 1,
                binding: Some(new_binding.clone()),
            });
            recorder.apply(projection::ProjectionTransition::CompleteSurfaceLoad {
                request_id: "load-current".into(),
                thread_id: "thread-a".into(),
                surface: projection::WorkSurface::Git,
                generation: 1,
                binding: Some(new_binding),
            });
        }
        WorkbenchConsistencyScenario::Reconnect => {
            recorder.require([
                conformance::ActionKind::Disconnect,
                conformance::ActionKind::Reconnect,
                conformance::ActionKind::ReceiveProjectionSnapshot,
            ]);
            recorder.apply(open_bound_thread(
                "thread-a",
                "worktree-a",
                &repository_surfaces,
            ));
            recorder.apply(projection::ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: projection::WorkSurface::Git,
            });
            recorder.apply(projection::ProjectionTransition::Disconnect);
            recorder.apply(projection::ProjectionTransition::Reconnect);
            recorder.apply(
                projection::ProjectionTransition::ReceiveProjectionSnapshot {
                    snapshot: recorder.snapshot(0),
                },
            );
            recorder.apply(
                projection::ProjectionTransition::ReceiveProjectionSnapshot {
                    snapshot: recorder.snapshot(1),
                },
            );
        }
        WorkbenchConsistencyScenario::ValidRestore => {
            recorder.require([
                conformance::ActionKind::PersistSelection,
                conformance::ActionKind::ColdStart,
                conformance::ActionKind::RestoreSelection,
            ]);
            recorder.apply(open_bound_thread(
                "thread-a",
                "worktree-a",
                &repository_surfaces,
            ));
            recorder.apply(projection::ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: projection::WorkSurface::Review,
            });
            recorder.apply(projection::ProjectionTransition::PersistSelection { revision: 1 });
            recorder.apply(projection::ProjectionTransition::ColdStart);
            recorder.apply(projection::ProjectionTransition::RestoreSelection);
        }
        WorkbenchConsistencyScenario::InvalidRestoreFallback => {
            recorder.require([
                conformance::ActionKind::PersistSelection,
                conformance::ActionKind::ColdStart,
                conformance::ActionKind::ChangeWorktree,
                conformance::ActionKind::RestoreSelection,
            ]);
            recorder.apply(open_bound_thread(
                "thread-a",
                "worktree-a",
                &repository_surfaces,
            ));
            recorder.apply(projection::ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: projection::WorkSurface::Git,
            });
            recorder.apply(projection::ProjectionTransition::PersistSelection { revision: 1 });
            recorder.apply(projection::ProjectionTransition::ColdStart);
            recorder.apply(projection::ProjectionTransition::ChangeWorktree {
                thread_id: "thread-a".into(),
                generation: 0,
                worktree_id: "worktree-b".into(),
                available_surfaces: vec![
                    projection::WorkSurface::Files,
                    projection::WorkSurface::Plan,
                ],
            });
            recorder.apply(projection::ProjectionTransition::RestoreSelection);
        }
    }

    recorder.finish()
}

pub struct WorkbenchTransitionRecorder {
    state: projection::WorkbenchProjection,
    trace: conformance::ConformanceTrace,
    attempted_transitions: usize,
}

impl Default for WorkbenchTransitionRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkbenchTransitionRecorder {
    pub fn new() -> Self {
        let state = projection::WorkbenchProjection::new();
        let trace = conformance::ConformanceTrace::new(conformance_state(&state));
        Self {
            state,
            trace,
            attempted_transitions: 0,
        }
    }

    pub fn require(&mut self, actions: impl IntoIterator<Item = conformance::ActionKind>) {
        self.trace.required_actions.extend(actions);
    }

    pub fn state(&self) -> &projection::WorkbenchProjection {
        &self.state
    }

    pub fn apply(&mut self, transition: projection::ProjectionTransition) {
        self.attempted_transitions += 1;
        let wire_transition = conformance_transition(&transition);
        let effect = match self.state.apply(transition.clone()) {
            Ok(effect) => conformance_effect(effect),
            Err(error) => conformance::TransitionEffect::Rejected {
                code: conformance_reject_code(&transition, &error),
            },
        };
        self.trace
            .push_with_effect(wire_transition, effect, conformance_state(&self.state));
    }

    pub fn snapshot(&self, revision: u64) -> projection::ProjectionSnapshot {
        projection::ProjectionSnapshot {
            revision,
            persistence_revision: self.state.persistence_revision,
            active_thread_id: self.state.active_thread_id.clone(),
            threads: self.state.threads.clone(),
            persisted_selection: self.state.persisted_selection.clone(),
        }
    }

    pub fn finish(self) -> Result<conformance::ConformanceTrace> {
        if self.trace.steps.len() != self.attempted_transitions {
            bail!(
                "workbench trace coverage breach: attempted {} critical transitions but recorded {}",
                self.attempted_transitions,
                self.trace.steps.len()
            );
        }
        conformance::check_trace(&self.trace)
            .map_err(|error| anyhow!("workbench conformance failed: {error}"))?;
        Ok(self.trace)
    }
}

fn production_binding(worktree_id: &str) -> projection::RepositoryBinding {
    projection::RepositoryBinding {
        repository_id: "repository-a".into(),
        worktree_id: worktree_id.into(),
    }
}

fn open_bound_thread(
    thread_id: &str,
    worktree_id: &str,
    surfaces: &[projection::WorkSurface],
) -> projection::ProjectionTransition {
    projection::ProjectionTransition::OpenThread {
        thread_id: thread_id.into(),
        binding: Some(production_binding(worktree_id)),
        available_surfaces: surfaces.to_vec(),
    }
}

fn conformance_state(state: &projection::WorkbenchProjection) -> conformance::WorkbenchState {
    let threads = state
        .threads
        .iter()
        .map(|(thread_id, thread)| {
            (
                conformance::ThreadId(thread_id.clone()),
                conformance::ThreadState {
                    generation: thread.generation,
                    binding: thread.binding.as_ref().map(conformance_binding),
                    available_surfaces: thread
                        .available_surfaces
                        .iter()
                        .copied()
                        .map(conformance_surface)
                        .collect(),
                    requested_surface: thread.requested_surface.map(conformance_surface),
                    effective_surface: thread.effective_surface.map(conformance_surface),
                    dock_visible: thread.dock_open,
                    focus_owner: thread.focus_owner.map(conformance_surface),
                    artifact_revision: thread.artifact_revision,
                    event_revision: thread.event_revision,
                },
            )
        })
        .collect();
    let pending_loads = state
        .pending_loads
        .iter()
        .map(|(request_id, load)| {
            (
                conformance::RequestId(request_id.clone()),
                conformance::PendingLoad {
                    thread_id: conformance::ThreadId(load.thread_id.clone()),
                    surface: conformance_surface(load.surface),
                    generation: load.generation,
                    binding: load.binding.as_ref().map(conformance_binding),
                },
            )
        })
        .collect();
    let mut projected = conformance::WorkbenchState {
        projection_revision: state.projection_revision,
        persistence_revision: state.persistence_revision,
        connection: conformance_connection(state.connection),
        active_thread: state
            .active_thread_id
            .as_ref()
            .map(|thread_id| conformance::ThreadId(thread_id.clone())),
        threads,
        pending_loads,
        persisted_selection: state
            .persisted_selection
            .as_ref()
            .map(conformance_persisted_selection),
        restore_pending: state.restore_pending,
        visible_projection: None,
    };
    projected.visible_projection = projected.expected_visible_projection();
    projected
}

fn conformance_transition(
    transition: &projection::ProjectionTransition,
) -> conformance::Transition {
    match transition {
        projection::ProjectionTransition::OpenThread {
            thread_id,
            binding,
            available_surfaces,
        } => conformance::Transition::OpenThread {
            thread_id: conformance::ThreadId(thread_id.clone()),
            seed: conformance::ThreadSeed::new(
                0,
                binding.as_ref().map(conformance_binding),
                available_surfaces.iter().copied().map(conformance_surface),
            ),
        },
        projection::ProjectionTransition::CloseThread { thread_id } => {
            conformance::Transition::CloseThread {
                thread_id: conformance::ThreadId(thread_id.clone()),
            }
        }
        projection::ProjectionTransition::SwitchThread { thread_id } => {
            conformance::Transition::SwitchThread {
                thread_id: conformance::ThreadId(thread_id.clone()),
            }
        }
        projection::ProjectionTransition::RequestSurface { thread_id, surface } => {
            conformance::Transition::RequestSurface {
                thread_id: conformance::ThreadId(thread_id.clone()),
                surface: conformance_surface(*surface),
            }
        }
        projection::ProjectionTransition::CloseSurface { thread_id } => {
            conformance::Transition::CloseSurface {
                thread_id: conformance::ThreadId(thread_id.clone()),
            }
        }
        projection::ProjectionTransition::CollapseDock { thread_id } => {
            conformance::Transition::CollapseDock {
                thread_id: conformance::ThreadId(thread_id.clone()),
            }
        }
        projection::ProjectionTransition::ExpandDock { thread_id } => {
            conformance::Transition::ExpandDock {
                thread_id: conformance::ThreadId(thread_id.clone()),
            }
        }
        projection::ProjectionTransition::BindRepository {
            thread_id,
            generation,
            binding,
            available_surfaces,
        } => conformance::Transition::BindRepository {
            thread_id: conformance::ThreadId(thread_id.clone()),
            generation: *generation,
            binding: conformance_binding(binding),
            available_surfaces: available_surfaces
                .iter()
                .copied()
                .map(conformance_surface)
                .collect(),
        },
        projection::ProjectionTransition::ChangeWorktree {
            thread_id,
            generation,
            worktree_id,
            available_surfaces,
        } => conformance::Transition::ChangeWorktree {
            thread_id: conformance::ThreadId(thread_id.clone()),
            generation: *generation,
            worktree_id: conformance::WorktreeId(worktree_id.clone()),
            available_surfaces: available_surfaces
                .iter()
                .copied()
                .map(conformance_surface)
                .collect(),
        },
        projection::ProjectionTransition::RemoveBinding {
            thread_id,
            generation,
            available_surfaces,
        } => conformance::Transition::RemoveBinding {
            thread_id: conformance::ThreadId(thread_id.clone()),
            generation: *generation,
            available_surfaces: available_surfaces
                .iter()
                .copied()
                .map(conformance_surface)
                .collect(),
        },
        projection::ProjectionTransition::ChangeBinding {
            thread_id,
            generation,
            binding,
            available_surfaces,
        } => conformance::Transition::ChangeBinding {
            thread_id: conformance::ThreadId(thread_id.clone()),
            generation: *generation,
            binding: binding.as_ref().map(conformance_binding),
            available_surfaces: available_surfaces
                .iter()
                .copied()
                .map(conformance_surface)
                .collect(),
        },
        projection::ProjectionTransition::BeginSurfaceLoad {
            request_id,
            thread_id,
            surface,
            generation,
            binding,
        } => conformance::Transition::BeginSurfaceLoad {
            request_id: conformance::RequestId(request_id.clone()),
            thread_id: conformance::ThreadId(thread_id.clone()),
            surface: conformance_surface(*surface),
            generation: *generation,
            binding: binding.as_ref().map(conformance_binding),
        },
        projection::ProjectionTransition::CompleteSurfaceLoad {
            request_id,
            thread_id,
            surface,
            generation,
            binding,
        } => conformance::Transition::CompleteSurfaceLoad {
            request_id: conformance::RequestId(request_id.clone()),
            thread_id: conformance::ThreadId(thread_id.clone()),
            surface: conformance_surface(*surface),
            generation: *generation,
            binding: binding.as_ref().map(conformance_binding),
        },
        projection::ProjectionTransition::FailSurfaceLoad {
            request_id,
            thread_id,
            surface,
            generation,
            binding,
        } => conformance::Transition::FailSurfaceLoad {
            request_id: conformance::RequestId(request_id.clone()),
            thread_id: conformance::ThreadId(thread_id.clone()),
            surface: conformance_surface(*surface),
            generation: *generation,
            binding: binding.as_ref().map(conformance_binding),
        },
        projection::ProjectionTransition::Disconnect => conformance::Transition::Disconnect,
        projection::ProjectionTransition::Reconnect => conformance::Transition::Reconnect,
        projection::ProjectionTransition::ReceiveProjectionSnapshot { snapshot } => {
            conformance::Transition::ReceiveProjectionSnapshot {
                snapshot: conformance::ProjectionSnapshot {
                    revision: snapshot.revision,
                    persistence_revision: snapshot.persistence_revision,
                    active_thread: snapshot
                        .active_thread_id
                        .as_ref()
                        .map(|thread_id| conformance::ThreadId(thread_id.clone())),
                    threads: snapshot
                        .threads
                        .iter()
                        .map(|(thread_id, thread)| conformance::SnapshotThread {
                            thread_id: conformance::ThreadId(thread_id.clone()),
                            seed: conformance::ThreadSeed {
                                generation: thread.generation,
                                binding: thread.binding.as_ref().map(conformance_binding),
                                available_surfaces: thread
                                    .available_surfaces
                                    .iter()
                                    .copied()
                                    .map(conformance_surface)
                                    .collect(),
                                requested_surface: thread
                                    .requested_surface
                                    .map(conformance_surface),
                                dock_visible: thread.dock_open,
                                artifact_revision: thread.artifact_revision,
                                event_revision: thread.event_revision,
                            },
                        })
                        .collect(),
                    persisted_selection: snapshot
                        .persisted_selection
                        .as_ref()
                        .map(conformance_persisted_selection),
                },
            }
        }
        projection::ProjectionTransition::PersistSelection { revision } => {
            conformance::Transition::PersistSelection {
                revision: *revision,
            }
        }
        projection::ProjectionTransition::ColdStart => conformance::Transition::ColdStart,
        projection::ProjectionTransition::RestoreSelection => {
            conformance::Transition::RestoreSelection
        }
        projection::ProjectionTransition::InvalidateCapability {
            thread_id,
            generation,
            surface,
        } => conformance::Transition::InvalidateCapability {
            thread_id: conformance::ThreadId(thread_id.clone()),
            generation: *generation,
            surface: conformance_surface(*surface),
        },
        projection::ProjectionTransition::DispatchSurfaceCommand {
            thread_id,
            surface,
            binding,
            generation,
        } => conformance::Transition::DispatchSurfaceCommand {
            thread_id: conformance::ThreadId(thread_id.clone()),
            surface: conformance_surface(*surface),
            generation: *generation,
            binding: binding.as_ref().map(conformance_binding),
        },
    }
}

fn conformance_binding(binding: &projection::RepositoryBinding) -> conformance::Binding {
    conformance::Binding::new(binding.repository_id.clone(), binding.worktree_id.clone())
}

fn conformance_persisted_selection(
    selection: &projection::PersistedSelection,
) -> conformance::PersistedSelection {
    conformance::PersistedSelection {
        revision: selection.revision,
        thread_id: conformance::ThreadId(selection.thread_id.clone()),
        generation: selection.generation,
        binding: selection.binding.as_ref().map(conformance_binding),
        requested_surface: selection.requested_surface.map(conformance_surface),
        dock_visible: selection.dock_open,
    }
}

fn conformance_surface(surface: projection::WorkSurface) -> conformance::SurfaceId {
    match surface {
        projection::WorkSurface::Files => conformance::SurfaceId::Files,
        projection::WorkSurface::Search => conformance::SurfaceId::Search,
        projection::WorkSurface::Review => conformance::SurfaceId::Review,
        projection::WorkSurface::Git => conformance::SurfaceId::Git,
        projection::WorkSurface::Terminal => conformance::SurfaceId::Terminal,
        projection::WorkSurface::Plan => conformance::SurfaceId::Plan,
    }
}

fn conformance_connection(connection: projection::ConnectionPhase) -> conformance::ConnectionPhase {
    match connection {
        projection::ConnectionPhase::Online => conformance::ConnectionPhase::Online,
        projection::ConnectionPhase::Offline => conformance::ConnectionPhase::Offline,
        projection::ConnectionPhase::Reconnecting => conformance::ConnectionPhase::Reconnecting,
        projection::ConnectionPhase::StaleProjection => {
            conformance::ConnectionPhase::StaleProjection
        }
    }
}

fn conformance_effect(effect: projection::TransitionEffect) -> conformance::TransitionEffect {
    match effect {
        projection::TransitionEffect::Applied => conformance::TransitionEffect::Applied,
        projection::TransitionEffect::StaleCompletionIgnored => {
            conformance::TransitionEffect::StaleCompletionIgnored
        }
        projection::TransitionEffect::OlderRevisionIgnored => {
            conformance::TransitionEffect::OlderRevisionIgnored
        }
        projection::TransitionEffect::DeterministicFallback => {
            conformance::TransitionEffect::DeterministicFallback
        }
    }
}

fn conformance_reject_code(
    transition: &projection::ProjectionTransition,
    error: &projection::ProjectionError,
) -> conformance::RejectCode {
    use conformance::RejectCode;
    use projection::ProjectionError;

    match error {
        ProjectionError::InvalidConnectionTransition { .. } => RejectCode::InvalidConnectionPhase,
        ProjectionError::InvalidState(_)
        | ProjectionError::UnknownThread(_)
        | ProjectionError::InvalidId { .. }
            if matches!(
                transition,
                projection::ProjectionTransition::ReceiveProjectionSnapshot { .. }
            ) =>
        {
            RejectCode::InvalidSnapshot
        }
        ProjectionError::InvalidState(_) | ProjectionError::InvalidBinding(_) => {
            RejectCode::InvalidBinding
        }
        ProjectionError::InvalidId { .. } => RejectCode::InvalidIdentifier,
        ProjectionError::DuplicateThread(_) => RejectCode::DuplicateThread,
        ProjectionError::UnknownThread(_) => RejectCode::UnknownThread,
        ProjectionError::DuplicateRequest(_) => RejectCode::DuplicateRequest,
        ProjectionError::UnknownRequest(_) => RejectCode::UnknownRequest,
        ProjectionError::RequestContextMismatch(_) => RejectCode::RequestContextMismatch,
        ProjectionError::UnavailableSurface { .. } => RejectCode::UnavailableSurface,
        ProjectionError::NoActiveThread => RejectCode::NoActiveSelection,
        ProjectionError::NoPersistedSelection => RejectCode::NoPersistedSelection,
        ProjectionError::RestoreNotPending => RejectCode::RestoreNotPending,
        ProjectionError::AlreadyBound(_) => RejectCode::AlreadyBound,
        ProjectionError::AlreadyUnbound(_) => RejectCode::AlreadyUnbound,
        ProjectionError::CapabilityAlreadyUnavailable { .. } => {
            RejectCode::CapabilityAlreadyUnavailable
        }
        ProjectionError::StaleGeneration { .. } => RejectCode::StaleGeneration,
        ProjectionError::CommandBindingMismatch(_) => RejectCode::CommandBindingMismatch,
        ProjectionError::InactiveThread { .. } => RejectCode::InactiveThread,
        ProjectionError::RevisionOverflow { .. } => RejectCode::RevisionOverflow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingInteractionBackend {
        selectors: Vec<String>,
        restarts: usize,
    }

    impl WorkbenchInteractionBackend for RecordingInteractionBackend {
        fn activate_selector(&mut self, selector: &str) -> Result<()> {
            self.selectors.push(selector.to_string());
            Ok(())
        }

        fn restart(&mut self) -> Result<()> {
            self.restarts += 1;
            Ok(())
        }
    }

    #[test]
    fn consistency_scenarios_conform_to_the_independent_checker() {
        for scenario in WorkbenchConsistencyScenario::ALL {
            let trace = workbench_consistency_trace(scenario)
                .unwrap_or_else(|error| panic!("{} failed: {error}", scenario.as_str()));
            let report = conformance::check_trace(&trace)
                .unwrap_or_else(|error| panic!("{} was rejected: {error}", scenario.as_str()));
            assert_eq!(report.steps_checked, trace.steps.len());

            let state = &report.final_state;
            match scenario {
                WorkbenchConsistencyScenario::ThreadSwitch => {
                    let visible = state
                        .visible_projection
                        .as_ref()
                        .expect("switch has a visible projection");
                    assert_eq!(visible.thread_id.0, "thread-b");
                    assert_eq!(
                        visible.effective_surface,
                        Some(conformance::SurfaceId::Terminal)
                    );
                    assert_eq!(visible.focus_owner, Some(conformance::SurfaceId::Terminal));
                    assert_eq!(visible.artifact_outline.thread_id.0, "thread-b");
                    assert_eq!(visible.event_outline.thread_id.0, "thread-b");
                    assert_eq!(
                        visible
                            .binding
                            .as_ref()
                            .map(|binding| binding.worktree_id.0.as_str()),
                        Some("worktree-b")
                    );
                }
                WorkbenchConsistencyScenario::WorktreeChange => {
                    let thread = state
                        .threads
                        .get(&conformance::ThreadId("thread-a".into()))
                        .expect("worktree scenario thread");
                    assert_eq!(thread.generation, 1);
                    assert_eq!(
                        thread
                            .binding
                            .as_ref()
                            .map(|binding| binding.worktree_id.0.as_str()),
                        Some("worktree-b")
                    );
                }
                WorkbenchConsistencyScenario::StaleCompletion => {
                    let thread = state
                        .threads
                        .get(&conformance::ThreadId("thread-a".into()))
                        .expect("stale completion thread");
                    assert_eq!(thread.generation, 1);
                    assert_eq!(thread.artifact_revision, 1);
                    assert_eq!(thread.event_revision, 1);
                    assert!(state.pending_loads.is_empty());
                    assert!(trace.steps.iter().any(|step| {
                        step.observed_effect
                            == conformance::TransitionEffect::StaleCompletionIgnored
                    }));
                }
                WorkbenchConsistencyScenario::Reconnect => {
                    assert_eq!(state.connection, conformance::ConnectionPhase::Online);
                    assert_eq!(state.projection_revision, 1);
                    assert!(trace.steps.iter().any(|step| {
                        step.observed_effect == conformance::TransitionEffect::OlderRevisionIgnored
                    }));
                }
                WorkbenchConsistencyScenario::ValidRestore => {
                    let visible = state
                        .visible_projection
                        .as_ref()
                        .expect("restored visible projection");
                    assert_eq!(visible.thread_id.0, "thread-a");
                    assert_eq!(
                        visible.effective_surface,
                        Some(conformance::SurfaceId::Review)
                    );
                    assert_eq!(visible.focus_owner, Some(conformance::SurfaceId::Review));
                }
                WorkbenchConsistencyScenario::InvalidRestoreFallback => {
                    let visible = state
                        .visible_projection
                        .as_ref()
                        .expect("fallback visible projection");
                    assert_eq!(
                        visible.effective_surface,
                        Some(conformance::SurfaceId::Files)
                    );
                    assert_eq!(
                        visible
                            .binding
                            .as_ref()
                            .map(|binding| binding.worktree_id.0.as_str()),
                        Some("worktree-b")
                    );
                    let persisted = state
                        .persisted_selection
                        .as_ref()
                        .expect("fallback rewrites persisted selection");
                    assert_eq!(
                        persisted.requested_surface,
                        Some(conformance::SurfaceId::Files)
                    );
                    assert_eq!(persisted.generation, 1);
                }
            }
        }
    }

    #[test]
    fn rejected_production_transition_is_recorded_without_mutating_state() {
        let mut recorder = WorkbenchTransitionRecorder::new();
        let surfaces = projection::WorkSurface::FALLBACK_ORDER.to_vec();
        recorder.apply(open_bound_thread("thread-a", "worktree-a", &surfaces));
        let before = recorder.state.clone();
        recorder.apply(open_bound_thread("thread-a", "worktree-b", &surfaces));
        assert_eq!(recorder.state, before);

        let trace = recorder.finish().expect("rejected trace conforms");
        assert_eq!(
            trace.steps.last().map(|step| step.observed_effect),
            Some(conformance::TransitionEffect::Rejected {
                code: conformance::RejectCode::DuplicateThread,
            })
        );
    }

    #[test]
    fn rejected_production_paths_match_closed_checker_reasons() {
        let mut recorder = WorkbenchTransitionRecorder::new();
        let surfaces = projection::WorkSurface::FALLBACK_ORDER.to_vec();
        recorder.apply(projection::ProjectionTransition::RequestSurface {
            thread_id: "thread-a".into(),
            surface: projection::WorkSurface::Git,
        });
        recorder.apply(open_bound_thread("thread-a", "worktree-a", &surfaces));
        recorder.apply(projection::ProjectionTransition::RequestSurface {
            thread_id: "thread-a".into(),
            surface: projection::WorkSurface::Git,
        });
        recorder.apply(projection::ProjectionTransition::CollapseDock {
            thread_id: "thread-a".into(),
        });
        recorder.apply(projection::ProjectionTransition::DispatchSurfaceCommand {
            thread_id: "thread-a".into(),
            surface: projection::WorkSurface::Git,
            binding: Some(production_binding("worktree-a")),
            generation: 0,
        });
        recorder.apply(projection::ProjectionTransition::ChangeWorktree {
            thread_id: "thread-a".into(),
            generation: 7,
            worktree_id: "worktree-b".into(),
            available_surfaces: surfaces,
        });
        recorder.apply(projection::ProjectionTransition::RestoreSelection);
        recorder.apply(projection::ProjectionTransition::Disconnect);
        recorder.apply(projection::ProjectionTransition::Disconnect);

        let trace = recorder.finish().expect("rejected paths conform");
        let rejected_codes: Vec<_> = trace
            .steps
            .iter()
            .filter_map(|step| match step.observed_effect {
                conformance::TransitionEffect::Rejected { code } => Some(code),
                _ => None,
            })
            .collect();
        assert_eq!(
            rejected_codes,
            [
                conformance::RejectCode::InactiveThread,
                conformance::RejectCode::UnavailableSurface,
                conformance::RejectCode::StaleGeneration,
                conformance::RejectCode::RestoreNotPending,
                conformance::RejectCode::InvalidConnectionPhase,
            ]
        );
    }

    #[test]
    fn production_trace_adapter_covers_every_critical_action() {
        let mut recorder = WorkbenchTransitionRecorder::new();
        recorder.require(conformance::ActionKind::ALL);
        let surfaces = projection::WorkSurface::FALLBACK_ORDER.to_vec();
        let binding_a = production_binding("worktree-a");
        recorder.apply(open_bound_thread("thread-a", "worktree-a", &surfaces));
        recorder.apply(projection::ProjectionTransition::RequestSurface {
            thread_id: "thread-a".into(),
            surface: projection::WorkSurface::Git,
        });
        recorder.apply(projection::ProjectionTransition::DispatchSurfaceCommand {
            thread_id: "thread-a".into(),
            surface: projection::WorkSurface::Git,
            binding: Some(binding_a.clone()),
            generation: 0,
        });
        recorder.apply(projection::ProjectionTransition::CollapseDock {
            thread_id: "thread-a".into(),
        });
        recorder.apply(projection::ProjectionTransition::ExpandDock {
            thread_id: "thread-a".into(),
        });
        recorder.apply(projection::ProjectionTransition::CloseSurface {
            thread_id: "thread-a".into(),
        });
        recorder.apply(projection::ProjectionTransition::BeginSurfaceLoad {
            request_id: "load-complete".into(),
            thread_id: "thread-a".into(),
            surface: projection::WorkSurface::Git,
            generation: 0,
            binding: Some(binding_a.clone()),
        });
        recorder.apply(projection::ProjectionTransition::CompleteSurfaceLoad {
            request_id: "load-complete".into(),
            thread_id: "thread-a".into(),
            surface: projection::WorkSurface::Git,
            generation: 0,
            binding: Some(binding_a.clone()),
        });
        recorder.apply(projection::ProjectionTransition::BeginSurfaceLoad {
            request_id: "load-fail".into(),
            thread_id: "thread-a".into(),
            surface: projection::WorkSurface::Terminal,
            generation: 0,
            binding: Some(binding_a),
        });
        recorder.apply(projection::ProjectionTransition::FailSurfaceLoad {
            request_id: "load-fail".into(),
            thread_id: "thread-a".into(),
            surface: projection::WorkSurface::Terminal,
            generation: 0,
            binding: Some(production_binding("worktree-a")),
        });
        recorder.apply(projection::ProjectionTransition::InvalidateCapability {
            thread_id: "thread-a".into(),
            generation: 0,
            surface: projection::WorkSurface::Search,
        });
        recorder.apply(projection::ProjectionTransition::ChangeWorktree {
            thread_id: "thread-a".into(),
            generation: 1,
            worktree_id: "worktree-b".into(),
            available_surfaces: surfaces.clone(),
        });
        recorder.apply(projection::ProjectionTransition::RemoveBinding {
            thread_id: "thread-a".into(),
            generation: 2,
            available_surfaces: vec![projection::WorkSurface::Plan],
        });
        recorder.apply(projection::ProjectionTransition::BindRepository {
            thread_id: "thread-a".into(),
            generation: 3,
            binding: production_binding("worktree-c"),
            available_surfaces: surfaces.clone(),
        });
        recorder.apply(projection::ProjectionTransition::ChangeBinding {
            thread_id: "thread-a".into(),
            generation: 4,
            binding: Some(
                projection::RepositoryBinding::new("repository-b", "worktree-d")
                    .expect("valid replacement binding"),
            ),
            available_surfaces: surfaces.clone(),
        });
        recorder.apply(projection::ProjectionTransition::PersistSelection { revision: 1 });
        recorder.apply(projection::ProjectionTransition::ColdStart);
        recorder.apply(projection::ProjectionTransition::RestoreSelection);
        recorder.apply(projection::ProjectionTransition::Disconnect);
        recorder.apply(projection::ProjectionTransition::Reconnect);
        recorder.apply(
            projection::ProjectionTransition::ReceiveProjectionSnapshot {
                snapshot: recorder.snapshot(1),
            },
        );
        recorder.apply(open_bound_thread("thread-b", "worktree-b", &surfaces));
        recorder.apply(projection::ProjectionTransition::SwitchThread {
            thread_id: "thread-b".into(),
        });
        recorder.apply(projection::ProjectionTransition::CloseThread {
            thread_id: "thread-b".into(),
        });

        let trace = recorder.finish().expect("all critical actions conform");
        let report = conformance::check_trace(&trace).expect("full trace remains conformant");
        assert_eq!(
            report.seen_actions,
            conformance::ActionKind::ALL.into_iter().collect()
        );
    }

    #[test]
    fn named_interactions_use_stable_semantic_targets() -> Result<()> {
        let mut driver = WorkbenchInteractionDriver::new(RecordingInteractionBackend::default());
        driver.select_rail_item(WorkSurfaceId::Files)?;
        driver.open_dock()?;
        driver.collapse_dock()?;
        driver.switch_thread("thread-a")?;
        driver.change_worktree("worktree_a")?;
        driver.focus_surface(WorkSurfaceId::Terminal)?;
        driver.restart()?;

        let backend = driver.into_backend();
        assert_eq!(
            backend.selectors,
            [
                "omega.workbench.control.rail.files",
                DOCK_OPEN_CONTROL,
                DOCK_COLLAPSE_CONTROL,
                "omega.workbench.control.thread.thread-a",
                "omega.workbench.control.worktree.worktree_a",
                "omega.workbench.surface.terminal",
            ]
        );
        assert_eq!(backend.restarts, 1);
        assert!(control_selector("omega.workbench.control.thread", "thread/a").is_err());
        Ok(())
    }

    fn valid_scene() -> WorkbenchScene {
        let mut scene = WorkbenchScene::empty("fixture", ViewportFixture::new(900, 720, 2000));
        scene.threads.push(ThreadFixture {
            id: "thread-a".to_string(),
            project_id: Some("project-a".to_string()),
            repository_id: Some("repo-a".to_string()),
            worktree_id: Some("worktree-a".to_string()),
        });
        scene.active_thread_id = Some("thread-a".to_string());
        scene.project = Some(ProjectFixture {
            id: "project-a".to_string(),
            display_name: "Project A".to_string(),
        });
        scene.repositories.push(RepositoryFixture {
            id: "repo-a".to_string(),
            project_id: "project-a".to_string(),
            worktrees: vec![
                WorktreeFixture {
                    id: "worktree-a".to_string(),
                    branch: Some("main".to_string()),
                    git_state: None,
                    dirty_files: 0,
                    conflicts: 0,
                    ahead: 0,
                    behind: 0,
                },
                WorktreeFixture {
                    id: "worktree-b".to_string(),
                    branch: Some("feature".to_string()),
                    git_state: None,
                    dirty_files: 2,
                    conflicts: 1,
                    ahead: 3,
                    behind: 1,
                },
            ],
        });
        scene.content_state = ContentStateFixture::Ready;
        scene.surfaces[0].available = true;
        scene.active_surface = Some(WorkSurfaceId::Files);
        scene.dock_open = true;
        scene
    }

    fn valid_v2_scene() -> WorkbenchScene {
        let mut scene = valid_scene();
        scene.fixture_version = 2;
        scene.thread_workbenches.push(ThreadWorkbenchFixture {
            thread_id: "thread-a".to_string(),
            generation: 7,
            binding: Some(WorkbenchBindingFixture {
                repository_id: "repo-a".to_string(),
                worktree_id: "worktree-a".to_string(),
            }),
            requested_surface: Some(WorkSurfaceId::Files),
            effective_surface: Some(WorkSurfaceId::Files),
            dock_open: true,
            surfaces: scene.surfaces.clone(),
        });
        scene
    }

    #[test]
    fn review_scene_catalog_uses_two_distinct_thread_worktree_checkpoints() -> Result<()> {
        let names = [
            "omega_workbench_review_empty",
            "omega_workbench_review_multi_file",
            "omega_workbench_review_selected_hunk",
            "omega_workbench_review_streaming_update",
            "omega_workbench_review_rename_delete",
            "omega_workbench_review_conflict",
            "omega_workbench_review_all_reviewed",
            "omega_workbench_review_narrow",
            "omega_workbench_review_error",
        ];
        for name in names {
            let scene = workbench_review_scene(name)?;
            assert_eq!(scene.fixture_version, 2);
            assert_eq!(scene.threads.len(), 2);
            assert_eq!(scene.review_sessions.len(), 2);
            assert_eq!(scene.active_surface, Some(WorkSurfaceId::Review));
            assert!(scene.dock_open);

            let active = scene
                .active_review_session()
                .context("Review fixture has no active session")?;
            let foreign = scene
                .review_sessions
                .iter()
                .find(|review| review.binding.thread_id != active.binding.thread_id)
                .context("Review fixture has no foreign session")?;
            assert_ne!(active.binding.thread_id, foreign.binding.thread_id);
            assert_ne!(active.binding.session_id, foreign.binding.session_id);
            assert_ne!(active.binding.worktree_id, foreign.binding.worktree_id);
            assert_ne!(
                active.binding.checkpoint.action_log_entity_id,
                foreign.binding.checkpoint.action_log_entity_id
            );
            assert_ne!(
                active.binding.checkpoint.generation,
                foreign.binding.checkpoint.generation
            );
        }
        Ok(())
    }

    #[test]
    fn review_fixture_validation_rejects_cross_binding_and_invalid_selection() -> Result<()> {
        let scene = workbench_review_scene("omega_workbench_review_selected_hunk")?;

        let mut cross_bound = scene.clone();
        cross_bound.review_sessions[0].binding.worktree_id = "alpha-worktree".into();
        assert!(
            cross_bound
                .validate()
                .expect_err("cross-bound Review fixture must fail")
                .to_string()
                .contains("does not match its repository/worktree binding")
        );

        let mut invalid_selection = scene;
        invalid_selection.review_sessions[0].selected_hunk_id = Some("foreign-hunk".into());
        assert!(
            invalid_selection
                .validate()
                .expect_err("foreign hunk selection must fail")
                .to_string()
                .contains("outside file")
        );
        Ok(())
    }

    #[test]
    fn review_proof_checks_identity_counts_status_selection_focus_and_leaks() -> Result<()> {
        let scene = workbench_review_scene("omega_workbench_review_selected_hunk")?;
        let actual = scene
            .active_review_session()
            .context("Review fixture has no active session")?
            .clone();
        let checks = prove_review_surface(&scene, &actual)?;
        let names = checks
            .iter()
            .map(|check| check.name.as_str())
            .collect::<BTreeSet<_>>();
        for required in [
            "review-binding-identity",
            "review-lifecycle",
            "review-file-count",
            "review-hunk-count",
            "review-ordered-file-hunk-status",
            "review-selected-file",
            "review-selected-hunk",
            "review-focus-owner",
            "review-filesystem-mutations",
            "review-pending-operation-count",
            "review-ignored-stale-completion-count",
            "review-no-foreign-thread-files",
        ] {
            assert!(names.contains(required), "missing Review proof {required}");
        }

        let mut leaked = actual;
        leaked.files.push(scene.review_sessions[1].files[0].clone());
        assert!(prove_review_surface(&scene, &leaked).is_err());
        Ok(())
    }

    #[test]
    fn review_proof_observes_mutations_and_stale_completion_rejection() -> Result<()> {
        let all_reviewed = workbench_review_scene("omega_workbench_review_all_reviewed")?;
        let actual = all_reviewed
            .active_review_session()
            .context("all-reviewed fixture has no active session")?
            .clone();
        assert_eq!(actual.mutations.len(), 3);
        prove_review_surface(&all_reviewed, &actual)?;

        let mut stale_scene = workbench_review_scene("omega_workbench_review_streaming_update")?;
        stale_scene.review_sessions[0].ignored_stale_completion_count = 1;
        stale_scene.validate()?;
        let stale_actual = stale_scene
            .active_review_session()
            .context("streaming fixture has no active session")?
            .clone();
        prove_review_surface(&stale_scene, &stale_actual)?;

        let mut missed_rejection = stale_actual;
        missed_rejection.ignored_stale_completion_count = 0;
        assert!(prove_review_surface(&stale_scene, &missed_rejection).is_err());
        Ok(())
    }

    #[test]
    fn git_scene_catalog_uses_distinct_repository_entities_and_generations() -> Result<()> {
        let names = [
            "omega_workbench_git_clean",
            "omega_workbench_git_dirty",
            "omega_workbench_git_staged",
            "omega_workbench_git_conflict",
            "omega_workbench_git_detached",
            "omega_workbench_git_unborn",
            "omega_workbench_git_pending",
            "omega_workbench_git_multi_repository",
            "omega_workbench_git_repository_removed",
            "omega_workbench_git_offline",
            "omega_workbench_git_reconnect",
            "omega_workbench_git_error",
        ];
        for name in names {
            let scene = workbench_git_scene(name)?;
            assert_eq!(scene.fixture_version, 2);
            assert_eq!(scene.repositories.len(), 2);
            assert_eq!(scene.git_snapshots.len(), 2);
            assert_eq!(scene.active_surface, Some(WorkSurfaceId::Git));
            assert!(scene.dock_open);

            let active = scene
                .active_git_snapshot()
                .context("Git fixture has no active snapshot")?;
            let foreign = scene
                .git_snapshots
                .iter()
                .find(|snapshot| snapshot.binding.thread_id != active.binding.thread_id)
                .context("Git fixture has no foreign snapshot")?;
            assert_ne!(active.binding.thread_id, foreign.binding.thread_id);
            assert_ne!(active.binding.repository_id, foreign.binding.repository_id);
            assert_ne!(active.binding.worktree_id, foreign.binding.worktree_id);
            assert_ne!(
                active.binding.repository_entity_id,
                foreign.binding.repository_entity_id
            );
            assert_ne!(active.binding.generation, foreign.binding.generation);
        }
        Ok(())
    }

    #[test]
    fn git_fixture_validation_rejects_cross_binding_order_counts_and_badges() -> Result<()> {
        let scene = workbench_git_scene("omega_workbench_git_dirty")?;

        let mut cross_bound = scene.clone();
        cross_bound
            .git_snapshots
            .first_mut()
            .context("active Git snapshot")?
            .binding
            .repository_id = "visual-repository-alpha".into();
        assert!(
            cross_bound
                .validate()
                .expect_err("cross-bound Git fixture must fail")
                .to_string()
                .contains("does not match its repository/worktree binding")
        );

        let mut unordered = scene.clone();
        unordered
            .git_snapshots
            .first_mut()
            .context("active Git snapshot")?
            .status_entries
            .reverse();
        assert!(
            unordered
                .validate()
                .expect_err("unordered Git status must fail")
                .to_string()
                .contains("ordered by path")
        );

        let mut wrong_counts = scene.clone();
        wrong_counts
            .git_snapshots
            .first_mut()
            .context("active Git snapshot")?
            .status_counts
            .staged = 1;
        assert!(
            wrong_counts
                .validate()
                .expect_err("incorrect Git counts must fail")
                .to_string()
                .contains("do not match entries")
        );

        let mut wrong_badge = scene;
        let active_workbench = wrong_badge
            .thread_workbenches
            .iter_mut()
            .find(|workbench| workbench.thread_id == "active-thread")
            .context("active workbench")?;
        let git_surface = active_workbench
            .surfaces
            .iter_mut()
            .find(|surface| surface.id == WorkSurfaceId::Git)
            .context("Git surface")?;
        git_surface.badge = Some(99);
        wrong_badge.surfaces = active_workbench.surfaces.clone();
        assert!(
            wrong_badge
                .validate()
                .expect_err("disagreeing Git badge must fail")
                .to_string()
                .contains("disagrees with typed snapshot badge")
        );
        Ok(())
    }

    #[test]
    fn git_proof_checks_identity_status_operations_focus_badge_and_leaks() -> Result<()> {
        let scene = workbench_git_scene("omega_workbench_git_pending")?;
        let actual = scene
            .active_git_snapshot()
            .context("Git fixture has no active snapshot")?
            .clone();
        let checks = prove_git_surface(&scene, &actual)?;
        let names = checks
            .iter()
            .map(|check| check.name.as_str())
            .collect::<BTreeSet<_>>();
        for required in [
            "git-binding-identity",
            "git-lifecycle",
            "git-branch-state",
            "git-ordered-status-staging",
            "git-status-counts",
            "git-selected-path",
            "git-pending-operation",
            "git-badge-count",
            "git-badge-agreement",
            "git-requested-mutations-results",
            "git-ignored-stale-refresh-count",
            "git-focus-owner",
            "git-no-foreign-repository-status",
        ] {
            assert!(names.contains(required), "missing Git proof {required}");
        }

        let mut leaked = actual;
        let foreign_entry = scene
            .git_snapshots
            .iter()
            .find(|snapshot| snapshot.binding.thread_id == "foreign-thread")
            .and_then(|snapshot| snapshot.status_entries.first())
            .context("foreign Git status entry")?
            .clone();
        leaked.status_entries.push(foreign_entry);
        assert!(
            prove_git_surface(&scene, &leaked)
                .expect_err("foreign repository status must fail")
                .to_string()
                .contains("foreign-repository paths")
        );
        Ok(())
    }

    #[test]
    fn git_proof_observes_cancelled_destructive_action_and_stale_refresh_rejection() -> Result<()> {
        let conflict = workbench_git_scene("omega_workbench_git_conflict")?;
        let conflict_actual = conflict
            .active_git_snapshot()
            .context("conflict fixture has no active Git snapshot")?
            .clone();
        assert_eq!(
            conflict_actual.requested_mutations,
            vec![GitMutationFixture {
                kind: GitOperationKindFixture::Discard,
                target: GitMutationTargetFixture::Path("src/conflicted.rs".into()),
                result: GitMutationResultFixture::Cancelled,
            }]
        );
        prove_git_surface(&conflict, &conflict_actual)?;

        let reconnect = workbench_git_scene("omega_workbench_git_reconnect")?;
        let reconnect_actual = reconnect
            .active_git_snapshot()
            .context("reconnect fixture has no active Git snapshot")?
            .clone();
        assert_eq!(reconnect_actual.ignored_stale_refresh_count, 1);
        prove_git_surface(&reconnect, &reconnect_actual)?;

        let mut missed_rejection = reconnect_actual;
        missed_rejection.ignored_stale_refresh_count = 0;
        assert!(prove_git_surface(&reconnect, &missed_rejection).is_err());
        Ok(())
    }

    #[test]
    fn git_fixture_round_trip_preserves_typed_pending_operation() -> Result<()> {
        let scene = workbench_git_scene("omega_workbench_git_pending")?;
        let encoded = serde_json::to_vec(&scene)?;
        let decoded: WorkbenchScene = serde_json::from_slice(&encoded)?;
        assert_eq!(decoded, scene);
        decoded.validate()?;
        Ok(())
    }

    #[test]
    fn terminal_scene_catalog_models_shared_panel_and_distinct_creation_bindings() -> Result<()> {
        for name in WORKBENCH_TERMINAL_PIXEL_SCENES {
            let scene = workbench_terminal_scene(name)?;
            assert_eq!(scene.fixture_version, 2);
            assert_eq!(scene.terminal_snapshots.len(), 2);
            assert_eq!(scene.active_surface, Some(WorkSurfaceId::Terminal));

            let active = scene
                .active_terminal_snapshot()
                .context("Terminal fixture has no active snapshot")?;
            let foreign = scene
                .terminal_snapshots
                .iter()
                .find(|snapshot| {
                    snapshot.creation_binding.thread_id != active.creation_binding.thread_id
                })
                .context("Terminal fixture has no foreign snapshot")?;
            assert_eq!(active.panel_entity_id, foreign.panel_entity_id);
            assert_ne!(
                active.creation_binding.thread_id,
                foreign.creation_binding.thread_id
            );
            assert_ne!(
                active.creation_binding.worktree_id,
                foreign.creation_binding.worktree_id
            );
            assert_ne!(
                active.creation_binding.generation,
                foreign.creation_binding.generation
            );
        }
        Ok(())
    }

    #[test]
    fn terminal_fixture_validation_rejects_implicit_spawn_owner_relabel_and_pane_leaks()
    -> Result<()> {
        let scene = workbench_terminal_scene("omega_workbench_terminal_running")?;

        let mut implicit_spawn = scene.clone();
        implicit_spawn.terminal_snapshots[0].implicit_spawn_count = 1;
        assert!(
            implicit_spawn
                .validate()
                .expect_err("implicit Terminal spawn must fail")
                .to_string()
                .contains("must not implicitly spawn")
        );

        let mut relabeled = scene.clone();
        relabeled.terminal_snapshots[0].processes[0]
            .owner
            .generation += 1;
        assert!(
            relabeled
                .validate()
                .expect_err("relabeled Terminal owner must fail")
                .to_string()
                .contains("immutable owner")
        );

        let mut missing_pane_membership = scene.clone();
        missing_pane_membership.terminal_snapshots[0].panes[0]
            .terminal_ids
            .clear();
        missing_pane_membership.terminal_snapshots[0].panes[0].active_terminal_id = None;
        assert!(
            missing_pane_membership
                .validate()
                .expect_err("orphaned Terminal process must fail")
                .to_string()
                .contains("every process exactly once")
        );

        let mut bad_badge = scene;
        bad_badge.terminal_snapshots[0].running_badge_count = 0;
        assert!(
            bad_badge
                .validate()
                .expect_err("incorrect Terminal badge must fail")
                .to_string()
                .contains("live processes")
        );
        Ok(())
    }

    #[test]
    fn terminal_proof_checks_structure_processes_spawn_focus_and_owner_identity() -> Result<()> {
        let scene = workbench_terminal_scene("omega_workbench_terminal_thread_switch")?;
        let actual = scene
            .active_terminal_snapshot()
            .context("Terminal fixture has no active snapshot")?
            .clone();
        let checks = prove_terminal_surface(&scene, &actual)?;
        let names = checks
            .iter()
            .map(|check| check.name.as_str())
            .collect::<BTreeSet<_>>();
        for required in [
            "terminal-creation-binding",
            "terminal-panel-entity",
            "terminal-lifecycle",
            "terminal-pane-layout",
            "terminal-pane-tabs-selection",
            "terminal-process-identities",
            "terminal-immutable-owners",
            "terminal-cwd-identity",
            "terminal-process-lifecycles",
            "terminal-input-bytes",
            "terminal-spawn-results",
            "terminal-running-badge",
            "terminal-badge-agreement",
            "terminal-no-implicit-spawn",
            "terminal-ignored-stale-completion-count",
            "terminal-rejected-foreign-spawn-count",
            "terminal-focus-owner",
            "terminal-no-owner-relabel",
        ] {
            assert!(
                names.contains(required),
                "missing Terminal proof {required}"
            );
        }

        let mut relabeled = actual;
        relabeled.processes[1].owner = relabeled.processes[0].owner.clone();
        assert!(prove_terminal_surface(&scene, &relabeled).is_err());
        Ok(())
    }

    #[test]
    fn terminal_proof_observes_stale_and_foreign_spawn_rejection() -> Result<()> {
        let stale = workbench_terminal_scene("omega_workbench_terminal_stale_spawn")?;
        let stale_actual = stale
            .active_terminal_snapshot()
            .context("stale Terminal fixture has no active snapshot")?
            .clone();
        assert_eq!(stale_actual.ignored_stale_completion_count, 1);
        assert!(matches!(
            stale_actual
                .requested_spawns
                .last()
                .map(|spawn| &spawn.result),
            Some(TerminalSpawnResultFixture::IgnoredStale)
        ));
        prove_terminal_surface(&stale, &stale_actual)?;

        let foreign = workbench_terminal_scene("omega_workbench_terminal_foreign_spawn_rejected")?;
        let foreign_actual = foreign
            .active_terminal_snapshot()
            .context("foreign Terminal fixture has no active snapshot")?
            .clone();
        assert_eq!(foreign_actual.rejected_foreign_spawn_count, 1);
        assert!(matches!(
            foreign_actual
                .requested_spawns
                .last()
                .map(|spawn| &spawn.result),
            Some(TerminalSpawnResultFixture::RejectedForeignBinding)
        ));
        prove_terminal_surface(&foreign, &foreign_actual)?;

        let mut missed_rejection = foreign_actual;
        missed_rejection.rejected_foreign_spawn_count = 0;
        assert!(prove_terminal_surface(&foreign, &missed_rejection).is_err());
        Ok(())
    }

    #[test]
    fn terminal_fixture_round_trip_preserves_split_and_spawn_ownership() -> Result<()> {
        let scene = workbench_terminal_scene("omega_workbench_terminal_split")?;
        let encoded = serde_json::to_vec(&scene)?;
        let decoded: WorkbenchScene = serde_json::from_slice(&encoded)?;
        assert_eq!(decoded, scene);
        assert!(matches!(
            decoded.terminal_snapshots[0].pane_layout,
            TerminalPaneLayoutFixture::Split { .. }
        ));
        decoded.validate()?;
        Ok(())
    }

    #[test]
    fn version_1_fixture_encoding_remains_backward_compatible() {
        let scene = valid_scene();
        let encoded = serde_json::to_value(&scene).expect("encode version 1 scene");
        assert!(encoded.get("thread_workbenches").is_none());
        assert!(encoded.get("git_snapshots").is_none());
        assert!(encoded.get("terminal_snapshots").is_none());

        let decoded: WorkbenchScene =
            serde_json::from_value(encoded).expect("decode version 1 scene without new field");
        assert_eq!(decoded, scene);
        decoded.validate().expect("version 1 scene remains valid");
    }

    #[test]
    fn version_2_fixture_tracks_independent_thread_workbenches() {
        let mut scene = valid_v2_scene();
        scene.threads.push(ThreadFixture {
            id: "thread-b".to_string(),
            project_id: Some("project-a".to_string()),
            repository_id: Some("repo-a".to_string()),
            worktree_id: Some("worktree-b".to_string()),
        });
        let mut thread_b_surfaces = WorkSurfaceId::ALL
            .into_iter()
            .map(|id| SurfaceFixture {
                id,
                available: false,
                badge: None,
            })
            .collect::<Vec<_>>();
        let terminal = thread_b_surfaces
            .iter_mut()
            .find(|surface| surface.id == WorkSurfaceId::Terminal)
            .expect("terminal fixture");
        terminal.available = true;
        terminal.badge = Some(3);
        scene.thread_workbenches.push(ThreadWorkbenchFixture {
            thread_id: "thread-b".to_string(),
            generation: 11,
            binding: Some(WorkbenchBindingFixture {
                repository_id: "repo-a".to_string(),
                worktree_id: "worktree-b".to_string(),
            }),
            requested_surface: Some(WorkSurfaceId::Terminal),
            effective_surface: Some(WorkSurfaceId::Terminal),
            dock_open: false,
            surfaces: thread_b_surfaces,
        });

        scene.validate().expect("valid version 2 scene");
        let active = scene
            .active_thread_workbench()
            .expect("active workbench projection");
        assert_eq!(active.thread_id, "thread-a");
        assert_eq!(active.generation, 7);
        assert_eq!(active.surfaces[0].badge, None);
        let inactive = &scene.thread_workbenches[1];
        assert_eq!(inactive.thread_id, "thread-b");
        assert_eq!(inactive.generation, 11);
        assert_eq!(
            inactive
                .surfaces
                .iter()
                .find(|surface| surface.id == WorkSurfaceId::Terminal)
                .and_then(|surface| surface.badge),
            Some(3)
        );

        let encoded = serde_json::to_vec(&scene).expect("encode version 2 scene");
        let decoded: WorkbenchScene =
            serde_json::from_slice(&encoded).expect("decode version 2 scene");
        assert_eq!(decoded, scene);
    }

    #[test]
    fn version_2_fixture_requires_exactly_one_projection_per_thread() {
        let mut missing = valid_v2_scene();
        missing.thread_workbenches.clear();
        assert!(
            missing
                .validate()
                .expect_err("missing projection must fail")
                .to_string()
                .contains("exactly one workbench fixture per thread")
        );

        let mut duplicate = valid_v2_scene();
        duplicate
            .thread_workbenches
            .push(duplicate.thread_workbenches[0].clone());
        assert!(
            duplicate
                .validate()
                .expect_err("duplicate projection must fail")
                .to_string()
                .contains("duplicate thread workbench")
        );

        let mut foreign = valid_v2_scene();
        foreign.thread_workbenches[0].thread_id = "missing".to_string();
        assert!(
            foreign
                .validate()
                .expect_err("foreign projection must fail")
                .to_string()
                .contains("no workbench fixture for thread")
        );
    }

    #[test]
    fn version_2_fixture_validates_binding_and_visible_projection() {
        let mut binding_mismatch = valid_v2_scene();
        binding_mismatch.thread_workbenches[0]
            .binding
            .as_mut()
            .expect("bound workbench")
            .worktree_id = "worktree-b".to_string();
        assert!(
            binding_mismatch
                .validate()
                .expect_err("binding mismatch must fail")
                .to_string()
                .contains("binding does not match")
        );

        let mut visible_mismatch = valid_v2_scene();
        visible_mismatch.dock_open = false;
        assert!(
            visible_mismatch
                .validate()
                .expect_err("visible dock mismatch must fail")
                .to_string()
                .contains("visible dock state")
        );

        let mut unbound = valid_v2_scene();
        unbound.threads[0].project_id = None;
        unbound.threads[0].repository_id = None;
        unbound.threads[0].worktree_id = None;
        unbound.thread_workbenches[0].binding = None;
        assert!(
            unbound
                .validate()
                .expect_err("unbound repository capability must fail")
                .to_string()
                .contains("repository-bound surface")
        );
    }

    #[test]
    fn version_2_fixture_validates_deterministic_surface_fallback() {
        let mut scene = valid_v2_scene();
        let workbench = &mut scene.thread_workbenches[0];
        workbench.surfaces[0].available = false;
        workbench.surfaces[1].available = true;
        workbench.requested_surface = Some(WorkSurfaceId::Files);
        workbench.effective_surface = Some(WorkSurfaceId::Search);
        scene.surfaces = workbench.surfaces.clone();
        scene.active_surface = workbench.effective_surface;
        scene.validate().expect("deterministic fallback is valid");

        scene.thread_workbenches[0].effective_surface = Some(WorkSurfaceId::Plan);
        assert!(
            scene
                .validate()
                .expect_err("non-deterministic fallback must fail")
                .to_string()
                .contains("deterministic projection")
        );
    }

    #[test]
    fn fixture_digest_is_deterministic_and_state_sensitive() {
        let scene = valid_scene();
        assert_eq!(scene.digest().unwrap(), scene.digest().unwrap());
        let mut changed = scene.clone();
        changed.repositories[0].worktrees[0].dirty_files = 1;
        assert_ne!(scene.digest().unwrap(), changed.digest().unwrap());
    }

    #[test]
    fn fixture_rejects_duplicate_and_foreign_state() {
        let mut duplicate = valid_scene();
        duplicate.threads.push(duplicate.threads[0].clone());
        assert!(
            duplicate
                .validate()
                .unwrap_err()
                .to_string()
                .contains("duplicate thread")
        );

        let mut unavailable = valid_scene();
        unavailable.surfaces[0].available = false;
        assert!(
            unavailable
                .validate()
                .unwrap_err()
                .to_string()
                .contains("unavailable")
        );

        let mut invalid_mutation = valid_scene();
        invalid_mutation.persisted = Some(PersistedSceneFixture {
            requested_surface: Some(WorkSurfaceId::Files),
            dock_open: true,
            revision: 1,
            mutations_before_restart: vec![SceneMutation::SetActiveThread {
                thread_id: "missing".to_string(),
            }],
        });
        assert!(
            invalid_mutation
                .validate()
                .unwrap_err()
                .to_string()
                .contains("missing thread")
        );
    }

    #[test]
    fn full_fixture_covers_async_content_and_restart_mutations() {
        let mut scene = valid_scene();
        scene.messages.push(MessageFixture {
            id: "message-a".to_string(),
            thread_id: "thread-a".to_string(),
            role: MessageRoleFixture::Assistant,
            state: MessageStateFixture::Streaming,
        });
        scene.tool_calls.push(ToolCallFixture {
            id: "tool-a".to_string(),
            thread_id: "thread-a".to_string(),
            state: ToolCallStateFixture::Running,
        });
        scene.plan_steps.push(PlanStepFixture {
            id: "plan-a".to_string(),
            thread_id: "thread-a".to_string(),
            state: PlanStepStateFixture::InProgress,
        });
        scene.artifacts.push(ArtifactFixture {
            id: "artifact-a".to_string(),
            thread_id: "thread-a".to_string(),
            worktree_id: Some("worktree-a".to_string()),
            kind: ArtifactKindFixture::Diff,
        });
        scene.events.push(EventFixture {
            id: "event-a".to_string(),
            thread_id: "thread-a".to_string(),
            revision: 1,
            kind: EventKindFixture::ToolCall,
        });
        scene.connectivity = ConnectivityFixture::Reconnecting;
        scene.content_state = ContentStateFixture::Loading;
        scene.persisted = Some(PersistedSceneFixture {
            requested_surface: Some(WorkSurfaceId::Files),
            dock_open: true,
            revision: 1,
            mutations_before_restart: vec![
                SceneMutation::CompleteMessage {
                    message_id: "message-a".to_string(),
                },
                SceneMutation::CompleteToolCall {
                    tool_call_id: "tool-a".to_string(),
                },
                SceneMutation::SetConnectivity {
                    connectivity: ConnectivityFixture::Online,
                },
                SceneMutation::AdvanceRevision { revision: 2 },
            ],
        });

        scene.validate().unwrap();
        assert_eq!(scene.repositories[0].worktrees.len(), 2);
        assert!(scene.digest().unwrap().starts_with("sha256:"));
    }

    #[test]
    fn catalog_selection_is_deterministic_and_disjoint() {
        let left = select_scenes(None, Some(0), Some(2)).unwrap();
        let right = select_scenes(None, Some(1), Some(2)).unwrap();
        let left_names: BTreeSet<_> = left.iter().map(|scene| scene.name).collect();
        let right_names: BTreeSet<_> = right.iter().map(|scene| scene.name).collect();
        assert!(left_names.is_disjoint(&right_names));
        assert_eq!(
            left_names.union(&right_names).count(),
            HERMETIC_SCENES.len()
        );
        assert_eq!(left, select_scenes(None, Some(0), Some(2)).unwrap());
    }

    #[test]
    fn search_pixel_catalog_covers_required_states_and_region() {
        let expected = BTreeSet::from([
            "omega_workbench_search_empty",
            "omega_workbench_search_populated",
            "omega_workbench_search_no_results",
            "omega_workbench_search_invalid_regex",
            "omega_workbench_search_loading",
            "omega_workbench_search_narrow",
            "omega_workbench_search_focused_result",
            "omega_workbench_search_error",
        ]);
        let registered = HERMETIC_SCENES
            .iter()
            .filter(|scene| scene.name.starts_with("omega_workbench_search_"))
            .map(|scene| scene.name)
            .collect::<BTreeSet<_>>();
        assert_eq!(registered, expected);
        for name in registered {
            let scene = scene_spec(name).expect("registered Search scene");
            assert_eq!(scene.phase, ScenePhase::Recording);
            assert_eq!(scene.regions, WORKBENCH_SEARCH_REGIONS);
            assert!(WORKBENCH_SHELL_PIXEL_SCENES.contains(&name));
        }
    }

    #[test]
    fn review_pixel_catalog_covers_required_states_and_region() {
        let expected = BTreeSet::from([
            "omega_workbench_review_empty",
            "omega_workbench_review_multi_file",
            "omega_workbench_review_selected_hunk",
            "omega_workbench_review_streaming_update",
            "omega_workbench_review_rename_delete",
            "omega_workbench_review_conflict",
            "omega_workbench_review_all_reviewed",
            "omega_workbench_review_narrow",
            "omega_workbench_review_error",
        ]);
        let registered = HERMETIC_SCENES
            .iter()
            .filter(|scene| scene.name.starts_with("omega_workbench_review_"))
            .map(|scene| scene.name)
            .collect::<BTreeSet<_>>();
        assert_eq!(registered, expected);
        for name in registered {
            let scene = scene_spec(name).expect("registered Review scene");
            assert_eq!(scene.phase, ScenePhase::Recording);
            assert_eq!(scene.fixture_version, 2);
            assert_eq!(scene.regions, WORKBENCH_REVIEW_REGIONS);
            assert!(WORKBENCH_SHELL_PIXEL_SCENES.contains(&name));
        }
    }

    #[test]
    fn git_pixel_catalog_covers_required_states_and_region() {
        let expected = BTreeSet::from([
            "omega_workbench_git_clean",
            "omega_workbench_git_dirty",
            "omega_workbench_git_staged",
            "omega_workbench_git_conflict",
            "omega_workbench_git_detached",
            "omega_workbench_git_unborn",
            "omega_workbench_git_pending",
            "omega_workbench_git_multi_repository",
            "omega_workbench_git_repository_removed",
            "omega_workbench_git_offline",
            "omega_workbench_git_reconnect",
            "omega_workbench_git_error",
        ]);
        let registered = HERMETIC_SCENES
            .iter()
            .filter(|scene| scene.name.starts_with("omega_workbench_git_"))
            .map(|scene| scene.name)
            .collect::<BTreeSet<_>>();
        assert_eq!(registered, expected);
        for name in registered {
            let scene = scene_spec(name).expect("registered Git scene");
            assert_eq!(scene.phase, ScenePhase::Recording);
            assert_eq!(scene.fixture_version, 2);
            assert_eq!(scene.regions, WORKBENCH_GIT_REGIONS);
            assert!(WORKBENCH_SHELL_PIXEL_SCENES.contains(&name));
        }
    }

    #[test]
    fn terminal_pixel_catalog_covers_required_states_and_region() {
        let registered = HERMETIC_SCENES
            .iter()
            .filter(|scene| scene.name.starts_with("omega_workbench_terminal_"))
            .map(|scene| scene.name)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            registered,
            WORKBENCH_TERMINAL_PIXEL_SCENES.into_iter().collect()
        );
        for name in registered {
            let scene = scene_spec(name).expect("registered Terminal scene");
            assert_eq!(scene.phase, ScenePhase::Recording);
            assert_eq!(scene.fixture_version, 2);
            let expected_regions = if name == "omega_workbench_terminal_hidden_running" {
                &[][..]
            } else {
                WORKBENCH_TERMINAL_REGIONS
            };
            assert_eq!(scene.regions, expected_regions);
            assert!(WORKBENCH_SHELL_PIXEL_SCENES.contains(&name));
        }
        let narrow = scene_spec("omega_workbench_terminal_narrow")
            .expect("registered narrow Terminal scene");
        assert_eq!(narrow.viewport, ViewportFixture::new(910, 720, 2000));
        assert_eq!(narrow.regions, WORKBENCH_TERMINAL_REGIONS);
    }

    #[test]
    fn catalog_rejects_unknown_and_empty_shards() {
        assert!(select_scenes(Some("missing"), None, None).is_err());
        assert!(
            select_scenes(
                None,
                Some(HERMETIC_SCENES.len()),
                Some(HERMETIC_SCENES.len() + 1)
            )
            .is_err()
        );
        assert!(select_scenes(None, Some(0), None).is_err());
    }

    #[test]
    fn pixel_comparison_honors_tolerance_and_emits_a_diff() {
        let expected = RgbaImage::from_pixel(2, 1, Rgba([10, 10, 10, 255]));
        let tolerated = RgbaImage::from_pixel(2, 1, Rgba([12, 8, 11, 255]));
        let comparison = compare_images(&tolerated, &expected, 2);
        assert_eq!(comparison.match_percentage, 1.0);
        assert_eq!(comparison.different_pixels, 0);

        let mut changed = tolerated;
        changed.put_pixel(1, 0, Rgba([255, 0, 0, 255]));
        let comparison = compare_images(&changed, &expected, 2);
        assert_eq!(comparison.match_percentage, 0.5);
        assert_eq!(comparison.different_pixels, 1);
        assert_eq!(
            *comparison.diff_image.get_pixel(1, 0),
            Rgba([255, 0, 0, 255])
        );
    }

    #[test]
    fn region_capture_rejects_out_of_bounds_rectangles() {
        let image = RgbaImage::new(10, 10);
        let region = CaptureRegion {
            name: "rail".to_string(),
            x: 2,
            y: 3,
            width: 4,
            height: 5,
        };
        assert_eq!(region.crop(&image).unwrap().dimensions(), (4, 5));
        let invalid = CaptureRegion { x: 9, ..region };
        assert!(invalid.crop(&image).is_err());
    }

    #[test]
    fn selector_region_unions_scales_and_pads_logical_bounds() {
        let region = resolve_selector_region(
            "rail-dock",
            [(40.0, 20.0, 40.0, 600.0), (80.0, 20.0, 320.0, 600.0)],
            8,
            ViewportFixture::new(1200, 720, 2000),
            2400,
            1440,
        )
        .expect("selector region should resolve");
        assert_eq!(
            region,
            CaptureRegion {
                name: "rail-dock".to_string(),
                x: 64,
                y: 24,
                width: 752,
                height: 1232,
            }
        );
    }

    #[test]
    fn selector_region_clamps_padding_to_the_captured_frame() {
        let region = resolve_selector_region(
            "rail-dock",
            [(0.0, 0.0, 40.0, 720.0)],
            8,
            ViewportFixture::new(909, 720, 2000),
            1818,
            1440,
        )
        .expect("rail-only region should resolve");
        assert_eq!(
            region,
            CaptureRegion {
                name: "rail-dock".to_string(),
                x: 0,
                y: 0,
                width: 96,
                height: 1440,
            }
        );
    }

    #[test]
    fn selector_region_rejects_missing_or_invalid_bounds() {
        assert!(
            resolve_selector_region(
                "rail-dock",
                [],
                8,
                ViewportFixture::new(1200, 720, 2000),
                2400,
                1440,
            )
            .is_err()
        );
        assert!(
            resolve_selector_region(
                "rail-dock",
                [(0.0, 0.0, 0.0, 10.0)],
                8,
                ViewportFixture::new(1200, 720, 2000),
                2400,
                1440,
            )
            .is_err()
        );
        assert!(
            resolve_selector_region(
                "rail-dock",
                [(f32::MAX, 0.0, f32::MAX, 10.0)],
                8,
                ViewportFixture::new(1200, 720, 2000),
                2400,
                1440,
            )
            .is_err()
        );
    }

    #[test]
    fn receipt_requires_semantics_and_pixel_evidence() {
        let scene = valid_scene();
        let receipt = ProofReceipt::new(&scene, 17, ProofLane::Semantic).unwrap();
        assert!(receipt.validate().is_err());

        let mut receipt = ProofReceipt::new(&scene, 17, ProofLane::Pixel).unwrap();
        receipt
            .semantic_checks
            .push(ProofCheck::passed("typed-state"));
        assert!(receipt.validate().is_err());
        receipt.pixel = Some(PixelProof {
            status: PixelStatus::Passed,
            minimum_match: 0.99,
            channel_tolerance: 2,
            policy_rationale: "test policy".to_string(),
            match_percentage: Some(1.0),
            different_pixels: Some(0),
            total_pixels: Some(1),
            baseline: "baseline.png".into(),
            current: "current.png".into(),
            diff: None,
            regions: Vec::new(),
        });
        assert!(receipt.validate().is_ok());
    }

    #[test]
    fn receipt_encoding_is_deterministic() {
        let scene = valid_scene();
        let mut receipt = ProofReceipt::new(&scene, 17, ProofLane::Semantic).unwrap();
        receipt
            .semantic_checks
            .push(ProofCheck::passed("typed-state"));
        let first = serde_json::to_vec_pretty(&receipt).unwrap();
        let second = serde_json::to_vec_pretty(&receipt).unwrap();
        assert_eq!(first, second);
        assert!(!String::from_utf8(first).unwrap().contains("timestamp"));
    }

    #[test]
    fn semantic_failure_cannot_be_hidden_by_matching_pixels() {
        let scene = valid_scene();
        let mut receipt = ProofReceipt::new(&scene, 17, ProofLane::Pixel).unwrap();
        receipt.semantic_checks.push(ProofCheck::failed(
            "active-surface-owner",
            "rendered surface belongs to the previous thread",
        ));
        receipt.pixel = Some(PixelProof {
            status: PixelStatus::Passed,
            minimum_match: 0.99,
            channel_tolerance: 2,
            policy_rationale: "test policy".to_string(),
            match_percentage: Some(1.0),
            different_pixels: Some(0),
            total_pixels: Some(1),
            baseline: "baseline.png".into(),
            current: "current.png".into(),
            diff: None,
            regions: Vec::new(),
        });
        assert!(receipt.validate().is_err());
        receipt.outcome = ProofOutcome::Failed;
        assert!(receipt.validate().is_ok());
    }

    #[test]
    fn failed_pixels_require_failed_outcome_and_diff_evidence_is_stable() {
        let expected = RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 255]));
        let current = RgbaImage::from_pixel(1, 1, Rgba([255, 255, 255, 255]));
        let comparison = compare_images(&current, &expected, 0);
        assert_eq!(comparison.different_pixels, 1);
        assert_eq!(
            *comparison.diff_image.get_pixel(0, 0),
            Rgba([255, 0, 0, 255])
        );

        let scene = valid_scene();
        let mut receipt = ProofReceipt::new(&scene, 17, ProofLane::Pixel).unwrap();
        receipt
            .semantic_checks
            .push(ProofCheck::passed("typed-state"));
        receipt.pixel = Some(PixelProof {
            status: PixelStatus::Failed,
            minimum_match: 0.99,
            channel_tolerance: 0,
            policy_rationale: "exact synthetic comparison".to_string(),
            match_percentage: Some(comparison.match_percentage),
            different_pixels: Some(comparison.different_pixels),
            total_pixels: Some(comparison.total_pixels),
            baseline: "baseline.png".into(),
            current: "current.png".into(),
            diff: Some("diff.png".into()),
            regions: Vec::new(),
        });
        assert!(receipt.validate().is_err());
        receipt.outcome = ProofOutcome::Failed;
        assert!(receipt.validate().is_ok());

        receipt.pixel.as_mut().unwrap().current = PathBuf::from("../escaped.png");
        assert!(receipt.validate().is_err());
    }
}
