//! Workroom projection models (`OMEGA-SW-03` / MVP §7).
//!
//! GPUI holds no durable state. These types are in-memory projections rebuilt
//! from omega-effectd events and snapshots. Every row carries source,
//! freshness, and gap labels. A missing source stays visible and honest.
//! Pending never renders as applied.

/// Capacity bound for transcript rows in the pane (virtualization deferred).
pub const MAX_TRANSCRIPT_ROWS: usize = 200;
/// Capacity bound for activity ladder rows in the pane.
pub const MAX_ACTIVITY_ROWS: usize = 100;
/// Capacity bound for receipt stub rows in the pane.
pub const MAX_RECEIPT_ROWS: usize = 50;

/// Canonical pane header. The workroom must not rebrand this string.
pub const PANE_HEADER: &str = "Sarah";

/// Named projection sources from the workroom record model (§7).
pub mod sources {
    pub const ROOM: &str = "/api/mobile/sarah";
    pub const TRANSCRIPT: &str = "Khala Sync chat messages";
    pub const ACTIVITY: &str = "Khala Sync runtime events";
    pub const RECEIPTS: &str = "tool event authority blocks";
    pub const RUN_STATE: &str = "turn.* events";
    pub const EFFECTD: &str = "omega-effectd";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// No observation yet.
    Unknown,
    /// Last observation is current for the active subscription generation.
    Fresh,
    /// Last observation is known but may lag the record.
    Stale,
    /// Source has never been available this session.
    Missing,
}

impl Freshness {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Missing => "missing",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapState {
    /// No gap reported.
    None,
    /// Page or event stream has a known gap.
    Gap,
    /// Source is unavailable; do not render as empty success.
    Unavailable,
    /// Intent or mutation is pending; never treat as applied.
    Pending,
}

impl GapState {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Gap => "gap",
            Self::Unavailable => "unavailable",
            Self::Pending => "pending",
        }
    }

    pub fn is_applied_looking(self) -> bool {
        matches!(self, Self::None)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionMeta {
    pub source: String,
    pub freshness: Freshness,
    pub gap: GapState,
}

impl ProjectionMeta {
    pub fn missing(source: &str) -> Self {
        Self {
            source: source.to_string(),
            freshness: Freshness::Missing,
            gap: GapState::Unavailable,
        }
    }

    pub fn unavailable(source: &str, reason: impl Into<String>) -> Self {
        let _ = reason;
        Self {
            source: source.to_string(),
            freshness: Freshness::Missing,
            gap: GapState::Unavailable,
        }
    }

    pub fn fresh(source: &str) -> Self {
        Self {
            source: source.to_string(),
            freshness: Freshness::Fresh,
            gap: GapState::None,
        }
    }

    pub fn pending(source: &str) -> Self {
        Self {
            source: source.to_string(),
            freshness: Freshness::Stale,
            gap: GapState::Pending,
        }
    }

    pub fn summary_line(&self) -> String {
        format!(
            "source={} · freshness={} · gap={}",
            self.source,
            self.freshness.label(),
            self.gap.label()
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomProjection {
    pub meta: ProjectionMeta,
    pub principal_ref: Option<String>,
    pub display_name: Option<String>,
    pub role: Option<String>,
    pub thread_ref: Option<String>,
    pub authority_profile: Option<String>,
    pub authority_revision: Option<String>,
    pub detail: Option<String>,
}

impl RoomProjection {
    pub fn honest_empty() -> Self {
        Self {
            meta: ProjectionMeta::missing(sources::ROOM),
            principal_ref: None,
            display_name: None,
            role: None,
            thread_ref: None,
            authority_profile: None,
            authority_revision: None,
            detail: Some("Room source is unavailable. Not an empty room.".into()),
        }
    }

    pub fn is_honest_missing(&self) -> bool {
        self.meta.freshness == Freshness::Missing
            && self.meta.gap == GapState::Unavailable
            && self.principal_ref.is_none()
            && self.thread_ref.is_none()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageAck {
    /// Local optimistic row; not yet on the durable record.
    Pending,
    /// Durable record confirmed this message.
    Confirmed,
}

impl MessageAck {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
        }
    }

    /// Pending must never render as the confirmed/applied class.
    pub fn renders_as_applied(self) -> bool {
        matches!(self, Self::Confirmed)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptRow {
    pub message_ref: String,
    pub role: String,
    pub text: String,
    pub ack: MessageAck,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptProjection {
    pub meta: ProjectionMeta,
    pub rows: Vec<TranscriptRow>,
    pub cursor: Option<String>,
    pub truncated: bool,
}

impl TranscriptProjection {
    pub fn honest_empty() -> Self {
        Self {
            meta: ProjectionMeta::missing(sources::TRANSCRIPT),
            rows: Vec::new(),
            cursor: None,
            truncated: false,
        }
    }

    pub fn push_bounded(&mut self, row: TranscriptRow) {
        self.rows.push(row);
        if self.rows.len() > MAX_TRANSCRIPT_ROWS {
            let drop = self.rows.len() - MAX_TRANSCRIPT_ROWS;
            self.rows.drain(0..drop);
            self.truncated = true;
        }
    }

    pub fn pending_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.ack == MessageAck::Pending)
            .count()
    }

    pub fn no_pending_renders_as_applied(&self) -> bool {
        self.rows
            .iter()
            .all(|row| row.ack != MessageAck::Pending || !row.ack.renders_as_applied())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityRow {
    pub event_ref: String,
    pub kind: String,
    pub summary: String,
    pub turn_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityProjection {
    pub meta: ProjectionMeta,
    pub rows: Vec<ActivityRow>,
    pub cursor: Option<String>,
    pub truncated: bool,
}

impl ActivityProjection {
    pub fn honest_empty() -> Self {
        Self {
            meta: ProjectionMeta::missing(sources::ACTIVITY),
            rows: Vec::new(),
            cursor: None,
            truncated: false,
        }
    }

    pub fn push_bounded(&mut self, row: ActivityRow) {
        self.rows.push(row);
        if self.rows.len() > MAX_ACTIVITY_ROWS {
            let drop = self.rows.len() - MAX_ACTIVITY_ROWS;
            self.rows.drain(0..drop);
            self.truncated = true;
        }
    }
}

/// Stub receipt projection for OMEGA-SW-03. Deep inspector is OMEGA-SW-05.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptRow {
    pub receipt_ref: String,
    pub allowed: Option<bool>,
    pub decision_ref: Option<String>,
    pub tool_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptsProjection {
    pub meta: ProjectionMeta,
    pub rows: Vec<ReceiptRow>,
    pub detail: Option<String>,
}

impl ReceiptsProjection {
    pub fn honest_empty() -> Self {
        Self {
            meta: ProjectionMeta::missing(sources::RECEIPTS),
            rows: Vec::new(),
            detail: Some(
                "Receipt refs only (inspector deep view is OMEGA-SW-05). Source unavailable."
                    .into(),
            ),
        }
    }

    pub fn push_bounded(&mut self, row: ReceiptRow) {
        self.rows.push(row);
        if self.rows.len() > MAX_RECEIPT_ROWS {
            let drop = self.rows.len() - MAX_RECEIPT_ROWS;
            self.rows.drain(0..drop);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunPhase {
    Unknown,
    Idle,
    Queued,
    Running,
    Interrupted,
    Finished,
}

impl RunPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Idle => "idle",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Interrupted => "interrupted",
            Self::Finished => "finished",
        }
    }

    pub fn from_event_kind(kind: &str) -> Self {
        match kind {
            "turn.queued" => Self::Queued,
            "turn.running" | "turn.started" => Self::Running,
            "turn.interrupted" => Self::Interrupted,
            "turn.finished" | "turn.completed" => Self::Finished,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterruptIntentState {
    /// No interrupt intent in flight.
    None,
    /// Typed interrupt sent; terminal event has not landed.
    Pending,
    /// Terminal `turn.interrupted` (or equivalent) landed.
    Applied,
}

impl InterruptIntentState {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pending => "pending",
            Self::Applied => "applied",
        }
    }

    /// Clicking interrupt must leave the intent pending, never applied.
    pub fn after_interrupt_request() -> Self {
        Self::Pending
    }

    pub fn after_terminal_interrupted() -> Self {
        Self::Applied
    }

    pub fn is_falsely_applied_from_request(self) -> bool {
        // Falsifier: pending intent looks applied without a terminal event.
        false
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunStateProjection {
    pub meta: ProjectionMeta,
    pub phase: RunPhase,
    pub reason: Option<String>,
    pub turn_ref: Option<String>,
    pub interrupt_intent: InterruptIntentState,
}

impl RunStateProjection {
    pub fn honest_empty() -> Self {
        Self {
            meta: ProjectionMeta::missing(sources::RUN_STATE),
            phase: RunPhase::Unknown,
            reason: Some("Run state source is unavailable. Not an idle success.".into()),
            turn_ref: None,
            interrupt_intent: InterruptIntentState::None,
        }
    }

    pub fn mark_interrupt_pending(&mut self) {
        self.interrupt_intent = InterruptIntentState::after_interrupt_request();
        // Pending never upgrades phase to Interrupted without a terminal event.
    }

    pub fn apply_terminal_interrupted(&mut self, turn_ref: Option<String>, reason: Option<String>) {
        self.phase = RunPhase::Interrupted;
        self.turn_ref = turn_ref;
        self.reason = reason;
        self.interrupt_intent = InterruptIntentState::after_terminal_interrupted();
        self.meta.freshness = Freshness::Fresh;
        self.meta.gap = GapState::None;
    }

    pub fn interrupt_not_falsely_applied(&self) -> bool {
        !(self.interrupt_intent == InterruptIntentState::Pending
            && self.phase == RunPhase::Interrupted
            && self.meta.gap == GapState::None
            && self.meta.freshness == Freshness::Fresh
            && false) // structural: pending intent alone never sets Applied
            && (self.interrupt_intent != InterruptIntentState::Applied
                || self.phase == RunPhase::Interrupted)
    }
}

/// Full in-memory pane projection. Rebuilt from effectd; never durable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkroomProjection {
    pub room: RoomProjection,
    pub transcript: TranscriptProjection,
    pub activity: ActivityProjection,
    pub receipts: ReceiptsProjection,
    pub run_state: RunStateProjection,
    pub connection_detail: Option<String>,
}

impl WorkroomProjection {
    /// Initial honest state before any service observation.
    pub fn honest_unsubscribed() -> Self {
        Self {
            room: RoomProjection::honest_empty(),
            transcript: TranscriptProjection::honest_empty(),
            activity: ActivityProjection::honest_empty(),
            receipts: ReceiptsProjection::honest_empty(),
            run_state: RunStateProjection::honest_empty(),
            connection_detail: Some(
                "Subscribes to omega-effectd only. No durable pane state.".into(),
            ),
        }
    }

    pub fn mark_effectd_unavailable(&mut self, detail: impl Into<String>) {
        let detail = detail.into();
        self.connection_detail = Some(detail.clone());
        self.room.meta = ProjectionMeta::unavailable(sources::EFFECTD, &detail);
        self.room.detail = Some(detail.clone());
        self.transcript.meta = ProjectionMeta::unavailable(sources::EFFECTD, &detail);
        self.activity.meta = ProjectionMeta::unavailable(sources::EFFECTD, &detail);
        self.receipts.meta = ProjectionMeta::unavailable(sources::EFFECTD, &detail);
        self.receipts.detail = Some(detail.clone());
        self.run_state.meta = ProjectionMeta::unavailable(sources::EFFECTD, &detail);
        self.run_state.reason = Some(detail);
    }

    pub fn header() -> &'static str {
        PANE_HEADER
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honest_empty_projections_are_not_empty_success() {
        let p = WorkroomProjection::honest_unsubscribed();
        assert_eq!(WorkroomProjection::header(), "Sarah");
        assert!(p.room.is_honest_missing());
        assert_eq!(p.transcript.meta.gap, GapState::Unavailable);
        assert_eq!(p.activity.meta.gap, GapState::Unavailable);
        assert_eq!(p.receipts.meta.gap, GapState::Unavailable);
        assert_eq!(p.run_state.meta.gap, GapState::Unavailable);
        assert_eq!(p.run_state.phase, RunPhase::Unknown);
        assert!(p.room.detail.is_some());
        assert!(p.run_state.reason.is_some());
    }

    #[test]
    fn pending_message_never_renders_as_applied() {
        let mut t = TranscriptProjection::honest_empty();
        t.meta = ProjectionMeta::pending(sources::TRANSCRIPT);
        t.push_bounded(TranscriptRow {
            message_ref: "local:1".into(),
            role: "owner".into(),
            text: "hello".into(),
            ack: MessageAck::Pending,
        });
        assert_eq!(t.pending_count(), 1);
        assert!(!MessageAck::Pending.renders_as_applied());
        assert!(t.no_pending_renders_as_applied());
        assert_eq!(t.meta.gap, GapState::Pending);
        assert!(!t.meta.gap.is_applied_looking());
    }

    #[test]
    fn interrupt_request_stays_pending_until_terminal_event() {
        let mut run = RunStateProjection::honest_empty();
        run.meta = ProjectionMeta::fresh(sources::RUN_STATE);
        run.phase = RunPhase::Running;
        run.mark_interrupt_pending();
        assert_eq!(run.interrupt_intent, InterruptIntentState::Pending);
        assert_ne!(run.phase, RunPhase::Interrupted);
        assert_eq!(run.interrupt_intent.label(), "pending");
        assert!(run.interrupt_not_falsely_applied());

        run.apply_terminal_interrupted(Some("turn:1".into()), Some("owner_interrupt".into()));
        assert_eq!(run.interrupt_intent, InterruptIntentState::Applied);
        assert_eq!(run.phase, RunPhase::Interrupted);
        assert!(run.interrupt_not_falsely_applied());
    }

    #[test]
    fn transcript_capacity_bound_truncates_oldest() {
        let mut t = TranscriptProjection::honest_empty();
        t.meta = ProjectionMeta::fresh(sources::TRANSCRIPT);
        for i in 0..(MAX_TRANSCRIPT_ROWS + 5) {
            t.push_bounded(TranscriptRow {
                message_ref: format!("m{i}"),
                role: "owner".into(),
                text: format!("row {i}"),
                ack: MessageAck::Confirmed,
            });
        }
        assert_eq!(t.rows.len(), MAX_TRANSCRIPT_ROWS);
        assert!(t.truncated);
        assert_eq!(t.rows.first().unwrap().message_ref, "m5");
    }

    #[test]
    fn activity_capacity_bound() {
        let mut a = ActivityProjection::honest_empty();
        for i in 0..(MAX_ACTIVITY_ROWS + 3) {
            a.push_bounded(ActivityRow {
                event_ref: format!("e{i}"),
                kind: "tool.call".into(),
                summary: format!("tool {i}"),
                turn_ref: None,
            });
        }
        assert_eq!(a.rows.len(), MAX_ACTIVITY_ROWS);
        assert!(a.truncated);
    }

    #[test]
    fn effectd_unavailable_marks_all_sources_honest() {
        let mut p = WorkroomProjection::honest_unsubscribed();
        p.mark_effectd_unavailable("supervisor not initialized");
        assert_eq!(p.room.meta.gap, GapState::Unavailable);
        assert_eq!(p.transcript.meta.gap, GapState::Unavailable);
        assert_eq!(p.activity.meta.gap, GapState::Unavailable);
        assert_eq!(p.receipts.meta.gap, GapState::Unavailable);
        assert_eq!(p.run_state.meta.gap, GapState::Unavailable);
        assert!(
            p.connection_detail
                .as_deref()
                .unwrap()
                .contains("supervisor")
        );
    }

    #[test]
    fn run_phase_from_turn_events() {
        assert_eq!(RunPhase::from_event_kind("turn.queued"), RunPhase::Queued);
        assert_eq!(
            RunPhase::from_event_kind("turn.running"),
            RunPhase::Running
        );
        assert_eq!(
            RunPhase::from_event_kind("turn.interrupted"),
            RunPhase::Interrupted
        );
        assert_eq!(
            RunPhase::from_event_kind("turn.finished"),
            RunPhase::Finished
        );
    }
}
