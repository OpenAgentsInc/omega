use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyContext, Render, Styled,
    Window, prelude::*,
};
use omega_effectd::all_work_contract::{
    AgentRef, AgentSessionRef, HostRef, OrganizationRef, PrincipalRef, RepositoryClaimLedger,
    RepositoryWorkClaim, RepositoryWorkClaimState, SignedWorkroomLedger, ThreadRef, WorkSnapshot,
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
use ui::{Button, ButtonSize, ButtonStyle, Color, Icon, IconName, Label, LabelSize, prelude::*};

use crate::omega_agent_session_simulation::{AgentSessionSimulation, AgentSessionSimulationScene};

const DOGFOOD_SURFACE_STATE_KEY: &str = "omega_dogfood_surface_state_v1";

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
    user_saved_view: EditableSavedPlanningView,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SavedPlanningQuery {
    saved_view: PlanningSavedView,
    filter: PlanningFilter,
    group: PlanningGroup,
    sort: PlanningSort,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct EditableSavedPlanningView {
    query: Option<SavedPlanningQuery>,
    active: bool,
}

impl EditableSavedPlanningView {
    fn from_persisted(query: Option<SavedPlanningQuery>, active: bool) -> Self {
        Self {
            active: active && query.is_some(),
            query,
        }
    }

    fn save(&mut self, query: SavedPlanningQuery) {
        self.query = Some(query);
        self.active = true;
    }

    fn apply(&mut self) -> Option<SavedPlanningQuery> {
        self.active = self.query.is_some();
        self.query
    }

    fn diverge(&mut self) {
        self.active = false;
    }

    fn remove(&mut self) {
        self.query = None;
        self.active = false;
    }
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
}

impl DogfoodSurface {
    pub fn new(fixture: DogfoodPlanningViewModel, cx: &mut Context<Self>) -> Self {
        let persisted = KeyValueStore::global(cx)
            .read_kvp(DOGFOOD_SURFACE_STATE_KEY)
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str::<PersistedDogfoodSurfaceState>(&json).ok())
            .filter(|state| fixture_state_is_valid(&fixture, state));
        let state = persisted.unwrap_or_else(default_fixture_state);
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
            user_saved_view: EditableSavedPlanningView::from_persisted(
                state.user_saved_view,
                state.user_saved_view_active,
            ),
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
        self.user_saved_view.diverge();
        self.save_state(cx);
        cx.notify();
    }

    fn set_saved_view(&mut self, saved_view: PlanningSavedView, cx: &mut Context<Self>) {
        self.saved_view = saved_view;
        self.user_saved_view.diverge();
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
        self.user_saved_view.diverge();
        self.save_state(cx);
        cx.notify();
    }

    fn cycle_sort(&mut self, cx: &mut Context<Self>) {
        self.sort = match self.sort {
            PlanningSort::SourceOrder => PlanningSort::Priority,
            PlanningSort::Priority => PlanningSort::Title,
            PlanningSort::Title => PlanningSort::SourceOrder,
        };
        self.user_saved_view.diverge();
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

    fn save_user_view(&mut self, cx: &mut Context<Self>) {
        let query = self.current_saved_planning_query();
        self.user_saved_view.save(query);
        self.save_state(cx);
        cx.notify();
    }

    fn apply_user_view(&mut self, cx: &mut Context<Self>) {
        let Some(query) = self.user_saved_view.apply() else {
            return;
        };
        self.saved_view = query.saved_view;
        self.filter = query.filter;
        self.group = query.group;
        self.sort = query.sort;
        self.save_state(cx);
        cx.notify();
    }

    fn remove_user_view(&mut self, cx: &mut Context<Self>) {
        self.user_saved_view.remove();
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
            user_saved_view: self.user_saved_view.query,
            user_saved_view_active: self.user_saved_view.active,
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
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_saved_view(saved_view, cx)
                        }))
                    }))
                    .child(
                        Button::new("planning-user-saved-view", "My view")
                            .style(if self.user_saved_view.active {
                                ButtonStyle::Filled
                            } else {
                                ButtonStyle::Subtle
                            })
                            .size(ButtonSize::Compact)
                            .disabled(self.user_saved_view.query.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.apply_user_view(cx))),
                    )
                    .child(
                        Button::new(
                            "planning-save-user-view",
                            if self.user_saved_view.query.is_some() {
                                "Update"
                            } else {
                                "Save"
                            },
                        )
                        .style(ButtonStyle::Subtle)
                        .size(ButtonSize::Compact)
                        .on_click(cx.listener(|this, _, _, cx| this.save_user_view(cx))),
                    )
                    .child(
                        Button::new("planning-remove-user-view", "Remove")
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .disabled(self.user_saved_view.query.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.remove_user_view(cx))),
                    ),
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
                        Label::new(format!(
                            "{} · cursor {} · {} gap(s) · {} projection issue(s) · no command or release authority",
                            self.fixture.provenance_label(),
                            self.fixture.event_cursor,
                            self.fixture.refresh_gap_refs.len(),
                            self.fixture.refresh_projection_issues.len()
                        ))
                            .size(LabelSize::XSmall)
                            .color(if self.fixture.is_fresh_live() {
                                Color::Success
                            } else {
                                Color::Warning
                            }),
                    )
                    .child(
                        Label::new(
                            self.fixture
                                .last_error
                                .clone()
                                .unwrap_or_else(|| "No refresh loss reported".into()),
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
                            .aria_label(format!("{} {}", issue.identifier, issue.title))
                            .when(selected, |row| row.bg(colors.element_selected))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_issue(issue_id.clone(), true, cx)
                            }))
                            .child(status_icon(issue.completed, blockers > 0))
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
                                    Label::new(format!("Blocked · {blockers}"))
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
                    .child(section_heading(label, cx))
                    .children(cards.map(|issue| {
                        let issue_id = issue.id.clone();
                        let blocked = !self.blocked_by(issue).is_empty();
                        v_flex()
                            .id(format!("board-{}", issue.id))
                            .gap_2()
                            .p_3()
                            .rounded_lg()
                            .border_1()
                            .border_color(if issue.id == self.selected_issue_id {
                                colors.border_selected
                            } else {
                                colors.border_variant
                            })
                            .cursor_pointer()
                            .role(gpui::Role::Button)
                            .tab_index(0isize)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_issue(issue_id.clone(), true, cx)
                            }))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(status_icon(issue.completed, blocked))
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
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_issue(issue_id.clone(), true, cx)
                    }))
                    .child(
                        div()
                            .w(px(84.))
                            .text_size(px(11.))
                            .text_color(colors.text_muted)
                            .child(row.identifier),
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
                    .child(section_heading(&group, cx))
                    .children(group_rows.map(|row| {
                        let issue_id = row.issue_id.clone();
                        h_flex()
                            .id(format!("timeline-{}", row.issue_id))
                            .gap_3()
                            .cursor_pointer()
                            .role(gpui::Role::Button)
                            .tab_index(0isize)
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
                                    format!("Blocked · {} blocker(s)", row.blocked_by_count)
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
                    .child(section_heading(&group, cx))
                    .child(
                        Label::new(format!("{completed}/{} complete", group_rows.len()))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(progress_dots(completed, group_rows.len(), cx))
                    .children(group_rows.into_iter().take(6).map(|row| {
                        let issue_id = row.issue_id.clone();
                        div()
                            .id(format!("roadmap-{}", row.issue_id))
                            .cursor_pointer()
                            .role(gpui::Role::Button)
                            .tab_index(0isize)
                            .text_size(px(11.))
                            .text_color(if row.blocked_by_count > 0 {
                                Color::Warning.color(cx)
                            } else {
                                Color::Muted.color(cx)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_issue(issue_id.clone(), true, cx)
                            }))
                            .child(if row.blocked_by_count > 0 {
                                format!("Blocked · {} · {}", row.identifier, row.title)
                            } else {
                                format!("{} · {}", row.identifier, row.title)
                            })
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
                            .child(status_icon(issue.completed, !blockers.is_empty()))
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
                            .when(blockers.is_empty(), |list| {
                                list.child(
                                    Label::new("No typed blockers in this snapshot.")
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                )
                            })
                            .children(blockers.iter().map(|blocker| {
                                Label::new(format!(
                                    "Blocked by {} · {}",
                                    blocker.identifier, blocker.title
                                ))
                                .size(LabelSize::Small)
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
                            .children(issue.label_ids.iter().map(|label_id| {
                                Label::new(label_id.trim_start_matches("label:").to_string())
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
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
                                            "Record handoff",
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
                    format!(
                        "{} is unassigned and has no simulated or live execution.",
                        issue.identifier
                    )
                }))
                .size(LabelSize::Small)
                .color(Color::Muted),
            )
            .child(
                Label::new("Development mock data · no evidence or owner disposition")
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
            .child(section_heading("Signed Work history", cx))
            .child(
                Label::new(self.signed_workroom_error.clone().unwrap_or_else(|| {
                    "Signature proves signer and bytes; relay acceptance is transport evidence, not command or effect authority.".into()
                }))
                .size(LabelSize::XSmall)
                .color(if self.signed_workroom_error.is_some() { Color::Error } else { Color::Muted }),
            )
            .when(activities.is_empty(), |view| view.child(
                v_flex().min_h(px(220.)).items_center().justify_center().gap_2()
                    .rounded_lg().border_1().border_color(colors.border_variant)
                    .child(Label::new("No signed Workroom activity").size(LabelSize::Large))
                    .child(Label::new("No activity is not an execution, verification, or owner-disposition fact.").size(LabelSize::Small).color(Color::Muted)),
            ))
            .children(activities.into_iter().map(|activity| {
                v_flex().gap_1().p_3().rounded_lg().border_1().border_color(colors.border_variant)
                    .child(h_flex().justify_between()
                        .child(Label::new(format!("{:?}", activity.kind)).size(LabelSize::Small))
                        .child(Label::new(workroom_audience_label(&activity.audience)).size(LabelSize::XSmall).color(Color::Muted)))
                    .child(Label::new(format!("Actor {} · signer {}…", activity.actor_ref.0, &activity.signer_pubkey.0[..12])).size(LabelSize::XSmall))
                    .child(Label::new(format!("{} · generation {} · {} parent(s)", activity.occurred_at.0, activity.generation.0, activity.causal_parent_refs.len())).size(LabelSize::XSmall).color(Color::Muted))
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
                            .child(
                                Label::new("SIMULATED · EPHEMERAL")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Warning),
                            ),
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
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.modifiers.modified() {
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
        .on_click(listener)
}

fn status_icon(completed: bool, blocked: bool) -> Icon {
    if completed {
        Icon::new(IconName::Check)
            .size(IconSize::Small)
            .color(Color::Success)
    } else if blocked {
        Icon::new(IconName::Warning)
            .size(IconSize::Small)
            .color(Color::Warning)
    } else {
        Icon::new(IconName::Circle)
            .size(IconSize::Small)
            .color(Color::Muted)
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
    fn editable_saved_view_captures_reapplies_updates_and_removes_one_exact_query() {
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
        let mut editable = EditableSavedPlanningView::default();

        editable.save(initial);
        assert_eq!(editable.apply(), Some(initial));
        assert!(editable.active);

        editable.diverge();
        assert!(!editable.active);
        editable.save(updated);
        assert_eq!(editable.apply(), Some(updated));

        editable.remove();
        assert_eq!(editable.apply(), None);
        assert!(!editable.active);
    }
}
use db::kvp::KeyValueStore;
