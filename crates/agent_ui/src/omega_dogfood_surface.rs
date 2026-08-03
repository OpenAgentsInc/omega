use editor::{Editor, EditorElement, EditorStyle};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyContext, Render,
    Styled, TextStyle, Window, prelude::*,
};
use omega_effectd::all_work_contract::{
    AgentRef, AgentSessionRef, DelegationGrantRef, HostRef, OrganizationRef, PrincipalRef,
    RepositoryClaimLedger, RepositoryWorkClaim, RepositoryWorkClaimState, SafeInteger, ShortText,
    SignedWorkroomDeliveryOutcome, SignedWorkroomLedger, SignedWorkroomOutboxState,
    SignedWorkroomProjectionProfile, SourceRef, ThreadRef, WorkCommandActivityKind,
    WorkSessionState, WorkSnapshot, WorkroomAudience,
};
#[cfg(test)]
use omega_work_index::DogfoodFixtureAdapter;
use omega_work_index::{
    DOGFOOD_PROJECT_ID, DogfoodPlanningOrigin, DogfoodPlanningViewModel, FixtureIssue,
    FixtureIssueRelationKind, FixtureLifecycleType, FixturePriority, PlanningAttentionKind,
    PlanningAttentionProjection, PlanningFilter, PlanningGroup, PlanningSavedView, PlanningSort,
    PlanningViewKind, PlanningViewProjection, PlanningViewQuery, SECURITY_PROJECT_ID,
    github_work_ref, project_planning_view_with_attention,
};
use serde::{Deserialize, Serialize};
use settings::Settings as _;
use theme_settings::ThemeSettings;
use ui::{Button, ButtonSize, ButtonStyle, Color, Icon, IconName, Label, LabelSize, prelude::*};

use crate::omega_agent_session_simulation::{AgentSessionSimulation, AgentSessionSimulationScene};
use crate::omega_status_cue::{OmegaStatus, omega_status_cue};

const DOGFOOD_SURFACE_STATE_KEY: &str = "omega_dogfood_surface_state_v1";
const MAX_USER_SAVED_VIEWS: usize = 8;
pub const DOGFOOD_SIGNED_WORKROOM_REF: &str = "workroom:omega:release-v0.2.0";

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
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
    SignedWorkroomCheckpointRequested {
        work_ref: String,
        causal_parent_refs: Vec<String>,
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
    signed_workroom_checkpoint_in_flight: bool,
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
            .or_else(|| {
                had_legacy_query
                    .then(|| admitted.first().map(|view| view.id.clone()))
                    .flatten()
            });
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
            signed_workroom_checkpoint_in_flight: false,
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

    pub fn finish_signed_workroom_checkpoint(
        &mut self,
        ledger: Option<SignedWorkroomLedger>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(ledger) = ledger {
            self.signed_workroom_ledger = Some(ledger);
        }
        self.signed_workroom_error = error;
        self.signed_workroom_checkpoint_in_flight = false;
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
        if self.signed_workroom_checkpoint_in_flight
            || self.signed_workroom_publish_in_flight.is_some()
        {
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

    fn request_signed_workroom_checkpoint(
        &mut self,
        work_ref: String,
        causal_parent_refs: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if self.signed_workroom_checkpoint_in_flight
            || self.signed_workroom_publish_in_flight.is_some()
        {
            return;
        }
        self.signed_workroom_checkpoint_in_flight = true;
        self.signed_workroom_error = None;
        cx.emit(DogfoodSurfaceEvent::SignedWorkroomCheckpointRequested {
            work_ref,
            causal_parent_refs,
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

    fn planning_attention_projection(&self) -> PlanningAttentionProjection {
        let Some(selected_work_ref) = self.selected_work_ref() else {
            return PlanningAttentionProjection::default();
        };
        let Some(snapshot) = self
            .work_command_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.summary.work_ref.0 == selected_work_ref)
        else {
            return PlanningAttentionProjection::default();
        };
        let latest_session = snapshot
            .session_projections
            .as_ref()
            .and_then(|sessions| sessions.last());
        let latest_activity = snapshot
            .agent_activity_projections
            .as_ref()
            .and_then(|activities| activities.last());
        let facts = planning_attention_facts(
            latest_session.map(|session| &session.state),
            latest_activity.map(|activity| &activity.kind),
        );
        PlanningAttentionProjection {
            by_work_ref: (!facts.is_empty())
                .then_some((selected_work_ref, facts))
                .into_iter()
                .collect(),
            // The detail reader currently holds one selected Work snapshot. It
            // must not claim complete portfolio attention coverage.
            complete: false,
        }
    }

    fn planning_projection(&self, kind: PlanningViewKind) -> PlanningViewProjection {
        project_planning_view_with_attention(
            &self.fixture,
            kind,
            &self.planning_query(),
            &self.planning_attention_projection(),
        )
    }

    fn visible_issues(&self, kind: PlanningViewKind) -> Vec<&FixtureIssue> {
        let projection = self.planning_projection(kind);
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
        let selected_attention_snapshot_available =
            self.selected_work_ref().is_some_and(|work_ref| {
                self.work_command_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.summary.work_ref.0 == work_ref)
            });
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
                    .debug_selector(move || scene_debug_selector(scene))
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
                        Button::new(
                            format!("planning-user-saved-view-{}", view.id),
                            view.name.clone(),
                        )
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
            .when(
                matches!(
                    self.saved_view,
                    PlanningSavedView::AgentActive | PlanningSavedView::NeedsOwner
                ),
                |header| {
                    header.child(
                        Label::new(if selected_attention_snapshot_available {
                            "Partial attention · exact selected Work snapshot only"
                        } else {
                            "Attention unavailable · refresh the selected Work snapshot"
                        })
                        .size(LabelSize::XSmall)
                        .color(Color::Warning),
                    )
                },
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
            .id("omega-dogfood-work-list")
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
                            .debug_selector({
                                let issue_id = issue.id.clone();
                                move || work_row_debug_selector("list", &issue_id)
                            })
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
                            .on_click(cx.listener(move |this, _, window, cx| {
                                // A pointer activation must leave the planning
                                // surface holding the keyboard. Without this the
                                // window drops focus entirely and Up/Down, J/K,
                                // Enter, and the scene digits stop arriving.
                                window.focus(&this.focus_handle, cx);
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
            .id("omega-dogfood-work-board")
            .items_start()
            .gap_3()
            .role(gpui::Role::Group)
            .aria_label("Work board")
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
                    .id(format!("omega-dogfood-board-column-{label}"))
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
                            .debug_selector({
                                let issue_id = issue.id.clone();
                                move || work_row_debug_selector("board", &issue_id)
                            })
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
                            .on_click(cx.listener(move |this, _, window, cx| {
                                // A pointer activation must leave the planning
                                // surface holding the keyboard. Without this the
                                // window drops focus entirely and Up/Down, J/K,
                                // Enter, and the scene digits stop arriving.
                                window.focus(&this.focus_handle, cx);
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
        let projection = self.planning_projection(PlanningViewKind::Table);
        let rows = projection.rows;
        v_flex()
            .id("omega-dogfood-work-table")
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
                    .debug_selector({
                        let issue_id = row.issue_id.clone();
                        move || work_row_debug_selector("table", &issue_id)
                    })
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
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&this.focus_handle, cx);
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
        let projection = self.planning_projection(PlanningViewKind::Timeline);
        let source_revision = projection.source_revision;
        let event_cursor = projection.event_cursor;
        let rows = projection.rows;
        let groups = projection.groups;
        v_flex()
            .id("omega-dogfood-work-timeline")
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
                    .id(format!("omega-dogfood-timeline-group-{group}"))
                    .gap_2()
                    .role(gpui::Role::List)
                    .aria_label(format!("{group} timeline group"))
                    .child(section_heading(&group, cx))
                    .children(group_rows.map(|row| {
                        let issue_id = row.issue_id.clone();
                        let selected = row.issue_id == self.selected_issue_id;
                        h_flex()
                            .id(format!("timeline-{}", row.issue_id))
                            .debug_selector({
                                let issue_id = row.issue_id.clone();
                                move || work_row_debug_selector("timeline", &issue_id)
                            })
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
                            .on_click(cx.listener(move |this, _, window, cx| {
                                // A pointer activation must leave the planning
                                // surface holding the keyboard. Without this the
                                // window drops focus entirely and Up/Down, J/K,
                                // Enter, and the scene digits stop arriving.
                                window.focus(&this.focus_handle, cx);
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
        let projection = self.planning_projection(PlanningViewKind::Roadmap);
        let rows = projection.rows;
        let groups = projection.groups;
        h_flex()
            .id("omega-dogfood-work-roadmap")
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
                    .id(format!("omega-dogfood-roadmap-group-{group}"))
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
                            .debug_selector({
                                let issue_id = row.issue_id.clone();
                                move || work_row_debug_selector("roadmap", &issue_id)
                            })
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
                            .on_click(cx.listener(move |this, _, window, cx| {
                                // A pointer activation must leave the planning
                                // surface holding the keyboard. Without this the
                                // window drops focus entirely and Up/Down, J/K,
                                // Enter, and the scene digits stop arriving.
                                window.focus(&this.focus_handle, cx);
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
        let latest_session_projection = command_snapshot
            .and_then(|snapshot| snapshot.session_projections.as_ref())
            .and_then(|sessions| sessions.last());
        let latest_activity_projection = command_snapshot
            .and_then(|snapshot| snapshot.agent_activity_projections.as_ref())
            .and_then(|activities| activities.last());
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
        let session_state = latest_session_projection.map_or("Not projected", |session| {
            work_session_state_label(&session.state)
        });
        let session_generation = latest_session_projection
            .map_or("Not projected".into(), |session| {
                session.generation.0.to_string()
            });
        let session_grant = latest_session_projection.map_or("Not projected".into(), |session| {
            session.delegation_grant_ref.0.clone()
        });
        let session_host = latest_session_projection
            .map_or("Not projected".into(), |session| session.host_ref.0.clone());
        let activity = latest_activity_projection
            .map_or("None".into(), |activity| activity.activity_ref.0.clone());
        let activity_kind = latest_activity_projection.map_or("Not projected", |activity| {
            work_activity_kind_label(&activity.kind)
        });
        let activity_generation = latest_activity_projection
            .map_or("Not projected".into(), |activity| {
                activity.generation.0.to_string()
            });
        let activity_summary = latest_activity_projection
            .map_or("Not projected".into(), |activity| {
                activity.summary.0.clone()
            });
        let canonical_provider_event = latest_activity_projection
            .and_then(|activity| activity.provider_event_ref.0.as_ref())
            .map_or("None".into(), |provider_event| provider_event.0.clone());
        let activity_losses = latest_activity_projection.map_or("None".into(), |activity| {
            if activity.loss_refs.is_empty() {
                "None".into()
            } else {
                activity
                    .loss_refs
                    .iter()
                    .map(|loss| loss.0.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        });
        let activity_effect = latest_activity_projection
            .and_then(|activity| activity.effect_ref.0.as_ref())
            .map_or("None".into(), |effect| effect.0.clone());
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
                    .id("omega-dogfood-work-detail")
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
                            .id("omega-dogfood-work-dependencies")
                            .gap_1()
                            .role(gpui::Role::List)
                            .aria_label("Work dependencies")
                            .when(blockers.is_empty(), |list| {
                                list.child(
                                    div()
                                        .id("omega-dogfood-work-dependencies-empty")
                                        .role(gpui::Role::ListItem)
                                        .child(
                                            Label::new("No typed blockers in this snapshot.")
                                                .size(LabelSize::Small)
                                                .color(Color::Muted),
                                        ),
                                )
                            })
                            .children(blockers.iter().map(|blocker| {
                                div()
                                    .id(format!(
                                        "omega-dogfood-work-dependency-{}",
                                        blocker.id
                                    ))
                                    .role(gpui::Role::ListItem)
                                    .child(
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
                            .id("omega-dogfood-work-labels")
                            .gap_1()
                            .flex_wrap()
                            .role(gpui::Role::List)
                            .aria_label("Work labels")
                            .children(issue.label_ids.iter().map(|label_id| {
                                div()
                                    .id(format!("omega-dogfood-work-label-{label_id}"))
                                    .role(gpui::Role::ListItem)
                                    .child(
                                        Label::new(
                                            label_id.trim_start_matches("label:").to_string(),
                                        )
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                    )
                            })),
                    )
                    .child(section_heading("Execution", cx))
                    .child(
                        Label::new(format!(
                            "{assignee} · {delegate} · {session} · {session_state}"
                        ))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(
                v_flex()
                    .id("omega-dogfood-work-inspector")
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
                    .child(inspector_row("Session state", session_state.into(), cx))
                    .child(inspector_row("Session generation", session_generation, cx))
                    .child(inspector_row("Delegation Grant", session_grant, cx))
                    .child(inspector_row("Session Host", session_host, cx))
                    .child(inspector_row("Agent Session", agent_session, cx))
                    .child(inspector_row("Run", run, cx))
                    .child(inspector_row("Agent Activity", activity, cx))
                    .child(inspector_row("Activity kind", activity_kind.into(), cx))
                    .child(inspector_row("Activity generation", activity_generation, cx))
                    .child(inspector_row("Portable summary", activity_summary, cx))
                    .child(inspector_row("Canonical provider event", canonical_provider_event, cx))
                    .child(inspector_row("Projection loss", activity_losses, cx))
                    .child(inspector_row("Effect", activity_effect, cx))
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
                        "Observed provider candidate",
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
        let causal_parent_refs = activities
            .iter()
            .filter(|activity| activity.workroom_ref.0 == DOGFOOD_SIGNED_WORKROOM_REF)
            .max_by_key(|activity| activity.revision.0)
            .map(|activity| vec![activity.event_ref.0.clone()])
            .unwrap_or_default();
        let checkpoint_work_ref = work_ref.clone();
        let checkpoint_busy = self.signed_workroom_checkpoint_in_flight;
        let any_signed_workroom_operation =
            checkpoint_busy || self.signed_workroom_publish_in_flight.is_some();
        v_flex()
            .id("omega-dogfood-signed-work-history")
            .gap_3()
            .role(gpui::Role::List)
            .aria_label("Signed Work history")
            .child(
                h_flex()
                    .justify_between()
                    .child(section_heading("Signed Work history", cx))
                    .when_some(checkpoint_work_ref, |header, work_ref| {
                        header.child(
                            Button::new(
                                "signed-workroom-checkpoint",
                                if checkpoint_busy {
                                    "Signing…"
                                } else {
                                    "Sign checkpoint"
                                },
                            )
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .disabled(any_signed_workroom_operation)
                            .aria_description(
                                "Prepare an exact Workroom checkpoint, sign it with the selected enrolled Omega identity, and commit it to the durable outbox before relay publication",
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.request_signed_workroom_checkpoint(
                                    work_ref.clone(),
                                    causal_parent_refs.clone(),
                                    cx,
                                )
                            })),
                        )
                    }),
            )
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
                    .id(format!("omega-dogfood-signed-work-{}", activity.event_ref.0))
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
                                    .id(format!(
                                        "omega-dogfood-relay-attempt-{}-{}",
                                        attempt.relay_url.0, attempt.attempted_at.0
                                    ))
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
                                .id(format!(
                                    "omega-dogfood-relay-attempts-{}",
                                    activity.event_ref.0
                                ))
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
                                                    format!(
                                                        "signed-workroom-publish-{event_ref}"
                                                    ),
                                                    if publish_in_flight {
                                                        "Publishing…"
                                                    } else {
                                                        label
                                                    },
                                                )
                                                .style(ButtonStyle::Subtle)
                                                .size(ButtonSize::Compact)
                                                .disabled(any_signed_workroom_operation)
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

fn work_session_state_label(state: &WorkSessionState) -> &'static str {
    match state {
        WorkSessionState::Active => "Active",
        WorkSessionState::Paused => "Paused",
        WorkSessionState::Stopped => "Stopped",
        WorkSessionState::Revoked => "Revoked",
    }
}

fn planning_attention_facts(
    session_state: Option<&WorkSessionState>,
    activity_kind: Option<&WorkCommandActivityKind>,
) -> std::collections::BTreeSet<PlanningAttentionKind> {
    let mut facts = std::collections::BTreeSet::new();
    if session_state == Some(&WorkSessionState::Active) {
        facts.insert(PlanningAttentionKind::AgentActive);
    }
    if session_state
        .is_some_and(|state| matches!(state, WorkSessionState::Active | WorkSessionState::Paused))
        && activity_kind == Some(&WorkCommandActivityKind::Question)
    {
        facts.insert(PlanningAttentionKind::NeedsOwner);
    }
    facts
}

#[cfg(test)]
mod planning_attention_tests {
    use super::*;

    #[test]
    fn exact_session_and_activity_states_supply_only_supported_attention() {
        let active_question = planning_attention_facts(
            Some(&WorkSessionState::Active),
            Some(&WorkCommandActivityKind::Question),
        );
        assert_eq!(
            active_question,
            std::collections::BTreeSet::from([
                PlanningAttentionKind::AgentActive,
                PlanningAttentionKind::NeedsOwner,
            ])
        );

        let paused_question = planning_attention_facts(
            Some(&WorkSessionState::Paused),
            Some(&WorkCommandActivityKind::Question),
        );
        assert_eq!(
            paused_question,
            std::collections::BTreeSet::from([PlanningAttentionKind::NeedsOwner])
        );
        assert!(
            planning_attention_facts(
                Some(&WorkSessionState::Revoked),
                Some(&WorkCommandActivityKind::Question),
            )
            .is_empty()
        );
        assert!(
            planning_attention_facts(
                Some(&WorkSessionState::Active),
                Some(&WorkCommandActivityKind::Progress),
            )
            .contains(&PlanningAttentionKind::AgentActive)
        );
    }
}

fn work_activity_kind_label(kind: &WorkCommandActivityKind) -> &'static str {
    match kind {
        WorkCommandActivityKind::Plan => "Plan",
        WorkCommandActivityKind::Progress => "Progress",
        WorkCommandActivityKind::Question => "Question",
        WorkCommandActivityKind::Action => "Action",
        WorkCommandActivityKind::Artifact => "Artifact",
        WorkCommandActivityKind::Error => "Error",
        WorkCommandActivityKind::Interruption => "Interruption",
        WorkCommandActivityKind::Result => "Result",
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

/// Test-only stable selectors so a simulated pointer click can reach exactly the
/// element a keyboard user reaches, without depending on rendered geometry.
fn work_row_debug_selector(renderer: &str, issue_id: &str) -> String {
    format!("omega.dogfood.row.{renderer}.{issue_id}")
}

fn scene_debug_selector(scene: DogfoodScene) -> String {
    format!("omega.dogfood.scene.{}", scene.label())
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};
    use omega_work_index::{DogfoodPlanningRefreshError, DogfoodPlanningSourceState};

    struct DogfoodTestWindow(Entity<DogfoodSurface>);

    impl Render for DogfoodTestWindow {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            self.0.clone()
        }
    }

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
    fn execution_projection_labels_keep_lifecycle_and_activity_distinct() {
        assert_eq!(
            work_session_state_label(&WorkSessionState::Active),
            "Active"
        );
        assert_eq!(
            work_session_state_label(&WorkSessionState::Revoked),
            "Revoked"
        );
        assert_eq!(
            work_activity_kind_label(&WorkCommandActivityKind::Interruption),
            "Interruption"
        );
        assert_eq!(
            work_activity_kind_label(&WorkCommandActivityKind::Result),
            "Result"
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

    #[gpui::test]
    async fn every_planning_scene_publishes_stable_named_accessibility_regions(
        cx: &mut TestAppContext,
    ) {
        crate::test_support::init_test(cx);
        let fixture = DogfoodPlanningViewModel::from_fixture(
            DogfoodFixtureAdapter::load_for_tests().expect("valid dogfood fixture"),
        );
        let window = cx.add_window(|window, cx| {
            DogfoodTestWindow(cx.new(|cx| DogfoodSurface::new(fixture, window, cx)))
        });
        let surface = window
            .read_with(cx, |window, _cx| window.0.clone())
            .expect("dogfood test window");
        let mut cx = VisualTestContext::from_window(window.clone().into(), cx);
        cx.set_debug_accessibility_active(true);

        for (scene, required_name) in [
            (DogfoodScene::List, "Work list"),
            (DogfoodScene::Board, "Work board"),
            (DogfoodScene::Table, "Work table"),
            (DogfoodScene::Timeline, "Work timeline"),
            (DogfoodScene::Roadmap, "Work roadmap"),
            (DogfoodScene::Issue, "Work detail"),
        ] {
            surface.update(&mut cx, |surface, cx| {
                surface.scene = scene;
                cx.notify();
            });
            cx.run_until_parked();
            let tree = cx
                .debug_render_snapshot()
                .accessibility_tree_json()
                .expect("accessibility tree must be active")
                .to_string();
            assert!(
                tree.contains(required_name),
                "{scene:?} did not publish {required_name:?}: {tree}"
            );
            for forbidden in ["nsec1", "ncryptsec1", "/Users/", "BEGIN PRIVATE KEY"] {
                assert!(
                    !tree.contains(forbidden),
                    "{scene:?} leaked {forbidden:?} into the accessibility tree"
                );
            }
        }
    }

    const RENDERER_SCENES: [(DogfoodScene, &str); 5] = [
        (DogfoodScene::List, "list"),
        (DogfoodScene::Board, "board"),
        (DogfoodScene::Table, "table"),
        (DogfoodScene::Timeline, "timeline"),
        (DogfoodScene::Roadmap, "roadmap"),
    ];

    const SCENE_DIGIT_BINDINGS: [(&str, DogfoodScene); 8] = [
        ("1", DogfoodScene::Overview),
        ("2", DogfoodScene::List),
        ("3", DogfoodScene::Board),
        ("4", DogfoodScene::Table),
        ("5", DogfoodScene::Timeline),
        ("6", DogfoodScene::Roadmap),
        ("7", DogfoodScene::Session),
        ("8", DogfoodScene::Review),
    ];

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct SurfaceOutcome {
        project_id: String,
        selected_issue_id: String,
        scene: DogfoodScene,
    }

    fn load_dogfood_fixture() -> DogfoodPlanningViewModel {
        DogfoodPlanningViewModel::from_fixture(
            DogfoodFixtureAdapter::load_for_tests().expect("valid dogfood fixture"),
        )
    }

    fn open_dogfood_surface(
        cx: &mut TestAppContext,
        fixture: DogfoodPlanningViewModel,
    ) -> (Entity<DogfoodSurface>, VisualTestContext) {
        let window = cx.add_window(|window, cx| {
            DogfoodTestWindow(cx.new(|cx| DogfoodSurface::new(fixture, window, cx)))
        });
        let surface = window
            .read_with(cx, |window, _cx| window.0.clone())
            .expect("dogfood test window");
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        surface.update_in(&mut visual, |surface, window, cx| {
            window.focus(&surface.focus_handle, cx);
            cx.notify();
        });
        visual.run_until_parked();
        (surface, visual)
    }

    fn surface_outcome(
        surface: &Entity<DogfoodSurface>,
        cx: &mut VisualTestContext,
    ) -> SurfaceOutcome {
        surface.update(cx, |surface, _cx| SurfaceOutcome {
            project_id: surface.project_id.clone(),
            selected_issue_id: surface.selected_issue_id.clone(),
            scene: surface.scene,
        })
    }

    fn project_issue_ids(
        surface: &Entity<DogfoodSurface>,
        cx: &mut VisualTestContext,
    ) -> Vec<String> {
        surface.update(cx, |surface, _cx| {
            surface
                .project_issues()
                .into_iter()
                .map(|issue| issue.id.clone())
                .collect()
        })
    }

    fn reset_surface(
        surface: &Entity<DogfoodSurface>,
        cx: &mut VisualTestContext,
        scene: DogfoodScene,
        issue_id: &str,
    ) {
        surface.update(cx, |surface, cx| {
            surface.scene = scene;
            surface.selected_issue_id = issue_id.to_string();
            cx.notify();
        });
        cx.run_until_parked();
    }

    /// The last Work row that a pointer can actually reach in the rendered
    /// frame, paired with its keyboard distance from the first Work row. Rows
    /// past the fold are excluded so a pointer failure means a wiring defect,
    /// not a scrolled-out target.
    fn reachable_row_target(
        cx: &mut VisualTestContext,
        renderer: &str,
        ordered_issue_ids: &[String],
    ) -> Option<(usize, String)> {
        let rendered = cx.debug_render_snapshot();
        ordered_issue_ids
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, issue_id)| {
                let occurrences =
                    rendered.occurrences(&work_row_debug_selector(renderer, issue_id));
                occurrences.len() == 1
                    && occurrences[0].hit_testable
                    && matches!(
                        occurrences[0].visibility,
                        gpui::DebugVisibility::Visible | gpui::DebugVisibility::PartiallyClipped
                    )
            })
            .map(|(index, issue_id)| (index, issue_id.clone()))
            .last()
    }

    #[gpui::test]
    async fn keyboard_and_pointer_reach_the_same_planning_work_in_every_renderer(
        cx: &mut TestAppContext,
    ) {
        crate::test_support::init_test(cx);
        let (surface, mut cx) = open_dogfood_surface(cx, load_dogfood_fixture());

        let ordered_issue_ids = project_issue_ids(&surface, &mut cx);
        assert!(
            ordered_issue_ids.len() > 2,
            "the development Project must carry enough Work to navigate"
        );
        let first_issue_id = ordered_issue_ids[0].clone();

        for (scene, renderer) in RENDERER_SCENES {
            reset_surface(&surface, &mut cx, scene, &first_issue_id);
            let (target_index, target_issue_id) =
                reachable_row_target(&mut cx, renderer, &ordered_issue_ids).unwrap_or_else(|| {
                    panic!("{scene:?} rendered no pointer-reachable Work row after the first")
                });

            reset_surface(&surface, &mut cx, scene, &first_issue_id);
            cx.simulate_click_selector(&work_row_debug_selector(renderer, &target_issue_id))
                .unwrap_or_else(|error| panic!("{scene:?} Work row was not clickable: {error}"));
            cx.run_until_parked();
            let pointer = surface_outcome(&surface, &mut cx);

            reset_surface(&surface, &mut cx, scene, &first_issue_id);
            for _ in 0..target_index {
                cx.simulate_keystrokes("down");
            }
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            let keyboard = surface_outcome(&surface, &mut cx);

            assert_eq!(
                keyboard, pointer,
                "{scene:?}: the keyboard and the pointer reached different Work"
            );
            assert_eq!(
                keyboard,
                SurfaceOutcome {
                    project_id: DOGFOOD_PROJECT_ID.into(),
                    selected_issue_id: target_issue_id.clone(),
                    scene: DogfoodScene::Issue,
                },
                "{scene:?}: neither input opened the intended Work detail"
            );

            reset_surface(&surface, &mut cx, scene, &first_issue_id);
            for _ in 0..target_index {
                cx.simulate_keystrokes("j");
            }
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            assert_eq!(
                surface_outcome(&surface, &mut cx),
                keyboard,
                "{scene:?}: j did not reach the same Work as down"
            );

            for _ in 0..target_index {
                cx.simulate_keystrokes("k");
            }
            cx.run_until_parked();
            assert_eq!(
                surface_outcome(&surface, &mut cx).selected_issue_id,
                first_issue_id,
                "{scene:?}: k did not walk the selection back"
            );

            reset_surface(&surface, &mut cx, scene, &first_issue_id);
            for _ in 0..target_index {
                cx.simulate_keystrokes("down");
            }
            for _ in 0..target_index {
                cx.simulate_keystrokes("up");
            }
            cx.run_until_parked();
            assert_eq!(
                surface_outcome(&surface, &mut cx).selected_issue_id,
                first_issue_id,
                "{scene:?}: up did not walk the selection back"
            );
        }
    }

    #[gpui::test]
    async fn planning_scene_digits_and_scene_tabs_reach_the_same_scene(cx: &mut TestAppContext) {
        crate::test_support::init_test(cx);
        let (surface, mut cx) = open_dogfood_surface(cx, load_dogfood_fixture());
        let selected_issue_id =
            surface.update(&mut cx, |surface, _cx| surface.selected_issue_id.clone());

        for (key, scene) in SCENE_DIGIT_BINDINGS {
            reset_surface(&surface, &mut cx, DogfoodScene::Issue, &selected_issue_id);
            cx.simulate_keystrokes(key);
            cx.run_until_parked();
            let keyboard = surface.update(&mut cx, |surface, _cx| surface.scene);
            assert_eq!(keyboard, scene, "{key:?} did not open the {scene:?} scene");

            reset_surface(&surface, &mut cx, DogfoodScene::Issue, &selected_issue_id);
            cx.simulate_click_selector(&scene_debug_selector(scene))
                .unwrap_or_else(|error| panic!("{scene:?} tab was not clickable: {error}"));
            cx.run_until_parked();
            let pointer = surface.update(&mut cx, |surface, _cx| surface.scene);
            assert_eq!(
                pointer, keyboard,
                "{scene:?}: the scene tab and {key:?} disagreed"
            );
        }

        reset_surface(&surface, &mut cx, DogfoodScene::Issue, &selected_issue_id);
        cx.simulate_keystrokes("cmd-3");
        cx.run_until_parked();
        assert_eq!(
            surface.update(&mut cx, |surface, _cx| surface.scene),
            DogfoodScene::Issue,
            "a modified keystroke must stay with the application, not switch the planning scene"
        );
    }

    #[gpui::test]
    async fn every_degraded_planning_state_stays_operable_by_keyboard_and_pointer(
        cx: &mut TestAppContext,
    ) {
        crate::test_support::init_test(cx);
        let baseline = load_dogfood_fixture();
        let mut without_work = baseline.clone();
        without_work.graph.issues.clear();
        without_work.graph.issue_relations.clear();

        let conflict_error = DogfoodPlanningRefreshError::RevisionRegression {
            previous: 9,
            incoming: 8,
        }
        .to_string();
        let cases: Vec<(
            &str,
            DogfoodPlanningViewModel,
            DogfoodPlanningSourceState,
            bool,
            bool,
        )> = vec![
            (
                "loading",
                baseline.clone(),
                DogfoodPlanningSourceState::Fixture,
                true,
                true,
            ),
            (
                "offline",
                baseline.clone().restored_offline(),
                DogfoodPlanningSourceState::Offline,
                false,
                true,
            ),
            (
                "stale",
                baseline.retain_after_failure(DogfoodPlanningSourceState::Stale, "source is stale"),
                DogfoodPlanningSourceState::Stale,
                false,
                true,
            ),
            (
                "partial",
                baseline.retain_after_incomplete(
                    DogfoodPlanningSourceState::Partial,
                    Vec::new(),
                    "a refresh page did not arrive",
                ),
                DogfoodPlanningSourceState::Partial,
                false,
                true,
            ),
            (
                "gap",
                baseline.retain_after_failure(DogfoodPlanningSourceState::Gap, "cursor gap"),
                DogfoodPlanningSourceState::Gap,
                false,
                true,
            ),
            (
                "error",
                baseline.retain_after_failure(DogfoodPlanningSourceState::Error, "refresh failed"),
                DogfoodPlanningSourceState::Error,
                false,
                true,
            ),
            (
                "conflict",
                baseline.retain_after_failure(
                    DogfoodPlanningSourceState::Error,
                    conflict_error.clone(),
                ),
                DogfoodPlanningSourceState::Error,
                false,
                true,
            ),
            (
                "empty",
                without_work,
                DogfoodPlanningSourceState::Fixture,
                false,
                false,
            ),
        ];

        let (surface, mut cx) = open_dogfood_surface(cx, baseline.clone());

        for (label, model, expected_state, busy, expects_work) in cases {
            let expected_error = model.last_error.clone();
            surface.update(&mut cx, |surface, cx| {
                surface.set_planning_view(model, cx);
                surface.set_repository_claim_state(
                    None,
                    busy.then(|| "claim refresh in flight".to_string()),
                    busy,
                    cx,
                );
                surface.set_work_command_state(
                    None,
                    busy.then(|| "command in flight".to_string()),
                    busy,
                    cx,
                );
            });
            cx.run_until_parked();

            surface.update(&mut cx, |surface, _cx| {
                assert_eq!(
                    surface.fixture.source_state, expected_state,
                    "{label}: the surface did not adopt the degraded planning state"
                );
                assert_eq!(
                    surface.fixture.last_error, expected_error,
                    "{label}: the surface dropped the degraded planning loss fact"
                );
                assert_eq!(
                    surface.repository_claim_busy, busy,
                    "{label}: the surface did not adopt the in-flight claim state"
                );
            });

            for (key, scene) in [("2", DogfoodScene::List), ("6", DogfoodScene::Roadmap)] {
                surface.update(&mut cx, |surface, cx| {
                    surface.scene = DogfoodScene::Issue;
                    cx.notify();
                });
                cx.run_until_parked();
                cx.simulate_keystrokes(key);
                cx.run_until_parked();
                assert_eq!(
                    surface.update(&mut cx, |surface, _cx| surface.scene),
                    scene,
                    "{label}: {key:?} stopped opening the {scene:?} scene"
                );
            }

            surface.update(&mut cx, |surface, cx| {
                surface.scene = DogfoodScene::Issue;
                cx.notify();
            });
            cx.run_until_parked();
            cx.simulate_click_selector(&scene_debug_selector(DogfoodScene::Table))
                .unwrap_or_else(|error| {
                    panic!("{label}: the Table tab was not clickable: {error}")
                });
            cx.run_until_parked();
            assert_eq!(
                surface.update(&mut cx, |surface, _cx| surface.scene),
                DogfoodScene::Table,
                "{label}: the scene tab stopped switching scenes"
            );

            let ordered_issue_ids = project_issue_ids(&surface, &mut cx);
            assert_eq!(
                !ordered_issue_ids.is_empty(),
                expects_work,
                "{label}: the degraded case did not carry the Work it declares"
            );
            if !expects_work {
                reset_surface(&surface, &mut cx, DogfoodScene::List, "");
                let rendered = cx.debug_render_snapshot();
                assert!(
                    rendered
                        .selectors()
                        .all(|(selector, _)| !selector.starts_with("omega.dogfood.row.")),
                    "{label}: a Work row rendered from a graph that carries no Work"
                );
                continue;
            }

            let first_issue_id = ordered_issue_ids[0].clone();
            reset_surface(&surface, &mut cx, DogfoodScene::List, &first_issue_id);
            let (target_index, target_issue_id) =
                reachable_row_target(&mut cx, "list", &ordered_issue_ids).unwrap_or_else(|| {
                    panic!("{label}: the Work list rendered no pointer-reachable row")
                });

            reset_surface(&surface, &mut cx, DogfoodScene::List, &first_issue_id);
            cx.simulate_click_selector(&work_row_debug_selector("list", &target_issue_id))
                .unwrap_or_else(|error| panic!("{label}: the Work row was not clickable: {error}"));
            cx.run_until_parked();
            let pointer = surface_outcome(&surface, &mut cx);

            reset_surface(&surface, &mut cx, DogfoodScene::List, &first_issue_id);
            for _ in 0..target_index {
                cx.simulate_keystrokes("down");
            }
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            let keyboard = surface_outcome(&surface, &mut cx);

            assert_eq!(
                keyboard, pointer,
                "{label}: the keyboard and the pointer reached different Work"
            );
            assert_eq!(
                keyboard,
                SurfaceOutcome {
                    project_id: DOGFOOD_PROJECT_ID.into(),
                    selected_issue_id: target_issue_id,
                    scene: DogfoodScene::Issue,
                },
                "{label}: a degraded planning state blocked Work selection"
            );
        }
    }

    #[derive(Debug, Eq, PartialEq)]
    struct RestorablePlanningState {
        project_id: String,
        selected_issue_id: String,
        scene: DogfoodScene,
        saved_view: PlanningSavedView,
        filter: PlanningFilter,
        group: PlanningGroup,
        sort: PlanningSort,
        user_saved_views: Vec<NamedSavedPlanningView>,
        active_user_saved_view: bool,
        selected_user_saved_view_id: Option<String>,
        query: PlanningViewQuery,
        rows_by_renderer: Vec<(String, Vec<String>)>,
        rendered_rows_by_renderer: Vec<(String, Vec<String>)>,
        searched_rows: Vec<(String, Vec<String>)>,
        source_revision: u64,
        event_cursor: String,
    }

    fn capture_restorable_state(
        surface: &Entity<DogfoodSurface>,
        cx: &mut VisualTestContext,
    ) -> RestorablePlanningState {
        let restored_scene = surface.update(cx, |surface, _cx| surface.scene);
        let mut rendered_rows_by_renderer = Vec::new();
        for (scene, renderer) in RENDERER_SCENES {
            surface.update(cx, |surface, cx| {
                surface.scene = scene;
                cx.notify();
            });
            cx.run_until_parked();
            let rendered = cx.debug_render_snapshot();
            let prefix = format!("omega.dogfood.row.{renderer}.");
            rendered_rows_by_renderer.push((
                renderer.to_string(),
                rendered
                    .selectors()
                    .filter_map(|(selector, _)| {
                        selector.strip_prefix(prefix.as_str()).map(str::to_owned)
                    })
                    .collect(),
            ));
        }

        surface.update(cx, |surface, cx| {
            surface.scene = restored_scene;
            cx.notify();
            let projection = surface.planning_projection(PlanningViewKind::List);
            let mut rows_by_renderer = Vec::new();
            let mut searched_rows = Vec::new();
            for (_, renderer) in RENDERER_SCENES {
                let kind = match renderer {
                    "list" => PlanningViewKind::List,
                    "board" => PlanningViewKind::Board,
                    "table" => PlanningViewKind::Table,
                    "timeline" => PlanningViewKind::Timeline,
                    _ => PlanningViewKind::Roadmap,
                };
                rows_by_renderer.push((
                    renderer.to_string(),
                    surface
                        .visible_issues(kind)
                        .into_iter()
                        .map(|issue| issue.id.clone())
                        .collect(),
                ));
            }
            // The development surface exposes no search input, so the persisted
            // query always restores an empty `search`. Applying a term to the
            // restored query is the only search state this surface can own.
            for term in ["dogfood", "release", "omega"] {
                let mut query = surface.planning_query();
                query.search = term.into();
                searched_rows.push((
                    term.to_string(),
                    omega_work_index::project_planning_view(
                        &surface.fixture,
                        PlanningViewKind::List,
                        &query,
                    )
                    .rows
                    .into_iter()
                    .map(|row| row.issue_id)
                    .collect(),
                ));
            }
            RestorablePlanningState {
                project_id: surface.project_id.clone(),
                selected_issue_id: surface.selected_issue_id.clone(),
                scene: restored_scene,
                saved_view: surface.saved_view,
                filter: surface.filter,
                group: surface.group,
                sort: surface.sort,
                user_saved_views: surface.user_saved_views.views.clone(),
                active_user_saved_view: surface.user_saved_views.active,
                selected_user_saved_view_id: surface.user_saved_views.selected_id.clone(),
                query: surface.planning_query(),
                rows_by_renderer,
                rendered_rows_by_renderer,
                searched_rows,
                source_revision: projection.source_revision,
                event_cursor: projection.event_cursor,
            }
        })
    }

    #[gpui::test]
    async fn planning_filter_group_sort_and_saved_views_restore_identically_across_restart(
        cx: &mut TestAppContext,
    ) {
        crate::test_support::init_test(cx);
        let (surface, mut first_run) = open_dogfood_surface(cx, load_dogfood_fixture());

        surface.update(&mut first_run, |surface, cx| {
            surface.set_saved_view(PlanningSavedView::Blocked, cx);
            surface.set_filter(PlanningFilter::Open, cx);
            surface.cycle_group(cx);
            surface.cycle_group(cx);
            surface.cycle_sort(cx);
        });
        first_run.run_until_parked();
        surface.update_in(&mut first_run, |surface, window, cx| {
            surface.view_name_editor.update(cx, |editor, cx| {
                editor.set_text("Blocked triage", window, cx)
            });
            surface.create_user_view(cx);
        });
        first_run.run_until_parked();
        surface.update_in(&mut first_run, |surface, window, cx| {
            surface.view_name_editor.update(cx, |editor, cx| {
                editor.set_text("Release watch", window, cx)
            });
            surface.create_user_view(cx);
        });
        first_run.run_until_parked();

        let second_issue_id = project_issue_ids(&surface, &mut first_run)
            .get(1)
            .cloned()
            .expect("the development Project must carry more than one Work item");
        surface.update(&mut first_run, |surface, cx| {
            surface.select_issue(second_issue_id.clone(), true, cx);
        });
        first_run.run_until_parked();

        let before = capture_restorable_state(&surface, &mut first_run);
        // Exact values, not merely "different from the default": a surface that
        // restores something is not the same as a surface that restores what was
        // saved.
        assert_eq!(before.saved_view, PlanningSavedView::Blocked);
        assert_eq!(before.filter, PlanningFilter::Open);
        assert_eq!(before.group, PlanningGroup::Project);
        assert_eq!(before.sort, PlanningSort::Priority);
        // The query the renderers actually consume must mirror that state. A
        // query that ignores it stays deterministic across a restart while
        // showing the wrong Work in both runs.
        assert_eq!(before.query.saved_view, before.saved_view);
        assert_eq!(before.query.filter, before.filter);
        assert_eq!(before.query.group, before.group);
        assert_eq!(before.query.sort, before.sort);
        assert_eq!(before.query.project_id, before.project_id);
        assert_eq!(
            before.user_saved_views.len(),
            2,
            "both local Views must exist before the restart"
        );
        assert!(
            before
                .rows_by_renderer
                .iter()
                .all(|(_, rows)| !rows.is_empty()),
            "the pre-restart query must still project Work in every renderer"
        );
        drop(first_run);

        let (restored, mut second_run) = open_dogfood_surface(cx, load_dogfood_fixture());
        let after = capture_restorable_state(&restored, &mut second_run);

        assert_eq!(
            after, before,
            "the planning query, saved Views, and projected Work did not survive the restart"
        );
        assert_eq!(after.query.sort, PlanningSort::Priority);
        assert_eq!(after.query.group, PlanningGroup::Project);
        assert_eq!(after.query.saved_view, PlanningSavedView::Blocked);
        assert_eq!(after.query.filter, PlanningFilter::Open);
    }

    #[gpui::test]
    async fn pointer_activation_keeps_the_planning_surface_operable_by_keyboard(
        cx: &mut TestAppContext,
    ) {
        crate::test_support::init_test(cx);
        let (surface, mut cx) = open_dogfood_surface(cx, load_dogfood_fixture());
        let ordered_issue_ids = project_issue_ids(&surface, &mut cx);
        let first_issue_id = ordered_issue_ids[0].clone();

        reset_surface(&surface, &mut cx, DogfoodScene::List, &first_issue_id);
        let (target_index, target_issue_id) =
            reachable_row_target(&mut cx, "list", &ordered_issue_ids)
                .expect("the Work list rendered no pointer-reachable row");

        for _ in 0..target_index {
            cx.simulate_keystrokes("down");
        }
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        let keyboard_only = surface_outcome(&surface, &mut cx);
        assert_eq!(keyboard_only.selected_issue_id, target_issue_id);
        assert_eq!(keyboard_only.scene, DogfoodScene::Issue);

        // Activate the same Work with the pointer, then keep driving the surface
        // from the keyboard without restoring focus by hand. A window that drops
        // focus on click silently disables Up/Down, J/K, Enter, and the scene
        // digits for every later keystroke.
        reset_surface(&surface, &mut cx, DogfoodScene::List, &first_issue_id);
        cx.simulate_click_selector(&work_row_debug_selector("list", &target_issue_id))
            .expect("the Work row must be clickable");
        cx.run_until_parked();
        surface.update_in(&mut cx, |surface, window, _cx| {
            assert!(
                surface.focus_handle.is_focused(window),
                "a Work row click released keyboard control of the planning surface"
            );
        });

        cx.simulate_keystrokes("2");
        cx.run_until_parked();
        assert_eq!(
            surface.update(&mut cx, |surface, _cx| surface.scene),
            DogfoodScene::List,
            "the scene digits stopped working after a Work row click"
        );
        surface.update(&mut cx, |surface, cx| {
            surface.selected_issue_id = first_issue_id.clone();
            cx.notify();
        });
        cx.run_until_parked();
        for _ in 0..target_index {
            cx.simulate_keystrokes("down");
        }
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(
            surface_outcome(&surface, &mut cx),
            keyboard_only,
            "the keyboard reached different Work after a pointer activation"
        );

        reset_surface(&surface, &mut cx, DogfoodScene::List, &first_issue_id);
        cx.simulate_click_selector(&scene_debug_selector(DogfoodScene::Board))
            .expect("the Board tab must be clickable");
        cx.run_until_parked();
        surface.update_in(&mut cx, |surface, window, _cx| {
            assert!(
                surface.focus_handle.is_focused(window),
                "a scene tab click released keyboard control of the planning surface"
            );
        });
        cx.simulate_keystrokes("5");
        cx.run_until_parked();
        assert_eq!(
            surface.update(&mut cx, |surface, _cx| surface.scene),
            DogfoodScene::Timeline,
            "the scene digits stopped working after a scene tab click"
        );
    }
}
use db::kvp::KeyValueStore;
