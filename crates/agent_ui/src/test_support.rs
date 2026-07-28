use acp_thread::{AgentConnection, StubAgentConnection};
use agent_client_protocol::schema::v1 as acp;
use agent_servers::{AgentServer, AgentServerDelegate};
use anyhow::{Context as _, Result, bail};
use gpui::{
    Action, AnyWindowHandle, App, AppContext as _, Context, DebugRenderSnapshot, Entity, EntityId,
    EventEmitter, FocusHandle, Focusable, IntoElement, Pixels, Render, Task, TestAppContext,
    VisualTestContext, Window, div, px, size,
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

pub struct AgentWorkbenchFrontDoor {
    scene: WorkbenchScene,
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
        for path in &root_paths {
            fs.insert_tree(path, serde_json::json!({"README.md": "# Fixture"}))
                .await;
        }
        let project =
            Project::test(fs, root_paths.iter().map(std::path::PathBuf::as_path), cx).await;
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
        } else {
            probe.require_absent(WORKBENCH_COMPOSER_SELECTOR)?;
            probe.require_absent(WORKBENCH_TRANSCRIPT_SELECTOR)?;
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
    };

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
