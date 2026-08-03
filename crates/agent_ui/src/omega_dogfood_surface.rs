use editor::{Editor, EditorElement, EditorStyle};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyContext, Render,
    Styled, TextStyle, Window, prelude::*,
};
use omega_effectd::all_work_contract::{
    AgentRef, AgentSessionRef, DelegationGrantRef, HostRef, OrganizationRef, PrincipalRef,
    RepositoryClaimLedger, RepositoryWorkClaim, RepositoryWorkClaimState, SafeInteger, ShortText,
    SignedWorkroomDeliveryOutcome, SignedWorkroomLedger, SignedWorkroomOutboxState,
    SignedWorkroomProjectionProfile, SourceRef, ThreadRef, WorkCommandActivityKind, WorkSnapshot,
    WorkroomAudience,
};
#[cfg(all(test, feature = "test-support"))]
use omega_work_index::DogfoodFixtureAdapter;
use omega_work_index::{
    DOGFOOD_PROJECT_ID, DogfoodPlanningOrigin, DogfoodPlanningViewModel, FixtureIssue,
    FixtureIssueRelationKind, FixtureLifecycleType, FixturePriority, PlanningFilter, PlanningGroup,
    PlanningSavedView, PlanningSort, PlanningViewKind, PlanningViewQuery, SECURITY_PROJECT_ID,
    github_work_ref, project_planning_view,
};
use serde::{Deserialize, Serialize};
use settings::Settings as _;
use theme_settings::ThemeSettings;
use ui::{Button, ButtonSize, ButtonStyle, Color, Icon, IconName, Label, LabelSize, prelude::*};

use crate::omega_agent_session_simulation::{AgentSessionSimulation, AgentSessionSimulationScene};
use crate::omega_status_cue::{OmegaStatus, omega_status_cue};

const DOGFOOD_SURFACE_STATE_KEY: &str = "omega_dogfood_surface_state_v1";
const MAX_USER_SAVED_VIEWS: usize = 8;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum DogfoodScene {
    #[default]
    Overview,
    List,
    Board,
    Table,
    Timeline,
    Roadmap,
    Issue,
    Session,
    Review,
}

impl DogfoodScene {
    const ALL: [Self; 9] = [
        Self::Overview,
        Self::List,
        Self::Board,
        Self::Table,
        Self::Timeline,
        Self::Roadmap,
        Self::Issue,
        Self::Session,
        Self::Review,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::List => "List",
            Self::Board => "Board",
            Self::Table => "Table",
            Self::Timeline => "Timeline",
            Self::Roadmap => "Roadmap",
            Self::Issue => "Issue",
            Self::Session => "Session",
            Self::Review => "Review",
        }
    }
}

#[derive(Clone, Debug)]
pub enum DogfoodSurfaceEvent {
    SelectionChanged {
        project_id: String,
        issue_id: String,
    },
    RepositoryClaimRequested {
        issue_id: String,
        action: DogfoodClaimAction,
    },
    WorkCommandRequested {
        issue_id: String,
        action: DogfoodWorkCommandAction,
    },
    SignedWorkroomPublishRequested {
        event_ref: String,
        effective_principal_ref: String,
        attempt_count: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DogfoodClaimAction {
    Refresh,
    Claim,
    Status,
    Heartbeat,
    Block,
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DogfoodWorkCommandAction {
    Refresh,
    AssignToMe,
    Unassign,
    Delegate,
    RevokeDelegate,
    LinkAgentSession,
    RecordHandoff,
    StopAgentSession,
    NeedsChanges,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DogfoodDelegationCandidate {
    pub agent_ref: AgentRef,
    pub host_ref: HostRef,
    pub label: String,
    pub thread_ref: ThreadRef,
    pub thread_key: String,
    pub agent_session_ref: Option<AgentSessionRef>,
    pub provider_event: Option<DogfoodProviderEventProjection>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DogfoodProviderEventProjection {
    pub provider_event_ref: SourceRef,
    pub event_id: u64,
    pub event_revision: u64,
    pub kind: WorkCommandActivityKind,
    pub summary: ShortText,
    pub loss_refs: Vec<SourceRef>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DogfoodWorkCommandContext {
    pub principal_ref: PrincipalRef,
    pub organization_ref: OrganizationRef,
    pub delegation_candidate: Option<DogfoodDelegationCandidate>,
}

pub struct DogfoodSurface {
    focus_handle: FocusHandle,
    fixture: DogfoodPlanningViewModel,
    project_id: String,
    selected_issue_id: String,
    scene: DogfoodScene,
    repository_claim_ledger: Option<RepositoryClaimLedger>,
    repository_claim_error: Option<String>,
    repository_claim_busy: bool,
    signed_workroom_ledger: Option<SignedWorkroomLedger>,
    signed_workroom_error: Option<String>,
    signed_workroom_publish_in_flight: Option<String>,
    work_command_context: Option<DogfoodWorkCommandContext>,
    work_command_context_error: Option<String>,
    work_command_snapshot: Option<WorkSnapshot>,
    work_command_error: Option<String>,
    work_command_busy: bool,
    agent_session_simulation_scene: AgentSessionSimulationScene,
    saved_view: PlanningSavedView,
    filter: PlanningFilter,
    group: PlanningGroup,
    sort: PlanningSort,
    user_saved_views: NamedSavedPlanningViews,
    view_name_editor: Entity<Editor>,
    user_saved_view_error: Option<String>,
    _view_name_subscription: gpui::Subscription,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SavedPlanningQuery {
    saved_view: PlanningSavedView,
    filter: PlanningFilter,
    group: PlanningGroup,
    sort: PlanningSort,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NamedSavedPlanningView {
    id: String,
    name: String,
    query: SavedPlanningQuery,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct NamedSavedPlanningViews {
    views: Vec<NamedSavedPlanningView>,
    selected_id: Option<String>,
    active: bool,
    next_sequence: u64,
}

impl NamedSavedPlanningViews {
    fn from_persisted(
        views: Vec<NamedSavedPlanningView>,
        active_id: Option<String>,
        selected_id: Option<String>,
        selected_matches_query: bool,
        next_sequence: u64,
        legacy_query: Option<SavedPlanningQuery>,
        legacy_active: bool,
    ) -> Self {
        let mut admitted = Vec::new();
        for view in views.into_iter().take(MAX_USER_SAVED_VIEWS) {
            if saved_view_name(&view.name).is_some()
                && view.id.starts_with("view:omega-local:")
                && !admitted.iter().any(|candidate: &NamedSavedPlanningView| {
                    candidate.id == view.id || candidate.name.eq_ignore_ascii_case(&view.name)
                })
            {
                admitted.push(view);
            }
        }
        let had_legacy_query = legacy_query.is_some();
        if admitted.is_empty()
            && let Some(query) = legacy_query
        {
            admitted.push(NamedSavedPlanningView {
                id: "view:omega-local:1".into(),
                name: "My view".into(),
                query,
            });
        }
        let active_id = active_id
            .filter(|id| admitted.iter().any(|view| &view.id == id))
            .or_else(|| {
                legacy_active
                    .then(|| admitted.first().map(|view| view.id.clone()))
                    .flatten()
            });
        let selected_id = selected_id
            .filter(|id| admitted.iter().any(|view| &view.id == id))
            .or_else(|| active_id.clone())
            .or_else(|| had_legacy_query.then(|| admitted.first()?.id.clone()));
        let observed_next_sequence = admitted
            .iter()
            .filter_map(|view| view.id.rsplit(':').next()?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        Self {
            next_sequence: next_sequence.max(observed_next_sequence).max(1),
            views: admitted,
            active: active_id.is_some() || (selected_matches_query && selected_id.is_some()),
            selected_id,
        }
    }

    fn create(&mut self, name: &str, query: SavedPlanningQuery) -> Result<(), &'static str> {
        if self.views.len() >= MAX_USER_SAVED_VIEWS {
            return Err("At most eight local Views can be saved.");
        }
        let name =
            saved_view_name(name).ok_or("View names must be 1–48 public-safe characters.")?;
        if self
            .views
            .iter()
            .any(|view| view.name.eq_ignore_ascii_case(&name))
        {
            return Err("A View with that name already exists.");
        }
        let id = format!("view:omega-local:{}", self.next_sequence.max(1));
        self.next_sequence = self.next_sequence.max(1).saturating_add(1);
        self.views.push(NamedSavedPlanningView {
            id: id.clone(),
            name,
            query,
        });
        self.selected_id = Some(id);
        self.active = true;
        Ok(())
    }

    fn apply(&mut self, id: &str) -> Option<SavedPlanningQuery> {
        let query = self.views.iter().find(|view| view.id == id)?.query;
        self.selected_id = Some(id.to_string());
        self.active = true;
        Some(query)
    }

    fn diverge(&mut self) {
        self.active = false;
    }

    fn update_active(&mut self, query: SavedPlanningQuery) -> bool {
        let Some(selected_id) = self.selected_id.as_deref() else {
            return false;
        };
        let Some(view) = self.views.iter_mut().find(|view| view.id == selected_id) else {
            return false;
        };
        view.query = query;
        self.active = true;
        true
    }

    fn rename_active(&mut self, name: &str) -> Result<(), &'static str> {
        let selected_id = self
            .selected_id
            .as_deref()
            .ok_or("Select a local View before renaming it.")?;
        let name =
            saved_view_name(name).ok_or("View names must be 1–48 public-safe characters.")?;
        if self
            .views
            .iter()
            .any(|view| view.id != selected_id && view.name.eq_ignore_ascii_case(&name))
        {
            return Err("A View with that name already exists.");
        }
        let view = self
            .views
            .iter_mut()
            .find(|view| view.id == selected_id)
            .ok_or("The selected local View is unavailable.")?;
        view.name = name;
        Ok(())
    }

    fn remove_active(&mut self) -> bool {
        let Some(selected_id) = self.selected_id.take() else {
            return false;
        };
        let previous_len = self.views.len();
        self.views.retain(|view| view.id != selected_id);
        self.active = false;
        self.views.len() != previous_len
    }
}

fn saved_view_name(value: &str) -> Option<String> {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    (!value.is_empty()
        && value.chars().count() <= 48
        && !value.chars().any(char::is_control)
        && !lower.contains("nsec1")
        && !lower.contains("ncryptsec1"))
    .then(|| value.to_string())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedDogfoodSurfaceState {
    project_id: String,
    selected_issue_id: String,
    scene: DogfoodScene,
    #[serde(default)]
    saved_view: PlanningSavedView,
    #[serde(default)]
    filter: PlanningFilter,
    #[serde(default)]
    group: PlanningGroup,
    #[serde(default)]
    sort: PlanningSort,
    #[serde(default)]
    user_saved_view: Option<SavedPlanningQuery>,
    #[serde(default)]
    user_saved_view_active: bool,
    #[serde(default)]
    user_saved_views: Vec<NamedSavedPlanningView>,
    #[serde(default)]
    active_user_saved_view_id: Option<String>,
    #[serde(default)]
    selected_user_saved_view_id: Option<String>,
    #[serde(default)]
    user_saved_view_matches_query: bool,
    #[serde(default)]
    next_user_saved_view_sequence: u64,
}

impl DogfoodSurface {
    pub fn new(
        fixture: DogfoodPlanningViewModel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let persisted = KeyValueStore::global(cx)
            .read_kvp(DOGFOOD_SURFACE_STATE_KEY)
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str::<PersistedDogfoodSurfaceState>(&json).ok())
            .filter(|state| fixture_state_is_valid(&fixture, state));
        let state = persisted.unwrap_or_else(default_fixture_state);
        let view_name_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("View name…", window, cx);
            editor
        });
        let view_name_subscription = cx.subscribe(&view_name_editor, |this, _, event, cx| {
            if matches!(event, editor::EditorEvent::Edited { .. }) {
                this.user_saved_view_error = None;
                cx.notify();
            }
        });
        Self {
            focus_handle: cx.focus_handle(),
            fixture,
            project_id: state.project_id,
            selected_issue_id: state.selected_issue_id,
            scene: state.scene,
            repository_claim_ledger: None,
            repository_claim_error: None,
            repository_claim_busy: false,
            signed_workroom_ledger: None,
            signed_workroom_error: None,
            signed_workroom_publish_in_flight: None,
            work_command_context: None,
            work_command_context_error: Some(
                "Live commands need a verified Effective Principal and Organization.".into(),
            ),
            work_command_snapshot: None,
            work_command_error: None,
            work_command_busy: false,
            agent_session_simulation_scene: AgentSessionSimulationScene::Pending,
            saved_view: state.saved_view,
            filter: state.filter,
            group: state.group,
            sort: state.sort,
            user_saved_views: NamedSavedPlanningViews::from_persisted(
                state.user_saved_views,
                state.active_user_saved_view_id,
                state.selected_user_saved_view_id,
                state.user_saved_view_matches_query,
                state.next_user_saved_view_sequence,
                state.user_saved_view,
                state.user_saved_view_active,
            ),
            view_name_editor,
            user_saved_view_error: None,
            _view_name_subscription: view_name_subscription,
        }
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn selected_issue_id(&self) -> &str {
        &self.selected_issue_id
    }

    pub fn work_command_snapshot(&self) -> Option<&WorkSnapshot> {
        self.work_command_snapshot.as_ref()
    }

    pub fn scene(&self) -> DogfoodScene {
        self.scene
    }

    /// Atomically swaps one complete planning projection while preserving a
    /// valid selection. An incomplete refresh is retained by the model layer
    /// and reaches this method only as updated freshness/loss metadata.
    pub fn set_planning_view(&mut self, fixture: DogfoodPlanningViewModel, cx: &mut Context<Self>) {
        let selection_is_valid =
            fixture.graph.issues.iter().any(|issue| {
                issue.id == self.selected_issue_id && issue.project_id == self.project_id
            });
        self.fixture = fixture;
        if !selection_is_valid {
            if let Some(issue) = self
                .fixture
                .graph
                .issues
                .iter()
                .find(|issue| issue.project_id == self.project_id && !issue.completed)
                .or_else(|| {
                    self.fixture
                        .graph
                        .issues
                        .iter()
                        .find(|issue| issue.project_id == self.project_id)
                })
            {
                self.selected_issue_id = issue.id.clone();
            }
        }
        cx.notify();
    }

    pub fn set_repository_claim_state(
        &mut self,
        ledger: Option<RepositoryClaimLedger>,
        error: Option<String>,
        busy: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(ledger) = ledger {
            self.repository_claim_ledger = Some(ledger);
        }
        self.repository_claim_error = error;
        self.repository_claim_busy = busy;
        cx.notify();
    }

    pub fn set_signed_workroom_state(
        &mut self,
        ledger: Option<SignedWorkroomLedger>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(ledger) = ledger {
            self.signed_workroom_ledger = Some(ledger);
        }
        self.signed_workroom_error = error;
        cx.notify();
    }

    pub fn finish_signed_workroom_publish(
        &mut self,
        ledger: Option<SignedWorkroomLedger>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(ledger) = ledger {
            self.signed_workroom_ledger = Some(ledger);
        }
        self.signed_workroom_error = error;
        self.signed_workroom_publish_in_flight = None;
        cx.notify();
    }

    pub fn set_work_command_context(
        &mut self,
        context: Option<DogfoodWorkCommandContext>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.work_command_context = context;
        self.work_command_context_error = error;
        cx.notify();
    }

    pub fn set_work_command_state(
        &mut self,
        snapshot: Option<WorkSnapshot>,
        error: Option<String>,
        busy: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(snapshot) = snapshot {
            self.work_command_snapshot = Some(snapshot);
        }
        self.work_command_error = error;
        self.work_command_busy = busy;
        cx.notify();
    }

    fn set_agent_session_simulation_scene(
        &mut self,
        scene: AgentSessionSimulationScene,
        cx: &mut Context<Self>,
    ) {
        self.agent_session_simulation_scene = scene;
        cx.notify();
    }

    fn selected_work_ref(&self) -> Option<String> {
        let issue = self.selected_issue()?;
        let repository = self
            .fixture
            .graph
            .source_repositories
            .iter()
            .find(|repository| repository.id == issue.repository_id)?;
        Some(github_work_ref(
            &repository.owner,
            &repository.name,
            issue.number,
        ))
    }

    fn selected_repository_claim(&self) -> Option<&RepositoryWorkClaim> {
        let work_ref = self.selected_work_ref()?;
        self.repository_claim_ledger
            .as_ref()?
            .claims
            .iter()
            .filter(|claim| claim.work_ref.0 == work_ref)
            .max_by_key(|claim| claim.generation.0)
    }

    fn request_claim_action(&mut self, action: DogfoodClaimAction, cx: &mut Context<Self>) {
        if self.repository_claim_busy {
            return;
        }
        self.repository_claim_busy = true;
        self.repository_claim_error = None;
        cx.emit(DogfoodSurfaceEvent::RepositoryClaimRequested {
            issue_id: self.selected_issue_id.clone(),
            action,
        });
        cx.notify();
    }

    fn request_work_command(&mut self, action: DogfoodWorkCommandAction, cx: &mut Context<Self>) {
        if self.work_command_busy {
            return;
        }
        if action != DogfoodWorkCommandAction::Refresh && self.work_command_context.is_none() {
            self.work_command_error = Some(
                self.work_command_context_error
                    .clone()
                    .unwrap_or_else(|| "Live command authority is unavailable.".into()),
            );
            cx.notify();
            return;
        }
        self.work_command_busy = true;
        self.work_command_error = None;
        cx.emit(DogfoodSurfaceEvent::WorkCommandRequested {
            issue_id: self.selected_issue_id.clone(),
            action,
        });
        cx.notify();
    }

    fn request_signed_workroom_publish(
        &mut self,
        event_ref: String,
        effective_principal_ref: String,
        attempt_count: u64,
        cx: &mut Context<Self>,
    ) {
        if self.signed_workroom_publish_in_flight.is_some() {
            return;
        }
        self.signed_workroom_publish_in_flight = Some(event_ref.clone());
        self.signed_workroom_error = None;
        cx.emit(DogfoodSurfaceEvent::SignedWorkroomPublishRequested {
            event_ref,
            effective_principal_ref,
            attempt_count,
        });
        cx.notify();
    }

    fn project_issues(&self) -> Vec<&FixtureIssue> {
        self.fixture
            .graph
            .issues
            .iter()
            .filter(|issue| issue.project_id == self.project_id)
            .collect()
    }

    fn planning_query(&self) -> PlanningViewQuery {
        PlanningViewQuery {
            organization_id: self.fixture.graph.organization.id.clone(),
            project_id: self.project_id.clone(),
            saved_view: self.saved_view,
            filter: self.filter,
            group: self.group,
            sort: self.sort,
            search: String::new(),
        }
    }

    fn visible_issues(&self, kind: PlanningViewKind) -> Vec<&FixtureIssue> {
        let projection = project_planning_view(&self.fixture, kind, &self.planning_query());
        projection
            .rows
            .iter()
            .filter_map(|row| {
                self.fixture
                    .graph
                    .issues
                    .iter()
                    .find(|issue| issue.id == row.issue_id)
            })
            .collect()
    }

    fn set_filter(&mut self, filter: PlanningFilter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.user_saved_views.diverge();
        self.save_state(cx);
        cx.notify();
    }

    fn set_saved_view(&mut self, saved_view: PlanningSavedView, cx: &mut Context<Self>) {
        self.saved_view = saved_view;
        self.user_saved_views.diverge();
        self.save_state(cx);
        cx.notify();
    }

    fn cycle_group(&mut self, cx: &mut Context<Self>) {
        self.group = match self.group {
            PlanningGroup::Lifecycle => PlanningGroup::Milestone,
            PlanningGroup::Milestone => PlanningGroup::Project,
            PlanningGroup::Project => PlanningGroup::Priority,
            PlanningGroup::Priority => PlanningGroup::Lifecycle,
        };
        self.user_saved_views.diverge();
        self.save_state(cx);
        cx.notify();
    }

    fn cycle_sort(&mut self, cx: &mut Context<Self>) {
        self.sort = match self.sort {
            PlanningSort::SourceOrder => PlanningSort::Priority,
            PlanningSort::Priority => PlanningSort::Title,
            PlanningSort::Title => PlanningSort::SourceOrder,
        };
        self.user_saved_views.diverge();
        self.save_state(cx);
        cx.notify();
    }

    fn current_saved_planning_query(&self) -> SavedPlanningQuery {
        SavedPlanningQuery {
            saved_view: self.saved_view,
            filter: self.filter,
            group: self.group,
            sort: self.sort,
        }
    }

    fn create_user_view(&mut self, cx: &mut Context<Self>) {
        let name = self.view_name_editor.read(cx).text(cx);
        let query = self.current_saved_planning_query();
        self.user_saved_view_error = self
            .user_saved_views
            .create(&name, query)
            .err()
            .map(str::to_string);
        self.save_state(cx);
        cx.notify();
    }

    fn apply_user_view(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(query) = self.user_saved_views.apply(id) else {
            return;
        };
        self.saved_view = query.saved_view;
        self.filter = query.filter;
        self.group = query.group;
        self.sort = query.sort;
        self.save_state(cx);
        cx.notify();
    }

    fn update_user_view(&mut self, cx: &mut Context<Self>) {
        let query = self.current_saved_planning_query();
        if !self.user_saved_views.update_active(query) {
            self.user_saved_view_error = Some("Select a local View before updating it.".into());
        } else {
            self.user_saved_view_error = None;
        }
        self.save_state(cx);
        cx.notify();
    }

    fn rename_user_view(&mut self, cx: &mut Context<Self>) {
        let name = self.view_name_editor.read(cx).text(cx);
        self.user_saved_view_error = self
            .user_saved_views
            .rename_active(&name)
            .err()
            .map(str::to_string);
        self.save_state(cx);
        cx.notify();
    }

    fn remove_user_view(&mut self, cx: &mut Context<Self>) {
        if !self.user_saved_views.remove_active() {
            self.user_saved_view_error = Some("Select a local View before removing it.".into());
        } else {
            self.user_saved_view_error = None;
        }
        self.save_state(cx);
        cx.notify();
    }

    fn selected_issue(&self) -> Option<&FixtureIssue> {
        self.fixture
            .graph
            .issues
            .iter()
            .find(|issue| issue.id == self.selected_issue_id)
    }

    fn set_scene(&mut self, scene: DogfoodScene, cx: &mut Context<Self>) {
        if self.scene != scene {
            self.scene = scene;
            self.save_state(cx);
            cx.notify();
        }
    }

    pub fn select_project(&mut self, project_id: &str, cx: &mut Context<Self>) {
        if self.project_id == project_id {
            return;
        }
        self.project_id = project_id.to_string();
        self.work_command_snapshot = None;
        self.work_command_error = None;
        self.work_command_busy = false;
        self.selected_issue_id = if project_id == DOGFOOD_PROJECT_ID {
            "issue:omega:214".into()
        } else {
            self.project_issues()
                .into_iter()
                .find(|issue| !issue.completed)
                .map(|issue| issue.id.clone())
                .unwrap_or_default()
        };
        if !self.selected_issue_id.is_empty() {
            cx.emit(DogfoodSurfaceEvent::SelectionChanged {
                project_id: self.project_id.clone(),
                issue_id: self.selected_issue_id.clone(),
            });
        }
        self.save_state(cx);
        cx.notify();
    }

    fn select_issue(&mut self, issue_id: String, open: bool, cx: &mut Context<Self>) {
        if !self
            .fixture
            .graph
            .issues
            .iter()
            .any(|issue| issue.id == issue_id && issue.project_id == self.project_id)
        {
            return;
        }
        self.selected_issue_id = issue_id;
        self.work_command_snapshot = None;
        self.work_command_error = None;
        self.work_command_busy = false;
        if open {
            self.scene = DogfoodScene::Issue;
        }
        cx.emit(DogfoodSurfaceEvent::SelectionChanged {
            project_id: self.project_id.clone(),
            issue_id: self.selected_issue_id.clone(),
        });
        self.save_state(cx);
        cx.notify();
    }

    fn save_state(&self, cx: &mut Context<Self>) {
        let state = PersistedDogfoodSurfaceState {
            project_id: self.project_id.clone(),
            selected_issue_id: self.selected_issue_id.clone(),
            scene: self.scene,
            saved_view: self.saved_view,
            filter: self.filter,
            group: self.group,
            sort: self.sort,
            user_saved_view: None,
            user_saved_view_active: false,
            user_saved_views: self.user_saved_views.views.clone(),
            active_user_saved_view_id: self
                .user_saved_views
                .active
                .then(|| self.user_saved_views.selected_id.clone())
                .flatten(),
            selected_user_saved_view_id: self.user_saved_views.selected_id.clone(),
            user_saved_view_matches_query: self.user_saved_views.active,
            next_user_saved_view_sequence: self.user_saved_views.next_sequence,
        };
        let Ok(json) = serde_json::to_string(&state) else {
            return;
        };
        let store = KeyValueStore::global(cx);
        cx.background_spawn(async move {
            if let Err(error) = store
                .write_kvp(DOGFOOD_SURFACE_STATE_KEY.to_string(), json)
                .await
            {
                log::warn!("could not persist the development Project selection: {error}");
            }
        })
        .detach();
    }

    fn select_relative(&mut self, delta: isize, cx: &mut Context<Self>) {
        let issues = self.project_issues();
        let current = issues
            .iter()
            .position(|issue| issue.id == self.selected_issue_id)
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(delta)
            .min(issues.len().saturating_sub(1));
        if let Some(issue) = issues.get(next) {
            self.select_issue(issue.id.clone(), false, cx);
        }
    }

    fn blocked_by(&self, issue: &FixtureIssue) -> Vec<&FixtureIssue> {
        self.fixture
            .graph
            .issue_relations
            .iter()
            .filter(|relation| {
                relation.related_issue_id == issue.id
                    && relation.kind == FixtureIssueRelationKind::Blocks
            })
            .filter_map(|relation| {
                self.fixture
                    .graph
                    .issues
                    .iter()
                    .find(|candidate| candidate.id == relation.issue_id)
            })
            .collect()
    }

    fn project_name(&self) -> &str {
        self.fixture
            .graph
            .projects
            .iter()
            .find(|project| project.id == self.project_id)
            .map_or("Unknown Project", |project| project.name.as_str())
    }

    fn render_view_name_input(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = ThemeSettings::get_global(cx);
        let text_style = TextStyle {
            color: cx.theme().colors().text,
            font_family: settings.ui_font.family.clone(),
            font_features: settings.ui_font.features.clone(),
            font_fallbacks: settings.ui_font.fallbacks.clone(),
            font_size: rems(0.75).into(),
            font_weight: settings.ui_font.weight,
            line_height: relative(1.3),
            ..Default::default()
        };
        div().w(px(150.)).child(EditorElement::new(
            &self.view_name_editor,
            EditorStyle {
                background: cx.theme().colors().editor_background,
                local_player: cx.theme().players().local(),
                text: text_style,
                ..Default::default()
            },
        ))
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let open = self
            .project_issues()
            .into_iter()
            .filter(|issue| !issue.completed)
            .count();
        let digest_prefix = self
            .fixture
            .fixture_sha256
            .chars()
            .take(12)
            .collect::<String>();
        v_flex()
            .gap_3()
            .pb_4()
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                h_flex()
                    .justify_between()
                    .gap_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .id("omega-dogfood-project-heading")
                                            .role(gpui::Role::Heading)
                                            .aria_level(1)
                                            .text_size(px(22.))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(self.project_name().to_string()),
                                    )
                                    .child(
                                        Label::new(if self.fixture.origin
                                            == DogfoodPlanningOrigin::Fixture
                                        {
                                            "DEV MOCKS"
                                        } else {
                                            "OWNED READ"
                                        })
                                            .size(LabelSize::XSmall)
                                            .color(if self.fixture.is_fresh_live() {
                                                Color::Success
                                            } else {
                                                Color::Warning
                                            }),
                                    ),
                            )
                            .child(
                                Label::new(format!(
                                    "v0.2.0 planning graph · {} · {} open · r{} · {}…",
                                    self.fixture.source_snapshot_at,
                                    open,
                                    self.fixture.revision,
                                    digest_prefix
                                ))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            )
                            .child(
                                Label::new(
                                    "OpenAgentsInc / Omega / Omega as the first-class All Work client",
                                )
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(project_button(
                                "dogfood",
                                "Omega v0.2.0",
                                self.project_id == DOGFOOD_PROJECT_ID,
                                cx.listener(|this, _, _, cx| {
                                    this.select_project(DOGFOOD_PROJECT_ID, cx)
                                }),
                            ))
                            .child(project_button(
                                "security",
                                "Security Work",
                                self.project_id == SECURITY_PROJECT_ID,
                                cx.listener(|this, _, _, cx| {
                                    this.select_project(SECURITY_PROJECT_ID, cx)
                                }),
                            )),
                    )
            )
            .child(h_flex().gap_1().children(DogfoodScene::ALL.map(|scene| {
                Button::new(format!("dogfood-scene-{}", scene.label()), scene.label())
                    .style(if self.scene == scene {
                        ButtonStyle::Filled
                    } else {
                        ButtonStyle::Subtle
                    })
                    .size(ButtonSize::Compact)
                    .aria_description(if self.scene == scene {
                        "Current Work view"
                    } else {
                        "Open Work view"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.set_scene(scene, cx)))
            })))
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        Label::new("Saved views")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .children(PlanningSavedView::ALL.map(|saved_view| {
                        Button::new(
                            format!("planning-saved-view-{}", saved_view.key()),
                            saved_view.label(),
                        )
                        .style(if self.saved_view == saved_view {
                            ButtonStyle::Filled
                        } else {
                            ButtonStyle::Subtle
                        })
                        .size(ButtonSize::Compact)
                        .aria_description(if self.saved_view == saved_view {
                            "Current saved Work view"
                        } else {
                            "Apply saved Work view"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_saved_view(saved_view, cx)
                        }))
                    }))
                    .children(self.user_saved_views.views.iter().map(|view| {
                        let id = view.id.clone();
                        Button::new(("planning-user-saved-view", view.id.clone()), view.name.clone())
                            .style(if self.user_saved_views.active
                                && self.user_saved_views.selected_id.as_deref() == Some(view.id.as_str()) {
                                ButtonStyle::Filled
                            } else {
                                ButtonStyle::Subtle
                            })
                            .size(ButtonSize::Compact)
                            .aria_description(if self.user_saved_views.active
                                && self.user_saved_views.selected_id.as_deref() == Some(view.id.as_str()) {
                                "Current local saved Work view"
                            } else {
                                "Apply local saved Work view"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| this.apply_user_view(&id, cx)))
                    }))
                    .child(self.render_view_name_input(cx))
                    .child(
                        Button::new("planning-create-user-view", "Save new")
                        .style(ButtonStyle::Subtle)
                        .size(ButtonSize::Compact)
                        .disabled(self.user_saved_views.views.len() >= MAX_USER_SAVED_VIEWS)
                        .on_click(cx.listener(|this, _, _, cx| this.create_user_view(cx))),
                    )
                    .child(
                        Button::new("planning-update-user-view", "Update")
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .disabled(self.user_saved_views.selected_id.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.update_user_view(cx))),
                    )
                    .child(
                        Button::new("planning-rename-user-view", "Rename")
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .disabled(self.user_saved_views.selected_id.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.rename_user_view(cx))),
                    )
                    .child(
                        Button::new("planning-remove-user-view", "Remove")
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .disabled(self.user_saved_views.selected_id.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.remove_user_view(cx))),
                    )
                    .when_some(self.user_saved_view_error.as_ref(), |row, error| {
                        row.child(Label::new(error.clone()).size(LabelSize::XSmall).color(Color::Error))
                    }),
            )
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        Label::new("Shared query")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .children([
                        ("planning-filter-all", "All", PlanningFilter::All),
                        ("planning-filter-open", "Open", PlanningFilter::Open),
                        ("planning-filter-blocked", "Blocked", PlanningFilter::Blocked),
                        (
                            "planning-filter-completed",
                            "Completed",
                            PlanningFilter::Completed,
                        ),
                    ].map(|(id, label, filter)| {
                        Button::new(id, label)
                            .style(if self.filter == filter {
                                ButtonStyle::Filled
                            } else {
                                ButtonStyle::Subtle
                            })
                            .size(ButtonSize::Compact)
                            .aria_description(if self.filter == filter {
                                "Current Work filter"
                            } else {
                                "Apply Work filter"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_filter(filter, cx)
                            }))
                    }))
                    .child(
                        Button::new(
                            "planning-group",
                            format!("Group · {}", planning_group_label(self.group)),
                        )
                        .style(ButtonStyle::Subtle)
                        .size(ButtonSize::Compact)
                        .on_click(cx.listener(|this, _, _, cx| this.cycle_group(cx))),
                    )
                    .child(
                        Button::new(
                            "planning-sort",
                            format!("Sort · {}", planning_sort_label(self.sort)),
                        )
                        .style(ButtonStyle::Subtle)
                        .size(ButtonSize::Compact)
                        .on_click(cx.listener(|this, _, _, cx| this.cycle_sort(cx))),
                    ),
            )
    }

    fn render_overview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let issues = self.project_issues();
        let completed = issues.iter().filter(|issue| issue.completed).count();
        let milestone_cards = self
            .fixture
            .graph
            .project_milestones
            .iter()
            .filter(|milestone| milestone.project_id == self.project_id)
            .map(|milestone| {
                let total = issues
                    .iter()
                    .filter(|issue| {
                        issue.project_milestone_id.as_deref() == Some(milestone.id.as_str())
                    })
                    .count();
                let done = issues
                    .iter()
                    .filter(|issue| {
                        issue.project_milestone_id.as_deref() == Some(milestone.id.as_str())
                            && issue.completed
                    })
                    .count();
                v_flex()
                    .min_w(px(190.))
                    .flex_1()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.border_variant)
                    .child(
                        Label::new(milestone.name.clone())
                            .size(LabelSize::Small)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(progress_dots(done, total, cx))
                            .child(
                                Label::new(format!("{done}/{total}"))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
            });
        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .gap_3()
                    .child(metric_card("Issues", issues.len().to_string(), cx))
                    .child(metric_card("Completed", completed.to_string(), cx))
                    .child(metric_card(
                        "Release stage",
                        if self.project_id == DOGFOOD_PROJECT_ID {
                            "Dogfood"
                        } else {
                            "Outside v0.2.0"
                        },
                        cx,
                    )),
            )
            .child(section_heading("Milestones", cx))
            .child(
                h_flex()
                    .items_stretch()
                    .gap_2()
                    .flex_wrap()
                    .children(milestone_cards),
            )
            .child(section_heading("Saved views", cx))
            .child(
                h_flex().gap_1().flex_wrap().children(
                    self.fixture
                        .graph
                        .custom_views
                        .iter()
                        .filter(|view| view.project_id == self.project_id)
                        .map(|view| {
                            Label::new(view.name.clone())
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                        }),
                ),
            )
            .child(section_heading("Planning provenance", cx))
            .child(
                v_flex()
                    .gap_1()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.border_variant)
                    .child(
                        Label::new(format!("SHA-256 · {}", self.fixture.fixture_sha256))
                            .size(LabelSize::XSmall),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(omega_status_cue(
                                "planning-provenance-status",
                                if self.fixture.is_fresh_live() {
                                    OmegaStatus::Ready
                                } else {
                                    OmegaStatus::Warning
                                },
                                "Planning source",
                            ))
                            .child(
                                Label::new(format!(
                                    "{} · {} gap(s) · {} issue(s)",
                                    self.fixture.provenance_label(),
                                    self.fixture.refresh_gap_refs.len(),
                                    self.fixture.refresh_projection_issues.len()
                                ))
                                .size(LabelSize::XSmall),
                            ),
                    )
                    .child(
                        Label::new(
                            self.fixture
                                .last_error
                                .clone()
                                .unwrap_or_else(|| "No projection loss".into()),
                        )
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    ),
            )
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        v_flex()
            .rounded_lg()
            .border_1()
            .border_color(colors.border_variant)
            .role(gpui::Role::List)
            .aria_label("Work list")
            .children(
                self.visible_issues(PlanningViewKind::List)
                    .into_iter()
                    .map(|issue| {
                        let issue_id = issue.id.clone();
                        let selected = issue.id == self.selected_issue_id;
                        let blockers = self.blocked_by(issue).len();
                        h_flex()
                            .id(issue.id.clone())
                            .min_h(px(42.))
                            .px_3()
                            .gap_3()
                            .border_b_1()
                            .border_color(colors.border_variant)
                            .cursor_pointer()
                            .role(gpui::Role::Button)
                            .tab_index(0isize)
                            .aria_label(work_row_accessibility_label(
                                &issue.identifier,
                                &issue.title,
                                issue.workflow_state_id.trim_start_matches("workflow:"),
                                issue.priority,
                                blockers,
                                issue.completed,
                            ))
                            .aria_selected(selected)
                            .when(selected, |row| row.bg(colors.element_selected))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_issue(issue_id.clone(), true, cx)
                            }))
                            .child(work_status_cue(
                                format!("work-list-status-{}", issue.id),
                                &issue.identifier,
                                issue.completed,
                                blockers > 0,
                            ))
                            .child(
                                Label::new(issue.identifier.clone())
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .child(issue.title.clone()),
                            )
                            .when(blockers > 0, |row| {
                                row.child(
                                    Label::new(format!("{blockers} blocker(s)"))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Warning),
                                )
                            })
                            .child(
                                Label::new(
                                    issue
                                        .workflow_state_id
                                        .trim_start_matches("workflow:")
                                        .to_string(),
                                )
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            )
                    }),
            )
    }

    fn render_board(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let columns = [
            ("Ready", FixtureLifecycleType::Unstarted),
            ("Active", FixtureLifecycleType::Started),
            ("Done", FixtureLifecycleType::Completed),
        ];
        h_flex()
            .items_start()
            .gap_3()
            .children(columns.map(|(label, lifecycle)| {
                let cards = self
                    .visible_issues(PlanningViewKind::Board)
                    .into_iter()
                    .filter(|issue| {
                        self.fixture
                            .graph
                            .workflow_states
                            .iter()
                            .find(|state| state.id == issue.workflow_state_id)
                            .is_some_and(|state| state.lifecycle_type == lifecycle)
                    });
                v_flex()
                    .min_w(px(210.))
                    .flex_1()
                    .gap_2()
                    .role(gpui::Role::List)
                    .aria_label(format!("{label} Work"))
                    .child(section_heading(label, cx))
                    .children(cards.map(|issue| {
                        let issue_id = issue.id.clone();
                        let blockers = self.blocked_by(issue).len();
                        let blocked = blockers > 0;
                        let selected = issue.id == self.selected_issue_id;
                        v_flex()
                            .id(format!("board-{}", issue.id))
                            .gap_2()
                            .p_3()
                            .rounded_lg()
                            .border_1()
                            .border_color(if selected {
                                colors.border_selected
                            } else {
                                colors.border_variant
                            })
                            .cursor_pointer()
                            .role(gpui::Role::Button)
                            .tab_index(0isize)
                            .aria_label(work_row_accessibility_label(
                                &issue.identifier,
                                &issue.title,
                                label,
                                issue.priority,
                                blockers,
                                issue.completed,
                            ))
                            .aria_selected(selected)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_issue(issue_id.clone(), true, cx)
                            }))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(work_status_cue(
                                        format!("work-board-status-{}", issue.id),
                                        &issue.identifier,
                                        issue.completed,
                                        blocked,
                                    ))
                                    .child(
                                        Label::new(issue.identifier.clone())
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    ),
                            )
                            .child(Label::new(issue.title.clone()).size(LabelSize::Small))
                    }))
            }))
    }

    fn render_table(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let projection = project_planning_view(
            &self.fixture,
            PlanningViewKind::Table,
            &self.planning_query(),
        );
        let rows = projection.rows;
        v_flex()
            .rounded_lg()
            .border_1()
            .border_color(colors.border_variant)
            .role(gpui::Role::List)
            .aria_label("Work table")
            .child(
                h_flex()
                    .min_h(px(34.))
                    .px_3()
                    .gap_3()
                    .bg(colors.surface_background)
                    .child(div().w(px(84.)).child("Work"))
                    .child(div().flex_1().child("Title"))
                    .child(div().w(px(90.)).child("State"))
                    .child(div().w(px(90.)).child("Priority"))
                    .child(div().w(px(110.)).child("Repository")),
            )
            .children(rows.into_iter().map(|row| {
                let issue_id = row.issue_id.clone();
                let selected = row.issue_id == self.selected_issue_id;
                h_flex()
                    .id(format!("table-{}", row.issue_id))
                    .min_h(px(38.))
                    .px_3()
                    .gap_3()
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .cursor_pointer()
                    .role(gpui::Role::Button)
                    .tab_index(0isize)
                    .aria_label(work_row_accessibility_label(
                        &row.identifier,
                        &row.title,
                        lifecycle_label(row.lifecycle),
                        row.priority,
                        row.blocked_by_count,
                        row.completed,
                    ))
                    .aria_selected(selected)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_issue(issue_id.clone(), true, cx)
                    }))
                    .child(
                        div()
                            .w(px(84.))
                            .text_size(px(11.))
                            .text_color(colors.text_muted)
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(work_status_cue(
                                        format!("work-table-status-{}", row.issue_id),
                                        &row.identifier,
                                        row.completed,
                                        row.blocked_by_count > 0,
                                    ))
                                    .child(row.identifier),
                            ),
                    )
                    .child(div().min_w_0().flex_1().truncate().child(row.title))
                    .child(
                        div()
                            .w(px(90.))
                            .text_size(px(11.))
                            .child(lifecycle_label(row.lifecycle)),
                    )
                    .child(
                        div()
                            .w(px(90.))
                            .text_size(px(11.))
                            .child(priority_label(row.priority)),
                    )
                    .child(
                        div()
                            .w(px(110.))
                            .truncate()
                            .text_size(px(11.))
                            .text_color(colors.text_muted)
                            .child(row.repository_id),
                    )
            }))
    }

    fn render_timeline(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let projection = project_planning_view(
            &self.fixture,
            PlanningViewKind::Timeline,
            &self.planning_query(),
        );
        let source_revision = projection.source_revision;
        let event_cursor = projection.event_cursor;
        let rows = projection.rows;
        let groups = projection.groups;
        v_flex()
            .gap_3()
            .role(gpui::Role::List)
            .aria_label("Work timeline")
            .child(
                Label::new(format!(
                    "Shared Work chronology · r{} · {}",
                    source_revision, event_cursor
                ))
                .size(LabelSize::XSmall)
                .color(Color::Muted),
            )
            .children(groups.into_iter().map(|(group, work_refs)| {
                let group_rows = rows.iter().filter(|row| work_refs.contains(&row.work_ref));
                v_flex()
                    .gap_2()
                    .role(gpui::Role::List)
                    .aria_label(format!("{group} timeline group"))
                    .child(section_heading(&group, cx))
                    .children(group_rows.map(|row| {
                        let issue_id = row.issue_id.clone();
                        let selected = row.issue_id == self.selected_issue_id;
                        h_flex()
                            .id(format!("timeline-{}", row.issue_id))
                            .gap_3()
                            .cursor_pointer()
                            .role(gpui::Role::Button)
                            .tab_index(0isize)
                            .aria_label(work_row_accessibility_label(
                                &row.identifier,
                                &row.title,
                                lifecycle_label(row.lifecycle),
                                row.priority,
                                row.blocked_by_count,
                                row.completed,
                            ))
                            .aria_selected(selected)
                            .child(work_status_cue(
                                format!("work-timeline-status-{}", row.issue_id),
                                &row.identifier,
                                row.completed,
                                row.blocked_by_count > 0,
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_issue(issue_id.clone(), true, cx)
                            }))
                            .child(
                                div()
                                    .w(px(150.))
                                    .truncate()
                                    .text_size(px(12.))
                                    .child(format!("{} · {}", row.identifier, row.title)),
                            )
                            .child(
                                div()
                                    .h(px(8.))
                                    .flex_1()
                                    .rounded_full()
                                    .bg(if row.completed {
                                        Color::Success.color(cx)
                                    } else if row.blocked_by_count > 0 {
                                        Color::Warning.color(cx)
                                    } else {
                                        colors.element_active
                                    }),
                            )
                            .child(
                                Label::new(if row.blocked_by_count > 0 {
                                    format!("{} blocker(s)", row.blocked_by_count)
                                } else {
                                    lifecycle_label(row.lifecycle).into()
                                })
                                .size(LabelSize::XSmall)
                                .color(
                                    if row.blocked_by_count > 0 {
                                        Color::Warning
                                    } else {
                                        Color::Muted
                                    },
                                ),
                            )
                    }))
            }))
    }

    fn render_roadmap(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let projection = project_planning_view(
            &self.fixture,
            PlanningViewKind::Roadmap,
            &self.planning_query(),
        );
        let rows = projection.rows;
        let groups = projection.groups;
        h_flex()
            .items_start()
            .gap_3()
            .flex_wrap()
            .role(gpui::Role::List)
            .aria_label("Work roadmap")
            .children(groups.into_iter().map(|(group, work_refs)| {
                let group_rows = rows
                    .iter()
                    .filter(|row| work_refs.contains(&row.work_ref))
                    .collect::<Vec<_>>();
                let completed = group_rows.iter().filter(|row| row.completed).count();
                v_flex()
                    .min_w(px(220.))
                    .flex_1()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.border_variant)
                    .role(gpui::Role::List)
                    .aria_label(format!("{group} roadmap group"))
                    .child(section_heading(&group, cx))
                    .child(
                        Label::new(format!("{completed}/{} complete", group_rows.len()))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(progress_dots(completed, group_rows.len(), cx))
                    .children(group_rows.into_iter().take(6).map(|row| {
                        let issue_id = row.issue_id.clone();
                        let selected = row.issue_id == self.selected_issue_id;
                        h_flex()
                            .id(format!("roadmap-{}", row.issue_id))
                            .gap_2()
                            .cursor_pointer()
                            .role(gpui::Role::Button)
                            .tab_index(0isize)
                            .aria_label(work_row_accessibility_label(
                                &row.identifier,
                                &row.title,
                                lifecycle_label(row.lifecycle),
                                row.priority,
                                row.blocked_by_count,
                                row.completed,
                            ))
                            .aria_selected(selected)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_issue(issue_id.clone(), true, cx)
                            }))
                            .child(work_status_cue(
                                format!("work-roadmap-status-{}", row.issue_id),
                                &row.identifier,
                                row.completed,
                                row.blocked_by_count > 0,
                            ))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(Color::Muted.color(cx))
                                    .child(format!("{} · {}", row.identifier, row.title)),
                            )
                    }))
            }))
    }

    fn render_issue(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let Some(issue) = self.selected_issue() else {
            return v_flex()
                .child("No fixture Issue selected.")
                .into_any_element();
        };
        let blockers = self.blocked_by(issue);
        let milestone = issue.project_milestone_id.as_ref().and_then(|id| {
            self.fixture
                .graph
                .project_milestones
                .iter()
                .find(|milestone| &milestone.id == id)
        });
        let repository = self
            .fixture
            .graph
            .source_repositories
            .iter()
            .find(|repository| repository.id == issue.repository_id);
        let claim = self.selected_repository_claim().cloned();
        let claim_is_active = claim.as_ref().is_some_and(|claim| {
            matches!(
                &claim.state,
                RepositoryWorkClaimState::Claimed | RepositoryWorkClaimState::Blocked
            )
        });
        let claim_state = claim
            .as_ref()
            .map(|claim| repository_claim_state_label(&claim.state))
            .unwrap_or("Unclaimed");
        let selected_work_ref = self.selected_work_ref();
        let command_snapshot = self.work_command_snapshot.as_ref().filter(|snapshot| {
            selected_work_ref
                .as_ref()
                .is_some_and(|work_ref| snapshot.summary.work_ref.0 == *work_ref)
        });
        let assignee = command_snapshot
            .and_then(|snapshot| snapshot.summary.assignee.0.as_ref())
            .map_or("Unassigned".into(), |assignee| {
                assignee.principal_ref.0.clone()
            });
        let delegate = command_snapshot
            .and_then(|snapshot| snapshot.summary.agent_delegate.as_ref())
            .and_then(|delegate| delegate.as_ref())
            .map_or("None".into(), |delegate| delegate.agent_ref.0.clone());
        let has_active_delegate = command_snapshot.is_some_and(|snapshot| {
            snapshot
                .summary
                .agent_delegate
                .as_ref()
                .is_some_and(|delegate| delegate.is_some())
        });
        let session = command_snapshot
            .and_then(|snapshot| snapshot.session_refs.last())
            .map_or("None".into(), |session| session.0.clone());
        let thread = command_snapshot
            .and_then(|snapshot| snapshot.thread_refs.last())
            .map_or("None".into(), |thread| thread.0.clone());
        let agent_session = command_snapshot
            .and_then(|snapshot| snapshot.agent_session_refs.last())
            .map_or("None".into(), |session| session.0.clone());
        let run = command_snapshot
            .and_then(|snapshot| snapshot.run_refs.last())
            .map_or("None".into(), |run| run.0.clone());
        let delegation_candidate = self
            .work_command_context
            .as_ref()
            .and_then(|context| context.delegation_candidate.as_ref());
        let execution_candidate = delegation_candidate.filter(|candidate| {
            candidate.agent_session_ref.is_some()
                && command_snapshot.is_some_and(|snapshot| {
                    snapshot
                        .summary
                        .agent_delegate
                        .as_ref()
                        .and_then(|delegate| delegate.as_ref())
                        .is_some_and(|delegate| delegate.agent_ref == candidate.agent_ref)
                })
        });
        let provider_event =
            execution_candidate.and_then(|candidate| candidate.provider_event.as_ref());
        let command_revision = command_snapshot
            .map(|snapshot| snapshot.summary.revision.0)
            .or(issue.work_revision);
        let command_context_ready = self.work_command_context.is_some()
            && self.fixture.origin != DogfoodPlanningOrigin::Fixture;
        let command_mutation_ready = command_context_ready
            && command_snapshot.is_some()
            && self.work_command_error.is_none();
        h_flex()
            .items_start()
            .gap_4()
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_4()
                    .role(gpui::Role::Group)
                    .aria_label(format!("Work detail, {}, {}", issue.identifier, issue.title))
                    .child(
                        Label::new(issue.identifier.clone())
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        div()
                            .id("omega-dogfood-issue-heading")
                            .role(gpui::Role::Heading)
                            .aria_level(2)
                            .text_size(px(20.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(issue.title.clone()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(work_status_cue(
                                "work-detail-status",
                                &issue.identifier,
                                issue.completed,
                                !blockers.is_empty(),
                            ))
                            .child(
                                Label::new(
                                    issue
                                        .workflow_state_id
                                        .trim_start_matches("workflow:")
                                        .to_string(),
                                )
                                .size(LabelSize::Small),
                            ),
                    )
                    .child(section_heading("Dependencies", cx))
                    .child(
                        v_flex()
                            .gap_1()
                            .role(gpui::Role::List)
                            .aria_label("Work dependencies")
                            .when(blockers.is_empty(), |list| {
                                list.child(
                                    div().role(gpui::Role::ListItem).child(
                                        Label::new("No typed blockers in this snapshot.")
                                            .size(LabelSize::Small)
                                            .color(Color::Muted),
                                    ),
                                )
                            })
                            .children(blockers.iter().map(|blocker| {
                                div().role(gpui::Role::ListItem).child(
                                    Label::new(format!(
                                        "Blocked by {} · {}",
                                        blocker.identifier, blocker.title
                                    ))
                                    .size(LabelSize::Small),
                                )
                            })),
                    )
                    .child(section_heading("Source", cx))
                    .child(
                        Label::new(issue.source_url.clone())
                            .size(LabelSize::Small)
                            .color(Color::Accent),
                    )
                    .child(section_heading("Labels", cx))
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .role(gpui::Role::List)
                            .aria_label("Work labels")
                            .children(issue.label_ids.iter().map(|label_id| {
                                div().role(gpui::Role::ListItem).child(
                                    Label::new(label_id.trim_start_matches("label:").to_string())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                            })),
                    )
                    .child(section_heading("Execution", cx))
                    .child(
                        Label::new(format!("{assignee} · {delegate} · {session}"))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(
                v_flex()
                    .w(px(300.))
                    .flex_none()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.border_variant)
                    .role(gpui::Role::Group)
                    .aria_label("Work inspector")
                    .child(section_heading("Inspector", cx))
                    .child(inspector_row(
                        "Work identity",
                        if self.fixture.origin == DogfoodPlanningOrigin::Fixture {
                            format!("work:fixture:{}", issue.id)
                        } else {
                            issue.identifier.clone()
                        },
                        cx,
                    ))
                    .child(inspector_row("Issue projection", issue.id.clone(), cx))
                    .child(inspector_row("Repository", issue.repository_id.clone(), cx))
                    .child(inspector_row(
                        "Source revision",
                        repository.map_or("Not supplied".into(), |value| value.revision.clone()),
                        cx,
                    ))
                    .child(inspector_row(
                        "Milestone",
                        milestone.map_or("Not supplied".into(), |value| value.name.clone()),
                        cx,
                    ))
                    .child(inspector_row(
                        "Priority",
                        priority_label(issue.priority).into(),
                        cx,
                    ))
                    .child(inspector_row("Assignee", assignee, cx))
                    .child(inspector_row("Agent delegate", delegate, cx))
                    .child(inspector_row("Thread", thread, cx))
                    .child(inspector_row("Session", session, cx))
                    .child(inspector_row("Agent Session", agent_session, cx))
                    .child(inspector_row("Run", run, cx))
                    .child(inspector_row(
                        "Eligible delegate",
                        delegation_candidate.map_or("None".into(), |candidate| {
                            format!("{} · {}", candidate.label, candidate.host_ref.0)
                        }),
                        cx,
                    ))
                    .child(inspector_row(
                        "Active Agent Session",
                        execution_candidate
                            .and_then(|candidate| candidate.agent_session_ref.as_ref())
                            .map_or("None".into(), |session| session.0.clone()),
                        cx,
                    ))
                    .child(inspector_row(
                        "Provider event",
                        provider_event.map_or("Not observed".into(), |event| {
                            event.provider_event_ref.0.clone()
                        }),
                        cx,
                    ))
                    .child(inspector_row(
                        "Authority",
                        if self.fixture.origin == DogfoodPlanningOrigin::Fixture {
                            "Simulation · read only".into()
                        } else {
                            if command_context_ready {
                                "Canonical command admission".into()
                            } else {
                                "Canonical read · command authority unavailable".into()
                            }
                        },
                        cx,
                    ))
                    .child(section_heading("Work commands", cx))
                    .child(inspector_row(
                        "Command revision",
                        command_revision.map_or("Unavailable".into(), |value| value.to_string()),
                        cx,
                    ))
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(
                                claim_button(
                                    "work-command-refresh",
                                    "Refresh",
                                    self.work_command_busy
                                        || self.fixture.origin == DogfoodPlanningOrigin::Fixture,
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.request_work_command(
                                        DogfoodWorkCommandAction::Refresh,
                                        cx,
                                    )
                                })),
                            )
                            .when(command_snapshot.is_some_and(|snapshot| {
                                snapshot.summary.assignee.0.is_none()
                            }), |controls| {
                                controls.child(
                                    claim_button(
                                        "work-command-assign",
                                        "Assign to me",
                                        self.work_command_busy || !command_mutation_ready,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.request_work_command(
                                            DogfoodWorkCommandAction::AssignToMe,
                                            cx,
                                        )
                                    })),
                                )
                            })
                            .when(command_snapshot.is_some_and(|snapshot| {
                                snapshot.summary.assignee.0.is_some() && !has_active_delegate
                            }), |controls| {
                                controls.child(
                                    claim_button(
                                        "work-command-unassign",
                                        "Unassign",
                                        self.work_command_busy || !command_mutation_ready,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.request_work_command(
                                            DogfoodWorkCommandAction::Unassign,
                                            cx,
                                        )
                                    })),
                                )
                            })
                            .when(
                                command_snapshot.is_some_and(|snapshot| {
                                    snapshot.summary.assignee.0.is_some()
                                        && !has_active_delegate
                                }) && delegation_candidate.is_some(),
                                |controls| {
                                    controls.child(
                                        claim_button(
                                            "work-command-delegate",
                                            "Delegate",
                                            self.work_command_busy || !command_mutation_ready,
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.request_work_command(
                                                DogfoodWorkCommandAction::Delegate,
                                                cx,
                                            )
                                        })),
                                    )
                                },
                            )
                            .when(has_active_delegate, |controls| {
                                controls.child(
                                    claim_button(
                                        "work-command-revoke-delegate",
                                        "Revoke delegate",
                                        self.work_command_busy || !command_mutation_ready,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.request_work_command(
                                            DogfoodWorkCommandAction::RevokeDelegate,
                                            cx,
                                        )
                                    })),
                                )
                            })
                            .when(
                                has_active_delegate
                                    && command_snapshot
                                        .is_some_and(|snapshot| snapshot.session_refs.is_empty())
                                    && execution_candidate.is_some(),
                                |controls| {
                                    controls.child(
                                        claim_button(
                                            "work-command-link-agent-session",
                                            "Link session",
                                            self.work_command_busy || !command_mutation_ready,
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.request_work_command(
                                                DogfoodWorkCommandAction::LinkAgentSession,
                                                cx,
                                            )
                                        })),
                                    )
                                },
                            )
                            .when(
                                has_active_delegate
                                    && command_snapshot.is_some_and(|snapshot| {
                                        !snapshot.session_refs.is_empty()
                                            && snapshot.agent_activity_refs.is_empty()
                                    }),
                                |controls| {
                                    controls.child(
                                        claim_button(
                                            "work-command-record-handoff",
                                            if provider_event.is_some() {
                                                "Record provider event"
                                            } else {
                                                "Record handoff"
                                            },
                                            self.work_command_busy || !command_mutation_ready,
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.request_work_command(
                                                DogfoodWorkCommandAction::RecordHandoff,
                                                cx,
                                            )
                                        })),
                                    )
                                },
                            )
                            .when(
                                has_active_delegate
                                    && command_snapshot.is_some_and(|snapshot| {
                                        !snapshot.session_refs.is_empty()
                                    }),
                                |controls| {
                                    controls.child(
                                        claim_button(
                                            "work-command-stop-agent-session",
                                            "Stop agent",
                                            self.work_command_busy || !command_mutation_ready,
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.request_work_command(
                                                DogfoodWorkCommandAction::StopAgentSession,
                                                cx,
                                            )
                                        })),
                                    )
                                },
                            )
                            .when(
                                command_snapshot.is_some_and(|snapshot| {
                                    !snapshot.agent_activity_refs.is_empty()
                                        && snapshot.owner_disposition_refs.is_empty()
                                }),
                                |controls| {
                                    controls.child(
                                        claim_button(
                                            "work-command-needs-changes",
                                            "Needs changes",
                                            self.work_command_busy || !command_mutation_ready,
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.request_work_command(
                                                DogfoodWorkCommandAction::NeedsChanges,
                                                cx,
                                            )
                                        })),
                                    )
                                },
                            ),
                    )
                    .child(
                        Label::new(
                            self.work_command_error
                                .clone()
                                .or_else(|| self.work_command_context_error.clone())
                                .unwrap_or_else(|| {
                                    "Commands use the displayed Effective Principal and Organization."
                                        .into()
                                }),
                        )
                        .size(LabelSize::XSmall)
                        .color(if self.work_command_error.is_some() {
                            Color::Error
                        } else if command_context_ready {
                            Color::Muted
                        } else {
                            Color::Warning
                        }),
                    )
                    .child(section_heading("Repository claim", cx))
                    .child(inspector_row("Claim state", claim_state.into(), cx))
                    .child(inspector_row(
                        "Holder",
                        claim
                            .as_ref()
                            .map_or("None".into(), |claim| claim.holder_ref.0.clone()),
                        cx,
                    ))
                    .child(inspector_row(
                        "Generation",
                        claim
                            .as_ref()
                            .map_or("None".into(), |claim| claim.generation.0.to_string()),
                        cx,
                    ))
                    .child(inspector_row(
                        "Last evidence",
                        claim.as_ref().map_or("None".into(), |claim| {
                            claim.last_evidence_at.0.clone()
                        }),
                        cx,
                    ))
                    .child(inspector_row(
                        "Bounded scope",
                        claim.as_ref().map_or("None".into(), |claim| {
                            format!(
                                "{} path(s) · {} hot file(s) · {} hot contract(s)",
                                claim.owned_paths.len(),
                                claim.hot_files.len(),
                                claim.hot_contracts.len()
                            )
                        }),
                        cx,
                    ))
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(
                                claim_button("claim-refresh", "Refresh", self.repository_claim_busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.request_claim_action(DogfoodClaimAction::Refresh, cx)
                                    })),
                            )
                            .child(
                                claim_button(
                                    "claim-create",
                                    "Claim packet",
                                    self.repository_claim_busy || claim_is_active,
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.request_claim_action(DogfoodClaimAction::Claim, cx)
                                })),
                            )
                            .when(claim_is_active, |controls| {
                                controls
                                    .child(
                                        claim_button("claim-status", "Status", self.repository_claim_busy)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.request_claim_action(DogfoodClaimAction::Status, cx)
                                            })),
                                    )
                                    .child(
                                        claim_button("claim-heartbeat", "Heartbeat", self.repository_claim_busy)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.request_claim_action(DogfoodClaimAction::Heartbeat, cx)
                                            })),
                                    )
                                    .child(
                                        claim_button("claim-block", "Block", self.repository_claim_busy)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.request_claim_action(DogfoodClaimAction::Block, cx)
                                            })),
                                    )
                                    .child(
                                        claim_button("claim-release", "Release", self.repository_claim_busy)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.request_claim_action(DogfoodClaimAction::Release, cx)
                                            })),
                                    )
                            }),
                    )
                    .child(
                        Label::new(
                            self.repository_claim_error
                                .clone()
                                .unwrap_or_else(|| {
                                    "Claim authority is separate from assignee, delegate, lease, verification, merge, and release authority.".into()
                                }),
                        )
                        .size(LabelSize::XSmall)
                        .color(if self.repository_claim_error.is_some() {
                            Color::Error
                        } else {
                            Color::Muted
                        }),
                    )
                    .child(inspector_row(
                        "Planning source",
                        self.fixture.provenance_label().into(),
                        cx,
                    ))
                    .child(inspector_row(
                        "Revision / cursor",
                        format!("{} / {}", self.fixture.revision, self.fixture.event_cursor),
                        cx,
                    ))
                    .child(inspector_row(
                        "Adapter generation",
                        self.fixture.adapter_generation.to_string(),
                        cx,
                    ))
                    .child(inspector_row(
                        "Projection version",
                        self.fixture.projection_version.clone(),
                        cx,
                    ))
            )
            .into_any_element()
    }

    fn render_empty_execution(&self, review: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let issue = self.selected_issue();
        v_flex()
            .min_h(px(260.))
            .items_center()
            .justify_center()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(colors.border_variant)
            .child(
                Icon::new(if review {
                    IconName::PullRequest
                } else {
                    IconName::OmegaAgent
                })
                .size(IconSize::XLarge)
                .color(Color::Muted),
            )
            .child(
                Label::new(if review {
                    "No review projection"
                } else {
                    "No live Agent Session"
                })
                .size(LabelSize::Large),
            )
            .child(
                Label::new(issue.map_or("Select an Issue first.".into(), |issue| {
                    format!("{} · Unassigned · No session", issue.identifier)
                }))
                .size(LabelSize::Small)
                .color(Color::Muted),
            )
            .child(
                Label::new("Mock · no authority")
                    .size(LabelSize::XSmall)
                    .color(Color::Warning),
            )
    }

    fn render_signed_workroom(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let work_ref = self.selected_work_ref();
        let activities = self
            .signed_workroom_ledger
            .as_ref()
            .map(|ledger| {
                ledger
                    .activities
                    .iter()
                    .filter(|activity| {
                        work_ref.as_ref().is_some_and(|work_ref| {
                            activity
                                .work_ref
                                .0
                                .as_ref()
                                .is_some_and(|value| &value.0 == work_ref)
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        v_flex()
            .gap_3()
            .role(gpui::Role::List)
            .aria_label("Signed Work history")
            .child(section_heading("Signed Work history", cx))
            .child(
                Label::new(
                    self.signed_workroom_error
                        .clone()
                        .unwrap_or_else(|| "Signed transport · non-authoritative".into()),
                )
                .size(LabelSize::XSmall)
                .color(if self.signed_workroom_error.is_some() {
                    Color::Error
                } else {
                    Color::Muted
                }),
            )
            .when(activities.is_empty(), |view| {
                view.child(
                    v_flex()
                        .min_h(px(220.))
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .rounded_lg()
                        .border_1()
                        .border_color(colors.border_variant)
                        .child(Label::new("No signed activity").size(LabelSize::Large)),
                )
            })
            .children(activities.into_iter().map(|activity| {
                let projection_profile =
                    signed_workroom_projection_profile_label(activity.projection_profile.as_ref());
                let actor_grant = signed_workroom_actor_grant_label(
                    activity.actor_grant_ref.as_ref(),
                    activity.actor_grant_generation.as_ref(),
                );
                let delivery = self.signed_workroom_ledger.as_ref().and_then(|ledger| {
                    ledger
                        .outbox
                        .iter()
                        .find(|record| record.activity.event_ref == activity.event_ref)
                });
                let delivery_state = delivery
                    .map(|record| signed_workroom_outbox_state_label(&record.state))
                    .unwrap_or("Unavailable");
                v_flex()
                    .gap_1()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.border_variant)
                    .role(gpui::Role::ListItem)
                    .aria_label(format!(
                        "Signed Workroom {:?} by {}, audience {}, profile {projection_profile}, {actor_grant}, delivery {delivery_state}",
                        activity.kind,
                        activity.actor_ref.0,
                        workroom_audience_label(&activity.audience)
                    ))
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                Label::new(format!("{:?}", activity.kind)).size(LabelSize::Small),
                            )
                            .child(
                                Label::new(workroom_audience_label(&activity.audience))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(
                        Label::new(format!(
                            "Actor {} · signer {}…",
                            activity.actor_ref.0,
                            &activity.signer_pubkey.0[..12]
                        ))
                        .size(LabelSize::XSmall),
                    )
                    .child(
                        Label::new(format!(
                            "Profile {projection_profile} · {actor_grant}"
                        ))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    )
                    .child(
                        Label::new(format!(
                            "{} · generation {} · {} parent(s)",
                            activity.occurred_at.0,
                            activity.generation.0,
                            activity.causal_parent_refs.len()
                        ))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    )
                    .when_some(delivery, |card, record| {
                        let publish_label = signed_workroom_publish_action(&record.state);
                        let event_ref = activity.event_ref.0.clone();
                        let effective_principal_ref = activity.actor_ref.0.clone();
                        let attempt_count = record.attempt_count.0;
                        let publish_in_flight = self
                            .signed_workroom_publish_in_flight
                            .as_deref()
                            == Some(event_ref.as_str());
                        let attempts = record
                            .delivery_attempts
                            .iter()
                            .rev()
                            .take(3)
                            .map(|attempt| {
                                h_flex()
                                    .justify_between()
                                    .role(gpui::Role::ListItem)
                                    .aria_label(format!(
                                        "{} at {} on {}",
                                        signed_workroom_delivery_outcome_label(&attempt.outcome),
                                        attempt.attempted_at.0,
                                        attempt.relay_url.0
                                    ))
                                    .child(
                                        Label::new(signed_workroom_delivery_outcome_label(
                                            &attempt.outcome,
                                        ))
                                        .size(LabelSize::XSmall)
                                        .color(
                                            signed_workroom_delivery_outcome_color(
                                                &attempt.outcome,
                                            ),
                                        ),
                                    )
                                    .child(
                                        Label::new(format!(
                                            "{} · {}",
                                            attempt.relay_url.0, attempt.attempted_at.0
                                        ))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                    )
                            })
                            .collect::<Vec<_>>();
                        card.child(
                            v_flex()
                                .gap_1()
                                .pt_2()
                                .border_t_1()
                                .border_color(colors.border_variant)
                                .role(gpui::Role::List)
                                .aria_label("Relay delivery attempts")
                                .child(
                                    h_flex()
                                        .justify_between()
                                        .child(omega_status_cue(
                                            format!(
                                                "signed-delivery-status-{}",
                                                activity.event_ref.0
                                            ),
                                            signed_workroom_outbox_status(&record.state),
                                            "Relay delivery",
                                        ))
                                        .child(
                                            Label::new(format!(
                                                "{}/{} relays · {} attempts",
                                                record.accepted_relay_urls.len(),
                                                record.relay_urls.len(),
                                                record.attempt_count.0
                                            ))
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                        ),
                                )
                                .when_some(publish_label, |history, label| {
                                    history.child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                Button::new(
                                                    (
                                                        "signed-workroom-publish",
                                                        event_ref.clone(),
                                                    ),
                                                    if publish_in_flight {
                                                        "Publishing…"
                                                    } else {
                                                        label
                                                    },
                                                )
                                                .style(ButtonStyle::Subtle)
                                                .size(ButtonSize::Compact)
                                                .disabled(publish_in_flight)
                                                .aria_description(
                                                    "Transport retry for the existing signed outbox event; this does not sign, enqueue, verify, merge, or release Work",
                                                )
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.request_signed_workroom_publish(
                                                        event_ref.clone(),
                                                        effective_principal_ref.clone(),
                                                        attempt_count,
                                                        cx,
                                                    )
                                                })),
                                            )
                                            .child(
                                                Label::new("Existing signed event only")
                                                    .size(LabelSize::XSmall)
                                                    .color(Color::Muted),
                                            ),
                                    )
                                })
                                .when(record.delivery_attempts.len() > 3, |history| {
                                    history.child(
                                        Label::new(format!(
                                            "{} earlier attempts",
                                            record.delivery_attempts.len() - 3
                                        ))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                    )
                                })
                                .children(attempts),
                        )
                    })
                    .when(delivery.is_none(), |card| {
                        card.child(omega_status_cue(
                            format!("signed-delivery-status-{}", activity.event_ref.0),
                            OmegaStatus::Warning,
                            "Relay delivery unavailable",
                        ))
                    })
            }))
    }

    fn render_agent_session(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let work_ref = self
            .selected_work_ref()
            .unwrap_or_else(|| "work:simulation:unselected".into());
        let projection =
            AgentSessionSimulation::for_work(&work_ref, self.agent_session_simulation_scene);
        debug_assert!(projection.validate());
        v_flex()
            .gap_4()
            .child(
                v_flex()
                    .id("omega.dogfood.agent-session-simulation")
                    .gap_3()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().colors().border_variant)
                    .role(gpui::Role::Group)
                    .aria_label(format!(
                        "Simulated Agent Session: {}",
                        self.agent_session_simulation_scene.label()
                    ))
                    .child(
                        h_flex()
                            .justify_between()
                            .child(Label::new("Agent Session simulation").size(LabelSize::Large))
                            .child(omega_status_cue(
                                "agent-session-simulation-status",
                                OmegaStatus::Warning,
                                "Agent Session simulation",
                            )),
                    )
                    .child(
                        h_flex().gap_1().flex_wrap().children(
                            AgentSessionSimulationScene::ALL
                                .into_iter()
                                .enumerate()
                                .map(|(index, scene)| {
                                    Button::new(("agent-session-scene", index), scene.label())
                                        .size(ButtonSize::Compact)
                                        .style(
                                            if scene == self.agent_session_simulation_scene {
                                                ButtonStyle::Filled
                                            } else {
                                                ButtonStyle::Subtle
                                            },
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_agent_session_simulation_scene(scene, cx)
                                        }))
                                }),
                        ),
                    )
                    .child(inspector_row("Activity", projection.activity.into(), cx))
                    .child(inspector_row("Assignee", projection.assignee_ref, cx))
                    .child(inspector_row(
                        "Agent Delegate",
                        projection.agent_delegate_ref,
                        cx,
                    ))
                    .child(inspector_row(
                        "Delegation Grant",
                        projection.delegation_grant_ref,
                        cx,
                    ))
                    .child(inspector_row(
                        "Repository Claim",
                        projection.repository_claim_ref,
                        cx,
                    ))
                    .child(inspector_row("Lease", projection.lease_ref, cx))
                    .child(inspector_row("Thread", projection.thread_ref, cx))
                    .child(inspector_row("Session", projection.session_ref, cx))
                    .child(inspector_row(
                        "Agent Session",
                        projection.agent_session_ref,
                        cx,
                    ))
                    .child(inspector_row("Run", projection.run_ref, cx))
                    .child(inspector_row("Host", projection.host_ref, cx))
                    .child(inspector_row(
                        "Generation",
                        projection.generation.to_string(),
                        cx,
                    ))
                    .child(inspector_row("Plan", projection.plan_ref, cx))
                    .child(inspector_row(
                        "Question",
                        projection.question.unwrap_or("None").into(),
                        cx,
                    ))
                    .child(inspector_row(
                        "Result",
                        projection.result.unwrap_or("None").into(),
                        cx,
                    ))
                    .child(inspector_row(
                        "Artifact",
                        projection.artifact_ref.unwrap_or_else(|| "None".into()),
                        cx,
                    ))
                    .child(inspector_row(
                        "Work Review",
                        projection.work_review.unwrap_or("None").into(),
                        cx,
                    ))
                    .child(inspector_row(
                        "Effect",
                        projection.effect_ref.unwrap_or_else(|| "None".into()),
                        cx,
                    ))
                    .child(inspector_row("Receipt", "None".into(), cx))
                    .child(inspector_row("Owner Disposition", "None".into(), cx))
                    .child(
                        Label::new(
                            "No live command, claim, lease, evidence, verification, receipt, release, or owner authority.",
                        )
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    ),
            )
            .child(self.render_signed_workroom(cx))
    }
}

fn workroom_audience_label(audience: &WorkroomAudience) -> &'static str {
    match audience {
        WorkroomAudience::Public => "Public",
        WorkroomAudience::Organization => "Organization",
        WorkroomAudience::Team => "Team",
        WorkroomAudience::Workroom => "Workroom",
        WorkroomAudience::Private => "Private",
        WorkroomAudience::OwnerOnly => "Owner only",
    }
}

fn signed_workroom_projection_profile_label(
    profile: Option<&SignedWorkroomProjectionProfile>,
) -> &'static str {
    match profile {
        None | Some(SignedWorkroomProjectionProfile::OpenagentsSignedWorkroomV1) => "v1 legacy",
        Some(SignedWorkroomProjectionProfile::OpenagentsSignedWorkroomV2) => "v2 current",
    }
}

fn signed_workroom_actor_grant_label(
    grant_ref: Option<&Option<DelegationGrantRef>>,
    generation: Option<&Option<SafeInteger>>,
) -> String {
    let grant_ref = grant_ref.and_then(|grant_ref| grant_ref.as_ref());
    let generation = generation.and_then(|generation| generation.as_ref());
    match (grant_ref, generation) {
        (Some(grant_ref), Some(generation)) => format!(
            "Purpose-bound actor grant {} · generation {}",
            grant_ref.0, generation.0
        ),
        (None, None) => "Direct signer · no actor grant".into(),
        _ => "Incomplete actor grant binding".into(),
    }
}

fn signed_workroom_outbox_state_label(state: &SignedWorkroomOutboxState) -> &'static str {
    match state {
        SignedWorkroomOutboxState::Pending => "Pending",
        SignedWorkroomOutboxState::Publishing => "Partial",
        SignedWorkroomOutboxState::Accepted => "Accepted",
        SignedWorkroomOutboxState::Failed => "Failed",
        SignedWorkroomOutboxState::Superseded => "Superseded",
        SignedWorkroomOutboxState::Revoked => "Revoked",
    }
}

fn signed_workroom_outbox_status(state: &SignedWorkroomOutboxState) -> OmegaStatus {
    match state {
        SignedWorkroomOutboxState::Pending => OmegaStatus::Ready,
        SignedWorkroomOutboxState::Publishing => OmegaStatus::Running,
        SignedWorkroomOutboxState::Accepted => OmegaStatus::Complete,
        SignedWorkroomOutboxState::Failed => OmegaStatus::Failed,
        SignedWorkroomOutboxState::Superseded => OmegaStatus::Warning,
        SignedWorkroomOutboxState::Revoked => OmegaStatus::Blocked,
    }
}

fn signed_workroom_publish_action(state: &SignedWorkroomOutboxState) -> Option<&'static str> {
    match state {
        SignedWorkroomOutboxState::Pending => Some("Publish to relays"),
        SignedWorkroomOutboxState::Publishing | SignedWorkroomOutboxState::Failed => {
            Some("Retry unresolved relays")
        }
        SignedWorkroomOutboxState::Accepted
        | SignedWorkroomOutboxState::Superseded
        | SignedWorkroomOutboxState::Revoked => None,
    }
}

fn signed_workroom_delivery_outcome_label(outcome: &SignedWorkroomDeliveryOutcome) -> &'static str {
    match outcome {
        SignedWorkroomDeliveryOutcome::Accepted => "Accepted",
        SignedWorkroomDeliveryOutcome::Rejected => "Rejected",
        SignedWorkroomDeliveryOutcome::Unreachable => "Unreachable",
    }
}

fn signed_workroom_delivery_outcome_color(outcome: &SignedWorkroomDeliveryOutcome) -> Color {
    match outcome {
        SignedWorkroomDeliveryOutcome::Accepted => Color::Success,
        SignedWorkroomDeliveryOutcome::Rejected => Color::Error,
        SignedWorkroomDeliveryOutcome::Unreachable => Color::Warning,
    }
}

fn repository_claim_state_label(state: &RepositoryWorkClaimState) -> &'static str {
    match state {
        RepositoryWorkClaimState::Claimed => "Claimed",
        RepositoryWorkClaimState::Blocked => "Blocked",
        RepositoryWorkClaimState::Released => "Released",
        RepositoryWorkClaimState::Superseded => "Superseded",
    }
}

fn claim_button(id: &'static str, label: &'static str, disabled: bool) -> Button {
    Button::new(id, label)
        .style(ButtonStyle::Subtle)
        .size(ButtonSize::Compact)
        .disabled(disabled)
}

impl Render for DogfoodSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut key_context = KeyContext::new_with_defaults();
        key_context.add("OmegaDogfoodFixture");
        v_flex()
            .id("omega-dogfood-surface")
            .debug_selector(|| "omega.omega.dogfood-fixture".into())
            .key_context(key_context)
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .p_5()
            .gap_5()
            .role(gpui::Role::Main)
            .aria_label("Omega v0.2.0 development mock planning surface")
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified()
                    || this.view_name_editor.focus_handle(cx).is_focused(window)
                {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "up" | "k" => this.select_relative(-1, cx),
                    "down" | "j" => this.select_relative(1, cx),
                    "enter" => this.set_scene(DogfoodScene::Issue, cx),
                    "1" => this.set_scene(DogfoodScene::Overview, cx),
                    "2" => this.set_scene(DogfoodScene::List, cx),
                    "3" => this.set_scene(DogfoodScene::Board, cx),
                    "4" => this.set_scene(DogfoodScene::Table, cx),
                    "5" => this.set_scene(DogfoodScene::Timeline, cx),
                    "6" => this.set_scene(DogfoodScene::Roadmap, cx),
                    "7" => this.set_scene(DogfoodScene::Session, cx),
                    "8" => this.set_scene(DogfoodScene::Review, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(self.render_header(cx))
            .child(match self.scene {
                DogfoodScene::Overview => self.render_overview(cx).into_any_element(),
                DogfoodScene::List => self.render_list(cx).into_any_element(),
                DogfoodScene::Board => self.render_board(cx).into_any_element(),
                DogfoodScene::Table => self.render_table(cx).into_any_element(),
                DogfoodScene::Timeline => self.render_timeline(cx).into_any_element(),
                DogfoodScene::Roadmap => self.render_roadmap(cx).into_any_element(),
                DogfoodScene::Issue => self.render_issue(cx).into_any_element(),
                DogfoodScene::Session => self.render_agent_session(cx).into_any_element(),
                DogfoodScene::Review => self.render_empty_execution(true, cx).into_any_element(),
            })
    }
}

impl Focusable for DogfoodSurface {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DogfoodSurfaceEvent> for DogfoodSurface {}

fn default_fixture_state() -> PersistedDogfoodSurfaceState {
    PersistedDogfoodSurfaceState {
        project_id: DOGFOOD_PROJECT_ID.into(),
        selected_issue_id: "issue:omega:214".into(),
        scene: DogfoodScene::Overview,
        saved_view: PlanningSavedView::All,
        filter: PlanningFilter::All,
        group: PlanningGroup::Lifecycle,
        sort: PlanningSort::SourceOrder,
        user_saved_view: None,
        user_saved_view_active: false,
        user_saved_views: Vec::new(),
        active_user_saved_view_id: None,
        selected_user_saved_view_id: None,
        user_saved_view_matches_query: false,
        next_user_saved_view_sequence: 1,
    }
}

fn fixture_state_is_valid(
    fixture: &DogfoodPlanningViewModel,
    state: &PersistedDogfoodSurfaceState,
) -> bool {
    fixture
        .graph
        .projects
        .iter()
        .any(|project| project.id == state.project_id)
        && fixture.graph.issues.iter().any(|issue| {
            issue.id == state.selected_issue_id && issue.project_id == state.project_id
        })
}

fn project_button(
    id: &'static str,
    label: &'static str,
    selected: bool,
    listener: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id, label)
        .style(if selected {
            ButtonStyle::Filled
        } else {
            ButtonStyle::Subtle
        })
        .size(ButtonSize::Compact)
        .aria_description(if selected {
            "Current Work project"
        } else {
            "Open Work project"
        })
        .on_click(listener)
}

fn work_status_cue(
    id: impl Into<gpui::ElementId>,
    context: &str,
    completed: bool,
    blocked: bool,
) -> gpui::AnyElement {
    omega_status_cue(
        id,
        work_omega_status(completed, blocked),
        &format!("{context} Work"),
    )
}

fn work_omega_status(completed: bool, blocked: bool) -> OmegaStatus {
    if completed {
        OmegaStatus::Complete
    } else if blocked {
        OmegaStatus::Blocked
    } else {
        OmegaStatus::Ready
    }
}

fn progress_dots(done: usize, total: usize, cx: &App) -> impl IntoElement {
    let done_color = Color::Success.color(cx);
    let remaining_color = Color::Muted.color(cx);
    h_flex()
        .gap_1()
        .children((0..total.max(1)).map(move |index| {
            div().size(px(7.)).rounded_full().bg(if index < done {
                done_color
            } else {
                remaining_color
            })
        }))
}

fn metric_card(label: &str, value: impl Into<String>, cx: &App) -> impl IntoElement {
    let colors = cx.theme().colors();
    v_flex()
        .min_w(px(150.))
        .gap_1()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(colors.border_variant)
        .child(
            Label::new(label.to_string())
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .child(
            Label::new(value.into())
                .size(LabelSize::Large)
                .weight(gpui::FontWeight::SEMIBOLD),
        )
}

fn section_heading(label: &str, cx: &App) -> impl IntoElement {
    div()
        .id(format!("omega-dogfood-section-heading-{label}"))
        .role(gpui::Role::Heading)
        .aria_level(2)
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(cx.theme().colors().text)
        .child(label.to_string())
}

fn inspector_row(label: &str, value: String, cx: &App) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .pb_2()
        .border_b_1()
        .border_color(cx.theme().colors().border_variant)
        .child(
            Label::new(label.to_string())
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .child(div().text_size(px(12.)).child(value))
}

fn priority_label(priority: FixturePriority) -> &'static str {
    match priority {
        FixturePriority::NoPriority => "None",
        FixturePriority::Urgent => "Urgent",
        FixturePriority::High => "High",
        FixturePriority::Normal => "Normal",
        FixturePriority::Low => "Low",
    }
}

fn lifecycle_label(lifecycle: FixtureLifecycleType) -> &'static str {
    match lifecycle {
        FixtureLifecycleType::Backlog => "Backlog",
        FixtureLifecycleType::Unstarted => "Ready",
        FixtureLifecycleType::Started => "Active",
        FixtureLifecycleType::Completed => "Done",
        FixtureLifecycleType::Canceled => "Canceled",
        FixtureLifecycleType::Planned => "Planned",
    }
}

fn work_row_accessibility_label(
    identifier: &str,
    title: &str,
    state: &str,
    priority: FixturePriority,
    blocker_count: usize,
    completed: bool,
) -> String {
    let attention = if completed {
        "completed".to_string()
    } else if blocker_count > 0 {
        format!("blocked by {blocker_count}")
    } else {
        "not blocked".to_string()
    };
    format!(
        "{identifier}, {title}, state {state}, priority {}, {attention}",
        priority_label(priority)
    )
}

fn planning_group_label(group: PlanningGroup) -> &'static str {
    match group {
        PlanningGroup::Lifecycle => "Lifecycle",
        PlanningGroup::Milestone => "Milestone",
        PlanningGroup::Project => "Project",
        PlanningGroup::Priority => "Priority",
    }
}

fn planning_sort_label(sort: PlanningSort) -> &'static str {
    match sort {
        PlanningSort::SourceOrder => "Planning",
        PlanningSort::Priority => "Priority",
        PlanningSort::Title => "Title",
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;

    #[test]
    fn fresh_fixture_state_opens_the_dogfood_project_on_omega_214() {
        let state = default_fixture_state();
        assert_eq!(state.project_id, DOGFOOD_PROJECT_ID);
        assert_eq!(state.selected_issue_id, "issue:omega:214");
        assert_eq!(state.scene, DogfoodScene::Overview);
        assert_eq!(state.saved_view, PlanningSavedView::All);
    }

    #[test]
    fn github_work_identity_matches_the_canonical_all_work_slug() {
        assert_eq!(
            github_work_ref("OpenAgentsInc", "omega", 214),
            "work:github:openagentsinc-omega:214"
        );
    }

    #[test]
    fn persisted_scene_keeps_one_issue_identity_and_rejects_cross_project_state() {
        let fixture = DogfoodPlanningViewModel::from_fixture(
            DogfoodFixtureAdapter::load_for_tests().expect("valid fixture"),
        );
        for scene in DogfoodScene::ALL {
            let state = PersistedDogfoodSurfaceState {
                project_id: DOGFOOD_PROJECT_ID.into(),
                selected_issue_id: "issue:omega:214".into(),
                scene,
                saved_view: PlanningSavedView::CriticalPath,
                filter: PlanningFilter::Open,
                group: PlanningGroup::Milestone,
                sort: PlanningSort::Priority,
                user_saved_view: None,
                user_saved_view_active: false,
                user_saved_views: Vec::new(),
                active_user_saved_view_id: None,
                selected_user_saved_view_id: None,
                user_saved_view_matches_query: false,
                next_user_saved_view_sequence: 1,
            };
            assert!(fixture_state_is_valid(&fixture, &state));
        }
        let invalid = PersistedDogfoodSurfaceState {
            project_id: SECURITY_PROJECT_ID.into(),
            selected_issue_id: "issue:omega:214".into(),
            scene: DogfoodScene::Issue,
            saved_view: PlanningSavedView::All,
            filter: PlanningFilter::All,
            group: PlanningGroup::Lifecycle,
            sort: PlanningSort::SourceOrder,
            user_saved_view: None,
            user_saved_view_active: false,
            user_saved_views: Vec::new(),
            active_user_saved_view_id: None,
            selected_user_saved_view_id: None,
            user_saved_view_matches_query: false,
            next_user_saved_view_sequence: 1,
        };
        assert!(!fixture_state_is_valid(&fixture, &invalid));
    }

    #[test]
    fn older_persisted_query_defaults_to_all_work_saved_view() {
        let state: PersistedDogfoodSurfaceState = serde_json::from_value(serde_json::json!({
            "projectId": DOGFOOD_PROJECT_ID,
            "selectedIssueId": "issue:omega:214",
            "scene": "list",
            "filter": "open",
            "group": "lifecycle",
            "sort": "source_order"
        }))
        .expect("backward-compatible saved state");
        assert_eq!(state.saved_view, PlanningSavedView::All);
        assert_eq!(state.user_saved_view, None);
        assert!(!state.user_saved_view_active);
    }

    #[test]
    fn named_saved_views_create_apply_update_rename_remove_and_stay_bounded() {
        let initial = SavedPlanningQuery {
            saved_view: PlanningSavedView::CriticalPath,
            filter: PlanningFilter::Open,
            group: PlanningGroup::Milestone,
            sort: PlanningSort::Priority,
        };
        let updated = SavedPlanningQuery {
            saved_view: PlanningSavedView::Blocked,
            filter: PlanningFilter::Blocked,
            group: PlanningGroup::Priority,
            sort: PlanningSort::Title,
        };
        let mut views = NamedSavedPlanningViews::default();

        views.create("Critical now", initial).expect("first View");
        let first_id = views.selected_id.clone().expect("selected View");
        views.create("Blocked later", updated).expect("second View");
        assert_eq!(views.views.len(), 2);
        assert_eq!(views.apply(&first_id), Some(initial));

        views.diverge();
        assert!(!views.active);
        assert_eq!(views.selected_id.as_deref(), Some(first_id.as_str()));
        assert_eq!(views.apply(&first_id), Some(initial));
        assert!(views.update_active(updated));
        views.rename_active("Critical reviewed").expect("rename");
        assert_eq!(views.views[0].name, "Critical reviewed");
        assert!(views.remove_active());
        assert_eq!(views.views.len(), 1);

        assert!(views.create("nsec1-secret", initial).is_err());
        assert!(views.create("Blocked later", initial).is_err());
    }

    #[test]
    fn legacy_single_saved_view_migrates_to_one_stable_named_view() {
        let query = SavedPlanningQuery {
            saved_view: PlanningSavedView::CriticalPath,
            filter: PlanningFilter::Open,
            group: PlanningGroup::Milestone,
            sort: PlanningSort::Priority,
        };
        let views = NamedSavedPlanningViews::from_persisted(
            Vec::new(),
            None,
            None,
            false,
            0,
            Some(query),
            true,
        );
        assert_eq!(views.views.len(), 1);
        assert_eq!(views.views[0].id, "view:omega-local:1");
        assert_eq!(views.views[0].name, "My view");
        assert_eq!(views.selected_id.as_deref(), Some("view:omega-local:1"));
        assert!(views.active);
    }

    #[test]
    fn signed_workroom_delivery_labels_stay_transport_specific() {
        assert_eq!(signed_workroom_projection_profile_label(None), "v1 legacy");
        assert_eq!(
            signed_workroom_projection_profile_label(Some(
                &SignedWorkroomProjectionProfile::OpenagentsSignedWorkroomV2
            )),
            "v2 current"
        );
        assert_eq!(
            signed_workroom_actor_grant_label(None, None),
            "Direct signer · no actor grant"
        );
        assert_eq!(
            signed_workroom_actor_grant_label(
                Some(&Some(DelegationGrantRef(
                    "delegation-grant:omega-216:3".into()
                ))),
                Some(&Some(SafeInteger(3)))
            ),
            "Purpose-bound actor grant delegation-grant:omega-216:3 · generation 3"
        );
        assert_eq!(
            signed_workroom_actor_grant_label(
                Some(&Some(DelegationGrantRef(
                    "delegation-grant:omega-216:3".into()
                ))),
                Some(&None)
            ),
            "Incomplete actor grant binding"
        );
        assert_eq!(
            signed_workroom_outbox_state_label(&SignedWorkroomOutboxState::Publishing),
            "Partial"
        );
        assert_eq!(
            signed_workroom_outbox_state_label(&SignedWorkroomOutboxState::Accepted),
            "Accepted"
        );
        assert_eq!(
            signed_workroom_delivery_outcome_label(&SignedWorkroomDeliveryOutcome::Unreachable),
            "Unreachable"
        );
        assert_eq!(
            signed_workroom_outbox_status(&SignedWorkroomOutboxState::Pending),
            OmegaStatus::Ready
        );
        assert_eq!(
            signed_workroom_outbox_status(&SignedWorkroomOutboxState::Accepted),
            OmegaStatus::Complete
        );
        assert_eq!(
            signed_workroom_outbox_status(&SignedWorkroomOutboxState::Failed),
            OmegaStatus::Failed
        );
        assert_eq!(
            signed_workroom_publish_action(&SignedWorkroomOutboxState::Pending),
            Some("Publish to relays")
        );
        assert_eq!(
            signed_workroom_publish_action(&SignedWorkroomOutboxState::Publishing),
            Some("Retry unresolved relays")
        );
        assert_eq!(
            signed_workroom_publish_action(&SignedWorkroomOutboxState::Failed),
            Some("Retry unresolved relays")
        );
        assert_eq!(
            signed_workroom_publish_action(&SignedWorkroomOutboxState::Accepted),
            None
        );
        assert_eq!(
            signed_workroom_publish_action(&SignedWorkroomOutboxState::Superseded),
            None
        );
        assert_eq!(
            signed_workroom_publish_action(&SignedWorkroomOutboxState::Revoked),
            None
        );
    }

    #[test]
    fn work_status_prefers_completion_then_blocker_then_readiness() {
        assert_eq!(work_omega_status(true, true), OmegaStatus::Complete);
        assert_eq!(work_omega_status(false, true), OmegaStatus::Blocked);
        assert_eq!(work_omega_status(false, false), OmegaStatus::Ready);
    }

    #[test]
    fn work_row_accessibility_name_uses_visible_domain_facts_only() {
        let label = work_row_accessibility_label(
            "OMEGA-217",
            "Restore accessibility",
            "active",
            FixturePriority::High,
            2,
            false,
        );
        assert_eq!(
            label,
            "OMEGA-217, Restore accessibility, state active, priority High, blocked by 2"
        );
        for forbidden in ["https://", "/Users/", "signature", "payload", "token"] {
            assert!(!label.contains(forbidden), "leaked {forbidden}");
        }
    }
}
use db::kvp::KeyValueStore;
