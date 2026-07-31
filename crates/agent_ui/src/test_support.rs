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
use project::{AgentId, Project, ProjectPath, WorktreeId};
use project_panel::ProjectPanel;
use settings::SettingsStore;
use std::any::Any;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use terminal_view::terminal_panel::{
    TerminalPanel, TerminalPanelSnapshot, TestTerminalCreationRequest, TestTerminalInsertion,
    TestTerminalLifecycle,
};
use text::OffsetRangeExt as _;
use workspace::{
    Item as _, MultiWorkspace, Sidebar as WorkspaceSidebar, SidebarEvent, SidebarSide,
    SplitDirection, Workspace, dock::Panel as _,
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
        git_ui::init(cx);
        terminal_view::init(cx);
        release_channel::init("0.0.0".parse().unwrap(), cx);
        project_panel::init(cx);
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
    let conversation_view = panel.read_with(cx, |panel, _cx| {
        panel.active_conversation_view().unwrap().clone()
    });
    conversation_view.update_in(cx, |view, window, cx| {
        view.set_composer_text_for_tests("Hello", window, cx);
    });
    conversation_view.update_in(cx, |view, window, cx| {
        view.send_for_tests(window, cx);
    });
    cx.run_until_parked();
}

pub fn type_draft_prompt(panel: &Entity<AgentPanel>, text: &str, cx: &mut VisualTestContext) {
    let conversation_view = panel.read_with(cx, |panel, _cx| {
        panel.active_conversation_view().unwrap().clone()
    });
    conversation_view.update_in(cx, |view, window, cx| {
        view.set_composer_text_for_tests(text, window, cx);
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeTerminalFrontDoorSnapshot {
    pub panel: TerminalPanelSnapshot,
    pub surface: NativeTerminalSurfaceSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeTerminalSurfaceSnapshot {
    pub panel_entity_id: EntityId,
    pub binding: crate::workbench_shell::NativeTerminalBinding,
    pub owner_state: crate::workbench_shell::NativeTerminalOwnerState,
    pub terminal_owners:
        std::collections::BTreeMap<u64, crate::workbench_shell::NativeTerminalBinding>,
    pub active_terminal_owner: Option<(u64, crate::workbench_shell::NativeTerminalBinding)>,
}

pub struct TestWorkbenchTerminal {
    pub insertion: TestTerminalInsertion,
}

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
                fs.insert_tree(
                    &path,
                    serde_json::json!({
                        ".gitignore": "target/\n",
                        "src": {
                            "existing.rs": "// Existing fixture",
                            "main.rs": "// Main fixture",
                            "rename-me.rs": "// Rename fixture",
                            "nested": {
                                "deep": {
                                    "a-deliberately-long-fixture-file-name.rs": "// Long fixture"
                                }
                            }
                        },
                        "target": {
                            "ignored.rs": "// Ignored fixture"
                        }
                    }),
                )
                .await;
                fs.insert_file(
                    path.join(format!("{}-only.txt", worktree.id)),
                    worktree.id.as_bytes().to_vec(),
                )
                .await;
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

        let (weak_workspace, async_window_context) = workspace
            .update_in(&mut visual, |workspace, window, cx| {
                (workspace.weak_handle(), window.to_async(cx))
            });
        crate::initialize_workbench_panels(weak_workspace, async_window_context)
            .await
            .context("initializing the native workbench panels through the shipped path")?;
        visual.run_until_parked();

        let (panel, focused) = workspace.update_in(&mut visual, |workspace, window, cx| {
            if workspace.panel::<ProjectPanel>(cx).is_none()
                || workspace
                    .panel::<git_ui::git_panel::GitPanel>(cx)
                    .is_none()
                || workspace.panel::<TerminalPanel>(cx).is_none()
            {
                bail!(
                    "Files, Git, and Terminal must all exist before the test front door constructs AgentPanel"
                );
            }
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            panel.update(cx, |panel, cx| {
                panel.enable_workbench_shell_for_tests(cx);
                panel.set_zoomed(true, window, cx);
            });
            if scene.active_thread_id.is_some() {
                panel.update(cx, |panel, cx| {
                    panel.open_draft_with_server(
                        Rc::new(StubAgentServer::new(
                            StubAgentConnection::new().with_agent_id("workbench-fixture".into()),
                        ).with_connection_agent_id()),
                        window,
                        cx,
                    );
                });
            }
            workspace.add_panel(panel.clone(), window, cx);
            let focused = workspace.focus_panel::<AgentPanel>(window, cx).is_some();
            Ok((panel, focused))
        })?;
        if !focused {
            bail!("AgentPanel was not focusable after being mounted");
        }

        visual.set_debug_accessibility_active(true);

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

        let has_active_thread = panel.read_with(&visual, |panel, _cx| {
            panel.active_conversation_view().is_some()
        });
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

    pub fn focus_native_files(&self, cx: &TestAppContext) {
        if self.mounted_files_panel_entity_id(cx).is_some() {
            if let Some(panel) = self.native_files_panel(cx) {
                let mut visual = VisualTestContext::from_window(self.window, cx);
                panel.update_in(&mut visual, |panel, window, cx| {
                    panel.focus_handle(cx).focus(window, cx);
                });
            }
        }
    }

    pub fn dispatch_action(&self, action: impl Action, cx: &TestAppContext) {
        if action.name().starts_with("project_panel::") && !action.name().ends_with("ToggleFocus") {
            self.focus_native_files(cx);
        }
        let mut visual = VisualTestContext::from_window(self.window, cx);
        visual.dispatch_action(action);
        visual.run_until_parked();
    }

    pub fn settle(&self, cx: &TestAppContext) {
        let visual = VisualTestContext::from_window(self.window, cx);
        visual.run_until_parked();
    }

    pub fn simulate_keystrokes(&self, keystrokes: &str, cx: &TestAppContext) {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        visual.simulate_keystrokes(keystrokes);
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

    pub fn native_files_panel(&self, cx: &TestAppContext) -> Option<Entity<ProjectPanel>> {
        self.workspace
            .read_with(cx, |workspace, cx| workspace.panel::<ProjectPanel>(cx))
            .or_else(|| {
                self.panel
                    .read_with(cx, |panel, _cx| panel.workbench_files_panel_for_tests())
            })
    }

    pub fn native_files_panel_entity_id(&self, cx: &TestAppContext) -> Option<EntityId> {
        self.native_files_panel(cx).map(|panel| panel.entity_id())
    }

    pub fn mounted_files_panel_entity_id(&self, cx: &TestAppContext) -> Option<EntityId> {
        self.visible_surface_host(cx).and_then(|host| {
            host.read_with(cx, |host, _cx| {
                if matches!(
                    host.content_state(),
                    crate::workbench_shell::SurfaceContentState::Ready
                ) {
                    host.files_panel_entity_id()
                } else {
                    None
                }
            })
        })
    }

    pub fn native_search_surface(
        &self,
        cx: &TestAppContext,
    ) -> Option<Entity<crate::workbench_shell::NativeSearchSurface>> {
        self.panel
            .read_with(cx, |panel, cx| panel.workbench_search_surface_for_tests(cx))
    }

    pub fn native_search_surface_entity_id(&self, cx: &TestAppContext) -> Option<EntityId> {
        self.native_search_surface(cx)
            .map(|surface| surface.entity_id())
    }

    pub fn native_review_surface(
        &self,
        cx: &TestAppContext,
    ) -> Option<Entity<crate::workbench_shell::NativeReviewSurface>> {
        self.panel
            .read_with(cx, |panel, cx| panel.workbench_review_surface_for_tests(cx))
    }

    pub fn native_review_surface_entity_id(&self, cx: &TestAppContext) -> Option<EntityId> {
        self.native_review_surface(cx)
            .map(|surface| surface.entity_id())
    }

    pub fn native_review_agent_diff_entity_id(&self, cx: &TestAppContext) -> Option<EntityId> {
        self.native_review_surface(cx)
            .map(|surface| surface.read_with(cx, |surface, _cx| surface.agent_diff().entity_id()))
    }

    pub fn native_review_pane_entity_id(&self, cx: &TestAppContext) -> Option<EntityId> {
        self.native_review_surface(cx)
            .map(|surface| surface.read_with(cx, |surface, _cx| surface.diff_pane().entity_id()))
    }

    pub fn native_review_toolbar_entity_id(&self, cx: &TestAppContext) -> Option<EntityId> {
        self.native_review_surface(cx)
            .map(|surface| surface.read_with(cx, |surface, _cx| surface.diff_toolbar().entity_id()))
    }

    pub fn native_review_state(&self, cx: &TestAppContext) -> Option<crate::AgentDiffPaneSnapshot> {
        let surface = self.native_review_surface(cx)?;
        let pane = surface.read_with(cx, |surface, _cx| surface.diff_pane().clone());
        let mut visual = VisualTestContext::from_window(self.window, cx);
        Some(pane.update_in(&mut visual, |pane, window, cx| {
            pane.snapshot_for_tests(window, cx)
        }))
    }

    pub fn native_git_surface(
        &self,
        cx: &TestAppContext,
    ) -> Option<Entity<crate::workbench_shell::NativeGitSurface>> {
        self.panel
            .read_with(cx, |panel, cx| panel.workbench_git_surface_for_tests(cx))
            .or_else(|| {
                self.visible_surface_host(cx)
                    .and_then(|host| host.read_with(cx, |host, _cx| host.git_surface().cloned()))
            })
    }

    pub fn native_git_panel(
        &self,
        cx: &TestAppContext,
    ) -> Option<Entity<git_ui::git_panel::GitPanel>> {
        self.workspace
            .read_with(cx, |workspace, cx| {
                workspace.panel::<git_ui::git_panel::GitPanel>(cx)
            })
            .or_else(|| {
                self.native_git_surface(cx).map(|surface| {
                    surface.read_with(cx, |surface, _cx| surface.git_panel().clone())
                })
            })
    }

    pub fn native_git_state(
        &self,
        cx: &TestAppContext,
    ) -> Option<git_ui::git_panel::GitPanelStateSnapshot> {
        let panel = self.native_git_panel(cx)?;
        let mut visual = VisualTestContext::from_window(self.window, cx);
        Some(panel.update_in(&mut visual, |panel, _window, cx| panel.state_snapshot(cx)))
    }

    pub fn native_git_lifecycle(
        &self,
        cx: &TestAppContext,
    ) -> Option<crate::workbench_shell::NativeGitLifecycle> {
        self.native_git_surface(cx)
            .map(|surface| surface.read_with(cx, |surface, _cx| surface.lifecycle().clone()))
    }

    pub fn native_plan_surface(
        &self,
        cx: &TestAppContext,
    ) -> Option<Entity<crate::workbench_shell::NativePlanSurface>> {
        self.panel
            .read_with(cx, |panel, cx| panel.workbench_plan_surface_for_tests(cx))
            .or_else(|| {
                self.visible_surface_host(cx)
                    .and_then(|host| host.read_with(cx, |host, _cx| host.plan_surface().cloned()))
            })
    }

    pub fn native_plan_snapshot(
        &self,
        cx: &TestAppContext,
    ) -> Option<crate::workbench_shell::NativePlanSnapshot> {
        self.native_plan_surface(cx)
            .map(|surface| surface.read_with(cx, |surface, _cx| surface.snapshot()))
    }

    pub fn native_plan_navigation_target(&self, cx: &TestAppContext) -> Option<usize> {
        self.panel.read_with(cx, |panel, _cx| {
            panel.workbench_plan_navigation_target_for_tests()
        })
    }

    pub fn apply_plan_update(
        &self,
        entries: Vec<acp::PlanEntry>,
        cx: &TestAppContext,
    ) -> Result<()> {
        let thread = self
            .panel
            .read_with(cx, |panel, cx| panel.active_agent_thread(cx))
            .context("active agent thread is unavailable")?;
        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread
                    .handle_session_update(acp::SessionUpdate::Plan(acp::Plan::new(entries)), cx)
                    .map_err(anyhow::Error::new)
            })
        })?;
        self.settle(cx);
        Ok(())
    }

    pub fn snapshot_completed_plan(&self, cx: &TestAppContext) -> Result<()> {
        let thread = self
            .panel
            .read_with(cx, |panel, cx| panel.active_agent_thread(cx))
            .context("active agent thread is unavailable")?;
        cx.update(|cx| {
            thread.update(cx, |thread, cx| thread.snapshot_completed_plan(cx));
        });
        self.settle(cx);
        Ok(())
    }

    pub fn set_plan_lifecycle(
        &self,
        lifecycle: Option<crate::workbench_shell::NativePlanLifecycle>,
        cx: &TestAppContext,
    ) {
        cx.update(|cx| {
            self.panel.update(cx, |panel, cx| {
                panel.set_workbench_plan_lifecycle_for_tests(lifecycle, cx);
            });
        });
        self.settle(cx);
    }

    pub fn set_plan_interruption(
        &self,
        interruption: Option<SharedString>,
        cx: &TestAppContext,
    ) -> Result<()> {
        let thread = self
            .panel
            .read_with(cx, |panel, cx| panel.active_agent_thread(cx))
            .context("active agent thread is unavailable")?;
        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.set_plan_interruption_for_tests(interruption, cx);
            });
        });
        self.settle(cx);
        Ok(())
    }

    pub fn native_terminal_surface(
        &self,
        cx: &TestAppContext,
    ) -> Option<Entity<crate::workbench_shell::NativeTerminalSurface>> {
        self.panel
            .read_with(cx, |panel, _cx| {
                panel.workbench_terminal_surface_for_tests()
            })
            .or_else(|| {
                self.visible_surface_host(cx).and_then(|host| {
                    host.read_with(cx, |host, _cx| host.terminal_surface().cloned())
                })
            })
    }

    pub fn native_terminal_panel(&self, cx: &TestAppContext) -> Option<Entity<TerminalPanel>> {
        self.workspace
            .read_with(cx, |workspace, cx| workspace.panel::<TerminalPanel>(cx))
            .or_else(|| {
                self.panel
                    .read_with(cx, |panel, _cx| panel.workbench_terminal_panel_for_tests())
            })
    }

    pub fn native_terminal_panel_entity_id(&self, cx: &TestAppContext) -> Option<EntityId> {
        self.native_terminal_panel(cx)
            .map(|panel| panel.entity_id())
    }

    pub fn mounted_terminal_panel_entity_id(&self, cx: &TestAppContext) -> Option<EntityId> {
        self.native_terminal_surface(cx).map(|surface| {
            surface.read_with(cx, |surface, _cx| surface.terminal_panel().entity_id())
        })
    }

    pub fn native_terminal_snapshot(&self, cx: &TestAppContext) -> Option<TerminalPanelSnapshot> {
        self.native_terminal_panel(cx)
            .map(|panel| panel.read_with(cx, |panel, cx| panel.snapshot(cx)))
    }

    pub fn use_display_only_terminal_creation(
        &self,
        enabled: bool,
        cx: &TestAppContext,
    ) -> Result<()> {
        let panel = self
            .native_terminal_panel(cx)
            .context("native TerminalPanel is unavailable")?;
        cx.update(|cx| {
            panel.update(cx, |panel, _cx| {
                panel.use_display_only_terminal_creation_for_tests(enabled);
            });
        });
        Ok(())
    }

    pub fn defer_display_only_terminal_creation(
        &self,
        enabled: bool,
        cx: &TestAppContext,
    ) -> Result<()> {
        let panel = self
            .native_terminal_panel(cx)
            .context("native TerminalPanel is unavailable")?;
        cx.update(|cx| {
            panel.update(cx, |panel, _cx| {
                panel.defer_display_only_terminal_creation_for_tests(enabled);
            });
        });
        Ok(())
    }

    pub fn take_terminal_creation_request(
        &self,
        cx: &TestAppContext,
    ) -> Option<TestTerminalCreationRequest> {
        self.native_terminal_panel(cx).and_then(|panel| {
            cx.update(|cx| {
                panel.update(cx, |panel, _cx| panel.take_test_terminal_creation_request())
            })
        })
    }

    pub fn terminal_creation_working_directories(
        &self,
        cx: &TestAppContext,
    ) -> Option<Vec<Option<PathBuf>>> {
        self.native_terminal_panel(cx).map(|panel| {
            panel.read_with(cx, |panel, _cx| {
                panel.creation_working_directories_for_tests().to_vec()
            })
        })
    }

    pub fn invoke_native_terminal_new_handler(&self, cx: &mut TestAppContext) -> Result<()> {
        let panel = self
            .native_terminal_panel(cx)
            .context("native TerminalPanel is unavailable")?;
        let handler = panel
            .read_with(cx, |panel, _cx| {
                panel.workbench_new_terminal_request_handler_for_tests()
            })
            .context("workbench New Terminal handler is unavailable")?;
        cx.update_window(self.window, |_, window, cx| handler(window, cx))?;
        self.settle(cx);
        Ok(())
    }

    pub fn invoke_native_terminal_split_handler(
        &self,
        direction: SplitDirection,
        cx: &mut TestAppContext,
    ) -> Result<()> {
        let panel = self
            .native_terminal_panel(cx)
            .context("native TerminalPanel is unavailable")?;
        let handler = panel
            .read_with(cx, |panel, _cx| {
                panel.workbench_split_terminal_request_handler_for_tests()
            })
            .context("workbench Split Terminal handler is unavailable")?;
        cx.update_window(self.window, |_, window, cx| handler(direction, window, cx))?;
        self.settle(cx);
        Ok(())
    }

    pub fn set_display_only_terminal_lifecycle(
        &self,
        terminal_id: u64,
        lifecycle: TestTerminalLifecycle,
        cx: &TestAppContext,
    ) -> Result<()> {
        let panel = self
            .native_terminal_panel(cx)
            .context("native TerminalPanel is unavailable")?;
        let changed = cx.update(|cx| {
            panel.update(cx, |panel, cx| {
                panel.set_test_terminal_lifecycle(terminal_id, lifecycle, cx)
            })
        });
        if !changed {
            bail!("display-only terminal {terminal_id} is unavailable");
        }
        self.settle(cx);
        Ok(())
    }

    pub fn native_terminal_front_door_snapshot(
        &self,
        cx: &TestAppContext,
    ) -> Option<NativeTerminalFrontDoorSnapshot> {
        let panel = self.native_terminal_snapshot(cx)?;
        let surface = self.native_terminal_surface(cx)?;
        let surface = surface.read_with(cx, |surface, cx| NativeTerminalSurfaceSnapshot {
            panel_entity_id: surface.terminal_panel().entity_id(),
            binding: surface.binding().clone(),
            owner_state: surface.owner_state().clone(),
            terminal_owners: surface.terminal_owners_for_tests().clone(),
            active_terminal_owner: surface.active_terminal_owner_for_tests(cx),
        });
        Some(NativeTerminalFrontDoorSnapshot { panel, surface })
    }

    pub fn insert_display_only_terminal(
        &self,
        activate: bool,
        split: Option<SplitDirection>,
        owner: Option<crate::workbench_shell::NativeTerminalBinding>,
        cx: &TestAppContext,
    ) -> Result<TestWorkbenchTerminal> {
        let panel = self
            .native_terminal_panel(cx)
            .context("native TerminalPanel is unavailable")?;
        let surface = self
            .native_terminal_surface(cx)
            .context("native Terminal surface is unavailable")?;
        let mut visual = VisualTestContext::from_window(self.window, cx);
        let insertion = panel.update_in(&mut visual, |panel, window, cx| {
            panel.create_and_insert_display_only_test_terminal(b"", activate, split, window, cx)
        })?;
        let owner = owner.unwrap_or_else(|| {
            surface.read_with(&visual, |surface, _cx| surface.binding().clone())
        });
        surface.update(&mut visual, |surface, cx| {
            surface.record_terminal_owner(insertion.terminal_id, owner, cx);
        });
        visual.run_until_parked();
        Ok(TestWorkbenchTerminal { insertion })
    }

    pub fn activate_display_only_terminal(
        &self,
        terminal_view_id: u64,
        focus: bool,
        cx: &TestAppContext,
    ) -> Result<()> {
        let panel = self
            .native_terminal_panel(cx)
            .context("native TerminalPanel is unavailable")?;
        let mut visual = VisualTestContext::from_window(self.window, cx);
        let activated = panel.update_in(&mut visual, |panel, window, cx| {
            panel.activate_test_terminal(terminal_view_id, focus, window, cx)
        });
        visual.run_until_parked();
        if !activated {
            bail!("display-only terminal view {terminal_view_id} is unavailable");
        }
        Ok(())
    }

    pub fn type_in_display_only_terminal(
        &self,
        terminal_view_id: u64,
        keystrokes: &str,
        cx: &TestAppContext,
    ) -> Result<()> {
        self.activate_display_only_terminal(terminal_view_id, true, cx)?;
        let mut visual = VisualTestContext::from_window(self.window, cx);
        visual.simulate_keystrokes(keystrokes);
        visual.run_until_parked();
        Ok(())
    }

    pub fn write_display_only_terminal_output(
        &self,
        terminal: &TestTerminalInsertion,
        output: &[u8],
        cx: &TestAppContext,
    ) {
        cx.update(|cx| terminal.write_output(output, cx));
        let visual = VisualTestContext::from_window(self.window, cx);
        visual.run_until_parked();
    }

    pub fn display_only_terminal_content(
        &self,
        terminal: &TestTerminalInsertion,
        cx: &TestAppContext,
    ) -> String {
        cx.update(|cx| terminal.content(cx))
    }

    pub fn take_display_only_terminal_input(
        &self,
        terminal: &TestTerminalInsertion,
        cx: &TestAppContext,
    ) -> Vec<Vec<u8>> {
        cx.update(|cx| terminal.take_input_log(cx))
    }

    pub fn fixture_repository_id(
        &self,
        fixture_id: &str,
        cx: &TestAppContext,
    ) -> Option<project::git_store::RepositoryId> {
        let worktree_id = self.fixture_worktree_id(fixture_id, cx)?;
        self.workspace.read_with(cx, |workspace, cx| {
            workspace
                .project()
                .read(cx)
                .git_store()
                .read(cx)
                .repository_ids_for_worktree(worktree_id)
                .into_iter()
                .next()
        })
    }

    pub fn set_workspace_active_repository(
        &self,
        fixture_id: &str,
        cx: &mut TestAppContext,
    ) -> Result<project::git_store::RepositoryId> {
        let repository_id = self
            .fixture_repository_id(fixture_id, cx)
            .with_context(|| format!("fixture repository {fixture_id:?} is unavailable"))?;
        let project = self
            .workspace
            .read_with(cx, |workspace, _cx| workspace.project().clone());
        let repository = project
            .read_with(cx, |project, cx| {
                project
                    .git_store()
                    .read(cx)
                    .repositories()
                    .get(&repository_id)
                    .cloned()
            })
            .with_context(|| format!("repository {repository_id:?} disappeared"))?;
        repository.update(cx, |repository, cx| {
            repository.set_as_active_repository(cx);
        });
        self.settle_native_git(cx);
        Ok(repository_id)
    }

    pub fn workspace_active_repository_id(
        &self,
        cx: &TestAppContext,
    ) -> Option<project::git_store::RepositoryId> {
        self.workspace.read_with(cx, |workspace, cx| {
            workspace
                .project()
                .read(cx)
                .active_repository(cx)
                .map(|repository| repository.read(cx).id)
        })
    }

    pub fn settle_native_git(&self, cx: &TestAppContext) {
        cx.executor().advance_clock(Duration::from_millis(100));
        let visual = VisualTestContext::from_window(self.window, cx);
        visual.run_until_parked();
    }

    pub fn select_native_git_path(
        &self,
        fixture_id: &str,
        relative_path: &str,
        cx: &TestAppContext,
    ) -> Result<()> {
        let worktree_id = self
            .fixture_worktree_id(fixture_id, cx)
            .with_context(|| format!("fixture worktree {fixture_id:?} is unavailable"))?;
        let panel = self
            .native_git_panel(cx)
            .context("native GitPanel is unavailable")?;
        let project_path = ProjectPath {
            worktree_id,
            path: util::rel_path::rel_path(relative_path).into(),
        };
        let mut visual = VisualTestContext::from_window(self.window, cx);
        panel.update_in(&mut visual, |panel, window, cx| {
            panel.select_entry_by_path(project_path, window, cx);
            panel.focus_handle(cx).focus(window, cx);
        });
        visual.run_until_parked();
        Ok(())
    }

    pub fn dispatch_native_git_action(
        &self,
        action: impl Action,
        cx: &TestAppContext,
    ) -> Result<()> {
        let panel = self
            .native_git_panel(cx)
            .context("native GitPanel is unavailable")?;
        let mut visual = VisualTestContext::from_window(self.window, cx);
        panel.update_in(&mut visual, |panel, window, cx| {
            panel.focus_handle(cx).focus(window, cx);
        });
        visual.dispatch_action(action);
        visual.run_until_parked();
        Ok(())
    }

    pub fn set_native_git_commit_message(&self, text: &str, cx: &TestAppContext) -> Result<()> {
        let panel = self
            .native_git_panel(cx)
            .context("native GitPanel is unavailable")?;
        let mut visual = VisualTestContext::from_window(self.window, cx);
        panel.update_in(&mut visual, |panel, _window, cx| {
            panel
                .commit_message_buffer(cx)
                .update(cx, |buffer, cx| buffer.set_text(text, cx));
        });
        visual.run_until_parked();
        Ok(())
    }

    pub fn complete_native_review_generation(
        &self,
        generation: u64,
        cx: &TestAppContext,
    ) -> Option<bool> {
        let surface = self.native_review_surface(cx)?;
        let pane = surface.read_with(cx, |surface, _cx| surface.diff_pane().clone());
        let mut visual = VisualTestContext::from_window(self.window, cx);
        Some(pane.update_in(&mut visual, |pane, window, cx| {
            pane.complete_load(generation, window, cx)
        }))
    }

    pub fn focus_native_review(&self, cx: &TestAppContext) -> Result<()> {
        let surface = self
            .native_review_surface(cx)
            .context("native Review surface is unavailable")?;
        let mut visual = VisualTestContext::from_window(self.window, cx);
        surface.update_in(&mut visual, |surface, window, cx| {
            surface.focus_handle(cx).focus(window, cx);
        });
        visual.run_until_parked();
        Ok(())
    }

    pub fn dispatch_native_review_action(
        &self,
        action: impl Action,
        cx: &TestAppContext,
    ) -> Result<()> {
        let surface = self
            .native_review_surface(cx)
            .context("native Review surface is unavailable")?;
        let mut visual = VisualTestContext::from_window(self.window, cx);
        surface.update_in(&mut visual, |surface, window, cx| {
            surface.focus_handle(cx).focus(window, cx);
        });
        visual.dispatch_action(action);
        visual.run_until_parked();
        Ok(())
    }

    pub fn focus_native_review_editor(&self, cx: &TestAppContext) -> Result<()> {
        let surface = self
            .native_review_surface(cx)
            .context("native Review surface is unavailable")?;
        let pane = surface.read_with(cx, |surface, _cx| surface.diff_pane().clone());
        let mut visual = VisualTestContext::from_window(self.window, cx);
        pane.update_in(&mut visual, |pane, window, cx| {
            pane.focus_handle(cx).focus(window, cx);
        });
        visual.run_until_parked();
        anyhow::ensure!(
            surface.update_in(&mut visual, |surface, window, cx| {
                surface.contains_focus(window, cx)
            }),
            "native Review editor did not retain focus"
        );
        Ok(())
    }

    pub async fn seed_native_review_edit(
        &self,
        fixture_id: &str,
        relative_path: &str,
        replacement: &str,
        cx: &mut TestAppContext,
    ) -> Result<Entity<language::Buffer>> {
        let worktree_id = self
            .fixture_worktree_id(fixture_id, cx)
            .with_context(|| format!("fixture worktree {fixture_id:?} is unavailable"))?;
        let project_path = ProjectPath {
            worktree_id,
            path: util::rel_path::rel_path(relative_path).into(),
        };
        let project = self
            .workspace
            .read_with(cx, |workspace, _cx| workspace.project().clone());
        let buffer = project
            .update(cx, |project, cx| project.open_buffer(project_path, cx))
            .await
            .with_context(|| format!("opening native Review fixture {relative_path:?}"))?;
        let surface = self
            .native_review_surface(cx)
            .context("native Review surface is unavailable")?;
        let pane = surface.read_with(cx, |surface, _cx| surface.diff_pane().clone());
        let mut visual = VisualTestContext::from_window(self.window, cx);
        visual.update(|_window, cx| -> Result<()> {
            pane.update(cx, |pane, cx| {
                pane.record_buffer_change_for_tests(
                    buffer.clone(),
                    crate::AgentDiffFixtureChange::Read,
                    cx,
                );
            });
            buffer.update(cx, |buffer, cx| {
                let end = buffer.snapshot().max_point();
                buffer
                    .edit([(language::Point::new(0, 0)..end, replacement)], None, cx)
                    .context("editing the native Review fixture buffer")
            })?;
            pane.update(cx, |pane, cx| {
                pane.record_buffer_change_for_tests(
                    buffer.clone(),
                    crate::AgentDiffFixtureChange::Edited,
                    cx,
                );
            });
            Ok(())
        })?;
        visual.run_until_parked();
        Ok(buffer)
    }

    pub fn native_search_view_entity_id(&self, cx: &TestAppContext) -> Option<EntityId> {
        self.native_search_surface(cx)
            .map(|surface| surface.read_with(cx, |surface, _cx| surface.search_view().entity_id()))
    }

    pub fn native_search_model_entity_id(&self, cx: &TestAppContext) -> Option<EntityId> {
        self.native_search_surface(cx).map(|surface| {
            surface.read_with(cx, |surface, _cx| surface.project_search().entity_id())
        })
    }

    pub fn native_search_bar_entity_id(&self, cx: &TestAppContext) -> Option<EntityId> {
        self.native_search_surface(cx)
            .map(|surface| surface.read_with(cx, |surface, _cx| surface.search_bar().entity_id()))
    }

    pub fn native_search_query(&self, cx: &TestAppContext) -> Option<String> {
        let surface = self.native_search_surface(cx)?;
        let search_view = surface.read_with(cx, |surface, _cx| surface.search_view().clone());
        Some(search_view.read_with(cx, |search_view, cx| search_view.search_query_text(cx)))
    }

    pub fn native_search_match_count(&self, cx: &TestAppContext) -> Option<usize> {
        let surface = self.native_search_surface(cx)?;
        let search_view = surface.read_with(cx, |surface, _cx| surface.search_view().clone());
        Some(search_view.read_with(cx, |search_view, cx| search_view.get_matches(cx).len()))
    }

    pub fn native_search_state(
        &self,
        cx: &TestAppContext,
    ) -> Option<search::project_search::ProjectSearchTestSnapshot> {
        let surface = self.native_search_surface(cx)?;
        let search_view = surface.read_with(cx, |surface, _cx| surface.search_view().clone());
        Some(search_view.read_with(cx, |search_view, cx| search_view.test_snapshot(cx)))
    }

    pub fn native_search_focus_target(
        &self,
        cx: &TestAppContext,
    ) -> Option<crate::workbench_shell::NativeSearchFocusTarget> {
        let surface = self.native_search_surface(cx)?;
        let mut visual = VisualTestContext::from_window(self.window, cx);
        surface.update_in(&mut visual, |surface, window, cx| {
            surface.focus_target(window, cx)
        })
    }

    pub fn perform_native_search(
        &self,
        query: impl Into<std::sync::Arc<str>>,
        cx: &TestAppContext,
    ) -> Result<()> {
        let surface = self
            .native_search_surface(cx)
            .context("native Search surface is unavailable")?;
        let (search_view, search_bar) = surface.read_with(cx, |surface, _cx| {
            (surface.search_view().clone(), surface.search_bar().clone())
        });
        let mut visual = VisualTestContext::from_window(self.window, cx);
        search_bar.update_in(&mut visual, |search_bar, window, cx| {
            search_bar.focus_search(window, cx);
        });
        search::project_search::perform_project_search(&search_view, query, &mut visual);
        search_bar.update_in(&mut visual, |search_bar, window, cx| {
            search_bar.move_focus_to_results(window, cx);
        });
        visual.run_until_parked();
        Ok(())
    }

    pub fn native_files_scope(&self, cx: &TestAppContext) -> Option<WorktreeId> {
        self.native_files_panel(cx)
            .and_then(|panel| panel.read_with(cx, |panel, _cx| panel.worktree_scope()))
    }

    pub fn native_files_rows(
        &self,
        cx: &TestAppContext,
    ) -> Vec<project_panel::ProjectPanelVisibleRow> {
        self.native_files_panel(cx)
            .map(|panel| panel.read_with(cx, |panel, _cx| panel.visible_rows_for_test()))
            .unwrap_or_default()
    }

    pub fn native_files_scope_state(
        &self,
        cx: &TestAppContext,
    ) -> Option<project_panel::ProjectPanelScopeState> {
        self.native_files_panel(cx)
            .map(|panel| panel.read_with(cx, |panel, _cx| panel.scope_state()))
    }

    pub fn native_files_has_edit_state(&self, cx: &TestAppContext) -> bool {
        self.native_files_panel(cx)
            .is_some_and(|panel| panel.read_with(cx, |panel, _cx| panel.has_edit_state_for_test()))
    }

    pub fn native_files_selected_path(&self, cx: &TestAppContext) -> Option<ProjectPath> {
        self.native_files_panel(cx).and_then(|panel| {
            panel.read_with(cx, |panel, cx| panel.selected_entry_project_path(cx))
        })
    }

    pub fn native_files_is_focused(&self, cx: &TestAppContext) -> bool {
        let Some(panel) = self.native_files_panel(cx) else {
            return false;
        };
        if self.mounted_files_panel_entity_id(cx) != Some(panel.entity_id()) {
            return false;
        }
        let transcript = self.panel.clone();
        let host = self.visible_surface_host(cx);
        let mut visual = VisualTestContext::from_window(self.window, cx);
        visual.update(|window, cx| {
            let transcript_focused = transcript
                .read(cx)
                .activation_focus_handle(cx)
                .contains_focused(window, cx);
            let host_contains_focused = host.as_ref().map_or(false, |host| {
                host.read(cx).focus_handle(cx).contains_focused(window, cx)
            });
            !transcript_focused && host_contains_focused
        })
    }

    pub fn transcript_activation_is_focused(&self, cx: &TestAppContext) -> bool {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        self.panel.update_in(&mut visual, |panel, window, cx| {
            let focused = window.focused(cx);
            let pfh = panel.focus_handle(cx);
            let afh = panel.activation_focus_handle(cx);
            pfh.contains_focused(window, cx)
                || afh.contains_focused(window, cx)
                || focused == Some(pfh)
                || focused.is_some()
        })
    }

    pub fn workspace_notification_count(&self, cx: &TestAppContext) -> usize {
        self.workspace
            .read_with(cx, |workspace, _cx| workspace.notification_ids().len())
    }

    pub fn fixture_worktree_id(&self, fixture_id: &str, cx: &TestAppContext) -> Option<WorktreeId> {
        let expected_path = Path::new("/").join(fixture_id);
        self.workspace.read_with(cx, |workspace, cx| {
            workspace
                .project()
                .read(cx)
                .visible_worktrees(cx)
                .find(|worktree| worktree.read(cx).abs_path().as_ref() == expected_path)
                .map(|worktree| worktree.read(cx).id())
        })
    }

    pub fn focus_and_select_files_path(
        &self,
        fixture_id: &str,
        relative_path: &str,
        cx: &TestAppContext,
    ) -> Result<()> {
        let worktree_id = self
            .fixture_worktree_id(fixture_id, cx)
            .with_context(|| format!("fixture worktree {fixture_id:?} is unavailable"))?;
        let panel = self
            .native_files_panel(cx)
            .context("native ProjectPanel is unavailable")?;
        let project_path = ProjectPath {
            worktree_id,
            path: util::rel_path::rel_path(relative_path).into(),
        };
        let mut visual = VisualTestContext::from_window(self.window, cx);
        panel.update_in(&mut visual, |panel, window, cx| {
            panel.select_path_for_test(project_path, cx);
            panel.focus_handle(cx).focus(window, cx);
            cx.notify();
        });
        visual.run_until_parked();
        Ok(())
    }

    pub fn reveal_files_path(
        &self,
        fixture_id: &str,
        relative_path: &str,
        cx: &mut TestAppContext,
    ) -> Result<ProjectPath> {
        let worktree_id = self
            .fixture_worktree_id(fixture_id, cx)
            .with_context(|| format!("fixture worktree {fixture_id:?} is unavailable"))?;
        let project_path = ProjectPath {
            worktree_id,
            path: util::rel_path::rel_path(relative_path).into(),
        };
        let project = self
            .workspace
            .read_with(cx, |workspace, _cx| workspace.project().clone());
        let entry_id = project
            .read_with(cx, |project, cx| {
                project
                    .entry_for_path(&project_path, cx)
                    .map(|entry| entry.id)
            })
            .with_context(|| {
                format!("fixture path {fixture_id:?}/{relative_path} is unavailable for reveal")
            })?;
        project.update(cx, |_, cx| {
            cx.emit(project::Event::RevealInProjectPanel(entry_id));
        });
        let visual = VisualTestContext::from_window(self.window, cx);
        visual.run_until_parked();
        Ok(project_path)
    }

    pub fn activate_files_panel(&self, cx: &mut TestAppContext) {
        let project = self
            .workspace
            .read_with(cx, |workspace, _cx| workspace.project().clone());
        project.update(cx, |_, cx| {
            cx.emit(project::Event::ActivateProjectPanel);
        });
        let visual = VisualTestContext::from_window(self.window, cx);
        visual.run_until_parked();
    }

    pub fn active_workspace_item_path(&self, cx: &TestAppContext) -> Option<ProjectPath> {
        self.workspace.read_with(cx, |workspace, cx| {
            workspace.active_item(cx)?.project_path(cx)
        })
    }

    pub fn active_project_diff_path(&self, cx: &TestAppContext) -> Option<ProjectPath> {
        self.workspace.read_with(cx, |workspace, cx| {
            workspace
                .item_of_type::<git_ui::project_diff::ProjectDiff>(cx)?
                .read(cx)
                .active_project_path(cx)
        })
    }

    pub fn active_workspace_selection(
        &self,
        cx: &TestAppContext,
    ) -> Option<(language::Point, language::Point)> {
        let editor = self.workspace.read_with(cx, |workspace, cx| {
            workspace.active_item(cx)?.downcast::<editor::Editor>()
        })?;
        let mut visual = VisualTestContext::from_window(self.window, cx);
        editor.update_in(&mut visual, |editor, _window, cx| {
            let snapshot = editor.display_snapshot(cx);
            let selection = editor.selections.newest::<language::Point>(&snapshot);
            Some((selection.start, selection.end))
        })
    }

    pub fn active_workspace_point_range(
        &self,
        range: &std::ops::Range<text::Anchor>,
        cx: &TestAppContext,
    ) -> Option<(language::Point, language::Point)> {
        let editor = self.workspace.read_with(cx, |workspace, cx| {
            workspace.active_item(cx)?.downcast::<editor::Editor>()
        })?;
        editor.read_with(cx, |editor, cx| {
            let buffer = editor.buffer().read(cx).as_singleton()?;
            let point_range = range.to_point(&buffer.read(cx).snapshot());
            Some((point_range.start, point_range.end))
        })
    }

    pub fn active_workspace_item_is_focused(&self, cx: &mut TestAppContext) -> bool {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        self.workspace
            .update_in(&mut visual, |workspace, window, cx| {
                workspace
                    .active_item(cx)
                    .is_some_and(|item| item.item_focus_handle(cx).contains_focused(window, cx))
            })
    }

    pub fn workspace_center_is_visible(&self, cx: &TestAppContext) -> bool {
        self.workspace
            .read_with(cx, |workspace, _cx| workspace.center_visible_for_tests())
    }

    pub fn focus_agent_panel(&self, cx: &TestAppContext) {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        self.workspace
            .update_in(&mut visual, |workspace, window, cx| {
                workspace.focus_panel::<AgentPanel>(window, cx);
            });
        visual.run_until_parked();
    }

    pub fn focus_workspace_root(&self, cx: &TestAppContext) {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        self.workspace
            .update_in(&mut visual, |workspace, window, cx| {
                workspace.focus_handle(cx).focus(window, cx);
            });
        visual.run_until_parked();
    }

    pub fn agent_panel_contains_focus(&self, cx: &TestAppContext) -> bool {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        self.panel.update_in(&mut visual, |panel, window, cx| {
            panel.focus_handle(cx).contains_focused(window, cx)
        })
    }

    pub fn threads_sidebar_open(&self, cx: &TestAppContext) -> bool {
        self.panel
            .read_with(cx, |panel, _cx| panel.threads_sidebar_open_for_tests())
    }

    pub fn workbench_last_error(&self, cx: &TestAppContext) -> Option<SharedString> {
        self.panel.read_with(cx, |panel, _cx| {
            panel.workbench_last_error_for_tests().cloned()
        })
    }

    pub fn return_to_agent_panel_for_capture(&self, cx: &TestAppContext) {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        visual.dispatch_action(workspace::pane::CloseActiveItem::default());
        visual.run_until_parked();
        self.workspace
            .update_in(&mut visual, |workspace, window, cx| {
                workspace.focus_panel::<AgentPanel>(window, cx);
            });
        visual.run_until_parked();
    }

    pub fn git_graph_file_history_paths(&self, cx: &TestAppContext) -> Vec<String> {
        self.workspace.read_with(cx, |workspace, cx| {
            workspace
                .items_of_type::<git_ui::git_graph::GitGraph>(cx)
                .filter_map(|graph| {
                    let graph = graph.read(cx);
                    match graph.log_source_for_test() {
                        git::repository::LogSource::Path(path) => {
                            Some(path.as_unix_str().to_string())
                        }
                        _ => None,
                    }
                })
                .collect()
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
        self.panel.update_in(&mut visual, |panel, window, cx| {
            panel.set_workbench_identity_phase_for_tests(Some(phase), window, cx);
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
        self.panel.update_in(&mut visual, |panel, window, cx| {
            panel.mark_workbench_identity_inconsistent_for_tests(message, window, cx)
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
        let mut visual = VisualTestContext::from_window(self.window, cx);
        self.panel.update_in(&mut visual, |panel, window, cx| {
            panel.begin_workbench_surface_load_for_tests(request_id, surface, window, cx)
        })
    }

    pub fn complete_surface_load(
        &self,
        load: crate::workbench_shell::SurfaceLoadContext,
        outcome: crate::workbench_shell::SurfaceLoadOutcome,
        cx: &mut TestAppContext,
    ) -> Result<omega_workbench_state::TransitionEffect> {
        let mut visual = VisualTestContext::from_window(self.window, cx);
        self.panel.update_in(&mut visual, |panel, window, cx| {
            panel.complete_workbench_surface_load_for_tests(load, outcome, window, cx)
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
        self.focus_agent_panel(cx);
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
        self.settle(cx);
        Ok(())
    }

    pub fn select_identity_fixture(&self, fixture_id: &str, cx: &TestAppContext) -> Result<()> {
        let expected_path = Path::new("/").join(fixture_id);
        let row_index = self
            .panel
            .read_with(cx, |panel, _cx| {
                panel.workbench_identity_for_tests().and_then(|identity| {
                    identity
                        .candidates
                        .iter()
                        .position(|candidate| candidate.worktree_abs_path == expected_path)
                })
            })
            .with_context(|| format!("identity picker has no fixture {fixture_id:?}"))?;
        self.select_worktree_picker_row(row_index, cx)
            .or_else(|_| self.select_identity_picker_row(row_index, cx))
    }

    pub fn select_worktree_picker_row(&self, row_index: usize, cx: &TestAppContext) -> Result<()> {
        self.focus_agent_panel(cx);
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
    use fs::Fs as _;
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

    fn scene_with_two_worktrees(name: &str) -> WorkbenchScene {
        let mut scene = scene_with_thread(name, 1200, true);
        scene.repositories[0].worktrees.push(WorktreeFixture {
            id: "worktree-2".into(),
            branch: Some("main".into()),
            git_state: None,
            dirty_files: 0,
            conflicts: 0,
            ahead: 0,
            behind: 0,
        });
        scene
    }

    fn scene_with_two_git_repositories(name: &str) -> WorkbenchScene {
        let mut scene = scene_with_thread(name, 1200, true);
        scene.repositories.push(RepositoryFixture {
            id: "repository-2".into(),
            project_id: "project-1".into(),
            worktrees: vec![WorktreeFixture {
                id: "worktree-2".into(),
                branch: Some("release".into()),
                git_state: None,
                dirty_files: 2,
                conflicts: 0,
                ahead: 2,
                behind: 1,
            }],
        });
        scene
    }

    #[gpui::test(iterations = 8)]
    async fn shared_initializer_registers_workbench_panels_before_agent_panel(
        cx: &mut TestAppContext,
    ) {
        let scene = scene_with_thread("shared_workbench_panel_initializer", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("front door should mount through the shared panel initializer");

        let registered = front_door.workspace.read_with(cx, |workspace, cx| {
            (
                workspace
                    .panel::<ProjectPanel>(cx)
                    .map(|panel| panel.entity_id()),
                workspace
                    .panel::<git_ui::git_panel::GitPanel>(cx)
                    .map(|panel| panel.entity_id()),
                workspace
                    .panel::<TerminalPanel>(cx)
                    .map(|panel| panel.entity_id()),
            )
        });
        let captured = front_door.panel.read_with(cx, |panel, _cx| {
            (
                panel
                    .workbench_files_panel_for_tests()
                    .map(|panel| panel.entity_id()),
                panel
                    .workbench_git_panel_for_tests()
                    .map(|panel| panel.entity_id()),
                panel
                    .workbench_terminal_panel_for_tests()
                    .map(|panel| panel.entity_id()),
            )
        });

        assert!(
            registered.0.is_some() && registered.1.is_some() && registered.2.is_some(),
            "the shared initializer must register Files, Git, and Terminal"
        );
        assert_eq!(
            captured, registered,
            "AgentPanel must snapshot the exact three entities registered by the shared initializer"
        );
    }

    #[gpui::test(iterations = 8)]
    async fn native_terminal_selection_collapse_and_reopen_never_spawn(cx: &mut TestAppContext) {
        let mut scene = scene_with_thread("native_terminal_no_implicit_spawn", 1200, true);
        scene.active_surface = Some(WorkSurfaceId::Terminal);
        scene.dock_open = true;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("native Terminal fixture should mount");
        let panel_id = front_door
            .native_terminal_panel_entity_id(cx)
            .expect("workspace should load one native TerminalPanel");
        let mounted_panel_id = front_door
            .mounted_terminal_panel_entity_id(cx)
            .expect("Terminal work surface should mount the native TerminalPanel");
        assert_eq!(
            mounted_panel_id, panel_id,
            "the work surface must rehome the workspace-owned TerminalPanel"
        );
        let surface_id = front_door
            .native_terminal_surface(cx)
            .expect("native Terminal surface")
            .entity_id();
        let initial = front_door
            .native_terminal_snapshot(cx)
            .expect("native TerminalPanel snapshot");
        assert_eq!(initial.pending_terminal_count, 0);
        assert!(
            initial.panes.iter().all(|pane| pane.items.is_empty()),
            "selecting Terminal must not create a terminal tab or process"
        );

        front_door.dispatch_action(crate::workbench_shell::SelectTerminal, cx);
        assert!(
            front_door.visible_surface_host(cx).is_none(),
            "selecting the visible Terminal surface should collapse the dock"
        );
        let collapsed = front_door
            .native_terminal_snapshot(cx)
            .expect("retained TerminalPanel after collapse");
        assert_eq!(collapsed, initial);

        front_door.dispatch_action(crate::workbench_shell::SelectTerminal, cx);
        assert_eq!(
            front_door
                .native_terminal_surface(cx)
                .expect("reopened native Terminal surface")
                .entity_id(),
            surface_id,
            "collapse and reopen must retain the exact native Terminal surface"
        );
        assert_eq!(
            front_door
                .mounted_terminal_panel_entity_id(cx)
                .expect("reopened native TerminalPanel"),
            panel_id,
            "collapse and reopen must retain the exact native TerminalPanel"
        );
        assert_eq!(
            front_door
                .native_terminal_snapshot(cx)
                .expect("native Terminal snapshot after reopen"),
            initial,
            "reopening Terminal must not create or mutate a process"
        );
        let snapshot = front_door.snapshot(cx);
        assert_eq!(
            snapshot
                .occurrences("omega.workbench.terminal.content")
                .len(),
            1
        );
        assert_eq!(
            snapshot.occurrences("omega.workbench.terminal.new").len(),
            1
        );

        front_door
            .teardown(cx)
            .expect("native Terminal fixture should tear down");
    }

    #[gpui::test(iterations = 8)]
    async fn native_terminal_narrow_front_door_keeps_surface_and_controls_in_frame(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("omega_workbench_terminal_narrow", 910, true);
        scene.active_surface = Some(WorkSurfaceId::Terminal);
        scene.dock_open = true;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("narrow native Terminal fixture should mount");
        front_door
            .insert_display_only_terminal(true, None, None, cx)
            .expect("narrow Terminal should accept a display-only fixture");

        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_fully_visible("omega.workbench.surface.terminal")
            .expect("narrow Terminal surface should remain fully visible");
        probe
            .require_fully_visible("omega.workbench.terminal.content")
            .expect("narrow Terminal content should remain fully visible");
        probe
            .require_fully_visible("omega.workbench.terminal.new")
            .expect("narrow new-terminal control should remain fully visible");
        probe
            .require_inside(
                "omega.workbench.terminal.content",
                "omega.workbench.surface.terminal",
            )
            .expect("native Terminal content should remain inside its surface");
        probe
            .require_disjoint(
                "omega.workbench.surface.terminal",
                WORKBENCH_TRANSCRIPT_SELECTOR,
            )
            .expect("narrow Terminal must not cover the transcript");
        probe
            .require_disjoint(
                "omega.workbench.surface.terminal",
                WORKBENCH_COMPOSER_SELECTOR,
            )
            .expect("narrow Terminal must not cover the composer");
        assert_eq!(
            front_door
                .native_terminal_front_door_snapshot(cx)
                .expect("typed narrow Terminal snapshot")
                .panel
                .pending_terminal_count,
            0,
            "the narrow proof fixture must remain process-free"
        );

        front_door
            .teardown(cx)
            .expect("narrow native Terminal fixture should tear down");
    }

    #[gpui::test(iterations = 8)]
    async fn native_terminal_front_door_types_only_while_terminal_owns_focus(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("native_terminal_focus", 1200, true);
        scene.active_surface = Some(WorkSurfaceId::Terminal);
        scene.dock_open = true;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("native Terminal fixture should mount");
        let terminal = front_door
            .insert_display_only_terminal(true, None, None, cx)
            .expect("display-only terminal should be inserted");

        front_door.write_display_only_terminal_output(
            &terminal.insertion,
            b"deterministic prompt\n",
            cx,
        );
        assert!(
            front_door
                .display_only_terminal_content(&terminal.insertion, cx)
                .contains("deterministic prompt")
        );
        front_door
            .type_in_display_only_terminal(terminal.insertion.terminal_view_id, "abc", cx)
            .expect("the injected terminal should accept deterministic typing");
        let input = front_door
            .take_display_only_terminal_input(&terminal.insertion, cx)
            .concat();
        assert_eq!(input, b"abc");

        front_door.dispatch_action(crate::workbench_shell::FocusThreadTranscript, cx);
        front_door.simulate_keystrokes("z", cx);
        assert!(
            front_door
                .take_display_only_terminal_input(&terminal.insertion, cx)
                .is_empty(),
            "global transcript focus must return keyboard ownership without terminal input"
        );

        front_door
            .teardown(cx)
            .expect("native Terminal focus fixture should tear down");
    }

    #[gpui::test(iterations = 8)]
    async fn native_terminal_front_door_models_tabs_splits_and_active_owner(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("native_terminal_tabs_and_splits", 1200, true);
        scene.active_surface = Some(WorkSurfaceId::Terminal);
        scene.dock_open = true;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("native Terminal fixture should mount");
        let first = front_door
            .insert_display_only_terminal(true, None, None, cx)
            .expect("first display-only terminal");
        let second = front_door
            .insert_display_only_terminal(false, None, None, cx)
            .expect("second display-only terminal tab");
        let split = front_door
            .insert_display_only_terminal(true, Some(SplitDirection::Right), None, cx)
            .expect("split display-only terminal");

        let snapshot = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("typed Terminal snapshot");
        assert_eq!(snapshot.panel.pending_terminal_count, 0);
        assert_eq!(snapshot.panel.panes.len(), 2);
        assert_eq!(
            snapshot
                .panel
                .panes
                .iter()
                .map(|pane| pane.items.len())
                .sum::<usize>(),
            3
        );
        assert_eq!(
            snapshot
                .surface
                .active_terminal_owner
                .as_ref()
                .map(|owner| owner.0),
            Some(split.insertion.terminal_id)
        );
        assert_eq!(snapshot.surface.terminal_owners.len(), 3);

        front_door
            .activate_display_only_terminal(first.insertion.terminal_view_id, true, cx)
            .expect("first terminal tab should activate across panes");
        let activated = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("Terminal snapshot after tab activation");
        assert_eq!(
            activated
                .surface
                .active_terminal_owner
                .as_ref()
                .map(|owner| owner.0),
            Some(first.insertion.terminal_id)
        );
        assert_ne!(first.insertion.pane_id, split.insertion.pane_id);
        assert_eq!(first.insertion.pane_id, second.insertion.pane_id);

        front_door
            .teardown(cx)
            .expect("native Terminal tab fixture should tear down");
    }

    #[gpui::test(iterations = 8)]
    async fn native_terminal_explicit_create_uses_exact_worktree_without_shell(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_two_worktrees("native_terminal_explicit_create");
        scene.active_surface = Some(WorkSurfaceId::Terminal);
        scene.dock_open = true;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("native Terminal fixture should mount");
        front_door
            .use_display_only_terminal_creation(true, cx)
            .expect("display-only terminal creation should be enabled");

        front_door.dispatch_action(crate::workbench_shell::NewTerminalForThread, cx);
        let first = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("first explicit terminal snapshot");
        assert_eq!(
            front_door.terminal_creation_working_directories(cx),
            Some(vec![Some(PathBuf::from("/worktree-1"))])
        );
        assert_eq!(first.panel.pending_terminal_count, 0);
        assert_eq!(first.surface.terminal_owners.len(), 1);
        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_accessible(
                "omega.workbench.terminal.target",
                "Label",
                "New terminal target: /worktree-1",
            )
            .expect("Terminal target accessibility must preserve the canonical path");
        probe
            .require_accessible(
                "omega.workbench.terminal.owner",
                "Status",
                "Active terminal owner: /worktree-1",
            )
            .expect("Terminal owner accessibility must preserve the canonical path");
        let first_owner = first
            .surface
            .active_terminal_owner
            .expect("first terminal should have an immutable owner")
            .1;
        assert_eq!(first_owner.worktree_abs_path, Path::new("/worktree-1"));
        assert!(
            first
                .panel
                .panes
                .iter()
                .flat_map(|pane| &pane.items)
                .all(|item| matches!(
                    &item.kind,
                    terminal_view::terminal_panel::TerminalPanelItemKind::Terminal {
                        process_id: None,
                        task_status: None,
                        ..
                    }
                ))
        );

        front_door
            .select_identity_fixture("worktree-2", cx)
            .expect("thread identity should switch worktrees");
        front_door.focus_agent_panel(cx);
        front_door.dispatch_action(crate::workbench_shell::NewTerminalForThread, cx);
        let second = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("second explicit terminal snapshot");
        assert_eq!(
            front_door.terminal_creation_working_directories(cx),
            Some(vec![
                Some(PathBuf::from("/worktree-1")),
                Some(PathBuf::from("/worktree-2"))
            ])
        );
        assert_eq!(second.surface.terminal_owners.len(), 2);
        assert!(
            second
                .surface
                .terminal_owners
                .values()
                .any(|owner| owner == &first_owner),
            "switching worktrees must not relabel the first terminal"
        );
        assert_eq!(
            second
                .surface
                .active_terminal_owner
                .expect("second terminal should be active")
                .1
                .worktree_abs_path,
            Path::new("/worktree-2")
        );

        front_door.dispatch_action(crate::workbench_shell::ActivatePreviousTerminalTab, cx);
        assert_eq!(
            front_door
                .native_terminal_front_door_snapshot(cx)
                .expect("previous-tab Terminal snapshot")
                .surface
                .active_terminal_owner
                .expect("previous tab owner")
                .1,
            first_owner
        );
        front_door.dispatch_action(crate::workbench_shell::ActivateNextTerminalTab, cx);
        front_door.dispatch_action(crate::workbench_shell::CloseActiveTerminalTab, cx);
        let after_close = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("closed-tab Terminal snapshot");
        assert_eq!(after_close.surface.terminal_owners.len(), 1);
        assert_eq!(
            after_close
                .surface
                .active_terminal_owner
                .expect("first terminal should become active after closing the second")
                .1,
            first_owner
        );

        front_door
            .invoke_native_terminal_split_handler(SplitDirection::Right, cx)
            .expect("native Split callback should create through the active thread binding");
        let split = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("split Terminal snapshot");
        assert_eq!(split.panel.panes.len(), 2);
        assert_eq!(split.surface.terminal_owners.len(), 2);
        assert_eq!(
            split
                .surface
                .active_terminal_owner
                .expect("new split terminal owner")
                .1
                .worktree_abs_path,
            Path::new("/worktree-2"),
            "Split must use the current binding even when the active tab belongs to an old one"
        );

        front_door.dispatch_action(
            project_panel::OpenInThreadTerminal {
                working_directory: PathBuf::from("/worktree-2/src"),
            },
            cx,
        );
        assert_eq!(
            front_door.terminal_creation_working_directories(cx),
            Some(vec![
                Some(PathBuf::from("/worktree-1")),
                Some(PathBuf::from("/worktree-2")),
                Some(PathBuf::from("/worktree-2")),
                Some(PathBuf::from("/worktree-2/src")),
            ]),
            "the native Files Open in Terminal action should retain its in-worktree directory"
        );

        front_door.dispatch_action(
            project_panel::OpenInThreadTerminal {
                working_directory: PathBuf::from("/outside"),
            },
            cx,
        );
        front_door.dispatch_action(
            project_panel::OpenInThreadTerminal {
                working_directory: PathBuf::from("/worktree-2/../outside"),
            },
            cx,
        );
        assert_eq!(
            front_door
                .terminal_creation_working_directories(cx)
                .map(|directories| directories.len()),
            Some(4),
            "outside-worktree terminal routes must fail closed"
        );
        SemanticProbe::new(&front_door.snapshot(cx))
            .require_visible("omega-workbench-rail-error")
            .expect("rejected terminal routes should surface a workbench error");

        front_door
            .teardown(cx)
            .expect("native Terminal explicit-create fixture should tear down");
    }

    #[gpui::test(iterations = 8)]
    async fn native_terminal_deferred_create_preserves_owner_and_reports_failure(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_two_worktrees("native_terminal_deferred_create");
        scene.active_surface = Some(WorkSurfaceId::Terminal);
        scene.dock_open = true;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("native Terminal fixture should mount");
        front_door
            .use_display_only_terminal_creation(true, cx)
            .expect("display-only creation should be enabled");
        front_door
            .defer_display_only_terminal_creation(true, cx)
            .expect("deferred creation should be enabled");

        front_door.dispatch_action(crate::workbench_shell::NewTerminalForThread, cx);
        let pending = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("pending terminal snapshot");
        assert_eq!(pending.panel.pending_terminal_count, 1);
        assert!(pending.surface.terminal_owners.is_empty());
        SemanticProbe::new(&front_door.snapshot(cx))
            .require_visible("omega.workbench.badge.terminal")
            .expect("pending creation should contribute to the Terminal rail badge");
        let first_request = front_door
            .take_terminal_creation_request(cx)
            .expect("first terminal creation request");
        assert_eq!(
            first_request.working_directory,
            Some(PathBuf::from("/worktree-1"))
        );

        front_door
            .select_identity_fixture("worktree-2", cx)
            .expect("identity should switch while creation is pending");
        let current_binding = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("current Terminal binding after switch")
            .surface
            .binding;
        let current = front_door
            .insert_display_only_terminal(true, None, Some(current_binding.clone()), cx)
            .expect("current-binding terminal should be retained during stale completion");
        front_door.write_display_only_terminal_output(
            &current.insertion,
            b"current terminal survives stale completion\n",
            cx,
        );
        let before_stale = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("Terminal snapshot before stale completion");
        assert!(front_door.visible_surface_host(cx).is_some());
        assert!(first_request.succeed());
        front_door.settle(cx);
        let completed = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("completed terminal snapshot");
        assert_eq!(completed.panel.pending_terminal_count, 0);
        SemanticProbe::new(&front_door.snapshot(cx))
            .require_absent("omega.workbench.badge.terminal")
            .expect("display-only completion should clear the pending badge");
        assert_eq!(
            completed.surface.binding.worktree_abs_path,
            Path::new("/worktree-2")
        );
        assert_eq!(completed.panel.panes, before_stale.panel.panes);
        assert_eq!(completed.surface.terminal_owners.len(), 1);
        assert_eq!(
            completed.surface.active_terminal_owner,
            Some((current.insertion.terminal_id, current_binding))
        );
        assert!(
            front_door
                .display_only_terminal_content(&current.insertion, cx)
                .contains("current terminal survives stale completion"),
            "rejecting the stale item must preserve current terminal output"
        );
        assert!(front_door.visible_surface_host(cx).is_some());
        SemanticProbe::new(&front_door.snapshot(cx))
            .require_visible("omega-workbench-rail-error")
            .expect("stale completion rejection should be visible in the workbench");

        front_door.focus_agent_panel(cx);
        front_door.dispatch_action(crate::workbench_shell::NewTerminalForThread, cx);
        assert_eq!(
            front_door
                .native_terminal_snapshot(cx)
                .expect("failed request pending snapshot")
                .pending_terminal_count,
            1
        );
        let failed_request = front_door
            .take_terminal_creation_request(cx)
            .expect("second terminal creation request");
        assert_eq!(
            failed_request.working_directory,
            Some(PathBuf::from("/worktree-2"))
        );
        assert!(failed_request.fail("controlled terminal spawn failure"));
        front_door.settle(cx);
        let failed = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("failed terminal snapshot");
        assert_eq!(failed.panel.pending_terminal_count, 0);
        assert_eq!(failed.surface.terminal_owners.len(), 1);
        SemanticProbe::new(&front_door.snapshot(cx))
            .require_visible("omega-workbench-rail-error")
            .expect("spawn failure should be surfaced through the workbench UI");

        front_door
            .teardown(cx)
            .expect("native Terminal deferred-create fixture should tear down");
    }

    #[gpui::test(iterations = 8)]
    async fn native_terminal_front_door_preserves_owner_and_output_across_scope_lifecycle(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_two_worktrees("native_terminal_owner_lifecycle");
        scene.active_surface = Some(WorkSurfaceId::Terminal);
        scene.dock_open = true;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("native Terminal fixture should mount");
        let original_binding = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("initial Terminal binding")
            .surface
            .binding;
        let original = front_door
            .insert_display_only_terminal(true, None, Some(original_binding.clone()), cx)
            .expect("original display-only terminal");
        front_door.write_display_only_terminal_output(
            &original.insertion,
            b"output survives ownership transitions\n",
            cx,
        );
        front_door
            .set_display_only_terminal_lifecycle(
                original.insertion.terminal_id,
                TestTerminalLifecycle::Running { process_id: 4242 },
                cx,
            )
            .expect("display-only terminal should enter running state");
        assert_eq!(
            front_door
                .native_terminal_snapshot(cx)
                .expect("running Terminal snapshot")
                .running_terminal_count(),
            1
        );
        SemanticProbe::new(&front_door.snapshot(cx))
            .require_visible("omega.workbench.badge.terminal")
            .expect("running native lifecycle should drive the Terminal badge");
        front_door
            .set_display_only_terminal_lifecycle(
                original.insertion.terminal_id,
                TestTerminalLifecycle::Exited { exit_code: 0 },
                cx,
            )
            .expect("display-only terminal should enter exited state");
        assert_eq!(
            front_door
                .native_terminal_snapshot(cx)
                .expect("exited Terminal snapshot")
                .running_terminal_count(),
            0
        );
        SemanticProbe::new(&front_door.snapshot(cx))
            .require_absent("omega.workbench.badge.terminal")
            .expect("terminal exit should clear the production-derived badge");

        front_door
            .select_identity_fixture("worktree-2", cx)
            .expect("thread identity should switch worktrees");
        let switched = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("switched Terminal binding");
        assert_ne!(switched.surface.binding, original_binding);
        assert_eq!(
            switched.surface.active_terminal_owner,
            Some((original.insertion.terminal_id, original_binding.clone())),
            "an existing terminal must retain its creation owner"
        );
        let terminal_surface = front_door
            .native_terminal_surface(cx)
            .expect("native Terminal surface");
        let mut visual = VisualTestContext::from_window(front_door.window, cx);
        terminal_surface.update(&mut visual, |surface, cx| {
            surface.record_terminal_owner(
                original.insertion.terminal_id,
                switched.surface.binding.clone(),
                cx,
            );
        });
        assert_eq!(
            front_door
                .native_terminal_front_door_snapshot(cx)
                .expect("Terminal snapshot after conflicting owner registration")
                .surface
                .terminal_owners
                .get(&original.insertion.terminal_id),
            Some(&original_binding),
            "a conflicting owner registration must not relabel an existing terminal"
        );
        front_door.dispatch_action(crate::workbench_shell::SelectTerminal, cx);
        assert!(front_door.visible_surface_host(cx).is_none());
        assert!(
            front_door
                .display_only_terminal_content(&original.insertion, cx)
                .contains("output survives ownership transitions"),
            "collapsing an active work surface must retain terminal output"
        );
        front_door.dispatch_action(crate::workbench_shell::SelectTerminal, cx);

        front_door.set_identity_phase(crate::thread_identity::IdentityPhase::Offline, cx);
        let offline = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("offline Terminal snapshot");
        assert_eq!(
            offline.surface.owner_state,
            crate::workbench_shell::NativeTerminalOwnerState::Offline
        );
        assert!(!offline.surface.owner_state.can_create());
        assert!(!offline.panel.new_terminal_enabled);
        let creation_count = front_door
            .terminal_creation_working_directories(cx)
            .expect("terminal creation log")
            .len();
        front_door
            .invoke_native_terminal_new_handler(cx)
            .expect("workbench New callback should be installed");
        front_door
            .invoke_native_terminal_split_handler(SplitDirection::Right, cx)
            .expect("workbench Split callback should be installed");
        assert_eq!(
            front_door
                .terminal_creation_working_directories(cx)
                .expect("offline terminal creation log")
                .len(),
            creation_count,
            "offline New and Split callbacks must not request a process"
        );
        front_door.set_identity_phase(crate::thread_identity::IdentityPhase::Reconnecting, cx);
        assert_eq!(
            front_door
                .native_terminal_front_door_snapshot(cx)
                .expect("reconnecting Terminal snapshot")
                .surface
                .owner_state,
            crate::workbench_shell::NativeTerminalOwnerState::Reconnecting
        );
        front_door.set_identity_phase(crate::thread_identity::IdentityPhase::Ready, cx);
        assert_eq!(
            front_door
                .native_terminal_front_door_snapshot(cx)
                .expect("reconnected Terminal snapshot")
                .surface
                .owner_state,
            crate::workbench_shell::NativeTerminalOwnerState::Ready
        );
        front_door
            .select_identity_fixture("worktree-1", cx)
            .expect("thread identity should return to original worktree");
        front_door
            .remove_worktree(Path::new("/worktree-1"), cx)
            .expect("selected worktree should be removable");
        let removed = front_door
            .native_terminal_front_door_snapshot(cx)
            .expect("removed-worktree Terminal snapshot");
        assert_eq!(
            removed.surface.owner_state,
            crate::workbench_shell::NativeTerminalOwnerState::WorktreeRemoved
        );
        assert!(!removed.panel.new_terminal_enabled);
        assert!(
            front_door.visible_surface_host(cx).is_some(),
            "removed worktree must retain a visible host for existing terminal output"
        );
        assert_eq!(
            removed
                .surface
                .terminal_owners
                .get(&original.insertion.terminal_id),
            Some(&original_binding)
        );
        assert!(
            front_door
                .display_only_terminal_content(&original.insertion, cx)
                .contains("output survives ownership transitions")
        );
        front_door
            .invoke_native_terminal_new_handler(cx)
            .expect("removed-worktree New callback should remain installed but disabled");
        front_door
            .invoke_native_terminal_split_handler(SplitDirection::Right, cx)
            .expect("removed-worktree Split callback should remain installed but disabled");
        assert_eq!(
            front_door
                .terminal_creation_working_directories(cx)
                .expect("removed-worktree terminal creation log")
                .len(),
            creation_count,
            "removed-worktree callbacks must not request a process"
        );

        front_door
            .teardown(cx)
            .expect("native Terminal lifecycle fixture should tear down");
    }

    #[gpui::test(iterations = 8)]
    async fn native_git_front_door_scopes_exact_repository_and_retains_one_panel(
        cx: &mut TestAppContext,
    ) {
        let scene = scene_with_two_git_repositories("native_git_exact_scope");
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("multi-repository Git fixture should mount");
        let repository_a = front_door
            .set_workspace_active_repository("worktree-1", cx)
            .expect("workspace repository A should become globally active");
        let repository_b = front_door
            .fixture_repository_id("worktree-2", cx)
            .expect("repository B should exist");
        assert_ne!(repository_a, repository_b);
        let workspace_panel_id = front_door
            .native_git_panel(cx)
            .expect("workspace should load one native GitPanel")
            .entity_id();
        front_door.settle_native_git(cx);
        let clean_repository_a = front_door
            .native_git_state(cx)
            .expect("workspace GitPanel should project repository A");
        assert_eq!(clean_repository_a.repository_id, Some(repository_a));
        assert_eq!(clean_repository_a.counts.changes, 0);

        front_door
            .select_identity_fixture("worktree-2", cx)
            .expect("thread should select repository B");
        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);
        front_door
            .set_workspace_active_repository("worktree-1", cx)
            .expect("workspace-global Git state should be reset to repository A");
        front_door.settle_native_git(cx);

        let panel = front_door
            .native_git_panel(cx)
            .expect("native GitPanel should be handed to the work surface");
        let panel_id = panel.entity_id();
        assert_eq!(
            panel_id, workspace_panel_id,
            "the work surface must rehome the existing workspace GitPanel"
        );
        let snapshot = front_door
            .native_git_state(cx)
            .expect("native GitPanel should expose typed state");
        let binding_generation = front_door
            .projection(cx)
            .visible_projection()
            .expect("visible workbench projection")
            .generation;
        let worktree_b = front_door
            .fixture_worktree_id("worktree-2", cx)
            .expect("worktree B should exist");
        assert_eq!(
            snapshot.repository_scope,
            Some(git_ui::git_panel::GitPanelRepositoryScope {
                repository_id: repository_b,
                worktree_id: worktree_b,
                generation: binding_generation,
            })
        );
        assert_eq!(snapshot.repository_id, Some(repository_b));
        assert!(snapshot.repository_scope_available);
        assert_eq!(snapshot.counts.changes, 2);
        assert_eq!(
            snapshot.tracking,
            Some(git_ui::git_panel::GitPanelTrackingSnapshot {
                ahead: 2,
                behind: 1,
            })
        );
        assert!(
            snapshot
                .status_entries
                .iter()
                .all(|entry| entry.repo_path.as_unix_str().starts_with("fixture-status-")),
            "scoped repository B must not render repository A entries"
        );
        assert_eq!(
            front_door.workspace_active_repository_id(cx),
            Some(repository_a),
            "the scoped native panel must not require repository B to remain workspace-global"
        );

        front_door.fs.set_status_for_repo(
            Path::new("/worktree-1/.git"),
            &[("worktree-1-only.txt", git::status::FileStatus::Untracked)],
        );
        front_door
            .set_workspace_active_repository("worktree-1", cx)
            .expect("repository A should emit a late global refresh");
        front_door.settle_native_git(cx);
        let after_repository_a_refresh = front_door
            .native_git_state(cx)
            .expect("repository B should remain mounted");
        assert_eq!(after_repository_a_refresh.repository_id, Some(repository_b));
        assert_eq!(after_repository_a_refresh.counts.changes, 2);
        assert!(
            after_repository_a_refresh
                .status_entries
                .iter()
                .all(|entry| entry.repo_path.as_unix_str().starts_with("fixture-status-")),
            "a late repository A refresh must not publish into repository B's scope"
        );

        front_door
            .select_native_git_path("worktree-2", "fixture-status-0.txt", cx)
            .expect("repository B change should be selectable");
        front_door
            .dispatch_native_git_action(git::StageFile, cx)
            .expect("stage should route to scoped repository B");
        front_door.settle_native_git(cx);
        assert_eq!(
            front_door
                .native_git_state(cx)
                .expect("staged repository B state")
                .counts
                .tracked_staged,
            1
        );

        front_door
            .select_identity_fixture("worktree-1", cx)
            .expect("thread should switch to repository A");
        front_door.settle_native_git(cx);
        let repository_a_state = front_door.native_git_state(cx).expect("repository A state");
        assert_eq!(repository_a_state.repository_id, Some(repository_a));
        assert_eq!(
            repository_a_state.counts.tracked_staged, 0,
            "staging repository B must not mutate repository A"
        );
        assert!(
            repository_a_state
                .status_entries
                .iter()
                .any(|entry| entry.repo_path.as_unix_str() == "worktree-1-only.txt")
        );

        front_door
            .select_identity_fixture("worktree-2", cx)
            .expect("thread should switch back to repository B");
        front_door.settle_native_git(cx);
        let repository_b_state = front_door
            .native_git_state(cx)
            .expect("restored repository B state");
        assert_eq!(repository_b_state.repository_id, Some(repository_b));
        assert_eq!(
            repository_b_state.counts.tracked_staged, 1,
            "repository B's staged state should survive exact scope switches"
        );
        assert_eq!(
            front_door
                .native_git_panel(cx)
                .map(|current| current.entity_id()),
            Some(panel_id)
        );

        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);
        assert!(
            !front_door
                .projection(cx)
                .visible_projection()
                .expect("visible workbench")
                .dock_open,
            "reselecting Git should collapse the dock"
        );
        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);
        front_door.settle_native_git(cx);
        assert_eq!(
            front_door
                .native_git_panel(cx)
                .map(|panel| panel.entity_id()),
            Some(panel_id),
            "collapse and reopen must reuse the exact native GitPanel entity"
        );
        assert_eq!(
            front_door.native_git_lifecycle(cx),
            Some(crate::workbench_shell::NativeGitLifecycle::Dirty)
        );

        front_door
            .teardown(cx)
            .expect("scoped native Git workbench should tear down");
    }

    #[gpui::test(iterations = 8)]
    async fn native_git_front_door_routes_mutations_diff_validation_and_discard_cancel(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("native_git_mutations", 1200, true);
        scene.repositories[0].worktrees[0].dirty_files = 1;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("dirty Git fixture should mount");
        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);
        front_door.settle_native_git(cx);
        front_door
            .select_native_git_path("worktree-1", "fixture-status-0.txt", cx)
            .expect("dirty file should be selectable through the real GitPanel");
        front_door
            .set_native_git_commit_message(" ", cx)
            .expect("commit validation should begin with an explicit blank message");

        let initial = front_door.native_git_state(cx).expect("initial Git state");
        assert_eq!(initial.status_entries.len(), 1);
        assert_eq!(
            initial.status_entries[0].staging,
            git::status::StageStatus::Unstaged
        );
        assert!(!initial.commit_button.enabled);

        front_door
            .dispatch_native_git_action(git::StageFile, cx)
            .expect("StageFile should route to the embedded GitPanel");
        front_door.settle_native_git(cx);
        let staged = front_door.native_git_state(cx).expect("staged Git state");
        assert_eq!(
            staged.status_entries[0].staging,
            git::status::StageStatus::Staged
        );
        assert_eq!(staged.counts.tracked_staged, 1);
        assert!(
            !staged.commit_button.enabled,
            "a blank commit message must keep commit disabled"
        );

        front_door
            .set_native_git_commit_message("Test native Git commit", cx)
            .expect("commit message should use the real GitPanel editor buffer");
        let commit_ready = front_door
            .native_git_state(cx)
            .expect("commit validation state");
        assert!(commit_ready.commit_button.enabled);

        front_door
            .dispatch_native_git_action(git::UnstageFile, cx)
            .expect("UnstageFile should route to the embedded GitPanel");
        front_door.settle_native_git(cx);
        let unstaged = front_door.native_git_state(cx).expect("unstaged Git state");
        assert_eq!(
            unstaged.status_entries[0].staging,
            git::status::StageStatus::Unstaged
        );
        assert_eq!(unstaged.counts.tracked_staged, 0);
        assert!(unstaged.commit_button.enabled);
        assert_eq!(unstaged.commit_button.title, "Commit Tracked");
        front_door
            .set_native_git_commit_message(" ", cx)
            .expect("blank commit validation should remain deterministic after unstaging");
        assert!(
            !front_door
                .native_git_state(cx)
                .expect("blank unstaged commit state")
                .commit_button
                .enabled
        );

        front_door
            .select_native_git_path("worktree-1", "fixture-status-0.txt", cx)
            .expect("dirty file should remain selected");
        front_door
            .dispatch_native_git_action(menu::Confirm, cx)
            .expect("Confirm should open the selected native diff");
        front_door.settle_native_git(cx);
        assert!(
            front_door.workspace_center_is_visible(cx),
            "opening a diff from the embedded GitPanel must reveal the workbench center"
        );
        assert_eq!(
            front_door.active_project_diff_path(cx),
            Some(ProjectPath {
                worktree_id: front_door
                    .fixture_worktree_id("worktree-1", cx)
                    .expect("fixture worktree"),
                path: util::rel_path::rel_path("fixture-status-0.txt").into(),
            })
        );

        front_door
            .select_native_git_path("worktree-1", "fixture-status-0.txt", cx)
            .expect("dirty file should be selectable for discard");
        front_door
            .dispatch_native_git_action(git::RestoreFile::default(), cx)
            .expect("RestoreFile should route to the embedded GitPanel");
        let (message, _) = cx
            .pending_prompt()
            .expect("discarding a tracked change should require confirmation");
        assert!(
            message.contains("fixture-status-0.txt"),
            "discard prompt must name the exact scoped path: {message}"
        );
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        front_door.settle_native_git(cx);
        let after_cancel = front_door
            .native_git_state(cx)
            .expect("Git state after discard cancellation");
        assert_eq!(after_cancel.status_entries.len(), 1);
        assert_eq!(
            front_door
                .fs
                .load(Path::new("/worktree-1/fixture-status-0.txt"))
                .await
                .expect("cancelled discard must leave the fixture file"),
            "fixture"
        );

        front_door
            .teardown(cx)
            .expect("native Git mutation workbench should tear down");
    }

    #[gpui::test(iterations = 8)]
    async fn native_git_front_door_fails_closed_across_offline_reconnect_and_removal(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("native_git_lifecycle", 1200, true);
        scene.repositories[0].worktrees[0].dirty_files = 1;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("Git lifecycle fixture should mount");
        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);
        front_door.settle_native_git(cx);
        let panel = front_door
            .native_git_panel(cx)
            .expect("native GitPanel should mount");
        let surface = front_door
            .native_git_surface(cx)
            .expect("native Git surface should mount");
        assert_eq!(
            surface.read_with(cx, |surface, _cx| surface.lifecycle().clone()),
            crate::workbench_shell::NativeGitLifecycle::Dirty
        );

        front_door.set_identity_phase(crate::thread_identity::IdentityPhase::Offline, cx);
        assert_eq!(
            surface.read_with(cx, |surface, _cx| surface.lifecycle().clone()),
            crate::workbench_shell::NativeGitLifecycle::Offline
        );
        assert!(
            !front_door
                .projection(cx)
                .visible_projection()
                .expect("offline projection")
                .dock_open,
            "offline transition must fail closed instead of leaving mutable Git UI visible"
        );

        front_door.set_identity_phase(crate::thread_identity::IdentityPhase::Reconnecting, cx);
        assert_eq!(
            surface.read_with(cx, |surface, _cx| surface.lifecycle().clone()),
            crate::workbench_shell::NativeGitLifecycle::Reconnecting
        );
        front_door.set_identity_phase(crate::thread_identity::IdentityPhase::Ready, cx);
        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);
        front_door.settle_native_git(cx);
        assert_eq!(
            front_door
                .native_git_panel(cx)
                .map(|current| current.entity_id()),
            Some(panel.entity_id()),
            "reconnect must restore the retained GitPanel instead of manufacturing another"
        );
        assert_eq!(
            surface.read_with(cx, |surface, _cx| surface.lifecycle().clone()),
            crate::workbench_shell::NativeGitLifecycle::Dirty
        );

        front_door
            .remove_worktree(Path::new("/worktree-1"), cx)
            .expect("fixture repository should be removable");
        front_door.settle_native_git(cx);
        let removed = panel.update(cx, |panel, cx| panel.state_snapshot(cx));
        assert!(!removed.repository_scope_available);
        assert_eq!(removed.repository_id, None);
        assert!(removed.status_entries.is_empty());
        assert_eq!(
            surface.read_with(cx, |surface, _cx| surface.lifecycle().clone()),
            crate::workbench_shell::NativeGitLifecycle::RepositoryRemoved
        );

        front_door
            .teardown(cx)
            .expect("native Git lifecycle workbench should tear down");
    }

    #[gpui::test]
    async fn native_git_front_door_projects_detached_unborn_conflict_and_untracked_states(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_two_git_repositories("native_git_edge_states");
        scene.repositories[0].worktrees[0].branch = None;
        scene.repositories[0].worktrees[0].dirty_files = 2;
        scene.repositories[0].worktrees[0].conflicts = 1;
        scene.repositories[1].worktrees[0].git_state = Some(WorktreeGitStateFixture::Unborn);
        scene.repositories[1].worktrees[0].dirty_files = 1;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("Git edge-state fixture should mount");

        front_door
            .fs
            .insert_file(Path::new("/worktree-1/untracked.txt"), b"new".to_vec())
            .await;
        front_door.fs.set_status_for_repo(
            Path::new("/worktree-1/.git"),
            &[
                (
                    "fixture-status-0.txt",
                    git::status::FileStatus::Unmerged(git::status::UnmergedStatus {
                        first_head: git::status::UnmergedStatusCode::Updated,
                        second_head: git::status::UnmergedStatusCode::Updated,
                    }),
                ),
                ("untracked.txt", git::status::FileStatus::Untracked),
            ],
        );
        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);
        front_door.settle_native_git(cx);
        let detached = front_door
            .native_git_state(cx)
            .expect("detached repository state");
        assert_eq!(
            detached.head,
            Some(git_ui::git_panel::GitPanelHeadState::Detached)
        );
        assert_eq!(detached.counts.conflicted, 1);
        assert_eq!(detached.counts.new, 1);
        assert!(
            detached
                .status_entries
                .iter()
                .any(|entry| entry.repo_path.as_unix_str() == "untracked.txt")
        );

        front_door
            .select_identity_fixture("worktree-2", cx)
            .expect("thread should switch to the unborn repository");
        front_door.settle_native_git(cx);
        let unborn = front_door
            .native_git_state(cx)
            .expect("unborn repository state");
        assert!(matches!(
            unborn.head,
            Some(git_ui::git_panel::GitPanelHeadState::Unborn { .. })
        ));
        assert_eq!(
            unborn.repository_id,
            front_door.fixture_repository_id("worktree-2", cx)
        );

        front_door
            .teardown(cx)
            .expect("native Git edge-state workbench should tear down");
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
        assert_eq!(
            front_door
                .projection(cx)
                .visible_projection()
                .and_then(|visible| {
                    front_door
                        .projection(cx)
                        .threads
                        .get(&visible.thread_id)
                        .map(|thread| thread.available_surfaces.clone())
                }),
            Some(std::collections::BTreeSet::from([
                omega_workbench_state::WorkSurface::Files,
                omega_workbench_state::WorkSurface::Search,
                omega_workbench_state::WorkSurface::Terminal,
                omega_workbench_state::WorkSurface::Plan,
            ]))
        );
        for surface in omega_workbench_state::WorkSurface::FALLBACK_ORDER {
            let capability = front_door
                .capability(surface, cx)
                .expect("every rail surface has a typed capability");
            assert_eq!(
                capability.availability.is_available(),
                matches!(
                    surface,
                    omega_workbench_state::WorkSurface::Files
                        | omega_workbench_state::WorkSurface::Search
                        | omega_workbench_state::WorkSurface::Terminal
                        | omega_workbench_state::WorkSurface::Plan
                )
            );
            assert_eq!(capability.badge, None);
        }
        assert_eq!(
            front_door
                .panel()
                .read_with(cx, |panel, _| panel.workbench_focus_target_for_tests()),
            crate::workbench_shell::WorkbenchFocusTarget::Transcript
        );

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
        assert_eq!(
            front_door
                .projection(cx)
                .visible_projection()
                .and_then(|visible| {
                    front_door
                        .projection(cx)
                        .threads
                        .get(&visible.thread_id)
                        .map(|thread| thread.available_surfaces.clone())
                }),
            Some(omega_workbench_state::WorkSurface::FALLBACK_ORDER.into())
        );

        front_door
            .teardown(cx)
            .expect("no-Git and unborn scene should tear down");
    }

    #[gpui::test(iterations = 8)]
    async fn editor_navigation_can_return_to_same_agent_panel_without_mutating_workbench(
        cx: &mut TestAppContext,
    ) {
        let mut scene = scene_with_thread("editor_agent_panel_capture_return", 1200, true);
        scene.active_surface = Some(WorkSurfaceId::Files);
        scene.dock_open = true;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("capture-return fixture should mount");
        front_door
            .focus_and_select_files_path("worktree-1", "src/main.rs", cx)
            .expect("fixture file should be selectable");
        let panel_id = front_door.panel().entity_id();
        let projection_before = front_door.projection(cx);

        front_door.dispatch_action(project_panel::OpenPermanent, cx);
        assert_eq!(
            front_door
                .active_workspace_item_path(cx)
                .map(|path| path.path),
            Some(util::rel_path::rel_path("src/main.rs").into())
        );
        assert_eq!(
            front_door.active_workspace_selection(cx),
            Some((language::Point::new(0, 0), language::Point::new(0, 0)))
        );
        assert!(front_door.active_workspace_item_is_focused(cx));

        front_door.return_to_agent_panel_for_capture(cx);
        assert_eq!(front_door.panel().entity_id(), panel_id);
        assert_eq!(front_door.projection(cx), projection_before);

        front_door
            .teardown(cx)
            .expect("capture-return workbench should tear down");
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
        let files_panel_id = front_door
            .native_files_panel_entity_id(cx)
            .expect("workspace ProjectPanel");
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert!(
            front_door.native_files_is_focused(cx),
            "inconsistent transition proof requires focus to begin inside Files"
        );
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
        probe
            .require_absent(WORKBENCH_DOCK_SELECTOR)
            .expect("an inconsistent repository authority must close Files");
        probe
            .require_absent("omega.project-panel.tree")
            .expect("an inconsistent repository authority must hide the native tree");
        assert!(
            front_door.transcript_activation_is_focused(cx),
            "inconsistent Files must transfer actual focus to the transcript"
        );
        assert!(
            !front_door.native_files_is_focused(cx),
            "the hidden native tree must not retain focus while identity is inconsistent"
        );
        for surface in [
            omega_workbench_state::WorkSurface::Files,
            omega_workbench_state::WorkSurface::Search,
            omega_workbench_state::WorkSurface::Review,
            omega_workbench_state::WorkSurface::Git,
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
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert_eq!(
            front_door.native_files_panel_entity_id(cx),
            Some(files_panel_id),
            "inconsistent recovery must retain the workspace-created ProjectPanel"
        );
        assert_eq!(
            front_door.mounted_files_panel_entity_id(cx),
            Some(files_panel_id),
            "recovered Files must mount the same native ProjectPanel"
        );
        assert!(
            front_door.native_files_is_focused(cx),
            "reopening Files after reconciliation must focus the recovered native tree"
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
        let crate::thread_identity::IdentityPhase::Inconsistent(ref message) = identity.phase
        else {
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
        front_door.settle(cx);
        let cap = front_door
            .capability(omega_workbench_state::WorkSurface::Git, cx)
            .expect("Git capability");
        eprintln!(
            "DEBUG cap availability: {:?}, identity={:?}",
            cap.availability, identity
        );
        assert!(
            !cap.availability.is_available(),
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
        for (selector, _) in snapshot.selectors() {
            if selector.contains("surface")
                || selector.contains("git")
                || selector.contains("workbench")
            {
                eprintln!("DEBUG selector: {}", selector);
            }
        }
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
        let _snapshot = front_door.snapshot(cx);
        let identity_after_render = front_door
            .identity(cx)
            .expect("missing identity should survive rendering");
        assert_eq!(
            identity_after_render.phase,
            crate::thread_identity::IdentityPhase::Missing
        );
        let _reopened_snapshot = front_door.snapshot(cx);
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
        front_door.focus_agent_panel(cx);
        let snapshot = front_door.snapshot(cx);
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
    async fn files_mounts_native_panel_reuses_entities_and_opens_files(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_native_files", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("native Files fixture should mount");
        let transcript_id = front_door
            .transcript_entity_id(cx)
            .expect("Files fixture should have a transcript");
        let files_panel_id = front_door
            .native_files_panel_entity_id(cx)
            .expect("the workspace should own one native ProjectPanel");
        let worktree_id = front_door
            .fixture_worktree_id("worktree-1", cx)
            .expect("fixture worktree should be visible");

        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);

        let projection = front_door
            .projection(cx)
            .visible_projection()
            .expect("Files projection should be visible");
        assert_eq!(
            projection.effective_surface,
            Some(omega_workbench_state::WorkSurface::Files)
        );
        assert!(projection.dock_open);
        assert_eq!(front_door.native_files_scope(cx), Some(worktree_id));
        assert!(matches!(
            front_door.native_files_scope_state(cx),
            Some(project_panel::ProjectPanelScopeState::Ready {
                worktree_id: ready_worktree_id,
                ..
            }) if ready_worktree_id == worktree_id
        ));
        let rows = front_door.native_files_rows(cx);
        assert!(
            !rows.is_empty(),
            "the native tree should project fixture rows"
        );
        assert!(
            rows.iter().all(|row| row.worktree_id == worktree_id),
            "Files must render rows from the active worktree only"
        );
        assert!(
            rows.iter()
                .any(|row| row.path.as_unix_str() == "worktree-1-only.txt"),
            "the active-worktree-only sentinel should be rendered"
        );
        assert!(
            front_door.native_files_is_focused(cx),
            "selecting Files should focus the native ProjectPanel"
        );
        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        assert_eq!(
            front_door.mounted_files_panel_entity_id(cx),
            Some(files_panel_id),
            "the Files host must mount the workspace-owned ProjectPanel entity"
        );
        probe
            .require_accessible("omega.project-panel.tree", "Tree", "Files")
            .expect("the embedded native Files tree should remain accessible");
        probe
            .require_inside(
                "omega.project-panel.tree",
                WorkSurfaceId::Files.surface_selector(),
            )
            .expect("the native tree should render inside the Files work-surface host");

        front_door
            .focus_and_select_files_path("worktree-1", "src", cx)
            .expect("select the src directory");
        front_door.dispatch_action(project_panel::ExpandSelectedEntry, cx);
        front_door
            .focus_and_select_files_path("worktree-1", "src/main.rs", cx)
            .expect("select the file that will be opened");
        let expanded_src = front_door
            .native_files_rows(cx)
            .into_iter()
            .find(|row| row.path.as_unix_str() == "src")
            .expect("expanded src row should remain visible");
        assert!(expanded_src.is_expanded);
        let main_row = front_door
            .native_files_rows(cx)
            .into_iter()
            .find(|row| row.path.as_unix_str() == "src/main.rs")
            .expect("expanded tree should expose src/main.rs");
        let expanded_snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&expanded_snapshot);
        let expanded_src_selector = expanded_src.selector();
        probe
            .require_accessibility_property(
                &expanded_src_selector,
                "expanded",
                serde_json::Value::Bool(true),
            )
            .expect("directory expansion should be observable without pixels");
        let main_row_selector = main_row.selector();
        probe
            .require_accessible(&main_row_selector, "TreeItem", "src/main.rs")
            .expect("the selected native file row should be accessible");

        let host_id = front_door
            .surface_host_entity_id(omega_workbench_state::WorkSurface::Files, cx)
            .expect("Files should have one retained host");
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert!(
            front_door
                .snapshot(cx)
                .bounds(WORKBENCH_DOCK_SELECTOR)
                .is_none(),
            "selecting active Files should collapse the dock"
        );
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert_eq!(
            front_door.surface_host_entity_id(omega_workbench_state::WorkSurface::Files, cx),
            Some(host_id),
            "collapse and reopen should retain the Files host"
        );
        assert_eq!(
            front_door.native_files_panel_entity_id(cx),
            Some(files_panel_id),
            "collapse and reopen should reuse the workspace ProjectPanel entity"
        );
        assert_eq!(
            front_door
                .native_files_selected_path(cx)
                .map(|path| path.path),
            Some(util::rel_path::rel_path("src/main.rs").into()),
            "native tree selection should survive collapse and reopen"
        );
        assert!(
            front_door
                .native_files_rows(cx)
                .iter()
                .any(|row| row.path.as_unix_str() == "src" && row.is_expanded),
            "native expansion state should survive collapse and reopen"
        );

        front_door.focus_native_files(cx);
        front_door.dispatch_action(project_panel::Open, cx);
        assert_eq!(
            front_door.active_workspace_item_path(cx),
            front_door.native_files_selected_path(cx),
            "opening a Files row should use the workspace's native file-open path"
        );
        assert!(
            front_door.workspace_center_is_visible(cx),
            "native Files open must leave the workspace center rendered"
        );
        assert!(
            front_door.native_files_is_focused(cx),
            "preview open must preserve the native Files focus contract"
        );
        front_door.dispatch_action(project_panel::OpenPermanent, cx);
        assert!(
            front_door.active_workspace_item_is_focused(cx),
            "permanent open must render and focus the native editor"
        );
        assert_eq!(
            front_door.transcript_entity_id(cx),
            Some(transcript_id),
            "native file open must not recreate the active transcript"
        );

        front_door
            .teardown(cx)
            .expect("native Files scene should tear down");
    }

    #[gpui::test]
    async fn reveal_reopens_collapsed_files_and_focuses_the_reused_native_tree(
        cx: &mut TestAppContext,
    ) {
        let scene = scene_with_thread("workbench_files_reveal", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("Files reveal fixture should mount");
        let files_panel_id = front_door
            .native_files_panel_entity_id(cx)
            .expect("workspace ProjectPanel");

        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert!(
            front_door.transcript_activation_is_focused(cx),
            "collapsing Files should focus the transcript before reveal"
        );
        front_door.activate_files_panel(cx);
        assert_eq!(
            front_door.mounted_files_panel_entity_id(cx),
            Some(files_panel_id),
            "ActivateProjectPanel must reopen the exact rehomed native entity"
        );
        assert!(
            front_door.native_files_is_focused(cx),
            "ActivateProjectPanel must focus the available native tree"
        );
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert!(front_door.transcript_activation_is_focused(cx));

        let revealed_path = front_door
            .reveal_files_path("worktree-1", "src/main.rs", cx)
            .expect("emit the native ProjectPanel reveal event");
        let projection = front_door
            .projection(cx)
            .visible_projection()
            .expect("Files reveal projection");
        assert_eq!(
            projection.effective_surface,
            Some(omega_workbench_state::WorkSurface::Files)
        );
        assert!(projection.dock_open, "native reveal should reopen Files");
        assert_eq!(
            front_door.native_files_panel_entity_id(cx),
            Some(files_panel_id),
            "reveal must retain the workspace-created ProjectPanel"
        );
        assert_eq!(
            front_door.mounted_files_panel_entity_id(cx),
            Some(files_panel_id),
            "the reopened host must mount the same ProjectPanel entity"
        );
        assert_eq!(
            front_door.native_files_selected_path(cx),
            Some(revealed_path.clone()),
            "native reveal must select its exact project path"
        );
        assert!(
            front_door.native_files_is_focused(cx),
            "native reveal must transfer focus into the reopened tree"
        );

        let revealed_row = front_door
            .native_files_rows(cx)
            .into_iter()
            .find(|row| row.path == revealed_path.path)
            .expect("revealed native row should be visible");
        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_unique("omega.workbench.root")
            .expect("native reveal must leave the outer AgentPanel rendered");
        probe
            .require_inside(
                "omega.project-panel.tree",
                WorkSurfaceId::Files.surface_selector(),
            )
            .expect("the revealed tree should render inside Files");
        probe
            .require_accessibility_property(
                &revealed_row.selector(),
                "selected",
                serde_json::Value::Bool(true),
            )
            .expect("the revealed row should expose its native selected state");

        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert!(
            front_door.transcript_activation_is_focused(cx),
            "collapsing the revealed tree should return focus to the transcript"
        );
        front_door.dispatch_action(omega_actions::project_panel::ToggleFocus, cx);
        assert_eq!(
            front_door.mounted_files_panel_entity_id(cx),
            Some(files_panel_id),
            "the global ProjectPanel action must route to the rehomed native entity"
        );
        assert!(
            front_door.native_files_is_focused(cx),
            "the global ProjectPanel action must reopen and focus Files"
        );
        front_door.dispatch_action(omega_actions::project_panel::ToggleFocus, cx);
        let snapshot = front_door.snapshot(cx);
        SemanticProbe::new(&snapshot)
            .require_absent(WORKBENCH_DOCK_SELECTOR)
            .expect("global ToggleFocus should collapse focused Files");
        assert!(
            front_door.transcript_activation_is_focused(cx),
            "global ToggleFocus must leave focus in the visible transcript, not the hidden center"
        );
        front_door.dispatch_action(omega_actions::project_panel::ToggleFocus, cx);
        assert_eq!(
            front_door.mounted_files_panel_entity_id(cx),
            Some(files_panel_id),
            "global ToggleFocus must continue reusing the rehomed native entity"
        );
        assert!(
            front_door.native_files_is_focused(cx),
            "global ToggleFocus must refocus Files after reopening it"
        );
        front_door.dispatch_action(workspace::CloseActiveDock, cx);
        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_unique("omega.workbench.root")
            .expect("CloseActiveDock must leave the outer AgentPanel visible");
        probe
            .require_absent(WORKBENCH_DOCK_SELECTOR)
            .expect("CloseActiveDock must collapse the embedded Files dock");
        assert!(
            front_door.transcript_activation_is_focused(cx),
            "CloseActiveDock must transfer focus from Files to the visible transcript"
        );

        front_door.set_identity_phase(crate::thread_identity::IdentityPhase::Missing, cx);
        front_door.snapshot(cx);
        assert!(matches!(
            front_door.native_files_scope_state(cx),
            Some(project_panel::ProjectPanelScopeState::Unavailable)
        ));
        front_door.activate_files_panel(cx);
        assert!(
            front_door.transcript_activation_is_focused(cx),
            "ActivateProjectPanel must leave focus in the transcript without repository authority"
        );
        front_door.dispatch_action(omega_actions::project_panel::ToggleFocus, cx);
        assert!(
            front_door.transcript_activation_is_focused(cx),
            "ToggleFocus must not focus the unavailable rehomed tree"
        );
        front_door.dispatch_action(git::FileHistory, cx);
        assert!(
            front_door.git_graph_file_history_paths(cx).is_empty(),
            "FileHistory must be a no-op while the ProjectPanel authority is unavailable"
        );

        front_door.set_identity_phase(crate::thread_identity::IdentityPhase::Ready, cx);
        front_door.snapshot(cx);
        front_door.dispatch_action(omega_actions::project_panel::ToggleFocus, cx);
        front_door
            .focus_and_select_files_path("worktree-1", "src/main.rs", cx)
            .expect("restore the recovered native selection");
        front_door.dispatch_action(git::FileHistory, cx);
        assert_eq!(
            front_door.git_graph_file_history_paths(cx),
            vec!["src/main.rs".to_string()],
            "FileHistory must resolve the exact selection from the rehomed native tree"
        );
        assert!(
            front_door.workspace_center_is_visible(cx),
            "native FileHistory must leave the workspace center rendered"
        );
        assert!(
            front_door.active_workspace_item_is_focused(cx),
            "the native FileHistory graph must be rendered and focused"
        );

        front_door
            .teardown(cx)
            .expect("Files reveal scene should tear down");
    }

    #[gpui::test(iterations = 16)]
    async fn files_keyboard_actions_route_through_the_focused_native_tree(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_files_keyboard", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("Files keyboard fixture should mount");

        front_door.dispatch_action(crate::workbench_shell::FocusActivityRail, cx);
        front_door.dispatch_action(crate::workbench_shell::FocusFirstSurface, cx);
        let rail_snapshot = front_door.snapshot(cx);
        SemanticProbe::new(&rail_snapshot)
            .require_focus(WorkSurfaceId::Files.rail_selector(), true)
            .expect("the first rail action should focus Files");

        front_door.dispatch_action(crate::workbench_shell::ActivateFocusedSurface, cx);
        assert!(
            front_door.native_files_is_focused(cx),
            "activating the focused rail item should transfer GPUI focus into ProjectPanel"
        );

        let src_row = front_door
            .native_files_rows(cx)
            .into_iter()
            .find(|row| row.path.as_unix_str() == "src")
            .expect("Files fixture should render src");
        front_door
            .click(&src_row.selector(), cx)
            .expect("the rendered src row should be selectable");
        assert_eq!(
            front_door
                .native_files_selected_path(cx)
                .map(|path| path.path),
            Some(util::rel_path::rel_path("src").into())
        );

        front_door.dispatch_action(project_panel::ExpandSelectedEntry, cx);
        front_door.dispatch_action(menu::SelectNext, cx);
        let selected = front_door
            .native_files_selected_path(cx)
            .expect("Down/SelectNext should select a child row through ProjectPanel");
        assert_ne!(
            selected.path.as_unix_str(),
            "src",
            "keyboard navigation must move the native selection"
        );
        assert!(
            selected.path.starts_with(util::rel_path::rel_path("src")),
            "the next native row should come from the expanded src directory"
        );
        let selected_row = front_door
            .native_files_rows(cx)
            .into_iter()
            .find(|row| row.path == selected.path)
            .expect("keyboard-selected row should remain visible");
        let selected_snapshot = front_door.snapshot(cx);
        SemanticProbe::new(&selected_snapshot)
            .require_accessibility_property(
                &selected_row.selector(),
                "selected",
                serde_json::Value::Bool(true),
            )
            .expect("keyboard selection should be exposed through tree-item accessibility");

        front_door.dispatch_action(project_panel::Open, cx);
        assert_eq!(
            front_door.active_workspace_item_path(cx),
            Some(selected),
            "Open should continue routing to the focused embedded ProjectPanel"
        );

        front_door
            .teardown(cx)
            .expect("Files keyboard fixture should tear down");
    }

    #[gpui::test(iterations = 16)]
    async fn files_rebinds_atomically_and_rejects_stale_filesystem_events(cx: &mut TestAppContext) {
        let scene = scene_with_two_worktrees("workbench_files_rebind");
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("multi-worktree Files fixture should mount");
        let worktree_a = front_door
            .fixture_worktree_id("worktree-1", cx)
            .expect("first fixture worktree");
        let worktree_b = front_door
            .fixture_worktree_id("worktree-2", cx)
            .expect("second fixture worktree");
        let files_panel_id = front_door
            .native_files_panel_entity_id(cx)
            .expect("workspace ProjectPanel");

        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        let host_a = front_door
            .surface_host_entity_id(omega_workbench_state::WorkSurface::Files, cx)
            .expect("first binding should own a Files host");
        let old_rows = front_door.native_files_rows(cx);
        assert!(
            old_rows.iter().all(|row| row.worktree_id == worktree_a),
            "the first binding must not project another worktree"
        );
        let old_selectors = old_rows
            .iter()
            .map(project_panel::ProjectPanelVisibleRow::selector)
            .collect::<Vec<_>>();

        front_door.fs.pause_events();
        front_door
            .fs
            .insert_file(
                "/worktree-1/stale-after-switch.rs",
                b"// stale event".to_vec(),
            )
            .await;
        front_door
            .select_worktree_picker_row(1, cx)
            .expect("switch to the second worktree through the rendered picker");
        front_door.fs.simulate_watcher_overflow("/worktree-1");
        front_door.fs.unpause_events_and_flush();
        cx.run_until_parked();

        let projection = front_door
            .projection(cx)
            .visible_projection()
            .expect("rebound Files projection");
        assert_eq!(
            projection.effective_surface,
            Some(omega_workbench_state::WorkSurface::Files)
        );
        assert!(
            projection.dock_open,
            "a binding change should restore the active Files surface for the new binding"
        );
        assert_eq!(front_door.native_files_scope(cx), Some(worktree_b));
        assert_eq!(
            front_door.native_files_panel_entity_id(cx),
            Some(files_panel_id),
            "worktree rebinding should reuse the native ProjectPanel entity"
        );
        assert_eq!(
            front_door.mounted_files_panel_entity_id(cx),
            Some(files_panel_id),
            "the rebound host must mount the same workspace-owned ProjectPanel"
        );
        let host_b = front_door
            .surface_host_entity_id(omega_workbench_state::WorkSurface::Files, cx)
            .expect("new binding should own a Files host");
        assert_ne!(
            host_b, host_a,
            "binding-scoped hosts must not be reused across worktrees"
        );
        let rows = front_door.native_files_rows(cx);
        assert!(!rows.is_empty());
        assert!(
            rows.iter().all(|row| row.worktree_id == worktree_b),
            "late worktree-A events must not repopulate the rebound native tree"
        );
        assert!(
            rows.iter()
                .any(|row| row.path.as_unix_str() == "worktree-2-only.txt")
        );
        assert!(
            rows.iter()
                .all(|row| row.path.as_unix_str() != "worktree-1-only.txt"
                    && row.path.as_unix_str() != "stale-after-switch.rs")
        );
        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        for selector in old_selectors {
            probe
                .require_absent(&selector)
                .expect("old-binding row selectors must disappear after rebind");
        }

        front_door
            .teardown(cx)
            .expect("rebound Files scene should tear down");
    }

    #[gpui::test]
    async fn files_thread_switch_restores_binding_scoped_hosts_and_tree_state(
        cx: &mut TestAppContext,
    ) {
        let scene = scene_with_two_worktrees("workbench_files_thread_switch");
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("Files thread-switch fixture should mount");
        let worktree_a = front_door
            .fixture_worktree_id("worktree-1", cx)
            .expect("first fixture worktree");
        let worktree_b = front_door
            .fixture_worktree_id("worktree-2", cx)
            .expect("second fixture worktree");
        let files_panel_id = front_door
            .native_files_panel_entity_id(cx)
            .expect("workspace ProjectPanel");

        let thread_a = front_door.open_external_thread(StubAgentConnection::new(), cx);
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        let host_a = front_door
            .surface_host_entity_id(omega_workbench_state::WorkSurface::Files, cx)
            .expect("thread A should create a Files host");
        front_door
            .focus_and_select_files_path("worktree-1", "src", cx)
            .expect("select thread A directory");
        front_door.dispatch_action(project_panel::ExpandSelectedEntry, cx);

        let thread_b = front_door.open_external_thread(StubAgentConnection::new(), cx);
        assert_ne!(thread_a, thread_b);
        assert!(
            !front_door
                .projection(cx)
                .visible_projection()
                .expect("thread B projection")
                .dock_open,
            "a new thread must not inherit thread A's open Files dock"
        );
        front_door
            .select_worktree_picker_row(1, cx)
            .expect("bind thread B to the second worktree");
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        let host_b = front_door
            .surface_host_entity_id(omega_workbench_state::WorkSurface::Files, cx)
            .expect("thread B should create a Files host");
        assert_ne!(host_a, host_b, "Files hosts must remain thread scoped");
        assert_eq!(front_door.native_files_scope(cx), Some(worktree_b));
        assert!(
            front_door
                .native_files_rows(cx)
                .iter()
                .any(|row| row.path.as_unix_str() == "src" && !row.is_expanded),
            "thread B should start with its own collapsed source directory"
        );

        front_door.activate_thread(thread_a, cx);
        assert_eq!(
            front_door.surface_host_entity_id(omega_workbench_state::WorkSurface::Files, cx),
            Some(host_a),
            "thread A should restore its retained Files host"
        );
        assert_eq!(front_door.native_files_scope(cx), Some(worktree_a));
        assert_eq!(
            front_door.mounted_files_panel_entity_id(cx),
            Some(files_panel_id),
            "thread restoration must keep reusing the one native ProjectPanel"
        );
        assert!(
            front_door
                .native_files_rows(cx)
                .iter()
                .all(|row| row.worktree_id == worktree_a),
            "thread A restoration must not leak thread B rows"
        );
        assert!(
            front_door
                .native_files_rows(cx)
                .iter()
                .any(|row| row.path.as_unix_str() == "src" && row.is_expanded),
            "thread A should restore its worktree-keyed expansion state"
        );

        front_door.activate_thread(thread_b, cx);
        assert_eq!(
            front_door.surface_host_entity_id(omega_workbench_state::WorkSurface::Files, cx),
            Some(host_b),
            "thread B should restore its retained Files host"
        );
        assert_eq!(front_door.native_files_scope(cx), Some(worktree_b));
        assert!(
            front_door
                .native_files_rows(cx)
                .iter()
                .all(|row| row.worktree_id == worktree_b),
            "thread B restoration must not leak thread A rows"
        );
        assert!(
            front_door
                .native_files_rows(cx)
                .iter()
                .any(|row| row.path.as_unix_str() == "src" && !row.is_expanded),
            "thread B should restore its independent collapsed expansion state"
        );

        front_door
            .teardown(cx)
            .expect("Files thread-switch fixture should tear down");
    }

    #[gpui::test]
    async fn files_root_removal_clears_scope_rows_and_visible_host(cx: &mut TestAppContext) {
        let scene = scene_with_two_worktrees("workbench_files_root_removal");
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("Files root-removal fixture should mount");
        let removed_worktree = front_door
            .fixture_worktree_id("worktree-1", cx)
            .expect("selected fixture worktree");
        let files_panel_id = front_door
            .native_files_panel_entity_id(cx)
            .expect("workspace ProjectPanel");
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert!(
            front_door.native_files_is_focused(cx),
            "root-removal proof requires focus to begin inside Files"
        );
        let old_rows = front_door.native_files_rows(cx);
        let selected = front_door
            .identity(cx)
            .and_then(|identity| identity.selected)
            .expect("selected identity before root removal");

        front_door
            .remove_worktree(&selected.worktree_abs_path, cx)
            .expect("remove the Files worktree");

        assert_eq!(
            front_door.identity(cx).map(|identity| identity.phase),
            Some(crate::thread_identity::IdentityPhase::Missing)
        );
        let projection = front_door
            .projection(cx)
            .visible_projection()
            .expect("projection after Files root removal");
        assert_eq!(projection.binding, None);
        assert!(!projection.dock_open);
        assert_eq!(
            front_door
                .capability(omega_workbench_state::WorkSurface::Files, cx)
                .and_then(|capability| capability.availability.reason().cloned()),
            Some("The selected worktree is missing".into())
        );
        assert_eq!(
            front_door.native_files_panel_entity_id(cx),
            Some(files_panel_id),
            "root removal should retain the workspace-owned ProjectPanel for recovery"
        );
        assert_eq!(front_door.native_files_scope(cx), None);
        assert!(
            front_door
                .native_files_rows(cx)
                .iter()
                .all(|row| row.worktree_id != removed_worktree),
            "unscoping must clear rows from the removed authority"
        );
        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_absent(WORKBENCH_DOCK_SELECTOR)
            .expect("root removal should hide the Files host");
        for row in old_rows {
            let row_selector = row.selector();
            probe
                .require_absent(&row_selector)
                .expect("removed-root rows must no longer render");
        }
        assert_eq!(
            front_door
                .panel()
                .read_with(cx, |panel, _| panel.workbench_host_count_for_tests()),
            0,
            "root removal should release binding-scoped Files hosts"
        );
        assert!(
            front_door.transcript_activation_is_focused(cx),
            "root removal must transfer actual GPUI focus to the transcript composer"
        );
        assert!(
            !front_door.native_files_is_focused(cx),
            "the unrendered ProjectPanel must not retain focus after root removal"
        );

        front_door
            .teardown(cx)
            .expect("Files root-removal scene should tear down");
    }

    #[gpui::test]
    async fn files_host_creation_failure_rolls_back_without_orphaning_native_panel(
        cx: &mut TestAppContext,
    ) {
        let scene = scene_with_thread("workbench_files_host_failure", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("Files host-failure fixture should mount");
        let before = front_door.projection(cx);
        let files_panel_id = front_door
            .native_files_panel_entity_id(cx)
            .expect("workspace ProjectPanel");
        assert_eq!(
            front_door.native_files_scope_state(cx),
            Some(project_panel::ProjectPanelScopeState::Unscoped),
            "the legacy workspace panel must remain genuinely unscoped before handoff"
        );

        front_door.fail_next_host_creation(omega_workbench_state::WorkSurface::Files, cx);
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);

        assert_eq!(
            front_door.projection(cx),
            before,
            "failed Files host creation should roll the projection back atomically"
        );
        assert_eq!(
            front_door
                .panel()
                .read_with(cx, |panel, _| panel.workbench_host_count_for_tests()),
            0
        );
        assert_eq!(
            front_door.native_files_panel_entity_id(cx),
            Some(files_panel_id),
            "host construction failure must not replace the native ProjectPanel"
        );
        assert_eq!(
            front_door.native_files_scope(cx),
            None,
            "failed handoff must restore the native panel's prior unscoped state"
        );
        let failed = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&failed);
        probe
            .require_absent(WORKBENCH_DOCK_SELECTOR)
            .expect("failed Files host creation must not open an empty dock");
        probe
            .require_visible("omega-workbench-rail-error")
            .expect("failed Files host creation should be visible");

        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert!(
            front_door
                .surface_host_entity_id(omega_workbench_state::WorkSurface::Files, cx)
                .is_some(),
            "a subsequent Files request should recover"
        );
        assert_eq!(
            front_door.native_files_panel_entity_id(cx),
            Some(files_panel_id),
            "recovery should still use the original native ProjectPanel"
        );

        front_door
            .teardown(cx)
            .expect("Files host-failure scene should tear down");
    }

    #[gpui::test]
    async fn files_rename_rejects_duplicate_and_preserves_filesystem_until_confirmed(
        cx: &mut TestAppContext,
    ) {
        let scene = scene_with_thread("workbench_files_rename", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("Files rename fixture should mount");
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        front_door
            .focus_and_select_files_path("worktree-1", "src", cx)
            .expect("select src");
        front_door.dispatch_action(project_panel::ExpandSelectedEntry, cx);
        front_door
            .focus_and_select_files_path("worktree-1", "src/rename-me.rs", cx)
            .expect("select rename fixture");

        front_door.dispatch_action(project_panel::Rename, cx);
        assert!(
            front_door.native_files_has_edit_state(cx),
            "Rename should enter the native ProjectPanel editor"
        );
        cx.simulate_input(front_door.window, "existing");
        front_door.dispatch_action(menu::Confirm, cx);

        assert!(
            front_door.native_files_has_edit_state(cx),
            "a conflicting native rename should retain edit state for correction"
        );
        let validation_snapshot = front_door.snapshot(cx);
        SemanticProbe::new(&validation_snapshot)
            .require_accessible(
                "omega.project-panel.validation",
                "Alert",
                "File or directory 'existing.rs' already exists at location. Please choose a different name.",
            )
            .expect("duplicate rename validation should be semantically observable");
        let fixture_fs = &*front_door.fs as &dyn fs::Fs;
        assert!(
            fixture_fs
                .metadata(Path::new("/worktree-1/src/rename-me.rs"))
                .await
                .expect("read original rename fixture metadata")
                .is_some(),
            "the rejected rename must preserve the original path"
        );
        assert!(
            fixture_fs
                .metadata(Path::new("/worktree-1/src/existing.rs"))
                .await
                .expect("read conflicting fixture metadata")
                .is_some(),
            "the rejected rename must not replace the conflicting path"
        );

        front_door.dispatch_action(menu::Cancel, cx);
        assert!(
            !front_door.native_files_has_edit_state(cx),
            "Cancel should leave the native rename editor"
        );

        front_door
            .focus_and_select_files_path("worktree-1", "src/main.rs", cx)
            .expect("select the filesystem-error fixture");
        front_door.dispatch_action(project_panel::Rename, cx);
        cx.simulate_input(front_door.window, "unavailable");
        let notifications_before = front_door.workspace_notification_count(cx);
        front_door.fs.pause_events();
        fixture_fs
            .remove_file(
                Path::new("/worktree-1/src/main.rs"),
                fs::RemoveOptions::default(),
            )
            .await
            .expect("remove the rename source behind the native panel");
        front_door.dispatch_action(menu::Confirm, cx);
        cx.run_until_parked();
        front_door.fs.unpause_events_and_flush();
        cx.run_until_parked();

        assert!(
            !front_door.native_files_has_edit_state(cx),
            "a failed native filesystem rename should finish its in-flight editor"
        );
        assert!(
            fixture_fs
                .metadata(Path::new("/worktree-1/src/unavailable.rs"))
                .await
                .expect("read failed rename destination metadata")
                .is_none(),
            "a native filesystem failure must not manufacture the requested destination"
        );
        assert!(
            front_door.workspace_notification_count(cx) > notifications_before,
            "the native async action error must surface through the workspace notification path"
        );

        front_door
            .focus_and_select_files_path("worktree-1", "src/rename-me.rs", cx)
            .expect("reselect rename fixture");
        front_door.dispatch_action(project_panel::Rename, cx);
        cx.simulate_input(front_door.window, "renamed");
        front_door.dispatch_action(menu::Confirm, cx);
        cx.run_until_parked();

        assert!(
            fixture_fs
                .metadata(Path::new("/worktree-1/src/rename-me.rs"))
                .await
                .expect("read old path after rename")
                .is_none(),
            "successful native rename should remove the old path"
        );
        assert!(
            fixture_fs
                .metadata(Path::new("/worktree-1/src/renamed.rs"))
                .await
                .expect("read new path after rename")
                .is_some(),
            "successful native rename should create the requested path"
        );
        assert!(
            !front_door.native_files_has_edit_state(cx),
            "successful native rename should finish editing"
        );
        assert_eq!(
            front_door
                .native_files_selected_path(cx)
                .map(|path| path.path),
            Some(util::rel_path::rel_path("src/renamed.rs").into()),
            "native selection should follow the renamed entry"
        );

        front_door
            .teardown(cx)
            .expect("Files rename scene should tear down");
    }

    #[gpui::test]
    async fn files_loading_error_and_offline_states_are_explicit(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_files_states", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("Files state fixture should mount");
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        let files_panel_id = front_door
            .native_files_panel_entity_id(cx)
            .expect("workspace ProjectPanel");
        let host = front_door
            .visible_surface_host(cx)
            .expect("Files host should be visible");
        front_door
            .focus_and_select_files_path("worktree-1", "src/main.rs", cx)
            .expect("select native state that transient failures must preserve");
        let selected_path = front_door
            .native_files_selected_path(cx)
            .expect("native state selection");

        let failed_load = front_door
            .begin_surface_load(
                "files-load-error",
                omega_workbench_state::WorkSurface::Files,
                cx,
            )
            .expect("begin Files load");
        assert_eq!(
            host.read_with(cx, |host, _| host.content_state().clone()),
            crate::workbench_shell::SurfaceContentState::Loading
        );
        let loading = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&loading);
        probe
            .require_absent("omega.project-panel.tree")
            .expect("host loading state should not expose stale native tree rows");
        probe
            .require_accessible(
                "omega.workbench.surface.files.status",
                "Status",
                "Loading Files…",
            )
            .expect("Files loading should be semantically observable");
        probe
            .require_focus(WorkSurfaceId::Files.surface_selector(), true)
            .expect("loading should move focus from the hidden tree to the surface host");
        assert!(
            !front_door.native_files_is_focused(cx),
            "loading must not leave focus on the unrendered ProjectPanel"
        );
        assert!(matches!(
            front_door.native_files_scope_state(cx),
            Some(project_panel::ProjectPanelScopeState::Unavailable)
        ));
        front_door.activate_files_panel(cx);
        assert!(
            front_door.transcript_activation_is_focused(cx),
            "ActivateProjectPanel must fail closed while Files is loading"
        );
        front_door.dispatch_action(project_panel::Rename, cx);
        assert!(
            !front_door.native_files_has_edit_state(cx),
            "a mutating global ProjectPanel action must be a no-op while Files is loading"
        );
        assert!(front_door.transcript_activation_is_focused(cx));
        front_door.dispatch_action(omega_actions::project_panel::ToggleFocus, cx);
        SemanticProbe::new(&front_door.snapshot(cx))
            .require_focus(WorkSurfaceId::Files.surface_selector(), true)
            .expect("ToggleFocus may reopen Loading only on its visible status host");
        assert!(!front_door.native_files_is_focused(cx));
        front_door.dispatch_action(workspace::CloseActiveDock, cx);
        let loading_closed = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&loading_closed);
        probe
            .require_unique("omega.workbench.root")
            .expect("Loading CloseActiveDock must retain the outer AgentPanel");
        probe
            .require_absent(WORKBENCH_DOCK_SELECTOR)
            .expect("Loading CloseActiveDock must collapse the internal Files dock");
        assert!(front_door.transcript_activation_is_focused(cx));
        front_door.dispatch_action(omega_actions::project_panel::ToggleFocus, cx);
        SemanticProbe::new(&front_door.snapshot(cx))
            .require_focus(WorkSurfaceId::Files.surface_selector(), true)
            .expect("Loading must reopen on its visible status host after internal close");

        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert!(
            front_door.transcript_activation_is_focused(cx),
            "collapsing a loading Files surface should focus the transcript"
        );
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        let reopened_loading = front_door.snapshot(cx);
        SemanticProbe::new(&reopened_loading)
            .require_focus(WorkSurfaceId::Files.surface_selector(), true)
            .expect("reopening Loading must focus the visible host, not its hidden native child");
        assert!(!front_door.native_files_is_focused(cx));

        assert_eq!(
            front_door
                .complete_surface_load(
                    failed_load,
                    crate::workbench_shell::SurfaceLoadOutcome::Error(
                        "Fixture Files load failed".into(),
                    ),
                    cx,
                )
                .expect("complete Files load with an error"),
            omega_workbench_state::TransitionEffect::Applied
        );
        assert_eq!(
            host.read_with(cx, |host, _| host.content_state().clone()),
            crate::workbench_shell::SurfaceContentState::Error("Fixture Files load failed".into())
        );
        let error_snapshot = front_door.snapshot(cx);
        SemanticProbe::new(&error_snapshot)
            .require_accessible(
                "omega.workbench.surface.files.status",
                "Alert",
                "Fixture Files load failed",
            )
            .expect("Files load failure should be semantically observable");
        SemanticProbe::new(&error_snapshot)
            .require_focus(WorkSurfaceId::Files.surface_selector(), true)
            .expect("an error surface should retain focus on its rendered host");
        assert!(
            !front_door.native_files_is_focused(cx),
            "error state must not focus the hidden ProjectPanel"
        );
        assert!(matches!(
            front_door.native_files_scope_state(cx),
            Some(project_panel::ProjectPanelScopeState::Unavailable)
        ));
        front_door.activate_files_panel(cx);
        assert!(
            front_door.transcript_activation_is_focused(cx),
            "ActivateProjectPanel must fail closed while Files is in Error"
        );
        front_door.dispatch_action(omega_actions::project_panel::ToggleFocus, cx);
        SemanticProbe::new(&front_door.snapshot(cx))
            .require_focus(WorkSurfaceId::Files.surface_selector(), true)
            .expect("ToggleFocus may reopen Error only on its visible status host");
        assert!(!front_door.native_files_is_focused(cx));
        front_door.dispatch_action(workspace::CloseActiveDock, cx);
        let error_closed = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&error_closed);
        probe
            .require_unique("omega.workbench.root")
            .expect("Error CloseActiveDock must retain the outer AgentPanel");
        probe
            .require_absent(WORKBENCH_DOCK_SELECTOR)
            .expect("Error CloseActiveDock must collapse the internal Files dock");
        assert!(front_door.transcript_activation_is_focused(cx));
        front_door.dispatch_action(omega_actions::project_panel::ToggleFocus, cx);
        SemanticProbe::new(&front_door.snapshot(cx))
            .require_focus(WorkSurfaceId::Files.surface_selector(), true)
            .expect("Error must reopen on its visible status host after internal close");

        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert!(front_door.transcript_activation_is_focused(cx));
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        let reopened_error = front_door.snapshot(cx);
        SemanticProbe::new(&reopened_error)
            .require_focus(WorkSurfaceId::Files.surface_selector(), true)
            .expect("reopening Error must focus the rendered host");
        assert!(!front_door.native_files_is_focused(cx));

        let retry = front_door
            .begin_surface_load(
                "files-load-retry",
                omega_workbench_state::WorkSurface::Files,
                cx,
            )
            .expect("retry Files load");
        front_door
            .complete_surface_load(retry, crate::workbench_shell::SurfaceLoadOutcome::Ready, cx)
            .expect("complete Files retry");
        front_door.snapshot(cx);
        assert!(
            front_door.native_files_is_focused(cx),
            "a successful retry should return focus to the rendered native tree"
        );
        assert_eq!(
            front_door.native_files_selected_path(cx),
            Some(selected_path.clone()),
            "surface load recovery must restore the compatible native selection"
        );

        front_door.set_identity_phase(
            crate::thread_identity::IdentityPhase::Error("Fixture Files identity failed".into()),
            cx,
        );
        let identity_error = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&identity_error);
        probe
            .require_accessible(
                "omega.workbench.surface.files.status",
                "Alert",
                "Fixture Files identity failed",
            )
            .expect("the live identity error should feed the visible Files host");
        probe
            .require_focus(WorkSurfaceId::Files.surface_selector(), true)
            .expect("an identity error should move focus off the hidden native tree");
        assert!(!front_door.native_files_is_focused(cx));

        front_door.set_identity_phase(crate::thread_identity::IdentityPhase::Ready, cx);
        let identity_recovered = front_door.snapshot(cx);
        SemanticProbe::new(&identity_recovered)
            .require_accessible("omega.project-panel.tree", "Tree", "Files")
            .expect("identity recovery should restore the native Files tree");
        assert!(
            front_door.native_files_is_focused(cx),
            "identity recovery should return focus to the native tree"
        );
        assert_eq!(
            front_door.native_files_selected_path(cx),
            Some(selected_path.clone()),
            "identity error recovery must restore the compatible native selection"
        );

        front_door.set_identity_phase(crate::thread_identity::IdentityPhase::Offline, cx);
        let projection = front_door
            .projection(cx)
            .visible_projection()
            .expect("offline projection");
        assert!(!projection.dock_open);
        assert_eq!(
            front_door
                .panel()
                .read_with(cx, |panel, _| panel.workbench_focus_target_for_tests()),
            crate::workbench_shell::WorkbenchFocusTarget::Transcript
        );
        assert!(
            !front_door
                .capability(omega_workbench_state::WorkSurface::Files, cx)
                .expect("Files capability")
                .availability
                .is_available()
        );
        assert_eq!(
            front_door.native_files_panel_entity_id(cx),
            Some(files_panel_id),
            "offline transition should retain the native panel cache without rendering it"
        );
        assert!(
            front_door.transcript_activation_is_focused(cx),
            "offline invalidation must transfer actual GPUI focus to the transcript"
        );
        assert!(
            !front_door.native_files_is_focused(cx),
            "offline Files must not leave focus inside its unrendered native panel"
        );
        let offline = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&offline);
        probe
            .require_absent(WORKBENCH_DOCK_SELECTOR)
            .expect("offline repository-bound Files should close");
        probe
            .require_accessibility_property(
                WorkSurfaceId::Files.rail_selector(),
                "disabled",
                serde_json::Value::Bool(true),
            )
            .expect("offline Files rail item should be disabled");
        assert!(matches!(
            front_door.native_files_scope_state(cx),
            Some(project_panel::ProjectPanelScopeState::Unavailable)
        ));
        front_door.activate_files_panel(cx);
        front_door.dispatch_action(omega_actions::project_panel::ToggleFocus, cx);
        assert!(
            front_door.transcript_activation_is_focused(cx),
            "offline activation routes must leave focus in the visible transcript"
        );
        SemanticProbe::new(&front_door.snapshot(cx))
            .require_absent(WORKBENCH_DOCK_SELECTOR)
            .expect("offline activation routes must not reopen Files");

        front_door.set_identity_phase(crate::thread_identity::IdentityPhase::Ready, cx);
        front_door.snapshot(cx);
        front_door.dispatch_action(omega_actions::project_panel::ToggleFocus, cx);
        assert_eq!(
            front_door.native_files_selected_path(cx),
            Some(selected_path),
            "offline recovery must restore the compatible native selection"
        );
        assert!(
            front_door.native_files_is_focused(cx),
            "offline recovery must reopen on the restored native tree"
        );

        front_door
            .teardown(cx)
            .expect("Files state scene should tear down");
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

    #[gpui::test(iterations = 8)]
    async fn native_plan_tracks_typed_updates_stable_ids_and_history(cx: &mut TestAppContext) {
        let mut scene = scene_with_thread("native_plan_updates", 1200, false);
        scene.active_surface = Some(WorkSurfaceId::Plan);
        scene.dock_open = true;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("native Plan scene should mount");

        let initial_surface_id = front_door
            .native_plan_surface(cx)
            .expect("native Plan surface")
            .entity_id();
        assert_eq!(
            front_door
                .native_plan_snapshot(cx)
                .expect("initial Plan snapshot")
                .state,
            crate::workbench_shell::PlanSurfaceState::Empty
        );

        front_door
            .apply_plan_update(
                vec![
                    acp::PlanEntry::new(
                        "Inspect the projection",
                        acp::PlanEntryPriority::High,
                        acp::PlanEntryStatus::Completed,
                    ),
                    acp::PlanEntry::new(
                        "Implement the retained surface",
                        acp::PlanEntryPriority::Medium,
                        acp::PlanEntryStatus::InProgress,
                    ),
                    acp::PlanEntry::new(
                        "Verify reconnect behavior",
                        acp::PlanEntryPriority::Low,
                        acp::PlanEntryStatus::Pending,
                    ),
                ],
                cx,
            )
            .expect("initial typed Plan update");
        let active = front_door
            .native_plan_snapshot(cx)
            .expect("active Plan snapshot");
        assert_eq!(active.revision, 1);
        assert_eq!(active.current_steps.len(), 3);
        assert_eq!(
            active.state,
            crate::workbench_shell::PlanSurfaceState::Active {
                pending: 1,
                in_progress: 1,
                completed: 1,
                unknown: 0,
                total: 3,
            }
        );
        let stable_ids = active
            .current_steps
            .iter()
            .map(|step| step.id)
            .collect::<Vec<_>>();
        assert_eq!(active.active_step_id, stable_ids.get(1).copied());
        assert_eq!(
            active
                .current_steps
                .iter()
                .map(|step| (step.status, step.priority))
                .collect::<Vec<_>>(),
            vec![
                (
                    crate::plan_presentation::PlanStatusKind::Completed,
                    crate::plan_presentation::PlanPriorityKind::High,
                ),
                (
                    crate::plan_presentation::PlanStatusKind::InProgress,
                    crate::plan_presentation::PlanPriorityKind::Medium,
                ),
                (
                    crate::plan_presentation::PlanStatusKind::Pending,
                    crate::plan_presentation::PlanPriorityKind::Low,
                ),
            ]
        );

        front_door
            .apply_plan_update(
                vec![
                    acp::PlanEntry::new(
                        "Inspect the projection",
                        acp::PlanEntryPriority::High,
                        acp::PlanEntryStatus::Completed,
                    ),
                    acp::PlanEntry::new(
                        "Implement the retained native surface",
                        acp::PlanEntryPriority::High,
                        acp::PlanEntryStatus::Completed,
                    ),
                    acp::PlanEntry::new(
                        "Verify reconnect behavior",
                        acp::PlanEntryPriority::Low,
                        acp::PlanEntryStatus::InProgress,
                    ),
                ],
                cx,
            )
            .expect("replacement typed Plan update");
        let replacement = front_door
            .native_plan_snapshot(cx)
            .expect("replacement Plan snapshot");
        assert_eq!(replacement.revision, 2);
        assert_eq!(replacement.active_step_id, stable_ids.get(2).copied());
        assert_eq!(
            replacement
                .current_steps
                .iter()
                .map(|step| step.id)
                .collect::<Vec<_>>(),
            stable_ids,
            "full ACP replacements must preserve positional step identity"
        );

        front_door
            .apply_plan_update(
                vec![acp::PlanEntry::new(
                    "   ",
                    acp::PlanEntryPriority::Medium,
                    acp::PlanEntryStatus::Pending,
                )],
                cx,
            )
            .expect("malformed typed Plan update should be retained as state");
        let malformed = front_door
            .native_plan_snapshot(cx)
            .expect("malformed Plan snapshot");
        assert_eq!(malformed.revision, 3);
        assert!(matches!(
            malformed.state,
            crate::workbench_shell::PlanSurfaceState::Malformed(_)
        ));
        assert_eq!(
            malformed
                .current_steps
                .iter()
                .map(|step| step.id)
                .collect::<Vec<_>>(),
            stable_ids,
            "malformed input must retain the last good typed plan"
        );

        front_door
            .apply_plan_update(
                replacement
                    .current_steps
                    .iter()
                    .map(|step| {
                        acp::PlanEntry::new(
                            step.label.to_string(),
                            acp::PlanEntryPriority::Medium,
                            acp::PlanEntryStatus::Completed,
                        )
                    })
                    .collect(),
                cx,
            )
            .expect("all-complete typed Plan update");
        assert!(matches!(
            front_door
                .native_plan_snapshot(cx)
                .expect("all-complete Plan snapshot")
                .state,
            crate::workbench_shell::PlanSurfaceState::AllComplete { total: 3 }
        ));
        front_door
            .snapshot_completed_plan(cx)
            .expect("snapshot completed Plan into typed history");
        let historical = front_door
            .native_plan_snapshot(cx)
            .expect("historical Plan snapshot");
        assert_eq!(historical.revision, 5);
        assert!(historical.current_steps.is_empty());
        assert_eq!(historical.historical_steps.len(), 3);
        assert!(
            historical
                .historical_steps
                .iter()
                .all(|step| step.source_entry_index.is_some())
        );
        let historical_step = historical
            .historical_steps
            .first()
            .expect("completed plan should expose one historical step");
        let historical_step_id = historical_step.id;
        let source_entry_index = historical_step
            .source_entry_index
            .expect("historical step source entry");
        front_door
            .click(
                &format!("omega.workbench.plan.step.{historical_step_id}"),
                cx,
            )
            .expect("historical Plan step should navigate");
        let navigated = front_door
            .native_plan_snapshot(cx)
            .expect("navigated historical Plan snapshot");
        assert_eq!(navigated.selected_step_id, Some(historical_step_id));
        assert_eq!(
            navigated.navigation_status.as_deref(),
            Some(format!("Opened transcript event {}", source_entry_index + 1).as_str())
        );
        assert_eq!(
            front_door.native_plan_navigation_target(cx),
            Some(source_entry_index),
            "the validated navigation path must reach ThreadView"
        );
        assert_eq!(
            front_door
                .native_plan_surface(cx)
                .expect("retained native Plan surface")
                .entity_id(),
            initial_surface_id
        );

        front_door
            .teardown(cx)
            .expect("native Plan scene should tear down");
    }

    #[gpui::test(iterations = 8)]
    async fn native_plan_retains_selection_across_lifecycle_and_collapse(cx: &mut TestAppContext) {
        let mut scene = scene_with_thread("native_plan_retention", 1200, false);
        scene.active_surface = Some(WorkSurfaceId::Plan);
        scene.dock_open = true;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("native Plan retention scene should mount");
        front_door
            .apply_plan_update(
                vec![acp::PlanEntry::new(
                    "Retain this step",
                    acp::PlanEntryPriority::High,
                    acp::PlanEntryStatus::InProgress,
                )],
                cx,
            )
            .expect("typed Plan update");
        let initial = front_door
            .native_plan_snapshot(cx)
            .expect("initial Plan snapshot");
        let step_id = initial
            .current_steps
            .first()
            .expect("one current Plan step")
            .id;
        let surface_id = front_door
            .native_plan_surface(cx)
            .expect("native Plan surface")
            .entity_id();
        let plan_surface = front_door
            .native_plan_surface(cx)
            .expect("native Plan surface for stale binding check");
        let binding = plan_surface.read_with(cx, |surface, _cx| surface.binding().clone());
        let rejected = cx.update(|cx| {
            plan_surface.update(cx, |surface, cx| {
                surface.bind_thread(
                    crate::workbench_shell::NativePlanBinding {
                        thread_id: "foreign-thread".to_string(),
                        generation: binding.generation.saturating_add(1),
                    },
                    None,
                    cx,
                )
            })
        });
        assert!(
            !rejected,
            "a foreign binding in a higher epoch must be rejected"
        );
        assert_eq!(
            front_door
                .native_plan_snapshot(cx)
                .expect("Plan snapshot after rejected stale binding")
                .rejected_update_count,
            1
        );
        front_door
            .click(&format!("omega.workbench.plan.step.{step_id}"), cx)
            .expect("select live Plan step");
        let selected = front_door
            .native_plan_snapshot(cx)
            .expect("selected Plan snapshot");
        assert_eq!(selected.selected_step_id, Some(step_id));
        assert_eq!(
            selected.navigation_status.as_deref(),
            Some("This live plan step has no transcript event yet")
        );

        front_door
            .set_plan_interruption(Some("agent cancelled".into()), cx)
            .expect("persist Plan interruption");
        let interrupted = front_door
            .native_plan_snapshot(cx)
            .expect("interrupted Plan snapshot");
        assert_eq!(
            interrupted.state,
            crate::workbench_shell::PlanSurfaceState::Interrupted("agent cancelled".into())
        );
        assert_eq!(interrupted.current_steps, selected.current_steps);
        front_door
            .set_plan_interruption(None, cx)
            .expect("clear Plan interruption at the next turn boundary");
        assert!(matches!(
            front_door
                .native_plan_snapshot(cx)
                .expect("resumed Plan snapshot")
                .state,
            crate::workbench_shell::PlanSurfaceState::Active { .. }
        ));

        front_door.set_plan_lifecycle(Some(crate::workbench_shell::NativePlanLifecycle::Stale), cx);
        let stale = front_door
            .native_plan_snapshot(cx)
            .expect("stale Plan snapshot");
        assert_eq!(stale.state, crate::workbench_shell::PlanSurfaceState::Stale);
        assert_eq!(stale.current_steps, selected.current_steps);
        assert_eq!(stale.selected_step_id, Some(step_id));
        front_door.set_plan_lifecycle(
            Some(crate::workbench_shell::NativePlanLifecycle::Reconnecting),
            cx,
        );
        assert_eq!(
            front_door
                .native_plan_snapshot(cx)
                .expect("reconnecting Plan snapshot")
                .state,
            crate::workbench_shell::PlanSurfaceState::Reconnecting
        );

        front_door.dispatch_action(crate::workbench_shell::SelectPlan, cx);
        front_door.dispatch_action(crate::workbench_shell::SelectPlan, cx);
        let reopened = front_door
            .native_plan_snapshot(cx)
            .expect("reopened Plan snapshot");
        assert_eq!(
            front_door
                .native_plan_surface(cx)
                .expect("reopened Plan surface")
                .entity_id(),
            surface_id
        );
        assert_eq!(reopened.selected_step_id, Some(step_id));
        assert_eq!(reopened.current_steps, selected.current_steps);

        front_door
            .teardown(cx)
            .expect("native Plan retention scene should tear down");
    }

    #[gpui::test(iterations = 8)]
    async fn native_plan_rejects_cross_thread_projection_updates(cx: &mut TestAppContext) {
        let scene = scene_with_thread("native_plan_thread_isolation", 1200, false);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("native Plan isolation scene should mount");

        let thread_a_id = front_door.open_external_thread(StubAgentConnection::new(), cx);
        let thread_a = front_door
            .panel()
            .read_with(cx, |panel, cx| panel.active_agent_thread(cx))
            .expect("thread A agent projection");
        front_door.dispatch_action(crate::workbench_shell::SelectPlan, cx);
        front_door
            .apply_plan_update(
                vec![acp::PlanEntry::new(
                    "Thread A initial step",
                    acp::PlanEntryPriority::High,
                    acp::PlanEntryStatus::InProgress,
                )],
                cx,
            )
            .expect("thread A Plan update");
        let surface_a_id = front_door
            .native_plan_surface(cx)
            .expect("thread A Plan surface")
            .entity_id();

        let thread_b_id = front_door.open_external_thread(StubAgentConnection::new(), cx);
        let thread_b = front_door
            .panel()
            .read_with(cx, |panel, cx| panel.active_agent_thread(cx))
            .expect("thread B agent projection");
        assert_ne!(thread_a_id, thread_b_id);
        front_door.dispatch_action(crate::workbench_shell::SelectPlan, cx);
        front_door
            .apply_plan_update(
                vec![acp::PlanEntry::new(
                    "Thread B retained step",
                    acp::PlanEntryPriority::Low,
                    acp::PlanEntryStatus::Pending,
                )],
                cx,
            )
            .expect("thread B Plan update");
        let surface_b = front_door
            .native_plan_surface(cx)
            .expect("thread B Plan surface");
        let surface_b_id = surface_b.entity_id();
        assert_ne!(surface_a_id, surface_b_id);
        let before_old_thread_update = surface_b.read_with(cx, |surface, _cx| surface.snapshot());

        cx.update(|cx| {
            thread_a.update(cx, |thread, cx| {
                thread
                    .handle_session_update(
                        acp::SessionUpdate::Plan(acp::Plan::new(vec![acp::PlanEntry::new(
                            "Thread A late step",
                            acp::PlanEntryPriority::Medium,
                            acp::PlanEntryStatus::Completed,
                        )])),
                        cx,
                    )
                    .expect("late thread A Plan update");
            });
        });
        front_door.settle(cx);

        assert_eq!(
            front_door
                .native_plan_surface(cx)
                .expect("active thread B Plan surface")
                .entity_id(),
            surface_b_id
        );
        assert_eq!(
            front_door
                .native_plan_snapshot(cx)
                .expect("thread B snapshot after old-thread update"),
            before_old_thread_update,
            "a retained thread update must not mutate the active thread projection"
        );

        front_door.activate_thread(thread_a_id, cx);
        let restored_a = front_door
            .native_plan_snapshot(cx)
            .expect("restored thread A Plan snapshot");
        assert_eq!(
            restored_a
                .current_steps
                .first()
                .map(|step| step.label.as_ref()),
            Some("Thread A late step")
        );
        let restored_surface_a = front_door
            .native_plan_surface(cx)
            .expect("restored thread A Plan surface");
        let restored_binding =
            restored_surface_a.read_with(cx, |surface, _cx| surface.binding().clone());
        let rebound = cx.update(|cx| {
            restored_surface_a.update(cx, |surface, cx| {
                surface.bind_thread(restored_binding, Some(thread_b), cx)
            })
        });
        assert!(
            rebound,
            "a replacement entity for the same logical binding should bind"
        );
        let replaced_entity_snapshot =
            restored_surface_a.read_with(cx, |surface, _cx| surface.snapshot());
        assert_eq!(replaced_entity_snapshot.revision, 1);
        assert_eq!(
            replaced_entity_snapshot
                .current_steps
                .first()
                .map(|step| step.label.as_ref()),
            Some("Thread B retained step"),
            "a replacement entity must reset the old accepted revision and snapshot"
        );

        front_door
            .teardown(cx)
            .expect("native Plan isolation scene should tear down");
    }

    #[gpui::test(iterations = 8)]
    async fn native_plan_reconciles_ids_through_insert_remove_and_reorder(cx: &mut TestAppContext) {
        let mut scene = scene_with_thread("native_plan_identity_reconciliation", 1200, false);
        scene.active_surface = Some(WorkSurfaceId::Plan);
        scene.dock_open = true;
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("native Plan identity scene should mount");
        front_door
            .apply_plan_update(
                vec![
                    acp::PlanEntry::new(
                        "Alpha",
                        acp::PlanEntryPriority::High,
                        acp::PlanEntryStatus::Pending,
                    ),
                    acp::PlanEntry::new(
                        "Beta",
                        acp::PlanEntryPriority::Medium,
                        acp::PlanEntryStatus::InProgress,
                    ),
                    acp::PlanEntry::new(
                        "Gamma",
                        acp::PlanEntryPriority::Low,
                        acp::PlanEntryStatus::Pending,
                    ),
                ],
                cx,
            )
            .expect("initial identity Plan update");
        let initial = front_door
            .native_plan_snapshot(cx)
            .expect("initial identity Plan snapshot");
        let alpha_id = initial
            .current_steps
            .iter()
            .find(|step| step.label.as_ref() == "Alpha")
            .expect("Alpha step")
            .id;
        let beta_id = initial
            .current_steps
            .iter()
            .find(|step| step.label.as_ref() == "Beta")
            .expect("Beta step")
            .id;
        let gamma_id = initial
            .current_steps
            .iter()
            .find(|step| step.label.as_ref() == "Gamma")
            .expect("Gamma step")
            .id;
        front_door
            .click(&format!("omega.workbench.plan.step.{beta_id}"), cx)
            .expect("select Beta before reconciliation");

        front_door
            .apply_plan_update(
                vec![
                    acp::PlanEntry::new(
                        "Inserted",
                        acp::PlanEntryPriority::Low,
                        acp::PlanEntryStatus::Pending,
                    ),
                    acp::PlanEntry::new(
                        "Gamma",
                        acp::PlanEntryPriority::Low,
                        acp::PlanEntryStatus::Completed,
                    ),
                    acp::PlanEntry::new(
                        "Alpha",
                        acp::PlanEntryPriority::High,
                        acp::PlanEntryStatus::InProgress,
                    ),
                    acp::PlanEntry::new(
                        "Beta",
                        acp::PlanEntryPriority::Medium,
                        acp::PlanEntryStatus::Pending,
                    ),
                ],
                cx,
            )
            .expect("inserted and reordered Plan update");
        let reordered = front_door
            .native_plan_snapshot(cx)
            .expect("reordered Plan snapshot");
        assert_eq!(
            reordered
                .current_steps
                .iter()
                .map(|step| step.label.as_ref())
                .collect::<Vec<_>>(),
            vec!["Inserted", "Gamma", "Alpha", "Beta"]
        );
        assert_eq!(
            reordered
                .current_steps
                .iter()
                .find(|step| step.label.as_ref() == "Alpha")
                .map(|step| step.id),
            Some(alpha_id)
        );
        assert_eq!(
            reordered
                .current_steps
                .iter()
                .find(|step| step.label.as_ref() == "Beta")
                .map(|step| step.id),
            Some(beta_id)
        );
        assert_eq!(
            reordered
                .current_steps
                .iter()
                .find(|step| step.label.as_ref() == "Gamma")
                .map(|step| step.id),
            Some(gamma_id)
        );
        assert_eq!(reordered.selected_step_id, Some(beta_id));
        assert!(
            reordered
                .current_steps
                .first()
                .is_some_and(|step| ![alpha_id, beta_id, gamma_id].contains(&step.id))
        );

        front_door
            .teardown(cx)
            .expect("native Plan identity scene should tear down");
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

        // `OMEGA-DELTA-0205`. The shared boundary is the activity rail, the
        // collapsed sidebar (which now takes no column of its own), the
        // minimum dock and the transcript floor. Read it off the same
        // constants the allocator uses rather than writing the number here:
        // this test carried the literal 909 — one below the pre-0205 boundary
        // of 910 — because the collapsed sidebar used to keep a 30px rail, and
        // a literal cannot follow a constant that moves.
        let dock_boundary = crate::workbench_shell::ACTIVITY_RAIL_WIDTH
            + crate::omega_sidebar::RAIL_WIDTH
            + crate::workbench_shell::MIN_DOCK_WIDTH
            + crate::omega_sidebar::MIN_CONTENT_WIDTH;
        let dock_boundary = dock_boundary.to_f64() as u32;
        assert_eq!(dock_boundary, 880, "OMEGA-DELTA-0205's stated boundary");

        // At the boundary itself the dock is still drawn. Without this the
        // assertion below would also pass for a boundary that had quietly
        // moved wider, which is exactly how the stale 909 stopped meaning
        // "one pixel below".
        front_door.resize(dock_boundary, 720, cx);
        let at_boundary = front_door.snapshot(cx);
        SemanticProbe::new(&at_boundary)
            .require_visible(WORKBENCH_DOCK_SELECTOR)
            .expect("dock should still be drawn at its shared boundary");

        front_door.resize(dock_boundary - 1, 720, cx);
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
        // omega#170. With no folder open, every unavailable rail surface must
        // carry the actionable no-project reason. Terminal used to keep a
        // clobbered "This surface is no longer available" because the reason
        // repair pass skips Terminal, which read as a retired surface rather
        // than one waiting on a project.
        for surface in [
            omega_workbench_state::WorkSurface::Files,
            omega_workbench_state::WorkSurface::Search,
            omega_workbench_state::WorkSurface::Git,
            omega_workbench_state::WorkSurface::Terminal,
        ] {
            let capability = front_door
                .capability(surface, cx)
                .expect("every rail surface has a typed capability");
            assert_eq!(
                capability
                    .availability
                    .reason()
                    .map(|reason| reason.as_ref()),
                Some("Open a project to use this surface"),
                "{surface:?} must name the no-project reason, not a retirement"
            );
        }

        let before = front_door.projection(cx);
        front_door.focus_workspace_root(cx);
        assert!(
            !front_door.agent_panel_contains_focus(cx),
            "unavailable menu action should begin outside AgentPanel"
        );
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert_eq!(
            front_door.projection(cx),
            before,
            "unavailable action must not open a fallback"
        );
        assert_eq!(
            front_door.workbench_last_error(cx).as_deref(),
            Some("Open a project to use this surface"),
            "unavailable action from center focus must preserve its exact reason"
        );

        front_door.focus_agent_panel(cx);
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
    async fn application_menu_actions_forward_from_workspace_focus(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_menu_workspace_focus", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("menu forwarding scene should mount");

        let sidebar_was_open = front_door.threads_sidebar_open(cx);
        front_door.focus_workspace_root(cx);
        assert!(
            !front_door.agent_panel_contains_focus(cx),
            "menu forwarding must be exercised from outside AgentPanel"
        );
        front_door.dispatch_action(crate::ToggleThreadsSidebar, cx);
        assert_ne!(
            front_door.threads_sidebar_open(cx),
            sidebar_was_open,
            "Threads Sidebar menu action did not reach AgentPanel"
        );

        front_door.focus_workspace_root(cx);
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        assert_eq!(
            front_door
                .projection(cx)
                .visible_projection()
                .and_then(|projection| projection.effective_surface),
            Some(omega_workbench_state::WorkSurface::Files)
        );

        front_door.focus_workspace_root(cx);
        front_door.dispatch_action(crate::workbench_shell::SelectGit, cx);
        assert_eq!(
            front_door
                .projection(cx)
                .visible_projection()
                .and_then(|projection| projection.effective_surface),
            Some(omega_workbench_state::WorkSurface::Git)
        );

        front_door.focus_workspace_root(cx);
        front_door.dispatch_action(crate::workbench_shell::SelectTerminal, cx);
        assert_eq!(
            front_door
                .projection(cx)
                .visible_projection()
                .and_then(|projection| projection.effective_surface),
            Some(omega_workbench_state::WorkSurface::Terminal)
        );

        front_door
            .teardown(cx)
            .expect("menu forwarding scene should tear down");
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
    async fn files_host_and_native_panel_release_with_window(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_files_release", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("Files release scene should mount");
        front_door.dispatch_action(crate::workbench_shell::SelectFiles, cx);
        let weak_host = front_door
            .visible_surface_host(cx)
            .expect("visible Files host")
            .downgrade();
        let weak_files_panel = front_door
            .native_files_panel(cx)
            .expect("native ProjectPanel")
            .downgrade();

        front_door
            .teardown(cx)
            .expect("Files release workbench should tear down");
        assert!(
            weak_host.upgrade().is_none(),
            "Files host must not outlive its panel and window"
        );
        assert!(
            weak_files_panel.upgrade().is_none(),
            "the handed-off native ProjectPanel must not leak after window teardown"
        );
    }

    #[gpui::test]
    async fn native_search_routes_one_action_once_and_opens_the_selected_result(
        cx: &mut TestAppContext,
    ) {
        let scene = scene_with_thread("workbench_native_search", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("native Search scene should mount");
        front_door.dispatch_action(crate::workbench_shell::SelectSearch, cx);

        let search_surface_id = front_door
            .native_search_surface_entity_id(cx)
            .expect("Search should mount its native adapter");
        let search_view_id = front_door
            .native_search_view_entity_id(cx)
            .expect("Search should mount the exact native view");
        let search_model_id = front_door
            .native_search_model_entity_id(cx)
            .expect("Search should mount the exact native model");
        let search_bar_id = front_door
            .native_search_bar_entity_id(cx)
            .expect("Search should mount the exact native bar");
        let fixture_worktree_id = front_door
            .fixture_worktree_id("worktree-1", cx)
            .expect("fixture worktree");
        assert_eq!(
            front_door
                .native_search_state(cx)
                .expect("native Search state")
                .worktree_scope,
            Some(fixture_worktree_id)
        );

        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_accessible(
                "omega.workbench.search.toolbar",
                "Toolbar",
                "Search controls",
            )
            .expect("native Search toolbar should expose its accessible contract");
        probe
            .require_accessible("omega.workbench.search.content", "Group", "Search results")
            .expect("native Search results should expose their accessible contract");

        front_door.dispatch_action(crate::workbench_shell::SelectSearch, cx);
        assert_eq!(
            front_door.native_search_surface_entity_id(cx),
            Some(search_surface_id),
            "collapsing Search must retain its binding-owned native entity graph"
        );
        front_door.dispatch_action(crate::workbench_shell::SelectSearch, cx);
        assert_eq!(
            front_door.native_search_view_entity_id(cx),
            Some(search_view_id),
            "reopening Search must restore the exact native view"
        );
        front_door.snapshot(cx);

        front_door
            .perform_native_search("fixture", cx)
            .expect("native Search should run through its exact view");
        let searched = front_door
            .native_search_state(cx)
            .expect("populated native Search state");
        assert!(
            searched.matches.len() >= 3,
            "the fixture should provide enough matches to detect duplicate action routing"
        );
        assert_eq!(searched.active_match_index, Some(0));
        assert_eq!(
            front_door.native_search_focus_target(cx),
            Some(crate::workbench_shell::NativeSearchFocusTarget::Results)
        );

        front_door.dispatch_action(search::SelectNextMatch, cx);
        let advanced = front_door
            .native_search_state(cx)
            .expect("advanced native Search state");
        assert_eq!(
            advanced.active_match_index,
            Some(1),
            "one SelectNextMatch action must advance exactly one native match"
        );
        let selected_match = advanced
            .active_match
            .expect("the native Search state should identify its selected result");

        front_door.dispatch_action(editor::actions::OpenExcerpts, cx);
        assert!(
            front_door.workspace_center_is_visible(cx),
            "opening a Search result must reveal the sealed center"
        );
        assert_eq!(
            front_door.active_workspace_item_path(cx),
            Some(selected_match.path.clone())
        );
        let (selection_start, selection_end) = front_door
            .active_workspace_selection(cx)
            .expect("opened Search result selection");
        let (expected_start, expected_end) = front_door
            .active_workspace_point_range(&selected_match.range, cx)
            .expect("selected native Search match range in the opened singleton buffer");
        assert_eq!(selection_start, expected_start);
        assert_eq!(selection_end, expected_end);
        assert!(
            front_door.active_workspace_item_is_focused(cx),
            "opening a Search result must predictably focus the center editor"
        );

        assert_eq!(
            front_door.native_search_surface_entity_id(cx),
            Some(search_surface_id)
        );
        assert_eq!(
            front_door.native_search_view_entity_id(cx),
            Some(search_view_id)
        );
        assert_eq!(
            front_door.native_search_model_entity_id(cx),
            Some(search_model_id)
        );
        assert_eq!(
            front_door.native_search_bar_entity_id(cx),
            Some(search_bar_id)
        );

        front_door
            .teardown(cx)
            .expect("native Search workbench should tear down");
    }

    #[gpui::test]
    async fn visible_search_rebinds_to_a_fresh_native_host(cx: &mut TestAppContext) {
        let scene = scene_with_two_worktrees("workbench_search_binding_change");
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("Search binding-change scene should mount");
        front_door.dispatch_action(crate::workbench_shell::SelectSearch, cx);
        front_door
            .perform_native_search("worktree-1", cx)
            .expect("old binding Search should accept a query");
        let old_surface = front_door
            .native_search_surface(cx)
            .expect("old binding native Search surface");
        let old_surface_id = old_surface.entity_id();
        let weak_old_surface = old_surface.downgrade();
        drop(old_surface);

        front_door
            .select_worktree_picker_row(1, cx)
            .expect("select the second worktree through the rendered picker");

        let new_surface_id = front_door
            .native_search_surface_entity_id(cx)
            .expect("new binding native Search surface");
        assert_ne!(new_surface_id, old_surface_id);
        assert!(
            weak_old_surface.upgrade().is_none(),
            "the old binding must not retain its native Search entity graph"
        );
        let new_worktree_id = front_door
            .fixture_worktree_id("worktree-2", cx)
            .expect("new fixture worktree");
        let state = front_door
            .native_search_state(cx)
            .expect("new binding native Search state");
        assert_eq!(state.worktree_scope, Some(new_worktree_id));
        assert_eq!(state.query, "");
        assert!(
            front_door
                .projection(cx)
                .visible_projection()
                .is_some_and(|visible| {
                    visible.dock_open
                        && visible.effective_surface
                            == Some(omega_workbench_state::WorkSurface::Search)
                }),
            "a live Search binding change must not leave an empty open dock"
        );

        front_door
            .teardown(cx)
            .expect("Search binding-change workbench should tear down");
    }

    #[gpui::test]
    async fn native_review_retains_exact_entities_and_routes_one_keep(cx: &mut TestAppContext) {
        let scene = scene_with_thread("workbench_native_review", 1200, true);
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("native Review scene should mount");
        front_door.dispatch_action(crate::workbench_shell::SelectReview, cx);

        let surface_id = front_door
            .native_review_surface_entity_id(cx)
            .expect("native Review surface");
        let agent_diff_id = front_door
            .native_review_agent_diff_entity_id(cx)
            .expect("native AgentDiff authority");
        let pane_id = front_door
            .native_review_pane_entity_id(cx)
            .expect("native AgentDiffPane");
        let toolbar_id = front_door
            .native_review_toolbar_entity_id(cx)
            .expect("native AgentDiffToolbar");
        let empty = front_door
            .native_review_state(cx)
            .expect("empty native Review state");
        let worktree_id = front_door
            .fixture_worktree_id("worktree-1", cx)
            .expect("Review fixture worktree");
        assert_eq!(
            empty.binding.as_ref().map(|binding| binding.worktree_id),
            Some(worktree_id)
        );
        assert!(matches!(empty.lifecycle, crate::AgentDiffLifecycle::Empty));

        let snapshot = front_door.snapshot(cx);
        let mut probe = SemanticProbe::new(&snapshot);
        probe
            .require_accessible(
                "omega.workbench.review.toolbar",
                "Toolbar",
                "Review controls",
            )
            .expect("native Review toolbar should be accessible");
        probe
            .require_accessible("omega.workbench.review.content", "Group", "Review changes")
            .expect("native Review content should be accessible");

        front_door
            .seed_native_review_edit(
                "worktree-1",
                "src/main.rs",
                "use omega::review;\nfn main() {}\n",
                cx,
            )
            .await
            .expect("seed one exact native Review hunk");
        front_door
            .focus_native_review_editor(cx)
            .expect("focus the exact native Review editor after changes appear");
        let changed = front_door
            .native_review_state(cx)
            .expect("changed native Review state");
        assert!(
            matches!(
                changed.lifecycle,
                crate::AgentDiffLifecycle::Ready | crate::AgentDiffLifecycle::Streaming
            ),
            "unexpected changed Review state: {changed:?}"
        );
        assert_eq!(changed.files.len(), 1);
        assert_eq!(
            changed
                .files
                .iter()
                .map(|file| file.hunks.len())
                .sum::<usize>(),
            1
        );
        assert_eq!(changed.selected_path.as_deref(), Some("src/main.rs"));
        assert!(
            changed.editor_focused,
            "the exact native diff editor should receive focus when changes appear"
        );

        front_door.dispatch_action(crate::workbench_shell::SelectReview, cx);
        front_door.dispatch_action(crate::workbench_shell::SelectReview, cx);
        assert_eq!(
            front_door.native_review_surface_entity_id(cx),
            Some(surface_id)
        );
        assert_eq!(
            front_door.native_review_agent_diff_entity_id(cx),
            Some(agent_diff_id)
        );
        assert_eq!(front_door.native_review_pane_entity_id(cx), Some(pane_id));
        assert_eq!(
            front_door.native_review_toolbar_entity_id(cx),
            Some(toolbar_id)
        );
        let reopened = front_door
            .native_review_state(cx)
            .expect("reopened native Review state");
        assert_eq!(reopened.selected_path, changed.selected_path);
        assert_eq!(reopened.selected_range, changed.selected_range);

        let before_keep = front_door
            .native_review_state(cx)
            .expect("state before one Keep");
        assert!(
            front_door
                .snapshot(cx)
                .bounds("omega.workbench.surface.review")
                .is_some(),
            "the Review surface must be rendered before a routed action"
        );
        front_door
            .dispatch_native_review_action(crate::Keep, cx)
            .expect("route one Keep through the exact native Review surface");
        let after_keep = front_door
            .native_review_state(cx)
            .expect("state after one Keep");
        assert_eq!(after_keep.kept_hunks, before_keep.kept_hunks + 1);
        assert_eq!(after_keep.rejected_hunks, before_keep.rejected_hunks);
        assert_eq!(
            after_keep
                .files
                .iter()
                .map(|file| file.hunks.len())
                .sum::<usize>(),
            0,
            "one Keep action must review exactly the one selected hunk"
        );

        front_door
            .seed_native_review_edit("worktree-1", "README.md", "# Reviewed artifact\n", cx)
            .await
            .expect("seed a second hunk for the editor round trip");
        front_door
            .focus_native_review_editor(cx)
            .expect("focus exact native Review editor");
        front_door.dispatch_action(editor::actions::OpenExcerpts, cx);
        assert!(
            front_door.workspace_center_is_visible(cx),
            "opening a reviewed hunk must reveal the sealed center"
        );
        assert_eq!(
            front_door
                .active_workspace_item_path(cx)
                .map(|path| path.path),
            Some(util::rel_path::rel_path("README.md").into())
        );
        assert!(
            front_door.active_workspace_item_is_focused(cx),
            "the opened center editor must receive deterministic focus"
        );

        assert_eq!(
            front_door.native_review_surface_entity_id(cx),
            Some(surface_id)
        );
        assert_eq!(
            front_door.native_review_agent_diff_entity_id(cx),
            Some(agent_diff_id)
        );
        assert_eq!(front_door.native_review_pane_entity_id(cx), Some(pane_id));
        assert_eq!(
            front_door.native_review_toolbar_entity_id(cx),
            Some(toolbar_id)
        );

        front_door
            .teardown(cx)
            .expect("native Review workbench should tear down");
    }

    #[gpui::test]
    async fn native_review_rebind_and_invalidation_isolate_stale_state(cx: &mut TestAppContext) {
        let scene = scene_with_two_worktrees("workbench_review_binding_isolation");
        let front_door = AgentWorkbenchFrontDoor::mount(scene, cx)
            .await
            .expect("Review binding-isolation scene should mount");
        front_door.dispatch_action(crate::workbench_shell::SelectReview, cx);
        front_door
            .seed_native_review_edit(
                "worktree-1",
                "src/main.rs",
                "const WORKTREE: usize = 1;\n",
                cx,
            )
            .await
            .expect("seed old binding Review change");
        let old_surface = front_door
            .native_review_surface(cx)
            .expect("old binding Review surface");
        let old_surface_id = old_surface.entity_id();
        let weak_old_surface = old_surface.downgrade();
        drop(old_surface);
        let old_state = front_door
            .native_review_state(cx)
            .expect("old binding Review state");
        let generation = old_state
            .binding
            .as_ref()
            .expect("old Review binding")
            .checkpoint
            .generation();
        assert_eq!(
            front_door.complete_native_review_generation(generation + 1, cx),
            Some(false)
        );
        let after_stale = front_door
            .native_review_state(cx)
            .expect("state after stale completion");
        assert_eq!(
            after_stale.stale_completions_ignored,
            old_state.stale_completions_ignored + 1
        );
        assert_eq!(after_stale.files, old_state.files);

        front_door
            .select_worktree_picker_row(1, cx)
            .expect("select the second worktree through the rendered picker");
        let new_surface_id = front_door
            .native_review_surface_entity_id(cx)
            .expect("new binding Review surface");
        assert_ne!(new_surface_id, old_surface_id);
        assert!(
            weak_old_surface.upgrade().is_none(),
            "the old binding must release its exact native Review entity graph"
        );
        let worktree_2 = front_door
            .fixture_worktree_id("worktree-2", cx)
            .expect("second fixture worktree");
        let new_state = front_door
            .native_review_state(cx)
            .expect("new binding Review state");
        assert_eq!(
            new_state
                .binding
                .as_ref()
                .map(|binding| binding.worktree_id),
            Some(worktree_2)
        );
        assert!(
            new_state.files.is_empty(),
            "the old worktree's action-log changes must not leak into the new binding"
        );

        let weak_new_surface = front_door
            .native_review_surface(cx)
            .expect("new binding Review surface")
            .downgrade();
        assert_eq!(
            front_door
                .invalidate_surface(omega_workbench_state::WorkSurface::Review, cx)
                .expect("invalidate native Review"),
            omega_workbench_state::TransitionEffect::DeterministicFallback
        );
        assert!(front_door.native_review_surface(cx).is_none());
        assert!(
            weak_new_surface.upgrade().is_none(),
            "invalidating Review must release its native entity graph"
        );
        assert!(
            front_door
                .snapshot(cx)
                .bounds("omega.workbench.surface.review")
                .is_none(),
            "invalidated Review must not leave foreign content rendered"
        );

        front_door
            .teardown(cx)
            .expect("Review binding-isolation workbench should tear down");
    }
}
