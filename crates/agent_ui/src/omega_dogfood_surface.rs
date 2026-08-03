use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyContext, Render, Styled,
    Window, prelude::*,
};
#[cfg(all(test, feature = "test-support"))]
use omega_work_index::DogfoodFixtureAdapter;
use omega_work_index::{
    DOGFOOD_PROJECT_ID, DogfoodFixtureProjection, FixtureIssue, FixtureIssueRelationKind,
    FixtureLifecycleType, FixturePriority, SECURITY_PROJECT_ID,
};
use serde::{Deserialize, Serialize};
use ui::{Button, ButtonSize, ButtonStyle, Color, Icon, IconName, Label, LabelSize, prelude::*};

const DOGFOOD_SURFACE_STATE_KEY: &str = "omega_dogfood_surface_state_v1";

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum DogfoodScene {
    #[default]
    Overview,
    List,
    Board,
    Issue,
    Session,
    Review,
}

impl DogfoodScene {
    const ALL: [Self; 6] = [
        Self::Overview,
        Self::List,
        Self::Board,
        Self::Issue,
        Self::Session,
        Self::Review,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::List => "List",
            Self::Board => "Board",
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
}

pub struct DogfoodSurface {
    focus_handle: FocusHandle,
    fixture: DogfoodFixtureProjection,
    project_id: String,
    selected_issue_id: String,
    scene: DogfoodScene,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedDogfoodSurfaceState {
    project_id: String,
    selected_issue_id: String,
    scene: DogfoodScene,
}

impl DogfoodSurface {
    pub fn new(fixture: DogfoodFixtureProjection, cx: &mut Context<Self>) -> Self {
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
        }
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn selected_issue_id(&self) -> &str {
        &self.selected_issue_id
    }

    pub fn scene(&self) -> DogfoodScene {
        self.scene
    }

    fn project_issues(&self) -> Vec<&FixtureIssue> {
        self.fixture
            .graph
            .issues
            .iter()
            .filter(|issue| issue.project_id == self.project_id)
            .collect()
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
                                            .role(gpui::Role::Heading)
                                            .aria_level(1)
                                            .text_size(px(22.))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(self.project_name().to_string()),
                                    )
                                    .child(
                                        Label::new("DEV MOCKS")
                                            .size(LabelSize::XSmall)
                                            .color(Color::Warning),
                                    ),
                            )
                            .child(
                                Label::new(format!(
                                    "v0.2.0 issue snapshot · {} · {} open · {}…",
                                    self.fixture.source_snapshot_at, open, digest_prefix
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
                    ),
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
            .child(section_heading("Fixture provenance", cx))
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
                        Label::new("Development mock data · no command or release authority")
                            .size(LabelSize::XSmall)
                            .color(Color::Warning),
                    ),
            )
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        v_flex()
            .rounded_lg()
            .border_1()
            .border_color(colors.border_variant)
            .children(self.project_issues().into_iter().map(|issue| {
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
            }))
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
                let cards = self.project_issues().into_iter().filter(|issue| {
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
                        Label::new("Unassigned · No delegate · No live session")
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
                        format!("work:fixture:{}", issue.id),
                        cx,
                    ))
                    .child(inspector_row("Issue projection", issue.id.clone(), cx))
                    .child(inspector_row("Repository", issue.repository_id.clone(), cx))
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
                    .child(inspector_row("Assignee", "Unassigned".into(), cx))
                    .child(inspector_row("Agent delegate", "None".into(), cx))
                    .child(inspector_row("Session", "None".into(), cx))
                    .child(inspector_row(
                        "Authority",
                        "Simulation · read only".into(),
                        cx,
                    )),
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
                .size(IconSize::Large)
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
                    "4" => this.set_scene(DogfoodScene::Session, cx),
                    "5" => this.set_scene(DogfoodScene::Review, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(self.render_header(cx))
            .child(match self.scene {
                DogfoodScene::Overview => self.render_overview(cx).into_any_element(),
                DogfoodScene::List => self.render_list(cx).into_any_element(),
                DogfoodScene::Board => self.render_board(cx).into_any_element(),
                DogfoodScene::Issue => self.render_issue(cx).into_any_element(),
                DogfoodScene::Session => self.render_empty_execution(false, cx).into_any_element(),
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
    }
}

fn fixture_state_is_valid(
    fixture: &DogfoodFixtureProjection,
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

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;

    #[test]
    fn fresh_fixture_state_opens_the_dogfood_project_on_omega_214() {
        let state = default_fixture_state();
        assert_eq!(state.project_id, DOGFOOD_PROJECT_ID);
        assert_eq!(state.selected_issue_id, "issue:omega:214");
        assert_eq!(state.scene, DogfoodScene::Overview);
    }

    #[test]
    fn persisted_scene_keeps_one_issue_identity_and_rejects_cross_project_state() {
        let fixture = DogfoodFixtureAdapter::load_for_tests().expect("valid fixture");
        for scene in DogfoodScene::ALL {
            let state = PersistedDogfoodSurfaceState {
                project_id: DOGFOOD_PROJECT_ID.into(),
                selected_issue_id: "issue:omega:214".into(),
                scene,
            };
            assert!(fixture_state_is_valid(&fixture, &state));
        }
        let invalid = PersistedDogfoodSurfaceState {
            project_id: SECURITY_PROJECT_ID.into(),
            selected_issue_id: "issue:omega:214".into(),
            scene: DogfoodScene::Issue,
        };
        assert!(!fixture_state_is_valid(&fixture, &invalid));
    }
}
use db::kvp::KeyValueStore;
