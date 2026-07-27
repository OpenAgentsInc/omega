//! `OMEGA-DELTA-0111`. Spending the address a truncation marker hands out.
//!
//! `OMEGA-DELTA-0103` gave a bounded tool result a marker naming the bytes and
//! lines it withheld and an address to fetch the rest from. Nothing could take
//! that address. **A marker naming an artifact the model has no way to fetch is
//! worse than no marker**: the reader is told the rest is available, acts as
//! though it is, and gets nothing. This is the fetch path, so the address is
//! spendable.
//!
//! Two things constrain it, and both are `OMEGA-DELTA-0060`'s:
//!
//! 1. **Scope.** The tool holds this thread's registry and nothing else. It
//!    cannot name another thread's artifact, so a bug here cannot widen what a
//!    thread can read.
//! 2. **Size.** An artifact is the whole result, and the whole result is what
//!    did not fit in the first place. Every rendering is windowed by line and
//!    bounded by bytes, and every bound that fires says so — using the *same*
//!    truncation sentence the marker uses, so a reader never has to learn a
//!    second one.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use acp_thread::{ToolResultArtifact, preview_tool_result};
use agent_client_protocol::schema::v1 as acp;
use anyhow::Result;
use gpui::{App, SharedString, Task};
use language_model::LanguageModelToolResultContent;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::tool_result_artifacts::{
    ArtifactLookup, ToolResultArtifactRegistry, parse_artifact_address,
};
use crate::{AgentTool, ToolCallEventStream, ToolInput};

/// How many lines one call returns when it does not say.
pub const DEFAULT_LINE_LIMIT: usize = 200;

/// The most lines one call will return, whatever `limit` asks for.
pub const MAX_LINE_LIMIT: usize = 2_000;

/// The ceiling on one rendered window, in bytes.
///
/// The backstop that makes the tool safe to call without first knowing how wide
/// the artifact's lines are. A line limit alone does not bound the total,
/// because a line is not bounded — the result this exists for was unwrapped
/// JSON, which is one line of forty thousand bytes as easily as forty lines of
/// a thousand.
pub const MAX_WINDOW_BYTES: usize = 24_000;

/// Read the complete text of a tool result that was truncated in your context.
///
/// ### When to use this
/// - When a tool result you have ends in a truncation marker naming an artifact,
///   and you need what the marker withheld.
/// - Not routinely. The preview is the default for a reason; fetch when the
///   part you need is behind the marker.
///
/// ### Addresses
/// - Copy the address out of the marker exactly, including the `@v<n>` version.
///   It looks like `tool:4:toolu_01abc@v1` or `terminal:2@v3`.
/// - Do not compose one. A version counts captures of that result, and guessing
///   at it either misses or reads a different capture than the one you saw.
///
/// ### Output
/// - A header naming the address, the line range returned, and the totals, so
///   you can page with `offset`.
/// - Every bound that fired is marked in the text. If you see a truncation
///   marker, the content behind it exists — narrow the window and ask again
///   rather than concluding it is absent.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct ReadToolResultArtifactToolInput {
    /// The artifact address from the truncation marker, e.g. `tool:4:toolu_01abc@v1`.
    pub artifact: String,
    /// Zero-based index of the first line to return. Defaults to 0.
    #[serde(default)]
    pub offset: Option<usize>,
    /// How many lines to return, at most 2000. Defaults to 200.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// The range of lines a call asks for, already clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactWindowRequest {
    pub offset: usize,
    pub limit: usize,
}

impl ArtifactWindowRequest {
    /// A missing limit is [`DEFAULT_LINE_LIMIT`]; a zero limit would return a
    /// window nobody can learn anything from, so it becomes the default too;
    /// anything above [`MAX_LINE_LIMIT`] is capped.
    #[must_use]
    pub const fn clamp(offset: Option<usize>, limit: Option<usize>) -> Self {
        let limit = match limit {
            Some(0) | None => DEFAULT_LINE_LIMIT,
            Some(limit) if limit > MAX_LINE_LIMIT => MAX_LINE_LIMIT,
            Some(limit) => limit,
        };
        Self {
            offset: match offset {
                Some(offset) => offset,
                None => 0,
            },
            limit,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadToolResultArtifactToolOutput {
    Artifact {
        address: String,
        total_lines: usize,
        total_bytes: usize,
        first_line: usize,
        rendered: String,
    },
    /// The address could not be read, or read fine and did not resolve. Either
    /// way the reason is already the sentence the model should read.
    Unavailable { address: String, reason: String },
}

impl From<ReadToolResultArtifactToolOutput> for LanguageModelToolResultContent {
    fn from(output: ReadToolResultArtifactToolOutput) -> Self {
        match output {
            ReadToolResultArtifactToolOutput::Artifact { rendered, .. } => rendered.into(),
            ReadToolResultArtifactToolOutput::Unavailable { reason, .. } => reason.into(),
        }
    }
}

/// Render `artifact`'s `window` into text for the model.
///
/// Two bounds fire here and they are reported separately, because they are
/// asked for separately: the *line* window is the caller's own `limit`, and the
/// *byte* ceiling is this tool's backstop. Collapsing them into one sentence
/// would tell a caller that asked for 200 lines and got 40 that its limit was
/// honoured.
///
/// The byte ceiling reuses `preview_tool_result`, so the sentence a reader sees
/// when a fetch is itself truncated is the same sentence that sent them here.
/// A second, differently worded truncation sentence is how a reader learns to
/// skip both.
#[must_use]
pub fn render_artifact_window(
    artifact: &ToolResultArtifact,
    window: ArtifactWindowRequest,
    max_bytes: usize,
) -> String {
    let total_lines = artifact.line_count();
    let total_bytes = artifact.byte_count();
    let address = artifact.id();

    if window.offset >= total_lines {
        return format!(
            "Artifact {address} has {total_lines} lines and {total_bytes} bytes; \
             offset {} is past its end. The result is here — ask again with an \
             offset below {total_lines}.",
            window.offset
        );
    }

    let lines: Vec<&str> = artifact.text().lines().collect();
    let end = window.offset.saturating_add(window.limit).min(total_lines);
    let body = lines[window.offset..end].join("\n");

    // The byte backstop, in the law's own words. Totals are this window's, not
    // the artifact's: the window's own remainder is stated by the footer below,
    // and reporting the artifact's total here would count the same withheld
    // lines twice.
    let bounded = preview_tool_result(
        &body,
        body.len(),
        end - window.offset,
        max_bytes,
        Some(address.clone()),
    );

    // When the byte backstop did not fire, the window showed exactly the lines
    // it selected. Counting the rendered text instead would report a run of
    // blank lines as no lines at all, and the footer would then hand back the
    // offset it was already given — a page instruction that never advances.
    let shown_lines = if bounded.is_truncated() {
        bounded.shown_lines
    } else {
        end - window.offset
    };
    let last_shown = window.offset + shown_lines;
    let mut rendered = format!(
        "Artifact {address}: lines {}–{} of {total_lines} ({total_bytes} bytes total).\n{}",
        window.offset,
        last_shown.saturating_sub(1),
        bounded.text,
    );

    // The falsifier's target. Without this a partial read is indistinguishable
    // from a complete one, and a reader who reaches the last line concludes
    // they have the whole result.
    if last_shown < total_lines {
        rendered.push_str(&format!(
            "\n… [{} more lines in this artifact; read them with offset: {last_shown}.]",
            total_lines - last_shown,
        ));
    }
    rendered
}

pub struct ReadToolResultArtifactTool {
    artifacts: Rc<RefCell<ToolResultArtifactRegistry>>,
}

impl ReadToolResultArtifactTool {
    pub fn new(artifacts: Rc<RefCell<ToolResultArtifactRegistry>>) -> Self {
        Self { artifacts }
    }

    pub fn read(
        &self,
        input: ReadToolResultArtifactToolInput,
        event_stream: &ToolCallEventStream,
    ) -> Result<ReadToolResultArtifactToolOutput, ReadToolResultArtifactToolOutput> {
        let given = input.artifact.clone();
        let address = parse_artifact_address(&given).map_err(|error| {
            ReadToolResultArtifactToolOutput::Unavailable {
                address: given.clone(),
                reason: error.sentence(&given),
            }
        })?;

        let window = ArtifactWindowRequest::clamp(input.offset, input.limit);
        let registry = self.artifacts.borrow();
        let found = match registry.lookup(&address) {
            ArtifactLookup::Found(found) => found,
            other => {
                let reason = other
                    .sentence(&address)
                    .unwrap_or_else(|| format!("No tool result is recorded at `{address}`."));
                return Err(ReadToolResultArtifactToolOutput::Unavailable {
                    address: address.to_string(),
                    reason,
                });
            }
        };
        let rendered = render_artifact_window(found, window, MAX_WINDOW_BYTES);
        event_stream
            .update_fields(acp::ToolCallUpdateFields::new().content(vec![rendered.clone().into()]));
        Ok(ReadToolResultArtifactToolOutput::Artifact {
            address: address.to_string(),
            total_lines: found.line_count(),
            total_bytes: found.byte_count(),
            first_line: window.offset,
            rendered,
        })
    }
}

impl AgentTool for ReadToolResultArtifactTool {
    type Input = ReadToolResultArtifactToolInput;
    type Output = ReadToolResultArtifactToolOutput;

    const NAME: &'static str = "read_tool_result_artifact";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Read
    }

    /// This tool's result is already windowed and already carries its own
    /// truncation markers. Bounding it again in `Thread::run_tool` would clip
    /// the footer that names the next offset, so a fetch would stop saying how
    /// to continue exactly when continuing is what it is for.
    fn bounds_own_result() -> bool {
        true
    }

    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        match input {
            Ok(input) => format!("Read tool result artifact {}", input.artifact).into(),
            Err(_) => "Read tool result artifact".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|error| {
                ReadToolResultArtifactToolOutput::Unavailable {
                    address: "<unparsed>".to_owned(),
                    reason: error.to_string(),
                }
            })?;

            self.read(input, &event_stream)
        })
    }

    fn replay(
        &self,
        _input: Self::Input,
        output: Self::Output,
        event_stream: ToolCallEventStream,
        _cx: &mut App,
    ) -> Result<()> {
        let content = match output {
            ReadToolResultArtifactToolOutput::Artifact { rendered, .. } => rendered,
            ReadToolResultArtifactToolOutput::Unavailable { reason, .. } => reason,
        };
        event_stream.update_fields(acp::ToolCallUpdateFields::new().content(vec![content.into()]));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_result_artifacts::tool_result_artifact_source;
    use acp_thread::ToolResultArtifactId;

    fn blob(lines: usize) -> String {
        (0..lines)
            .map(|index| format!("line {index}: {}", "x".repeat(40)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn recorded(text: &str) -> (ToolResultArtifactRegistry, ToolResultArtifactId) {
        let mut registry = ToolResultArtifactRegistry::default();
        let address = registry
            .bound(&tool_result_artifact_source("4:toolu_01abc"), text)
            .artifact
            .expect("a result over the budget is recorded");
        (registry, address)
    }

    #[test]
    fn the_address_a_marker_hands_out_resolves_to_the_whole_result() {
        // The point of the whole tool: the marker's address is spendable.
        let full = blob(400);
        let (registry, address) = recorded(&full);

        let rendered = render_artifact_window(
            registry.lookup(&address).artifact().expect("resolves"),
            ArtifactWindowRequest::clamp(Some(0), Some(MAX_LINE_LIMIT)),
            MAX_WINDOW_BYTES * 8,
        );

        assert!(rendered.contains("line 0:"));
        assert!(
            rendered.contains("line 399:"),
            "the fetch path does not reach the end of the result it addresses"
        );
        assert!(
            !rendered.contains("more lines in this artifact"),
            "a complete read claims there is more: {}",
            &rendered[rendered.len().saturating_sub(200)..]
        );
    }

    #[test]
    fn a_partial_read_is_distinguishable_from_a_complete_one() {
        // The falsifier. Delete the footer and these two become the same shape,
        // and a reader who reaches the last line concludes they have it all.
        let full = blob(400);
        let (registry, address) = recorded(&full);
        let lookup = registry.lookup(&address);
        let artifact = lookup.artifact().expect("resolves");

        let partial = render_artifact_window(
            artifact,
            ArtifactWindowRequest::clamp(Some(0), Some(50)),
            MAX_WINDOW_BYTES * 8,
        );
        let complete = render_artifact_window(
            artifact,
            ArtifactWindowRequest::clamp(Some(0), Some(MAX_LINE_LIMIT)),
            MAX_WINDOW_BYTES * 8,
        );

        assert!(
            partial.contains("350 more lines in this artifact"),
            "a partial read does not say how much it withheld: {partial}"
        );
        assert!(
            partial.contains("offset: 50"),
            "a partial read does not name the offset to continue with: {partial}"
        );
        assert!(!complete.contains("more lines in this artifact"));
    }

    #[test]
    fn the_next_offset_a_partial_read_names_is_the_line_it_stopped_at() {
        let full = blob(400);
        let (registry, address) = recorded(&full);
        let lookup = registry.lookup(&address);
        let artifact = lookup.artifact().expect("resolves");

        let first = render_artifact_window(
            artifact,
            ArtifactWindowRequest::clamp(Some(0), Some(50)),
            MAX_WINDOW_BYTES * 8,
        );
        assert!(first.contains("line 49:"));
        assert!(!first.contains("line 50:"));

        let second = render_artifact_window(
            artifact,
            ArtifactWindowRequest::clamp(Some(50), Some(50)),
            MAX_WINDOW_BYTES * 8,
        );
        assert!(
            second.contains("line 50:"),
            "paging with the offset the previous read named skipped a line, so \
             the two windows do not join up"
        );
        assert!(second.contains("Artifact tool:4:toolu_01abc@v1: lines 50–99 of 400"));
    }

    #[test]
    fn a_byte_ceiling_inside_the_window_speaks_the_laws_own_sentence() {
        // A caller can ask for 2000 lines of unwrapped JSON. The line window is
        // not a byte bound, so the backstop has to fire — and when it does it
        // must use the marker a reader already knows.
        let full = blob(2_000);
        let (registry, address) = recorded(&full);

        let rendered = render_artifact_window(
            registry.lookup(&address).artifact().expect("resolves"),
            ArtifactWindowRequest::clamp(Some(0), Some(MAX_LINE_LIMIT)),
            MAX_WINDOW_BYTES,
        );

        assert!(
            rendered.contains("tool result truncated"),
            "the byte backstop fired silently: {}",
            &rendered[..200.min(rendered.len())]
        );
        assert!(rendered.contains(&format!("artifact {address}")));
        assert!(
            rendered.contains("more lines in this artifact"),
            "the byte backstop cut the window and the footer still called it \
             complete: {rendered}"
        );
    }

    #[test]
    fn the_footer_counts_what_the_byte_ceiling_withheld_and_not_only_the_line_limit() {
        let full = blob(2_000);
        let (registry, address) = recorded(&full);

        let rendered = render_artifact_window(
            registry.lookup(&address).artifact().expect("resolves"),
            ArtifactWindowRequest::clamp(Some(0), Some(MAX_LINE_LIMIT)),
            MAX_WINDOW_BYTES,
        );

        // Each line is about 48 bytes, so a 24,000-byte ceiling stops well
        // short of the 2,000 lines asked for. The offset the footer names must
        // be where the *bytes* stopped, not where the line limit would have.
        let next: usize = rendered
            .rsplit_once("offset: ")
            .and_then(|(_, rest)| rest.split_once('.'))
            .and_then(|(digits, _)| digits.parse().ok())
            .expect("the footer names a next offset");
        assert!(
            next < MAX_LINE_LIMIT,
            "the footer named offset {next}, which is past what the byte \
             ceiling actually showed — paging from it skips the middle"
        );
        assert!(
            !rendered.contains(&format!("line {next}:")),
            "the footer's next offset names a line the window already showed, \
             so paging from it repeats content"
        );
        assert!(
            rendered.contains(&format!("line {}:", next - 1)),
            "the footer's next offset is not the line after the last one shown"
        );
    }

    #[test]
    fn an_offset_past_the_end_says_the_result_is_still_there() {
        let full = blob(400);
        let (registry, address) = recorded(&full);

        let rendered = render_artifact_window(
            registry.lookup(&address).artifact().expect("resolves"),
            ArtifactWindowRequest::clamp(Some(9_000), None),
            MAX_WINDOW_BYTES,
        );

        assert!(rendered.contains("400 lines"));
        assert!(
            rendered.contains("The result is here"),
            "an over-long offset reads as an empty result: {rendered}"
        );
    }

    #[test]
    fn a_window_request_is_clamped_rather_than_honoured_unbounded() {
        assert_eq!(
            ArtifactWindowRequest::clamp(None, None),
            ArtifactWindowRequest {
                offset: 0,
                limit: DEFAULT_LINE_LIMIT
            }
        );
        assert_eq!(
            ArtifactWindowRequest::clamp(Some(3), Some(0)).limit,
            DEFAULT_LINE_LIMIT,
            "a zero limit returned a window nobody can learn anything from"
        );
        assert_eq!(
            ArtifactWindowRequest::clamp(None, Some(usize::MAX)).limit,
            MAX_LINE_LIMIT
        );
    }

    #[test]
    fn the_header_states_the_totals_so_paging_is_a_decision_and_not_a_guess() {
        let full = blob(400);
        let (registry, address) = recorded(&full);

        let rendered = render_artifact_window(
            registry.lookup(&address).artifact().expect("resolves"),
            ArtifactWindowRequest::clamp(Some(0), Some(10)),
            MAX_WINDOW_BYTES,
        );

        assert!(rendered.starts_with(&format!("Artifact {address}: lines 0–9 of 400 (")));
        assert!(rendered.contains(&format!("{} bytes total", full.len())));
    }
}
