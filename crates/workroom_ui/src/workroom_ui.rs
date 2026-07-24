//! Sarah workroom GPUI pane (`OMEGA-SW-03` / `OMEGA-SW-04` / `OMEGA-SW-06`).
//!
//! Dock panel + Agent menu entry + open / focus-composer / send / interrupt
//! actions. Renders the five §7 projections with source, freshness, and gap
//! labels. GPUI is projection-only: no durable thread, receipt, or turn store
//! here. Framed requests go to supervised `omega-effectd` only.
//!
//! - **OMEGA-SW-04**: interaction states (pending send, tool ladder, answer
//!   block, terminal reason, interrupt pending→applied). Transport is
//!   SARAH-NR-06. Honest liveness is the ordered tool ladder — never fake
//!   token streaming.
//! - **OMEGA-SW-06**: local unread count + attention marker; proactive tick
//!   turns render as ordinary transcript rows. Read state is local MVP only
//!   (NIP-RS is SARAH-NR-07). Autonomous tick stays default off.

mod attention;
mod interaction;
mod panel;
mod projections;

pub use attention::{
    autonomous_tick_enabled, compute_room_attention, count_unread, empty_room_is_honest,
    is_attention_role, proactive_turn_as_transcript_row, row_raises_attention,
    tick_off_honest_note, AttentionMarker, LocalReadState, RoomAttention,
    OMEGA_AUTONOMOUS_TICK_ENABLED, SARAH_AUTONOMOUS_TICK_FLAG,
};
pub use interaction::{
    AnswerState, InteractionEvent, InteractionState, LocalPendingSend, TerminalOutcome,
    ToolLadderEntry, ToolLadderKind,
};
pub use panel::{init, SarahWorkroomPanel};
pub use projections::{
    sources, ActivityProjection, ActivityRow, Freshness, GapState, InterruptIntentState,
    MessageAck, ProjectionMeta, ReceiptRow, ReceiptsProjection, RoomProjection, RunPhase,
    RunStateProjection, TranscriptProjection, TranscriptRow, WorkroomProjection,
    MAX_ACTIVITY_ROWS, MAX_RECEIPT_ROWS, MAX_TRANSCRIPT_ROWS, PANE_HEADER,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workroom_surface_is_not_full_auto_or_agent_computer() {
        let names = module_path!();
        assert!(names.contains("workroom_ui"));
        assert!(!names.contains("full_auto"));
        assert!(!names.contains("agent_computer"));
        assert_eq!(PANE_HEADER, "Sarah");
        assert_eq!(WorkroomProjection::header(), "Sarah");
    }

    #[test]
    fn five_projections_start_honest_not_empty_success() {
        let p = WorkroomProjection::honest_unsubscribed();
        assert!(p.room.is_honest_missing());
        assert_eq!(p.transcript.meta.gap, GapState::Unavailable);
        assert_eq!(p.activity.meta.gap, GapState::Unavailable);
        assert_eq!(p.receipts.meta.gap, GapState::Unavailable);
        assert_eq!(p.run_state.meta.gap, GapState::Unavailable);
        assert_eq!(p.run_state.interrupt_intent, InterruptIntentState::None);
    }
}
