//! Reading the transcript of a subagent you spawned.
//!
//! `spawn_agent` returns one final message. That is the right default — the
//! whole point of delegating is that the subagent's intermediate work stays out
//! of the parent's context — but it leaves the parent with no way to check the
//! work. If the summary is thin or wrong, the parent's only move is to delegate
//! again and hope. This tool is the way to look.
//!
//! Two things constrain it, and both are load-bearing:
//!
//! 1. **Scope.** A thread reads only the subagents it spawned itself. The
//!    decision is [`subagent_transcript_access`], a total function over
//!    (caller, target, target's parent). The tool never names a thread; the
//!    environment knows which thread is asking and answers for that one, so a
//!    bug in the tool cannot widen the scope.
//! 2. **Size.** A transcript can be larger than the context it is being read
//!    into. Every rendering is bounded, and every bound that fires says so in
//!    the output. Silently dropping the middle of a transcript is how a reader
//!    concludes something did not happen when it did.

use agent_client_protocol::schema::v1 as acp;
use anyhow::Result;
use gpui::{App, SharedString, Task};
use language_model::LanguageModelToolResultContent;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::rc::Rc;
use std::sync::Arc;

use crate::{AgentTool, ThreadEnvironment, ToolCallEventStream, ToolInput};

/// How many messages a call returns when it does not say.
///
/// Small on purpose. A parent that wants more can page with `offset`, and the
/// header always says how many messages there are, so paging is a decision made
/// with the total in hand rather than a guess.
pub const DEFAULT_MESSAGE_LIMIT: usize = 20;

/// The most messages one call will return, whatever `limit` asks for.
pub const MAX_MESSAGE_LIMIT: usize = 100;

/// Bytes of any single text, thinking, tool-input or tool-result block that
/// `full` detail renders before it truncates that block.
pub const FULL_BLOCK_BYTE_LIMIT: usize = 2_000;

/// The same limit for `outline` detail, which is meant to answer "what did it
/// do" rather than "what exactly did it say".
pub const OUTLINE_BLOCK_BYTE_LIMIT: usize = 200;

/// The ceiling on one rendered transcript, across all messages in the window.
///
/// This is the backstop that makes the tool safe to call without first knowing
/// how big the subagent's work was. Per-block limits alone do not bound the
/// total, because the number of blocks is not bounded.
pub const MAX_TRANSCRIPT_BYTES: usize = 24_000;

/// Read the transcript of a subagent that **this thread** spawned, to inspect
/// delegated work: which tools the subagent ran, what it read, and where it
/// went wrong.
///
/// ### When to use this
/// - The final message from `spawn_agent` is usually enough. This is not a
///   routine follow-up to every delegation.
/// - Reach for it when the summary is thin, looks wrong, contradicts something
///   you know, or when the subagent reported an error and you need to see what
///   it actually did before delegating again.
/// - Prefer `detail: "outline"` first. It lists the tool calls and the shape of
///   the turn cheaply. Only ask for `detail: "full"` on a narrow `offset`/`limit`
///   window once the outline shows you where to look.
///
/// ### What you can read
/// - Only subagents this thread spawned. A session belonging to another thread
///   is refused, and the refusal says so.
/// - Session IDs come from `spawn_agent`. A session ID you saw quoted inside
///   another transcript is not yours to read.
///
/// ### Output
/// - A header naming the session, the message range returned, and the total
///   number of messages, so you can page.
/// - Every bound that fired is marked in the text. If you see a truncation
///   marker, the content behind it exists — narrow the window and ask again
///   rather than concluding it is absent.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct ReadSubagentTranscriptToolInput {
    /// Session ID of the subagent to read, as returned by `spawn_agent`.
    pub session_id: acp::SessionId,
    /// Zero-based index of the first message to return. Defaults to 0.
    #[serde(default)]
    pub offset: Option<usize>,
    /// How many messages to return, at most 100. Defaults to 20.
    #[serde(default)]
    pub limit: Option<usize>,
    /// How much of each message to render. `outline` lists tool calls and
    /// clips every block hard; `full` renders bodies up to a per-block limit.
    #[serde(default)]
    pub detail: TranscriptDetail,
}

/// How much of each message a rendering shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptDetail {
    /// Tool names, input shape and result sizes; every block clipped to
    /// [`OUTLINE_BLOCK_BYTE_LIMIT`]. The cheap default.
    #[default]
    Outline,
    /// Bodies, clipped to [`FULL_BLOCK_BYTE_LIMIT`] each.
    Full,
}

impl TranscriptDetail {
    #[must_use]
    pub const fn block_byte_limit(self) -> usize {
        match self {
            Self::Outline => OUTLINE_BLOCK_BYTE_LIMIT,
            Self::Full => FULL_BLOCK_BYTE_LIMIT,
        }
    }
}

/// The range of messages a call asks for, already clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranscriptWindowRequest {
    pub offset: usize,
    pub limit: usize,
}

impl TranscriptWindowRequest {
    /// Clamp a caller's `offset`/`limit` to something the tool will honour.
    ///
    /// A missing limit is [`DEFAULT_MESSAGE_LIMIT`]; a zero limit would return
    /// a window nobody can learn anything from, so it becomes the default too;
    /// anything above [`MAX_MESSAGE_LIMIT`] is capped.
    #[must_use]
    pub const fn clamp(offset: Option<usize>, limit: Option<usize>) -> Self {
        let limit = match limit {
            Some(0) | None => DEFAULT_MESSAGE_LIMIT,
            Some(limit) if limit > MAX_MESSAGE_LIMIT => MAX_MESSAGE_LIMIT,
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

/// Whether a thread may read another thread's transcript.
///
/// Every refusal names its reason. A caller debugging its own delegation is
/// told the thread is not its own rather than that it does not exist: pretending
/// a real session is missing sends the caller looking for a bug that is not
/// there, and it does not withhold anything, because the caller already had the
/// session ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptAccess {
    /// The target is a subagent of the caller.
    Granted,
    /// The caller asked for itself. Its own transcript is already its context.
    RefusedIsCaller,
    /// The target is a top-level thread — nobody spawned it.
    RefusedNotASubagent,
    /// The target is a subagent of some other thread.
    RefusedOtherParent { parent: acp::SessionId },
}

impl TranscriptAccess {
    #[must_use]
    pub const fn is_granted(&self) -> bool {
        matches!(self, Self::Granted)
    }

    /// The sentence the model is shown when the read is refused.
    #[must_use]
    pub fn refusal(&self, target: &acp::SessionId) -> Option<String> {
        match self {
            Self::Granted => None,
            Self::RefusedIsCaller => Some(format!(
                "Session {target} is this thread. Your own transcript is already \
                 your context; this tool reads subagents you spawned."
            )),
            Self::RefusedNotASubagent => Some(format!(
                "Session {target} is not a subagent of this thread — it is a \
                 top-level thread, so this thread did not spawn it. You can \
                 read only the subagents you spawned yourself."
            )),
            Self::RefusedOtherParent { parent } => Some(format!(
                "Session {target} is a subagent of thread {parent}, not of this \
                 thread. You can read only the subagents you spawned yourself. \
                 If you saw this session ID quoted inside another transcript, it \
                 belongs to that agent's delegation, not yours."
            )),
        }
    }
}

/// Decide whether `caller` may read `target`, given the thread that spawned
/// `target` (`None` when `target` is top-level).
///
/// Total by construction: every combination of the two inputs lands on a named
/// variant, and there is no fallthrough arm that could quietly become "allow".
///
/// **Direct children only.** The parent is compared once; the ancestor chain is
/// never walked. Today `MAX_SUBAGENT_DEPTH` is 1, so there are no grandchildren
/// to walk to — but the rule is written for the day that constant changes,
/// because this tool is exactly what makes a grandchild's session ID visible to
/// a root thread. Once a parent can read its child's transcript, every session
/// ID the child mentions becomes quotable, and transitive access would turn
/// "read what you delegated" into "read anything named by anything you
/// delegated". If nested reads are ever wanted, they should be a separate,
/// argued change to this function, not a side effect of raising the depth
/// constant.
#[must_use]
pub fn subagent_transcript_access(
    caller: &acp::SessionId,
    target: &acp::SessionId,
    target_parent: Option<&acp::SessionId>,
) -> TranscriptAccess {
    if caller == target {
        return TranscriptAccess::RefusedIsCaller;
    }
    match target_parent {
        None => TranscriptAccess::RefusedNotASubagent,
        Some(parent) if parent == caller => TranscriptAccess::Granted,
        Some(parent) => TranscriptAccess::RefusedOtherParent {
            parent: parent.clone(),
        },
    }
}

/// The sentence for a session Omega holds no transcript for.
///
/// Live external ACP sessions spawned by this parent are retained by its
/// `ThreadEnvironment` and are readable. After that parent or process is gone,
/// however, the external agent owns the transcript and Omega has no durable
/// native thread to restore.
///
/// **It says both possibilities and asserts neither, and that is deliberate.**
/// A definite answer — "session X ran on Codex" — would need Omega to remember
/// which sessions it opened externally. That memory would be in process
/// memory only: an external subagent has no `DbThread`, so nothing about it
/// survives a reload. After a restart the same lookup would confidently answer
/// "that is not an external subagent" about one that is, which is a wrong
/// answer where this is merely an incomplete one. `OMEGA-DELTA-0061`'s own
/// rule against two sources for one question applies here: an honest
/// disjunction beats a second store that is right until the process ends.
///
/// What it must not do is read as "you got the ID wrong". That was the sentence
/// before external subagents existed, and it sends a caller holding a perfectly
/// good session ID looking for a bug in its own bookkeeping.
#[must_use]
pub fn no_transcript_available(session_id: &acp::SessionId) -> String {
    format!(
        concat!(
            "No transcript is available for session {session_id}. If this was an ",
            "external executor such as `codex-acp` from an earlier Omega process, ",
            "its live session is no longer retained here; you have its ",
            "final message",
            " only. Otherwise, check the ID — session IDs come from `delegate`."
        ),
        session_id = session_id
    )
}

/// Who produced a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptRole {
    User,
    Assistant,
    Resume,
    Compaction,
}

impl TranscriptRole {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Resume => "resume",
            Self::Compaction => "compaction",
        }
    }
}

/// One piece of a message, flattened out of the thread's own representation so
/// the renderer does not depend on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptBlock {
    Text(String),
    Thinking(String),
    ToolUse {
        name: String,
        id: String,
        input: String,
    },
    ToolResult {
        name: String,
        id: String,
        is_error: bool,
        text: String,
    },
    /// An image, which is never rendered into the parent's context.
    Image,
}

/// One message of a subagent's transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptEntry {
    /// Index of this message in the subagent's full transcript, so a truncation
    /// marker can name a real `offset` to ask again with.
    pub index: usize,
    pub role: TranscriptRole,
    pub blocks: Vec<TranscriptBlock>,
}

/// A subagent's transcript, already narrowed to the requested message range.
///
/// Narrowing by *message* happens where the thread is read; bounding by *bytes*
/// happens in [`render_transcript`]. Keeping the two apart means the byte
/// bounds are testable without a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentTranscript {
    pub session_id: acp::SessionId,
    pub title: String,
    /// Messages in the whole transcript, not in this window.
    pub total_messages: usize,
    /// Index of `entries[0]` in the whole transcript.
    pub first_index: usize,
    pub entries: Vec<TranscriptEntry>,
}

/// Clip `text` to at most `limit` bytes without splitting a character.
fn clip(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Append `text`, clipped to `limit` bytes, marking the clip when it fires.
fn push_clipped(out: &mut String, text: &str, limit: usize) {
    let clipped = clip(text, limit);
    out.push_str(clipped);
    if clipped.len() < text.len() {
        let _ = write!(
            out,
            "\n… [block truncated: {} of {} bytes shown]",
            clipped.len(),
            text.len()
        );
    }
}

/// Render one message. Separated out so the byte cap can be applied by
/// measuring a whole message before committing it.
fn render_entry(entry: &TranscriptEntry, detail: TranscriptDetail) -> String {
    let limit = detail.block_byte_limit();
    let mut out = String::new();
    let _ = writeln!(out, "[{}] {}", entry.index, entry.role.label());

    if entry.blocks.is_empty() {
        out.push_str("  (empty)\n");
        return out;
    }

    for block in &entry.blocks {
        match block {
            TranscriptBlock::Text(text) => {
                out.push_str("  ");
                push_clipped(&mut out, text, limit);
                out.push('\n');
            }
            TranscriptBlock::Thinking(text) => {
                if detail == TranscriptDetail::Full {
                    out.push_str("  <thinking> ");
                    push_clipped(&mut out, text, limit);
                    out.push('\n');
                } else {
                    let _ = writeln!(out, "  <thinking, {} bytes>", text.len());
                }
            }
            TranscriptBlock::ToolUse { name, id, input } => {
                let _ = write!(out, "  → tool {name} ({id}) ");
                push_clipped(&mut out, input, limit);
                out.push('\n');
            }
            TranscriptBlock::ToolResult {
                name,
                id,
                is_error,
                text,
            } => {
                let marker = if *is_error { " ERROR" } else { "" };
                let _ = write!(
                    out,
                    "  ← tool {name} ({id}){marker} [{} bytes] ",
                    text.len()
                );
                push_clipped(&mut out, text, limit);
                out.push('\n');
            }
            TranscriptBlock::Image => out.push_str("  <image>\n"),
        }
    }
    out
}

/// Render a window of transcript into text for the model, bounded by
/// `byte_cap`.
///
/// Both bounds are visible in the result. The header says which messages the
/// window covers and how many exist; if the byte cap stops the rendering early,
/// the last line names the messages that were dropped and the `offset` to
/// resume from. A reader is never left to infer absence from silence.
#[must_use]
pub fn render_transcript(
    transcript: &SubagentTranscript,
    detail: TranscriptDetail,
    byte_cap: usize,
) -> String {
    let mut out = String::new();
    let last_requested = transcript
        .first_index
        .saturating_add(transcript.entries.len());
    let _ = writeln!(
        out,
        "Transcript of subagent {} ({})",
        transcript.session_id, transcript.title
    );
    let _ = writeln!(
        out,
        "Messages {}–{} of {} total. Detail: {}.",
        transcript.first_index,
        last_requested.saturating_sub(1),
        transcript.total_messages,
        match detail {
            TranscriptDetail::Outline => "outline",
            TranscriptDetail::Full => "full",
        }
    );

    if transcript.entries.is_empty() {
        out.push_str(
            "\nNo messages in this window. The subagent has \
             produced nothing at this offset.\n",
        );
        return out;
    }

    out.push('\n');

    let mut rendered_through = transcript.first_index;
    for entry in &transcript.entries {
        let piece = render_entry(entry, detail);
        // Reserve room for the marker itself, so the cap can never be the
        // reason the reader is not told about the cap.
        if out.len() + piece.len() > byte_cap.saturating_sub(TRUNCATION_MARKER_RESERVE) {
            let dropped = last_requested.saturating_sub(entry.index);
            let _ = writeln!(
                out,
                "[transcript truncated at the {byte_cap}-byte cap: {dropped} \
                 message(s) of this window were not rendered, starting at index \
                 {}. Ask again with offset={} — or with detail=\"outline\" — to \
                 see them.]",
                entry.index, entry.index
            );
            return out;
        }
        out.push_str(&piece);
        rendered_through = entry.index.saturating_add(1);
    }

    if rendered_through < transcript.total_messages {
        let _ = writeln!(
            out,
            "\n[{} more message(s) after this window. Ask again with offset={}.]",
            transcript.total_messages.saturating_sub(rendered_through),
            rendered_through
        );
    }

    out
}

/// Bytes held back from `byte_cap` so the truncation marker always fits.
const TRUNCATION_MARKER_RESERVE: usize = 320;

/// Tool that reads the transcript of a subagent the calling thread spawned.
pub struct ReadSubagentTranscriptTool {
    environment: Rc<dyn ThreadEnvironment>,
}

impl ReadSubagentTranscriptTool {
    pub fn new(environment: Rc<dyn ThreadEnvironment>) -> Self {
        Self { environment }
    }

    pub fn read(
        &self,
        input: ReadSubagentTranscriptToolInput,
        event_stream: &ToolCallEventStream,
        cx: &mut App,
    ) -> Result<ReadSubagentTranscriptToolOutput, ReadSubagentTranscriptToolOutput> {
        let session_id = input.session_id.clone();
        let window = TranscriptWindowRequest::clamp(input.offset, input.limit);
        let transcript = self
            .environment
            .read_subagent_transcript(session_id.clone(), window, cx)
            .map_err(|reason| ReadSubagentTranscriptToolOutput::Refused {
                session_id: session_id.clone(),
                reason,
            })?;
        let rendered = render_transcript(&transcript, input.detail, MAX_TRANSCRIPT_BYTES);
        event_stream
            .update_fields(acp::ToolCallUpdateFields::new().content(vec![rendered.clone().into()]));
        Ok(ReadSubagentTranscriptToolOutput::Transcript {
            session_id,
            total_messages: transcript.total_messages,
            first_index: transcript.first_index,
            rendered,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadSubagentTranscriptToolOutput {
    Transcript {
        session_id: acp::SessionId,
        total_messages: usize,
        first_index: usize,
        rendered: String,
    },
    Refused {
        session_id: acp::SessionId,
        reason: String,
    },
}

impl From<ReadSubagentTranscriptToolOutput> for LanguageModelToolResultContent {
    fn from(output: ReadSubagentTranscriptToolOutput) -> Self {
        match output {
            ReadSubagentTranscriptToolOutput::Transcript { rendered, .. } => rendered.into(),
            ReadSubagentTranscriptToolOutput::Refused { reason, .. } => reason.into(),
        }
    }
}

impl AgentTool for ReadSubagentTranscriptTool {
    type Input = ReadSubagentTranscriptToolInput;
    type Output = ReadSubagentTranscriptToolOutput;

    const NAME: &'static str = "read_subagent_transcript";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Read
    }

    fn bounds_own_result() -> bool {
        // `OMEGA-DELTA-0060` bounds this to `MAX_TRANSCRIPT_BYTES` and marks
        // every bound that fires, including the message index to ask again
        // with. A second bound at 4,000 bytes would cut those markers off and
        // leave a reader who paged to the end believing they had the transcript.
        true
    }

    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        match input {
            Ok(input) => format!("Read subagent transcript {}", input.session_id).into(),
            Err(_) => "Read subagent transcript".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            // A malformed input has no session to name, so the refusal carries
            // the parse error instead of inventing an ID.
            let input =
                input
                    .recv()
                    .await
                    .map_err(|error| ReadSubagentTranscriptToolOutput::Refused {
                        session_id: acp::SessionId::from("<unparsed>"),
                        reason: error.to_string(),
                    })?;

            cx.update(|cx| self.read(input, &event_stream, cx))
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
            ReadSubagentTranscriptToolOutput::Transcript { rendered, .. } => rendered,
            ReadSubagentTranscriptToolOutput::Refused { reason, .. } => reason,
        };
        event_stream.update_fields(acp::ToolCallUpdateFields::new().content(vec![content.into()]));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str) -> acp::SessionId {
        acp::SessionId::from(id.to_string())
    }

    fn transcript(entries: Vec<TranscriptEntry>, total: usize, first: usize) -> SubagentTranscript {
        SubagentTranscript {
            session_id: session("sub"),
            title: "Researching".into(),
            total_messages: total,
            first_index: first,
            entries,
        }
    }

    fn text_entry(index: usize, body: &str) -> TranscriptEntry {
        TranscriptEntry {
            index,
            role: TranscriptRole::Assistant,
            blocks: vec![TranscriptBlock::Text(body.into())],
        }
    }

    // --- A session Omega holds no transcript for. ---

    /// `OMEGA-DELTA-0061` made "not in the session map" mean two different
    /// things. The sentence has to cover both without asserting either.
    #[test]
    fn a_missing_transcript_names_both_reasons_and_asserts_neither() {
        let message = no_transcript_available(&session("sub-7"));

        assert!(
            message.contains("sub-7"),
            "the sentence must name the session asked for: {message}"
        );
        // The external case, said as a condition.
        assert!(
            message.contains("external executor"),
            "a correct ID for an external subagent must not read as a bad ID: \
             {message}"
        );
        assert!(
            message.contains("codex-acp"),
            "the condition must be concrete enough to recognise: {message}"
        );
        // What the parent still has, so it does not conclude the work is gone.
        assert!(
            message.contains("final message"),
            "the parent keeps the final message, and must be told so: {message}"
        );
        // The other case survives. Before this delta it was the only one, and
        // dropping it would send a caller with a genuine typo hunting an
        // external subagent it never spawned.
        assert!(
            message.contains("check the ID"),
            "a genuinely wrong ID must still be a named possibility: {message}"
        );

        // And it claims neither. A definite "this ran on Codex" would need
        // durable memory of external sessions, which do not have any.
        for asserted in [
            "This was a subagent you ran",
            "ran on an external executor.",
            "does not exist",
        ] {
            assert!(
                !message.contains(asserted),
                "the sentence asserts `{asserted}` about a session it cannot \
                 classify: {message}"
            );
        }
    }

    /// It is a different sentence from every scoping refusal.
    ///
    /// The two are reached by different paths and mean different things —
    /// "Omega never had this" against "this is not yours" — and a caller that
    /// cannot tell them apart cannot tell a bookkeeping bug from a permission
    /// boundary.
    #[test]
    fn a_missing_transcript_is_not_a_scoping_refusal() {
        let target = session("sub-7");
        let missing = no_transcript_available(&target);

        for access in [
            TranscriptAccess::RefusedIsCaller,
            TranscriptAccess::RefusedNotASubagent,
            TranscriptAccess::RefusedOtherParent {
                parent: session("other"),
            },
        ] {
            let refusal = access.refusal(&target).expect("must refuse");
            assert_ne!(refusal, missing);
            assert!(
                !missing.contains("You can read only the subagents you spawned"),
                "a session Omega does not hold must not be reported as a \
                 permission decision: {missing}"
            );
        }
    }

    // --- Scoping. The boundary that matters. ---

    #[test]
    fn a_parent_reads_the_subagent_it_spawned() {
        let access = subagent_transcript_access(
            &session("parent"),
            &session("child"),
            Some(&session("parent")),
        );
        assert_eq!(access, TranscriptAccess::Granted);
        assert!(access.is_granted());
        assert!(access.refusal(&session("child")).is_none());
    }

    #[test]
    fn a_thread_cannot_read_a_subagent_it_did_not_spawn() {
        let access = subagent_transcript_access(
            &session("stranger"),
            &session("child"),
            Some(&session("parent")),
        );
        assert_eq!(
            access,
            TranscriptAccess::RefusedOtherParent {
                parent: session("parent")
            }
        );
        assert!(!access.is_granted());

        // The refusal says whose it is rather than claiming it is missing. A
        // caller debugging its own delegation must not be sent looking for a
        // bug that is not there.
        let refusal = access.refusal(&session("child")).expect("refused");
        assert!(refusal.contains("subagent of thread parent"));
        assert!(!refusal.contains("does not exist"));
        assert!(!refusal.contains("not found"));
    }

    #[test]
    fn a_top_level_thread_is_nobodys_subagent() {
        let access = subagent_transcript_access(&session("parent"), &session("other_root"), None);
        assert_eq!(access, TranscriptAccess::RefusedNotASubagent);
        assert!(
            access
                .refusal(&session("other_root"))
                .expect("refused")
                .contains("top-level thread")
        );
    }

    #[test]
    fn a_thread_does_not_read_itself() {
        // Even a subagent asking for its own ID is refused: its transcript is
        // already its context, and returning it would double the cost.
        for parent in [None, Some(session("parent"))] {
            let access =
                subagent_transcript_access(&session("me"), &session("me"), parent.as_ref());
            assert_eq!(access, TranscriptAccess::RefusedIsCaller);
        }
    }

    #[test]
    fn access_never_walks_past_the_immediate_parent() {
        // A grandchild names its own parent (the child), not the root. The root
        // asking for it is refused, and the refusal points at the child. This
        // is the case `MAX_SUBAGENT_DEPTH > 1` would create.
        let access = subagent_transcript_access(
            &session("root"),
            &session("grandchild"),
            Some(&session("child")),
        );
        assert_eq!(
            access,
            TranscriptAccess::RefusedOtherParent {
                parent: session("child")
            }
        );
    }

    // --- Bounds. ---

    #[test]
    fn a_window_request_is_clamped() {
        assert_eq!(
            TranscriptWindowRequest::clamp(None, None),
            TranscriptWindowRequest {
                offset: 0,
                limit: DEFAULT_MESSAGE_LIMIT
            }
        );
        // Zero would return a window nobody can learn from.
        assert_eq!(
            TranscriptWindowRequest::clamp(Some(3), Some(0)).limit,
            DEFAULT_MESSAGE_LIMIT
        );
        assert_eq!(
            TranscriptWindowRequest::clamp(Some(3), Some(10_000)),
            TranscriptWindowRequest {
                offset: 3,
                limit: MAX_MESSAGE_LIMIT
            }
        );
        assert_eq!(TranscriptWindowRequest::clamp(Some(7), Some(4)).offset, 7);
    }

    #[test]
    fn a_clipped_block_says_it_was_clipped() {
        let long = "x".repeat(FULL_BLOCK_BYTE_LIMIT + 500);
        let rendered = render_transcript(
            &transcript(vec![text_entry(0, &long)], 1, 0),
            TranscriptDetail::Full,
            MAX_TRANSCRIPT_BYTES,
        );
        assert!(rendered.contains("block truncated"));
        assert!(rendered.contains(&format!("of {} bytes shown", long.len())));
    }

    #[test]
    fn clipping_never_splits_a_character() {
        // A multi-byte character straddling the limit must not panic or
        // produce invalid UTF-8.
        let text = "é".repeat(OUTLINE_BLOCK_BYTE_LIMIT);
        let clipped = clip(&text, OUTLINE_BLOCK_BYTE_LIMIT + 1);
        assert!(text.starts_with(clipped));
        assert_eq!(clipped.len(), OUTLINE_BLOCK_BYTE_LIMIT);
    }

    #[test]
    fn the_byte_cap_names_the_messages_it_dropped() {
        let long = "y".repeat(FULL_BLOCK_BYTE_LIMIT);
        let entries: Vec<_> = (0..40).map(|i| text_entry(i, &long)).collect();
        let rendered = render_transcript(
            &transcript(entries, 40, 0),
            TranscriptDetail::Full,
            MAX_TRANSCRIPT_BYTES,
        );

        assert!(rendered.len() <= MAX_TRANSCRIPT_BYTES);
        assert!(
            rendered.contains("transcript truncated at the"),
            "the cap fired silently: {rendered}"
        );
        assert!(
            rendered.contains("message(s) of this window were not rendered"),
            "the cap did not say what it dropped"
        );
        // The marker must name an offset that actually resumes the reading.
        assert!(rendered.contains("Ask again with offset="));
    }

    #[test]
    fn a_short_window_of_a_long_transcript_says_more_remains() {
        let rendered = render_transcript(
            &transcript(vec![text_entry(0, "hi"), text_entry(1, "there")], 9, 0),
            TranscriptDetail::Outline,
            MAX_TRANSCRIPT_BYTES,
        );
        assert!(rendered.contains("of 9 total"));
        assert!(rendered.contains("7 more message(s) after this window"));
        assert!(rendered.contains("offset=2"));
    }

    #[test]
    fn a_complete_window_claims_nothing_is_missing() {
        let rendered = render_transcript(
            &transcript(vec![text_entry(0, "hi"), text_entry(1, "bye")], 2, 0),
            TranscriptDetail::Outline,
            MAX_TRANSCRIPT_BYTES,
        );
        assert!(!rendered.contains("more message(s) after"));
        assert!(!rendered.contains("truncated"));
    }

    #[test]
    fn an_empty_window_says_so_rather_than_rendering_nothing() {
        let rendered = render_transcript(
            &transcript(Vec::new(), 4, 40),
            TranscriptDetail::Outline,
            MAX_TRANSCRIPT_BYTES,
        );
        assert!(rendered.contains("No messages in this window"));
    }

    #[test]
    fn outline_reports_tool_calls_and_result_sizes_without_the_bodies() {
        let body = "z".repeat(5_000);
        let entry = TranscriptEntry {
            index: 2,
            role: TranscriptRole::Assistant,
            blocks: vec![
                TranscriptBlock::Thinking("secret reasoning".into()),
                TranscriptBlock::ToolUse {
                    name: "grep".into(),
                    id: "t1".into(),
                    input: "{\"regex\":\"foo\"}".into(),
                },
                TranscriptBlock::ToolResult {
                    name: "grep".into(),
                    id: "t1".into(),
                    is_error: false,
                    text: body.clone(),
                },
            ],
        };
        let rendered = render_transcript(
            &transcript(vec![entry], 3, 2),
            TranscriptDetail::Outline,
            MAX_TRANSCRIPT_BYTES,
        );

        assert!(rendered.contains("→ tool grep (t1) {\"regex\":\"foo\"}"));
        // The size is stated even though the body is not shown, so the reader
        // can tell a big result from an empty one.
        assert!(rendered.contains("← tool grep (t1) [5000 bytes]"));
        assert!(rendered.contains("block truncated"));
        assert!(!rendered.contains(&body));
        // Outline does not spend the parent's context on thinking text.
        assert!(rendered.contains("<thinking, 16 bytes>"));
        assert!(!rendered.contains("secret reasoning"));
    }

    #[test]
    fn an_errored_tool_result_is_marked() {
        let entry = TranscriptEntry {
            index: 0,
            role: TranscriptRole::Assistant,
            blocks: vec![TranscriptBlock::ToolResult {
                name: "terminal".into(),
                id: "t9".into(),
                is_error: true,
                text: "command not found".into(),
            }],
        };
        let rendered = render_transcript(
            &transcript(vec![entry], 1, 0),
            TranscriptDetail::Full,
            MAX_TRANSCRIPT_BYTES,
        );
        assert!(rendered.contains("← tool terminal (t9) ERROR"));
        assert!(rendered.contains("command not found"));
    }

    #[test]
    fn the_description_tells_the_model_this_is_for_inspecting_delegated_work() {
        let description = ReadSubagentTranscriptTool::description();
        assert!(description.contains("spawned"));
        assert!(
            description.contains("usually enough"),
            "the description must say the final message is normally sufficient, \
             or the model will call this after every delegation"
        );
    }
}
