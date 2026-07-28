use std::{cell::Cell, collections::BTreeMap};

use acp_thread::AcpThread;
use anyhow::{Result, anyhow, bail};
use git_ui::git_panel::{GitPanel, GitPanelRepositoryScope};
use gpui::{
    Action, App, Context, Entity, FocusHandle, Focusable, Pixels, Render, SharedString, WeakEntity,
    Window, actions, px,
};
use omega_workbench_state::{
    ConnectionPhase, ProjectionSnapshot, ProjectionTransition, RepositoryBinding, WorkSurface,
    WorkbenchProjection,
};
use project::{Project, WorktreeId, git_store::RepositoryId};
use project_panel::ProjectPanel;
use search::{
    FocusSearch, ReplaceAll, ReplaceNext, SearchOptions, SelectNextMatch, SelectPreviousMatch,
    ToggleCaseSensitive, ToggleIncludeIgnored, ToggleRegex, ToggleReplace, ToggleWholeWord,
    project_search::{
        ProjectSearch, ProjectSearchBar, ProjectSearchView, ToggleFilters, ToggleFocus,
    },
};
use ui::{Color, Icon, IconName, IconSize, Label, LabelSize, prelude::*, v_flex};
use workspace::{Panel, ToolbarItemView, Workspace, item::Item};

use crate::{
    AgentDiff, AgentDiffBinding, AgentDiffLifecycle, AgentDiffPane, AgentDiffToolbar,
    omega_sidebar,
    thread_identity::{
        BranchIdentity, IdentityPhase, ThreadIdentityObservation, ThreadIdentityProjection,
        ThreadIdentityState,
    },
};

pub const ACTIVITY_RAIL_WIDTH: Pixels = px(40.);
pub const DEFAULT_DOCK_WIDTH: Pixels = px(320.);
pub const MIN_DOCK_WIDTH: Pixels = px(240.);
pub const MAX_DOCK_WIDTH: Pixels = px(480.);
pub const RESIZE_HANDLE_WIDTH: Pixels = px(6.);

actions!(
    omega_workbench,
    [
        /// Focus the work-surface activity rail.
        FocusActivityRail,
        /// Select the Files work surface.
        SelectFiles,
        /// Select the Search work surface.
        SelectSearch,
        /// Select the Review work surface.
        SelectReview,
        /// Select the Git work surface.
        SelectGit,
        /// Select the Terminal work surface.
        SelectTerminal,
        /// Select the Plan work surface.
        SelectPlan,
        /// Move focus to the next activity-rail item.
        FocusNextSurface,
        /// Move focus to the previous activity-rail item.
        FocusPreviousSurface,
        /// Move focus to the first activity-rail item.
        FocusFirstSurface,
        /// Move focus to the last activity-rail item.
        FocusLastSurface,
        /// Activate the focused activity-rail item.
        ActivateFocusedSurface,
        /// Collapse the work-surface dock.
        CollapseWorkSurfaceDock,
        /// Return focus to the active thread transcript.
        FocusThreadTranscript,
        /// Open the active thread repository picker.
        ToggleRepositoryPicker,
        /// Open the active thread worktree picker.
        ToggleWorktreePicker,
        /// Open the active thread branch picker.
        ToggleBranchPicker,
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkbenchFocusTarget {
    Transcript,
    Rail(WorkSurface),
    Surface(WorkSurface),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeTone {
    Neutral,
    Accent,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceBadge {
    Count {
        count: usize,
        tone: BadgeTone,
        label: SharedString,
    },
    Attention {
        tone: BadgeTone,
        label: SharedString,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceAvailability {
    Available,
    Unavailable { reason: SharedString },
}

impl SurfaceAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    pub fn reason(&self) -> Option<&SharedString> {
        match self {
            Self::Available => None,
            Self::Unavailable { reason } => Some(reason),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceCapability {
    pub availability: SurfaceAvailability,
    pub badge: Option<SurfaceBadge>,
}

impl SurfaceCapability {
    fn available() -> Self {
        Self {
            availability: SurfaceAvailability::Available,
            badge: None,
        }
    }

    fn unavailable(reason: impl Into<SharedString>) -> Self {
        Self {
            availability: SurfaceAvailability::Unavailable {
                reason: reason.into(),
            },
            badge: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SurfaceHostKey {
    pub thread_id: String,
    pub binding: Option<RepositoryBinding>,
    pub surface: WorkSurface,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceContentState {
    Ready,
    Loading,
    Error(SharedString),
    Offline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeSearchFocusTarget {
    Query,
    Results,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeGitBinding {
    pub thread_id: String,
    pub repository: RepositoryBinding,
    pub worktree_id: WorktreeId,
    pub git_repository_id: RepositoryId,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeGitLifecycle {
    Loading,
    Clean,
    Dirty,
    Conflicted,
    Detached,
    Unborn,
    OperationPending,
    Offline,
    Reconnecting,
    RepositoryRemoved,
    Error(SharedString),
}

impl NativeGitLifecycle {
    fn is_actionable(&self) -> bool {
        matches!(
            self,
            Self::Clean
                | Self::Dirty
                | Self::Conflicted
                | Self::Detached
                | Self::Unborn
                | Self::OperationPending
        )
    }

    fn accessible_label(&self) -> SharedString {
        match self {
            Self::Loading => "Loading Git repository".into(),
            Self::Clean => "Git repository is clean".into(),
            Self::Dirty => "Git repository has changes".into(),
            Self::Conflicted => "Git repository has conflicts".into(),
            Self::Detached => "Git repository has a detached HEAD".into(),
            Self::Unborn => "Git repository has an unborn branch".into(),
            Self::OperationPending => "Git operation in progress".into(),
            Self::Offline => "Git repository is unavailable offline".into(),
            Self::Reconnecting => "Reconnecting Git repository".into(),
            Self::RepositoryRemoved => "Git repository was removed".into(),
            Self::Error(error) => error.clone(),
        }
    }
}

pub struct NativeGitSurface {
    focus_handle: FocusHandle,
    git_panel: Entity<GitPanel>,
    binding: Option<NativeGitBinding>,
    lifecycle: NativeGitLifecycle,
}

impl NativeGitSurface {
    pub fn new(git_panel: Entity<GitPanel>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            git_panel,
            binding: None,
            lifecycle: NativeGitLifecycle::Loading,
        }
    }

    pub fn bind(
        &mut self,
        binding: NativeGitBinding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if self.binding.as_ref() == Some(&binding) {
            return Ok(());
        }
        self.git_panel.update(cx, |git_panel, cx| {
            git_panel.set_repository_scope(
                Some(GitPanelRepositoryScope {
                    repository_id: binding.git_repository_id,
                    worktree_id: binding.worktree_id,
                    generation: binding.generation,
                }),
                window,
                cx,
            )
        })?;
        self.binding = Some(binding);
        self.lifecycle = NativeGitLifecycle::Loading;
        cx.notify();
        Ok(())
    }

    pub fn set_lifecycle(
        &mut self,
        generation: u64,
        git_repository_id: RepositoryId,
        lifecycle: NativeGitLifecycle,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(binding) = self.binding.as_ref() else {
            return false;
        };
        if binding.generation != generation || binding.git_repository_id != git_repository_id {
            return false;
        }
        if self.lifecycle == lifecycle {
            return true;
        }
        self.lifecycle = lifecycle;
        cx.notify();
        true
    }

    pub fn git_panel(&self) -> &Entity<GitPanel> {
        &self.git_panel
    }

    pub fn binding(&self) -> Option<&NativeGitBinding> {
        self.binding.as_ref()
    }

    pub fn lifecycle(&self) -> &NativeGitLifecycle {
        &self.lifecycle
    }

    pub fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.contains_focused(window, cx)
            || self.git_panel.focus_handle(cx).contains_focused(window, cx)
    }
}

impl Focusable for NativeGitSurface {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.lifecycle.is_actionable() {
            self.git_panel.read(cx).activation_focus_handle(cx)
        } else {
            self.focus_handle.clone()
        }
    }
}

impl Render for NativeGitSurface {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let lifecycle = self.lifecycle.clone();
        let lifecycle_label = lifecycle.accessible_label();
        v_flex()
            .id("omega-native-git-surface")
            .debug_selector(|| "omega.workbench.git.content".to_string())
            .role(gpui::Role::Group)
            .aria_label("Git changes")
            .track_focus(&self.focus_handle)
            .size_full()
            .when(lifecycle.is_actionable(), |this| {
                this.child(self.git_panel.clone())
            })
            .when(!lifecycle.is_actionable(), |this| {
                let role = if matches!(lifecycle, NativeGitLifecycle::Error(_)) {
                    gpui::Role::Alert
                } else {
                    gpui::Role::Status
                };
                this.child(
                    v_flex()
                        .id("omega.workbench.git.lifecycle")
                        .debug_selector(|| "omega.workbench.git.lifecycle".to_string())
                        .role(role)
                        .aria_label(lifecycle_label.clone())
                        .size_full()
                        .items_center()
                        .justify_center()
                        .child(
                            Label::new(lifecycle_label)
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        ),
                )
            })
    }
}

pub struct NativeReviewSurface {
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    agent_diff: Entity<AgentDiff>,
    diff_pane: Entity<AgentDiffPane>,
    diff_toolbar: Entity<AgentDiffToolbar>,
}

impl NativeReviewSurface {
    pub fn new(
        workspace: Entity<Workspace>,
        thread: Entity<AcpThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak_workspace = workspace.downgrade();
        AgentDiff::set_active_thread(&weak_workspace, thread.clone(), window, cx);
        let agent_diff = AgentDiff::global(cx);
        let diff_pane = cx.new(|cx| AgentDiffPane::new(thread, weak_workspace.clone(), window, cx));
        workspace.update(cx, |workspace, cx| {
            diff_pane.update(cx, |diff_pane, cx| {
                diff_pane.added_to_workspace(workspace, window, cx);
            });
        });
        let diff_toolbar = cx.new(AgentDiffToolbar::new);
        diff_toolbar.update(cx, |diff_toolbar, cx| {
            diff_toolbar.set_agent_diff_pane(&diff_pane, cx);
        });

        Self {
            workspace: weak_workspace,
            focus_handle: cx.focus_handle(),
            agent_diff,
            diff_pane,
            diff_toolbar,
        }
    }

    pub fn bind(
        &mut self,
        binding: AgentDiffBinding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if self.diff_pane.read(cx).binding_snapshot().as_ref() == Some(&binding) {
            return Ok(());
        }
        let generation = binding.checkpoint.generation();
        self.diff_pane.update(cx, |diff_pane, cx| {
            diff_pane.bind(binding, window, cx)?;
            diff_pane.complete_load(generation, window, cx);
            Ok(())
        })
    }

    pub fn set_offline(&mut self, generation: u64, cx: &mut Context<Self>) -> bool {
        self.diff_pane
            .update(cx, |diff_pane, cx| diff_pane.set_offline(generation, cx))
    }

    pub fn set_online(
        &mut self,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.diff_pane.update(cx, |diff_pane, cx| {
            diff_pane.set_online(generation, window, cx)
        })
    }

    pub fn set_checkpoint_unavailable(
        &mut self,
        generation: u64,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.diff_pane.update(cx, |diff_pane, cx| {
            diff_pane.set_checkpoint_unavailable(generation, message, cx)
        })
    }

    pub fn invalidate(
        &mut self,
        generation: u64,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.diff_pane.update(cx, |diff_pane, cx| {
            diff_pane.invalidate(generation, message, cx)
        })
    }

    pub fn agent_diff(&self) -> &Entity<AgentDiff> {
        &self.agent_diff
    }

    pub fn diff_pane(&self) -> &Entity<AgentDiffPane> {
        &self.diff_pane
    }

    pub fn diff_toolbar(&self) -> &Entity<AgentDiffToolbar> {
        &self.diff_toolbar
    }

    pub fn binding(&self, cx: &App) -> Option<AgentDiffBinding> {
        self.diff_pane.read(cx).binding_snapshot()
    }

    pub fn lifecycle(&self, cx: &App) -> AgentDiffLifecycle {
        self.diff_pane.read(cx).lifecycle()
    }

    pub fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.contains_focused(window, cx)
    }

    fn keep(&mut self, action: &crate::Keep, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            cx.propagate();
            return;
        }
        self.diff_pane
            .update(cx, |diff_pane, cx| diff_pane.keep(action, window, cx));
    }

    fn reject(&mut self, action: &crate::Reject, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            cx.propagate();
            return;
        }
        self.diff_pane
            .update(cx, |diff_pane, cx| diff_pane.reject(action, window, cx));
    }

    fn keep_all(&mut self, action: &crate::KeepAll, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            cx.propagate();
            return;
        }
        self.diff_pane
            .update(cx, |diff_pane, cx| diff_pane.keep_all(action, window, cx));
    }

    fn reject_all(
        &mut self,
        action: &crate::RejectAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_handle.is_focused(window) {
            cx.propagate();
            return;
        }
        self.diff_pane
            .update(cx, |diff_pane, cx| diff_pane.reject_all(action, window, cx));
    }

    fn reveal_center_for_open_excerpts(
        &mut self,
        _: &editor::actions::OpenExcerpts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                workspace.reveal_zero_base_center(window, cx);
            });
        }
        cx.propagate();
    }

    fn reveal_center_for_open_excerpts_split(
        &mut self,
        _: &editor::actions::OpenExcerptsSplit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                workspace.reveal_zero_base_center(window, cx);
            });
        }
        cx.propagate();
    }
}

impl Focusable for NativeReviewSurface {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for NativeReviewSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("omega-native-review-surface")
            .track_focus(&self.focus_handle)
            .size_full()
            .on_action(cx.listener(Self::keep))
            .on_action(cx.listener(Self::reject))
            .on_action(cx.listener(Self::keep_all))
            .on_action(cx.listener(Self::reject_all))
            .capture_action(cx.listener(Self::reveal_center_for_open_excerpts))
            .capture_action(cx.listener(Self::reveal_center_for_open_excerpts_split))
            .child(
                v_flex()
                    .id("omega.workbench.review.toolbar")
                    .debug_selector(|| "omega.workbench.review.toolbar".to_string())
                    .role(gpui::Role::Toolbar)
                    .aria_label("Review controls")
                    .flex_none()
                    .w_full()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .p_2()
                    .child(self.diff_toolbar.clone()),
            )
            .child(
                v_flex()
                    .id("omega.workbench.review.content")
                    .debug_selector(|| "omega.workbench.review.content".to_string())
                    .role(gpui::Role::Group)
                    .aria_label("Review changes")
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.diff_pane.clone()),
            )
    }
}

pub struct NativeSearchSurface {
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    project_search: Entity<ProjectSearch>,
    search_view: Entity<ProjectSearchView>,
    search_bar: Entity<ProjectSearchBar>,
}

impl NativeSearchSurface {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let project_search = cx.new(|cx| ProjectSearch::new(project, cx));
        let search_view = cx.new(|cx| {
            ProjectSearchView::new(workspace.clone(), project_search.clone(), window, cx, None)
        });
        if let Some(workspace_entity) = workspace.upgrade() {
            workspace_entity.update(cx, |workspace, cx| {
                search_view.update(cx, |search_view, cx| {
                    search_view.added_to_workspace(workspace, window, cx);
                });
            });
        }
        let search_bar = cx.new(|_| ProjectSearchBar::new());
        search_bar.update(cx, |search_bar, cx| {
            search_bar.set_compact_mode(true, cx);
            search_bar.set_active_pane_item(Some(&search_view), window, cx);
        });

        Self {
            workspace,
            focus_handle: cx.focus_handle(),
            project_search,
            search_view,
            search_bar,
        }
    }

    pub fn project_search(&self) -> &Entity<ProjectSearch> {
        &self.project_search
    }

    pub fn search_view(&self) -> &Entity<ProjectSearchView> {
        &self.search_view
    }

    pub fn search_bar(&self) -> &Entity<ProjectSearchBar> {
        &self.search_bar
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn focus_target(&self, window: &Window, cx: &App) -> Option<NativeSearchFocusTarget> {
        let query_focus = self
            .search_view
            .read(cx)
            .query_editor_for_tests()
            .focus_handle(cx);
        let results_focus = self
            .search_view
            .read(cx)
            .results_editor_for_tests()
            .focus_handle(cx);
        if query_focus.is_focused(window) || query_focus.contains_focused(window, cx) {
            Some(NativeSearchFocusTarget::Query)
        } else if results_focus.is_focused(window) || results_focus.contains_focused(window, cx) {
            Some(NativeSearchFocusTarget::Results)
        } else {
            None
        }
    }

    pub fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.contains_focused(window, cx)
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search_bar.update(cx, |search_bar, cx| {
            search_bar.focus_search(window, cx);
        });
        cx.stop_propagation();
    }

    fn toggle_filters(&mut self, _: &ToggleFilters, window: &mut Window, cx: &mut Context<Self>) {
        self.search_bar.update(cx, |search_bar, cx| {
            search_bar.toggle_filters(window, cx);
        });
        cx.stop_propagation();
    }

    fn toggle_search_option(
        &mut self,
        search_options: SearchOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_bar.update(cx, |search_bar, cx| {
            search_bar.toggle_search_option(search_options, window, cx);
        });
        cx.stop_propagation();
    }

    fn toggle_regex(&mut self, _: &ToggleRegex, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_search_option(SearchOptions::REGEX, window, cx);
    }

    fn toggle_case_sensitive(
        &mut self,
        _: &ToggleCaseSensitive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_search_option(SearchOptions::CASE_SENSITIVE, window, cx);
    }

    fn toggle_whole_word(
        &mut self,
        _: &ToggleWholeWord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_search_option(SearchOptions::WHOLE_WORD, window, cx);
    }

    fn toggle_include_ignored(
        &mut self,
        _: &ToggleIncludeIgnored,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_search_option(SearchOptions::INCLUDE_IGNORED, window, cx);
    }

    fn move_focus_to_results(
        &mut self,
        _: &ToggleFocus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_bar.update(cx, |search_bar, cx| {
            search_bar.move_focus_to_results(window, cx);
        });
        cx.stop_propagation();
    }

    fn toggle_replace(
        &mut self,
        action: &ToggleReplace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_bar.update(cx, |search_bar, cx| {
            search_bar.toggle_replace(action, window, cx);
        });
        cx.stop_propagation();
    }

    fn replace_next(&mut self, action: &ReplaceNext, window: &mut Window, cx: &mut Context<Self>) {
        self.search_bar.update(cx, |search_bar, cx| {
            search_bar.replace_next(action, window, cx);
        });
        cx.stop_propagation();
    }

    fn replace_all(&mut self, action: &ReplaceAll, window: &mut Window, cx: &mut Context<Self>) {
        self.search_bar.update(cx, |search_bar, cx| {
            search_bar.replace_all(action, window, cx);
        });
        cx.stop_propagation();
    }

    fn select_next_match(
        &mut self,
        action: &SelectNextMatch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_bar.update(cx, |search_bar, cx| {
            search_bar.select_next_match(action, window, cx);
        });
        cx.stop_propagation();
    }

    fn select_previous_match(
        &mut self,
        action: &SelectPreviousMatch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_bar.update(cx, |search_bar, cx| {
            search_bar.select_prev_match(action, window, cx);
        });
        cx.stop_propagation();
    }

    fn reveal_center_for_open_excerpts(
        &mut self,
        _: &editor::actions::OpenExcerpts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                workspace.reveal_zero_base_center(window, cx);
            });
        }
        cx.propagate();
    }

    fn reveal_center_for_open_excerpts_split(
        &mut self,
        _: &editor::actions::OpenExcerptsSplit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                workspace.reveal_zero_base_center(window, cx);
            });
        }
        cx.propagate();
    }
}

impl Focusable for NativeSearchSurface {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search_view.focus_handle(cx)
    }
}

impl Render for NativeSearchSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("omega-native-search-surface")
            .track_focus(&self.focus_handle)
            .size_full()
            .capture_action(cx.listener(Self::reveal_center_for_open_excerpts))
            .capture_action(cx.listener(Self::reveal_center_for_open_excerpts_split))
            .capture_action(cx.listener(Self::focus_search))
            .capture_action(cx.listener(Self::toggle_filters))
            .capture_action(cx.listener(Self::toggle_regex))
            .capture_action(cx.listener(Self::toggle_case_sensitive))
            .capture_action(cx.listener(Self::toggle_whole_word))
            .capture_action(cx.listener(Self::toggle_include_ignored))
            .capture_action(cx.listener(Self::move_focus_to_results))
            .capture_action(cx.listener(Self::toggle_replace))
            .capture_action(cx.listener(Self::replace_next))
            .capture_action(cx.listener(Self::replace_all))
            .capture_action(cx.listener(Self::select_next_match))
            .capture_action(cx.listener(Self::select_previous_match))
            .child(
                v_flex()
                    .id("omega.workbench.search.toolbar")
                    .debug_selector(|| "omega.workbench.search.toolbar".to_string())
                    .role(gpui::Role::Toolbar)
                    .aria_label("Search controls")
                    .flex_none()
                    .w_full()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .p_2()
                    .child(self.search_bar.clone()),
            )
            .child(
                v_flex()
                    .id("omega.workbench.search.content")
                    .debug_selector(|| "omega.workbench.search.content".to_string())
                    .role(gpui::Role::Group)
                    .aria_label("Search results")
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.search_view.clone()),
            )
    }
}

pub struct WorkSurfaceHost {
    key: SurfaceHostKey,
    focus_handle: FocusHandle,
    content_state: SurfaceContentState,
    files_panel: Option<Entity<ProjectPanel>>,
    search_surface: Option<Entity<NativeSearchSurface>>,
    review_surface: Option<Entity<NativeReviewSurface>>,
    git_surface: Option<Entity<NativeGitSurface>>,
}

impl WorkSurfaceHost {
    fn new(
        key: SurfaceHostKey,
        files_panel: Option<Entity<ProjectPanel>>,
        search_surface: Option<Entity<NativeSearchSurface>>,
        review_surface: Option<Entity<NativeReviewSurface>>,
        git_surface: Option<Entity<NativeGitSurface>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            key,
            focus_handle: cx.focus_handle(),
            content_state: SurfaceContentState::Ready,
            files_panel,
            search_surface,
            review_surface,
            git_surface,
        }
    }

    pub fn key(&self) -> &SurfaceHostKey {
        &self.key
    }

    pub fn content_state(&self) -> &SurfaceContentState {
        &self.content_state
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn files_panel_entity_id(&self) -> Option<gpui::EntityId> {
        self.files_panel.as_ref().map(Entity::entity_id)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn search_surface(&self) -> Option<&Entity<NativeSearchSurface>> {
        self.search_surface.as_ref()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn review_surface(&self) -> Option<&Entity<NativeReviewSurface>> {
        self.review_surface.as_ref()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn git_surface(&self) -> Option<&Entity<NativeGitSurface>> {
        self.git_surface.as_ref()
    }

    fn native_content_contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.files_panel
            .as_ref()
            .is_some_and(|panel| panel.focus_handle(cx).contains_focused(window, cx))
            || self
                .search_surface
                .as_ref()
                .is_some_and(|surface| surface.read(cx).contains_focus(window, cx))
            || self
                .review_surface
                .as_ref()
                .is_some_and(|surface| surface.read(cx).contains_focus(window, cx))
            || self
                .git_surface
                .as_ref()
                .is_some_and(|surface| surface.read(cx).contains_focus(window, cx))
    }

    fn focus_native_content(&self, window: &mut Window, cx: &mut App) {
        if let Some(panel) = self.files_panel.as_ref() {
            panel.focus_handle(cx).focus(window, cx);
        } else if let Some(surface) = self.search_surface.as_ref() {
            surface.focus_handle(cx).focus(window, cx);
        } else if let Some(surface) = self.review_surface.as_ref() {
            surface.focus_handle(cx).focus(window, cx);
        } else if let Some(surface) = self.git_surface.as_ref() {
            surface.focus_handle(cx).focus(window, cx);
        }
    }

    fn set_content_state(&mut self, content_state: SurfaceContentState, cx: &mut Context<Self>) {
        if self.content_state == content_state {
            return;
        }
        self.content_state = content_state;
        cx.notify();
    }

    fn set_content_state_with_focus(
        &mut self,
        content_state: SurfaceContentState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_ready = matches!(self.content_state, SurfaceContentState::Ready);
        let is_ready = matches!(content_state, SurfaceContentState::Ready);
        if was_ready && !is_ready && self.native_content_contains_focus(window, cx) {
            self.focus_handle.focus(window, cx);
        } else if !was_ready && is_ready && self.focus_handle.contains_focused(window, cx) {
            self.focus_native_content(window, cx);
        }
        self.set_content_state(content_state, cx);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceLoadContext {
    request_id: String,
    thread_id: String,
    surface: WorkSurface,
    generation: u64,
    binding: Option<RepositoryBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceLoadOutcome {
    Ready,
    Error(SharedString),
}

impl Focusable for WorkSurfaceHost {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if matches!(self.content_state, SurfaceContentState::Ready) {
            self.files_panel
                .as_ref()
                .map(|panel| panel.focus_handle(cx))
                .or_else(|| {
                    self.search_surface
                        .as_ref()
                        .map(|surface| surface.focus_handle(cx))
                })
                .or_else(|| {
                    self.review_surface
                        .as_ref()
                        .map(|surface| surface.focus_handle(cx))
                })
                .or_else(|| {
                    self.git_surface
                        .as_ref()
                        .map(|surface| surface.focus_handle(cx))
                })
                .unwrap_or_else(|| self.focus_handle.clone())
        } else {
            self.focus_handle.clone()
        }
    }
}

impl Render for WorkSurfaceHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let surface = self.key.surface;
        let label = surface.label();
        let files_panel = if matches!(self.content_state, SurfaceContentState::Ready) {
            self.files_panel.clone()
        } else {
            None
        };
        let search_surface = if matches!(self.content_state, SurfaceContentState::Ready) {
            self.search_surface.clone()
        } else {
            None
        };
        let review_surface = if matches!(self.content_state, SurfaceContentState::Ready) {
            self.review_surface.clone()
        } else {
            None
        };
        let git_surface = if matches!(self.content_state, SurfaceContentState::Ready) {
            self.git_surface.clone()
        } else {
            None
        };
        let status = match &self.content_state {
            SurfaceContentState::Ready
                if files_panel.is_some()
                    || search_surface.is_some()
                    || review_surface.is_some()
                    || git_surface.is_some() =>
            {
                None
            }
            SurfaceContentState::Ready => {
                let message: SharedString = format!("{label} is ready").into();
                Some((
                    Label::new(message.clone())
                        .size(LabelSize::Small)
                        .color(Color::Muted)
                        .into_any_element(),
                    gpui::Role::Status,
                    message,
                ))
            }
            SurfaceContentState::Loading => {
                let message: SharedString = format!("Loading {label}…").into();
                Some((
                    Label::new(message.clone())
                        .size(LabelSize::Small)
                        .color(Color::Muted)
                        .into_any_element(),
                    gpui::Role::Status,
                    message,
                ))
            }
            SurfaceContentState::Error(error) => Some((
                Label::new(error.clone())
                    .size(LabelSize::Small)
                    .color(Color::Error)
                    .into_any_element(),
                gpui::Role::Alert,
                error.clone(),
            )),
            SurfaceContentState::Offline => {
                let message: SharedString = format!("{label} is unavailable offline").into();
                Some((
                    Label::new(message.clone())
                        .size(LabelSize::Small)
                        .color(Color::Warning)
                        .into_any_element(),
                    gpui::Role::Status,
                    message,
                ))
            }
        };

        v_flex()
            .id(surface.surface_element_id())
            .debug_selector(move || surface.surface_selector())
            .track_focus(&self.focus_handle)
            .tab_index(0)
            .role(gpui::Role::Group)
            .aria_label(format!("{label} work surface"))
            .size_full()
            .when_some(files_panel, |this, panel| this.child(panel))
            .when_some(search_surface, |this, surface| this.child(surface))
            .when_some(review_surface, |this, surface| this.child(surface))
            .when_some(git_surface, |this, surface| this.child(surface))
            .when_some(status, |this, (status, role, message)| {
                this.child(
                    v_flex()
                        .size_full()
                        .id(format!("{}.status", surface.surface_selector()))
                        .debug_selector(move || format!("{}.status", surface.surface_selector()))
                        .role(role)
                        .aria_label(message)
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(
                            Icon::new(surface.icon())
                                .size(IconSize::Medium)
                                .color(Color::Muted),
                        )
                        .child(status),
                )
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkbenchLayout {
    pub sidebar: omega_sidebar::Layout,
    pub dock_visible: bool,
    pub dock_width: Pixels,
}

impl WorkbenchLayout {
    pub fn clamp_dock_width(available: Pixels, requested_dock_width: Pixels) -> Option<Pixels> {
        let maximum_dock_width = (available
            - omega_sidebar::RAIL_WIDTH
            - ACTIVITY_RAIL_WIDTH
            - omega_sidebar::MIN_CONTENT_WIDTH)
            .min(MAX_DOCK_WIDTH);
        if maximum_dock_width < MIN_DOCK_WIDTH {
            return None;
        }

        Some(requested_dock_width.clamp(MIN_DOCK_WIDTH, maximum_dock_width))
    }

    pub fn allocate(
        available: Pixels,
        sidebar_requested_open: bool,
        dock_requested_open: bool,
        requested_dock_width: Pixels,
    ) -> Self {
        let dock_width = if dock_requested_open {
            Self::clamp_dock_width(available, requested_dock_width)
        } else {
            None
        };
        let dock_visible = dock_width.is_some();
        let dock_width = dock_width.unwrap_or(Pixels::ZERO);

        let sidebar = if sidebar_requested_open
            && available - ACTIVITY_RAIL_WIDTH - dock_width - omega_sidebar::SIDEBAR_WIDTH
                >= omega_sidebar::MIN_CONTENT_WIDTH
        {
            omega_sidebar::Layout::Expanded
        } else {
            omega_sidebar::Layout::Rail
        };

        Self {
            sidebar,
            dock_visible,
            dock_width,
        }
    }
}

#[derive(Debug)]
pub(crate) struct WorkbenchDockResizeDrag {
    width_before: Pixels,
    pointer_x_before: Cell<Pixels>,
}

impl WorkbenchDockResizeDrag {
    pub(crate) fn new(width_before: Pixels) -> Self {
        Self {
            width_before,
            pointer_x_before: Cell::new(Pixels::ZERO),
        }
    }

    pub(crate) fn begin(&self, pointer_x: Pixels) {
        self.pointer_x_before.set(pointer_x);
    }

    pub(crate) fn requested_width(&self, pointer_x: Pixels) -> Pixels {
        self.width_before + pointer_x - self.pointer_x_before.get()
    }
}

pub struct WorkbenchShell {
    projection: WorkbenchProjection,
    identity: ThreadIdentityProjection,
    capabilities: BTreeMap<WorkSurface, SurfaceCapability>,
    hosts: BTreeMap<SurfaceHostKey, Entity<WorkSurfaceHost>>,
    rail_focus_handles: BTreeMap<WorkSurface, FocusHandle>,
    focused_rail_surface: WorkSurface,
    focus_target: WorkbenchFocusTarget,
    dock_width: Pixels,
    last_error: Option<SharedString>,
    files_identity_error_visible: bool,
    #[cfg(any(test, feature = "test-support"))]
    fail_next_host_creation: Option<WorkSurface>,
}

impl WorkbenchShell {
    pub fn new(cx: &mut Context<crate::AgentPanel>) -> Self {
        let rail_focus_handles = WorkSurface::FALLBACK_ORDER
            .into_iter()
            .map(|surface| (surface, cx.focus_handle()))
            .collect();
        Self {
            projection: WorkbenchProjection::new(),
            identity: ThreadIdentityProjection::default(),
            capabilities: WorkSurface::FALLBACK_ORDER
                .into_iter()
                .map(|surface| {
                    (
                        surface,
                        SurfaceCapability::unavailable("Open a thread to use this surface"),
                    )
                })
                .collect(),
            hosts: BTreeMap::new(),
            rail_focus_handles,
            focused_rail_surface: WorkSurface::Files,
            focus_target: WorkbenchFocusTarget::Transcript,
            dock_width: DEFAULT_DOCK_WIDTH,
            last_error: None,
            files_identity_error_visible: false,
            #[cfg(any(test, feature = "test-support"))]
            fail_next_host_creation: None,
        }
    }

    pub fn projection(&self) -> &WorkbenchProjection {
        &self.projection
    }

    pub fn identity(&self) -> Option<&ThreadIdentityState> {
        self.identity.active()
    }

    pub fn identity_thread_id(&self) -> Option<&str> {
        self.identity.active_thread_id()
    }

    pub fn capabilities(&self) -> &BTreeMap<WorkSurface, SurfaceCapability> {
        &self.capabilities
    }

    pub fn capability(&self, surface: WorkSurface) -> Option<&SurfaceCapability> {
        self.capabilities.get(&surface)
    }

    pub fn focus_target(&self) -> WorkbenchFocusTarget {
        self.focus_target
    }

    pub fn focused_rail_surface(&self) -> WorkSurface {
        self.focused_rail_surface
    }

    pub fn rail_focus_handle(&self, surface: WorkSurface) -> Option<&FocusHandle> {
        self.rail_focus_handles.get(&surface)
    }

    pub fn last_error(&self) -> Option<&SharedString> {
        self.last_error.as_ref()
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    pub fn record_error(&mut self, error: impl Into<SharedString>) {
        self.last_error = Some(error.into());
    }

    pub fn dock_width(&self) -> Pixels {
        self.dock_width
    }

    pub fn resize_dock(&mut self, width: Pixels, available: Pixels) -> bool {
        let Some(width) = WorkbenchLayout::clamp_dock_width(available, width) else {
            return false;
        };
        if width == self.dock_width {
            return false;
        }

        self.dock_width = width;
        true
    }

    pub fn sync_active_thread(
        &mut self,
        thread_id: Option<String>,
        observation: ThreadIdentityObservation,
    ) -> Result<()> {
        let connection = match observation.phase {
            IdentityPhase::Offline => ConnectionPhase::Offline,
            IdentityPhase::Reconnecting => ConnectionPhase::Reconnecting,
            IdentityPhase::Stale => ConnectionPhase::StaleProjection,
            _ => ConnectionPhase::Online,
        };
        self.set_connection(connection)?;
        self.identity
            .sync_active_thread(thread_id.clone(), observation);
        let Some(thread_id) = thread_id else {
            self.capabilities = unavailable_capabilities("Open a thread to use this surface");
            self.focus_target = WorkbenchFocusTarget::Transcript;
            return Ok(());
        };

        let identity = self
            .identity
            .active()
            .ok_or_else(|| anyhow!("active thread identity disappeared during synchronization"))?;
        let binding = identity.binding().cloned();
        let available_surfaces = available_surfaces_for_identity(identity);
        if !self.projection.threads.contains_key(&thread_id) {
            self.projection
                .apply(ProjectionTransition::OpenThread {
                    thread_id: thread_id.clone(),
                    binding,
                    available_surfaces,
                })
                .map_err(anyhow::Error::new)?;
        } else {
            self.reconcile_binding(&thread_id, binding, available_surfaces)?;
        }
        if self.projection.active_thread_id.as_deref() != Some(thread_id.as_str()) {
            self.projection
                .apply(ProjectionTransition::SwitchThread {
                    thread_id: thread_id.clone(),
                })
                .map_err(anyhow::Error::new)?;
        }

        let thread =
            self.projection.threads.get(&thread_id).ok_or_else(|| {
                anyhow!("thread {thread_id:?} disappeared during capability sync")
            })?;
        let available_surfaces = thread.available_surfaces.clone();
        let mut capabilities = capabilities_for_surfaces(&available_surfaces);
        for (surface, capability) in &mut capabilities {
            if !available_surfaces.contains(surface) {
                capability.availability = SurfaceAvailability::Unavailable {
                    reason: "This surface is no longer available".into(),
                };
            }
            capability.badge = self
                .capabilities
                .get(surface)
                .and_then(|previous| previous.badge.clone());
        }
        if let Some(identity) = self.identity.active() {
            let phase_reason = match identity.phase {
                IdentityPhase::NoProject => Some("Open a project to use this surface"),
                IdentityPhase::Loading => Some("Wait for repository identity to finish loading"),
                IdentityPhase::Missing => Some("The selected worktree is missing"),
                IdentityPhase::Inconsistent(_) => {
                    Some("Reconnect this thread before using repository-bound surfaces")
                }
                _ => None,
            };
            if let Some(reason) = phase_reason {
                for (surface, capability) in &mut capabilities {
                    if *surface != WorkSurface::Plan {
                        capability.availability = SurfaceAvailability::Unavailable {
                            reason: reason.into(),
                        };
                    }
                }
            }
        }
        if let Some(identity) = self.identity.active()
            && matches!(
                identity.phase,
                IdentityPhase::Offline | IdentityPhase::Reconnecting | IdentityPhase::Stale
            )
        {
            let reason: SharedString = identity
                .phase
                .label()
                .unwrap_or_else(|| "Repository identity is unavailable".into());
            for (surface, capability) in &mut capabilities {
                if *surface != WorkSurface::Plan {
                    capability.availability = SurfaceAvailability::Unavailable {
                        reason: reason.clone(),
                    };
                }
            }
        }
        if let Some(identity) = self.identity.active()
            && let Some(selected) = &identity.selected
        {
            let git = selected.git;
            let label: SharedString = format!(
                "{} changed, {} conflicted, {} ahead, {} behind",
                git.dirty_files, git.conflicts, git.ahead, git.behind
            )
            .into();
            let badge = if git.conflicts > 0 {
                Some(SurfaceBadge::Attention {
                    tone: BadgeTone::Error,
                    label,
                })
            } else if git.dirty_files > 0 || git.ahead > 0 || git.behind > 0 {
                Some(SurfaceBadge::Count {
                    count: if git.dirty_files > 0 {
                        git.dirty_files
                    } else {
                        git.ahead.saturating_add(git.behind)
                    },
                    tone: BadgeTone::Warning,
                    label,
                })
            } else {
                None
            };
            if let Some(capability) = capabilities.get_mut(&WorkSurface::Git) {
                capability.badge = badge;
            }
        }
        self.capabilities = capabilities;
        let identity_is_inconsistent = self
            .identity
            .active()
            .is_some_and(|identity| matches!(identity.phase, IdentityPhase::Inconsistent(_)));
        if (self.projection.connection != ConnectionPhase::Online || identity_is_inconsistent)
            && self.projection.visible_projection().is_some_and(|visible| {
                visible.dock_open
                    && visible
                        .effective_surface
                        .is_some_and(|surface| surface != omega_workbench_state::WorkSurface::Plan)
            })
        {
            self.projection
                .apply(ProjectionTransition::CollapseDock {
                    thread_id: thread_id.clone(),
                })
                .map_err(anyhow::Error::new)?;
        }
        if let Some(visible) = self.projection.visible_projection() {
            self.focus_target = if visible.dock_open {
                visible
                    .effective_surface
                    .map(WorkbenchFocusTarget::Surface)
                    .unwrap_or(WorkbenchFocusTarget::Transcript)
            } else {
                WorkbenchFocusTarget::Transcript
            };
        }
        Ok(())
    }

    pub fn select_identity(
        &mut self,
        expected_observation_revision: u64,
        binding: &RepositoryBinding,
    ) -> Result<bool> {
        let identity = self
            .identity
            .active()
            .ok_or_else(|| anyhow!("identity selection has no active thread"))?;
        if identity.observation_revision != expected_observation_revision {
            bail!(
                "identity observation changed from revision {expected_observation_revision} to {}",
                identity.observation_revision
            );
        }
        let candidate = identity
            .candidates
            .iter()
            .find(|candidate| &candidate.binding == binding)
            .cloned()
            .ok_or_else(|| anyhow!("selected repository/worktree is unavailable"))?;
        if identity.selected.as_ref() == Some(&candidate) {
            return Ok(false);
        }
        let thread_id = self
            .identity
            .active_thread_id()
            .ok_or_else(|| anyhow!("identity selection has no active thread"))?
            .to_string();
        let available_surfaces = if candidate.branch == BranchIdentity::NoGit {
            vec![
                WorkSurface::Files,
                WorkSurface::Search,
                WorkSurface::Terminal,
                WorkSurface::Plan,
            ]
        } else {
            WorkSurface::FALLBACK_ORDER.into()
        };
        self.reconcile_binding(&thread_id, Some(candidate.binding), available_surfaces)?;
        let changed = self
            .identity
            .select(expected_observation_revision, binding)?;
        debug_assert!(changed);
        Ok(true)
    }

    pub fn refresh_binding_epoch(
        &mut self,
        expected_thread_id: &str,
        expected_binding: &RepositoryBinding,
        expected_generation: u64,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<()> {
        let thread = self
            .projection
            .threads
            .get(expected_thread_id)
            .ok_or_else(|| anyhow!("thread {expected_thread_id:?} is unavailable"))?
            .clone();
        if thread.generation != expected_generation
            || thread.binding.as_ref() != Some(expected_binding)
        {
            bail!("repository identity changed before its content epoch could be refreshed");
        }
        self.projection
            .apply(ProjectionTransition::ChangeBinding {
                thread_id: expected_thread_id.into(),
                generation: expected_generation,
                binding: thread.binding,
                available_surfaces: thread.available_surfaces.into_iter().collect(),
            })
            .map_err(anyhow::Error::new)?;
        for (key, host) in &self.hosts {
            if key.thread_id == expected_thread_id && key.binding.as_ref() == Some(expected_binding)
            {
                host.update(cx, |host, cx| {
                    host.set_content_state(SurfaceContentState::Ready, cx);
                });
            }
        }
        Ok(())
    }

    pub fn close_thread(&mut self, thread_id: &str) -> Result<()> {
        if self.projection.threads.contains_key(thread_id) {
            self.projection
                .apply(ProjectionTransition::CloseThread {
                    thread_id: thread_id.into(),
                })
                .map_err(anyhow::Error::new)?;
        }
        self.hosts.retain(|key, _| key.thread_id != thread_id);
        if self.projection.active_thread_id.is_none() {
            self.focus_target = WorkbenchFocusTarget::Transcript;
        }
        Ok(())
    }

    fn reconcile_binding(
        &mut self,
        thread_id: &str,
        binding: Option<RepositoryBinding>,
        available_surfaces: Vec<WorkSurface>,
    ) -> Result<()> {
        let thread = self
            .projection
            .threads
            .get(thread_id)
            .ok_or_else(|| anyhow!("thread {thread_id:?} disappeared during reconciliation"))?
            .clone();
        if thread.binding == binding {
            return Ok(());
        }

        let previous_effective = thread.effective_surface;
        self.projection
            .apply(ProjectionTransition::ChangeBinding {
                thread_id: thread_id.into(),
                generation: thread.generation,
                binding,
                available_surfaces,
            })
            .map_err(anyhow::Error::new)?;

        let current = self
            .projection
            .threads
            .get(thread_id)
            .cloned()
            .ok_or_else(|| anyhow!("thread {thread_id:?} disappeared after reconciliation"))?;
        if previous_effective != current.effective_surface && current.dock_open {
            self.projection
                .apply(ProjectionTransition::CollapseDock {
                    thread_id: thread_id.into(),
                })
                .map_err(anyhow::Error::new)?;
            self.focus_target = WorkbenchFocusTarget::Transcript;
        }
        let current_binding = current.binding;
        self.hosts
            .retain(|key, _| key.thread_id != thread_id || key.binding == current_binding);
        Ok(())
    }

    pub fn set_connection(&mut self, phase: ConnectionPhase) -> Result<()> {
        if self.projection.connection == phase {
            return Ok(());
        }

        if matches!(
            phase,
            ConnectionPhase::Offline
                | ConnectionPhase::Reconnecting
                | ConnectionPhase::StaleProjection
        ) && matches!(
            self.projection.connection,
            ConnectionPhase::Reconnecting | ConnectionPhase::StaleProjection
        ) {
            self.receive_current_projection_snapshot(true)?;
        }
        if matches!(
            phase,
            ConnectionPhase::Offline
                | ConnectionPhase::Reconnecting
                | ConnectionPhase::StaleProjection
        ) && self.projection.connection == ConnectionPhase::Online
        {
            self.projection
                .apply(ProjectionTransition::Disconnect)
                .map_err(anyhow::Error::new)?;
        }
        if matches!(
            phase,
            ConnectionPhase::Reconnecting | ConnectionPhase::StaleProjection
        ) && self.projection.connection == ConnectionPhase::Offline
        {
            self.projection
                .apply(ProjectionTransition::Reconnect)
                .map_err(anyhow::Error::new)?;
        }
        match phase {
            ConnectionPhase::Online => {
                if self.projection.connection == ConnectionPhase::Offline {
                    self.projection
                        .apply(ProjectionTransition::Reconnect)
                        .map_err(anyhow::Error::new)?;
                }
                if matches!(
                    self.projection.connection,
                    ConnectionPhase::Reconnecting | ConnectionPhase::StaleProjection
                ) {
                    self.receive_current_projection_snapshot(true)?;
                }
            }
            ConnectionPhase::StaleProjection => {
                self.receive_current_projection_snapshot(false)?;
            }
            ConnectionPhase::Offline | ConnectionPhase::Reconnecting => {}
        }
        if phase != ConnectionPhase::Online {
            self.focus_target = WorkbenchFocusTarget::Transcript;
        }
        Ok(())
    }

    fn receive_current_projection_snapshot(&mut self, advance_revision: bool) -> Result<()> {
        let revision = if advance_revision {
            self.projection.projection_revision.saturating_add(1)
        } else {
            self.projection.projection_revision
        };
        let snapshot = ProjectionSnapshot {
            revision,
            persistence_revision: self.projection.persistence_revision,
            active_thread_id: self.projection.active_thread_id.clone(),
            threads: self.projection.threads.clone(),
            persisted_selection: self.projection.persisted_selection.clone(),
        };
        self.projection
            .apply(ProjectionTransition::ReceiveProjectionSnapshot { snapshot })
            .map_err(anyhow::Error::new)?;
        Ok(())
    }

    pub fn select_surface(
        &mut self,
        surface: WorkSurface,
        files_panel: Option<Entity<ProjectPanel>>,
        search_surface: Option<Entity<NativeSearchSurface>>,
        review_surface: Option<Entity<NativeReviewSurface>>,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<SurfaceSelection> {
        self.select_surface_with_native(
            surface,
            files_panel,
            search_surface,
            review_surface,
            None,
            cx,
        )
    }

    pub fn select_git_surface(
        &mut self,
        git_surface: Entity<NativeGitSurface>,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<SurfaceSelection> {
        self.select_surface_with_native(WorkSurface::Git, None, None, None, Some(git_surface), cx)
    }

    fn select_surface_with_native(
        &mut self,
        surface: WorkSurface,
        files_panel: Option<Entity<ProjectPanel>>,
        search_surface: Option<Entity<NativeSearchSurface>>,
        review_surface: Option<Entity<NativeReviewSurface>>,
        git_surface: Option<Entity<NativeGitSurface>>,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<SurfaceSelection> {
        let unavailable_reason = self
            .capability(surface)
            .ok_or_else(|| anyhow!("the {} surface is not registered", surface.label()))?
            .availability
            .reason()
            .cloned();
        if let Some(reason) = unavailable_reason {
            self.last_error = Some(reason.clone());
            bail!("{reason}");
        }
        let visible = self
            .projection
            .visible_projection()
            .ok_or_else(|| anyhow!("open a thread before selecting a work surface"))?;
        let thread_id = visible.thread_id.clone();

        if visible.dock_open && visible.effective_surface == Some(surface) {
            self.projection
                .apply(ProjectionTransition::CollapseDock { thread_id })
                .map_err(anyhow::Error::new)?;
            self.focus_target = WorkbenchFocusTarget::Transcript;
            self.last_error = None;
            return Ok(SurfaceSelection::Collapsed);
        }

        let previous_projection = self.projection.clone();
        self.projection
            .apply(ProjectionTransition::RequestSurface { thread_id, surface })
            .map_err(anyhow::Error::new)?;
        let host = match self.ensure_host(
            &visible.thread_id,
            visible.binding.clone(),
            surface,
            files_panel,
            search_surface,
            review_surface,
            git_surface,
            cx,
        ) {
            Ok(host) => host,
            Err(error) => {
                self.projection = previous_projection;
                return Err(error);
            }
        };
        self.focused_rail_surface = surface;
        self.focus_target = WorkbenchFocusTarget::Surface(surface);
        self.last_error = None;
        Ok(SurfaceSelection::Opened(host))
    }

    fn ensure_host(
        &mut self,
        thread_id: &str,
        binding: Option<RepositoryBinding>,
        surface: WorkSurface,
        files_panel: Option<Entity<ProjectPanel>>,
        search_surface: Option<Entity<NativeSearchSurface>>,
        review_surface: Option<Entity<NativeReviewSurface>>,
        git_surface: Option<Entity<NativeGitSurface>>,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<Entity<WorkSurfaceHost>> {
        if surface == WorkSurface::Files && files_panel.is_none() {
            bail!("the native Files surface is unavailable");
        }
        if surface == WorkSurface::Search && search_surface.is_none() {
            bail!("the native Search surface is unavailable");
        }
        if surface == WorkSurface::Review && review_surface.is_none() {
            bail!("the native Review surface is unavailable");
        }
        if surface == WorkSurface::Git && git_surface.is_none() {
            bail!("the native Git surface is unavailable");
        }
        let key = SurfaceHostKey {
            thread_id: thread_id.into(),
            binding,
            surface,
        };
        if let Some(host) = self.hosts.get(&key) {
            return Ok(host.clone());
        }
        #[cfg(any(test, feature = "test-support"))]
        if self.fail_next_host_creation == Some(surface) {
            self.fail_next_host_creation = None;
            let message: SharedString =
                format!("Could not create the {} surface", surface.label()).into();
            self.last_error = Some(message.clone());
            bail!("{message}");
        }
        let host = cx.new(|cx| {
            WorkSurfaceHost::new(
                key.clone(),
                files_panel,
                search_surface,
                review_surface,
                git_surface,
                cx,
            )
        });
        self.hosts.insert(key, host.clone());
        Ok(host)
    }

    pub fn ensure_visible_files_host(
        &mut self,
        files_panel: Entity<ProjectPanel>,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<Option<Entity<WorkSurfaceHost>>> {
        let Some(visible) = self.projection.visible_projection() else {
            return Ok(None);
        };
        if !visible.dock_open || visible.effective_surface != Some(WorkSurface::Files) {
            return Ok(None);
        }
        let thread_id = visible.thread_id.clone();
        let binding = visible.binding;
        self.ensure_host(
            &thread_id,
            binding,
            WorkSurface::Files,
            Some(files_panel),
            None,
            None,
            None,
            cx,
        )
        .map(Some)
    }

    pub fn search_surface_for_active_binding(
        &self,
        cx: &App,
    ) -> Option<Entity<NativeSearchSurface>> {
        let visible = self.projection.visible_projection()?;
        let key = SurfaceHostKey {
            thread_id: visible.thread_id,
            binding: visible.binding,
            surface: WorkSurface::Search,
        };
        self.hosts
            .get(&key)?
            .read(cx)
            .search_surface
            .as_ref()
            .cloned()
    }

    pub fn ensure_visible_search_host(
        &mut self,
        search_surface: Entity<NativeSearchSurface>,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<Option<Entity<WorkSurfaceHost>>> {
        let Some(visible) = self.projection.visible_projection() else {
            return Ok(None);
        };
        if !visible.dock_open || visible.effective_surface != Some(WorkSurface::Search) {
            return Ok(None);
        }
        let thread_id = visible.thread_id.clone();
        let binding = visible.binding;
        self.ensure_host(
            &thread_id,
            binding,
            WorkSurface::Search,
            None,
            Some(search_surface),
            None,
            None,
            cx,
        )
        .map(Some)
    }

    pub fn review_surface_for_active_binding(
        &self,
        cx: &App,
    ) -> Option<Entity<NativeReviewSurface>> {
        let visible = self.projection.visible_projection()?;
        let key = SurfaceHostKey {
            thread_id: visible.thread_id,
            binding: visible.binding,
            surface: WorkSurface::Review,
        };
        self.hosts
            .get(&key)?
            .read(cx)
            .review_surface
            .as_ref()
            .cloned()
    }

    pub fn ensure_visible_review_host(
        &mut self,
        review_surface: Entity<NativeReviewSurface>,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<Option<Entity<WorkSurfaceHost>>> {
        let Some(visible) = self.projection.visible_projection() else {
            return Ok(None);
        };
        if !visible.dock_open || visible.effective_surface != Some(WorkSurface::Review) {
            return Ok(None);
        }
        let thread_id = visible.thread_id.clone();
        let binding = visible.binding;
        self.ensure_host(
            &thread_id,
            binding,
            WorkSurface::Review,
            None,
            None,
            Some(review_surface),
            None,
            cx,
        )
        .map(Some)
    }

    pub fn git_surface_for_active_binding(&self, cx: &App) -> Option<Entity<NativeGitSurface>> {
        let visible = self.projection.visible_projection()?;
        let key = SurfaceHostKey {
            thread_id: visible.thread_id,
            binding: visible.binding,
            surface: WorkSurface::Git,
        };
        self.hosts.get(&key)?.read(cx).git_surface.as_ref().cloned()
    }

    pub fn ensure_visible_git_host(
        &mut self,
        git_surface: Entity<NativeGitSurface>,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<Option<Entity<WorkSurfaceHost>>> {
        let Some(visible) = self.projection.visible_projection() else {
            return Ok(None);
        };
        if !visible.dock_open || visible.effective_surface != Some(WorkSurface::Git) {
            return Ok(None);
        }
        let thread_id = visible.thread_id.clone();
        let binding = visible.binding;
        self.ensure_host(
            &thread_id,
            binding,
            WorkSurface::Git,
            None,
            None,
            None,
            Some(git_surface),
            cx,
        )
        .map(Some)
    }

    pub fn set_active_git_content_state(
        &mut self,
        content_state: SurfaceContentState,
        window: &mut Window,
        cx: &mut Context<crate::AgentPanel>,
    ) -> bool {
        let Some(visible) = self.projection.visible_projection() else {
            return false;
        };
        let key = SurfaceHostKey {
            thread_id: visible.thread_id,
            binding: visible.binding,
            surface: WorkSurface::Git,
        };
        let Some(host) = self.hosts.get(&key) else {
            return false;
        };
        host.update(cx, |host, cx| {
            host.set_content_state_with_focus(content_state, window, cx);
        });
        true
    }

    pub fn set_git_scope_lifecycle(
        &mut self,
        scope: GitPanelRepositoryScope,
        lifecycle: NativeGitLifecycle,
        cx: &mut Context<crate::AgentPanel>,
    ) -> usize {
        let git_surfaces = self
            .hosts
            .values()
            .filter_map(|host| host.read(cx).git_surface.as_ref().cloned())
            .collect::<Vec<_>>();
        let mut updated = 0;
        for git_surface in git_surfaces {
            let binding = git_surface.read(cx).binding().cloned();
            if let Some(binding) = binding
                && binding.git_repository_id == scope.repository_id
                && binding.worktree_id == scope.worktree_id
                && binding.generation == scope.generation
                && git_surface.update(cx, |git_surface, cx| {
                    git_surface.set_lifecycle(
                        binding.generation,
                        binding.git_repository_id,
                        lifecycle.clone(),
                        cx,
                    )
                })
            {
                updated += 1;
            }
        }
        updated
    }

    pub fn set_active_review_content_state(
        &mut self,
        content_state: SurfaceContentState,
        window: &mut Window,
        cx: &mut Context<crate::AgentPanel>,
    ) -> bool {
        let Some(visible) = self.projection.visible_projection() else {
            return false;
        };
        let key = SurfaceHostKey {
            thread_id: visible.thread_id,
            binding: visible.binding,
            surface: WorkSurface::Review,
        };
        let Some(host) = self.hosts.get(&key) else {
            return false;
        };
        host.update(cx, |host, cx| {
            host.set_content_state_with_focus(content_state, window, cx);
        });
        true
    }

    pub fn set_visible_files_identity_error(
        &mut self,
        error: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<crate::AgentPanel>,
    ) -> bool {
        if error.is_none() && !self.files_identity_error_visible {
            return false;
        }
        let Some(visible) = self.projection.visible_projection() else {
            return false;
        };
        if !visible.dock_open || visible.effective_surface != Some(WorkSurface::Files) {
            return false;
        }
        let key = SurfaceHostKey {
            thread_id: visible.thread_id,
            binding: visible.binding,
            surface: WorkSurface::Files,
        };
        let Some(host) = self.hosts.get(&key) else {
            return false;
        };
        let has_error = error.is_some();
        let content_state = error.map_or(SurfaceContentState::Ready, SurfaceContentState::Error);
        host.update(cx, |host, cx| {
            host.set_content_state_with_focus(content_state, window, cx);
        });
        self.files_identity_error_visible = has_error;
        true
    }

    pub fn begin_surface_load(
        &mut self,
        request_id: impl Into<String>,
        surface: WorkSurface,
        window: &mut Window,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<SurfaceLoadContext> {
        let request_id = request_id.into();
        let visible = self
            .projection
            .visible_projection()
            .ok_or_else(|| anyhow!("open a thread before loading a work surface"))?;
        if !visible.dock_open || visible.effective_surface != Some(surface) {
            bail!("the {} surface is not visible", surface.label());
        }
        let key = SurfaceHostKey {
            thread_id: visible.thread_id.clone(),
            binding: visible.binding.clone(),
            surface,
        };
        let host = self
            .hosts
            .get(&key)
            .cloned()
            .ok_or_else(|| anyhow!("the {} host is not mounted", surface.label()))?;
        let context = SurfaceLoadContext {
            request_id,
            thread_id: visible.thread_id,
            surface,
            generation: visible.generation,
            binding: visible.binding,
        };
        self.projection
            .apply(ProjectionTransition::BeginSurfaceLoad {
                request_id: context.request_id.clone(),
                thread_id: context.thread_id.clone(),
                surface: context.surface,
                generation: context.generation,
                binding: context.binding.clone(),
            })
            .map_err(anyhow::Error::new)?;
        host.update(cx, |host, cx| {
            host.set_content_state_with_focus(SurfaceContentState::Loading, window, cx);
        });
        Ok(context)
    }

    pub fn complete_surface_load(
        &mut self,
        context: SurfaceLoadContext,
        outcome: SurfaceLoadOutcome,
        window: &mut Window,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<omega_workbench_state::TransitionEffect> {
        let transition = match &outcome {
            SurfaceLoadOutcome::Ready => ProjectionTransition::CompleteSurfaceLoad {
                request_id: context.request_id.clone(),
                thread_id: context.thread_id.clone(),
                surface: context.surface,
                generation: context.generation,
                binding: context.binding.clone(),
            },
            SurfaceLoadOutcome::Error(_) => ProjectionTransition::FailSurfaceLoad {
                request_id: context.request_id.clone(),
                thread_id: context.thread_id.clone(),
                surface: context.surface,
                generation: context.generation,
                binding: context.binding.clone(),
            },
        };
        let effect = self
            .projection
            .apply(transition)
            .map_err(anyhow::Error::new)?;
        if effect == omega_workbench_state::TransitionEffect::Applied {
            let key = SurfaceHostKey {
                thread_id: context.thread_id,
                binding: context.binding,
                surface: context.surface,
            };
            if let Some(host) = self.hosts.get(&key) {
                let content_state = match outcome {
                    SurfaceLoadOutcome::Ready => SurfaceContentState::Ready,
                    SurfaceLoadOutcome::Error(error) => SurfaceContentState::Error(error),
                };
                host.update(cx, |host, cx| {
                    host.set_content_state_with_focus(content_state, window, cx);
                });
            }
        }
        Ok(effect)
    }

    pub fn visible_host(&self) -> Option<&Entity<WorkSurfaceHost>> {
        let visible = self.projection.visible_projection()?;
        if !visible.dock_open {
            return None;
        }
        let key = SurfaceHostKey {
            thread_id: visible.thread_id,
            binding: visible.binding,
            surface: visible.effective_surface?,
        };
        self.hosts.get(&key)
    }

    pub fn active_surface_content_state(
        &self,
        surface: WorkSurface,
        cx: &App,
    ) -> Option<SurfaceContentState> {
        let visible = self.projection.visible_projection()?;
        let key = SurfaceHostKey {
            thread_id: visible.thread_id,
            binding: visible.binding,
            surface,
        };
        self.hosts
            .get(&key)
            .map(|host| host.read(cx).content_state().clone())
    }

    pub fn collapse_dock(&mut self) -> Result<bool> {
        let Some(visible) = self.projection.visible_projection() else {
            return Ok(false);
        };
        if !visible.dock_open {
            return Ok(false);
        }
        self.projection
            .apply(ProjectionTransition::CollapseDock {
                thread_id: visible.thread_id,
            })
            .map_err(anyhow::Error::new)?;
        self.focus_target = WorkbenchFocusTarget::Transcript;
        Ok(true)
    }

    pub fn collapse_for_layout(&mut self, layout: WorkbenchLayout) -> Result<bool> {
        if layout.dock_visible {
            return Ok(false);
        }
        self.collapse_dock()
    }

    pub fn focus_rail(&mut self) -> WorkSurface {
        let surface = self
            .projection
            .visible_projection()
            .and_then(|visible| visible.effective_surface)
            .unwrap_or(self.focused_rail_surface);
        self.focused_rail_surface = surface;
        self.focus_target = WorkbenchFocusTarget::Rail(surface);
        surface
    }

    pub fn move_rail_focus(&mut self, movement: RailFocusMovement) -> WorkSurface {
        let current_index = WorkSurface::FALLBACK_ORDER
            .iter()
            .position(|surface| *surface == self.focused_rail_surface)
            .unwrap_or(0);
        let last_index = WorkSurface::FALLBACK_ORDER.len() - 1;
        let next_index = match movement {
            RailFocusMovement::Next => (current_index + 1).min(last_index),
            RailFocusMovement::Previous => current_index.saturating_sub(1),
            RailFocusMovement::First => 0,
            RailFocusMovement::Last => last_index,
        };
        let surface = WorkSurface::FALLBACK_ORDER[next_index];
        self.focused_rail_surface = surface;
        self.focus_target = WorkbenchFocusTarget::Rail(surface);
        surface
    }

    pub fn return_to_transcript(&mut self) {
        self.focus_target = WorkbenchFocusTarget::Transcript;
    }

    pub fn host_count(&self) -> usize {
        self.hosts.len()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn fail_next_host_creation(&mut self, surface: WorkSurface) {
        self.fail_next_host_creation = Some(surface);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_badge(&mut self, surface: WorkSurface, badge: Option<SurfaceBadge>) {
        if let Some(capability) = self.capabilities.get_mut(&surface) {
            capability.badge = badge;
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn invalidate_surface(
        &mut self,
        surface: WorkSurface,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<omega_workbench_state::TransitionEffect> {
        let visible = self
            .projection
            .visible_projection()
            .ok_or_else(|| anyhow!("no active thread"))?;
        let effect = self
            .projection
            .apply(ProjectionTransition::InvalidateCapability {
                thread_id: visible.thread_id.clone(),
                generation: visible.generation,
                surface,
            })
            .map_err(anyhow::Error::new)?;
        self.capabilities.insert(
            surface,
            SurfaceCapability::unavailable("This surface is no longer available"),
        );
        let invalidated_host = self.hosts.remove(&SurfaceHostKey {
            thread_id: visible.thread_id.clone(),
            binding: visible.binding.clone(),
            surface,
        });
        if surface == WorkSurface::Review
            && let Some(review_surface) = invalidated_host
                .as_ref()
                .and_then(|host| host.read(cx).review_surface.clone())
            && let Some(binding) = review_surface.read(cx).binding(cx)
        {
            review_surface.update(cx, |review_surface, cx| {
                review_surface.invalidate(
                    binding.checkpoint.generation(),
                    "The Review capability was invalidated",
                    cx,
                );
            });
        }
        if visible.effective_surface == Some(surface) {
            self.collapse_dock()?;
            self.focus_target = WorkbenchFocusTarget::Transcript;
        }
        Ok(effect)
    }
}

#[derive(Clone)]
pub enum SurfaceSelection {
    Collapsed,
    Opened(Entity<WorkSurfaceHost>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailFocusMovement {
    Next,
    Previous,
    First,
    Last,
}

pub trait WorkSurfaceExt {
    fn label(self) -> &'static str;
    fn icon(self) -> IconName;
    fn rail_element_id(self) -> &'static str;
    fn surface_element_id(self) -> &'static str;
    fn surface_selector(self) -> String;
}

impl WorkSurfaceExt for WorkSurface {
    fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Search => "Search",
            Self::Review => "Review",
            Self::Git => "Git",
            Self::Terminal => "Terminal",
            Self::Plan => "Plan",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Files => IconName::FileTree,
            Self::Search => IconName::MagnifyingGlass,
            Self::Review => IconName::ListTodo,
            Self::Git => IconName::GitBranch,
            Self::Terminal => IconName::TerminalAlt,
            Self::Plan => IconName::TodoProgress,
        }
    }

    fn rail_element_id(self) -> &'static str {
        match self {
            Self::Files => "omega.workbench.control.rail.files",
            Self::Search => "omega.workbench.control.rail.search",
            Self::Review => "omega.workbench.control.rail.review",
            Self::Git => "omega.workbench.control.rail.git",
            Self::Terminal => "omega.workbench.control.rail.terminal",
            Self::Plan => "omega.workbench.control.rail.plan",
        }
    }

    fn surface_element_id(self) -> &'static str {
        match self {
            Self::Files => "omega.workbench.surface.files",
            Self::Search => "omega.workbench.surface.search",
            Self::Review => "omega.workbench.surface.review",
            Self::Git => "omega.workbench.surface.git",
            Self::Terminal => "omega.workbench.surface.terminal",
            Self::Plan => "omega.workbench.surface.plan",
        }
    }

    fn surface_selector(self) -> String {
        self.surface_element_id().into()
    }
}

pub fn select_action(surface: WorkSurface) -> Box<dyn Action> {
    match surface {
        WorkSurface::Files => SelectFiles.boxed_clone(),
        WorkSurface::Search => SelectSearch.boxed_clone(),
        WorkSurface::Review => SelectReview.boxed_clone(),
        WorkSurface::Git => SelectGit.boxed_clone(),
        WorkSurface::Terminal => SelectTerminal.boxed_clone(),
        WorkSurface::Plan => SelectPlan.boxed_clone(),
    }
}

fn available_surfaces_for_identity(identity: &ThreadIdentityState) -> Vec<WorkSurface> {
    if identity.binding().is_none() {
        return vec![WorkSurface::Plan];
    }
    let Some(selected) = identity.selected.as_ref() else {
        return vec![WorkSurface::Plan];
    };
    if selected.branch == BranchIdentity::NoGit {
        vec![
            WorkSurface::Files,
            WorkSurface::Search,
            WorkSurface::Terminal,
            WorkSurface::Plan,
        ]
    } else {
        WorkSurface::FALLBACK_ORDER.into()
    }
}

fn capabilities_for_surfaces(
    available_surfaces: &std::collections::BTreeSet<WorkSurface>,
) -> BTreeMap<WorkSurface, SurfaceCapability> {
    WorkSurface::FALLBACK_ORDER
        .into_iter()
        .map(|surface| {
            let capability = if available_surfaces.contains(&surface) {
                SurfaceCapability::available()
            } else {
                SurfaceCapability::unavailable(
                    if matches!(surface, WorkSurface::Git | WorkSurface::Review) {
                        "This worktree has no Git repository"
                    } else {
                        "Open a project to use this surface"
                    },
                )
            };
            (surface, capability)
        })
        .collect()
}

fn unavailable_capabilities(reason: &'static str) -> BTreeMap<WorkSurface, SurfaceCapability> {
    WorkSurface::FALLBACK_ORDER
        .into_iter()
        .map(|surface| (surface, SurfaceCapability::unavailable(reason)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_has_one_shared_allocation_boundary() {
        let boundary = omega_sidebar::RAIL_WIDTH
            + ACTIVITY_RAIL_WIDTH
            + MIN_DOCK_WIDTH
            + omega_sidebar::MIN_CONTENT_WIDTH;
        let below = WorkbenchLayout::allocate(boundary - px(1.), true, true, DEFAULT_DOCK_WIDTH);
        assert!(!below.dock_visible);

        let exact = WorkbenchLayout::allocate(boundary, true, true, DEFAULT_DOCK_WIDTH);
        assert!(exact.dock_visible);
        assert_eq!(exact.dock_width, MIN_DOCK_WIDTH);
        assert_eq!(exact.sidebar, omega_sidebar::Layout::Rail);

        let above = WorkbenchLayout::allocate(boundary + px(1.), true, true, DEFAULT_DOCK_WIDTH);
        assert!(above.dock_visible);
        assert_eq!(above.dock_width, MIN_DOCK_WIDTH + px(1.));
        assert_eq!(above.sidebar, omega_sidebar::Layout::Rail);

        let sidebar_boundary = omega_sidebar::SIDEBAR_WIDTH
            + ACTIVITY_RAIL_WIDTH
            + DEFAULT_DOCK_WIDTH
            + omega_sidebar::MIN_CONTENT_WIDTH;
        let both = WorkbenchLayout::allocate(sidebar_boundary, true, true, DEFAULT_DOCK_WIDTH);
        assert!(both.dock_visible);
        assert_eq!(both.sidebar, omega_sidebar::Layout::Expanded);
    }

    #[test]
    fn dock_width_is_clamped_without_stealing_transcript_floor() {
        let available = px(1050.);
        let layout = WorkbenchLayout::allocate(available, false, true, MAX_DOCK_WIDTH);
        assert!(layout.dock_visible);
        assert_eq!(
            layout.dock_width,
            available
                - omega_sidebar::RAIL_WIDTH
                - ACTIVITY_RAIL_WIDTH
                - omega_sidebar::MIN_CONTENT_WIDTH
        );
    }

    #[test]
    fn dock_resize_clamp_preserves_limits_and_transcript_floor() {
        let transcript_limited_width = px(300.);
        let transcript_limited_available = omega_sidebar::RAIL_WIDTH
            + ACTIVITY_RAIL_WIDTH
            + omega_sidebar::MIN_CONTENT_WIDTH
            + transcript_limited_width;
        assert_eq!(
            WorkbenchLayout::clamp_dock_width(transcript_limited_available, MAX_DOCK_WIDTH),
            Some(transcript_limited_width)
        );

        let roomy_available = omega_sidebar::RAIL_WIDTH
            + ACTIVITY_RAIL_WIDTH
            + omega_sidebar::MIN_CONTENT_WIDTH
            + MAX_DOCK_WIDTH
            + px(100.);
        assert_eq!(
            WorkbenchLayout::clamp_dock_width(roomy_available, px(100.)),
            Some(MIN_DOCK_WIDTH)
        );
        assert_eq!(
            WorkbenchLayout::clamp_dock_width(roomy_available, px(600.)),
            Some(MAX_DOCK_WIDTH)
        );

        let too_narrow = omega_sidebar::RAIL_WIDTH
            + ACTIVITY_RAIL_WIDTH
            + omega_sidebar::MIN_CONTENT_WIDTH
            + MIN_DOCK_WIDTH
            - px(1.);
        assert_eq!(
            WorkbenchLayout::clamp_dock_width(too_narrow, DEFAULT_DOCK_WIDTH),
            None
        );
    }

    #[test]
    fn dock_resize_drag_uses_the_width_and_pointer_at_drag_start() {
        let drag = WorkbenchDockResizeDrag::new(px(320.));
        drag.begin(px(800.));

        assert_eq!(drag.requested_width(px(800.)), px(320.));
        assert_eq!(drag.requested_width(px(920.)), px(440.));
        assert_eq!(drag.requested_width(px(700.)), px(220.));
    }
}
