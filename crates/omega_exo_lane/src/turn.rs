//! Reading what Exo said. `OMEGA-DELTA-0042`, omega#87.
//!
//! Tier A is coarse on purpose. Exo's turn streaming exists only as an
//! in-process `ExecutionStreamEvent` enum consumed by its own terminal REPL and
//! serialised to no transport whatsoever, so a host application gets one shot
//! per turn, no live text deltas, and tool activity only after the fact. That
//! is a limit of Exo at this pin, not a shortcut here, and Tier B is where it
//! is lifted — by contributing a transport to Exo, whose adapter types are a
//! closed Rust enum.
//!
//! What `exo conversation send` prints is the messages the turn appended, one
//! per line, each prefixed with a compact clock and a role:
//!
//! ```text
//! [04:52:12] user: Reply with exactly the word OMEGA-EXO-TIER-A and nothing else.
//! [04:52:12] assistant: [reasoning]
//! [04:52:12] assistant: OMEGA-EXO-TIER-A
//! ```
//!
//! # A turn is not "whatever came back on stdout"
//!
//! This is the parser's real job, and the reason it refuses rather than doing
//! its best. Driven against the pinned Exo, `conversation send` with a prompt of
//! `--help` **exits 0 and prints Exo's usage text**, because Exo accepts its
//! global options after the subcommand and consumed the prompt as one.
//! [`crate::command`] makes that unreachable with an argument terminator — but a
//! reader that treated any successful stdout as a reply would have rendered a
//! usage message as the model's answer, and it would have done so on exit 0
//! with nothing to see in a log.
//!
//! So [`ExoTurn::read`] requires the shape of a turn — at least one assistant
//! line — and returns [`NotATurn`] otherwise. Two independent guards against one
//! silent failure, because the failure was silent.

/// What Exo produced for one turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExoTurn {
    /// The assistant's text, reasoning parts removed, lines joined.
    pub text: String,
    /// Tool activity, in the order Exo printed it. Visible after the fact; see
    /// the module documentation.
    pub tools: Vec<ExoToolActivity>,
}

/// One tool result Exo recorded during the turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExoToolActivity {
    /// The tool's name, as Exo reports it.
    pub name: String,
    /// The rendered result. Exo shows a preview here; the full result is an
    /// artifact in its durable log.
    pub output: String,
}

/// Why stdout was not a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotATurn {
    /// Nothing on stdout carried a role at all. Exo's usage text lands here,
    /// which is the case this exists for.
    NoMessages,
    /// Messages were printed and none of them were the assistant's. A turn
    /// that echoed the user and said nothing is not a reply.
    NoAssistantMessage,
}

impl std::fmt::Display for NotATurn {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NoMessages => "Exo printed no messages, so this turn produced no reply",
            Self::NoAssistantMessage => "Exo printed no assistant message for this turn",
        })
    }
}

impl std::error::Error for NotATurn {}

/// The prefix Exo puts on a reasoning part.
const REASONING_PREFIX: &str = "[reasoning]";

impl ExoTurn {
    /// Read `exo conversation send` output.
    ///
    /// # Errors
    ///
    /// [`NotATurn`] when the output does not have the shape of a turn.
    pub fn read(stdout: &str) -> Result<Self, NotATurn> {
        let mut saw_message = false;
        let mut text_lines: Vec<String> = Vec::new();
        let mut tools = Vec::new();
        let mut in_assistant = false;

        for line in stdout.lines() {
            match strip_clock(line) {
                Some(body) => {
                    saw_message = true;
                    in_assistant = false;
                    if let Some(assistant) = body.strip_prefix("assistant: ") {
                        in_assistant = true;
                        push_assistant(&mut text_lines, assistant);
                    } else if let Some(tool) = body.strip_prefix("tool ") {
                        if let Some((name, output)) = tool.split_once(": ") {
                            tools.push(ExoToolActivity {
                                name: name.to_owned(),
                                output: output.to_owned(),
                            });
                        }
                    }
                }
                // A continuation of the previous message: assistant text with a
                // newline in it. Only kept while an assistant message is open,
                // so a multi-line tool result cannot leak into the reply.
                None if in_assistant => text_lines.push(line.to_owned()),
                None => {}
            }
        }

        if !saw_message {
            return Err(NotATurn::NoMessages);
        }
        if text_lines.is_empty() {
            return Err(NotATurn::NoAssistantMessage);
        }
        Ok(Self {
            text: text_lines.join("\n").trim().to_owned(),
            tools,
        })
    }
}

/// `[04:52:12] rest` → `rest`.
///
/// Matched by shape rather than by regex: eleven characters, brackets at the
/// ends of the clock, digits and colons between. A line Exo did not print with
/// a clock is a continuation, not a message.
fn strip_clock(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    let (clock, rest) = rest.split_once("] ")?;
    if clock.len() != 8 || !clock.chars().all(|c| c.is_ascii_digit() || c == ':') {
        return None;
    }
    Some(rest)
}

/// Keep the assistant's words and drop the parts that are not words.
///
/// Reasoning is dropped rather than rendered: at this pin Exo prints it as a
/// bare `[reasoning]` marker with the text usually empty, and showing an empty
/// marker as the model's reply would be worse than showing nothing. Tool-call
/// and tool-result parts are dropped from the *text* because they are carried
/// as [`ExoToolActivity`] instead, so a reader does not see them twice.
fn push_assistant(text_lines: &mut Vec<String>, content: &str) {
    let content = content.trim();
    if content.is_empty() || content == REASONING_PREFIX {
        return;
    }
    if content.starts_with(REASONING_PREFIX)
        || content.starts_with("[tool_call ")
        || content.starts_with("[tool_result ")
    {
        return;
    }
    text_lines.push(content.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exactly what the pinned Exo printed for the turn that proved this lane,
    /// on 2026-07-25. Copied from the terminal, not composed here.
    const DRIVEN_TURN: &str = "\
[04:52:12] user: Reply with exactly the word OMEGA-EXO-TIER-A and nothing else.
[04:52:12] assistant: [reasoning]
[04:52:12] assistant: OMEGA-EXO-TIER-A
";

    #[test]
    fn the_turn_that_was_actually_driven_reads_back() {
        let turn = ExoTurn::read(DRIVEN_TURN).expect("a real turn");
        assert_eq!(turn.text, "OMEGA-EXO-TIER-A");
        assert!(turn.tools.is_empty());
    }

    /// The silent failure, as a test. Exo's usage text on exit 0 must not be
    /// readable as a reply.
    #[test]
    fn exos_usage_text_is_not_a_turn() {
        let usage = "\
Usage: exo conversation send [OPTIONS] <AGENT> <CONVERSATION> <PROMPT>

Arguments:
  <AGENT>
  <CONVERSATION>
  <PROMPT>
";
        assert_eq!(ExoTurn::read(usage), Err(NotATurn::NoMessages));
    }

    #[test]
    fn a_turn_with_no_assistant_message_is_refused() {
        let echoed = "[04:52:12] user: hello\n";
        assert_eq!(ExoTurn::read(echoed), Err(NotATurn::NoAssistantMessage));
    }

    #[test]
    fn tool_activity_is_read_and_kept_out_of_the_reply_text() {
        let with_tools = "\
[04:52:12] user: list the directory
[04:52:13] assistant: [tool_call shell] {\"command\":\"ls\"}
[04:52:14] tool shell: Cargo.toml
[04:52:15] assistant: There is one file.
";
        let turn = ExoTurn::read(with_tools).expect("a turn");
        assert_eq!(turn.text, "There is one file.");
        assert_eq!(
            turn.tools,
            vec![ExoToolActivity {
                name: "shell".into(),
                output: "Cargo.toml".into(),
            }]
        );
    }

    /// A reply with a newline in it arrives as an unprefixed continuation line.
    #[test]
    fn a_multi_line_reply_keeps_its_lines() {
        let multi = "[04:52:12] assistant: first\nsecond\n";
        assert_eq!(ExoTurn::read(multi).expect("a turn").text, "first\nsecond");
    }

    /// A multi-line *tool* result must not run on into the reply, or a tool
    /// that printed a paragraph would be attributed to the model.
    #[test]
    fn a_multi_line_tool_result_does_not_leak_into_the_reply() {
        let leaky = "\
[04:52:14] tool shell: line one
line two
[04:52:15] assistant: done
";
        let turn = ExoTurn::read(leaky).expect("a turn");
        assert_eq!(turn.text, "done");
    }
}
