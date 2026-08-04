//! Zero base's persistent sidebar: what it contains, how wide it is allowed to
//! be, and what it remembers.
//!
//! `OMEGA-DELTA-0130`. The owner's words: *"i want a persistent sidebar,
//! collapsible, kinda like the thread sidebar but also with some vertical
//! collapsible menus. i want the last 10 chat threads as one thing, and codex
//! etc ratelimits showing, and nostr nip 29 activity too etc — get me an
//! initial version of that added now. default open on the zerobase chat page."*
//!
//! # One sidebar, not two
//!
//! `OMEGA-DELTA-0118` built a threads sidebar as an **overlay**: absolutely
//! positioned over the thread surface, opened by `cmd-alt-j`, closed again to
//! get at what was under it. That was right for a surface you visit. It is
//! wrong for one that is *persistent* and *default open*, because an overlay
//! that is always there is a permanent lid on the left third of the transcript.
//!
//! So the overlay is gone and this is what `cmd-alt-j` now toggles. **The key
//! binding, the action and the menu entry are unchanged** — `cmd-alt-j` is
//! still `agent::ToggleThreadsSidebar` in the `agent` namespace zero base
//! admits, which was the entire load-bearing repair in `OMEGA-DELTA-0118` and
//! is not touched here. What changed is what appears when you press it: the
//! threads are now the first section of a sidebar rather than the whole of it,
//! and pressing it collapses the sidebar to a rail rather than removing it.
//!
//! Keeping both would have been the worse answer, and `OMEGA-DELTA-0118`'s own
//! notes say why: two surfaces listing the same threads is one window giving
//! two answers to one question.
//!
//! # The composer, which is the constraint that actually decides this
//!
//! `OMEGA-DELTA-0105` records that the composer's bottom row wraps so a narrow
//! dock does not clip **Send**. `OMEGA-DELTA-0118` protected that by refusing
//! to take part in the layout at all — an overlay changes nobody's width. A
//! persistent sidebar cannot make that promise, because a column that is always
//! there is always taking width from the column beside it.
//!
//! The answer is not to hope the window is wide. It is [`layout`]: the sidebar
//! is a real column, and it **yields** rather than squeezing. Below the width
//! at which the content column can still hold a composer, the sidebar column is
//! not drawn; expand/collapse and Settings live on the workbench activity rail
//! (`OMEGA-DELTA-0205`), and the content gets everything else. The person's
//! preference is not overwritten when this happens, so widening the window
//! restores the sidebar they asked for.
//!
//! That is a stronger promise than the overlay's, not a weaker one. The overlay
//! never narrowed the composer and always covered it; this never covers it and
//! never narrows it past the floor.
//!
//! # Adding a fourth section
//!
//! Three things, all in this file except the last:
//!
//! 1. A variant on [`SectionId`], and its entry in [`SectionId::ALL`] — which
//!    is the draw order.
//! 2. Its `key` and `title` arms. The key is what persists a collapsed section
//!    across launches, so it must never be renamed once shipped.
//! 3. One arm in the panel's `render_sidebar_section`, producing the section's
//!    element.
//!
//! `omega_deltas` asserts that every variant of [`SectionId`] appears in that
//! match, so a section added here and forgotten there fails the suite rather
//! than drawing an empty heading.
//!
//! # No section may interrupt
//!
//! There is no toast, banner, modal, or refusal in this model. A section that
//! cannot load draws one quiet line in its own body. The sections around it do
//! not know or care, and the sidebar still draws the owner's threads.

use gpui::{Pixels, SharedString, px};
use serde::{Deserialize, Serialize};

/// The width the sidebar takes when it is expanded.
///
/// `OMEGA-DELTA-0118`'s overlay width, kept: the rows are the same rows, and a
/// person who has been reading thread titles at 280px should not have to
/// relearn where they wrap.
pub const SIDEBAR_WIDTH: Pixels = px(280.);

/// The width the sidebar takes when it is collapsed.
///
/// Zero. `OMEGA-DELTA-0205` moved expand/collapse and Settings onto the
/// workbench activity rail, so a collapsed sidebar no longer keeps its own
/// vertical strip. "Persistent" is kept by those controls remaining drawn —
/// not by a second empty rail beside the activity rail.
pub const RAIL_WIDTH: Pixels = px(0.);

/// The width the content column may never be reduced below.
///
/// This is the number the whole layout turns on, so it is worth saying what it
/// is and is not. It is **not** a measurement of the composer; the composer is
/// a wrapping flex row and has no single minimum. It is the width below which
/// that wrap is the *only* thing left protecting **Send** — the state
/// `OMEGA-DELTA-0105` describes as already tight. `thread_view` independently
/// picks 960px of viewport as the point where the Exo controls go compact, so
/// this sits comfortably under that: at 880px of window the sidebar is still
/// expanded and the content still has 600.
pub const MIN_CONTENT_WIDTH: Pixels = px(600.);

/// How many threads the first section lists.
///
/// The owner's number: "the last 10 chat threads as one thing".
pub const RECENT_THREADS: usize = 10;

/// Where the collapsed state is written.
pub const STATE_KEY: &str = "omega-zero-base-sidebar";

/// How wide the sidebar draws, given the room it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Drawn in full, at [`SIDEBAR_WIDTH`].
    Expanded,
    /// Collapsed: no column of its own. Expand lives on the activity rail.
    Rail,
}

impl Layout {
    #[must_use]
    pub fn width(self) -> Pixels {
        match self {
            Layout::Expanded => SIDEBAR_WIDTH,
            Layout::Rail => RAIL_WIDTH,
        }
    }

    #[must_use]
    pub fn is_expanded(self) -> bool {
        matches!(self, Layout::Expanded)
    }
}

/// What the sidebar may draw in `available` pixels, given what the person asked
/// for.
///
/// `wants_open` is the persisted preference, and this **never writes back to
/// it**. A window dragged narrow and then wide again shows the sidebar it
/// showed before, because the narrow window changed what was drawn and not what
/// was wanted. The alternative — collapsing the stored preference on a resize —
/// is how a sidebar quietly stops coming back and the person concludes the
/// toggle is broken.
#[must_use]
pub fn layout(available: Pixels, wants_open: bool) -> Layout {
    if !wants_open {
        return Layout::Rail;
    }
    if available - SIDEBAR_WIDTH < MIN_CONTENT_WIDTH {
        return Layout::Rail;
    }
    Layout::Expanded
}

/// The sections, in the order they are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionId {
    /// The last [`RECENT_THREADS`] conversations. `OMEGA-DELTA-0118`'s rows.
    RecentThreads,
    /// Public channel destinations. Messages belong in the selected main view.
    PublicChannels,
}

impl SectionId {
    /// Draw order, top to bottom.
    ///
    /// Threads first because that is the section with something to do in it.
    /// TEMPORARILY HIDDEN (2026-08-04, owner request): `SectionId::PublicChannels`
    /// ("Tester channels") is out of the draw order while it is not ready to
    /// show. The variant, its key, and its title are intentionally retained so
    /// persisted collapse state and `OMEGA-DELTA` checks still resolve; restore
    /// by putting it back in this list.
    pub const ALL: &'static [SectionId] = &[SectionId::RecentThreads];

    /// The stable name this section is remembered under.
    ///
    /// Written into the key-value store. Renaming one silently un-collapses
    /// that section for everybody who had collapsed it, so these are frozen
    /// once shipped even if the title above them changes.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            SectionId::RecentThreads => "recent-threads",
            SectionId::PublicChannels => "nostr-activity",
        }
    }

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            SectionId::RecentThreads => "Recent threads",
            SectionId::PublicChannels => "Tester channels",
        }
    }
}

/// What the sidebar remembers between launches.
///
/// Deliberately small and deliberately forgiving: an unknown key in `collapsed`
/// is ignored rather than rejected, so a build that drops a section does not
/// make an older build's stored state unreadable.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarState {
    /// Whether the person wants the sidebar expanded. Not whether it is.
    pub open: bool,
    /// The `key`s of sections the person has collapsed.
    #[serde(default)]
    pub collapsed: Vec<String>,
}

impl SidebarState {
    /// The state a machine that has never stored one gets.
    ///
    /// Open, with recent threads and tester channels expanded. A clean profile
    /// must expose the alpha feedback destination without requiring a person
    /// to discover a collapsed section first.
    #[must_use]
    pub fn default_open() -> Self {
        Self {
            open: true,
            collapsed: Vec::new(),
        }
    }

    #[must_use]
    pub fn is_collapsed(&self, section: SectionId) -> bool {
        self.collapsed.iter().any(|key| key == section.key())
    }

    pub fn toggle_section(&mut self, section: SectionId) {
        if let Some(index) = self.collapsed.iter().position(|key| key == section.key()) {
            self.collapsed.remove(index);
        } else {
            self.collapsed.push(section.key().to_string());
        }
    }

    /// Read stored state, or the default when there is none or it is unreadable.
    ///
    /// Unreadable JSON is the default rather than an error. This is a sidebar's
    /// collapsed state; refusing to draw it because a stored string is corrupt
    /// would be the section that interrupts, one layer down.
    #[must_use]
    pub fn from_stored(stored: Option<&str>) -> Self {
        stored
            .and_then(|json| serde_json::from_str::<SidebarState>(json).ok())
            .unwrap_or_else(SidebarState::default_open)
    }
}

/// One row in a section that is not the threads list.
///
/// The threads section draws its own rows, because those are clickable and
/// carry a thread's identity. Everything else is two strings and whether they
/// are muted, and a fourth section should not have to invent a third shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionRow {
    pub primary: SharedString,
    pub secondary: Option<SharedString>,
    /// Drawn dimmer: this row is about something absent, unknown, or refused.
    pub muted: bool,
}

impl SectionRow {
    #[must_use]
    pub fn new(primary: impl Into<SharedString>) -> Self {
        Self {
            primary: primary.into(),
            secondary: None,
            muted: false,
        }
    }

    #[must_use]
    pub fn secondary(mut self, secondary: impl Into<SharedString>) -> Self {
        self.secondary = Some(secondary.into());
        self
    }

    #[must_use]
    pub fn muted(mut self) -> Self {
        self.muted = true;
        self
    }
}

/// What a section has to draw.
///
/// There is no error variant, on purpose. A section that could not load has no
/// rows and a [`note`], which is the same shape as a section that loaded and
/// had nothing to show. The drawing code therefore has no failure branch to get
/// wrong, and no section can take the sidebar down with it.
///
/// [`note`]: SectionBody::note
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SectionBody {
    pub rows: Vec<SectionRow>,
    /// One quiet line, drawn under the rows — or in their place when there are
    /// none. Never a toast, never a banner, never red.
    pub note: Option<SharedString>,
}

impl SectionBody {
    #[must_use]
    pub fn rows(rows: Vec<SectionRow>) -> Self {
        Self { rows, note: None }
    }

    /// A section with nothing to draw, and the sentence saying why.
    #[must_use]
    pub fn note(note: impl Into<SharedString>) -> Self {
        Self {
            rows: Vec::new(),
            note: Some(note.into()),
        }
    }

    #[must_use]
    pub fn with_note(mut self, note: impl Into<SharedString>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// Whether this section is drawing anything at all.
    ///
    /// A body with neither rows nor a note is a heading over blank space, which
    /// is the empty gauge this delta exists to not draw.
    #[must_use]
    pub fn is_silent(&self) -> bool {
        self.rows.is_empty() && self.note.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_window_draws_the_sidebar_the_person_asked_for() {
        assert_eq!(layout(px(1_400.), true), Layout::Expanded);
    }

    #[test]
    fn a_narrow_window_yields_the_sidebar_rather_than_the_composer() {
        // 800 - 280 = 520, under the 600 the content column must keep.
        assert_eq!(
            layout(px(800.), true),
            Layout::Rail,
            "the sidebar must give up its width before the composer gives up \
             Send. OMEGA-DELTA-0105 records that row already wrapping to stay \
             whole in a narrow dock."
        );
    }

    #[test]
    fn the_floor_is_exactly_where_it_says_it_is() {
        // 880 - 280 = 600, exactly the floor, which is allowed.
        assert_eq!(layout(px(880.), true), Layout::Expanded);
        assert_eq!(layout(px(879.), true), Layout::Rail);
    }

    #[test]
    fn a_closed_sidebar_stays_closed_however_wide_the_window_is() {
        assert_eq!(layout(px(4_000.), false), Layout::Rail);
    }

    #[test]
    fn a_collapsed_sidebar_takes_no_column_of_its_own() {
        assert_eq!(
            Layout::Rail.width(),
            px(0.),
            "OMEGA-DELTA-0205: the collapsed sidebar must not keep a separate \
             rail. Expand and Settings live on the workbench activity rail."
        );
        assert!(Layout::Expanded.width() > Layout::Rail.width());
        assert_eq!(RAIL_WIDTH, px(0.));
    }

    #[test]
    fn a_machine_that_has_never_stored_a_state_opens_threads_and_tester_channels() {
        let state = SidebarState::from_stored(None);
        assert!(state.open, "the owner asked for default open on zero base");
        assert!(!state.is_collapsed(SectionId::RecentThreads));
        assert!(!state.is_collapsed(SectionId::PublicChannels));
        assert_eq!(SectionId::PublicChannels.title(), "Tester channels");
    }

    #[test]
    fn a_corrupt_stored_state_opens_rather_than_failing() {
        assert_eq!(
            SidebarState::from_stored(Some("{ not json")),
            SidebarState::default_open(),
            "no section may interrupt, and that includes the one that reads the \
             sidebar's own state."
        );
    }

    #[test]
    fn section_choices_survive_a_round_trip() {
        let mut state = SidebarState::default_open();
        state.toggle_section(SectionId::RecentThreads);
        state.toggle_section(SectionId::PublicChannels);
        let json = serde_json::to_string(&state).expect("serialises");
        let restored = SidebarState::from_stored(Some(&json));

        assert!(
            restored.is_collapsed(SectionId::RecentThreads),
            "'persistent' covers the vertical menus too — a section collapsed \
             before a restart must come back collapsed."
        );
        assert!(
            restored.is_collapsed(SectionId::PublicChannels),
            "an explicitly collapsed tester-channel section must remain collapsed"
        );
        assert!(restored.open);
    }

    #[test]
    fn toggling_a_section_twice_leaves_it_as_it_was() {
        let mut state = SidebarState::default_open();
        state.toggle_section(SectionId::PublicChannels);
        assert!(state.is_collapsed(SectionId::PublicChannels));
        state.toggle_section(SectionId::PublicChannels);
        assert!(!state.is_collapsed(SectionId::PublicChannels));
        assert_eq!(state, SidebarState::default_open());
    }

    #[test]
    fn a_state_stored_by_a_build_with_a_section_this_one_lacks_still_reads() {
        let state = SidebarState::from_stored(Some(
            r#"{"open":true,"collapsed":["rate-limits","a-section-from-the-future"]}"#,
        ));
        assert!(state.open);
        assert_eq!(
            state.collapsed,
            ["rate-limits", "a-section-from-the-future"],
            "unknown keys stay readable so removing a section does not corrupt \
             state written by an older build"
        );
    }

    #[test]
    fn every_section_has_a_distinct_key_and_a_title() {
        let mut keys: Vec<&str> = SectionId::ALL.iter().map(|id| id.key()).collect();
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(
            keys.len(),
            count,
            "two sections sharing a key collapse together and cannot be told \
             apart in stored state"
        );
        for section in SectionId::ALL {
            assert!(!section.title().is_empty());
            assert!(!section.key().is_empty());
        }
    }

    #[test]
    fn no_section_body_can_carry_a_refusal_shape() {
        let failed = SectionBody::note("relay.openagents.com did not answer.");
        assert!(failed.rows.is_empty());
        assert_eq!(
            failed.note.as_deref(),
            Some("relay.openagents.com did not answer."),
            "a section that cannot load says so in place, quietly, and the \
             sections around it keep working"
        );
        assert!(
            !failed.is_silent(),
            "a heading over blank space is the failure"
        );
    }
}
