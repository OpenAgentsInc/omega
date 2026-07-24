//! Sarah workroom GPUI pane (`OMEGA-SW-03`).
//!
//! Dock panel + Agent menu entry + open / focus-composer / interrupt actions.
//! Renders the five §7 projections with source, freshness, and gap labels.
//! GPUI is projection-only: no durable thread, receipt, or turn store here.
//! Framed requests go to supervised `omega-effectd` only.

mod panel;
mod projections;

pub use panel::{init, SarahWorkroomPanel};
pub use projections::{
    ActivityProjection, ActivityRow, Freshness, GapState, InterruptIntentState, MessageAck,
    ProjectionMeta, ReceiptRow, ReceiptsProjection, RoomProjection, RunPhase, RunStateProjection,
    TranscriptProjection, TranscriptRow, WorkroomProjection, MAX_ACTIVITY_ROWS,
    MAX_RECEIPT_ROWS, MAX_TRANSCRIPT_ROWS, PANE_HEADER, sources,
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
