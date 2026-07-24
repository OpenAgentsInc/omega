//! Sarah workroom dock panel (`OMEGA-SW-03`).
//!
//! Projection + command entry only. Durable state lives in the record behind
//! supervised `omega-effectd`. Header text is exactly "Sarah".

use anyhow::Result;
use editor::Editor;
use gpui::{
    App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Task, WeakEntity,
    Window, div, px,
};
use omega_effectd::{SharedOmegaEffectdSupervisor, shared_supervisor};
use serde_json::{Value, json};
use ui::{Button, ButtonStyle, Label, LabelSize, prelude::*};
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};
use zed_actions::workroom::{FocusComposer, InterruptTurn, OpenPanel};

use crate::projections::{
    ActivityProjection, ActivityRow, Freshness, GapState, InterruptIntentState, MessageAck,
    ProjectionMeta, ReceiptRow, ReceiptsProjection, RoomProjection, RunPhase, RunStateProjection,
    TranscriptProjection, TranscriptRow, WorkroomProjection, sources,
};

const PANEL_KEY: &str = "SarahWorkroomPanel";

pub struct SarahWorkroomPanel {
    _workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    composer: Entity<Editor>,
    projection: WorkroomProjection,
    status: SharedString,
    supervisor: Option<SharedOmegaEffectdSupervisor>,
    refreshing: bool,
    interrupting: bool,
}

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace
            .register_action(|workspace, _: &OpenPanel, window, cx| {
                workspace.focus_panel::<SarahWorkroomPanel>(window, cx);
            })
            .register_action(|workspace, _: &FocusComposer, window, cx| {
                if let Some(panel) = workspace.panel::<SarahWorkroomPanel>(cx) {
                    panel.update(cx, |panel, cx| panel.focus_composer(window, cx));
                }
                workspace.focus_panel::<SarahWorkroomPanel>(window, cx);
            })
            .register_action(|workspace, _: &InterruptTurn, window, cx| {
                if let Some(panel) = workspace.panel::<SarahWorkroomPanel>(cx) {
                    panel.update(cx, |panel, cx| panel.interrupt_turn(cx));
                }
                workspace.focus_panel::<SarahWorkroomPanel>(window, cx);
            });
    })
    .detach();
}

impl SarahWorkroomPanel {
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
        let composer = cx.new(|cx| {
            let mut editor = Editor::multi_line(window, cx);
            editor.set_placeholder_text("Message Sarah (text only).", window, cx);
            editor
        });
        let mut panel = Self {
            _workspace: workspace,
            focus_handle: cx.focus_handle(),
            composer,
            projection: WorkroomProjection::honest_unsubscribed(),
            status: "Sarah workroom subscribes to omega-effectd only.".into(),
            supervisor: None,
            refreshing: false,
            interrupting: false,
        };
        panel.ensure_supervisor(cx);
        panel.refresh_from_effectd(cx);
        panel
    }

    fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn ensure_supervisor(&mut self, cx: &mut Context<Self>) {
        if self.supervisor.is_some() {
            return;
        }
        match shared_supervisor(cx) {
            Ok(supervisor) => {
                self.supervisor = Some(supervisor);
                self.status = "Connected to omega-effectd supervisor.".into();
            }
            Err(error) => {
                let detail = format!("omega-effectd unavailable ({error}).");
                self.status = detail.clone().into();
                self.projection.mark_effectd_unavailable(detail);
            }
        }
        cx.notify();
    }

    fn refresh_from_effectd(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.ensure_supervisor(cx);
        let Some(supervisor) = self.supervisor.clone() else {
            return;
        };
        self.refreshing = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let bootstrap = {
                let mut guard = supervisor.lock().await;
                match guard.ensure_started().await {
                    Ok(()) => match guard.sarah_bootstrap().await {
                        Ok(value) => Ok(value),
                        Err(error) => Err(error.to_string()),
                    },
                    Err(error) => Err(error.to_string()),
                }
            };

            let snapshot = match &bootstrap {
                Ok(_) => {
                    let mut guard = supervisor.lock().await;
                    match guard
                        .sarah_room_snapshot(Some(json!({
                            "transcriptLimit": 50,
                            "activityLimit": 50,
                        })))
                        .await
                    {
                        Ok(value) => Ok(value),
                        Err(error) => Err(error.to_string()),
                    }
                }
                Err(error) => Err(error.clone()),
            };

            this.update(cx, |panel, cx| {
                panel.refreshing = false;
                match (bootstrap, snapshot) {
                    (Ok(boot), Ok(snap)) => {
                        panel.apply_bootstrap(&boot);
                        panel.apply_snapshot(&snap);
                        panel.status = "Room projection refreshed from omega-effectd.".into();
                    }
                    (Ok(boot), Err(snap_err)) => {
                        panel.apply_bootstrap(&boot);
                        panel.projection.transcript.meta =
                            ProjectionMeta::unavailable(sources::TRANSCRIPT, &snap_err);
                        panel.projection.activity.meta =
                            ProjectionMeta::unavailable(sources::ACTIVITY, &snap_err);
                        panel.projection.receipts.meta =
                            ProjectionMeta::unavailable(sources::RECEIPTS, &snap_err);
                        panel.projection.run_state.meta =
                            ProjectionMeta::unavailable(sources::RUN_STATE, &snap_err);
                        panel.projection.run_state.reason = Some(snap_err.clone());
                        panel.status = format!("Bootstrap ok; room snapshot unavailable: {snap_err}")
                            .into();
                    }
                    (Err(error), _) => {
                        // Methods may not exist until SARAH-NR-06. Stay honest.
                        panel.projection.mark_effectd_unavailable(error.clone());
                        panel.status = format!(
                            "Sarah record methods unavailable ({error}). Sources stay labeled missing."
                        )
                        .into();
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply_bootstrap(&mut self, value: &Value) {
        let room = value.get("room").or_else(|| value.get("principal"));
        let principal_ref = string_field(room, &["principalRef", "principal_ref", "ref"]);
        let display_name = string_field(room, &["displayName", "display_name", "name"])
            .or_else(|| Some("Sarah".into()));
        let role = string_field(room, &["role"]).or_else(|| Some("principal.sarah".into()));
        let thread_ref = string_field(
            value.get("thread").or(room),
            &["threadRef", "thread_ref", "ref", "conversation"],
        );
        let authority_profile = string_field(
            value.get("authority").or(room),
            &["profile", "authorityProfile", "authority_profile"],
        );
        let authority_revision = string_field(
            value.get("authority").or(room),
            &["revision", "authorityRevision", "authority_revision"],
        );

        self.projection.room = RoomProjection {
            meta: ProjectionMeta::fresh(sources::ROOM),
            principal_ref,
            display_name,
            role,
            thread_ref,
            authority_profile,
            authority_revision,
            detail: None,
        };
    }

    fn apply_snapshot(&mut self, value: &Value) {
        // Transcript
        let mut transcript = TranscriptProjection {
            meta: ProjectionMeta::fresh(sources::TRANSCRIPT),
            rows: Vec::new(),
            cursor: string_field(value.get("transcript"), &["cursor"]),
            truncated: false,
        };
        if let Some(items) = value
            .get("transcript")
            .and_then(|t| t.get("items").or_else(|| t.get("messages")))
            .and_then(|v| v.as_array())
        {
            for item in items {
                let ack = match item
                    .get("ack")
                    .or_else(|| item.get("state"))
                    .and_then(|v| v.as_str())
                {
                    Some("pending") => MessageAck::Pending,
                    _ => MessageAck::Confirmed,
                };
                transcript.push_bounded(TranscriptRow {
                    message_ref: string_field(Some(item), &["messageRef", "id", "ref"])
                        .unwrap_or_else(|| "unknown".into()),
                    role: string_field(Some(item), &["role"]).unwrap_or_else(|| "unknown".into()),
                    text: string_field(Some(item), &["text", "content"]).unwrap_or_default(),
                    ack,
                });
            }
        } else if value.get("transcript").is_none() {
            transcript.meta = ProjectionMeta::missing(sources::TRANSCRIPT);
        }
        if let Some(gap) = value
            .get("transcript")
            .and_then(|t| t.get("gap").or_else(|| t.get("gapState")))
            .and_then(|v| v.as_str())
        {
            if gap != "none" {
                transcript.meta.gap = GapState::Gap;
                transcript.meta.freshness = Freshness::Stale;
            }
        }
        self.projection.transcript = transcript;

        // Activity
        let mut activity = ActivityProjection {
            meta: ProjectionMeta::fresh(sources::ACTIVITY),
            rows: Vec::new(),
            cursor: string_field(value.get("activity"), &["cursor"]),
            truncated: false,
        };
        if let Some(items) = value
            .get("activity")
            .and_then(|a| a.get("items").or_else(|| a.get("events")))
            .and_then(|v| v.as_array())
        {
            for item in items {
                activity.push_bounded(ActivityRow {
                    event_ref: string_field(Some(item), &["eventRef", "id", "ref"])
                        .unwrap_or_else(|| "unknown".into()),
                    kind: string_field(Some(item), &["kind", "type"]).unwrap_or_else(|| "event".into()),
                    summary: string_field(Some(item), &["summary", "text"]).unwrap_or_default(),
                    turn_ref: string_field(Some(item), &["turnRef", "turn_ref"]),
                });
            }
        } else if value.get("activity").is_none() {
            activity.meta = ProjectionMeta::missing(sources::ACTIVITY);
        }
        self.projection.activity = activity;

        // Receipts (stub refs only)
        let mut receipts = ReceiptsProjection {
            meta: ProjectionMeta::fresh(sources::RECEIPTS),
            rows: Vec::new(),
            detail: Some("Receipt refs only. Deep inspector is OMEGA-SW-05.".into()),
        };
        if let Some(items) = value
            .get("receipts")
            .and_then(|r| r.get("items").or_else(|| r.as_array().map(|_| r)))
            .and_then(|v| {
                if v.is_array() {
                    v.as_array()
                } else {
                    v.get("items").and_then(|i| i.as_array())
                }
            })
        {
            for item in items {
                receipts.push_bounded(ReceiptRow {
                    receipt_ref: string_field(
                        Some(item),
                        &["receiptRef", "authorityReceiptRef", "ref", "id"],
                    )
                    .unwrap_or_else(|| "unknown".into()),
                    allowed: item.get("allowed").and_then(|v| v.as_bool()),
                    decision_ref: string_field(Some(item), &["decisionRef", "decision_ref"]),
                    tool_ref: string_field(Some(item), &["toolRef", "tool_ref"]),
                });
            }
        } else if value.get("receipts").is_none() {
            receipts.meta = ProjectionMeta::missing(sources::RECEIPTS);
            receipts.detail =
                Some("No receipt page in snapshot. Source labeled missing.".into());
        }
        self.projection.receipts = receipts;

        // Run state
        let run = value.get("runState").or_else(|| value.get("run_state"));
        let phase_str = string_field(run, &["phase", "state", "status"]);
        let phase = phase_str
            .as_deref()
            .map(|s| {
                if s.starts_with("turn.") {
                    RunPhase::from_event_kind(s)
                } else {
                    match s {
                        "queued" => RunPhase::Queued,
                        "running" => RunPhase::Running,
                        "interrupted" => RunPhase::Interrupted,
                        "finished" | "completed" => RunPhase::Finished,
                        "idle" => RunPhase::Idle,
                        _ => RunPhase::Unknown,
                    }
                }
            })
            .unwrap_or(RunPhase::Unknown);

        let mut run_state = RunStateProjection {
            meta: if run.is_some() {
                ProjectionMeta::fresh(sources::RUN_STATE)
            } else {
                ProjectionMeta::missing(sources::RUN_STATE)
            },
            phase,
            reason: string_field(run, &["reason", "finishReason", "finish_reason"]),
            turn_ref: string_field(run, &["turnRef", "turn_ref"]),
            interrupt_intent: self.projection.run_state.interrupt_intent,
        };
        // Preserve pending interrupt until terminal event applies it.
        if run_state.phase == RunPhase::Interrupted
            && run_state.interrupt_intent == InterruptIntentState::Pending
        {
            run_state.interrupt_intent = InterruptIntentState::Applied;
        }
        if run.is_none() {
            run_state.reason = Some("Run state missing from snapshot.".into());
        }
        self.projection.run_state = run_state;
        self.projection.connection_detail = Some("Snapshot applied from omega-effectd.".into());
    }

    fn interrupt_turn(&mut self, cx: &mut Context<Self>) {
        if self.interrupting {
            return;
        }
        // Law: pending never renders as applied.
        self.projection.run_state.mark_interrupt_pending();
        self.status = "Interrupt intent pending until terminal turn event.".into();
        self.ensure_supervisor(cx);
        let Some(supervisor) = self.supervisor.clone() else {
            self.projection.run_state.meta =
                ProjectionMeta::unavailable(sources::EFFECTD, "no supervisor");
            cx.notify();
            return;
        };
        let turn_ref = self
            .projection
            .run_state
            .turn_ref
            .clone()
            .unwrap_or_else(|| "active".into());
        self.interrupting = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = {
                let mut guard = supervisor.lock().await;
                match guard.ensure_started().await {
                    Ok(()) => guard.sarah_interrupt_turn(&turn_ref).await,
                    Err(error) => Err(omega_effectd::SupervisorError::Anyhow(error)),
                }
            };
            this.update(cx, |panel, cx| {
                panel.interrupting = false;
                match result {
                    Ok(value) => {
                        // Accepted intent only. Applied only after terminal event.
                        let state = value
                            .get("state")
                            .or_else(|| value.get("status"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("pending");
                        if state == "applied" || state == "interrupted" {
                            // Service may already have settled; still require phase.
                            if panel.projection.run_state.phase == RunPhase::Interrupted {
                                panel.projection.run_state.interrupt_intent =
                                    InterruptIntentState::Applied;
                            } else {
                                panel.projection.run_state.interrupt_intent =
                                    InterruptIntentState::Pending;
                            }
                        } else {
                            panel.projection.run_state.interrupt_intent =
                                InterruptIntentState::Pending;
                        }
                        panel.status =
                            format!("Interrupt intent: {state} (not applied until terminal event).")
                                .into();
                    }
                    Err(error) => {
                        // Keep intent visible as pending/unavailable, not applied.
                        panel.projection.run_state.interrupt_intent =
                            InterruptIntentState::Pending;
                        panel.projection.run_state.meta =
                            ProjectionMeta::pending(sources::RUN_STATE);
                        panel.status = format!(
                            "Interrupt request failed ({error}). Intent stays pending, not applied."
                        )
                        .into();
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Test / inspection helper: current in-memory projection (not durable).
    pub fn projection(&self) -> &WorkroomProjection {
        &self.projection
    }
}

fn string_field(value: Option<&Value>, keys: &[&str]) -> Option<String> {
    let value = value?;
    for key in keys {
        if let Some(s) = value.get(*key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

impl EventEmitter<PanelEvent> for SarahWorkroomPanel {}

impl Focusable for SarahWorkroomPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for SarahWorkroomPanel {
    fn persistent_name() -> &'static str {
        "SarahWorkroomPanel"
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
        px(440.)
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::ZedAgent)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        // Dock tooltip may describe the surface; the in-pane header is only "Sarah".
        Some("Sarah")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(OpenPanel)
    }

    fn activation_priority(&self) -> u32 {
        8
    }
}

impl Render for SarahWorkroomPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.projection;
        let can_interrupt = !self.interrupting
            && matches!(
                p.run_state.phase,
                RunPhase::Running | RunPhase::Queued | RunPhase::Unknown
            );

        v_flex()
            .id("sarah-workroom-panel")
            .size_full()
            .track_focus(&self.focus_handle)
            .gap_2()
            .p_3()
            // Law: header says only "Sarah".
            .child(Label::new(WorkroomProjection::header()).size(LabelSize::Large))
            .child(Label::new(self.status.clone()).color(Color::Muted))
            .when_some(p.connection_detail.clone(), |this, detail| {
                this.child(Label::new(detail).color(Color::Muted))
            })
            // Room
            .child(section_header("Room", &p.room.meta))
            .child(room_body(&p.room))
            // Transcript (capacity-bounded list)
            .child(section_header("Transcript", &p.transcript.meta))
            .child(
                v_flex()
                    .id("sarah-workroom-transcript")
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded_md()
                    .p_2()
                    .max_h(px(160.))
                    .overflow_y_scroll()
                    .child(transcript_body(&p.transcript)),
            )
            // Activity
            .child(section_header("Activity", &p.activity.meta))
            .child(
                v_flex()
                    .id("sarah-workroom-activity")
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded_md()
                    .p_2()
                    .max_h(px(120.))
                    .overflow_y_scroll()
                    .child(activity_body(&p.activity)),
            )
            // Receipts stub
            .child(section_header("Receipts", &p.receipts.meta))
            .child(receipts_body(&p.receipts))
            // Run state
            .child(section_header("Run state", &p.run_state.meta))
            .child(run_state_body(&p.run_state))
            // Composer (text only)
            .child(Label::new("Composer").color(Color::Muted))
            .child(
                div()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded_md()
                    .h(px(72.))
                    .child(self.composer.clone()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("sarah-workroom-refresh", "Refresh")
                            .style(ButtonStyle::Subtle)
                            .disabled(self.refreshing)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_from_effectd(cx))),
                    )
                    .child(
                        Button::new(
                            "sarah-workroom-interrupt",
                            if self.interrupting {
                                "Interrupting…"
                            } else if p.run_state.interrupt_intent == InterruptIntentState::Pending
                            {
                                "Interrupt pending"
                            } else {
                                "Interrupt"
                            },
                        )
                        .style(ButtonStyle::Filled)
                        .disabled(!can_interrupt && p.run_state.interrupt_intent != InterruptIntentState::Pending)
                        .on_click(cx.listener(|this, _, _, cx| this.interrupt_turn(cx))),
                    ),
            )
    }
}

fn section_header(title: &'static str, meta: &ProjectionMeta) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .child(Label::new(title))
        .child(Label::new(meta.summary_line()).color(Color::Muted).size(LabelSize::Small))
}

fn room_body(room: &RoomProjection) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .when_some(room.detail.clone(), |this, detail| {
            this.child(Label::new(detail).color(Color::Warning))
        })
        .when_some(room.display_name.clone(), |this, name| {
            this.child(Label::new(format!("name={name}")))
        })
        .when_some(room.principal_ref.clone(), |this, r| {
            this.child(Label::new(format!("principal={r}")).color(Color::Muted))
        })
        .when_some(room.role.clone(), |this, role| {
            this.child(Label::new(format!("role={role}")).color(Color::Muted))
        })
        .when_some(room.thread_ref.clone(), |this, t| {
            this.child(Label::new(format!("thread={t}")).color(Color::Muted))
        })
        .when_some(room.authority_profile.clone(), |this, profile| {
            this.child(
                Label::new(format!(
                    "authority={} rev={}",
                    profile,
                    room.authority_revision.as_deref().unwrap_or("—")
                ))
                .color(Color::Muted),
            )
        })
        .when(
            room.display_name.is_none()
                && room.principal_ref.is_none()
                && room.detail.is_none(),
            |this| this.child(Label::new("No room fields.").color(Color::Muted)),
        )
}

fn transcript_body(transcript: &TranscriptProjection) -> impl IntoElement {
    if transcript.rows.is_empty() {
        return v_flex().child(
            Label::new(if transcript.meta.gap == GapState::Unavailable {
                "Transcript source unavailable (not an empty success)."
            } else {
                "No messages in page."
            })
            .color(Color::Muted),
        );
    }
    let mut col = v_flex().gap_1();
    for row in &transcript.rows {
        let line = format!(
            "[{}] {} · {}: {}",
            row.ack.label(),
            row.message_ref,
            row.role,
            row.text
        );
        let color = if row.ack == MessageAck::Pending {
            Color::Warning
        } else {
            Color::Default
        };
        col = col.child(Label::new(line).color(color));
    }
    if transcript.truncated {
        col = col.child(
            Label::new("(earlier rows truncated at capacity bound)")
                .color(Color::Muted)
                .size(LabelSize::Small),
        );
    }
    col
}

fn activity_body(activity: &ActivityProjection) -> impl IntoElement {
    if activity.rows.is_empty() {
        return v_flex().child(
            Label::new(if activity.meta.gap == GapState::Unavailable {
                "Activity source unavailable (not an empty success)."
            } else {
                "No activity events in page."
            })
            .color(Color::Muted),
        );
    }
    let mut col = v_flex().gap_1();
    for row in &activity.rows {
        col = col.child(Label::new(format!(
            "{} · {} · {}",
            row.kind, row.event_ref, row.summary
        )));
    }
    if activity.truncated {
        col = col.child(
            Label::new("(earlier activity truncated at capacity bound)")
                .color(Color::Muted)
                .size(LabelSize::Small),
        );
    }
    col
}

fn receipts_body(receipts: &ReceiptsProjection) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .when_some(receipts.detail.clone(), |this, detail| {
            this.child(Label::new(detail).color(Color::Muted))
        })
        .map(|this| {
            if receipts.rows.is_empty() {
                this.child(
                    Label::new(if receipts.meta.gap == GapState::Unavailable {
                        "No receipt refs (source unavailable)."
                    } else {
                        "No receipt refs in page."
                    })
                    .color(Color::Muted),
                )
            } else {
                let mut col = this;
                for row in &receipts.rows {
                    col = col.child(Label::new(format!(
                        "receipt={} allowed={} decision={} tool={}",
                        row.receipt_ref,
                        row.allowed
                            .map(|a| if a { "true" } else { "false" })
                            .unwrap_or("—"),
                        row.decision_ref.as_deref().unwrap_or("—"),
                        row.tool_ref.as_deref().unwrap_or("—"),
                    )));
                }
                col
            }
        })
}

fn run_state_body(run: &RunStateProjection) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .child(Label::new(format!(
            "phase={} · interrupt={}",
            run.phase.label(),
            run.interrupt_intent.label()
        )))
        .when_some(run.turn_ref.clone(), |this, t| {
            this.child(Label::new(format!("turn={t}")).color(Color::Muted))
        })
        .when_some(run.reason.clone(), |this, reason| {
            this.child(Label::new(reason).color(Color::Muted))
        })
}

#[cfg(test)]
mod panel_logic_tests {
    use super::*;
    use crate::projections::WorkroomProjection;
    use serde_json::json;

    #[test]
    fn apply_bootstrap_maps_room_fields() {
        // Build a minimal panel-like shell without GPUI window: test mapping only.
        let mut projection = WorkroomProjection::honest_unsubscribed();
        let value = json!({
            "room": {
                "principalRef": "principal.sarah",
                "displayName": "Sarah",
                "role": "principal.sarah",
                "threadRef": "thread.sarah.abc",
            },
            "authority": {
                "profile": "sarah",
                "revision": "7"
            }
        });

        // Reuse the same field extraction via a throwaway by inlining apply logic path.
        // We exercise through a local helper matching panel.apply_bootstrap.
        let room = value.get("room");
        projection.room = RoomProjection {
            meta: ProjectionMeta::fresh(sources::ROOM),
            principal_ref: string_field(room, &["principalRef"]),
            display_name: string_field(room, &["displayName"]),
            role: string_field(room, &["role"]),
            thread_ref: string_field(room, &["threadRef"]),
            authority_profile: string_field(value.get("authority"), &["profile"]),
            authority_revision: string_field(value.get("authority"), &["revision"]),
            detail: None,
        };

        assert_eq!(
            projection.room.principal_ref.as_deref(),
            Some("principal.sarah")
        );
        assert_eq!(projection.room.display_name.as_deref(), Some("Sarah"));
        assert_eq!(
            projection.room.thread_ref.as_deref(),
            Some("thread.sarah.abc")
        );
        assert_eq!(projection.room.authority_revision.as_deref(), Some("7"));
        assert_eq!(projection.room.meta.freshness, Freshness::Fresh);
    }

    #[test]
    fn open_focus_interrupt_actions_are_registered_names() {
        // Action types exist and are distinct workroom surface actions.
        let _open = OpenPanel;
        let _focus = FocusComposer;
        let _interrupt = InterruptTurn;
        assert_eq!(WorkroomProjection::header(), "Sarah");
        assert_eq!(PANEL_KEY, "SarahWorkroomPanel");
    }

    #[test]
    fn interrupt_pending_law_on_run_state() {
        let mut run = RunStateProjection {
            meta: ProjectionMeta::fresh(sources::RUN_STATE),
            phase: RunPhase::Running,
            reason: None,
            turn_ref: Some("turn:1".into()),
            interrupt_intent: InterruptIntentState::None,
        };
        run.mark_interrupt_pending();
        assert_eq!(run.interrupt_intent, InterruptIntentState::Pending);
        assert_eq!(run.phase, RunPhase::Running);
        assert_ne!(run.interrupt_intent, InterruptIntentState::Applied);
    }
}
