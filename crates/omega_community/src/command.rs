//! The room, operated from the conversation. `OMEGA-DELTA-0113`, omega#108.
//!
//! omega#108 deliverable 3: "Controlled from the chat. The owner's requirement
//! is that it is all operable through the Omega Agent conversation rather than
//! a separate administrative surface. Joining, seeing who is present, and
//! posting are conversation actions."
//!
//! So there is no settings page, no dialog, and no admin pane. There is a line
//! a person types.
//!
//! # Why this is a literal prefix and not an interpretation
//!
//! [`parse`] returns `None` for anything that does not begin with
//! [`COMMAND_PREFIX`], and it never guesses. That is not laziness about
//! language: an ordinary sentence in a conversation must never be mistaken for
//! an instruction to publish, and the only way to guarantee that is for the
//! recogniser to be a literal one. A person writing *"I should join the omega
//! room"* to their agent has said something about their intentions, not issued
//! a command, and a parser that could not tell the difference would be a
//! parser that sometimes published on a hunch.
//!
//! The verbs after the prefix are a closed set for the same reason. An
//! unrecognised one is refused by name with the list, rather than resolved to
//! the nearest match — "did you mean post" is a helpful sentence and a
//! dangerous behaviour.
//!
//! # What this module does not do
//!
//! It parses and it refuses. Executing a [`Command`] needs the durable record
//! of what this profile has joined, a signer, and a transport, and none of
//! those belong in a crate that must not open a socket. The caller at the edge
//! holds them; this holds the grammar, where it can be checked without one.

use std::fmt;

use crate::{Invitation, InvitationRefused};

/// The literal a line must begin with to be an instruction about the room.
pub const COMMAND_PREFIX: &str = "/community";

/// The verb for accepting an invitation.
pub const JOIN: &str = "join";
/// The verb for asking who Omega has seen.
pub const WHO: &str = "who";
/// The verb for sending a message into the room.
pub const POST: &str = "post";
/// The verb for leaving.
pub const LEAVE: &str = "leave";
/// The verb for asking what this profile is in, and what is outstanding.
pub const STATUS: &str = "status";

/// Every verb, in the order the help lists them.
pub const COMMAND_VERBS: &[&str] = &[JOIN, WHO, POST, LEAVE, STATUS];

/// What a person is told when they get the verb wrong, and what an agent is
/// told the surface can do.
///
/// One string rather than a sentence assembled at each call site, so the help a
/// person reads and the help a refusal quotes cannot drift apart.
pub const COMMAND_HELP: &str = "\
/community status — what this profile has joined, and what has not been sent
/community join <invitation> — accept an invitation the owner sent you
/community who — who Omega has verified writing in the room
/community post <message> — send a message into the room
/community leave — leave, and keep nothing";

/// An instruction about the room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Accept an invitation.
    ///
    /// Boxed because an [`Invitation`] carries a room and a membership, and an
    /// enum is as large as its largest variant — every `Command` on the stack
    /// would otherwise be the size of the one that carries a room.
    Join(Box<Invitation>),
    /// Ask who Omega has seen writing.
    Who,
    /// Send a message.
    Post(String),
    /// Leave.
    Leave,
    /// Ask what this profile is in, and what has not been sent.
    Status,
}

/// Why a line beginning with [`COMMAND_PREFIX`] was not an instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandRefused {
    /// A verb that is not in [`COMMAND_VERBS`].
    UnknownVerb(String),
    /// `join` with nothing after it.
    JoinNeedsAnInvitation,
    /// `join` with something after it that is not an invitation.
    Invitation(InvitationRefused),
    /// `post` with nothing to say.
    PostNeedsAMessage,
    /// A verb that takes nothing was given something.
    ///
    /// Refused rather than ignored. `\/community leave the room` reads like a
    /// sentence and would otherwise silently leave; and a trailing word on
    /// `who` is more likely a person expecting a filter this does not have.
    TakesNoArgument {
        /// The verb.
        verb: &'static str,
        /// What was given to it.
        given: String,
    },
}

impl fmt::Display for CommandRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownVerb(verb) => write!(
                formatter,
                "`{verb}` is not something this room does.\n\n{COMMAND_HELP}"
            ),
            Self::JoinNeedsAnInvitation => write!(
                formatter,
                "`{COMMAND_PREFIX} {JOIN}` needs the invitation the owner sent you, on the same \
                 line."
            ),
            Self::Invitation(refusal) => write!(formatter, "{refusal}"),
            Self::PostNeedsAMessage => write!(
                formatter,
                "`{COMMAND_PREFIX} {POST}` needs something to say, on the same line."
            ),
            Self::TakesNoArgument { verb, given } => write!(
                formatter,
                "`{COMMAND_PREFIX} {verb}` takes nothing after it, and this line adds `{given}`. \
                 Refusing rather than ignoring it, because ignoring the rest of a line is how a \
                 sentence becomes an instruction by accident."
            ),
        }
    }
}

/// Reads a line as an instruction about the room.
///
/// - `None` when the line is not addressed to the room at all. Not an error:
///   almost every line in a conversation is this, and it is the caller's signal
///   to leave the line alone entirely.
/// - `Some(Err(_))` when it was addressed to the room and could not be read.
/// - `Some(Ok(_))` when it was.
#[must_use]
pub fn parse(line: &str) -> Option<Result<Command, CommandRefused>> {
    let line = line.trim();
    let rest = match line.strip_prefix(COMMAND_PREFIX) {
        // The prefix must be the whole first word. Without this,
        // `/communityfoo` and, more to the point, a path or a URL that happens
        // to start with the same letters would be read as an instruction.
        Some(rest) if rest.is_empty() || rest.starts_with(char::is_whitespace) => rest.trim(),
        _ => return None,
    };

    let (verb, argument) = match rest.split_once(char::is_whitespace) {
        Some((verb, argument)) => (verb, argument.trim()),
        None => (rest, ""),
    };

    // A bare prefix asks what the state is, which is the one answer that is
    // never destructive and is what somebody typing the word alone wants.
    if verb.is_empty() {
        return Some(Ok(Command::Status));
    }

    let takes_nothing = |verb: &'static str, command: Command| {
        if argument.is_empty() {
            Ok(command)
        } else {
            Err(CommandRefused::TakesNoArgument {
                verb,
                given: argument.to_string(),
            })
        }
    };

    Some(match verb {
        JOIN if argument.is_empty() => Err(CommandRefused::JoinNeedsAnInvitation),
        JOIN => Invitation::parse(argument)
            .map(|invitation| Command::Join(Box::new(invitation)))
            .map_err(CommandRefused::Invitation),
        POST if argument.is_empty() => Err(CommandRefused::PostNeedsAMessage),
        POST => Ok(Command::Post(argument.to_string())),
        WHO => takes_nothing(WHO, Command::Who),
        LEAVE => takes_nothing(LEAVE, Command::Leave),
        STATUS => takes_nothing(STATUS, Command::Status),
        other => Err(CommandRefused::UnknownVerb(other.to_string())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invitation::tests::omega_invitation_text;

    /// The reason the recogniser is a literal, stated as a test rather than as
    /// a comment: none of these are instructions, and one of them is somebody
    /// telling their agent what they intend to do.
    #[test]
    fn an_ordinary_sentence_is_not_an_instruction_about_the_room() {
        for line in [
            "",
            "hello",
            "I should join the omega community room",
            "post this to the community when you are done",
            "see /community for the docs",
            "/communityjoin",
            "/community-join",
            "/join",
        ] {
            assert!(
                parse(line).is_none(),
                "`{line}` must not be read as an instruction about the room"
            );
        }
    }

    #[test]
    fn the_bare_prefix_asks_and_changes_nothing() {
        assert_eq!(parse("/community"), Some(Ok(Command::Status)));
        assert_eq!(parse("  /community  "), Some(Ok(Command::Status)));
        assert_eq!(parse("/community status"), Some(Ok(Command::Status)));
    }

    #[test]
    fn joining_takes_the_invitation_on_the_same_line() {
        let line = format!("/community join {}", omega_invitation_text());
        let parsed = parse(&line).expect("a line addressed to the room");

        let expected = Invitation::parse(&omega_invitation_text()).expect("a well formed one");
        assert_eq!(parsed, Ok(Command::Join(Box::new(expected))));
    }

    #[test]
    fn joining_with_nothing_says_what_is_missing() {
        assert_eq!(
            parse("/community join"),
            Some(Err(CommandRefused::JoinNeedsAnInvitation))
        );
        assert!(
            CommandRefused::JoinNeedsAnInvitation
                .to_string()
                .contains("invitation the owner sent you")
        );
    }

    #[test]
    fn joining_with_something_that_is_not_an_invitation_carries_the_reason() {
        let refused = parse("/community join tenant=tenant.openagents")
            .expect("a line addressed to the room")
            .expect_err("and not an invitation");

        assert_eq!(
            refused,
            CommandRefused::Invitation(InvitationRefused::NotAnInvitation)
        );
        assert!(
            refused.to_string().contains("omega-invite:1"),
            "the refusal tells a person what an invitation looks like: {refused}"
        );
    }

    #[test]
    fn posting_keeps_the_whole_line_including_its_spacing() {
        assert_eq!(
            parse("/community post the check fails on a fresh install"),
            Some(Ok(Command::Post(
                "the check fails on a fresh install".to_string()
            )))
        );
        assert_eq!(
            parse("/community post  two  spaces  inside "),
            Some(Ok(Command::Post("two  spaces  inside".to_string()))),
            "only the ends are trimmed; what somebody typed in the middle is theirs"
        );
        assert_eq!(
            parse("/community post"),
            Some(Err(CommandRefused::PostNeedsAMessage))
        );
    }

    /// The trailing-word rule, which is the one that stops a sentence becoming
    /// an act.
    #[test]
    fn a_verb_that_takes_nothing_refuses_the_rest_of_the_line() {
        assert_eq!(
            parse("/community leave the room when you are done"),
            Some(Err(CommandRefused::TakesNoArgument {
                verb: "leave",
                given: "the room when you are done".to_string(),
            })),
            "ignoring the rest of the line would have left the room"
        );
        assert_eq!(parse("/community leave"), Some(Ok(Command::Leave)));
        assert_eq!(parse("/community who"), Some(Ok(Command::Who)));
        assert!(matches!(
            parse("/community who is here"),
            Some(Err(CommandRefused::TakesNoArgument { verb: "who", .. }))
        ));
    }

    #[test]
    fn an_unknown_verb_is_named_and_never_guessed_at() {
        let refused = parse("/community pots hello")
            .expect("a line addressed to the room")
            .expect_err("and not a verb");

        assert_eq!(refused, CommandRefused::UnknownVerb("pots".to_string()));
        let sentence = refused.to_string();
        assert!(
            !sentence.contains("did you mean"),
            "resolving to the nearest verb is a helpful sentence and a \
             dangerous behaviour: {sentence}"
        );
        for verb in COMMAND_VERBS {
            assert!(
                sentence.contains(verb),
                "the refusal lists what the room does, and omits `{verb}`"
            );
        }
    }

    /// The help and the refusal quote the same string, so they cannot drift.
    #[test]
    fn the_help_is_written_once() {
        assert!(
            CommandRefused::UnknownVerb("x".to_string())
                .to_string()
                .contains(COMMAND_HELP)
        );
        for verb in COMMAND_VERBS {
            assert!(
                COMMAND_HELP.contains(&format!("{COMMAND_PREFIX} {verb}")),
                "`{verb}` is a verb the help does not mention"
            );
        }
    }
}
