//! Pane interaction states for send / stream / interrupt (`OMEGA-SW-04`).
//!
//! Record-agnostic pure transitions. Transport lives in `SARAH-NR-06`
//! (`omega_effectd` Sarah conversation client). GPUI only projects these
//! states; it holds no durable thread or turn store.
//!
//! Liveness honesty: the provider call is not a token stream
//! (`runSarahAgentTurn` sets `stream: false`). The ordered tool ladder is
//! the honest liveness signal. This module never invents partial tokens.

use crate::projections::{
    ActivityRow, Freshness, GapState, InterruptIntentState, MessageAck, ProjectionMeta, RunPhase,
    RunStateProjection, TranscriptRow, sources,
};

/// Kinds that form the ordered tool ladder (honest liveness signal).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolLadderKind {
    Call,
    Result,
    Error,
}

impl ToolLadderKind {
    pub fn as_event_kind(self) -> &'static str {
        match self {
            Self::Call => "tool.call",
            Self::Result => "tool.result",
            Self::Error => "tool.error",
        }
    }

    pub fn from_event_kind(kind: &str) -> Option<Self> {
        match kind {
            "tool.call" => Some(Self::Call),
            "tool.result" => Some(Self::Result),
            "tool.error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// One ordered tool-ladder step for the active or recent turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolLadderEntry {
    pub event_ref: String,
    pub kind: ToolLadderKind,
    pub tool_ref: Option<String>,
    pub summary: String,
    pub turn_ref: Option<String>,
}

impl ToolLadderEntry {
    pub fn to_activity_row(&self) -> ActivityRow {
        ActivityRow {
            event_ref: self.event_ref.clone(),
            kind: self.kind.as_event_kind().to_string(),
            summary: self.summary.clone(),
            turn_ref: self.turn_ref.clone(),
        }
    }
}

/// Answer projection. Full text arrives as one block; never token-streamed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnswerState {
    /// No answer for the active turn yet.
    None,
    /// `text.delta` landed as one complete block (`stream: false`).
    Text { text: String },
    /// `text.completed` landed; answer is final for this turn.
    Completed { text: String },
}

impl AnswerState {
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::Text { text } | Self::Completed { text } => Some(text.as_str()),
        }
    }

    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Text { .. } => "text",
            Self::Completed { .. } => "completed",
        }
    }
}

/// Terminal turn outcome with the **exact** reason from the record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalOutcome {
    None,
    Finished { reason: String },
    Interrupted { reason: String },
    Error { reason: String },
}

impl TerminalOutcome {
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::Finished { reason }
            | Self::Interrupted { reason }
            | Self::Error { reason } => Some(reason.as_str()),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Finished { .. } => "finished",
            Self::Interrupted { .. } => "interrupted",
            Self::Error { .. } => "error",
        }
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Record-agnostic events the pane applies in arrival order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InteractionEvent {
    /// Durable claim: turn is running.
    TurnQueued { turn_ref: String },
    TurnStarted { turn_ref: String },
    TurnRunning { turn_ref: String },
    ToolCall {
        event_ref: String,
        turn_ref: Option<String>,
        tool_ref: Option<String>,
        summary: String,
    },
    ToolResult {
        event_ref: String,
        turn_ref: Option<String>,
        tool_ref: Option<String>,
        summary: String,
    },
    ToolError {
        event_ref: String,
        turn_ref: Option<String>,
        tool_ref: Option<String>,
        summary: String,
    },
    /// Full answer block (`stream: false`). Not a token fragment.
    TextDelta {
        event_ref: String,
        turn_ref: Option<String>,
        text: String,
    },
    TextCompleted {
        event_ref: String,
        turn_ref: Option<String>,
    },
    TurnFinished {
        turn_ref: Option<String>,
        reason: String,
    },
    TurnInterrupted {
        turn_ref: Option<String>,
        reason: String,
    },
    /// Message confirmed on the durable record (may match a local pending row).
    MessageConfirmed {
        local_ref: Option<String>,
        message_ref: String,
        text: String,
        role: String,
    },
}

impl InteractionEvent {
    /// Parse a runtime event kind string into an interaction event.
    ///
    /// Unknown kinds return `None` (ignored, not invented).
    pub fn from_runtime_kind(
        kind: &str,
        event_ref: impl Into<String>,
        turn_ref: Option<String>,
        summary_or_text: impl Into<String>,
        tool_ref: Option<String>,
        reason: Option<String>,
    ) -> Option<Self> {
        let event_ref = event_ref.into();
        let summary_or_text = summary_or_text.into();
        match kind {
            "turn.queued" => Some(Self::TurnQueued {
                turn_ref: turn_ref.unwrap_or_else(|| "turn.unknown".into()),
            }),
            "turn.started" => Some(Self::TurnStarted {
                turn_ref: turn_ref.unwrap_or_else(|| "turn.unknown".into()),
            }),
            "turn.running" => Some(Self::TurnRunning {
                turn_ref: turn_ref.unwrap_or_else(|| "turn.unknown".into()),
            }),
            "tool.call" => Some(Self::ToolCall {
                event_ref,
                turn_ref,
                tool_ref,
                summary: summary_or_text,
            }),
            "tool.result" => Some(Self::ToolResult {
                event_ref,
                turn_ref,
                tool_ref,
                summary: summary_or_text,
            }),
            "tool.error" => Some(Self::ToolError {
                event_ref,
                turn_ref,
                tool_ref,
                summary: summary_or_text,
            }),
            "text.delta" => Some(Self::TextDelta {
                event_ref,
                turn_ref,
                text: summary_or_text,
            }),
            "text.completed" => Some(Self::TextCompleted {
                event_ref,
                turn_ref,
            }),
            "turn.finished" | "turn.completed" => Some(Self::TurnFinished {
                turn_ref,
                reason: reason.unwrap_or_else(|| summary_or_text),
            }),
            "turn.interrupted" => Some(Self::TurnInterrupted {
                turn_ref,
                reason: reason.unwrap_or_else(|| summary_or_text),
            }),
            _ => None,
        }
    }
}

/// Local optimistic send until the durable record confirms the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalPendingSend {
    pub local_ref: String,
    pub text: String,
    pub turn_ref: Option<String>,
}

/// Pure interaction projection for one workroom pane session.
///
/// Rebuilt from effectd snapshots and ordered events. Never durable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteractionState {
    next_local_seq: u64,
    /// Pending local sends not yet confirmed on the record.
    pub pending_sends: Vec<LocalPendingSend>,
    /// Ordered tool ladder for the active turn (honest liveness).
    pub tool_ladder: Vec<ToolLadderEntry>,
    pub answer: AnswerState,
    pub terminal: TerminalOutcome,
    pub run: RunStateProjection,
    /// When true, the pane must not render fake token motion.
    pub honest_liveness_only: bool,
    /// Sarah answer rows already projected into the transcript for this turn.
    answer_message_ref: Option<String>,
}

impl Default for InteractionState {
    fn default() -> Self {
        Self::new()
    }
}

impl InteractionState {
    pub fn new() -> Self {
        Self {
            next_local_seq: 1,
            pending_sends: Vec::new(),
            tool_ladder: Vec::new(),
            answer: AnswerState::None,
            terminal: TerminalOutcome::None,
            run: RunStateProjection {
                meta: ProjectionMeta::fresh(sources::RUN_STATE),
                phase: RunPhase::Idle,
                reason: None,
                turn_ref: None,
                interrupt_intent: InterruptIntentState::None,
            },
            honest_liveness_only: true,
            answer_message_ref: None,
        }
    }

    /// Start a local pending owner message. Returns the local ref and row.
    ///
    /// The row stays `MessageAck::Pending` until [`confirm_send`] or a matching
    /// `MessageConfirmed` event lands. Pending never renders as applied.
    pub fn begin_send(&mut self, text: impl Into<String>) -> (String, TranscriptRow) {
        let text = text.into();
        let local_ref = format!("local:{}", self.next_local_seq);
        self.next_local_seq = self.next_local_seq.saturating_add(1);
        self.pending_sends.push(LocalPendingSend {
            local_ref: local_ref.clone(),
            text: text.clone(),
            turn_ref: None,
        });
        let row = TranscriptRow {
            message_ref: local_ref.clone(),
            role: "owner".into(),
            text,
            ack: MessageAck::Pending,
        };
        (local_ref, row)
    }

    /// Record accepted the send. Upgrade pending → confirmed.
    ///
    /// Does **not** mark the turn running; that waits for the claim
    /// (`turn.started` / `turn.running`).
    pub fn confirm_send(
        &mut self,
        local_ref: &str,
        message_ref: impl Into<String>,
        turn_ref: Option<String>,
    ) -> Option<TranscriptRow> {
        let message_ref = message_ref.into();
        let idx = self
            .pending_sends
            .iter()
            .position(|p| p.local_ref == local_ref)?;
        let pending = self.pending_sends.remove(idx);
        if let Some(turn_ref) = turn_ref.clone() {
            self.run.turn_ref = Some(turn_ref);
        }
        // Accepted message only — claim may still be outstanding.
        if self.run.phase == RunPhase::Idle || self.run.phase == RunPhase::Unknown {
            self.run.phase = RunPhase::Queued;
            self.run.meta = ProjectionMeta::fresh(sources::RUN_STATE);
        }
        Some(TranscriptRow {
            message_ref,
            role: "owner".into(),
            text: pending.text,
            ack: MessageAck::Confirmed,
        })
    }

    /// Send failed before record confirmation. Drop the pending local row.
    pub fn fail_send(&mut self, local_ref: &str) -> bool {
        let before = self.pending_sends.len();
        self.pending_sends.retain(|p| p.local_ref != local_ref);
        before != self.pending_sends.len()
    }

    pub fn pending_send_count(&self) -> usize {
        self.pending_sends.len()
    }

    /// Typed interrupt intent. Pending until a terminal interrupted event.
    pub fn begin_interrupt(&mut self) {
        self.run.mark_interrupt_pending();
        // Law: pending never upgrades phase to Interrupted without terminal.
    }

    /// Apply one ordered record event. Returns transcript rows to merge (if any).
    pub fn apply_event(&mut self, event: InteractionEvent) -> Vec<TranscriptRow> {
        let mut transcript_rows = Vec::new();
        match event {
            InteractionEvent::TurnQueued { turn_ref } => {
                self.run.phase = RunPhase::Queued;
                self.run.turn_ref = Some(turn_ref);
                self.run.reason = None;
                self.run.meta = ProjectionMeta::fresh(sources::RUN_STATE);
                self.clear_turn_progress_if_new_turn();
            }
            InteractionEvent::TurnStarted { turn_ref } | InteractionEvent::TurnRunning { turn_ref } => {
                self.run.phase = RunPhase::Running;
                self.run.turn_ref = Some(turn_ref);
                self.run.reason = None;
                self.run.meta = ProjectionMeta::fresh(sources::RUN_STATE);
                // New claim: clear prior ladder/answer unless already terminal
                // for this same turn (idempotent replay).
                if self.terminal.is_terminal() {
                    // Restart / rehydrate mid-history: keep terminal if already set
                    // for a finished turn; a new start after terminal resets.
                    self.reset_open_turn_progress();
                }
            }
            InteractionEvent::ToolCall {
                event_ref,
                turn_ref,
                tool_ref,
                summary,
            } => {
                self.push_tool(ToolLadderKind::Call, event_ref, turn_ref, tool_ref, summary);
                self.ensure_running_from_ladder();
            }
            InteractionEvent::ToolResult {
                event_ref,
                turn_ref,
                tool_ref,
                summary,
            } => {
                self.push_tool(ToolLadderKind::Result, event_ref, turn_ref, tool_ref, summary);
                self.ensure_running_from_ladder();
            }
            InteractionEvent::ToolError {
                event_ref,
                turn_ref,
                tool_ref,
                summary,
            } => {
                self.push_tool(ToolLadderKind::Error, event_ref, turn_ref, tool_ref, summary);
                self.ensure_running_from_ladder();
            }
            InteractionEvent::TextDelta {
                event_ref,
                turn_ref,
                text,
            } => {
                // One block only — never append token fragments.
                self.answer = AnswerState::Text { text: text.clone() };
                if let Some(tr) = turn_ref {
                    self.run.turn_ref = Some(tr);
                }
                let msg_ref = self
                    .answer_message_ref
                    .clone()
                    .unwrap_or_else(|| format!("answer:{}", event_ref));
                self.answer_message_ref = Some(msg_ref.clone());
                transcript_rows.push(TranscriptRow {
                    message_ref: msg_ref,
                    role: "sarah".into(),
                    text,
                    ack: MessageAck::Confirmed,
                });
            }
            InteractionEvent::TextCompleted { event_ref, turn_ref } => {
                let text = self
                    .answer
                    .text()
                    .map(str::to_string)
                    .unwrap_or_default();
                self.answer = AnswerState::Completed { text: text.clone() };
                if let Some(tr) = turn_ref {
                    self.run.turn_ref = Some(tr);
                }
                if !text.is_empty() {
                    let msg_ref = self
                        .answer_message_ref
                        .clone()
                        .unwrap_or_else(|| format!("answer:{}", event_ref));
                    self.answer_message_ref = Some(msg_ref.clone());
                    transcript_rows.push(TranscriptRow {
                        message_ref: msg_ref,
                        role: "sarah".into(),
                        text,
                        ack: MessageAck::Confirmed,
                    });
                }
            }
            InteractionEvent::TurnFinished { turn_ref, reason } => {
                self.run.phase = RunPhase::Finished;
                if let Some(tr) = turn_ref {
                    self.run.turn_ref = Some(tr);
                }
                self.run.reason = Some(reason.clone());
                self.run.meta.freshness = Freshness::Fresh;
                self.run.meta.gap = GapState::None;
                self.terminal = TerminalOutcome::Finished { reason };
                // Finished is not interrupt-applied.
            }
            InteractionEvent::TurnInterrupted { turn_ref, reason } => {
                self.run.apply_terminal_interrupted(turn_ref, Some(reason.clone()));
                self.terminal = TerminalOutcome::Interrupted { reason };
            }
            InteractionEvent::MessageConfirmed {
                local_ref,
                message_ref,
                text,
                role,
            } => {
                if let Some(local_ref) = local_ref.as_deref() {
                    if let Some(row) = self.confirm_send(local_ref, message_ref.clone(), None) {
                        transcript_rows.push(row);
                        return transcript_rows;
                    }
                }
                // Record-side confirm without local pending (other reader view).
                self.pending_sends
                    .retain(|p| p.local_ref != message_ref && p.text != text);
                transcript_rows.push(TranscriptRow {
                    message_ref,
                    role,
                    text,
                    ack: MessageAck::Confirmed,
                });
            }
        }
        transcript_rows
    }

    fn push_tool(
        &mut self,
        kind: ToolLadderKind,
        event_ref: String,
        turn_ref: Option<String>,
        tool_ref: Option<String>,
        summary: String,
    ) {
        // De-dupe by event_ref for idempotent rehydrate.
        if self.tool_ladder.iter().any(|e| e.event_ref == event_ref) {
            return;
        }
        self.tool_ladder.push(ToolLadderEntry {
            event_ref,
            kind,
            tool_ref,
            summary,
            turn_ref,
        });
    }

    fn ensure_running_from_ladder(&mut self) {
        if matches!(
            self.run.phase,
            RunPhase::Idle | RunPhase::Queued | RunPhase::Unknown
        ) && !self.terminal.is_terminal()
        {
            self.run.phase = RunPhase::Running;
            self.run.meta = ProjectionMeta::fresh(sources::RUN_STATE);
        }
    }

    fn clear_turn_progress_if_new_turn(&mut self) {
        if self.terminal.is_terminal() {
            self.reset_open_turn_progress();
        }
    }

    fn reset_open_turn_progress(&mut self) {
        self.tool_ladder.clear();
        self.answer = AnswerState::None;
        self.terminal = TerminalOutcome::None;
        self.answer_message_ref = None;
        if self.run.interrupt_intent == InterruptIntentState::Applied {
            self.run.interrupt_intent = InterruptIntentState::None;
        }
    }

    /// Activity rows for the pane, in arrival order (tool ladder).
    pub fn activity_rows(&self) -> Vec<ActivityRow> {
        self.tool_ladder
            .iter()
            .map(ToolLadderEntry::to_activity_row)
            .collect()
    }

    /// True when the only liveness signal is the tool ladder (no fake tokens).
    pub fn uses_honest_liveness(&self) -> bool {
        self.honest_liveness_only
    }

    /// Rehydrate run phase from a snapshot field without inventing progress.
    pub fn apply_snapshot_run(
        &mut self,
        phase: RunPhase,
        turn_ref: Option<String>,
        reason: Option<String>,
    ) {
        // Preserve pending interrupt intent across snapshot refresh.
        let interrupt = self.run.interrupt_intent;
        self.run.phase = phase;
        self.run.turn_ref = turn_ref;
        self.run.reason = reason.clone();
        self.run.meta = ProjectionMeta::fresh(sources::RUN_STATE);
        self.run.interrupt_intent = interrupt;

        if phase == RunPhase::Interrupted {
            if interrupt == InterruptIntentState::Pending
                || interrupt == InterruptIntentState::Applied
            {
                self.run.interrupt_intent = InterruptIntentState::Applied;
            }
            if let Some(reason) = reason {
                self.terminal = TerminalOutcome::Interrupted { reason };
            }
        } else if phase == RunPhase::Finished {
            if let Some(reason) = reason {
                self.terminal = TerminalOutcome::Finished { reason };
            }
        }
    }

    /// Pending interrupt must not look applied.
    pub fn interrupt_not_falsely_applied(&self) -> bool {
        self.run.interrupt_not_falsely_applied()
            && !(self.run.interrupt_intent == InterruptIntentState::Pending
                && matches!(self.terminal, TerminalOutcome::Interrupted { .. }))
    }

    /// Status line for the pane chrome.
    pub fn status_line(&self) -> String {
        let phase = self.run.phase.label();
        let interrupt = self.run.interrupt_intent.label();
        let answer = self.answer.label();
        let terminal = self.terminal.label();
        let tools = self.tool_ladder.len();
        let pending = self.pending_sends.len();
        let mut line = format!(
            "phase={phase} · interrupt={interrupt} · answer={answer} · terminal={terminal} · tools={tools} · pending_sends={pending}"
        );
        if let Some(reason) = self.terminal.reason().or(self.run.reason.as_deref()) {
            line.push_str(" · reason=");
            line.push_str(reason);
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_local_message_until_record_confirms() {
        let mut s = InteractionState::new();
        let (local_ref, row) = s.begin_send("hello Sarah");
        assert_eq!(row.ack, MessageAck::Pending);
        assert!(!row.ack.renders_as_applied());
        assert_eq!(s.pending_send_count(), 1);
        assert!(local_ref.starts_with("local:"));

        let confirmed = s
            .confirm_send(&local_ref, "msg.1", Some("turn.1".into()))
            .expect("confirm");
        assert_eq!(confirmed.ack, MessageAck::Confirmed);
        assert_eq!(confirmed.message_ref, "msg.1");
        assert_eq!(confirmed.text, "hello Sarah");
        assert_eq!(s.pending_send_count(), 0);
        // Claim not yet landed — queued, not running.
        assert_eq!(s.run.phase, RunPhase::Queued);
        assert_eq!(s.run.turn_ref.as_deref(), Some("turn.1"));
    }

    #[test]
    fn turn_running_after_claim() {
        let mut s = InteractionState::new();
        let (local_ref, _) = s.begin_send("plan next packet");
        s.confirm_send(&local_ref, "msg.1", Some("turn.1".into()));
        assert_ne!(s.run.phase, RunPhase::Running);

        s.apply_event(InteractionEvent::TurnStarted {
            turn_ref: "turn.1".into(),
        });
        assert_eq!(s.run.phase, RunPhase::Running);
        assert_eq!(s.run.turn_ref.as_deref(), Some("turn.1"));
        assert!(!s.terminal.is_terminal());
    }

    #[test]
    fn ordered_tool_ladder_is_liveness_signal() {
        let mut s = InteractionState::new();
        s.apply_event(InteractionEvent::TurnStarted {
            turn_ref: "turn.1".into(),
        });
        s.apply_event(InteractionEvent::ToolCall {
            event_ref: "e1".into(),
            turn_ref: Some("turn.1".into()),
            tool_ref: Some("codex_workers_capacity".into()),
            summary: "capacity".into(),
        });
        s.apply_event(InteractionEvent::ToolResult {
            event_ref: "e2".into(),
            turn_ref: Some("turn.1".into()),
            tool_ref: Some("codex_workers_capacity".into()),
            summary: "ready=2".into(),
        });
        s.apply_event(InteractionEvent::ToolError {
            event_ref: "e3".into(),
            turn_ref: Some("turn.1".into()),
            tool_ref: Some("full_auto_control".into()),
            summary: "refused: authority".into(),
        });

        let kinds: Vec<_> = s
            .tool_ladder
            .iter()
            .map(|e| e.kind.as_event_kind())
            .collect();
        assert_eq!(kinds, ["tool.call", "tool.result", "tool.error"]);
        assert!(s.uses_honest_liveness());
        // No answer yet — liveness is the ladder, not token motion.
        assert_eq!(s.answer, AnswerState::None);
        let activity = s.activity_rows();
        assert_eq!(activity.len(), 3);
        assert_eq!(activity[0].kind, "tool.call");
        assert_eq!(activity[2].kind, "tool.error");
    }

    #[test]
    fn answer_arrives_as_one_block_then_completes() {
        let mut s = InteractionState::new();
        s.apply_event(InteractionEvent::TurnStarted {
            turn_ref: "turn.1".into(),
        });
        // Full block — not per-token deltas.
        let rows = s.apply_event(InteractionEvent::TextDelta {
            event_ref: "td1".into(),
            turn_ref: Some("turn.1".into()),
            text: "Here is the plan.".into(),
        });
        assert_eq!(s.answer.label(), "text");
        assert_eq!(s.answer.text(), Some("Here is the plan."));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].role, "sarah");
        assert_eq!(rows[0].ack, MessageAck::Confirmed);

        s.apply_event(InteractionEvent::TextCompleted {
            event_ref: "tc1".into(),
            turn_ref: Some("turn.1".into()),
        });
        assert!(s.answer.is_completed());
        assert_eq!(s.answer.text(), Some("Here is the plan."));
    }

    #[test]
    fn terminal_outcome_carries_exact_reason() {
        let mut s = InteractionState::new();
        s.apply_event(InteractionEvent::TurnStarted {
            turn_ref: "turn.1".into(),
        });
        s.apply_event(InteractionEvent::TurnFinished {
            turn_ref: Some("turn.1".into()),
            reason: "stop".into(),
        });
        assert_eq!(s.run.phase, RunPhase::Finished);
        assert_eq!(s.terminal.reason(), Some("stop"));
        assert_eq!(s.run.reason.as_deref(), Some("stop"));
        assert_eq!(s.terminal.label(), "finished");
    }

    #[test]
    fn interrupt_pending_until_terminal_applied() {
        let mut s = InteractionState::new();
        s.apply_event(InteractionEvent::TurnStarted {
            turn_ref: "turn.1".into(),
        });
        s.begin_interrupt();
        assert_eq!(s.run.interrupt_intent, InterruptIntentState::Pending);
        assert_eq!(s.run.phase, RunPhase::Running);
        assert!(s.interrupt_not_falsely_applied());
        assert_ne!(s.run.interrupt_intent, InterruptIntentState::Applied);

        s.apply_event(InteractionEvent::TurnInterrupted {
            turn_ref: Some("turn.1".into()),
            reason: "owner_interrupt".into(),
        });
        assert_eq!(s.run.interrupt_intent, InterruptIntentState::Applied);
        assert_eq!(s.run.phase, RunPhase::Interrupted);
        assert_eq!(s.terminal.reason(), Some("owner_interrupt"));
        assert!(s.interrupt_not_falsely_applied());
    }

    #[test]
    fn full_send_to_finish_state_machine() {
        let mut s = InteractionState::new();
        let (local_ref, pending_row) = s.begin_send("What is capacity?");
        assert_eq!(pending_row.ack, MessageAck::Pending);

        let confirmed = s
            .confirm_send(&local_ref, "msg.9", Some("turn.9".into()))
            .unwrap();
        assert_eq!(confirmed.ack, MessageAck::Confirmed);
        assert_eq!(s.run.phase, RunPhase::Queued);

        s.apply_event(InteractionEvent::TurnStarted {
            turn_ref: "turn.9".into(),
        });
        assert_eq!(s.run.phase, RunPhase::Running);

        s.apply_event(InteractionEvent::ToolCall {
            event_ref: "t1".into(),
            turn_ref: Some("turn.9".into()),
            tool_ref: Some("codex_workers_capacity".into()),
            summary: "check".into(),
        });
        s.apply_event(InteractionEvent::ToolResult {
            event_ref: "t2".into(),
            turn_ref: Some("turn.9".into()),
            tool_ref: Some("codex_workers_capacity".into()),
            summary: "ok".into(),
        });
        s.apply_event(InteractionEvent::TextDelta {
            event_ref: "a1".into(),
            turn_ref: Some("turn.9".into()),
            text: "Two workers ready.".into(),
        });
        s.apply_event(InteractionEvent::TextCompleted {
            event_ref: "a2".into(),
            turn_ref: Some("turn.9".into()),
        });
        s.apply_event(InteractionEvent::TurnFinished {
            turn_ref: Some("turn.9".into()),
            reason: "stop".into(),
        });

        assert_eq!(s.tool_ladder.len(), 2);
        assert!(s.answer.is_completed());
        assert_eq!(s.terminal.reason(), Some("stop"));
        assert_eq!(s.run.phase, RunPhase::Finished);
        assert!(s.uses_honest_liveness());
        assert!(s.status_line().contains("reason=stop"));
    }

    #[test]
    fn fail_send_drops_pending() {
        let mut s = InteractionState::new();
        let (local_ref, _) = s.begin_send("will fail");
        assert!(s.fail_send(&local_ref));
        assert_eq!(s.pending_send_count(), 0);
        assert!(s.confirm_send(&local_ref, "msg.x", None).is_none());
    }

    #[test]
    fn from_runtime_kind_parses_ladder_and_terminals() {
        let call = InteractionEvent::from_runtime_kind(
            "tool.call",
            "e1",
            Some("turn.1".into()),
            "cap",
            Some("tool.a".into()),
            None,
        );
        assert!(matches!(call, Some(InteractionEvent::ToolCall { .. })));

        let fin = InteractionEvent::from_runtime_kind(
            "turn.finished",
            "e2",
            Some("turn.1".into()),
            "",
            None,
            Some("error".into()),
        );
        match fin {
            Some(InteractionEvent::TurnFinished { reason, .. }) => {
                assert_eq!(reason, "error");
            }
            other => panic!("unexpected {other:?}"),
        }

        assert!(InteractionEvent::from_runtime_kind(
            "usage.recorded",
            "e3",
            None,
            "",
            None,
            None
        )
        .is_none());
    }

    #[test]
    fn tool_event_dedupes_by_event_ref() {
        let mut s = InteractionState::new();
        s.apply_event(InteractionEvent::ToolCall {
            event_ref: "same".into(),
            turn_ref: None,
            tool_ref: None,
            summary: "first".into(),
        });
        s.apply_event(InteractionEvent::ToolCall {
            event_ref: "same".into(),
            turn_ref: None,
            tool_ref: None,
            summary: "replay".into(),
        });
        assert_eq!(s.tool_ladder.len(), 1);
        assert_eq!(s.tool_ladder[0].summary, "first");
    }

    #[test]
    fn snapshot_preserves_pending_interrupt() {
        let mut s = InteractionState::new();
        s.apply_event(InteractionEvent::TurnStarted {
            turn_ref: "turn.1".into(),
        });
        s.begin_interrupt();
        s.apply_snapshot_run(RunPhase::Running, Some("turn.1".into()), None);
        assert_eq!(s.run.interrupt_intent, InterruptIntentState::Pending);

        s.apply_snapshot_run(
            RunPhase::Interrupted,
            Some("turn.1".into()),
            Some("owner_interrupt".into()),
        );
        assert_eq!(s.run.interrupt_intent, InterruptIntentState::Applied);
        assert_eq!(s.terminal.reason(), Some("owner_interrupt"));
    }

    #[test]
    fn restart_mid_turn_shows_one_terminal_never_two_answers() {
        // Rehydrate path: apply claim + one answer + terminal once.
        let mut s = InteractionState::new();
        s.apply_event(InteractionEvent::TurnStarted {
            turn_ref: "turn.1".into(),
        });
        s.apply_event(InteractionEvent::TextDelta {
            event_ref: "a1".into(),
            turn_ref: Some("turn.1".into()),
            text: "single answer".into(),
        });
        s.apply_event(InteractionEvent::TextCompleted {
            event_ref: "a2".into(),
            turn_ref: Some("turn.1".into()),
        });
        s.apply_event(InteractionEvent::TurnFinished {
            turn_ref: Some("turn.1".into()),
            reason: "stop".into(),
        });
        // Idempotent text.completed does not invent a second answer body.
        assert_eq!(s.answer.text(), Some("single answer"));
        assert!(s.answer.is_completed());
        assert_eq!(s.terminal.label(), "finished");
    }
}
