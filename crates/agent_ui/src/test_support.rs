use acp_thread::{AgentConnection, StubAgentConnection};
use agent_client_protocol::schema::v1 as acp;
use agent_servers::{AgentServer, AgentServerDelegate};
use anyhow::{Context as _, Result, bail};
use gpui::{
    Action, AnyWindowHandle, App, AppContext as _, Context, DebugRenderSnapshot, Entity, EntityId,
    EventEmitter, FocusHandle, Focusable, IntoElement, Pixels, Render, SharedString, Task,
    TestAppContext, VisualTestContext, Window, div, px, size,
};
use omega_workbench_harness::{
    ConnectivityFixture, ContentStateFixture, ProofCheck, SemanticProbe, ThemeFixture,
    WorkSurfaceId, WorkbenchScene,
};
use project::{AgentId, Project};
use settings::SettingsStore;
use std::any::Any;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use workspace::{
    MultiWorkspace, Sidebar as WorkspaceSidebar, SidebarEvent, SidebarSide, Workspace,
    dock::Panel as _,
};

use crate::AgentPanel;
use crate::agent_panel;

thread_local! {
    static STUB_AGENT_CONNECTION: RefCell<Option<StubAgentConnection>> = const { RefCell::new(None) };
}

/// Registers a `StubAgentConnection` that will be used by `Agent::Stub`.
///
/// Returns the same connection so callers can hold onto it and control
/// the stub's behavior (e.g. `connection.set_next_prompt_updates(...)`).
pub fn set_stub_agent_connection(connection: StubAgentConnection) -> StubAgentConnection {
    STUB_AGENT_CONNECTION.with(|cell| {
        *cell.borrow_mut() = Some(connection.clone());
    });
    connection
}

/// Returns the shared `StubAgentConnection` used by `Agent::Stub`,
/// creating a default one if none was registered.
pub fn stub_agent_connection() -> StubAgentConnection {
    STUB_AGENT_CONNECTION.with(|cell| {
        let mut borrow = cell.borrow_mut();
        borrow.get_or_insert_with(StubAgentConnection::new).clone()
    })
}

pub struct StubAgentServer<C> {
    connection: C,
    agent_id: AgentId,
}

impl<C> StubAgentServer<C>
where
    C: AgentConnection,
{
    pub fn new(connection: C) -> Self {
        Self {
            connection,
            agent_id: "Test".into(),
        }
    }

    pub fn with_connection_agent_id(mut self) -> Self {
        self.agent_id = self.connection.agent_id();
        self
    }
}

impl StubAgentServer<StubAgentConnection> {
    pub fn default_response() -> Self {
        let conn = StubAgentConnection::new();
        conn.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Default response".into()),
        )]);
        Self::new(conn)
    }
}

impl<C> AgentServer for StubAgentServer<C>
where
    C: 'static + AgentConnection + Send + Clone,
{
    fn logo(&self) -> ui::IconName {
        ui::IconName::OmegaAgent
    }

    fn agent_id(&self) -> AgentId {
        self.agent_id.clone()
    }

    fn connect(
        &self,
        _delegate: AgentServerDelegate,
        _project: Entity<Project>,
        _cx: &mut gpui::App,
    ) -> Task<gpui::Result<Rc<dyn AgentConnection>>> {
        Task::ready(Ok(Rc::new(self.connection.clone())))
    }

    fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
        self
    }
}

pub fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        // Use an isolated DB so parallel tests can't see each other's
        // persisted records (e.g. created-worktree records).
        cx.set_global(db::AppDatabase::test_new());
        cx.set_global(acp_thread::StubSessionCounter(
            std::sync::atomic::AtomicUsize::new(0),
        ));
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        editor::init(cx);
        release_channel::init("0.0.0".parse().unwrap(), cx);
        agent_panel::init(cx);
        crate::terminal_thread_metadata_store::TerminalThreadMetadataStore::init_global(cx);
    });
}

/// Returns the creation time assigned to a linked worktree's git metadata
/// directory, mirroring `FakeGitRepository::worktree_created_at` (which uses
/// the FakeFs directory mtime as a stand-in for the creation time).
pub async fn fake_worktree_created_at(fs: &dyn fs::Fs, worktree_path: &Path) -> SystemTime {
    let git_file = fs.load(&worktree_path.join(".git")).await.unwrap();
    let git_dir = worktree_path.join(git_file.strip_prefix("gitdir:").unwrap().trim());
    let (seconds, nanos) = fs
        .metadata(&git_dir)
        .await
        .unwrap()
        .unwrap()
        .mtime
        .to_seconds_and_nanos_for_persistence()
        .unwrap();
    UNIX_EPOCH + Duration::new(seconds, nanos)
}

/// Records the worktree in the created-worktrees registry with its actual
/// (fake) creation time, as the worktree creation flow would. Tests that
/// expect a worktree to be archivable must call this after setting it up.
pub async fn record_zed_created_worktree(
    fs: &dyn fs::Fs,
    worktree_path: &Path,
    remote: Option<&remote::RemoteConnectionOptions>,
    cx: &mut TestAppContext,
) {
    let created_at = fake_worktree_created_at(fs, worktree_path).await;
    cx.update(|cx| {
        git_ui::created_worktrees::record_created_worktree(worktree_path, remote, created_at, cx)
    })
    .await
    .unwrap();
}

pub struct TestWorkspaceSidebar {
    focus_handle: FocusHandle,
    threads_list_active: bool,
}

impl TestWorkspaceSidebar {
    fn new(threads_list_active: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            threads_list_active,
        }
    }
}

impl EventEmitter<SidebarEvent> for TestWorkspaceSidebar {}

impl Focusable for TestWorkspaceSidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl WorkspaceSidebar for TestWorkspaceSidebar {
    fn width(&self, _cx: &App) -> Pixels {
        px(300.)
    }

    fn set_width(&mut self, _width: Option<Pixels>, _cx: &mut Context<Self>) {}

    fn has_notifications(&self, _cx: &App) -> bool {
        false
    }

    fn side(&self, _cx: &App) -> SidebarSide {
        SidebarSide::Left
    }

    fn is_threads_list_view_active(&self) -> bool {
        self.threads_list_active
    }
}

impl Render for TestWorkspaceSidebar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

pub fn register_test_sidebar(
    threads_list_active: bool,
    cx: &mut VisualTestContext,
) -> Entity<TestWorkspaceSidebar> {
    cx.update(|window, cx| {
        let multi_workspace = window
            .root::<MultiWorkspace>()
            .flatten()
            .expect("test window should have a MultiWorkspace root");
        let sidebar = cx.new(|cx| TestWorkspaceSidebar::new(threads_list_active, cx));
        multi_workspace.update(cx, |multi_workspace, cx| {
            multi_workspace.register_sidebar(sidebar.clone(), cx);
        });
        sidebar
    })
}

pub fn open_thread_with_connection(
    panel: &Entity<AgentPanel>,
    connection: StubAgentConnection,
    cx: &mut VisualTestContext,
) {
    panel.update_in(cx, |panel, window, cx| {
        panel.open_external_thread_with_server(
            Rc::new(StubAgentServer::new(connection)),
            window,
            cx,
        );
    });
    cx.run_until_parked();
}

/// Opens a draft thread against a stub server so the panel's `draft_thread`
/// pointer is populated for tests that care about draft UX.
pub fn open_draft_with_connection(
    panel: &Entity<AgentPanel>,
    connection: StubAgentConnection,
    cx: &mut VisualTestContext,
) {
    panel.update_in(cx, |panel, window, cx| {
        panel.open_draft_with_server(Rc::new(StubAgentServer::new(connection)), window, cx);
    });
    cx.run_until_parked();
}

pub fn open_thread_with_custom_connection<C>(
    panel: &Entity<AgentPanel>,
    connection: C,
    cx: &mut VisualTestContext,
) where
    C: 'static + AgentConnection + Send + Clone,
{
    panel.update_in(cx, |panel, window, cx| {
        panel.open_external_thread_with_server(
            Rc::new(StubAgentServer::new(connection).with_connection_agent_id()),
            window,
            cx,
        );
    });
    cx.run_until_parked();
}

pub fn send_message(panel: &Entity<AgentPanel>, cx: &mut VisualTestContext) {
    let thread_view = panel.read_with(cx, |panel, cx| panel.active_thread_view(cx).unwrap());
    let message_editor = thread_view.read_with(cx, |view, _cx| view.message_editor.clone());
    message_editor.update_in(cx, |editor, window, cx| {
        editor.set_text("Hello", window, cx);
    });
    thread_view.update_in(cx, |view, window, cx| view.send(window, cx));
    cx.run_until_parked();
}

pub fn type_draft_prompt(panel: &Entity<AgentPanel>, text: &str, cx: &mut VisualTestContext) {
    let thread_view = panel.read_with(cx, |panel, cx| panel.active_thread_view(cx).unwrap());
    let message_editor = thread_view.read_with(cx, |view, _cx| view.message_editor.clone());
    message_editor.update_in(cx, |editor, window, cx| {
        editor.set_text(text, window, cx);
    });
    cx.run_until_parked();
    // Drain the debounced draft-prompt persist task so the kvp write has
    // landed by the time we return.
    cx.executor()
        .advance_clock(crate::conversation_view::DRAFT_PROMPT_PERSIST_DEBOUNCE * 2);
    cx.run_until_parked();
}

pub fn active_session_id(panel: &Entity<AgentPanel>, cx: &VisualTestContext) -> acp::SessionId {
    panel.read_with(cx, |panel, cx| {
        let thread = panel.active_agent_thread(cx).unwrap();
        thread.read(cx).session_id().clone()
    })
}

pub fn active_thread_id(
    panel: &Entity<AgentPanel>,
    cx: &VisualTestContext,
) -> crate::thread_metadata_store::ThreadId {
    panel.read_with(cx, |panel, cx| panel.active_thread_id(cx).unwrap())
}

pub const WORKBENCH_ROOT_SELECTOR: &str = "omega.workbench.root";
pub const WORKBENCH_TOOLBAR_SELECTOR: &str = "omega.workbench.toolbar";
pub const WORKBENCH_NEW_THREAD_SELECTOR: &str = "omega.workbench.control.new-thread-menu";
pub const WORKBENCH_COMPOSER_SELECTOR: &str = "omega.workbench.composer";
pub const WORKBENCH_ACTIVITY_RAIL_SELECTOR: &str = "omega.workbench.activity-rail";
pub const WORKBENCH_DOCK_SELECTOR: &str = "omega.workbench.dock";
pub const WORKBENCH_TRANSCRIPT_SELECTOR: &str = "omega.workbench.transcript";
pub const WORKBENCH_IDENTITY_SELECTOR: &str = "omega.workbench.thread-identity";
pub const WORKBENCH_REPOSITORY_SELECTOR: &str = "omega.workbench.control.identity.repository";
pub const WORKBENCH_WORKTREE_SELECTOR: &str = "omega.workbench.control.identity.worktree";
pub const WORKBENCH_BRANCH_SELECTOR: &str = "omega.workbench.control.identity.branch";

pub struct AgentWorkbenchFrontDoor {
    scene: WorkbenchScene,
    fs: std::sync::Arc<fs::FakeFs>,
    window: AnyWindowHandle,
    workspace: Entity<Workspace>,
    panel: Entity<AgentPanel>,
}

impl AgentWorkbenchFrontDoor {
    pub async fn mount(scene: WorkbenchScene, cx: &mut TestAppContext) -> Result<Self> {
        validate_front_door_scene(&scene)?;

        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            crate::thread_metadata_store::ThreadMetadataStore::init_global(cx);
        });

        let fs = fs::FakeFs::new(cx.executor());
        cx.update(|cx| <dyn fs::Fs>::set_global(fs.clone(), cx));
        let root_paths = scene
            .repositories
            .iter()
            .flat_map(|repository| repository.worktrees.iter())
            .map(|worktree| std::path::PathBuf::from(format!("/{}", worktree.id)))
            .collect::<Vec<_>>();
        for repository in &scene.repositories {
            let main_worktree_path = repository
                .worktrees
                .first()
                .map(|worktree| std::path::PathBuf::from(format!("/{}", worktree.id)));
            for (worktree_index, worktree) in repository.worktrees.iter().enumerate() {
                let path = std::path::PathBuf::from(format!("/{}", worktree.id));
                if matches!(
                    worktree.git_state,
                    Some(omega_workbench_harness::WorktreeGitStateFixture::NoGit)
                ) {
                    fs.insert_tree(&path, serde_json::json!({"README.md": "# Fixture"}))
                        .await;
                    continue;
                }

                if worktree_index == 0 {
                    fs.insert_tree(
                        &path,
                        serde_json::json!({".git": {}, "README.md": "# Fixture"}),
                    )
                    .await;
                } else {
                    let main_worktree_path = main_worktree_path
                        .as_ref()
                        .context("linked worktree fixture has no main worktree")?;
                    let worktree_git_dir = main_worktree_path
                        .join(".git")
                        .join("worktrees")
                        .join(&worktree.id);
                    let head = worktree.branch.as_ref().map_or_else(
                        || "1111111111111111111111111111111111111111".to_string(),
                        |branch| format!("ref: refs/heads/{branch}"),
                    );
                    fs.insert_tree(
                        &worktree_git_dir,
                        serde_json::json!({
                            "HEAD": head,
                            "commondir": "../..",
                            "gitdir": path.join(".git").to_string_lossy(),
                        }),
                    )
                    .await;
                    fs.insert_tree(
                        &path,
                        serde_json::json!({
                            ".git": format!("gitdir: {}", worktree_git_dir.display()),
                            "README.md": "# Fixture",
                        }),
                    )
                    .await;
                }

                let dot_git = path.join(".git");
                fs.insert_branches(&dot_git, &["main", "release"]);
                fs.set_branch_name(&dot_git, worktree.branch.clone());
                if let Some(branch) = &worktree.branch {
                    fs.set_branch_tracking_for_repo(
                        &dot_git,
                        branch.clone(),
                        worktree.ahead,
                        worktree.behind,
                    );
                }
                if !matches!(
                    worktree.git_state,
                    Some(omega_workbench_harness::WorktreeGitStateFixture::Unborn)
                ) {
                    fs.set_head_for_repo(
                        &dot_git,
                        &[("README.md", "# Fixture".to_string())],
                        "1111111111111111111111111111111111111111",
                    );
                } else {
                    fs.with_git_state(&dot_git, true, |state| {
                        state.refs.remove("HEAD");
                    })?;
                }
                let status_count = worktree.dirty_files.max(worktree.conflicts);
                let mut statuses = Vec::new();
                for index in 0..status_count {
                    let relative_path = format!("fixture-status-{index}.txt");
                    fs.insert_file(&path.join(&relative_path), b"fixture".to_vec())
                        .await;
                    let status = if index < worktree.conflicts {
                        git::status::FileStatus::Unmerged(git::status::UnmergedStatus {
                            first_head: git::status::UnmergedStatusCode::Updated,
                            second_head: git::status::UnmergedStatusCode::Updated,
                        })
                    } else {
                        git::status::FileStatus::worktree(git::status::StatusCode::Modified)
                    };
                    statuses.push((relative_path, status));
                }
                let statuses = statuses
                    .iter()
                    .map(|(path, status)| (path.as_str(), *status))
                    .collect::<Vec<_>>();
                fs.set_status_for_repo(&dot_git, &statuses);
            }
        }
        let project = Project::test(
            fs.clone(),
            root_paths.iter().map(std::path::PathBuf::as_path),
            cx,
        )
        .await;
        let git_scans_complete = project.update(cx, |project, cx| project.git_scans_complete(cx));
        git_scans_complete.await;
        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project, window, cx));
        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .context("test window did not create a workspace")?;
        let window = multi_workspace.into();
        let mut visual = VisualTestContext::from_window(window, cx);
        visual.simulate_resize(size(
            px(scene.viewport.width as f32),
            px(scene.viewport.height as f32),
        ));

        let (panel, focused) = workspace.update_in(&mut visual, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            panel.update(cx, |panel, cx| {
                panel.enable_workbench_shell_for_tests(cx);
                panel.set_zoomed(true, window, cx);
            });
            workspace.add_panel(panel.clone(), window, cx);
            let focused = workspace.focus_panel::<AgentPanel>(window, cx).is_some();
            (panel, focused)
        });
        if !focused {
            bail!("AgentPanel was not focusable after being mounted");
        }

        visual.set_debug_accessibility_active(true);
        if scene.active_thread_id.is_some() {
            visual.dispatch_action(crate::NewThread);
        }

        panel.update_in(&mut visual, |panel, _window, cx| {
            for surface in &scene.surfaces {
                let badge =
                    surface
                        .badge
                        .map(|count| crate::workbench_shell::SurfaceBadge::Count {
                            count: count as usize,
                            tone: crate::workbench_shell::BadgeTone::Neutral,
                            label: format!("{} notifications", surface.id.label()).into(),
                        });
                panel.set_workbench_badge_for_tests(work_surface(surface.id), badge, cx);
            }
        });
        if let Some(surface) = scene.active_surface {
            dispatch_surface_action(&mut visual, work_surface(surface));
        }
        visual.run_until_parked();

        let has_active_thread =
            panel.read_with(&visual, |panel, cx| panel.active_thread_view(cx).is_some());
        if has_active_thread != scene.active_thread_id.is_some() {
            bail!(
                "rendered active-thread state {has_active_thread} did not match fixture expectation {}",
                scene.active_thread_id.is_some()
            );
        }

        Ok(Self {
            scene,
            fs,
            window,
            workspace,
            panel,
        })
    }

    pub fn scene(&self) -> &WorkbenchScene {
        &self.scene
    }

    pub fn panel(&self) -> &Entity<AgentPanel> {
        &self.panel
    }

    pub fn snapshot(&self, cx: &TestAppContext) -> DebugRenderSnapshot {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        visual.run_until_parked();
        visual.debug_render_snapshot()
    }

    pub fn dispatch_action(&self, action: impl Action, cx: &TestAppContext) {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        visual.dispatch_action(action);
        visual.run_until_parked();
    }

    pub fn click(&self, selector: &str, cx: &TestAppContext) -> Result<()> {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        visual.simulate_click_selector(selector)?;
        visual.run_until_parked();
        Ok(())
    }

    pub fn resize(&self, width: u32, height: u32, cx: &TestAppContext) {
        let visual = VisualTestContext::from_window(self.window, cx);
        visual.simulate_resize(size(px(width as f32), px(height as f32)));
        visual.run_until_parked();
    }

    pub fn open_external_thread(
        &self,
        connection: StubAgentConnection,
        cx: &mut TestAppContext,
    ) -> crate::thread_metadata_store::ThreadId {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        open_thread_with_connection(&self.panel, connection, &mut visual);
        active_thread_id(&self.panel, &visual)
    }

    pub fn activate_thread(
        &self,
        thread_id: crate::thread_metadata_store::ThreadId,
        cx: &mut TestAppContext,
    ) {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        self.panel.update_in(&mut visual, |panel, window, cx| {
            panel.activate_retained_thread(thread_id, true, window, cx);
        });
        visual.run_until_parked();
    }

    pub fn transcript_entity_id(&self, cx: &TestAppContext) -> Option<EntityId> {
        self.panel.read_with(cx, |panel, _cx| {
            panel.active_conversation_view().map(Entity::entity_id)
        })
    }

    pub fn surface_host_entity_id(
        &self,
        surface: omega_workbench_state::WorkSurface,
        cx: &TestAppContext,
    ) -> Option<EntityId> {
        self.panel.read_with(cx, |panel, cx| {
            panel.workbench_host_entity_id_for_tests(surface, cx)
        })
    }

    pub fn projection(&self, cx: &TestAppContext) -> omega_workbench_state::WorkbenchProjection {
        self.panel.read_with(cx, |panel, _cx| {
            panel.workbench_projection_for_tests().clone()
        })
    }

    pub fn identity(
        &self,
        cx: &TestAppContext,
    ) -> Option<crate::thread_identity::ThreadIdentityState> {
        self.panel.read_with(cx, |panel, _cx| {
            panel.workbench_identity_for_tests().cloned()
        })
    }

    pub fn capability(
        &self,
        surface: omega_workbench_state::WorkSurface,
        cx: &TestAppContext,
    ) -> Option<crate::workbench_shell::SurfaceCapability> {
        self.panel.read_with(cx, |panel, _cx| {
            panel.workbench_capability_for_tests(surface).cloned()
        })
    }

    pub fn set_identity_phase(
        &self,
        phase: crate::thread_identity::IdentityPhase,
        cx: &mut TestAppContext,
    ) {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        self.panel.update_in(&mut visual, |panel, _window, cx| {
            panel.set_workbench_identity_phase_for_tests(Some(phase), cx);
        });
        visual.run_until_parked();
    }

    pub fn mark_identity_inconsistent(
        &self,
        message: impl Into<SharedString>,
        cx: &mut TestAppContext,
    ) -> Result<()> {
        let message = message.into();
        let mut visual = VisualTestContext::from_window(self.window, cx);
        self.panel.update_in(&mut visual, |panel, _window, cx| {
            panel.mark_workbench_identity_inconsistent_for_tests(message, cx)
        })?;
        visual.run_until_parked();
        Ok(())
    }

    pub fn fail_next_host_creation(
        &self,
        surface: omega_workbench_state::WorkSurface,
        cx: &mut TestAppContext,
    ) {
        self.panel.update(cx, |panel, _cx| {
            panel.fail_next_workbench_host_creation_for_tests(surface);
        });
    }

    pub fn invalidate_surface(
        &self,
        surface: omega_workbench_state::WorkSurface,
        cx: &mut TestAppContext,
    ) -> Result<omega_workbench_state::TransitionEffect> {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        self.panel.update_in(&mut visual, |panel, window, cx| {
            panel.invalidate_workbench_surface_for_tests(surface, window, cx)
        })
    }

    pub fn begin_surface_load(
        &self,
        request_id: impl Into<String>,
        surface: omega_workbench_state::WorkSurface,
        cx: &mut TestAppContext,
    ) -> Result<crate::workbench_shell::SurfaceLoadContext> {
        let request_id = request_id.into();
        self.panel.update(cx, |panel, cx| {
            panel.begin_workbench_surface_load_for_tests(request_id, surface, cx)
        })
    }

    pub fn complete_surface_load(
        &self,
        load: crate::workbench_shell::SurfaceLoadContext,
        outcome: crate::workbench_shell::SurfaceLoadOutcome,
        cx: &mut TestAppContext,
    ) -> Result<omega_workbench_state::TransitionEffect> {
        self.panel.update(cx, |panel, cx| {
            panel.complete_workbench_surface_load_for_tests(load, outcome, cx)
        })
    }

    pub fn visible_surface_host(
        &self,
        cx: &TestAppContext,
    ) -> Option<Entity<crate::workbench_shell::WorkSurfaceHost>> {
        self.panel
            .read_with(cx, |panel, _cx| panel.visible_workbench_host_for_tests())
    }

    pub fn exercise_new_thread_menu(&self, cx: &TestAppContext) -> Result<()> {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        let before = visual.debug_render_snapshot();
        if accessibility_expanded(&before, WORKBENCH_NEW_THREAD_SELECTOR)? {
            bail!("new-thread menu started expanded");
        }

        visual.simulate_click_selector(WORKBENCH_NEW_THREAD_SELECTOR)?;
        let opened = visual.debug_render_snapshot();
        if !accessibility_expanded(&opened, WORKBENCH_NEW_THREAD_SELECTOR)? {
            bail!("clicking the rendered new-thread control did not expand its menu");
        }

        visual.simulate_click_selector(WORKBENCH_NEW_THREAD_SELECTOR)?;
        let closed = visual.debug_render_snapshot();
        if accessibility_expanded(&closed, WORKBENCH_NEW_THREAD_SELECTOR)? {
            bail!("clicking the rendered new-thread control again did not close its menu");
        }
        Ok(())
    }

    pub fn select_identity_picker_row(&self, row_index: usize, cx: &TestAppContext) -> Result<()> {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        let repository_selector = self
            .panel
            .read_with(&visual, |panel, _cx| {
                panel
                    .workbench_identity_for_tests()
                    .and_then(|identity| identity.candidates.get(row_index))
                    .map(|candidate| {
                        format!(
                            "omega.workbench.control.repository.{}",
                            candidate.binding.repository_id
                        )
                    })
            })
            .with_context(|| format!("repository picker has no row {row_index}"))?;
        visual.simulate_click_selector(WORKBENCH_REPOSITORY_SELECTOR)?;
        visual.run_until_parked();
        if !accessibility_expanded(
            &visual.debug_render_snapshot(),
            WORKBENCH_REPOSITORY_SELECTOR,
        )? {
            bail!("repository picker did not expand");
        }
        visual.simulate_click_selector(&repository_selector)?;
        visual.run_until_parked();
        Ok(())
    }

    pub fn select_worktree_picker_row(&self, row_index: usize, cx: &TestAppContext) -> Result<()> {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        let candidates = self
            .panel
            .read_with(&visual, |panel, _cx| {
                panel
                    .workbench_identity_for_tests()
                    .map(|identity| identity.candidates.clone())
            })
            .context("worktree picker has no identity candidates")?;
        let worktree_selector = candidates
            .get(row_index)
            .map(|candidate| {
                format!(
                    "omega.workbench.control.worktree.{}",
                    candidate.binding.worktree_id
                )
            })
            .with_context(|| format!("worktree picker has no row {row_index}"))?;
        visual.simulate_click_selector(WORKBENCH_WORKTREE_SELECTOR)?;
        visual.run_until_parked();
        let snapshot = visual.debug_render_snapshot();
        if !accessibility_expanded(&snapshot, WORKBENCH_WORKTREE_SELECTOR)? {
            bail!("worktree picker did not expand");
        }
        let mut probe = SemanticProbe::new(&snapshot);
        let mut previous_y = None;
        for candidate in &candidates {
            let selector = format!(
                "omega.workbench.control.worktree.{}",
                candidate.binding.worktree_id
            );
            let label = format!(
                "{} — {}",
                candidate.repository_name, candidate.worktree_path
            );
            probe.require_accessible(&selector, "MenuItem", &label)?;
            let y = snapshot
                .bounds(&selector)
                .with_context(|| format!("rendered worktree row {label:?} has no bounds"))?
                .origin
                .y;
            if let Some(previous_y) = previous_y {
                if y <= previous_y {
                    bail!("worktree picker rows do not preserve candidate order");
                }
            }
            previous_y = Some(y);
        }
        visual.simulate_click_selector(&worktree_selector)?;
        visual.run_until_parked();
        Ok(())
    }

    pub fn remove_worktree(&self, path: &Path, cx: &mut TestAppContext) -> Result<()> {
        let project = self
            .workspace
            .read_with(cx, |workspace, _cx| workspace.project().clone());
        let worktree_id = project
            .read_with(cx, |project, cx| {
                project
                    .visible_worktrees(cx)
                    .find(|worktree| worktree.read(cx).abs_path().as_ref() == path)
                    .map(|worktree| worktree.read(cx).id())
            })
            .with_context(|| format!("no visible worktree at {}", path.display()))?;
        project.update(cx, |project, cx| {
            project.remove_worktree(worktree_id, cx);
        });
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(self.window, cx);
        self.workspace
            .update_in(&mut visual, |workspace, window, cx| {
                workspace.focus_panel::<AgentPanel>(window, cx);
            });
        self.panel.update(&mut visual, |_, cx| cx.notify());
        visual.run_until_parked();
        Ok(())
    }

    pub fn fail_next_branch_selection(
        &self,
        worktree_path: &Path,
        message: &str,
        cx: &TestAppContext,
    ) -> Result<()> {
        let dot_git = worktree_path.join(".git");
        self.fs
            .set_simulated_change_branch_error(&dot_git, Some(message.to_string()));
        cx.run_until_parked();

        let mut visual = VisualTestContext::from_window(self.window, cx);
        visual.simulate_click_selector(WORKBENCH_BRANCH_SELECTOR)?;
        visual.run_until_parked();
        if !accessibility_expanded(&visual.debug_render_snapshot(), WORKBENCH_BRANCH_SELECTOR)? {
            bail!("branch picker did not expand");
        }
        visual.dispatch_action(menu::SelectNext);
        visual.dispatch_action(menu::Confirm);
        visual.run_until_parked();
        Ok(())
    }

    pub fn select_next_branch(&self, cx: &TestAppContext) -> Result<()> {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        visual.simulate_click_selector(WORKBENCH_BRANCH_SELECTOR)?;
        visual.run_until_parked();
        if !accessibility_expanded(&visual.debug_render_snapshot(), WORKBENCH_BRANCH_SELECTOR)? {
            bail!("branch picker did not expand");
        }
        visual.dispatch_action(menu::SelectNext);
        visual.dispatch_action(menu::Confirm);
        visual.run_until_parked();
        Ok(())
    }

    pub fn prove_scene(
        scene: &WorkbenchScene,
        snapshot: &DebugRenderSnapshot,
    ) -> Result<Vec<ProofCheck>> {
        validate_front_door_scene(scene)?;
        let mut checks = omega_workbench_harness::prove_workbench_shell(scene, snapshot)?;
        let mut probe = SemanticProbe::new(snapshot);
        probe.require_visible(WORKBENCH_ROOT_SELECTOR)?;
        probe.require_visible(WORKBENCH_TOOLBAR_SELECTOR)?;
        probe.require_interactive(WORKBENCH_NEW_THREAD_SELECTOR)?;
        probe.require_inside(WORKBENCH_TOOLBAR_SELECTOR, WORKBENCH_ROOT_SELECTOR)?;
        probe.require_inside(WORKBENCH_NEW_THREAD_SELECTOR, WORKBENCH_TOOLBAR_SELECTOR)?;
        probe.require_accessible(WORKBENCH_NEW_THREAD_SELECTOR, "Button", "New Thread")?;
        probe.require_fully_visible(WORKBENCH_ACTIVITY_RAIL_SELECTOR)?;
        probe.require_inside(WORKBENCH_ACTIVITY_RAIL_SELECTOR, WORKBENCH_ROOT_SELECTOR)?;
        probe.require_accessible(WORKBENCH_ACTIVITY_RAIL_SELECTOR, "Toolbar", "Work surfaces")?;
        probe.require_accessibility_property(
            WORKBENCH_ACTIVITY_RAIL_SELECTOR,
            "orientation",
            serde_json::Value::String("Vertical".into()),
        )?;

        if scene.active_thread_id.is_some() {
            probe.require_visible(WORKBENCH_COMPOSER_SELECTOR)?;
            probe.require_inside(WORKBENCH_COMPOSER_SELECTOR, WORKBENCH_ROOT_SELECTOR)?;
            probe.require_visible(WORKBENCH_TRANSCRIPT_SELECTOR)?;
            probe.require_visible(WORKBENCH_IDENTITY_SELECTOR)?;
            probe.require_inside(WORKBENCH_IDENTITY_SELECTOR, WORKBENCH_TOOLBAR_SELECTOR)?;
            probe.require_interactive(WORKBENCH_REPOSITORY_SELECTOR)?;
            if let Some(repository) = scene.repositories.first()
                && let Some(worktree) = repository.worktrees.first()
            {
                probe.require_accessible(
                    WORKBENCH_REPOSITORY_SELECTOR,
                    "Button",
                    &format!("Repository {}", worktree.id),
                )?;
                probe.require_interactive(WORKBENCH_WORKTREE_SELECTOR)?;
                probe.require_accessible(
                    WORKBENCH_WORKTREE_SELECTOR,
                    "Button",
                    &format!("Worktree {}", worktree.id),
                )?;
                if let Some(branch) = &worktree.branch {
                    probe.require_interactive(WORKBENCH_BRANCH_SELECTOR)?;
                    probe.require_accessible(
                        WORKBENCH_BRANCH_SELECTOR,
                        "Button",
                        &format!("Branch {branch}"),
                    )?;
                }
            } else {
                probe.require_accessible(
                    WORKBENCH_REPOSITORY_SELECTOR,
                    "Button",
                    "Choose a repository folder",
                )?;
                probe.require_absent(WORKBENCH_WORKTREE_SELECTOR)?;
            }
        } else {
            probe.require_absent(WORKBENCH_COMPOSER_SELECTOR)?;
            probe.require_absent(WORKBENCH_TRANSCRIPT_SELECTOR)?;
            probe.require_absent(WORKBENCH_IDENTITY_SELECTOR)?;
        }

        for surface in &scene.surfaces {
            let selector = surface.id.rail_selector();
            probe.require_accessible(selector, "Button", surface.id.label())?;
            probe.require_accessibility_property(
                selector,
                "disabled",
                serde_json::Value::Bool(!surface.available),
            )?;
            let expanded = scene.dock_open && scene.active_surface == Some(surface.id);
            probe.require_accessibility_property(
                selector,
                "expanded",
                serde_json::Value::Bool(expanded),
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
            probe.require_visible(WORKBENCH_DOCK_SELECTOR)?;
            probe.require_inside(WORKBENCH_DOCK_SELECTOR, WORKBENCH_ROOT_SELECTOR)?;
            probe.require_visible(surface.surface_selector())?;
            probe.require_inside(surface.surface_selector(), WORKBENCH_DOCK_SELECTOR)?;
            probe.require_disjoint(WORKBENCH_DOCK_SELECTOR, WORKBENCH_TRANSCRIPT_SELECTOR)?;
        } else {
            probe.require_absent(WORKBENCH_DOCK_SELECTOR)?;
        }

        checks.extend(probe.into_checks());
        Ok(checks)
    }

    pub fn teardown(self, cx: &mut TestAppContext) -> Result<()> {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        self.workspace
            .update_in(&mut visual, |workspace, window, cx| {
                workspace.close_panel::<AgentPanel>(window, cx);
            });
        visual.run_until_parked();

        let Self {
            scene: _,
            fs: _,
            window,
            workspace,
            panel,
        } = self;
        drop(panel);
        drop(workspace);

        cx.update_window(window, |_, window, _cx| window.remove_window())
            .context("closing AgentUI workbench test window")?;
        cx.run_until_parked();

        if cx.windows().contains(&window) {
            bail!("AgentUI workbench test window remained open after teardown");
        }
        Ok(())
    }
}

fn dispatch_surface_action(
    visual: &mut VisualTestContext,
    surface: omega_workbench_state::WorkSurface,
) {
    match surface {
        omega_workbench_state::WorkSurface::Files => {
            visual.dispatch_action(crate::workbench_shell::SelectFiles)
        }
        omega_workbench_state::WorkSurface::Search => {
            visual.dispatch_action(crate::workbench_shell::SelectSearch)
        }
        omega_workbench_state::WorkSurface::Review => {
            visual.dispatch_action(crate::workbench_shell::SelectReview)
        }
        omega_workbench_state::WorkSurface::Git => {
            visual.dispatch_action(crate::workbench_shell::SelectGit)
        }
        omega_workbench_state::WorkSurface::Terminal => {
            visual.dispatch_action(crate::workbench_shell::SelectTerminal)
        }
        omega_workbench_state::WorkSurface::Plan => {
            visual.dispatch_action(crate::workbench_shell::SelectPlan)
        }
    }
}

fn validate_front_door_scene(scene: &WorkbenchScene) -> Result<()> {
    scene.validate()?;
    if scene.fixture_version != 1 {
        bail!(
            "AgentUI front-door adapter supports fixture version 1, got {}",
            scene.fixture_version
        );
    }
    if scene.viewport.scale_milli != 2000 {
        bail!("AgentUI front-door adapter requires a 2x fixture scale");
    }
    if scene.theme != ThemeFixture::Dark
        || scene.fake_time_ms != 0
        || scene.connectivity != ConnectivityFixture::Online
        || scene.content_state != ContentStateFixture::Empty
    {
        bail!("AgentUI front-door adapter only supports the deterministic empty online dark scene");
    }
    if !scene.messages.is_empty()
        || !scene.tool_calls.is_empty()
        || !scene.plan_steps.is_empty()
        || !scene.artifacts.is_empty()
        || !scene.events.is_empty()
        || scene.persisted.is_some()
    {
        bail!("AgentUI front-door adapter received unsupported workbench content");
    }
    let has_thread = scene.active_thread_id.is_some();
    let has_project = scene.project.is_some();
    if has_project != !scene.repositories.is_empty() {
        bail!("AgentUI scene project and repository fixtures must be present together");
    }
    for surface in &scene.surfaces {
        let expected_available = has_thread && (has_project || surface.id == WorkSurfaceId::Plan);
        if surface.available != expected_available {
            bail!(
                "projectless AgentUI scene expected {} availability={}, got {}",
                surface.id.as_str(),
                expected_available,
                surface.available
            );
        }
    }
    if !has_project
        && scene
            .active_surface
            .is_some_and(|surface| surface != WorkSurfaceId::Plan)
    {
        bail!("projectless AgentUI scene can only select Plan");
    }
    if scene.dock_open != scene.active_surface.is_some() {
        bail!("dock visibility must agree with the active surface");
    }
    if scene.threads.len() > 1 {
        bail!("AgentUI front-door adapter supports at most one projectless thread");
    }
    if !has_project
        && scene.threads.iter().any(|thread| {
            thread.project_id.is_some()
                || thread.repository_id.is_some()
                || thread.worktree_id.is_some()
        })
    {
        bail!("projectless AgentUI threads cannot carry repository identity");
    }
    if scene.active_thread_id.is_some() && scene.threads.len() != 1 {
        bail!("an active front-door thread requires exactly one thread fixture");
    }
    Ok(())
}

fn work_surface(surface: WorkSurfaceId) -> omega_workbench_state::WorkSurface {
    match surface {
        WorkSurfaceId::Files => omega_workbench_state::WorkSurface::Files,
        WorkSurfaceId::Search => omega_workbench_state::WorkSurface::Search,
        WorkSurfaceId::Review => omega_workbench_state::WorkSurface::Review,
        WorkSurfaceId::Git => omega_workbench_state::WorkSurface::Git,
        WorkSurfaceId::Terminal => omega_workbench_state::WorkSurface::Terminal,
        WorkSurfaceId::Plan => omega_workbench_state::WorkSurface::Plan,
    }
}

fn accessibility_expanded(snapshot: &DebugRenderSnapshot, element_id: &str) -> Result<bool> {
    let tree = snapshot
        .accessibility_tree_json()
        .context("accessibility tree was not active")?;
    let value: serde_json::Value =
        serde_json::from_str(tree).context("parsing accessibility tree")?;
    let nodes = value
        .get("nodes")
        .and_then(serde_json::Value::as_object)
        .context("accessibility tree has no nodes object")?;
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
    Ok(matching[0]
        .get("aria")
        .and_then(|aria| aria.get("expanded"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false))
}

#[cfg(test)]
mod workbench_front_door_tests {
    use super::*;
    use omega_workbench_harness::{
        ProjectFixture, RepositoryFixture, ThreadFixture, ViewportFixture, WorktreeFixture,
        WorktreeGitStateFixture,
    };
    use workspace::PathList;

    #[derive(Clone)]
    struct PendingSessionConnection;

    impl AgentConnection for PendingSessionConnection {
        fn agent_id(&self) -> AgentId {
            AgentId::new("pending-session-test")
        }

        fn telemetry_id(&self) -> gpui::SharedString {
            "pending-session-test".into()
        }

        fn new_session(
            self: Rc<Self>,
            _project: Entity<Project>,
            _work_dirs: PathList,
            cx: &mut App,
        ) -> Task<Result<Entity<acp_thread::AcpThread>>> {
            cx.spawn(async move |_cx| {
                futures::future::pending::<Result<Entity<acp_thread::AcpThread>>>().await
            })
        }

        fn auth_methods(&self) -> &[acp::AuthMethod] {
            &[]
        }

        fn authenticate(&self, _method: acp::AuthMethodId, _cx: &mut App) -> Task<Result<()>> {
            Task::ready(Ok(()))
        }

        fn prompt(
            &self,
            _params: acp::PromptRequest,
            _cx: &mut App,
        ) -> Task<Result<acp::PromptResponse>> {
            Task::ready(Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)))
        }

        fn cancel(&self, _session_id: &acp::SessionId, _cx: &mut App) {}

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    fn scene_with_thread(name: &str, width: u32, with_project: bool) -> WorkbenchScene {
        let mut scene = WorkbenchScene::empty(name, ViewportFixture::new(width, 720, 2000));
        scene.threads.push(ThreadFixture {
            id: "thread-1".into(),
            project_id: with_project.then(|| "project-1".into()),
            repository_id: with_project.then(|| "repository-1".into()),
            worktree_id: with_project.then(|| "worktree-1".into()),
        });
        scene.active_thread_id = Some("thread-1".into());
        if with_project {
            scene.project = Some(ProjectFixture {
                id: "project-1".into(),
                display_name: "Fixture project".into(),
            });
            scene.repositories.push(RepositoryFixture {
                id: "repository-1".into(),
                project_id: "project-1".into(),
                worktrees: vec![WorktreeFixture {
                    id: "worktree-1".into(),
                    branch: Some("main".into()),
                    git_state: None,
                    dirty_files: 0,
                    conflicts: 0,
                    ahead: 0,
                    behind: 0,
                }],
            });
        }
        for surface in &mut scene.surfaces {
            surface.available = with_project || surface.id == WorkSurfaceId::Plan;
        }
        scene
    }

    #[gpui::test]
    async fn typed_fixture_drives_rendered_front_door_semantics(cx: &mut TestAppContext) {
        let scene = scene_with_thread("agent_ui_front_door", 900, false);

        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("typed front-door fixture should mount");
        front_door
            .exercise_new_thread_menu(cx)
            .expect("the rendered new-thread menu should be operable by selector");

        let snapshot = front_door.snapshot(cx);
        let checks = AgentWorkbenchFrontDoor::prove_scene(front_door.scene(), &snapshot)
            .expect("rendered front door should satisfy fixture semantics");
        assert!(
            !checks.is_empty(),
            "front-door proof must record semantic checks"
        );

        let rendered_composer_bounds = snapshot.bounds(WORKBENCH_COMPOSER_SELECTOR);
        let mut semantic_mutation = front_door.scene().clone();
        semantic_mutation.active_thread_id = None;
        semantic_mutation.threads.clear();
        for surface in &mut semantic_mutation.surfaces {
            surface.available = false;
        }
        let error = AgentWorkbenchFrontDoor::prove_scene(&semantic_mutation, &snapshot)
            .expect_err("semantic mutation must fail without another render");
        assert!(
            error.to_string().contains(WORKBENCH_COMPOSER_SELECTOR)
                || error
                    .to_string()
                    .contains(WorkSurfaceId::Plan.rail_selector()),
            "unexpected semantic mutation failure: {error:#}"
        );
        assert_eq!(
            snapshot.bounds(WORKBENCH_COMPOSER_SELECTOR),
            rendered_composer_bounds,
            "the semantic oracle must fail against the unchanged rendered frame"
        );

        front_door
            .teardown(cx)
            .expect("front-door teardown should release its window and entities");
    }

    #[gpui::test]
    async fn thread_identity_uses_real_project_and_git_projection(cx: &mut TestAppContext) {
        let scene = scene_with_thread("thread_identity_real_git", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene.clone(), cx)
            .await
            .expect("Git identity fixture should mount");

        let identity = front_door.identity(cx).expect("active identity");
        assert_eq!(identity.phase, crate::thread_identity::IdentityPhase::Ready);
        let selected = identity.selected.expect("selected repository");
        assert_eq!(selected.repository_name.as_ref(), "worktree-1");
        assert_eq!(selected.worktree_name.as_ref(), "worktree-1");
        assert_eq!(
            selected.branch,
            crate::thread_identity::BranchIdentity::Branch("main".into())
        );
        assert_eq!(
            front_door
                .projection(cx)
                .visible_projection()
                .and_then(|visible| visible.binding),
            Some(selected.binding),
            "header identity and work-surface routing must share one binding"
        );

        let snapshot = front_door.snapshot(cx);
        AgentWorkbenchFrontDoor::prove_scene(&scene, &snapshot)
            .expect("identity scene should satisfy the shared semantic oracle");
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_fully_visible(WORKBENCH_IDENTITY_SELECTOR)
            .expect("identity strip should stay within the toolbar");
        probe
            .require_accessibility_property(
                WORKBENCH_REPOSITORY_SELECTOR,
                "description",
                serde_json::Value::String(
                    "Project worktree-1, repository worktree-1, worktree worktree-1 at /worktree-1, main"
                        .into(),
                ),
            )
            .expect("repository accessibility should retain the full identity");

        front_door
            .teardown(cx)
            .expect("identity scene should tear down");
    }

    #[gpui::test]
    async fn linked_worktrees_share_repository_identity_and_select_independently(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("thread_identity_linked_worktrees", 1200, true);
        scene.repositories[0].worktrees.push(WorktreeFixture {
            id: "worktree-2".into(),
            branch: Some("main".into()),
            git_state: None,
            dirty_files: 0,
            conflicts: 0,
            ahead: 0,
            behind: 0,
        });
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("linked-worktree fixture should mount");
        let identity = front_door.identity(cx).expect("linked-worktree identity");
        assert_eq!(identity.candidates.len(), 2);
        assert_eq!(
            identity.candidates[0].binding.repository_id,
            identity.candidates[1].binding.repository_id,
            "linked worktrees must project one repository identity"
        );
        assert_ne!(
            identity.candidates[0].binding.worktree_id,
            identity.candidates[1].binding.worktree_id
        );

        front_door
            .select_worktree_picker_row(1, cx)
            .expect("select the linked worktree through the rendered worktree picker");
        assert_eq!(
            front_door
                .identity(cx)
                .and_then(|identity| identity.selected)
                .map(|selected| selected.worktree_name),
            Some("worktree-2".into())
        );

        front_door
            .teardown(cx)
            .expect("linked-worktree scene should tear down");
    }

    #[gpui::test]
    async fn loading_thread_projects_its_desired_worktree_before_session_creation(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("thread_identity_loading_desired_worktree", 1200, true);
        scene.repositories[0].worktrees.push(WorktreeFixture {
            id: "worktree-2".into(),
            branch: Some("main".into()),
            git_state: None,
            dirty_files: 0,
            conflicts: 0,
            ahead: 0,
            behind: 0,
        });
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("loading worktree fixture should mount");
        let mut visual = VisualTestContext::from_window(front_door.window, cx);
        front_door
            .panel
            .update_in(&mut visual, |panel, window, cx| {
                panel.open_external_thread_with_server_and_work_dirs(
                    Rc::new(StubAgentServer::new(PendingSessionConnection)),
                    PathList::new(&[std::path::PathBuf::from("/worktree-2")]),
                    window,
                    cx,
                );
            });
        visual.run_until_parked();

        assert!(
            front_door.panel.read_with(&visual, |panel, cx| {
                panel
                    .active_thread_view_for_tests()
                    .is_some_and(|view| view.read(cx).is_loading())
            }),
            "the fixture must still be waiting for session creation"
        );
        assert_eq!(
            front_door
                .identity(cx)
                .and_then(|identity| identity.selected)
                .map(|selected| selected.worktree_name),
            Some("worktree-2".into()),
            "desired work dirs, not lexicographic project order, must select the loading identity"
        );

        front_door
            .teardown(cx)
            .expect("loading worktree scene should tear down");
    }

    #[gpui::test]
    async fn no_git_and_unborn_worktrees_are_projected_from_real_project_scans(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("thread_identity_no_git_unborn", 1200, true);
        let no_git = &mut scene.repositories[0].worktrees[0];
        no_git.branch = None;
        no_git.git_state = Some(WorktreeGitStateFixture::NoGit);
        scene.repositories.push(RepositoryFixture {
            id: "repository-2".into(),
            project_id: "project-1".into(),
            worktrees: vec![WorktreeFixture {
                id: "worktree-2".into(),
                branch: Some("main".into()),
                git_state: Some(WorktreeGitStateFixture::Unborn),
                dirty_files: 0,
                conflicts: 0,
                ahead: 0,
                behind: 0,
            }],
        });
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("no-Git and unborn fixture should mount");
        assert!(matches!(
            front_door
                .identity(cx)
                .and_then(|identity| identity.selected)
                .map(|selected| selected.branch),
            Some(crate::thread_identity::BranchIdentity::NoGit)
        ));

        front_door
            .select_identity_picker_row(1, cx)
            .expect("select the unborn repository through the rendered picker");
        assert!(matches!(
            front_door
                .identity(cx)
                .and_then(|identity| identity.selected)
                .map(|selected| selected.branch),
            Some(crate::thread_identity::BranchIdentity::Unborn)
        ));

        front_door
            .teardown(cx)
            .expect("no-Git and unborn scene should tear down");
    }

    #[gpui::test]
    async fn detached_head_is_rendered_as_a_typed_branch_state(cx: &mut TestAppContext) {
        let mut scene = scene_with_thread("thread_identity_detached", 1200, true);
        scene.repositories[0].worktrees[0].branch = None;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("detached fixture should mount");

        assert_eq!(
            front_door
                .identity(cx)
                .and_then(|identity| identity.selected)
                .map(|selected| selected.branch),
            Some(crate::thread_identity::BranchIdentity::Detached(
                "11111111".into()
            ))
        );
        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_accessible(
                WORKBENCH_BRANCH_SELECTOR,
                "Button",
                "Branch Detached at 11111111",
            )
            .expect("detached HEAD must be explicit in the rendered header");
        probe
            .require_interactive(WORKBENCH_BRANCH_SELECTOR)
            .expect("detached HEAD should still allow choosing a named branch");

        front_door
            .teardown(cx)
            .expect("detached workbench should tear down");
    }

    #[gpui::test]
    async fn long_identity_segments_remain_independently_visible_at_desktop_width(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("thread_identity_long_desktop", 1200, true);
        let long_worktree_id: String =
            "worktree-with-a-deliberately-long-name-that-must-not-cover-neighboring-controls"
                .into();
        scene.repositories[0].worktrees[0].id = long_worktree_id.clone();
        scene.threads[0].worktree_id = Some(long_worktree_id);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("long desktop identity fixture should mount");

        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        for selector in [
            WORKBENCH_REPOSITORY_SELECTOR,
            WORKBENCH_WORKTREE_SELECTOR,
            WORKBENCH_BRANCH_SELECTOR,
        ] {
            probe
                .require_fully_visible(selector)
                .unwrap_or_else(|error| {
                    panic!("{selector} must remain independently visible: {error:#}")
                });
        }
        let repository_bounds = snapshot
            .bounds(WORKBENCH_REPOSITORY_SELECTOR)
            .expect("repository segment bounds");
        let worktree_bounds = snapshot
            .bounds(WORKBENCH_WORKTREE_SELECTOR)
            .expect("worktree segment bounds");
        let branch_bounds = snapshot
            .bounds(WORKBENCH_BRANCH_SELECTOR)
            .expect("branch segment bounds");
        assert!(
            repository_bounds.origin.x < worktree_bounds.origin.x
                && worktree_bounds.origin.x < branch_bounds.origin.x,
            "identity segments must render in repository, worktree, branch order"
        );
        probe
            .require_accessibility_property(
                WORKBENCH_WORKTREE_SELECTOR,
                "description",
                serde_json::Value::String(
                    "Worktree path \
                     /worktree-with-a-deliberately-long-name-that-must-not-cover-neighboring-controls"
                        .into(),
                ),
            )
            .expect("truncation must preserve the full worktree path for accessibility");

        front_door
            .teardown(cx)
            .expect("long desktop identity scene should tear down");
    }

    #[gpui::test(iterations = 16)]
    async fn picker_changes_repository_atomically_and_rejects_stale_completion(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("thread_identity_picker", 1200, true);
        scene.repositories.push(RepositoryFixture {
            id: "repository-2".into(),
            project_id: "project-1".into(),
            worktrees: vec![WorktreeFixture {
                id: "worktree-2".into(),
                branch: Some("main".into()),
                git_state: None,
                dirty_files: 0,
                conflicts: 0,
                ahead: 0,
                behind: 0,
            }],
        });
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("multi-repository fixture should mount");
        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);
        let before = front_door
            .projection(cx)
            .visible_projection()
            .expect("visible workbench before selection");
        let load = front_door
            .begin_surface_load(
                "identity-switch-load",
                omega_workbench_state::WorkSurface::Git,
                cx,
            )
            .expect("begin old repository load");

        front_door
            .select_identity_picker_row(1, cx)
            .expect("choose the second repository through the rendered picker");

        let after = front_door
            .projection(cx)
            .visible_projection()
            .expect("visible workbench after selection");
        let identity = front_door.identity(cx).expect("selected identity");
        assert_ne!(after.binding, before.binding);
        assert_eq!(after.generation, before.generation + 1);
        assert_eq!(
            identity
                .selected
                .as_ref()
                .map(|candidate| candidate.binding.clone()),
            after.binding,
            "the picker, header, and downstream routing must commit one binding"
        );
        assert_eq!(
            front_door
                .complete_surface_load(load, crate::workbench_shell::SurfaceLoadOutcome::Ready, cx,)
                .expect("complete captured old load"),
            omega_workbench_state::TransitionEffect::StaleCompletionIgnored
        );

        front_door
            .teardown(cx)
            .expect("multi-repository scene should tear down");
    }

    #[gpui::test]
    async fn rejected_agent_retarget_leaves_binding_generation_and_pending_load_unchanged(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("thread_identity_retarget_rejected", 1200, true);
        scene.repositories.push(RepositoryFixture {
            id: "repository-2".into(),
            project_id: "project-1".into(),
            worktrees: vec![WorktreeFixture {
                id: "worktree-2".into(),
                branch: Some("main".into()),
                git_state: None,
                dirty_files: 0,
                conflicts: 0,
                ahead: 0,
                behind: 0,
            }],
        });
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("retarget rejection fixture should mount");
        front_door.open_external_thread(
            StubAgentConnection::new()
                .with_work_dir_update_error("server rejected working directory"),
            cx,
        );
        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);
        let before = front_door
            .projection(cx)
            .visible_projection()
            .expect("visible workbench before rejected selection");
        let load = front_door
            .begin_surface_load(
                "retarget-rejection-load",
                omega_workbench_state::WorkSurface::Git,
                cx,
            )
            .expect("begin load under the original binding");

        front_door
            .select_identity_picker_row(1, cx)
            .expect("invoke the second repository through the rendered picker");

        let after = front_door
            .projection(cx)
            .visible_projection()
            .expect("visible workbench after rejected selection");
        assert_eq!(after.binding, before.binding);
        assert_eq!(after.generation, before.generation);
        assert!(matches!(
            front_door.identity(cx).map(|identity| identity.phase),
            Some(crate::thread_identity::IdentityPhase::Error(_))
        ));
        assert_eq!(
            front_door
                .complete_surface_load(load, crate::workbench_shell::SurfaceLoadOutcome::Ready, cx)
                .expect("original-target load should remain valid"),
            omega_workbench_state::TransitionEffect::Applied
        );

        front_door
            .teardown(cx)
            .expect("retarget rejection scene should tear down");
    }

    #[gpui::test]
    async fn reselecting_target_reconciles_inconsistent_sessions_and_advances_epoch(
        cx: &mut TestAppContext,
    ) {
        let scene = scene_with_thread("thread_identity_reconcile_inconsistent", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("inconsistent reconciliation fixture should mount");
        front_door.open_external_thread(StubAgentConnection::new(), cx);
        let before = front_door
            .projection(cx)
            .visible_projection()
            .expect("projection before inconsistency");
        front_door
            .mark_identity_inconsistent("Thread working directories disagree", cx)
            .expect("inject the typed result of a failed rollback");

        let inconsistent = front_door.identity(cx).expect("inconsistent identity");
        assert_eq!(
            inconsistent.phase,
            crate::thread_identity::IdentityPhase::Inconsistent(
                "Thread working directories disagree".into()
            )
        );
        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_interactive(WORKBENCH_REPOSITORY_SELECTOR)
            .expect("target selection is the explicit reconciliation action");
        for surface in [
            omega_workbench_state::WorkSurface::Files,
            omega_workbench_state::WorkSurface::Search,
            omega_workbench_state::WorkSurface::Review,
            omega_workbench_state::WorkSurface::Git,
            omega_workbench_state::WorkSurface::Terminal,
        ] {
            assert!(
                !front_door
                    .capability(surface, cx)
                    .expect("registered repository-bound capability")
                    .availability
                    .is_available(),
                "{surface:?} must remain disabled until reconciliation succeeds"
            );
        }

        front_door
            .select_identity_picker_row(0, cx)
            .expect("reselect the authoritative target");
        assert_eq!(
            front_door.identity(cx).map(|identity| identity.phase),
            Some(crate::thread_identity::IdentityPhase::Ready)
        );
        let after = front_door
            .projection(cx)
            .visible_projection()
            .expect("projection after reconciliation");
        assert_eq!(after.binding, before.binding);
        assert_eq!(after.generation, before.generation + 1);
        assert!(
            front_door
                .capability(omega_workbench_state::WorkSurface::Git, cx)
                .expect("Git capability after reconciliation")
                .availability
                .is_available()
        );

        front_door
            .teardown(cx)
            .expect("inconsistent reconciliation workbench should tear down");
    }

    #[gpui::test]
    async fn partial_retarget_with_failed_rollback_projects_inconsistent_authority(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("thread_identity_partial_rollback", 1200, true);
        scene.repositories.push(RepositoryFixture {
            id: "repository-2".into(),
            project_id: "project-1".into(),
            worktrees: vec![WorktreeFixture {
                id: "worktree-2".into(),
                branch: Some("main".into()),
                git_state: None,
                dirty_files: 0,
                conflicts: 0,
                ahead: 0,
                behind: 0,
            }],
        });
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("partial rollback fixture should mount");
        let first_connection = StubAgentConnection::new();
        first_connection.set_work_dir_update_results(vec![
            None,
            Some("first session rejected rollback".into()),
        ]);
        front_door.open_external_thread(first_connection, cx);
        let before = front_door
            .projection(cx)
            .visible_projection()
            .expect("projection before partial retarget");

        let second_connection = StubAgentConnection::new();
        second_connection
            .set_work_dir_update_results(vec![Some("second session rejected target".into())]);
        let conversation_view = front_door
            .panel
            .read_with(cx, |panel, _| panel.active_conversation_view().cloned())
            .expect("active conversation");
        let project = front_door
            .workspace
            .read_with(cx, |workspace, _| workspace.project().clone());
        let work_dirs = conversation_view.read_with(cx, |conversation_view, _| {
            conversation_view.work_dirs().clone()
        });
        let second_connection: Rc<dyn AgentConnection> = Rc::new(second_connection);
        let mut visual = VisualTestContext::from_window(front_door.window, cx);
        let second_thread = visual
            .update(|_window, cx| {
                second_connection
                    .clone()
                    .new_session(project, work_dirs, cx)
            })
            .await
            .expect("second session should open");
        conversation_view
            .update(&mut visual, |conversation_view, cx| {
                conversation_view.register_acp_thread_for_tests(second_thread, cx)
            })
            .expect("register second session");
        visual.run_until_parked();
        drop(visual);

        front_door
            .select_identity_picker_row(1, cx)
            .expect("retarget both sessions through the rendered picker");
        let identity = front_door.identity(cx).expect("inconsistent identity");
        let crate::thread_identity::IdentityPhase::Inconsistent(message) = identity.phase else {
            panic!("partial rollback must project an inconsistent identity phase");
        };
        assert!(message.contains("second session rejected target"));
        assert!(message.contains("first session rejected rollback"));
        let after = front_door
            .projection(cx)
            .visible_projection()
            .expect("projection after partial rollback");
        assert_eq!(after.binding, before.binding);
        assert_eq!(after.generation, before.generation);
        assert!(
            !front_door
                .capability(omega_workbench_state::WorkSurface::Terminal, cx)
                .expect("Terminal capability")
                .availability
                .is_available(),
            "repository-bound actions must stop when sessions disagree"
        );

        front_door
            .teardown(cx)
            .expect("partial rollback workbench should tear down");
    }

    #[gpui::test]
    async fn generating_thread_disables_picker_button_and_keyboard_action(cx: &mut TestAppContext) {
        let mut scene = scene_with_thread("thread_identity_busy_retarget", 1200, true);
        scene.repositories.push(RepositoryFixture {
            id: "repository-2".into(),
            project_id: "project-1".into(),
            worktrees: vec![WorktreeFixture {
                id: "worktree-2".into(),
                branch: Some("main".into()),
                git_state: None,
                dirty_files: 0,
                conflicts: 0,
                ahead: 0,
                behind: 0,
            }],
        });
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("busy retarget fixture should mount");
        front_door.open_external_thread(StubAgentConnection::new(), cx);
        let before = front_door
            .projection(cx)
            .visible_projection()
            .expect("visible workbench before the active turn");
        let mut visual = VisualTestContext::from_window(front_door.window, cx);
        send_message(&front_door.panel, &mut visual);

        let snapshot = visual.debug_render_snapshot();
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_accessibility_property(
                WORKBENCH_REPOSITORY_SELECTOR,
                "disabled",
                serde_json::Value::Bool(true),
            )
            .expect("repository picker should be disabled during a turn");
        probe
            .require_accessibility_property(
                WORKBENCH_BRANCH_SELECTOR,
                "disabled",
                serde_json::Value::Bool(true),
            )
            .expect("branch picker should be disabled during a turn");
        visual.dispatch_action(crate::workbench_shell::ToggleRepositoryPicker);
        visual.dispatch_action(crate::workbench_shell::ToggleBranchPicker);
        visual.run_until_parked();
        assert!(
            !accessibility_expanded(
                &visual.debug_render_snapshot(),
                WORKBENCH_REPOSITORY_SELECTOR,
            )
            .expect("read repository picker expansion"),
            "keyboard action must enforce the same unavailable predicate as the button"
        );
        assert!(
            !accessibility_expanded(&visual.debug_render_snapshot(), WORKBENCH_BRANCH_SELECTOR,)
                .expect("read branch picker expansion"),
            "branch keyboard action must enforce the same busy predicate as the button"
        );
        let after = front_door
            .projection(cx)
            .visible_projection()
            .expect("visible workbench during the active turn");
        assert_eq!(after.binding, before.binding);
        assert_eq!(after.generation, before.generation);

        drop(visual);
        front_door
            .teardown(cx)
            .expect("busy retarget scene should tear down");
    }

    #[gpui::test(iterations = 16)]
    async fn branch_picker_opened_while_idle_cannot_checkout_during_a_turn(
        cx: &mut TestAppContext,
    ) {
        let scene = scene_with_thread("thread_identity_busy_branch", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("busy branch fixture should mount");
        front_door.open_external_thread(StubAgentConnection::new(), cx);
        let mut visual = VisualTestContext::from_window(front_door.window, cx);
        visual
            .simulate_click_selector(WORKBENCH_BRANCH_SELECTOR)
            .expect("open branch picker while the thread is idle");
        visual.run_until_parked();
        let stale_branch_menu = front_door
            .panel
            .read_with(&visual, |panel, _cx| {
                panel.workbench_branch_menu_for_tests()
            })
            .expect("idle branch picker should be deployed");
        send_message(&front_door.panel, &mut visual);

        stale_branch_menu.update_in(&mut visual, |branch_menu, window, cx| {
            branch_menu.focus_handle(cx).focus(window, cx);
            window.dispatch_action(menu::SelectNext.boxed_clone(), cx);
            window.dispatch_action(menu::Confirm.boxed_clone(), cx);
        });
        visual.run_until_parked();

        assert_eq!(
            front_door
                .identity(cx)
                .and_then(|identity| identity.selected)
                .map(|selected| selected.branch),
            Some(crate::thread_identity::BranchIdentity::Branch(
                "main".into()
            )),
            "confirm-time busy validation must reject a stale branch menu"
        );

        front_door
            .teardown(cx)
            .expect("busy branch scene should tear down");
    }

    #[gpui::test(iterations = 16)]
    async fn pending_branch_checkout_blocks_prompts_and_identity_mutations(
        cx: &mut TestAppContext,
    ) {
        let scene = scene_with_thread("thread_identity_pending_branch", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("pending branch fixture should mount");
        front_door.open_external_thread(StubAgentConnection::new(), cx);
        let worktree_path = front_door
            .identity(cx)
            .and_then(|identity| identity.selected)
            .expect("selected identity")
            .worktree_abs_path;
        front_door.fs.set_simulated_change_branch_delay(
            &worktree_path.join(".git"),
            Some(Duration::from_secs(1)),
        );
        let mut visual = VisualTestContext::from_window(front_door.window, cx);
        let thread_view = front_door
            .panel
            .read_with(&visual, |panel, cx| {
                panel
                    .active_conversation_view()
                    .and_then(|view| view.read(cx).root_thread_view())
            })
            .expect("active thread view");
        let message_editor =
            thread_view.read_with(&visual, |thread_view, _| thread_view.message_editor.clone());
        message_editor.update_in(&mut visual, |message_editor, window, cx| {
            message_editor.set_text("prompt held during checkout", window, cx);
        });

        visual
            .simulate_click_selector(WORKBENCH_BRANCH_SELECTOR)
            .expect("open branch picker");
        visual.dispatch_action(menu::SelectNext);
        visual.dispatch_action(menu::Confirm);

        let pending_snapshot = visual.debug_render_snapshot();
        let mut probe = SemanticProbe::new(&pending_snapshot);
        for selector in [
            WORKBENCH_REPOSITORY_SELECTOR,
            WORKBENCH_WORKTREE_SELECTOR,
            WORKBENCH_BRANCH_SELECTOR,
        ] {
            probe
                .require_accessibility_property(selector, "disabled", serde_json::Value::Bool(true))
                .expect("branch checkout must gate every identity mutation");
        }
        thread_view.update_in(&mut visual, |thread_view, window, cx| {
            thread_view.send(window, cx);
        });
        assert_eq!(
            thread_view.read_with(&visual, |thread_view, cx| {
                thread_view.thread.read(cx).status()
            }),
            acp_thread::ThreadStatus::Idle,
            "a prompt must not start while branch checkout is pending"
        );
        assert_eq!(
            message_editor.read_with(&visual, |message_editor, cx| message_editor.text(cx)),
            "prompt held during checkout",
            "the blocked prompt must remain in the composer"
        );

        visual.executor().advance_clock(Duration::from_secs(1));
        visual.run_until_parked();
        front_door
            .fs
            .set_simulated_change_branch_delay(&worktree_path.join(".git"), None);
        assert_eq!(
            front_door.identity(&visual).map(|identity| identity.phase),
            Some(crate::thread_identity::IdentityPhase::Ready)
        );
        thread_view.update_in(&mut visual, |thread_view, window, cx| {
            thread_view.send(window, cx);
        });
        visual.run_until_parked();
        assert_ne!(
            thread_view.read_with(&visual, |thread_view, cx| {
                thread_view.thread.read(cx).status()
            }),
            acp_thread::ThreadStatus::Idle,
            "the held prompt should send after checkout releases its gate"
        );

        drop(visual);
        front_door
            .teardown(cx)
            .expect("pending branch workbench should tear down");
    }

    #[gpui::test]
    async fn picker_opened_for_another_thread_cannot_retarget_the_active_thread(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("thread_identity_stale_picker", 1200, true);
        scene.repositories.push(RepositoryFixture {
            id: "repository-2".into(),
            project_id: "project-1".into(),
            worktrees: vec![WorktreeFixture {
                id: "worktree-2".into(),
                branch: Some("main".into()),
                git_state: None,
                dirty_files: 0,
                conflicts: 0,
                ahead: 0,
                behind: 0,
            }],
        });
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("stale picker fixture should mount");
        let stale_menu = {
            let mut visual = VisualTestContext::from_window(front_door.window, cx);
            visual
                .simulate_click_selector(WORKBENCH_REPOSITORY_SELECTOR)
                .expect("open repository picker for thread A");
            visual.run_until_parked();
            front_door
                .panel
                .read_with(&visual, |panel, _cx| {
                    panel.workbench_repository_menu_for_tests()
                })
                .expect("thread A repository menu")
        };

        front_door.open_external_thread(StubAgentConnection::new(), cx);
        let before = front_door
            .projection(cx)
            .visible_projection()
            .expect("thread B projection before stale action");
        let mut visual = VisualTestContext::from_window(front_door.window, cx);
        stale_menu.update_in(&mut visual, |menu, window, cx| {
            menu.select_first(&menu::SelectFirst, window, cx);
            menu.select_next(&menu::SelectNext, window, cx);
            menu.confirm(&menu::Confirm, window, cx);
        });
        visual.run_until_parked();

        let after = front_door
            .projection(cx)
            .visible_projection()
            .expect("thread B projection after stale action");
        assert_eq!(after.thread_id, before.thread_id);
        assert_eq!(after.binding, before.binding);
        assert_eq!(after.generation, before.generation);

        front_door
            .teardown(cx)
            .expect("stale picker scene should tear down");
    }

    #[gpui::test]
    async fn git_indicators_and_git_rail_badge_share_one_summary(cx: &mut TestAppContext) {
        let mut scene = scene_with_thread("thread_identity_git_summary", 1200, true);
        let worktree = &mut scene.repositories[0].worktrees[0];
        worktree.dirty_files = 3;
        worktree.conflicts = 1;
        worktree.ahead = 2;
        worktree.behind = 1;
        scene
            .surfaces
            .iter_mut()
            .find(|surface| surface.id == WorkSurfaceId::Git)
            .expect("Git surface fixture")
            .badge = Some(3);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("Git summary fixture should mount");

        let git = front_door
            .identity(cx)
            .and_then(|identity| identity.selected)
            .expect("selected identity")
            .git;
        assert_eq!(git.dirty_files, 3);
        assert_eq!(git.conflicts, 1);
        assert_eq!(git.ahead, 2);
        assert_eq!(git.behind, 1);
        let capability = front_door
            .capability(omega_workbench_state::WorkSurface::Git, cx)
            .expect("Git capability");
        assert_eq!(
            capability.badge,
            Some(crate::workbench_shell::SurfaceBadge::Attention {
                tone: crate::workbench_shell::BadgeTone::Error,
                label: "3 changed, 1 conflicted, 2 ahead, 1 behind".into(),
            })
        );

        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        for selector in [
            "omega.workbench.identity.indicator.dirty",
            "omega.workbench.identity.indicator.conflict",
            "omega.workbench.identity.indicator.ahead",
            "omega.workbench.identity.indicator.behind",
            WorkSurfaceId::Git.badge_selector(),
        ] {
            probe
                .require_visible(selector)
                .unwrap_or_else(|error| panic!("{selector} should be rendered: {error:#}"));
        }

        front_door
            .teardown(cx)
            .expect("Git summary scene should tear down");
    }

    #[gpui::test]
    async fn ahead_only_git_badge_reports_commits_instead_of_zero_files(cx: &mut TestAppContext) {
        let mut scene = scene_with_thread("thread_identity_ahead_only", 1200, true);
        let worktree = &mut scene.repositories[0].worktrees[0];
        worktree.ahead = 2;
        worktree.behind = 1;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("ahead-only fixture should mount");

        assert_eq!(
            front_door
                .capability(omega_workbench_state::WorkSurface::Git, cx)
                .expect("Git capability")
                .badge,
            Some(crate::workbench_shell::SurfaceBadge::Count {
                count: 3,
                tone: crate::workbench_shell::BadgeTone::Warning,
                label: "0 changed, 0 conflicted, 2 ahead, 1 behind".into(),
            })
        );

        front_door
            .teardown(cx)
            .expect("ahead-only scene should tear down");
    }

    #[gpui::test]
    async fn removed_selected_worktree_stays_missing_and_falls_back(cx: &mut TestAppContext) {
        let mut scene = scene_with_thread("thread_identity_removed", 1200, true);
        scene.repositories.push(RepositoryFixture {
            id: "repository-2".into(),
            project_id: "project-1".into(),
            worktrees: vec![WorktreeFixture {
                id: "worktree-2".into(),
                branch: Some("main".into()),
                git_state: None,
                dirty_files: 0,
                conflicts: 0,
                ahead: 0,
                behind: 0,
            }],
        });
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("removal fixture should mount");
        front_door
            .select_identity_picker_row(1, cx)
            .expect("select second worktree");
        let selected = front_door
            .identity(cx)
            .and_then(|identity| identity.selected)
            .expect("selected identity before removal");
        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);

        front_door
            .remove_worktree(&selected.worktree_abs_path, cx)
            .expect("remove selected worktree");

        let identity = front_door.identity(cx).expect("missing identity state");
        assert_eq!(
            identity.phase,
            crate::thread_identity::IdentityPhase::Missing
        );
        assert_eq!(
            identity
                .selected
                .as_ref()
                .map(|candidate| &candidate.binding),
            Some(&selected.binding),
            "the missing state should retain the last-known label and identity"
        );
        assert_eq!(identity.binding(), None);
        let missing_selection_revision = identity.selection_revision;
        let visible = front_door
            .projection(cx)
            .visible_projection()
            .expect("visible workbench after removal");
        assert_eq!(visible.binding, None);
        assert_eq!(
            visible.effective_surface,
            Some(omega_workbench_state::WorkSurface::Plan)
        );
        assert!(
            !visible.dock_open,
            "incompatible selected surface should collapse after removal"
        );
        assert_eq!(
            front_door
                .capability(omega_workbench_state::WorkSurface::Git, cx)
                .and_then(|capability| capability.availability.reason().cloned()),
            Some("The selected worktree is missing".into())
        );
        let missing_generation = visible.generation;
        let snapshot = front_door.snapshot(cx);
        let identity_after_render = front_door
            .identity(cx)
            .expect("missing identity should survive rendering");
        assert_eq!(
            identity_after_render.phase,
            crate::thread_identity::IdentityPhase::Missing
        );
        front_door.snapshot(cx);
        assert_eq!(
            front_door
                .identity(cx)
                .expect("missing identity after repeated render")
                .selection_revision,
            missing_selection_revision,
            "unchanged Missing observations must not manufacture selection revisions"
        );
        assert!(
            identity_after_render.selected.is_some(),
            "missing identity should keep its last-known selected label"
        );
        assert_eq!(
            front_door
                .projection(cx)
                .visible_projection()
                .and_then(|visible| visible.binding),
            None
        );
        SemanticProbe::new(&snapshot)
            .require_interactive(WORKBENCH_REPOSITORY_SELECTOR)
            .expect("the retained identity must expose the repository picker for recovery");

        front_door
            .select_identity_picker_row(0, cx)
            .expect("recover the missing identity through the rendered repository picker");
        let recovered_identity = front_door.identity(cx).expect("recovered identity state");
        assert_eq!(
            recovered_identity.phase,
            crate::thread_identity::IdentityPhase::Ready
        );
        let recovered = recovered_identity
            .selected
            .expect("replacement identity should be selected");
        assert_ne!(recovered.binding, selected.binding);
        let recovered_visible = front_door
            .projection(cx)
            .visible_projection()
            .expect("visible workbench after recovery");
        assert_eq!(recovered_visible.binding, Some(recovered.binding));
        assert_eq!(recovered_visible.generation, missing_generation + 1);

        front_door
            .teardown(cx)
            .expect("removed-worktree scene should tear down");
    }

    #[gpui::test]
    async fn failed_missing_recovery_is_visible_without_reviving_removed_binding(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("thread_identity_failed_missing_recovery", 1200, true);
        scene.repositories.push(RepositoryFixture {
            id: "repository-2".into(),
            project_id: "project-1".into(),
            worktrees: vec![WorktreeFixture {
                id: "worktree-2".into(),
                branch: Some("main".into()),
                git_state: None,
                dirty_files: 0,
                conflicts: 0,
                ahead: 0,
                behind: 0,
            }],
        });
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("failed recovery fixture should mount");
        let connection = StubAgentConnection::new();
        front_door.open_external_thread(connection.clone(), cx);
        front_door
            .select_identity_picker_row(1, cx)
            .expect("select the worktree that will disappear");
        let removed = front_door
            .identity(cx)
            .and_then(|identity| identity.selected)
            .expect("selected identity before removal");
        front_door
            .remove_worktree(&removed.worktree_abs_path, cx)
            .expect("remove selected worktree");
        let missing = front_door
            .projection(cx)
            .visible_projection()
            .expect("missing projection");
        assert_eq!(missing.binding, None);

        connection.set_work_dir_update_error("server rejected replacement worktree");
        front_door
            .select_identity_picker_row(0, cx)
            .expect("attempt recovery through the rendered picker");

        let identity = front_door.identity(cx).expect("failed recovery identity");
        assert_eq!(
            identity.phase,
            crate::thread_identity::IdentityPhase::Error(
                "server rejected replacement worktree".into()
            )
        );
        assert_eq!(identity.binding(), None);
        let visible = front_door
            .projection(cx)
            .visible_projection()
            .expect("projection after failed recovery");
        assert_eq!(visible.binding, None);
        assert_eq!(visible.generation, missing.generation);
        assert_eq!(
            front_door
                .projection(cx)
                .threads
                .get(&visible.thread_id)
                .map(|thread| thread.available_surfaces.clone()),
            Some(std::collections::BTreeSet::from([
                omega_workbench_state::WorkSurface::Plan
            ]))
        );
        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_accessible(
                "omega.workbench.identity.status",
                "Status",
                "server rejected replacement worktree",
            )
            .expect("failed recovery should be announced");
        probe
            .require_interactive(WORKBENCH_REPOSITORY_SELECTOR)
            .expect("failed recovery must remain retryable");
        probe
            .require_absent("omega-workbench-rail-error")
            .expect("failed recovery should be a typed identity phase, not a shell sync error");
        connection.clear_work_dir_update_error();

        front_door
            .teardown(cx)
            .expect("failed recovery workbench should tear down");
    }

    #[gpui::test]
    async fn identity_connection_and_error_phases_are_typed_and_accessible(
        cx: &mut TestAppContext,
    ) {
        let scene = scene_with_thread("thread_identity_phases", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("identity phase fixture should mount");
        let phases = [
            crate::thread_identity::IdentityPhase::Loading,
            crate::thread_identity::IdentityPhase::Stale,
            crate::thread_identity::IdentityPhase::Offline,
            crate::thread_identity::IdentityPhase::Reconnecting,
            crate::thread_identity::IdentityPhase::Error("Branch operation failed".into()),
            crate::thread_identity::IdentityPhase::Inconsistent("Thread worktrees disagree".into()),
        ];

        for phase in phases {
            let expected_label = phase.label().expect("non-ready phase label");
            front_door.set_identity_phase(phase.clone(), cx);
            assert_eq!(
                front_door.identity(cx).map(|identity| identity.phase),
                Some(phase.clone())
            );
            let snapshot = front_door.snapshot(cx);
            let mut probe = SemanticProbe::new(&snapshot);
            probe
                .require_accessible(
                    "omega.workbench.identity.status",
                    "Status",
                    expected_label.as_ref(),
                )
                .unwrap_or_else(|error| panic!("{phase:?} should expose a status node: {error:#}"));
            let target_selection_disabled = matches!(
                &phase,
                crate::thread_identity::IdentityPhase::Loading
                    | crate::thread_identity::IdentityPhase::Stale
                    | crate::thread_identity::IdentityPhase::Offline
                    | crate::thread_identity::IdentityPhase::Reconnecting
            );
            probe
                .require_accessibility_property(
                    WORKBENCH_REPOSITORY_SELECTOR,
                    "disabled",
                    serde_json::Value::Bool(target_selection_disabled),
                )
                .unwrap_or_else(|error| {
                    panic!("{phase:?} repository picker availability is wrong: {error:#}")
                });
            if target_selection_disabled {
                front_door.dispatch_action(crate::workbench_shell::ToggleRepositoryPicker, cx);
                assert!(
                    !accessibility_expanded(
                        &front_door.snapshot(cx),
                        WORKBENCH_REPOSITORY_SELECTOR
                    )
                    .expect("read repository picker expansion"),
                    "{phase:?} keyboard action must not bypass disabled target selection"
                );
            }
            if matches!(
                &phase,
                crate::thread_identity::IdentityPhase::Stale
                    | crate::thread_identity::IdentityPhase::Offline
                    | crate::thread_identity::IdentityPhase::Reconnecting
                    | crate::thread_identity::IdentityPhase::Inconsistent(_)
            ) {
                assert!(
                    !front_door
                        .capability(omega_workbench_state::WorkSurface::Git, cx)
                        .expect("Git capability")
                        .availability
                        .is_available(),
                    "{phase:?} must disable repository-bound actions"
                );
                probe
                    .require_accessibility_property(
                        WorkSurfaceId::Plan.rail_selector(),
                        "disabled",
                        serde_json::Value::Bool(false),
                    )
                    .unwrap_or_else(|error| {
                        panic!(
                            "{phase:?} must keep the thread-local Plan surface enabled: {error:#}"
                        )
                    });
            }
            if matches!(
                &phase,
                crate::thread_identity::IdentityPhase::Inconsistent(_)
            ) {
                probe
                    .require_accessibility_property(
                        WORKBENCH_BRANCH_SELECTOR,
                        "disabled",
                        serde_json::Value::Bool(true),
                    )
                    .unwrap_or_else(|error| {
                        panic!(
                            "{phase:?} must keep branch selection blocked until reconciliation: {error:#}"
                        )
                    });
            }
            let expected_connection = match &phase {
                crate::thread_identity::IdentityPhase::Stale => {
                    omega_workbench_state::ConnectionPhase::StaleProjection
                }
                crate::thread_identity::IdentityPhase::Offline => {
                    omega_workbench_state::ConnectionPhase::Offline
                }
                crate::thread_identity::IdentityPhase::Reconnecting => {
                    omega_workbench_state::ConnectionPhase::Reconnecting
                }
                _ => omega_workbench_state::ConnectionPhase::Online,
            };
            assert_eq!(
                front_door.projection(cx).connection,
                expected_connection,
                "identity and workbench projection connection phases must agree"
            );
        }

        front_door
            .teardown(cx)
            .expect("identity phase scene should tear down");
    }

    #[gpui::test]
    async fn branch_picker_failure_projects_a_typed_header_error(cx: &mut TestAppContext) {
        let scene = scene_with_thread("thread_identity_branch_error", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("branch error fixture should mount");
        let worktree_path = front_door
            .identity(cx)
            .and_then(|identity| identity.selected)
            .expect("selected identity")
            .worktree_abs_path;

        front_door
            .fail_next_branch_selection(&worktree_path, "simulated branch checkout failure", cx)
            .expect("exercise the rendered branch picker");

        let identity = front_door.identity(cx).expect("identity after failure");
        assert_eq!(
            identity.phase,
            crate::thread_identity::IdentityPhase::Error(
                "simulated branch checkout failure".into()
            )
        );
        assert_eq!(
            identity
                .selected
                .as_ref()
                .map(|candidate| &candidate.branch),
            Some(&crate::thread_identity::BranchIdentity::Branch(
                "main".into()
            )),
            "failed checkout must preserve the last confirmed branch"
        );
        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_accessible(
                "omega.workbench.identity.status",
                "Status",
                "simulated branch checkout failure",
            )
            .expect("branch failure should be announced in the header");
        probe
            .require_interactive(WORKBENCH_BRANCH_SELECTOR)
            .expect("a target-scoped branch failure must leave the branch picker retryable");

        front_door
            .teardown(cx)
            .expect("branch error scene should tear down");
    }

    #[gpui::test(iterations = 16)]
    async fn branch_checkout_advances_binding_epoch_and_stales_prior_loads(
        cx: &mut TestAppContext,
    ) {
        let scene = scene_with_thread("thread_identity_branch_epoch", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("branch epoch fixture should mount");
        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);
        let before = front_door
            .projection(cx)
            .visible_projection()
            .expect("visible workbench before checkout");
        let load = front_door
            .begin_surface_load(
                "branch-checkout-load",
                omega_workbench_state::WorkSurface::Git,
                cx,
            )
            .expect("begin Git load before checkout");

        front_door
            .select_next_branch(cx)
            .expect("checkout the next branch through the rendered picker");

        let after = front_door
            .projection(cx)
            .visible_projection()
            .expect("visible workbench after checkout");
        assert_eq!(after.binding, before.binding);
        assert_eq!(after.generation, before.generation + 1);
        let host = front_door
            .visible_surface_host(cx)
            .expect("branch refresh should retain the visible Git host");
        assert_eq!(
            host.read_with(cx, |host, _| host.content_state().clone()),
            crate::workbench_shell::SurfaceContentState::Ready,
            "a content epoch refresh must not strand the retained host in Loading"
        );
        assert_eq!(
            front_door
                .identity(cx)
                .and_then(|identity| identity.selected)
                .map(|selected| selected.branch),
            Some(crate::thread_identity::BranchIdentity::Branch(
                "release".into()
            ))
        );
        assert_eq!(
            front_door
                .complete_surface_load(load, crate::workbench_shell::SurfaceLoadOutcome::Ready, cx)
                .expect("complete the old branch load"),
            omega_workbench_state::TransitionEffect::StaleCompletionIgnored
        );
        assert_eq!(
            host.read_with(cx, |host, _| host.content_state().clone()),
            crate::workbench_shell::SurfaceContentState::Ready
        );

        front_door
            .teardown(cx)
            .expect("branch epoch scene should tear down");
    }

    #[gpui::test]
    async fn workbench_rail_wide_selects_every_surface_and_retains_transcript(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("workbench_rail_wide", 1440, true);
        if let Some(git) = scene
            .surfaces
            .iter_mut()
            .find(|surface| surface.id == WorkSurfaceId::Git)
        {
            git.badge = Some(3);
        }
        scene.repositories[0].worktrees[0].dirty_files = 3;
        let front_door = AgentWorkbenchFrontDoor::mount(scene.clone(), cx)
            .await
            .expect("wide workbench fixture should mount");
        let transcript_id = front_door
            .transcript_entity_id(cx)
            .expect("wide scene should have a transcript entity");

        for surface in WorkSurfaceId::ALL {
            front_door
                .click(surface.rail_selector(), cx)
                .expect("rail item should activate");
            let mut expected = scene.clone();
            expected.active_surface = Some(surface);
            expected.dock_open = true;
            let snapshot = front_door.snapshot(cx);
            if let Err(error) = AgentWorkbenchFrontDoor::prove_scene(&expected, &snapshot) {
                panic!(
                    "{surface:?} should satisfy rendered semantics: {error:#}; root={:?}, dock={:?}, transcript={:?}, composer={:?}",
                    snapshot.bounds(WORKBENCH_ROOT_SELECTOR),
                    snapshot.bounds(WORKBENCH_DOCK_SELECTOR),
                    snapshot.bounds(WORKBENCH_TRANSCRIPT_SELECTOR),
                    snapshot.bounds(WORKBENCH_COMPOSER_SELECTOR),
                );
            }
            assert_eq!(
                front_door.transcript_entity_id(cx),
                Some(transcript_id),
                "selecting {surface:?} must not recreate the transcript"
            );
            assert!(
                front_door
                    .surface_host_entity_id(work_surface(surface), cx)
                    .is_some(),
                "{surface:?} should own one retained host"
            );
        }
        assert_eq!(
            front_door
                .panel()
                .read_with(cx, |panel, _| panel.workbench_host_count_for_tests()),
            WorkSurfaceId::ALL.len()
        );
        front_door
            .teardown(cx)
            .expect("wide workbench should tear down");
    }

    #[gpui::test]
    async fn workbench_active_item_collapses_and_reuses_host(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_reuse", 1200, false);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("reuse scene should mount");

        front_door
            .click(WorkSurfaceId::Plan.rail_selector(), cx)
            .expect("Plan should open");
        let host_id = front_door
            .surface_host_entity_id(omega_workbench_state::WorkSurface::Plan, cx)
            .expect("Plan host should exist");
        front_door
            .click(WorkSurfaceId::Plan.rail_selector(), cx)
            .expect("active Plan should collapse");
        assert!(
            front_door
                .snapshot(cx)
                .bounds(WORKBENCH_DOCK_SELECTOR)
                .is_none(),
            "collapsed dock should not render"
        );
        assert_eq!(
            front_door
                .projection(cx)
                .visible_projection()
                .and_then(|visible| visible.requested_surface),
            Some(omega_workbench_state::WorkSurface::Plan),
            "collapse should retain requested surface"
        );

        front_door
            .click(WorkSurfaceId::Plan.rail_selector(), cx)
            .expect("Plan should reopen");
        assert_eq!(
            front_door.surface_host_entity_id(omega_workbench_state::WorkSurface::Plan, cx),
            Some(host_id),
            "collapse and reopen should reuse the same host entity"
        );
        front_door
            .teardown(cx)
            .expect("reuse workbench should tear down");
    }

    #[gpui::test]
    async fn workbench_thread_switch_restores_independent_hosts(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_thread_switch", 1200, false);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("thread-switch scene should mount");

        let thread_a = front_door.open_external_thread(StubAgentConnection::new(), cx);
        front_door.dispatch_action(crate::workbench_shell::SelectPlan, cx);
        let host_a = front_door
            .surface_host_entity_id(omega_workbench_state::WorkSurface::Plan, cx)
            .expect("thread A should create a Plan host");

        let thread_b = front_door.open_external_thread(StubAgentConnection::new(), cx);
        assert_ne!(thread_a, thread_b);
        front_door.dispatch_action(crate::workbench_shell::SelectPlan, cx);
        let host_b = front_door
            .surface_host_entity_id(omega_workbench_state::WorkSurface::Plan, cx)
            .expect("thread B should create a Plan host");
        assert_ne!(host_a, host_b, "thread hosts must not be shared");

        front_door.activate_thread(thread_a, cx);
        assert_eq!(
            front_door.surface_host_entity_id(omega_workbench_state::WorkSurface::Plan, cx),
            Some(host_a),
            "thread A should restore its own retained selection and host"
        );
        front_door.activate_thread(thread_b, cx);
        assert_eq!(
            front_door.surface_host_entity_id(omega_workbench_state::WorkSurface::Plan, cx),
            Some(host_b),
            "thread B should restore its own retained selection and host"
        );

        front_door
            .teardown(cx)
            .expect("thread-switch workbench should tear down");
    }

    #[gpui::test(iterations = 16)]
    async fn workbench_keyboard_actions_route_focus(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_keyboard", 1200, false);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("keyboard scene should mount");

        front_door.dispatch_action(crate::workbench_shell::FocusActivityRail, cx);
        front_door.dispatch_action(crate::workbench_shell::FocusLastSurface, cx);
        let rail_snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&rail_snapshot);
        probe
            .require_focus(WorkSurfaceId::Plan.rail_selector(), true)
            .expect("Plan rail item should own keyboard focus");

        front_door.dispatch_action(crate::workbench_shell::ActivateFocusedSurface, cx);
        let surface_snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&surface_snapshot);
        probe
            .require_focus(WorkSurfaceId::Plan.surface_selector(), true)
            .expect("activation should transfer focus into the surface");

        front_door.dispatch_action(crate::workbench_shell::CollapseWorkSurfaceDock, cx);
        let collapsed = front_door.snapshot(cx);
        assert!(
            collapsed
                .bounds(WorkSurfaceId::Plan.surface_selector())
                .is_none(),
            "a focused surface must unmount after focus returns to the transcript"
        );
        front_door
            .teardown(cx)
            .expect("keyboard workbench should tear down");
    }

    #[gpui::test]
    async fn offline_plan_opens_collapses_and_reuses_one_host(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_offline_plan", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("offline Plan scene should mount");
        front_door.set_identity_phase(crate::thread_identity::IdentityPhase::Offline, cx);

        front_door.dispatch_action(crate::workbench_shell::SelectPlan, cx);
        let first_host = front_door
            .surface_host_entity_id(omega_workbench_state::WorkSurface::Plan, cx)
            .expect("offline Plan should mount");
        assert_eq!(
            front_door
                .panel()
                .read_with(cx, |panel, _| panel.workbench_host_count_for_tests()),
            1
        );
        front_door.dispatch_action(crate::workbench_shell::SelectPlan, cx);
        assert!(
            !front_door
                .projection(cx)
                .visible_projection()
                .expect("offline projection")
                .dock_open
        );
        front_door.dispatch_action(crate::workbench_shell::SelectPlan, cx);
        assert_eq!(
            front_door.surface_host_entity_id(omega_workbench_state::WorkSurface::Plan, cx),
            Some(first_host)
        );
        assert_eq!(
            front_door
                .panel()
                .read_with(cx, |panel, _| panel.workbench_host_count_for_tests()),
            1,
            "offline reopen must not manufacture a second host"
        );

        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);
        assert_eq!(
            front_door
                .panel()
                .read_with(cx, |panel, _| panel.workbench_host_count_for_tests()),
            1,
            "a rejected offline surface request must not orphan a host"
        );
        assert_eq!(
            front_door
                .projection(cx)
                .visible_projection()
                .and_then(|visible| visible.effective_surface),
            Some(omega_workbench_state::WorkSurface::Plan)
        );

        front_door
            .teardown(cx)
            .expect("offline Plan workbench should tear down");
    }

    #[gpui::test]
    async fn workbench_narrow_layout_collapses_without_covering_composer(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_narrow", 1200, false);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("narrow scene should mount");
        front_door
            .click(WorkSurfaceId::Plan.rail_selector(), cx)
            .expect("Plan should open before resize");
        let host_id = front_door
            .surface_host_entity_id(omega_workbench_state::WorkSurface::Plan, cx)
            .expect("Plan host should exist");

        front_door.resize(909, 720, cx);
        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_absent(WORKBENCH_DOCK_SELECTOR)
            .expect("dock should collapse one pixel below its shared boundary");
        probe
            .require_fully_visible(WORKBENCH_COMPOSER_SELECTOR)
            .expect("composer should remain fully visible");
        probe
            .require_inside(WORKBENCH_COMPOSER_SELECTOR, WORKBENCH_ROOT_SELECTOR)
            .expect("composer should stay inside the workbench");
        assert!(
            !front_door
                .projection(cx)
                .visible_projection()
                .is_some_and(|visible| visible.dock_open),
            "narrow suppression must update logical dock state"
        );

        front_door.resize(1200, 720, cx);
        front_door
            .click(WorkSurfaceId::Plan.rail_selector(), cx)
            .expect("Plan should explicitly reopen after widening");
        assert_eq!(
            front_door.surface_host_entity_id(omega_workbench_state::WorkSurface::Plan, cx),
            Some(host_id)
        );
        front_door
            .teardown(cx)
            .expect("narrow workbench should tear down");
    }

    #[gpui::test]
    async fn workbench_no_project_and_creation_failure_are_explicit(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_unavailable", 1200, false);
        let front_door = AgentWorkbenchFrontDoor::mount(scene.clone(), cx)
            .await
            .expect("unavailable scene should mount");
        let before = front_door.projection(cx);
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert_eq!(
            front_door.projection(cx),
            before,
            "unavailable action must not open a fallback"
        );

        front_door.fail_next_host_creation(omega_workbench_state::WorkSurface::Plan, cx);
        front_door.dispatch_action(crate::workbench_shell::SelectPlan, cx);
        let snapshot = front_door.snapshot(cx);
        assert!(
            snapshot.bounds("omega-workbench-rail-error").is_some(),
            "surface construction failure should be visible in the rail"
        );
        assert!(
            snapshot.bounds(WORKBENCH_DOCK_SELECTOR).is_none(),
            "failed construction must not open an empty dock"
        );
        assert_eq!(
            front_door
                .panel()
                .read_with(cx, |panel, _| panel.workbench_host_count_for_tests()),
            0
        );

        AgentWorkbenchFrontDoor::prove_scene(&scene, &snapshot)
            .expect("failure should not invalidate base rail semantics");
        front_door
            .teardown(cx)
            .expect("unavailable workbench should tear down");
    }

    #[gpui::test]
    async fn workbench_active_invalidation_ignores_stale_completion(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_stale_completion", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("stale-completion scene should mount");
        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);
        let host = front_door
            .visible_surface_host(cx)
            .expect("Git should have a visible host");
        let weak_host = host.downgrade();
        let error_load = front_door
            .begin_surface_load(
                "git-load-error",
                omega_workbench_state::WorkSurface::Git,
                cx,
            )
            .expect("visible Git should begin its error-state load");
        assert_eq!(
            front_door
                .complete_surface_load(
                    error_load,
                    crate::workbench_shell::SurfaceLoadOutcome::Error(
                        "Fixture Git load failed".into(),
                    ),
                    cx,
                )
                .expect("current load failure should be handled"),
            omega_workbench_state::TransitionEffect::Applied
        );
        assert_eq!(
            host.read_with(cx, |host, _| host.content_state().clone()),
            crate::workbench_shell::SurfaceContentState::Error("Fixture Git load failed".into())
        );
        let load = front_door
            .begin_surface_load(
                "git-load-before-invalidation",
                omega_workbench_state::WorkSurface::Git,
                cx,
            )
            .expect("visible Git should begin loading");
        assert_eq!(
            host.read_with(cx, |host, _| host.content_state().clone()),
            crate::workbench_shell::SurfaceContentState::Loading
        );
        drop(host);

        front_door
            .invalidate_surface(omega_workbench_state::WorkSurface::Git, cx)
            .expect("active Git capability should invalidate");
        assert_eq!(
            front_door
                .panel()
                .read_with(cx, |panel, _| panel.workbench_focus_target_for_tests()),
            crate::workbench_shell::WorkbenchFocusTarget::Transcript
        );
        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_absent(WORKBENCH_DOCK_SELECTOR)
            .expect("invalidated active surface should close the dock");
        probe
            .require_accessibility_property(
                WorkSurfaceId::Git.rail_selector(),
                "disabled",
                serde_json::Value::Bool(true),
            )
            .expect("invalidated Git should be disabled");

        assert_eq!(
            front_door
                .complete_surface_load(load, crate::workbench_shell::SurfaceLoadOutcome::Ready, cx,)
                .expect("stale completion should be a handled transition"),
            omega_workbench_state::TransitionEffect::StaleCompletionIgnored
        );
        assert!(
            weak_host.upgrade().is_none(),
            "the invalidated host should be released before its stale completion"
        );
        assert_eq!(
            front_door
                .panel()
                .read_with(cx, |panel, _| panel.workbench_host_count_for_tests()),
            0
        );

        front_door
            .teardown(cx)
            .expect("stale-completion workbench should tear down");
    }

    #[gpui::test]
    async fn workbench_surface_hosts_release_with_window(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_release", 1200, false);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("release scene should mount");
        front_door
            .click(WorkSurfaceId::Plan.rail_selector(), cx)
            .expect("Plan should open");
        let weak_host = front_door
            .visible_surface_host(cx)
            .expect("visible Plan host")
            .downgrade();

        front_door
            .teardown(cx)
            .expect("release workbench should tear down");
        assert!(
            weak_host.upgrade().is_none(),
            "surface host must not outlive its panel and window"
        );
    }
}
