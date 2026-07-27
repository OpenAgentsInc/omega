//! `OMEGA-DELTA-0103`. A tool result is a versioned artifact; the event that
//! carries it is a bounded preview that names what it withheld.
//!
//! The failure this exists for: a `terminal` call returned a Nostr event and
//! forty lines of hex and signatures went into the record whole. A rendering
//! ceiling (`OMEGA-DELTA-0080`) hides them from a reader, but every other
//! consumer — the model's own context, a transcript reader, a receipt — still
//! carries the blob. The bound has to be a property of the record, not of one
//! surface.
//!
//! The shape is `OMEGA-DELTA-0060`'s, deliberately: a bound is only honest if
//! it is visible when it fires, and room for the marker is reserved *inside*
//! the budget so the budget can never be the reason the reader is not told
//! about the budget. Here the reserve is computed from the marker rather than
//! guessed at, so no constant can drift away from the sentence it protects.
//!
//! Nothing in this module can reach a file, a window, or a clock. A tool
//! result's bound is arithmetic over text, and it is tested as such.

use std::fmt;
use std::sync::Arc;

/// Bytes of a tool result the record keeps inline before it withholds the rest.
///
/// Sized for a model's context rather than a reader's screen: roughly a
/// thousand tokens of output, enough for a build log's tail or a test summary,
/// and about a quarter of the 16 KiB that `terminal_tool` already allows.
///
/// This is emphatically *not* a substitute for the rendering ceiling, nor it
/// for this. Four thousand bytes of unwrapped Nostr JSON is still around forty
/// lines — the exact height the owner objected to. One bounds the record, the
/// other bounds the height; a result can need both.
pub const TOOL_RESULT_PREVIEW_BYTE_BUDGET: usize = 4_000;

/// The address of one version of one tool result.
///
/// `source` names what produced the result (a terminal id, a tool call id);
/// `version` counts the captures of that source. Both are needed: a source
/// alone would silently re-point at a later capture, which is how a reader ends
/// up quoting output that no longer exists.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ToolResultArtifactId {
    source: Arc<str>,
    version: u32,
}

impl ToolResultArtifactId {
    pub fn new(source: impl Into<Arc<str>>, version: u32) -> Self {
        Self {
            source: source.into(),
            version,
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

impl fmt::Display for ToolResultArtifactId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}@v{}", self.source, self.version)
    }
}

/// One complete capture of a tool result. Never a preview: an artifact that
/// were itself bounded would leave the full result nowhere at all, and the
/// fetch path would be answering with the thing the reader already had.
#[derive(Clone, Debug)]
pub struct ToolResultArtifact {
    id: ToolResultArtifactId,
    text: Arc<str>,
    line_count: usize,
}

impl ToolResultArtifact {
    pub fn id(&self) -> &ToolResultArtifactId {
        &self.id
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn byte_count(&self) -> usize {
        self.text.len()
    }

    pub fn line_count(&self) -> usize {
        self.line_count
    }
}

/// Every version of one source's results, oldest first.
///
/// Held by whatever owns the result for as long as it owns it — for a terminal,
/// that is past the end of the turn that ran it — so [`Self::get`] is the fetch
/// path the preview's marker points a reader at.
#[derive(Clone, Debug)]
pub struct ToolResultArtifactStore {
    source: Arc<str>,
    versions: Vec<ToolResultArtifact>,
}

impl ToolResultArtifactStore {
    pub fn new(source: impl Into<Arc<str>>) -> Self {
        Self {
            source: source.into(),
            versions: Vec::new(),
        }
    }

    /// Record `text` as the next version, and return its address.
    ///
    /// Recording text identical to the current latest returns that version
    /// instead of appending: a result read twice is one result, and a version
    /// number that counts reads rather than changes tells a reader nothing.
    pub fn record(&mut self, text: &str) -> ToolResultArtifactId {
        if let Some(latest) = self.versions.last()
            && latest.text() == text
        {
            return latest.id.clone();
        }
        let id = ToolResultArtifactId::new(
            self.source.clone(),
            u32::try_from(self.versions.len())
                .unwrap_or(u32::MAX)
                .saturating_add(1),
        );
        self.versions.push(ToolResultArtifact {
            id: id.clone(),
            text: text.into(),
            line_count: line_count(text),
        });
        id
    }

    /// The fetch path. `None` when nothing at this address was ever recorded,
    /// which a caller must report rather than paper over — an unreachable full
    /// result is the failure the marker exists to make visible.
    pub fn get(&self, id: &ToolResultArtifactId) -> Option<&ToolResultArtifact> {
        self.versions.iter().find(|artifact| &artifact.id == id)
    }

    pub fn latest(&self) -> Option<&ToolResultArtifact> {
        self.versions.last()
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn version_count(&self) -> usize {
        self.versions.len()
    }
}

/// What the event carries: a bounded body, and the arithmetic a reader needs to
/// decide whether to fetch the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolResultPreview {
    /// The body, with the truncation marker already appended when one fired.
    /// This is the whole of what goes into the event.
    pub text: String,
    pub shown_bytes: usize,
    pub total_bytes: usize,
    pub shown_lines: usize,
    pub total_lines: usize,
    /// Where the complete result can be fetched, when there is one. `None`
    /// while the producing command is still running: there is no complete
    /// result to address yet, and saying so is better than inventing an
    /// address that resolves to nothing.
    pub artifact: Option<ToolResultArtifactId>,
}

impl ToolResultPreview {
    pub fn is_truncated(&self) -> bool {
        self.shown_bytes < self.total_bytes
    }

    pub fn withheld_bytes(&self) -> usize {
        self.total_bytes.saturating_sub(self.shown_bytes)
    }

    pub fn withheld_lines(&self) -> usize {
        self.total_lines.saturating_sub(self.shown_lines)
    }
}

/// Bound `body` to `budget` bytes, appending a marker naming what was withheld.
///
/// `body` may already be a prefix of the result — a terminal applies its own
/// byte limit before this ever sees the text — so the totals are passed in
/// rather than measured off `body`. Both bounds are then reported through the
/// one marker, and a result cut twice does not describe itself as cut once.
///
/// A result that fits is returned unchanged: no marker, no artifact reference,
/// no ceremony. That is load-bearing. A marker on every result is a marker a
/// reader stops reading.
pub fn preview_tool_result(
    body: &str,
    total_bytes: usize,
    total_lines: usize,
    budget: usize,
    artifact: Option<ToolResultArtifactId>,
) -> ToolResultPreview {
    let total_bytes = total_bytes.max(body.len());
    let total_lines = total_lines.max(line_count(body));

    if body.len() >= total_bytes && body.len() <= budget {
        return ToolResultPreview {
            shown_bytes: body.len(),
            total_bytes,
            shown_lines: total_lines,
            total_lines,
            text: body.to_owned(),
            artifact: None,
        };
    }

    // Reserve room for the marker itself, so the budget can never be the reason
    // the reader is not told about the budget. The reserve is the marker
    // rendered at its widest — every count at the total, which no real count
    // can exceed — rather than a constant that drifts as the sentence changes.
    let reserve = truncation_marker(
        total_bytes,
        total_bytes,
        total_lines,
        total_lines,
        artifact.as_ref(),
    )
    .len();
    let shown = clip_to_line(body, budget.saturating_sub(reserve));

    let shown_lines = line_count(shown);
    let mut text = shown.to_owned();
    text.push_str(&truncation_marker(
        shown.len(),
        total_bytes,
        shown_lines,
        total_lines,
        artifact.as_ref(),
    ));

    ToolResultPreview {
        text,
        shown_bytes: shown.len(),
        total_bytes,
        shown_lines,
        total_lines,
        artifact,
    }
}

/// The only place the withheld amount is put into words.
///
/// It states what is missing and where the rest is, in that order, because a
/// reader who stops after the first clause must still have been told the body
/// is incomplete.
fn truncation_marker(
    shown_bytes: usize,
    total_bytes: usize,
    shown_lines: usize,
    total_lines: usize,
    artifact: Option<&ToolResultArtifactId>,
) -> String {
    let withheld_bytes = total_bytes.saturating_sub(shown_bytes);
    let withheld_lines = total_lines.saturating_sub(shown_lines);
    let whereabouts = match artifact {
        Some(id) => format!("Full result: artifact {id}."),
        None => "The command is still running; the full result becomes an \
                 artifact when it exits."
            .to_owned(),
    };
    format!(
        "\n… [tool result truncated: {shown_bytes} of {total_bytes} bytes and \
         {shown_lines} of {total_lines} lines shown, {withheld_bytes} bytes and \
         {withheld_lines} lines withheld. {whereabouts}]"
    )
}

/// Clip to at most `limit` bytes, on a line boundary where one is available and
/// on a character boundary always.
///
/// Preferring the line boundary is not cosmetic: half a line of JSON reads as a
/// syntactically broken record rather than a clipped one, and the marker that
/// follows is then competing with a parse error for the reader's attention.
fn clip_to_line(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    match text[..end].rfind('\n') {
        Some(newline) => &text[..newline],
        None => &text[..end],
    }
}

/// Lines in `text`, counting a final line with no trailing newline.
fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nostr_event_lines(count: usize) -> String {
        (0..count)
            .map(|index| {
                format!(
                    "{{\"id\":\"{index:064x}\",\"sig\":\"{}\"}}",
                    "ab".repeat(64)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_small_result_is_unchanged() {
        let body = "publishing to relay.openagents.com... success.\n";
        let preview = preview_tool_result(
            body,
            body.len(),
            1,
            TOOL_RESULT_PREVIEW_BYTE_BUDGET,
            Some(ToolResultArtifactId::new("terminal:1", 1)),
        );
        assert_eq!(preview.text, body);
        assert!(!preview.is_truncated());
        assert_eq!(preview.withheld_bytes(), 0);
        assert_eq!(preview.withheld_lines(), 0);
        assert_eq!(
            preview.artifact, None,
            "a result that fits references no artifact: no marker, no ceremony"
        );
    }

    #[test]
    fn a_large_result_leaves_a_bounded_event_and_a_complete_artifact() {
        let full = nostr_event_lines(200);
        let mut store = ToolResultArtifactStore::new("terminal:blob");
        let id = store.record(&full);

        let preview = preview_tool_result(
            &full,
            full.len(),
            line_count(&full),
            TOOL_RESULT_PREVIEW_BYTE_BUDGET,
            Some(id.clone()),
        );

        assert!(
            preview.text.len() <= TOOL_RESULT_PREVIEW_BYTE_BUDGET,
            "the event is {} bytes, over the {TOOL_RESULT_PREVIEW_BYTE_BUDGET}-byte budget",
            preview.text.len()
        );
        assert!(preview.text.len() < full.len() / 4);
        assert!(preview.is_truncated());

        let artifact = store.get(&id).expect("the full result is addressable");
        assert_eq!(artifact.text(), full);
        assert_eq!(artifact.line_count(), 200);
    }

    #[test]
    fn the_marker_states_the_withheld_amount_and_where_the_rest_is() {
        let full = nostr_event_lines(200);
        let id = ToolResultArtifactId::new("terminal:blob", 3);
        let preview = preview_tool_result(
            &full,
            full.len(),
            200,
            TOOL_RESULT_PREVIEW_BYTE_BUDGET,
            Some(id.clone()),
        );

        assert!(preview.text.contains("tool result truncated"));
        assert!(
            preview
                .text
                .contains(&format!("{} bytes and", preview.withheld_bytes())),
            "the marker does not name the withheld bytes: {}",
            preview.text
        );
        assert!(
            preview
                .text
                .contains(&format!("{} lines withheld", preview.withheld_lines())),
            "the marker does not name the withheld lines: {}",
            preview.text
        );
        assert!(preview.text.contains(&format!("artifact {id}")));
        assert_eq!(id.to_string(), "terminal:blob@v3");
    }

    #[test]
    fn the_marker_never_costs_itself_its_own_room() {
        // `OMEGA-DELTA-0060`'s property, restated: the budget must not be the
        // reason the marker is missing. Squeeze the budget down to nothing and
        // the marker survives; only the body gives way.
        let full = nostr_event_lines(200);
        let id = ToolResultArtifactId::new("terminal:a-very-long-source-name-indeed", 987_654);
        for budget in [0, 1, 32, 200, 400, 4_000] {
            let preview = preview_tool_result(&full, full.len(), 200, budget, Some(id.clone()));
            assert!(
                preview.text.contains("tool result truncated"),
                "a {budget}-byte budget dropped the marker"
            );
            assert!(
                preview.text.contains(&format!("artifact {id}")),
                "a {budget}-byte budget dropped the artifact address"
            );
        }
    }

    #[test]
    fn a_truncated_preview_is_distinguishable_from_a_complete_one() {
        // The falsifier. Strip the marker and these two must become the same
        // text; this is the check that fails on exactly that.
        let full = nostr_event_lines(200);
        let truncated = preview_tool_result(
            &full,
            full.len(),
            200,
            TOOL_RESULT_PREVIEW_BYTE_BUDGET,
            Some(ToolResultArtifactId::new("terminal:blob", 1)),
        );
        let head = truncated
            .text
            .split_once("\n… [")
            .expect("a truncated preview carries the marker")
            .0
            .to_owned();
        let complete = preview_tool_result(
            &head,
            head.len(),
            line_count(&head),
            TOOL_RESULT_PREVIEW_BYTE_BUDGET,
            Some(ToolResultArtifactId::new("terminal:blob", 1)),
        );

        assert!(!complete.is_truncated());
        assert_eq!(complete.text, head);
        assert_ne!(
            truncated.text, complete.text,
            "a truncated preview and a complete one of the same body are \
             indistinguishable, so nothing tells a reader to fetch"
        );
    }

    #[test]
    fn a_body_already_cut_upstream_reports_both_cuts_through_one_marker() {
        // A terminal applies its own byte limit before the preview sees the
        // text. If the totals were measured off the body, the marker would
        // describe the second cut and silently absorb the first.
        let full = nostr_event_lines(200);
        let upstream_limit = 2_048;
        let body = clip_to_line(&full, upstream_limit);

        let preview = preview_tool_result(
            body,
            full.len(),
            200,
            TOOL_RESULT_PREVIEW_BYTE_BUDGET,
            Some(ToolResultArtifactId::new("terminal:blob", 1)),
        );

        assert_eq!(preview.total_bytes, full.len());
        assert_eq!(preview.total_lines, 200);
        assert!(preview.withheld_bytes() > full.len() - upstream_limit);
        assert_eq!(preview.text.matches("tool result truncated").count(), 1);
    }

    #[test]
    fn a_running_command_says_it_has_no_artifact_yet() {
        let full = nostr_event_lines(200);
        let preview = preview_tool_result(&full, full.len(), 200, 512, None);
        assert!(preview.text.contains("still running"));
        assert!(!preview.text.contains("Full result: artifact"));
        assert_eq!(preview.artifact, None);
    }

    #[test]
    fn clipping_never_splits_a_character() {
        let text = "→→→→→→→→→→\nnext line\n";
        for limit in 0..text.len() {
            let clipped = clip_to_line(text, limit);
            assert!(text.starts_with(clipped));
        }
    }

    #[test]
    fn versions_accumulate_and_stay_separately_addressable() {
        let mut store = ToolResultArtifactStore::new("terminal:x");
        let first = store.record("one\n");
        let second = store.record("one\ntwo\n");
        assert_ne!(first, second);
        assert_eq!(store.version_count(), 2);
        assert_eq!(
            store.get(&first).map(ToolResultArtifact::text),
            Some("one\n"),
            "an earlier version stopped resolving when a later one arrived"
        );
        assert_eq!(
            store.latest().map(ToolResultArtifact::text),
            Some("one\ntwo\n")
        );
    }

    #[test]
    fn recording_the_same_result_twice_does_not_invent_a_version() {
        let mut store = ToolResultArtifactStore::new("terminal:x");
        let first = store.record("same\n");
        let again = store.record("same\n");
        assert_eq!(first, again);
        assert_eq!(store.version_count(), 1);
    }

    #[test]
    fn an_unrecorded_address_resolves_to_nothing_rather_than_to_something_else() {
        let mut store = ToolResultArtifactStore::new("terminal:x");
        store.record("one\n");
        assert!(
            store
                .get(&ToolResultArtifactId::new("terminal:x", 9))
                .is_none()
        );
        assert!(
            store
                .get(&ToolResultArtifactId::new("terminal:y", 1))
                .is_none(),
            "an address from another source resolved here, so the source part \
             of an address is decorative"
        );
    }

    #[test]
    fn an_empty_result_has_no_lines_and_needs_no_marker() {
        let preview = preview_tool_result("", 0, 0, TOOL_RESULT_PREVIEW_BYTE_BUDGET, None);
        assert_eq!(preview.text, "");
        assert!(!preview.is_truncated());
        assert_eq!(preview.total_lines, 0);
    }
}
