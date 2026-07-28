//! Sarah workroom GPUI pane (`OMEGA-SW-03` / `OMEGA-SW-04` / `OMEGA-SW-06` /
//! `SARAH-CW-08`).
//!
//! Dock panel + Agent menu entry + open / focus-composer / send / interrupt
//! actions. Renders the five §7 projections with source, freshness, and gap
//! labels. GPUI owns no durable thread, receipt, or turn store. Durable room
//! requests go to supervised `omega-effectd`; the owner-private room also has
//! an isolated, ephemeral managed voice connection to OpenAgents.
//!
//! - **OMEGA-SW-04**: interaction states (pending send, tool ladder, answer
//!   block, terminal reason, interrupt pending→applied). Transport is
//!   SARAH-NR-06. Honest liveness is the ordered tool ladder — never fake
//!   token streaming.
//! - **OMEGA-SW-06**: local unread count + attention marker; proactive tick
//!   turns render as ordinary transcript rows. Read state is local MVP only
//!   (NIP-RS is SARAH-NR-07). Autonomous tick stays default off.
//! - **SARAH-CW-08**: same pane hosts a second, isolated community room
//!   (membership, work units, experience rank). Not a second dock pane,
//!   composer, or receipt inspector. Two-room rule: never share membership
//!   or history with owner-private Sarah.

mod attention;
mod community;
mod full_auto;
mod interaction;
mod panel;
mod projections;
mod voice;

pub const PUBLIC_DEMO_ENV: &str = "OMEGA_PUBLIC_DEMO";

pub fn public_demo_mode() -> bool {
    std::env::var(PUBLIC_DEMO_ENV).as_deref() == Ok("1")
}

pub use attention::{
    AttentionMarker, LocalReadState, OMEGA_AUTONOMOUS_TICK_ENABLED, RoomAttention,
    SARAH_AUTONOMOUS_TICK_FLAG, autonomous_tick_enabled, compute_room_attention, count_unread,
    empty_room_is_honest, is_attention_role, proactive_turn_as_transcript_row,
    row_raises_attention, tick_off_honest_note,
};
pub use community::{
    AgentRosterRow, COMMUNITY_ROOM_HEADER, COMMUNITY_ROOM_SUBTITLE, CommunityRoomMeta,
    CommunityRoomProjection, ContentTrust, EXPERIENCE_LABEL, ExperienceAwardRow,
    ExperienceRankProjection, FORBIDDEN_EARNINGS_LABEL, MAX_MEMBER_ROWS, MAX_RECENT_AWARDS,
    MAX_WORK_UNIT_ROWS, MemberRosterRow, MembershipProjection, OWNER_PRIVATE_ROOM_HEADER, RoomKind,
    UNTRUSTED_CONTENT_BOUNDARY, V1_NO_PAY_FIRST_RUN_COPY, V1_NO_PAY_ROOM_DESCRIPTION,
    WorkUnitAcceptance, WorkUnitQuoteRow, WorkUnitRow, WorkUnitsProjection, WorkroomSurface,
    community_sources, copy_forbids_payment, label_implies_payment,
    quote_as_untrusted_member_content,
};
pub use full_auto::WorkroomFullAutoRun;
pub use interaction::{
    AnswerState, InteractionEvent, InteractionState, LocalPendingSend, TerminalOutcome,
    ToolLadderEntry, ToolLadderKind,
};
pub use panel::{SarahWorkroomPanel, init};
pub use projections::{
    ActivityProjection, ActivityRow, Freshness, GapState, InterruptIntentState, MAX_ACTIVITY_ROWS,
    MAX_RECEIPT_ROWS, MAX_TRANSCRIPT_ROWS, MessageAck, PANE_HEADER, ProjectionMeta, ReceiptRow,
    ReceiptsProjection, RoomProjection, RunPhase, RunStateProjection, TranscriptProjection,
    TranscriptRow, WorkroomProjection, sources,
};
pub use voice::{
    AgentThreadPresentation, ApprovedEditorAction, CommandConfirmation, CommandResultStatus,
    ManagedSarahVoiceClient, SARAH_AUDIO_SAMPLE_RATE, SARAH_VOICE_GATEWAY_PATH, SARAH_VOICE_MODEL,
    SARAH_VOICE_PROTOCOL_VERSION, SarahEditorCommand, SarahVoiceConnection, SarahVoiceControl,
    SarahVoiceEvent, SarahVoiceState, VoiceCommandRequest, VoiceCommandResult, VoiceParticipant,
    VoiceTranscriptItem, voice_gateway_url,
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

    #[test]
    fn public_demo_projection_is_explicitly_fictional_and_complete() {
        let projection = WorkroomProjection::public_demo();
        assert_eq!(
            projection.room.detail.as_deref(),
            Some("Fictional public demo data")
        );
        assert_eq!(projection.transcript.meta.gap, GapState::None);
        assert_eq!(projection.activity.meta.gap, GapState::None);
        assert_eq!(projection.run_state.phase, RunPhase::Finished);
        assert!(
            projection
                .transcript
                .rows
                .iter()
                .all(|row| row.ack == MessageAck::Confirmed)
        );
    }

    #[test]
    fn community_room_is_second_room_not_second_pane() {
        let surface = WorkroomSurface::honest_unsubscribed();
        assert_eq!(surface.active, RoomKind::OwnerPrivate);
        assert_eq!(CommunityRoomProjection::header(), COMMUNITY_ROOM_HEADER);
        assert_ne!(
            WorkroomProjection::header(),
            CommunityRoomProjection::header()
        );
        assert!(surface.rooms_are_isolated());
        assert!(surface.community.is_v1_compliant());
        // Single panel type — community is a room kind, not agent_computer / full_auto.
        assert_eq!(module_path!().contains("workroom_ui"), true);
    }
}
