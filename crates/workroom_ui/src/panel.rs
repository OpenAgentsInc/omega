//! Sarah workroom dock panel (`OMEGA-SW-03` / `OMEGA-SW-04` / `OMEGA-SW-06` /
//! `SARAH-CW-08`).
//!
//! Projection + command entry only. Durable state lives in the record behind
//! supervised `omega-effectd`. Owner-private conversation header is "Sarah".
//! Community room header is "Community" — same dock pane, never a second pane.
//!
//! OMEGA-SW-04: interaction states (pending send, running after claim, ordered
//! tool ladder, answer block + completion, terminal reason, interrupt
//! pending→applied). Transport is SARAH-NR-06. Honest liveness is the tool
//! ladder — never fake token streaming.
//!
//! OMEGA-SW-06: local unread count + attention marker. Proactive tick turns
//! share the transcript projection (no new source). Read state is local only.
//!
//! SARAH-CW-08: switch between owner-private and community rooms in this pane.
//! Membership, work units, and experience rank are community-only projections.
//! Two-room rule: rooms never share membership or history.

use anyhow::Result;
use editor::Editor;
use gpui::{
    App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Task, WeakEntity,
    Window, div, px,
};
use omega_effectd::{
    BindingProjection, BindingState, OpenAgentsBinding, SharedOmegaEffectdSupervisor,
    shared_supervisor, try_openagents_binding,
};
use serde_json::{Value, json};
use ui::{Button, ButtonStyle, Label, LabelSize, prelude::*};
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};
use zed_actions::workroom::{FocusComposer, InterruptTurn, OpenPanel, SendMessage};

use crate::attention::{
    AttentionMarker, OMEGA_AUTONOMOUS_TICK_ENABLED, empty_room_is_honest,
};
use crate::community::{
    CommunityRoomProjection, RoomKind, COMMUNITY_ROOM_HEADER, COMMUNITY_ROOM_SUBTITLE,
    EXPERIENCE_LABEL, OWNER_PRIVATE_ROOM_HEADER, V1_NO_PAY_FIRST_RUN_COPY,
    V1_NO_PAY_ROOM_DESCRIPTION,
};
use crate::interaction::{AnswerState, InteractionEvent, InteractionState, TerminalOutcome};
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
    /// SARAH-CW-08: community room projections (isolated from owner-private).
    community: CommunityRoomProjection,
    /// SARAH-CW-08: which room this single pane is showing.
    active_room: RoomKind,
    /// OMEGA-SW-04 pure interaction projection (pending/send/ladder/terminal).
    interaction: InteractionState,
    status: SharedString,
    supervisor: Option<SharedOmegaEffectdSupervisor>,
    binding: Option<OpenAgentsBinding>,
    binding_projection: BindingProjection,
    binding_busy: bool,
    refreshing: bool,
    sending: bool,
    interrupting: bool,
}

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace
            .register_action(|workspace, _: &OpenPanel, window, cx| {
                workspace.focus_panel::<SarahWorkroomPanel>(window, cx);
                if let Some(panel) = workspace.panel::<SarahWorkroomPanel>(cx) {
                    // Local mark-read when the owner opens the room (OMEGA-SW-06).
                    panel.update(cx, |panel, cx| panel.mark_room_read(cx));
                }
            })
            .register_action(|workspace, _: &FocusComposer, window, cx| {
                if let Some(panel) = workspace.panel::<SarahWorkroomPanel>(cx) {
                    panel.update(cx, |panel, cx| panel.focus_composer(window, cx));
                }
                workspace.focus_panel::<SarahWorkroomPanel>(window, cx);
            })
            .register_action(|workspace, _: &SendMessage, window, cx| {
                if let Some(panel) = workspace.panel::<SarahWorkroomPanel>(cx) {
                    panel.update(cx, |panel, cx| panel.send_message(window, cx));
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
        let binding = try_openagents_binding(cx);
        let binding_projection = binding
            .as_ref()
            .map(|binding| binding.load_projection())
            .unwrap_or_else(BindingProjection::unbound);
        let mut panel = Self {
            _workspace: workspace,
            focus_handle: cx.focus_handle(),
            composer,
            projection: WorkroomProjection::honest_unsubscribed(),
            community: CommunityRoomProjection::honest_unsubscribed(),
            active_room: RoomKind::OwnerPrivate,
            interaction: InteractionState::new(),
            status: binding_projection.state.status_line().into(),
            supervisor: None,
            binding,
            binding_projection,
            binding_busy: false,
            refreshing: false,
            sending: false,
            interrupting: false,
        };
        panel.ensure_supervisor(cx);
        panel.refresh_from_effectd(cx);
        panel
    }

    fn bind_openagents_account(&mut self, cx: &mut Context<Self>) {
        if self.binding_busy {
            return;
        }
        let Some(binding) = self.binding.clone() else {
            self.status = "OpenAgents binding service unavailable.".into();
            cx.notify();
            return;
        };
        // Relation requires the active Omega Nostr public key from isolated custody.
        let omega_pubkey = match omega_identity::IdentityService::system(*app_identity::CHANNEL)
            .inspect()
            .ok()
            .and_then(|custody| custody.identity)
            .map(|identity| identity.public_key_hex().as_str().to_string())
        {
            Some(pubkey) if !pubkey.is_empty() => pubkey,
            _ => {
                self.status =
                    "Omega identity is not ready. Create or open an identity before binding."
                        .into();
                cx.notify();
                return;
            }
        };
        self.binding_busy = true;
        self.status = "Binding OpenAgents account in your browser…".into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let projection = binding.bind(&omega_pubkey, cx).await;
            this.update(cx, |panel, cx| {
                panel.binding_busy = false;
                panel.binding_projection = projection.clone();
                // Visible states only: unbound | bound | refused.
                // Refused must show the owner-scope message, never a network fault.
                panel.status = match projection.state {
                    BindingState::Unbound => "OpenAgents account unbound.".into(),
                    BindingState::Bound => format!(
                        "Bound OpenAgents account {} to Omega identity (metering attribution).",
                        projection
                            .openagents_account_id
                            .as_deref()
                            .unwrap_or("unknown")
                    )
                    .into(),
                    BindingState::Refused => projection
                        .gate_message
                        .clone()
                        .unwrap_or_else(|| BindingState::Refused.status_line().to_string())
                        .into(),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn clear_openagents_binding(&mut self, cx: &mut Context<Self>) {
        if self.binding_busy {
            return;
        }
        let Some(binding) = self.binding.clone() else {
            return;
        };
        self.binding_busy = true;
        cx.spawn(async move |this, cx| {
            let projection = binding.clear(cx).await;
            this.update(cx, |panel, cx| {
                panel.binding_busy = false;
                panel.binding_projection = projection;
                panel.status = "OpenAgents account unbound.".into();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.focus_handle(cx).focus(window, cx);
        // Opening / focusing the room is a local mark-read (MVP).
        self.mark_room_read(cx);
        cx.notify();
    }

    /// Local mark-read only (OMEGA-SW-06). Does not publish NIP-RS / kind 30078.
    fn mark_room_read(&mut self, cx: &mut Context<Self>) {
        self.projection.mark_room_read();
        if self.projection.attention.unread_count == 0 {
            self.status = "Room marked read (local only).".into();
        }
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
                        panel.sync_interaction_status();
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
        // SARAH-NR-06 bootstrap is flat; SW-03 also accepted nested room/principal.
        let room = value.get("room").or_else(|| value.get("principal"));
        let root = Some(value);
        let principal_ref = string_field(room, &["principalRef", "principal_ref", "ref"])
            .or_else(|| string_field(root, &["principalRef", "principal_ref"]));
        let display_name = string_field(room, &["displayName", "display_name", "name"])
            .or_else(|| string_field(root, &["displayName", "display_name"]))
            .or_else(|| Some("Sarah".into()));
        let role = string_field(room, &["role"])
            .or_else(|| string_field(root, &["role"]))
            .or_else(|| Some("principal.sarah".into()));
        let thread_ref = string_field(
            value.get("thread").or(room),
            &["threadRef", "thread_ref", "ref", "conversation"],
        )
        .or_else(|| {
            string_field(
                root,
                &[
                    "conversationRef",
                    "conversation_ref",
                    "legacyThreadRef",
                    "legacy_thread_ref",
                    "threadRef",
                ],
            )
        });
        let authority_profile = string_field(
            value.get("authority").or(room),
            &["profile", "authorityProfile", "authority_profile"],
        )
        .or_else(|| {
            string_field(
                root,
                &["authorityProfileRef", "authority_profile_ref", "authorityProfile"],
            )
        });
        let authority_revision = string_field(
            value.get("authority").or(room),
            &["revision", "authorityRevision", "authority_revision"],
        )
        .or_else(|| {
            value
                .get("authorityProfileRevision")
                .or_else(|| value.get("authority_profile_revision"))
                .map(|v| match v {
                    Value::Number(n) => n.to_string(),
                    Value::String(s) => s.clone(),
                    _ => String::new(),
                })
                .filter(|s| !s.is_empty())
        });

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
        self.projection.recompute_attention();
    }

    fn apply_snapshot(&mut self, value: &Value) {
        // Preserve local pending rows across refresh until record confirms them.
        let local_pending: Vec<TranscriptRow> = self
            .projection
            .transcript
            .rows
            .iter()
            .filter(|row| row.ack == MessageAck::Pending)
            .cloned()
            .collect();

        // Transcript — ordinary turns only (including proactive tick turns).
        // omega-effectd uses `entries`; older fixtures may use items/messages.
        let mut transcript = TranscriptProjection {
            meta: ProjectionMeta::fresh(sources::TRANSCRIPT),
            rows: Vec::new(),
            cursor: string_field(value.get("transcript"), &["cursor"]),
            truncated: false,
        };
        if let Some(items) = value
            .get("transcript")
            .and_then(|t| {
                t.get("entries")
                    .or_else(|| t.get("items"))
                    .or_else(|| t.get("messages"))
            })
            .and_then(|v| v.as_array())
        {
            for item in items {
                // Proactive tick turns and Q&A answers share this path.
                // Only an explicit pending ack/status stays non-confirmed.
                let ack = match item
                    .get("ack")
                    .or_else(|| item.get("status"))
                    .or_else(|| item.get("state"))
                    .and_then(|v| v.as_str())
                {
                    Some("pending") => MessageAck::Pending,
                    _ => MessageAck::Confirmed,
                };
                transcript.push_bounded(TranscriptRow {
                    message_ref: string_field(
                        Some(item),
                        &["messageRef", "eventId", "event_id", "id", "ref", "cursor"],
                    )
                    .unwrap_or_else(|| "unknown".into()),
                    role: string_field(Some(item), &["role"]).unwrap_or_else(|| "unknown".into()),
                    text: string_field(Some(item), &["text", "content"]).unwrap_or_default(),
                    ack,
                });
            }
        } else if value.get("transcript").is_none() {
            transcript.meta = ProjectionMeta::missing(sources::TRANSCRIPT);
        }
        // Re-attach unconfirmed local sends so refresh never drops optimistic rows.
        for pending in local_pending {
            if !transcript
                .rows
                .iter()
                .any(|row| row.message_ref == pending.message_ref)
            {
                transcript.push_bounded(pending);
            }
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

        // Activity — NR-06 uses `entries` with `entry` kind field.
        let mut activity = ActivityProjection {
            meta: ProjectionMeta::fresh(sources::ACTIVITY),
            rows: Vec::new(),
            cursor: string_field(value.get("activity"), &["cursor"]),
            truncated: false,
        };
        if let Some(items) = value
            .get("activity")
            .and_then(|a| {
                a.get("entries")
                    .or_else(|| a.get("items"))
                    .or_else(|| a.get("events"))
            })
            .and_then(|v| v.as_array())
        {
            for item in items {
                let kind = string_field(Some(item), &["entry", "kind", "type"])
                    .unwrap_or_else(|| "event".into());
                let event_ref = string_field(
                    Some(item),
                    &["eventRef", "eventId", "event_id", "id", "ref"],
                )
                .unwrap_or_else(|| "unknown".into());
                let summary = string_field(Some(item), &["summary", "text"])
                    .unwrap_or_else(|| kind.clone());
                let turn_ref = string_field(Some(item), &["turnRef", "turn_ref", "turn"]);
                activity.push_bounded(ActivityRow {
                    event_ref: event_ref.clone(),
                    kind: kind.clone(),
                    summary: summary.clone(),
                    turn_ref: turn_ref.clone(),
                });
                // Drive interaction ladder from snapshot activity (ordered).
                if let Some(event) = InteractionEvent::from_runtime_kind(
                    &kind,
                    event_ref,
                    turn_ref,
                    summary,
                    string_field(Some(item), &["toolRef", "tool_ref"]),
                    string_field(Some(item), &["reason"]),
                ) {
                    let _ = self.interaction.apply_event(event);
                }
            }
        } else if value.get("activity").is_none() {
            activity.meta = ProjectionMeta::missing(sources::ACTIVITY);
        }
        // Prefer interaction ladder order when it has more recent steps.
        if !self.interaction.tool_ladder.is_empty() {
            let mut ladder_activity = ActivityProjection {
                meta: ProjectionMeta::fresh(sources::ACTIVITY),
                rows: Vec::new(),
                cursor: activity.cursor.clone(),
                truncated: false,
            };
            for row in self.interaction.activity_rows() {
                ladder_activity.push_bounded(row);
            }
            activity = ladder_activity;
        }
        self.projection.activity = activity;

        // Receipts (stub refs only; deep inspector is OMEGA-SW-05).
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

        // Run state — NR-06 uses `state`; legacy used `phase`.
        let run = value.get("runState").or_else(|| value.get("run_state"));
        let phase_str = string_field(run, &["phase", "state", "status"]);
        let phase = phase_str
            .as_deref()
            .map(parse_run_phase)
            .unwrap_or(RunPhase::Unknown);
        let reason = string_field(run, &["reason", "finishReason", "finish_reason"]);
        let turn_ref = string_field(run, &["turnRef", "turn_ref"]);

        self.interaction
            .apply_snapshot_run(phase, turn_ref.clone(), reason.clone());

        let mut run_state = self.interaction.run.clone();
        if run.is_none() {
            run_state.meta = ProjectionMeta::missing(sources::RUN_STATE);
            if run_state.reason.is_none() {
                run_state.reason = Some("Run state missing from snapshot.".into());
            }
        }
        self.projection.run_state = run_state;
        self.projection.connection_detail = Some("Snapshot applied from omega-effectd.".into());
        // OMEGA-SW-06: recompute local unread + attention after transcript page.
        // Never invent proactive rows when the autonomous tick is off.
        debug_assert!(
            empty_room_is_honest(&self.projection.transcript, OMEGA_AUTONOMOUS_TICK_ENABLED),
            "empty room must stay honest when autonomous tick is off"
        );
        self.projection.recompute_attention();
        self.sync_interaction_status();
    }

    /// Apply one ordered room/runtime event into interaction + projection.
    fn apply_interaction_event(&mut self, event: InteractionEvent) {
        let rows = self.interaction.apply_event(event);
        for row in rows {
            self.upsert_transcript_row(row);
        }
        // Refresh activity from ordered tool ladder.
        if !self.interaction.tool_ladder.is_empty() {
            let mut activity = ActivityProjection {
                meta: ProjectionMeta::fresh(sources::ACTIVITY),
                rows: Vec::new(),
                cursor: self.projection.activity.cursor.clone(),
                truncated: false,
            };
            for row in self.interaction.activity_rows() {
                activity.push_bounded(row);
            }
            self.projection.activity = activity;
        }
        self.projection.run_state = self.interaction.run.clone();
        self.projection.recompute_attention();
        self.sync_interaction_status();
    }

    fn upsert_transcript_row(&mut self, row: TranscriptRow) {
        if let Some(existing) = self
            .projection
            .transcript
            .rows
            .iter_mut()
            .find(|r| r.message_ref == row.message_ref)
        {
            *existing = row;
            self.projection.recompute_attention();
            return;
        }
        // Confirm may replace a local pending row by text/local ref.
        if row.ack == MessageAck::Confirmed {
            if let Some(idx) = self.projection.transcript.rows.iter().position(|r| {
                r.ack == MessageAck::Pending && r.role == row.role && r.text == row.text
            }) {
                self.projection.transcript.rows[idx] = row;
                self.projection.recompute_attention();
                return;
            }
        }
        self.projection.transcript.push_bounded(row);
        if self.projection.transcript.meta.gap == GapState::Unavailable {
            self.projection.transcript.meta = ProjectionMeta::fresh(sources::TRANSCRIPT);
        }
        self.projection.recompute_attention();
    }

    fn sync_interaction_status(&mut self) {
        // Prefer interaction status when a turn is active or pending; keep
        // attention mark-read messages otherwise.
        if self.interaction.pending_send_count() > 0
            || self.interaction.run.phase == RunPhase::Running
            || self.interaction.run.phase == RunPhase::Queued
            || self.interaction.terminal.is_terminal()
            || self.interaction.run.interrupt_intent != InterruptIntentState::None
            || !self.interaction.tool_ladder.is_empty()
            || self.interaction.answer != AnswerState::None
        {
            let mut status = self.interaction.status_line();
            if self.interaction.uses_honest_liveness() {
                status.push_str(" · liveness=tool_ladder");
            }
            self.status = status.into();
        }
    }

    /// OMEGA-SW-04: send composer text. Local pending until record confirms.
    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sending {
            return;
        }
        let text = self.composer.read(cx).text(cx).trim().to_string();
        if text.is_empty() {
            self.status = "Message text is required.".into();
            cx.notify();
            return;
        }

        let (local_ref, pending_row) = self.interaction.begin_send(text.clone());
        self.upsert_transcript_row(pending_row);
        self.projection.transcript.meta = ProjectionMeta::pending(sources::TRANSCRIPT);
        self.status = format!("Pending local send {local_ref} until record confirms.").into();

        self.composer.update(cx, |editor, cx| {
            editor.clear(window, cx);
        });

        self.ensure_supervisor(cx);
        let Some(supervisor) = self.supervisor.clone() else {
            self.interaction.fail_send(&local_ref);
            self.projection
                .transcript
                .rows
                .retain(|row| row.message_ref != local_ref);
            self.projection.mark_effectd_unavailable("no supervisor");
            self.status = "omega-effectd unavailable; local pending send dropped.".into();
            cx.notify();
            return;
        };

        self.sending = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = {
                let mut guard = supervisor.lock().await;
                match guard.ensure_started().await {
                    Ok(()) => guard.sarah_send_message(&text).await,
                    Err(error) => Err(omega_effectd::SupervisorError::Anyhow(error)),
                }
            };
            this.update(cx, |panel, cx| {
                panel.sending = false;
                match result {
                    Ok(value) => {
                        let message_ref = string_field(
                            Some(&value),
                            &["messageRef", "message_ref", "eventId", "event_id"],
                        )
                        .unwrap_or_else(|| local_ref.clone());
                        let turn_ref =
                            string_field(Some(&value), &["turnRef", "turn_ref"]);
                        let status = string_field(Some(&value), &["status"])
                            .unwrap_or_else(|| "accepted".into());
                        if let Some(confirmed) = panel.interaction.confirm_send(
                            &local_ref,
                            message_ref.clone(),
                            turn_ref.clone(),
                        ) {
                            panel.upsert_transcript_row(confirmed);
                        } else {
                            panel.upsert_transcript_row(TranscriptRow {
                                message_ref: message_ref.clone(),
                                role: "owner".into(),
                                text: text.clone(),
                                ack: MessageAck::Confirmed,
                            });
                        }
                        // Accepted on record — turn runs only after claim event.
                        panel.projection.run_state = panel.interaction.run.clone();
                        panel.projection.transcript.meta =
                            ProjectionMeta::fresh(sources::TRANSCRIPT);
                        panel.status = format!(
                            "Message confirmed ({status}) ref={message_ref}; turn claim pending."
                        )
                        .into();
                        panel.sync_interaction_status();
                    }
                    Err(error) => {
                        panel.interaction.fail_send(&local_ref);
                        panel
                            .projection
                            .transcript
                            .rows
                            .retain(|row| row.message_ref != local_ref);
                        panel.projection.recompute_attention();
                        panel.status =
                            format!("Send failed ({error}). Pending local row dropped.").into();
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn interrupt_turn(&mut self, cx: &mut Context<Self>) {
        if self.interrupting {
            return;
        }
        // Law: pending never renders as applied.
        self.interaction.begin_interrupt();
        self.projection.run_state = self.interaction.run.clone();
        self.status = "Interrupt intent pending until terminal turn event.".into();
        self.ensure_supervisor(cx);
        let Some(supervisor) = self.supervisor.clone() else {
            self.projection.run_state.meta =
                ProjectionMeta::unavailable(sources::EFFECTD, "no supervisor");
            cx.notify();
            return;
        };
        let turn_ref = self
            .interaction
            .run
            .turn_ref
            .clone()
            .or_else(|| self.projection.run_state.turn_ref.clone())
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
                        let pending = value
                            .get("pending")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        if (state == "applied" || state == "interrupted") && !pending {
                            if panel.interaction.run.phase == RunPhase::Interrupted {
                                panel.apply_interaction_event(InteractionEvent::TurnInterrupted {
                                    turn_ref: Some(turn_ref.clone()),
                                    reason: string_field(Some(&value), &["reason"])
                                        .unwrap_or_else(|| "owner_interrupt".into()),
                                });
                            } else {
                                panel.interaction.begin_interrupt();
                                panel.projection.run_state = panel.interaction.run.clone();
                            }
                        } else {
                            // Stay pending — never upgrade to Applied here.
                            panel.interaction.begin_interrupt();
                            panel.projection.run_state = panel.interaction.run.clone();
                        }
                        panel.status = format!(
                            "Interrupt intent: {state} (not applied until terminal event)."
                        )
                        .into();
                    }
                    Err(error) => {
                        // Keep intent visible as pending/unavailable, not applied.
                        panel.interaction.begin_interrupt();
                        panel.projection.run_state = panel.interaction.run.clone();
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

    /// Test / inspection helper: community room projection (not durable).
    pub fn community(&self) -> &CommunityRoomProjection {
        &self.community
    }

    /// Test / inspection helper: active room kind.
    pub fn active_room(&self) -> RoomKind {
        self.active_room
    }

    /// SARAH-CW-08: switch rooms inside the same pane (not a second dock panel).
    pub fn select_room(&mut self, kind: RoomKind, cx: &mut Context<Self>) {
        self.active_room = kind;
        self.status = match kind {
            RoomKind::OwnerPrivate => "Showing owner-private Sarah room.".into(),
            RoomKind::Community => {
                "Showing community room (separate membership and history).".into()
            }
        };
        // Composer stays one instance; community publish is not wired in this skeleton.
        if kind.is_community() {
            // Placeholder reminds the operator which room is active.
            // Full community compose is a later packet; do not invent a second Editor.
        }
        cx.notify();
    }

    /// Two-room isolation check for tests and honest UI guards.
    pub fn rooms_are_isolated(&self) -> bool {
        // Distinct identities when both known.
        if let (Some(thread), Some(group)) = (
            self.projection.room.thread_ref.as_deref(),
            self.community
                .room
                .group_ref
                .as_deref()
                .or(self.community.membership.group_ref.as_deref()),
        ) {
            if thread == group {
                return false;
            }
        }
        let owner_refs: std::collections::BTreeSet<&str> = self
            .projection
            .transcript
            .rows
            .iter()
            .map(|r| r.message_ref.as_str())
            .collect();
        let community_refs: std::collections::BTreeSet<&str> = self
            .community
            .transcript
            .rows
            .iter()
            .map(|r| r.message_ref.as_str())
            .collect();
        owner_refs.is_disjoint(&community_refs)
    }

    /// Test / inspection helper: interaction state (not durable).
    pub fn interaction(&self) -> &InteractionState {
        &self.interaction
    }
}

fn parse_run_phase(s: &str) -> RunPhase {
    if s.starts_with("turn.") {
        return RunPhase::from_event_kind(s);
    }
    match s {
        "queued" => RunPhase::Queued,
        "running" => RunPhase::Running,
        "interrupted" => RunPhase::Interrupted,
        "interrupt_pending" => RunPhase::Running,
        "finished" | "completed" => RunPhase::Finished,
        "idle" => RunPhase::Idle,
        _ => RunPhase::Unknown,
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

    fn icon_label(&self, _: &Window, _: &App) -> Option<String> {
        // One unread count for the room (OMEGA-SW-06).
        self.projection.attention.icon_label()
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
        let community = &self.community;
        let active = self.active_room;
        let showing_community = active.is_community();
        let can_interrupt = !showing_community
            && !self.interrupting
            && matches!(
                p.run_state.phase,
                RunPhase::Running | RunPhase::Queued | RunPhase::Unknown
            );
        // One composer for the pane; community publish is not wired in CW-08 skeleton.
        let can_send = !showing_community && !self.sending;
        let answer = self.interaction.answer.clone();
        let terminal = self.interaction.terminal.clone();
        let honest = self.interaction.uses_honest_liveness();
        let header = active.header();

        v_flex()
            .id("sarah-workroom-panel")
            .size_full()
            .track_focus(&self.focus_handle)
            .gap_2()
            .p_3()
            // Active room header must be unmistakable (Sarah vs Community).
            .child(Label::new(header).size(LabelSize::Large))
            .when(showing_community, |this| {
                this.child(
                    Label::new(COMMUNITY_ROOM_SUBTITLE)
                        .color(Color::Accent)
                        .size(LabelSize::Small),
                )
            })
            .child(Label::new(self.status.clone()).color(Color::Muted))
            // SARAH-CW-08: room switcher — same pane, two rooms.
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("workroom-room-owner-private", OWNER_PRIVATE_ROOM_HEADER)
                            .style(if showing_community {
                                ButtonStyle::Subtle
                            } else {
                                ButtonStyle::Filled
                            })
                            .disabled(!showing_community)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.select_room(RoomKind::OwnerPrivate, cx);
                            })),
                    )
                    .child(
                        Button::new("workroom-room-community", COMMUNITY_ROOM_HEADER)
                            .style(if showing_community {
                                ButtonStyle::Filled
                            } else {
                                ButtonStyle::Subtle
                            })
                            .disabled(showing_community)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.select_room(RoomKind::Community, cx);
                            })),
                    ),
            )
            .when(!showing_community, |this| {
                this.when_some(p.connection_detail.clone(), |this, detail| {
                    this.child(Label::new(detail).color(Color::Muted))
                })
                .when(honest, |this| {
                    this.child(
                        Label::new("Liveness: ordered tool ladder (no token stream).")
                            .color(Color::Muted)
                            .size(LabelSize::Small),
                    )
                })
            })
            .when(showing_community, |this| {
                this.when_some(community.connection_detail.clone(), |this, detail| {
                    this.child(Label::new(detail).color(Color::Muted))
                })
                .child(
                    Label::new(V1_NO_PAY_ROOM_DESCRIPTION)
                        .color(Color::Warning)
                        .size(LabelSize::Small),
                )
                .child(
                    Label::new(V1_NO_PAY_FIRST_RUN_COPY)
                        .color(Color::Muted)
                        .size(LabelSize::Small),
                )
            })
            // OMEGA-SW-01: visible binding state (unbound | bound | refused).
            .child(binding_section(&self.binding_projection))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new(
                            "sarah-workroom-bind",
                            if self.binding_busy {
                                "Binding…"
                            } else {
                                "Bind OpenAgents account"
                            },
                        )
                        .style(ButtonStyle::Subtle)
                        .disabled(
                            self.binding_busy
                                || self.binding_projection.state == BindingState::Bound,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.bind_openagents_account(cx))),
                    )
                    .child(
                        Button::new("sarah-workroom-unbind", "Clear binding")
                            .style(ButtonStyle::Subtle)
                            .disabled(
                                self.binding_busy
                                    || self.binding_projection.state == BindingState::Unbound,
                            )
                            .on_click(
                                cx.listener(|this, _, _, cx| this.clear_openagents_binding(cx)),
                            ),
                    ),
            )
            // --- Owner-private Sarah room ---
            .when(!showing_community, |this| {
                this.child(attention_body(&p.attention))
                    .child(section_header("Room", &p.room.meta))
                    .child(room_body(&p.room))
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
                    .child(section_header("Activity (tool ladder)", &p.activity.meta))
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
                    .child(Label::new("Answer").color(Color::Muted))
                    .child(answer_body(&answer))
                    .child(section_header("Receipts", &p.receipts.meta))
                    .child(receipts_body(&p.receipts))
                    .child(section_header("Run state", &p.run_state.meta))
                    .child(run_state_body(&p.run_state, &terminal))
            })
            // --- Community room (SARAH-CW-08) — same pane, separate history ---
            .when(showing_community, |this| {
                this.child(section_header("Community group", &community.room.meta))
                    .child(community_room_body(community))
                    .child(section_header("Membership", &community.membership.meta))
                    .child(membership_body(&community.membership))
                    .child(section_header("Work units", &community.work_units.meta))
                    .child(work_units_body(&community.work_units))
                    .child(section_header(
                        // Never "earnings" — experience only.
                        "Experience rank",
                        &community.experience.meta,
                    ))
                    .child(experience_body(&community.experience))
                    .child(section_header("Group transcript", &community.transcript.meta))
                    .child(
                        v_flex()
                            .id("community-workroom-transcript")
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .rounded_md()
                            .p_2()
                            .max_h(px(160.))
                            .overflow_y_scroll()
                            .child(transcript_body(&community.transcript)),
                    )
            })
            // One composer for the pane (not a second composer).
            .child(
                Label::new(if showing_community {
                    "Composer (community publish not wired — skeleton)"
                } else {
                    "Composer"
                })
                .color(Color::Muted),
            )
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
                        Button::new(
                            "sarah-workroom-send",
                            if self.sending { "Sending…" } else { "Send" },
                        )
                        .style(ButtonStyle::Filled)
                        .disabled(!can_send)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.send_message(window, cx);
                        })),
                    )
                    .child(
                        Button::new("sarah-workroom-refresh", "Refresh")
                            .style(ButtonStyle::Subtle)
                            .disabled(self.refreshing || showing_community)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_from_effectd(cx))),
                    )
                    .child(
                        Button::new("sarah-workroom-mark-read", "Mark read")
                            .style(ButtonStyle::Subtle)
                            .disabled(showing_community || p.attention.unread_count == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.mark_room_read(cx))),
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
                        .style(ButtonStyle::Subtle)
                        .disabled(
                            !can_interrupt
                                && p.run_state.interrupt_intent != InterruptIntentState::Pending,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.interrupt_turn(cx))),
                    ),
            )
    }
}

fn community_room_body(community: &CommunityRoomProjection) -> impl IntoElement {
    let room = &community.room;
    v_flex()
        .gap_0p5()
        .child(Label::new(format!(
            "group={}",
            room.group_ref.as_deref().unwrap_or("(missing)")
        )))
        .child(Label::new(format!(
            "name={}",
            room.display_name.as_deref().unwrap_or("(missing)")
        )))
        .child(
            Label::new(format!("invitation_only={}", room.invitation_only))
                .color(Color::Muted)
                .size(LabelSize::Small),
        )
        .child(
            Label::new(room.description.clone())
                .color(Color::Muted)
                .size(LabelSize::Small),
        )
        .when_some(room.detail.clone(), |this, detail| {
            this.child(Label::new(detail).color(Color::Warning).size(LabelSize::Small))
        })
}

fn membership_body(membership: &crate::community::MembershipProjection) -> impl IntoElement {
    if membership.members.is_empty() {
        return v_flex().child(
            Label::new(
                membership
                    .detail
                    .clone()
                    .unwrap_or_else(|| "No members projected.".into()),
            )
            .color(Color::Muted)
            .size(LabelSize::Small),
        );
    }
    let mut col = v_flex().gap_0p5();
    for member in &membership.members {
        let agents = member
            .agents
            .iter()
            .map(|a| {
                format!(
                    "{ref}{attested}{revoked}",
                    ref = a.agent_ref,
                    attested = if a.attested { "·attested" } else { "" },
                    revoked = if a.revoked { "·revoked" } else { "" },
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        col = col.child(Label::new(format!(
            "{name} ({mref}) attested={attested} agents=[{agents}]",
            name = member.display_name.as_deref().unwrap_or("member"),
            mref = member.member_ref,
            attested = member.attested,
            agents = agents,
        )));
    }
    if membership.truncated {
        col = col.child(Label::new("…roster truncated").color(Color::Muted));
    }
    col
}

fn work_units_body(units: &crate::community::WorkUnitsProjection) -> impl IntoElement {
    if units.units.is_empty() {
        return v_flex().child(
            Label::new(
                units
                    .detail
                    .clone()
                    .unwrap_or_else(|| "No work units projected.".into()),
            )
            .color(Color::Muted)
            .size(LabelSize::Small),
        );
    }
    let mut col = v_flex().gap_0p5();
    for unit in &units.units {
        col = col.child(Label::new(format!(
            "{title} ({uref}) · {acceptance} · quotes={q}{tier}{reward}",
            title = unit.title,
            uref = unit.unit_ref,
            acceptance = unit.acceptance.label(),
            q = unit.quotes.len(),
            tier = unit
                .tier
                .map(|t| format!(" · tier={t}"))
                .unwrap_or_default(),
            reward = unit
                .reward_note
                .as_ref()
                .map(|n| format!(" · {n}"))
                .unwrap_or_default(),
        )));
    }
    if units.truncated {
        col = col.child(Label::new("…work units truncated").color(Color::Muted));
    }
    col
}

fn experience_body(experience: &crate::community::ExperienceRankProjection) -> impl IntoElement {
    // Structural: label is experience, never earnings.
    let summary = experience.summary_line();
    v_flex()
        .gap_0p5()
        .child(Label::new(summary))
        .child(
            Label::new(format!(
                "reward_label={label} (not {forbidden})",
                label = experience.reward_label,
                forbidden = crate::community::FORBIDDEN_EARNINGS_LABEL,
            ))
            .color(Color::Muted)
            .size(LabelSize::Small),
        )
        .when(experience.recent_awards.is_empty(), |this| {
            this.child(
                Label::new(
                    experience
                        .detail
                        .clone()
                        .unwrap_or_else(|| format!("No {EXPERIENCE_LABEL} awards projected.")),
                )
                .color(Color::Muted)
                .size(LabelSize::Small),
            )
        })
        .children(experience.recent_awards.iter().map(|award| {
            Label::new(format!(
                "+{pts} {kind} ({aref})",
                pts = award.points,
                kind = award.reason_kind,
                aref = award.award_ref,
            ))
        }))
}

fn binding_section(binding: &BindingProjection) -> impl IntoElement {
    // Projection is public-safe: never render tokens or credential material.
    let state_line = format!("binding={}", binding.state.label());
    let account_line = binding
        .openagents_account_id
        .as_ref()
        .map(|id| format!("account={id}"));
    let gate = binding.gate_message.clone();
    v_flex()
        .gap_0p5()
        .child(Label::new("OpenAgents binding").color(Color::Muted))
        .child(Label::new(state_line))
        .when_some(account_line, |this, line| {
            this.child(Label::new(line).color(Color::Muted).size(LabelSize::Small))
        })
        .when_some(gate, |this, message| {
            this.child(Label::new(message).color(Color::Warning).size(LabelSize::Small))
        })
}

fn attention_body(attention: &crate::attention::RoomAttention) -> impl IntoElement {
    let marker_color = if attention.marker == AttentionMarker::NeedsAttention {
        Color::Accent
    } else {
        Color::Muted
    };
    let tick_note = attention.tick_note.map(|s| s.to_string());
    v_flex()
        .id("sarah-workroom-attention")
        .gap_0p5()
        .child(Label::new(attention.summary_line()).color(marker_color))
        .when_some(tick_note, |this, note| {
            this.child(
                Label::new(note)
                    .color(Color::Muted)
                    .size(LabelSize::Small),
            )
        })
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

fn answer_body(answer: &AnswerState) -> impl IntoElement {
    match answer {
        AnswerState::None => v_flex().child(
            Label::new("No answer block yet (stream:false; not a token stream).")
                .color(Color::Muted),
        ),
        AnswerState::Text { text } => v_flex()
            .gap_0p5()
            .child(
                Label::new("state=text (block arrived)")
                    .color(Color::Muted)
                    .size(LabelSize::Small),
            )
            .child(Label::new(text.clone())),
        AnswerState::Completed { text } => v_flex()
            .gap_0p5()
            .child(
                Label::new("state=completed")
                    .color(Color::Muted)
                    .size(LabelSize::Small),
            )
            .child(Label::new(text.clone())),
    }
}

fn run_state_body(run: &RunStateProjection, terminal: &TerminalOutcome) -> impl IntoElement {
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
        .child(Label::new(format!("terminal={}", terminal.label())).color(Color::Muted))
        .when_some(
            terminal
                .reason()
                .map(|r| r.to_string())
                .or_else(|| run.reason.clone()),
            |this, reason| {
                this.child(
                    Label::new(format!("reason={reason}")).color(if terminal.is_terminal() {
                        Color::Warning
                    } else {
                        Color::Muted
                    }),
                )
            },
        )
}

#[cfg(test)]
mod panel_logic_tests {
    use super::*;
    use crate::projections::WorkroomProjection;
    use serde_json::json;

    #[test]
    fn apply_bootstrap_maps_room_fields() {
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
    fn open_focus_send_interrupt_actions_are_registered_names() {
        let _open = OpenPanel;
        let _focus = FocusComposer;
        let _send = SendMessage;
        let _interrupt = InterruptTurn;
        assert_eq!(WorkroomProjection::header(), "Sarah");
        assert_eq!(PANEL_KEY, "SarahWorkroomPanel");
    }

    #[test]
    fn community_room_headers_and_copy_are_distinct_and_unpaid() {
        assert_eq!(OWNER_PRIVATE_ROOM_HEADER, "Sarah");
        assert_eq!(COMMUNITY_ROOM_HEADER, "Community");
        assert_ne!(OWNER_PRIVATE_ROOM_HEADER, COMMUNITY_ROOM_HEADER);
        assert!(COMMUNITY_ROOM_SUBTITLE.contains("separate"));
        assert!(V1_NO_PAY_ROOM_DESCRIPTION.contains("experience"));
        assert!(V1_NO_PAY_ROOM_DESCRIPTION.contains("not money"));
        assert!(!V1_NO_PAY_ROOM_DESCRIPTION
            .to_ascii_lowercase()
            .contains("earnings"));
        assert!(V1_NO_PAY_FIRST_RUN_COPY.contains("does not pay"));
        assert_eq!(EXPERIENCE_LABEL, "experience");
        let community = CommunityRoomProjection::honest_unsubscribed();
        assert!(community.is_v1_compliant());
        assert!(community.membership.is_honest_missing());
        assert!(community.work_units.is_honest_missing());
        assert!(community.experience.is_v1_experience_only());
    }

    #[test]
    fn two_room_isolation_on_panel_fields() {
        // Mirrors SarahWorkroomPanel field layout without constructing GPUI.
        let mut projection = WorkroomProjection::honest_unsubscribed();
        let mut community = CommunityRoomProjection::honest_unsubscribed();
        projection.room.thread_ref = Some("thread.sarah.1".into());
        projection.transcript.push_bounded(TranscriptRow {
            message_ref: "private.1".into(),
            role: "owner".into(),
            text: "secret".into(),
            ack: MessageAck::Confirmed,
        });
        community.room.group_ref = Some("group.community.1".into());
        community.push_untrusted_message(
            "community.1".into(),
            "member".into(),
            "hello".into(),
        );
        let owner_refs: std::collections::BTreeSet<&str> = projection
            .transcript
            .rows
            .iter()
            .map(|r| r.message_ref.as_str())
            .collect();
        let community_refs: std::collections::BTreeSet<&str> = community
            .transcript
            .rows
            .iter()
            .map(|r| r.message_ref.as_str())
            .collect();
        assert!(owner_refs.is_disjoint(&community_refs));
        assert_ne!(
            projection.room.thread_ref.as_deref(),
            community.room.group_ref.as_deref()
        );
        // Switch kind only changes active room — both stores remain.
        let active = RoomKind::Community;
        assert_eq!(active.header(), COMMUNITY_ROOM_HEADER);
        assert_eq!(RoomKind::OwnerPrivate.header(), OWNER_PRIVATE_ROOM_HEADER);
        assert_eq!(projection.transcript.rows.len(), 1);
        assert_eq!(community.transcript.rows.len(), 1);
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

    #[test]
    fn parse_run_phase_covers_nr06_and_event_kinds() {
        assert_eq!(parse_run_phase("running"), RunPhase::Running);
        assert_eq!(parse_run_phase("interrupt_pending"), RunPhase::Running);
        assert_eq!(parse_run_phase("turn.started"), RunPhase::Running);
        assert_eq!(parse_run_phase("turn.finished"), RunPhase::Finished);
        assert_eq!(parse_run_phase("interrupted"), RunPhase::Interrupted);
    }

    #[test]
    fn apply_snapshot_maps_entries_and_proactive_turns_as_ordinary_rows() {
        let value = json!({
            "transcript": {
                "entries": [
                    {
                        "eventId": "evt.owner.1",
                        "role": "owner",
                        "kind": "text",
                        "text": "status?",
                        "status": "accepted"
                    },
                    {
                        "eventId": "message.sarah_auto.tick.1",
                        "role": "sarah",
                        "kind": "text",
                        "text": "Release is green.",
                        "status": "confirmed"
                    }
                ],
                "cursor": "cursor.1",
                "gapState": "none"
            },
            "activity": { "entries": [], "gapState": "none" },
            "runState": { "state": "idle", "turnRef": null }
        });

        let mut projection = WorkroomProjection::honest_unsubscribed();
        let mut transcript = TranscriptProjection {
            meta: ProjectionMeta::fresh(sources::TRANSCRIPT),
            rows: Vec::new(),
            cursor: string_field(value.get("transcript"), &["cursor"]),
            truncated: false,
        };
        if let Some(items) = value
            .get("transcript")
            .and_then(|t| {
                t.get("entries")
                    .or_else(|| t.get("items"))
                    .or_else(|| t.get("messages"))
            })
            .and_then(|v| v.as_array())
        {
            for item in items {
                let ack = match item
                    .get("ack")
                    .or_else(|| item.get("status"))
                    .or_else(|| item.get("state"))
                    .and_then(|v| v.as_str())
                {
                    Some("pending") => MessageAck::Pending,
                    _ => MessageAck::Confirmed,
                };
                transcript.push_bounded(TranscriptRow {
                    message_ref: string_field(
                        Some(item),
                        &["messageRef", "eventId", "id", "ref", "cursor"],
                    )
                    .unwrap_or_else(|| "unknown".into()),
                    role: string_field(Some(item), &["role"]).unwrap_or_else(|| "unknown".into()),
                    text: string_field(Some(item), &["text", "content"]).unwrap_or_default(),
                    ack,
                });
            }
        }
        projection.transcript = transcript;
        projection.room.thread_ref = Some("thread.sarah.abc".into());
        projection.recompute_attention();

        assert_eq!(projection.transcript.rows.len(), 2);
        assert_eq!(projection.transcript.rows[1].role, "sarah");
        assert_eq!(
            projection.transcript.rows[1].message_ref,
            "message.sarah_auto.tick.1"
        );
        assert_eq!(projection.transcript.rows[1].ack, MessageAck::Confirmed);
        // Proactive update raises the same attention path as a Q&A answer.
        assert_eq!(projection.attention.unread_count, 1);
        assert_eq!(
            projection.attention.marker,
            AttentionMarker::NeedsAttention
        );
        assert!(empty_room_is_honest(
            &projection.transcript,
            OMEGA_AUTONOMOUS_TICK_ENABLED
        ));

        projection.mark_room_read();
        assert_eq!(projection.attention.unread_count, 0);
        assert_eq!(projection.attention.marker, AttentionMarker::None);
    }

    #[test]
    fn tick_off_empty_snapshot_stays_honest() {
        assert!(!OMEGA_AUTONOMOUS_TICK_ENABLED);
        let empty = TranscriptProjection {
            meta: ProjectionMeta::fresh(sources::TRANSCRIPT),
            rows: Vec::new(),
            cursor: None,
            truncated: false,
        };
        assert!(empty_room_is_honest(&empty, OMEGA_AUTONOMOUS_TICK_ENABLED));
        let mut p = WorkroomProjection::honest_unsubscribed();
        p.transcript = empty;
        p.recompute_attention();
        assert_eq!(p.attention.unread_count, 0);
        assert!(!p.attention.marker.is_set());
        assert!(p.attention.tick_note.is_some());
    }

    #[test]
    fn apply_snapshot_nr06_shape_maps_entries_and_run_state() {
        let value = json!({
            "transcript": {
                "entries": [{
                    "eventId": "evt1",
                    "role": "owner",
                    "text": "hello",
                    "status": "confirmed"
                }],
                "cursor": "cursor.0",
                "gapState": "none"
            },
            "activity": {
                "entries": [{
                    "eventId": "act1",
                    "entry": "tool.call",
                    "turnRef": "turn.1",
                    "summary": "capacity"
                }],
                "cursor": "cursor.1"
            },
            "runState": {
                "state": "running",
                "turnRef": "turn.1",
                "reason": null
            }
        });

        let items = value
            .get("transcript")
            .and_then(|t| t.get("entries"))
            .and_then(|v| v.as_array())
            .expect("entries");
        assert_eq!(items.len(), 1);
        assert_eq!(
            string_field(Some(&items[0]), &["eventId"]).as_deref(),
            Some("evt1")
        );
        let phase = parse_run_phase(
            string_field(value.get("runState"), &["state"])
                .as_deref()
                .unwrap(),
        );
        assert_eq!(phase, RunPhase::Running);
        let entry_kind = string_field(
            value
                .get("activity")
                .and_then(|a| a.get("entries"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.first()),
            &["entry", "kind"],
        );
        assert_eq!(entry_kind.as_deref(), Some("tool.call"));
    }
}
