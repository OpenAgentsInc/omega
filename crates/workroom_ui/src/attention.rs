//! Proactive updates and room attention (`OMEGA-SW-06`).
//!
//! Proactive tick updates arrive as ordinary hosted-runtime turns on the same
//! conversation. The pane needs no new source. Unread count and one attention
//! marker are derived from the transcript projection plus a **local** read
//! marker. Cross-device read state is `SARAH-NR-07` (NIP-RS kind 30078) — not
//! invented here.
//!
//! `SARAH_AUTONOMOUS_TICK_ENABLED` stays default off. Omega never enables it.
//! When the tick is off and the room has no turns, the empty room stays honest.

use crate::projections::{MessageAck, TranscriptProjection, TranscriptRow};

/// Server-side autonomous tick env flag name (OpenAgents API). Omega must not
/// enable this flag; it is observed only as ambient product truth.
pub const SARAH_AUTONOMOUS_TICK_FLAG: &str = "SARAH_AUTONOMOUS_TICK_ENABLED";

/// Omega disposition: the autonomous tick stays **off** for this client.
///
/// The dogfood window may change server disposition later; Omega still never
/// enables the flag from the pane.
pub const OMEGA_AUTONOMOUS_TICK_ENABLED: bool = false;

/// Local-only read marker for one Sarah room (MVP). Not a cross-device protocol.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LocalReadState {
    /// Thread / conversation the marker applies to, when known.
    pub thread_ref: Option<String>,
    /// Last message the owner marked read (message_ref or event id).
    pub last_read_message_ref: Option<String>,
    /// True after the owner has explicitly or implicitly marked the room read.
    pub has_marked_read: bool,
}

impl LocalReadState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn for_thread(thread_ref: impl Into<String>) -> Self {
        Self {
            thread_ref: Some(thread_ref.into()),
            last_read_message_ref: None,
            has_marked_read: false,
        }
    }

    /// Bind the marker to a room thread. A different thread clears the cursor.
    pub fn bind_thread(&mut self, thread_ref: Option<&str>) {
        match (self.thread_ref.as_deref(), thread_ref) {
            (Some(existing), Some(next)) if existing == next => {}
            (_, Some(next)) => {
                *self = Self::for_thread(next);
            }
            (_, None) => {}
        }
    }

    /// Mark every currently known transcript message as read.
    pub fn mark_read_through(&mut self, last_message_ref: Option<String>) {
        self.last_read_message_ref = last_message_ref;
        self.has_marked_read = true;
    }

    /// Mark read through the latest confirmed row in the transcript page.
    pub fn mark_read_from_transcript(&mut self, transcript: &TranscriptProjection) {
        let last = transcript
            .rows
            .iter()
            .rev()
            .find(|row| row.ack == MessageAck::Confirmed)
            .map(|row| row.message_ref.clone())
            .or_else(|| transcript.rows.last().map(|row| row.message_ref.clone()));
        self.mark_read_through(last);
    }

    /// Serialize for local KVP / test fixtures only. Not a wire protocol.
    pub fn to_local_value(&self) -> String {
        match (
            self.thread_ref.as_deref(),
            self.last_read_message_ref.as_deref(),
            self.has_marked_read,
        ) {
            (Some(thread), Some(msg), true) => format!("v1|{thread}|{msg}|1"),
            (Some(thread), None, true) => format!("v1|{thread}||1"),
            (None, Some(msg), true) => format!("v1||{msg}|1"),
            (Some(thread), Some(msg), false) => format!("v1|{thread}|{msg}|0"),
            _ => "v1|||0".into(),
        }
    }

    /// Parse a value produced by [`Self::to_local_value`]. Unknown forms yield
    /// an empty marker (honest unread until the owner marks read).
    pub fn from_local_value(raw: &str) -> Self {
        let parts: Vec<&str> = raw.splitn(4, '|').collect();
        if parts.len() != 4 || parts[0] != "v1" {
            return Self::default();
        }
        let thread_ref = if parts[1].is_empty() {
            None
        } else {
            Some(parts[1].to_string())
        };
        let last_read_message_ref = if parts[2].is_empty() {
            None
        } else {
            Some(parts[2].to_string())
        };
        let has_marked_read = parts[3] == "1";
        Self {
            thread_ref,
            last_read_message_ref,
            has_marked_read,
        }
    }
}

/// Whether the autonomous tick is considered enabled for honesty checks.
///
/// Omega hard-codes off. A future observation of server disposition may pass
/// `server_enabled`; it must never flip the Omega client default.
pub fn autonomous_tick_enabled(server_enabled: Option<bool>) -> bool {
    let _ = server_enabled;
    OMEGA_AUTONOMOUS_TICK_ENABLED
}

/// One attention marker for the Sarah room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttentionMarker {
    /// No unread Sarah-originated turns.
    None,
    /// Owner has unread room turns that need attention.
    NeedsAttention,
}

impl AttentionMarker {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::NeedsAttention => "needs_attention",
        }
    }

    pub fn is_set(self) -> bool {
        matches!(self, Self::NeedsAttention)
    }

    pub fn from_unread(unread_count: usize) -> Self {
        if unread_count > 0 {
            Self::NeedsAttention
        } else {
            Self::None
        }
    }
}

/// Unread count + attention marker for the Sarah workroom room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomAttention {
    pub unread_count: usize,
    pub marker: AttentionMarker,
    /// Local-only read marker (MVP). Not NIP-RS.
    pub read_state: LocalReadState,
    /// Ambient note when the autonomous tick is off (default).
    pub tick_note: Option<&'static str>,
}

impl RoomAttention {
    pub fn honest_empty() -> Self {
        Self {
            unread_count: 0,
            marker: AttentionMarker::None,
            read_state: LocalReadState::new(),
            tick_note: Some(tick_off_honest_note()),
        }
    }

    pub fn summary_line(&self) -> String {
        format!(
            "unread={} · attention={} · tick={}",
            self.unread_count,
            self.marker.label(),
            if autonomous_tick_enabled(None) {
                "on"
            } else {
                "off"
            }
        )
    }

    pub fn icon_label(&self) -> Option<String> {
        if self.unread_count == 0 {
            None
        } else if self.unread_count > 99 {
            Some("99+".into())
        } else {
            Some(self.unread_count.to_string())
        }
    }
}

/// Public-safe note when the autonomous tick is off.
pub fn tick_off_honest_note() -> &'static str {
    "Autonomous tick off (default). Empty room is not synthetic activity."
}

/// Roles whose confirmed messages count toward room attention when unread.
pub fn is_attention_role(role: &str) -> bool {
    let normalized = role.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "sarah" | "assistant" | "agent" | "principal.sarah" | "owner_orchestrator"
    )
}

/// True when a transcript row is a Sarah-originated (or proactive) turn that
/// can raise attention when unread. Owner pending/confirmed rows do not.
pub fn row_raises_attention(row: &TranscriptRow) -> bool {
    row.ack == MessageAck::Confirmed && is_attention_role(&row.role)
}

/// Count unread attention-raising rows after the local read marker.
///
/// Ordering is page order (oldest → newest). Rows at or before the last-read
/// message_ref are treated as read. If the marker is missing and the owner has
/// never marked read, every attention-raising confirmed row is unread.
pub fn count_unread(transcript: &TranscriptProjection, read: &LocalReadState) -> usize {
    let rows: Vec<&TranscriptRow> = transcript
        .rows
        .iter()
        .filter(|row| row_raises_attention(row))
        .collect();
    if rows.is_empty() {
        return 0;
    }
    let Some(last_read) = read.last_read_message_ref.as_deref() else {
        // Never marked: all attention rows are unread (including first open).
        return rows.len();
    };
    if let Some(idx) = transcript
        .rows
        .iter()
        .position(|row| row.message_ref == last_read)
    {
        transcript.rows[idx + 1..]
            .iter()
            .filter(|row| row_raises_attention(row))
            .count()
    } else {
        // Marker not in page (truncated / different page): treat all page
        // attention rows as potentially unread rather than inventing zero.
        rows.len()
    }
}

/// Recompute room attention from transcript + local read state.
pub fn compute_room_attention(
    transcript: &TranscriptProjection,
    mut read_state: LocalReadState,
    thread_ref: Option<&str>,
) -> RoomAttention {
    if let Some(thread) = thread_ref {
        if read_state.thread_ref.as_deref() != Some(thread) {
            // Thread change resets the local marker so we do not bleed cursors.
            if read_state.thread_ref.is_some() {
                read_state = LocalReadState::for_thread(thread);
            } else {
                read_state.thread_ref = Some(thread.to_string());
            }
        }
    }

    let unread_count = count_unread(transcript, &read_state);
    let marker = AttentionMarker::from_unread(unread_count);
    let tick_note = if autonomous_tick_enabled(None) {
        None
    } else {
        Some(tick_off_honest_note())
    };

    RoomAttention {
        unread_count,
        marker,
        read_state,
        tick_note,
    }
}

/// Proactive tick turns are ordinary transcript rows. No separate source.
///
/// Accepts the same shape as a Q&A answer: role + text + ack + message_ref.
/// Optional `origin` / model tags on the wire are ignored for rendering class.
pub fn proactive_turn_as_transcript_row(
    message_ref: impl Into<String>,
    text: impl Into<String>,
    ack: MessageAck,
) -> TranscriptRow {
    TranscriptRow {
        message_ref: message_ref.into(),
        role: "sarah".into(),
        text: text.into(),
        ack,
    }
}

/// Honest empty-room check when the autonomous tick is off: no synthetic rows.
pub fn empty_room_is_honest(
    transcript: &TranscriptProjection,
    tick_enabled: bool,
) -> bool {
    if tick_enabled {
        // When enabled, emptiness is still allowed; honesty is "no fake rows".
        return !transcript_has_synthetic_tick_filler(transcript);
    }
    // Off: empty or real rows only — never invent proactive filler.
    transcript.rows.is_empty() || !transcript_has_synthetic_tick_filler(transcript)
}

/// Detect pane-local synthetic filler that would fake tick activity.
///
/// Real service rows (including proactive ticks when the server posts them)
/// use ordinary message refs. Only an explicit local placeholder is dishonest.
fn transcript_has_synthetic_tick_filler(transcript: &TranscriptProjection) -> bool {
    transcript.rows.iter().any(|row| {
        row.message_ref.starts_with("local:synthetic_tick:")
            || row.text == "__omega_fake_proactive_activity__"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projections::{ProjectionMeta, sources};

    fn confirmed_sarah(id: &str, text: &str) -> TranscriptRow {
        TranscriptRow {
            message_ref: id.into(),
            role: "sarah".into(),
            text: text.into(),
            ack: MessageAck::Confirmed,
        }
    }

    fn confirmed_owner(id: &str, text: &str) -> TranscriptRow {
        TranscriptRow {
            message_ref: id.into(),
            role: "owner".into(),
            text: text.into(),
            ack: MessageAck::Confirmed,
        }
    }

    fn page(rows: Vec<TranscriptRow>) -> TranscriptProjection {
        TranscriptProjection {
            meta: ProjectionMeta::fresh(sources::TRANSCRIPT),
            rows,
            cursor: None,
            truncated: false,
        }
    }

    #[test]
    fn omega_never_enables_autonomous_tick() {
        assert!(!OMEGA_AUTONOMOUS_TICK_ENABLED);
        assert!(!autonomous_tick_enabled(None));
        assert!(!autonomous_tick_enabled(Some(true)));
        assert_eq!(SARAH_AUTONOMOUS_TICK_FLAG, "SARAH_AUTONOMOUS_TICK_ENABLED");
    }

    #[test]
    fn empty_room_stays_honest_when_tick_off() {
        let empty = page(vec![]);
        assert!(empty_room_is_honest(&empty, false));
        let attention = compute_room_attention(&empty, LocalReadState::new(), None);
        assert_eq!(attention.unread_count, 0);
        assert_eq!(attention.marker, AttentionMarker::None);
        assert!(attention.tick_note.is_some());
    }

    #[test]
    fn does_not_invent_fake_proactive_activity() {
        let empty = page(vec![]);
        assert!(!transcript_has_synthetic_tick_filler(&empty));
        let fake = page(vec![TranscriptRow {
            message_ref: "local:synthetic_tick:1".into(),
            role: "sarah".into(),
            text: "__omega_fake_proactive_activity__".into(),
            ack: MessageAck::Confirmed,
        }]);
        assert!(transcript_has_synthetic_tick_filler(&fake));
        assert!(!empty_room_is_honest(&fake, false));
    }

    #[test]
    fn proactive_tick_turn_is_ordinary_transcript_row() {
        let row = proactive_turn_as_transcript_row(
            "message.sarah_auto.tick.1",
            "Release state is green.",
            MessageAck::Confirmed,
        );
        assert_eq!(row.role, "sarah");
        assert_eq!(row.ack, MessageAck::Confirmed);
        // Same evidence class as a Q&A answer: ordinary row, same role path.
        let answer = confirmed_sarah("message.answer.1", "Release state is green.");
        assert_eq!(row.role, answer.role);
        assert_eq!(row.ack, answer.ack);

        let mut t = page(vec![confirmed_owner("m1", "status?"), row]);
        t.push_bounded(answer);
        assert_eq!(t.rows.len(), 3);
        assert!(t.rows.iter().all(|r| r.role == "owner" || r.role == "sarah"));
    }

    #[test]
    fn unread_count_and_attention_marker() {
        let transcript = page(vec![
            confirmed_owner("m1", "hello"),
            confirmed_sarah("m2", "hi"),
            confirmed_sarah("m3", "proactive update"),
        ]);
        let read = LocalReadState::new();
        let attention = compute_room_attention(&transcript, read, Some("thread.sarah.abc"));
        assert_eq!(attention.unread_count, 2);
        assert_eq!(attention.marker, AttentionMarker::NeedsAttention);
        assert_eq!(attention.icon_label().as_deref(), Some("2"));
        assert!(attention.marker.is_set());
    }

    #[test]
    fn mark_read_clears_attention() {
        let transcript = page(vec![
            confirmed_owner("m1", "hello"),
            confirmed_sarah("m2", "hi"),
            confirmed_sarah("m3", "proactive update"),
        ]);
        let mut read = LocalReadState::for_thread("thread.sarah.abc");
        read.mark_read_from_transcript(&transcript);
        assert_eq!(read.last_read_message_ref.as_deref(), Some("m3"));
        assert!(read.has_marked_read);

        let attention = compute_room_attention(&transcript, read, Some("thread.sarah.abc"));
        assert_eq!(attention.unread_count, 0);
        assert_eq!(attention.marker, AttentionMarker::None);
        assert!(attention.icon_label().is_none());
    }

    #[test]
    fn mark_read_mid_page_leaves_later_unread() {
        let transcript = page(vec![
            confirmed_sarah("m1", "a"),
            confirmed_sarah("m2", "b"),
            confirmed_sarah("m3", "c"),
        ]);
        let mut read = LocalReadState::new();
        read.mark_read_through(Some("m1".into()));
        assert_eq!(count_unread(&transcript, &read), 2);

        read.mark_read_through(Some("m2".into()));
        assert_eq!(count_unread(&transcript, &read), 1);
    }

    #[test]
    fn owner_rows_do_not_raise_attention() {
        let transcript = page(vec![
            confirmed_owner("m1", "hello"),
            confirmed_owner("m2", "still me"),
        ]);
        let attention = compute_room_attention(&transcript, LocalReadState::new(), None);
        assert_eq!(attention.unread_count, 0);
        assert_eq!(attention.marker, AttentionMarker::None);
    }

    #[test]
    fn pending_sarah_row_does_not_raise_attention() {
        let transcript = page(vec![TranscriptRow {
            message_ref: "local:1".into(),
            role: "sarah".into(),
            text: "streaming".into(),
            ack: MessageAck::Pending,
        }]);
        assert_eq!(count_unread(&transcript, &LocalReadState::new()), 0);
    }

    #[test]
    fn local_read_state_round_trip() {
        let mut read = LocalReadState::for_thread("thread.sarah.abc");
        read.mark_read_through(Some("msg.9".into()));
        let encoded = read.to_local_value();
        let decoded = LocalReadState::from_local_value(&encoded);
        assert_eq!(decoded, read);
        assert_eq!(
            LocalReadState::from_local_value("garbage"),
            LocalReadState::default()
        );
    }

    #[test]
    fn thread_change_resets_read_marker() {
        let mut read = LocalReadState::for_thread("thread.a");
        read.mark_read_through(Some("m9".into()));
        let transcript = page(vec![confirmed_sarah("m1", "hi")]);
        let attention = compute_room_attention(&transcript, read, Some("thread.b"));
        assert_eq!(attention.read_state.thread_ref.as_deref(), Some("thread.b"));
        assert!(attention.read_state.last_read_message_ref.is_none());
        assert_eq!(attention.unread_count, 1);
    }

    #[test]
    fn attention_roles_cover_principal_aliases() {
        assert!(is_attention_role("sarah"));
        assert!(is_attention_role("principal.sarah"));
        assert!(is_attention_role("Assistant"));
        assert!(!is_attention_role("owner"));
        assert!(!is_attention_role("user"));
    }
}
