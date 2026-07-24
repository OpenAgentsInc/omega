//! Dedicated Full Auto GPUI panel — launcher + concurrent monitor.
//!
//! Mutations go through `omega_effectd`. There is no ordinary composer here.

use std::time::Duration;

use anyhow::{Result, anyhow};
use editor::Editor;
use gpui::{
    Action, App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Task, WeakEntity,
    Window, div, px,
};
use omega_effectd::{SharedOmegaEffectdSupervisor, shared_supervisor};
use serde_json::{Value, json};
use ui::{Button, ButtonStyle, IconButton, IconName, Label, LabelSize, Tooltip, prelude::*};
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};
use zed_actions::full_auto_panel::{OpenLauncher, ToggleFocus};

use crate::draft::{
    DEFAULT_DONE_CONDITION, DEFAULT_TURN_CAP, FULL_AUTO_ACTIVE_LIMIT, FULL_AUTO_WORKSPACE_REF,
    FullAutoLauncherDraft, validate_launcher_draft,
};

const PANEL_KEY: &str = "FullAutoPanel";
const ACTIVE_STATES: &[&str] = &["running", "pausing", "paused", "retrying", "stalled"];

#[derive(Debug, Clone)]
struct RunRow {
    run_ref: String,
    title: String,
    state: String,
}

#[derive(Debug, Clone)]
struct NativeEvidence {
    project_ref: String,
    worktree_ref: String,
    git_head: Option<String>,
}

#[derive(Debug, Clone)]
struct RunDetail {
    run_ref: String,
    title: String,
    state: String,
    objective: String,
    done_condition: String,
    workspace_ref: Option<String>,
    lane: Option<String>,
    turn_cap: u32,
    successful_attempts: u32,
    failed_attempts: u32,
    stall_cause: Option<String>,
    recovery_action: String,
    objective_digest: Option<String>,
    native_evidence: Option<NativeEvidence>,
    turns: Vec<(String, String, String)>,
}

#[derive(Debug, Clone)]
struct CapacityLane {
    lane: String,
    state: String,
    active_runs: u32,
}

enum SurfaceMode {
    Launcher,
    Run,
}

pub struct FullAutoPanel {
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    mode: SurfaceMode,
    draft: FullAutoLauncherDraft,
    objective_editor: Entity<Editor>,
    title_editor: Entity<Editor>,
    done_editor: Entity<Editor>,
    turn_cap_editor: Entity<Editor>,
    runs: Vec<RunRow>,
    active_run: Option<RunDetail>,
    active_run_ref: Option<String>,
    capacity_lanes: Vec<CapacityLane>,
    status: SharedString,
    supervisor: Option<SharedOmegaEffectdSupervisor>,
    _refresh: Option<Task<()>>,
}

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace
            .register_action(|workspace, _: &ToggleFocus, window, cx| {
                workspace.toggle_panel_focus::<FullAutoPanel>(window, cx);
            })
            .register_action(|workspace, _: &OpenLauncher, window, cx| {
                if let Some(panel) = workspace.panel::<FullAutoPanel>(cx) {
                    panel.update(cx, |panel, cx| panel.open_launcher(cx));
                }
                workspace.focus_panel::<FullAutoPanel>(window, cx);
            });
    })
    .detach();
}

impl FullAutoPanel {
    pub fn load(
        workspace: WeakEntity<Workspace>,
        cx: AsyncWindowContext,
    ) -> Task<Result<Entity<Self>>> {
        cx.spawn(async move |cx| {
            let workspace_for_panel = workspace.clone();
            workspace.update_in(cx, |_workspace, window, cx| {
                Ok(cx.new(|cx| Self::new(workspace_for_panel, window, cx)))
            })?
        })
    }

    fn new(workspace: WeakEntity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let objective_editor = cx.new(|cx| {
            let mut editor = Editor::multi_line(window, cx);
            editor.set_placeholder_text(
                "Implement the outcome, verify it, and keep going until it works.",
                window,
                cx,
            );
            editor
        });
        let title_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("From the first line of the mission", window, cx);
            editor
        });
        let done_editor = cx.new(|cx| {
            let mut editor = Editor::multi_line(window, cx);
            editor.set_placeholder_text(DEFAULT_DONE_CONDITION, window, cx);
            editor
        });
        let turn_cap_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(DEFAULT_TURN_CAP.to_string(), window, cx);
            editor
        });

        let mut panel = Self {
            workspace,
            focus_handle: cx.focus_handle(),
            mode: SurfaceMode::Launcher,
            draft: FullAutoLauncherDraft::default(),
            objective_editor,
            title_editor,
            done_editor,
            turn_cap_editor,
            runs: Vec::new(),
            active_run: None,
            active_run_ref: None,
            capacity_lanes: Vec::new(),
            status: "Full Auto is a run, not a chat option.".into(),
            supervisor: None,
            _refresh: None,
        };
        panel.ensure_supervisor(cx);
        panel.schedule_refresh(cx);
        panel
    }

    fn open_launcher(&mut self, cx: &mut Context<Self>) {
        self.mode = SurfaceMode::Launcher;
        self.active_run_ref = None;
        self.active_run = None;
        self.draft.submitting = false;
        self.draft.error = None;
        cx.notify();
    }

    fn sync_draft_from_editors(&mut self, cx: &App) {
        self.draft.objective = self.objective_editor.read(cx).text(cx);
        self.draft.title = self.title_editor.read(cx).text(cx);
        self.draft.done_condition = self.done_editor.read(cx).text(cx);
        self.draft.turn_cap_text = self.turn_cap_editor.read(cx).text(cx);
        if self.draft.workspace_ref.is_empty() {
            self.draft.workspace_ref = FULL_AUTO_WORKSPACE_REF.to_string();
        }
    }

    fn ensure_supervisor(&mut self, cx: &mut Context<Self>) {
        if self.supervisor.is_some() {
            return;
        }
        let handle = match shared_supervisor(cx) {
            Ok(handle) => handle,
            Err(error) => {
                self.status = format!("omega-effectd unavailable ({error}).").into();
                self.draft.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let start_handle = handle.clone();
        self.supervisor = Some(handle);
        cx.spawn(async move |this, cx| {
            let started = {
                let mut guard = start_handle.lock().await;
                guard.ensure_started().await
            };
            this.update(cx, |this, cx| {
                this.status = match started {
                    Ok(_) => "Connected to omega-effectd.".into(),
                    Err(error) => format!("omega-effectd unavailable ({error}).").into(),
                };
                this.refresh_runs(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn schedule_refresh(&mut self, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        self._refresh = Some(cx.spawn(async move |this, cx| {
            loop {
                executor.timer(Duration::from_secs(3)).await;
                if this.update(cx, |this, cx| this.refresh_runs(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn refresh_runs(&mut self, cx: &mut Context<Self>) {
        let Some(supervisor) = self.supervisor.clone() else {
            return;
        };
        let active = self.active_run_ref.clone();
        cx.spawn(async move |this, cx| {
            let listed = {
                let mut guard = supervisor.lock().await;
                guard.list_runs().await
            };
            let capacity = {
                let mut guard = supervisor.lock().await;
                guard.get_capacity().await.ok()
            };
            let detail = if let Some(run_ref) = active {
                let mut guard = supervisor.lock().await;
                let run = guard.get_run(&run_ref).await.ok();
                let receipt = guard.get_receipt(&run_ref).await.ok();
                (run, receipt)
            } else {
                (None, None)
            };
            this.update(cx, |this, cx| {
                if let Ok(runs) = listed {
                    this.runs = runs
                        .into_iter()
                        .map(|run| RunRow {
                            run_ref: run.run_ref,
                            title: run.title,
                            state: run.state,
                        })
                        .collect();
                }
                if let Some(value) = capacity {
                    this.capacity_lanes = parse_capacity_lanes(value);
                }
                if let Some(value) = detail.0 {
                    if let Ok(mut parsed) = parse_detail(value) {
                        if let Some(receipt) = detail.1 {
                            parsed.objective_digest = receipt
                                .get("objectiveDigest")
                                .and_then(|v| v.as_str())
                                .map(str::to_string);
                        }
                        this.active_run = Some(parsed);
                        this.mode = SurfaceMode::Run;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_run(&mut self, cx: &mut Context<Self>) {
        self.sync_draft_from_editors(cx);
        let validation = validate_launcher_draft(&self.draft);
        if !validation.ok {
            self.draft.error = validation.message;
            cx.notify();
            return;
        }
        let Some(supervisor) = self.supervisor.clone() else {
            self.draft.error = Some("omega-effectd is not connected.".into());
            cx.notify();
            return;
        };
        self.draft.submitting = true;
        self.draft.error = None;
        let (project_ref, worktree_ref) = self
            .workspace
            .upgrade()
            .map(|workspace| {
                let project = workspace.read(cx).project().clone();
                let project_id = project.entity_id().as_u64();
                let worktree_id = project
                    .read(cx)
                    .worktrees(cx)
                    .next()
                    .map(|wt| wt.read(cx).id());
                (
                    format!("project.{project_id}"),
                    worktree_id
                        .map(|id| format!("worktree.{}", id.to_proto()))
                        .unwrap_or_else(|| "worktree.missing".into()),
                )
            })
            .unwrap_or_else(|| ("project.missing".into(), "worktree.missing".into()));
        if project_ref.ends_with("missing") || worktree_ref.ends_with("missing") {
            self.draft.submitting = false;
            self.draft.error = Some("Open a project worktree before starting Full Auto.".into());
            cx.notify();
            return;
        }
        let params = json!({
            "workspaceRef": self.draft.workspace_ref,
            "title": validation.title,
            "objective": validation.objective,
            "doneCondition": validation.done_condition,
            "lane": self.draft.lane,
            "turnCap": validation.turn_cap,
            "projectRef": project_ref,
            "worktreeRef": worktree_ref,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let started = {
                let mut guard = supervisor.lock().await;
                guard.start_run(params).await
            };
            this.update(cx, |this, cx| {
                this.draft.submitting = false;
                match started {
                    Ok(value) => match parse_detail(value) {
                        Ok(detail) => {
                            this.active_run_ref = Some(detail.run_ref.clone());
                            this.active_run = Some(detail);
                            this.mode = SurfaceMode::Run;
                            this.status = "Full Auto run started.".into();
                            this.refresh_runs(cx);
                        }
                        Err(error) => this.draft.error = Some(error.to_string()),
                    },
                    Err(error) => this.draft.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn mutate_active(&mut self, method: &'static str, cx: &mut Context<Self>) {
        let Some(run_ref) = self.active_run_ref.clone() else {
            return;
        };
        let Some(supervisor) = self.supervisor.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = {
                let mut guard = supervisor.lock().await;
                match method {
                    "pause" => guard.pause_run(&run_ref).await,
                    "resume" => guard.resume_run(&run_ref).await,
                    "handoff-claude" => guard.handoff_run(&run_ref, "claude-local").await,
                    "handoff-codex" => guard.handoff_run(&run_ref, "codex-local").await,
                    "stop" => guard.stop_run(&run_ref).await,
                    "retry" => guard.retry_run(&run_ref).await,
                    _ => Err(omega_effectd::SupervisorError::Anyhow(anyhow!(
                        "unknown mutation"
                    ))),
                }
            };
            this.update(cx, |this, cx| {
                match result {
                    Ok(value) => match parse_detail(value) {
                        Ok(detail) => {
                            this.status = "Full Auto run updated.".into();
                            this.active_run = Some(detail);
                        }
                        Err(error) => {
                            this.status = format!("Full Auto response was invalid: {error}").into();
                        }
                    },
                    Err(error) => {
                        this.status = format!("Full Auto action failed: {error}").into();
                    }
                }
                this.refresh_runs(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_run(&mut self, run_ref: String, cx: &mut Context<Self>) {
        self.active_run_ref = Some(run_ref);
        self.mode = SurfaceMode::Run;
        self.refresh_runs(cx);
        cx.notify();
    }
}

fn parse_detail(value: Value) -> Result<RunDetail> {
    Ok(RunDetail {
        run_ref: value
            .get("runRef")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("runRef"))?
            .to_string(),
        title: value
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Full Auto")
            .to_string(),
        state: value
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        objective: value
            .get("objective")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        done_condition: value
            .get("doneCondition")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        workspace_ref: value
            .get("workspaceRef")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        lane: value
            .get("lane")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        turn_cap: value
            .get("turnCap")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TURN_CAP as u64) as u32,
        successful_attempts: value
            .get("successfulAttempts")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        failed_attempts: value
            .get("failedAttempts")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        stall_cause: value
            .get("stallCause")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        recovery_action: value
            .get("recoveryAction")
            .and_then(|v| v.as_str())
            .unwrap_or("none")
            .to_string(),
        objective_digest: None,
        native_evidence: value.get("nativeEvidence").and_then(|evidence| {
            Some(NativeEvidence {
                project_ref: evidence.get("projectRef")?.as_str()?.to_string(),
                worktree_ref: evidence.get("worktreeRef")?.as_str()?.to_string(),
                git_head: evidence
                    .get("gitHead")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
        }),
        turns: value
            .get("turns")
            .and_then(|v| v.as_array())
            .map(|turns| {
                turns
                    .iter()
                    .filter_map(|turn| {
                        Some((
                            turn.get("lane")?.as_str()?.to_string(),
                            turn.get("outcomeSummary")?.as_str()?.to_string(),
                            turn.get("createdAt")?.as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn parse_capacity_lanes(value: Value) -> Vec<CapacityLane> {
    value
        .get("lanes")
        .and_then(|v| v.as_array())
        .map(|lanes| {
            lanes
                .iter()
                .filter_map(|lane| {
                    Some(CapacityLane {
                        lane: lane.get("lane")?.as_str()?.to_string(),
                        state: lane.get("state")?.as_str()?.to_string(),
                        active_runs: lane.get("activeRuns")?.as_u64()? as u32,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl EventEmitter<PanelEvent> for FullAutoPanel {}

impl Focusable for FullAutoPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FullAutoPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_start = {
            self.sync_draft_from_editors(cx);
            validate_launcher_draft(&self.draft).ok
                && !self.draft.submitting
                && self.supervisor.is_some()
        };
        let active_count = self
            .runs
            .iter()
            .filter(|run| ACTIVE_STATES.contains(&run.state.as_str()))
            .count()
            .min(FULL_AUTO_ACTIVE_LIMIT);

        h_flex()
            .id("full-auto-panel")
            .size_full()
            .track_focus(&self.focus_handle)
            .child(
                v_flex()
                    .flex_1()
                    .size_full()
                    .gap_2()
                    .p_3()
                    .child(Label::new(self.status.clone()).color(Color::Muted))
                    .map(|this| match self.mode {
                        SurfaceMode::Launcher => this
                            .child(Label::new("Full Auto").size(LabelSize::Large))
                            .child(
                                Label::new(
                                    "Describe the outcome once. Providers keep moving until it is done or needs you.",
                                )
                                .color(Color::Muted),
                            )
                            .child(Label::new("What should Full Auto accomplish?"))
                            .child(
                                div()
                                    .border_1()
                                    .border_color(cx.theme().colors().border)
                                    .rounded_md()
                                    .h(px(120.))
                                    .child(self.objective_editor.clone()),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Label::new(FULL_AUTO_WORKSPACE_REF).color(Color::Muted),
                                    )
                                    .child(Label::new("Codex → Claude").color(Color::Muted))
                                    .child(Label::new(format!("{DEFAULT_TURN_CAP} turns")).color(Color::Muted)),
                            )
                            .when(!self.capacity_lanes.is_empty(), |this| {
                                this.child(
                                    Label::new(format!(
                                        "Capacity: {}",
                                        self.capacity_lanes
                                            .iter()
                                            .map(|lane| {
                                                format!(
                                                    "{}={}({})",
                                                    lane.lane, lane.state, lane.active_runs
                                                )
                                            })
                                            .collect::<Vec<_>>()
                                            .join(" · ")
                                    ))
                                    .color(Color::Muted),
                                )
                            })
                            .child(
                                Button::new("full-auto-advanced-toggle", if self.draft.advanced_open {
                                    "Hide Advanced"
                                } else {
                                    "Advanced"
                                })
                                .style(ButtonStyle::Subtle)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.draft.advanced_open = !this.draft.advanced_open;
                                    cx.notify();
                                })),
                            )
                            .when(self.draft.advanced_open, |this| {
                                this.child(Label::new("Title"))
                                    .child(
                                        div()
                                            .border_1()
                                            .border_color(cx.theme().colors().border)
                                            .rounded_md()
                                            .child(self.title_editor.clone()),
                                    )
                                    .child(Label::new("Done condition"))
                                    .child(
                                        div()
                                            .border_1()
                                            .border_color(cx.theme().colors().border)
                                            .rounded_md()
                                            .h(px(72.))
                                            .child(self.done_editor.clone()),
                                    )
                                    .child(Label::new("Turn cap"))
                                    .child(
                                        div()
                                            .border_1()
                                            .border_color(cx.theme().colors().border)
                                            .rounded_md()
                                            .child(self.turn_cap_editor.clone()),
                                    )
                            })
                            .when_some(self.draft.error.clone(), |this, error| {
                                this.child(Label::new(error).color(Color::Error))
                            })
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("full-auto-start", "Start Full Auto")
                                            .style(ButtonStyle::Filled)
                                            .disabled(!can_start)
                                            .on_click(cx.listener(|this, _, _, cx| this.start_run(cx))),
                                    )
                                    .child(
                                        Button::new("full-auto-cancel", "Cancel")
                                            .style(ButtonStyle::Subtle)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.draft = FullAutoLauncherDraft::default();
                                                this.objective_editor.update(cx, |editor, cx| {
                                                    editor.clear(window, cx);
                                                });
                                                cx.notify();
                                            })),
                                    ),
                            ),
                        SurfaceMode::Run => {
                            let Some(run) = self.active_run.clone() else {
                                return this.child(Label::new(
                                    "This Full Auto run could not be found.",
                                ));
                            };
                            let can_pause = run.state == "running";
                            let can_resume = run.state == "paused";
                            let handoff_target = match run.lane.as_deref() {
                                Some("codex-local") if can_resume => Some((
                                    "Handoff to Claude",
                                    "handoff-claude",
                                )),
                                Some("claude-local") if can_resume => {
                                    Some(("Handoff to Codex", "handoff-codex"))
                                }
                                _ => None,
                            };
                            let can_retry =
                                run.state == "stalled" && run.recovery_action == "retry_now";
                            let terminal = ["completed", "failed", "stopped", "cap_reached"]
                                .contains(&run.state.as_str());
                            this.child(Label::new(run.title.clone()).size(LabelSize::Large))
                                .child(Label::new(format!("State: {}", run.state)).color(Color::Accent))
                                .when_some(run.stall_cause.clone(), |this, cause| {
                                    this.child(
                                        Label::new(format!(
                                            "Stall: {cause} · recovery: {}",
                                            run.recovery_action
                                        ))
                                        .color(Color::Warning),
                                    )
                                })
                                .child(
                                    Label::new(format!(
                                        "Workspace: {} · Provider: {} · Cap: {}/{}",
                                        run.workspace_ref.as_deref().unwrap_or("—"),
                                        run.lane.as_deref().unwrap_or("—"),
                                        run.successful_attempts + run.failed_attempts,
                                        run.turn_cap
                                    ))
                                    .color(Color::Muted),
                                )
                                .when_some(run.native_evidence.clone(), |this, evidence| {
                                    this.child(
                                        Label::new(format!(
                                            "Native: {} · {} · git {}",
                                            evidence.project_ref,
                                            evidence.worktree_ref,
                                            evidence.git_head.as_deref().unwrap_or("—")
                                        ))
                                        .color(Color::Muted),
                                    )
                                })
                                .when(!self.capacity_lanes.is_empty(), |this| {
                                    this.child(
                                        Label::new(format!(
                                            "Capacity: {}",
                                            self.capacity_lanes
                                                .iter()
                                                .map(|lane| {
                                                    format!(
                                                        "{}={}({})",
                                                        lane.lane, lane.state, lane.active_runs
                                                    )
                                                })
                                                .collect::<Vec<_>>()
                                                .join(" · ")
                                        ))
                                        .color(Color::Muted),
                                    )
                                })
                                .child(Label::new(run.objective.clone()))
                                .when_some(run.objective_digest.clone(), |this, digest| {
                                    this.child(
                                        Label::new(format!("Receipt objective digest: {digest}"))
                                            .color(Color::Muted),
                                    )
                                })
                                .child(
                                    Label::new(format!("Done when: {}", run.done_condition))
                                        .color(Color::Muted),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .when(can_pause, |row| {
                                            row.child(
                                                Button::new("full-auto-pause", "Pause").on_click(
                                                    cx.listener(|this, _, _, cx| {
                                                        this.mutate_active("pause", cx)
                                                    }),
                                                ),
                                            )
                                        })
                                        .when(can_resume, |row| {
                                            row.child(
                                                Button::new("full-auto-resume", "Resume").on_click(
                                                    cx.listener(|this, _, _, cx| {
                                                        this.mutate_active("resume", cx)
                                                    }),
                                                ),
                                            )
                                        })
                                        .when_some(handoff_target, |row, (label, method)| {
                                            row.child(
                                                Button::new("full-auto-handoff", label)
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.mutate_active(method, cx)
                                                    })),
                                            )
                                        })
                                        .when(can_retry, |row| {
                                            row.child(
                                                Button::new("full-auto-retry", "Retry now")
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.mutate_active("retry", cx)
                                                    })),
                                            )
                                        })
                                        .when(!terminal, |row| {
                                            row.child(
                                                Button::new("full-auto-stop", "Stop")
                                                    .style(ButtonStyle::Tinted(ui::TintColor::Error))
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.mutate_active("stop", cx)
                                                    })),
                                            )
                                        })
                                        .child(
                                            Button::new("full-auto-new", "New run")
                                                .style(ButtonStyle::Subtle)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.open_launcher(cx)
                                                })),
                                        ),
                                )
                                .child(
                                    Label::new(
                                        "Read-only conversation joins through Sync in FA-05. Turns below are service-backed.",
                                    )
                                    .color(Color::Muted),
                                )
                                .child(Label::new("Turns"))
                                .when(run.turns.is_empty(), |this| {
                                    this.child(
                                        Label::new("No turns recorded yet.").color(Color::Muted),
                                    )
                                })
                                .children(run.turns.into_iter().map(|(lane, summary, when)| {
                                    h_flex()
                                        .gap_2()
                                        .child(Label::new(lane).color(Color::Accent))
                                        .child(Label::new(summary))
                                        .child(Label::new(when).color(Color::Muted))
                                }))
                        }
                    }),
            )
            .child(
                v_flex()
                    .id("full-auto-monitor")
                    .w(px(220.))
                    .gap_2()
                    .p_2()
                    .border_l_1()
                    .border_color(cx.theme().colors().border)
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                v_flex()
                                    .child(Label::new("Runs"))
                                    .child(Label::new(if active_count == 0 {
                                        "No active runs".into()
                                    } else {
                                        format!("{active_count} active")
                                    }).color(Color::Muted)),
                            )
                            .child(
                                IconButton::new("full-auto-monitor-new", IconName::Plus)
                                    .tooltip(Tooltip::text("New Full Auto run"))
                                    .on_click(cx.listener(|this, _, _, cx| this.open_launcher(cx))),
                            ),
                    )
                    .children(self.runs.iter().enumerate().take(FULL_AUTO_ACTIVE_LIMIT + 6).map(|(index, run)| {
                        let run_ref = run.run_ref.clone();
                        let selected = self.active_run_ref.as_deref() == Some(run.run_ref.as_str());
                        Button::new(
                            ("full-auto-run-row", index),
                            format!("{} · {}", run.title, run.state),
                        )
                        .style(if selected {
                            ButtonStyle::Filled
                        } else {
                            ButtonStyle::Subtle
                        })
                        .full_width()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_run(run_ref.clone(), cx);
                        }))
                    })),
            )
    }
}

impl Panel for FullAutoPanel {
    fn persistent_name() -> &'static str {
        "FullAutoPanel"
    }

    fn panel_key() -> &'static str {
        PANEL_KEY
    }

    fn position(&self, _: &Window, _: &App) -> DockPosition {
        DockPosition::Right
    }

    fn position_is_valid(&self, _: DockPosition) -> bool {
        true
    }

    fn set_position(&mut self, _: DockPosition, _: &mut Window, _: &mut Context<Self>) {}

    fn default_size(&self, _: &Window, _: &App) -> Pixels {
        px(520.)
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::ZedAgent)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        Some("Full Auto")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        8
    }
}
