//! Bounded Agent Computer panel — start one cloud turn and show outcome.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use editor::Editor;
use gpui::{
    div, px, App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Task, WeakEntity,
    Window,
};
use omega_effectd::{
    fixture_command, OmegaEffectdCommand, OmegaEffectdSupervisor, OmegaEffectdSupervisorOptions,
};
use serde_json::json;
use smol::lock::Mutex as AsyncMutex;
use ui::{prelude::*, Button, ButtonStyle, Label, LabelSize};
use workspace::{
    dock::{DockPosition, Panel, PanelEvent},
    Workspace,
};
use zed_actions::agent_computer::{OpenPanel, StartTurn};

use crate::{DEFAULT_CONTROL_PLANE_BASE_URL, DEFAULT_REPO_REF};

const PANEL_KEY: &str = "AgentComputerPanel";

#[derive(Clone)]
struct SupervisorHandle {
    inner: Arc<AsyncMutex<OmegaEffectdSupervisor>>,
}

#[derive(Clone, Debug)]
struct TurnOutcome {
    session_ref: String,
    state: String,
    finish_reason: String,
    artifact_ref: Option<String>,
}

pub struct AgentComputerPanel {
    _workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    objective_editor: Entity<Editor>,
    repo_editor: Entity<Editor>,
    status: SharedString,
    outcome: Option<TurnOutcome>,
    submitting: bool,
    supervisor: Option<SupervisorHandle>,
}

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace
            .register_action(|workspace, _: &OpenPanel, window, cx| {
                workspace.focus_panel::<AgentComputerPanel>(window, cx);
            })
            .register_action(|workspace, _: &StartTurn, window, cx| {
                if let Some(panel) = workspace.panel::<AgentComputerPanel>(cx) {
                    panel.update(cx, |panel, cx| panel.start_turn(cx));
                }
                workspace.focus_panel::<AgentComputerPanel>(window, cx);
            });
    })
    .detach();
}

impl AgentComputerPanel {
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
                "Public-safe objective for one Agent Computer turn.",
                window,
                cx,
            );
            editor
        });
        let repo_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(DEFAULT_REPO_REF, window, cx);
            editor
        });
        let mut panel = Self {
            _workspace: workspace,
            focus_handle: cx.focus_handle(),
            objective_editor,
            repo_editor,
            status: "Agent Computer launches through omega-effectd only.".into(),
            outcome: None,
            submitting: false,
            supervisor: None,
        };
        panel.ensure_supervisor(cx);
        panel
    }

    fn ensure_supervisor(&mut self, cx: &mut Context<Self>) {
        if self.supervisor.is_some() {
            return;
        }
        let data_root = std::env::var_os("OPENAGENTS_OMEGA_EFFECTD_DATA_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join(".local/share/omega/agent-computer"))
                    .unwrap_or_else(|| std::env::temp_dir().join("omega-agent-computer"))
            });
        self.supervisor = Some(SupervisorHandle {
            inner: Arc::new(AsyncMutex::new(OmegaEffectdSupervisor::new(
                OmegaEffectdSupervisorOptions {
                    data_root,
                    command: effectd_command(),
                    initial_generation: 1,
                    // Cloud turns can take longer than Full Auto control RPCs.
                    request_timeout: Duration::from_secs(180),
                },
            ))),
        });
        cx.notify();
    }

    fn start_turn(&mut self, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        let objective = self.objective_editor.read(cx).text(cx).trim().to_string();
        let repo_ref = self.repo_editor.read(cx).text(cx).trim().to_string();
        if objective.is_empty() {
            self.status = "Objective is required.".into();
            cx.notify();
            return;
        }
        if repo_ref.is_empty() {
            self.status = "repoRef is required.".into();
            cx.notify();
            return;
        }
        let bearer = match std::env::var("OPENAGENTS_AGENT_TOKEN") {
            Ok(token) if !token.trim().is_empty() => token,
            _ => {
                self.status =
                    "Set OPENAGENTS_AGENT_TOKEN in the environment (runtime-only; never stored)."
                        .into();
                cx.notify();
                return;
            }
        };
        let control_plane = std::env::var("OPENAGENTS_CONTROL_PLANE_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_CONTROL_PLANE_BASE_URL.to_string());
        self.ensure_supervisor(cx);
        let Some(supervisor) = self.supervisor.clone() else {
            self.status = "omega-effectd supervisor is unavailable.".into();
            cx.notify();
            return;
        };

        self.submitting = true;
        self.outcome = None;
        self.status = "Starting Agent Computer turn…".into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = {
                let mut guard = supervisor.inner.lock().await;
                let _ = guard.start().await;
                guard
                    .run_agent_computer_turn(json!({
                        "bearerToken": bearer,
                        "controlPlaneBaseUrl": control_plane,
                        "repoRef": repo_ref,
                        "objective": objective,
                    }))
                    .await
            };
            this.update(cx, |panel, cx| {
                panel.submitting = false;
                match result {
                    Ok(value) => {
                        let session = value.get("session");
                        let session_ref = session
                            .and_then(|s| s.get("sessionRef"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let state = session
                            .and_then(|s| s.get("state"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let finish_reason = value
                            .get("finishReason")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let artifact_ref = session
                            .and_then(|s| s.get("artifactRef"))
                            .and_then(|v| v.as_str())
                            .map(str::to_string);
                        panel.outcome = Some(TurnOutcome {
                            session_ref: session_ref.clone(),
                            state: state.clone(),
                            finish_reason: finish_reason.clone(),
                            artifact_ref,
                        });
                        panel.status = format!(
                            "Turn finished: session={session_ref} state={state} finish={finish_reason}"
                        )
                        .into();
                    }
                    Err(error) => {
                        panel.status = format!("Agent Computer turn failed: {error}").into();
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn effectd_command() -> OmegaEffectdCommand {
    if let Ok(bin) = std::env::var("OPENAGENTS_OMEGA_EFFECTD_BIN") {
        return OmegaEffectdCommand {
            program: PathBuf::from(bin),
            args: Vec::new(),
        };
    }
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../openagents/packages/omega-effectd/src/bin/omega-effectd.ts"),
        PathBuf::from(
            "/Users/christopherdavid/work/openagents/packages/omega-effectd/src/bin/omega-effectd.ts",
        ),
    ];
    for candidate in candidates {
        if candidate.exists() {
            return OmegaEffectdCommand {
                program: PathBuf::from("node"),
                args: vec![
                    "--import".into(),
                    "tsx".into(),
                    candidate.display().to_string(),
                ],
            };
        }
    }
    fixture_command(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../omega_effectd/fixtures/fake_effectd.mjs"),
    )
}

impl EventEmitter<PanelEvent> for AgentComputerPanel {}

impl Focusable for AgentComputerPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for AgentComputerPanel {
    fn persistent_name() -> &'static str {
        "AgentComputerPanel"
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

    fn default_size(&self, _: &Window, _: &App) -> gpui::Pixels {
        px(420.)
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::ZedAgent)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        Some("Agent Computer")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(OpenPanel)
    }

    fn activation_priority(&self) -> u32 {
        7
    }
}

impl Render for AgentComputerPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_start = !self.submitting;
        v_flex()
            .id("agent-computer-panel")
            .size_full()
            .track_focus(&self.focus_handle)
            .gap_2()
            .p_3()
            .child(Label::new("Agent Computer").size(LabelSize::Large))
            .child(
                Label::new("Starts one openagents_cloud turn through omega-effectd. Not Full Auto.")
                    .color(Color::Muted),
            )
            .child(Label::new(self.status.clone()).color(Color::Muted))
            .child(Label::new("Objective"))
            .child(
                div()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded_md()
                    .h(px(100.))
                    .child(self.objective_editor.clone()),
            )
            .child(Label::new("repoRef"))
            .child(
                div()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded_md()
                    .child(self.repo_editor.clone()),
            )
            .child(
                Button::new("agent-computer-start", if self.submitting {
                    "Starting…"
                } else {
                    "Start cloud turn"
                })
                .style(ButtonStyle::Filled)
                .disabled(!can_start)
                .on_click(cx.listener(|this, _, _, cx| this.start_turn(cx))),
            )
            .when_some(self.outcome.clone(), |this, outcome| {
                this.child(
                    Label::new(format!(
                        "session={} · state={} · finish={}{}",
                        outcome.session_ref,
                        outcome.state,
                        outcome.finish_reason,
                        outcome
                            .artifact_ref
                            .as_ref()
                            .map(|artifact| format!(" · artifact={artifact}"))
                            .unwrap_or_default()
                    )),
                )
            })
    }
}
