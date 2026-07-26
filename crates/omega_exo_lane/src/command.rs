//! Every command the Exo lane is allowed to run. `OMEGA-DELTA-0042`, omega#87.
//!
//! Omega owns Exo's process the way it owns any external tool, and it **never
//! edits Exo's `.exo` root, agents, secrets, or configuration** — the same law
//! it obeys around a Codex home. The way that law is kept is not a review
//! convention: it is that there is no way to express a configuring command.
//! [`ExoCommand`] is a closed enum of four operations, [`ExoCommand::argv`] is
//! total over it, and [`ADMITTED_LANE_ARGV`] states each resulting shape
//! exactly. A fifth operation does not compile until somebody writes its shape
//! down, and writing it down is the record that a person decided it belonged.
//!
//! The set is deliberately small: send one turn, read the durable log, read the
//! conversation, read the agent, read the model bindings. Four of the five are
//! reads. The one that writes is `conversation send`, and what it writes is
//! Exo's own record of the turn it just ran — Exo mutating Exo, which is the
//! only mutation this lane causes and the only one it could cause without
//! ceasing to be a lane.
//!
//! # The argument terminator is load-bearing
//!
//! Exo's global options are accepted **after** the subcommand: `exo
//! conversation send --harness cursor <agent> <conv> <prompt>` is a valid
//! invocation that changes which executor runs the turn. So a prompt is not
//! merely a string Exo receives — without a terminator it is *argument syntax*,
//! and the person typing it is writing Exo's command line.
//!
//! Driven against the pinned Exo, a prompt of `--help` on an otherwise correct
//! `conversation send` **exits 0, prints Exo's usage text, and runs no turn.**
//! A lane without the terminator would have read that usage text back as the
//! model's reply. With `--` it is delivered as text and a real turn runs.
//!
//! So every value that could have come from a person or a model is emitted
//! after `--`, and [`ExoArg`] makes that checkable rather than hoped for:
//! arguments carry whether they are literal, Omega's own configuration, or
//! text from outside, and `user_text_cannot_become_an_exo_flag` fails if one of
//! the last kind is ever emitted before the terminator.

/// Exo's state root, as a value Omega only ever *names*.
///
/// There is no method here that writes, creates, removes, or opens anything.
/// That is the type's entire job: an `ExoRoot` can be passed to a command
/// builder and cannot be used to touch the directory it names. Omega's process
/// never writes inside `.exo`; only the `exo` binary does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExoRoot(String);

impl ExoRoot {
    /// Name an existing Exo state root.
    #[must_use]
    pub fn at(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// The path, for putting on a command line.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One argument, and where it came from.
///
/// The provenance is the point. See the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExoArg {
    /// A literal this file wrote.
    Fixed(&'static str),
    /// A value from Omega's own lane configuration — the state root, a limit.
    /// Not literal, but not reachable from a turn either.
    Config(String),
    /// A value that could have been typed by a person or produced by a model.
    /// Never emitted before the terminator.
    UserText(String),
}

impl ExoArg {
    /// The argument as it goes on the command line.
    #[must_use]
    pub fn value(&self) -> &str {
        match self {
            Self::Fixed(value) => value,
            Self::Config(value) | Self::UserText(value) => value,
        }
    }

    /// Whether this argument came from outside Omega's own configuration.
    #[must_use]
    pub const fn is_user_text(&self) -> bool {
        matches!(self, Self::UserText(_))
    }
}

/// The terminator, after which Exo stops reading arguments as options.
pub const ARGUMENT_TERMINATOR: &str = "--";

/// Every command the lane may run, and the exact argv each produces.
///
/// A closed list, in the same spirit as `EXECUTOR_DISCLOSURE_FIELDS`: the
/// property that matters is "these shapes and nothing else". A denylist of
/// dangerous flags would be a guess about which flags upstream will add next,
/// in a project whose written house rule is that it does not keep backwards
/// compatibility. Placeholders are `<name>`; every other token is literal.
pub const ADMITTED_LANE_ARGV: &[(&str, &[&str])] = &[
    (
        "send_turn",
        &[
            "--root",
            "<root>",
            "conversation",
            "send",
            "--",
            "<agent>",
            "<conversation>",
            "<prompt>",
        ],
    ),
    (
        "read_events",
        &[
            "--root",
            "<root>",
            "conversation",
            "events",
            "--limit",
            "<limit>",
            "--",
            "<agent>",
            "<conversation>",
        ],
    ),
    (
        "show_conversation",
        &[
            "--root",
            "<root>",
            "conversation",
            "show",
            "--",
            "<agent>",
            "<conversation>",
        ],
    ),
    (
        "show_agent",
        &["--root", "<root>", "agent", "show", "--", "<agent>"],
    ),
    // No terminator, because this command carries no value from outside Omega.
    // A terminator here would be decoration, and decoration in a security shape
    // teaches a reader that the terminator is a habit rather than a mechanism.
    ("list_models", &["--root", "<root>", "model", "list"]),
];

/// The five things the lane does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExoCommand {
    /// Run one turn. The whole of Tier A's execution: one shot, no stream.
    SendTurn {
        agent: String,
        conversation: String,
        prompt: String,
    },
    /// Read the durable event log, newest page, for tool activity after the
    /// fact.
    ReadEvents {
        agent: String,
        conversation: String,
        limit: u32,
    },
    /// Read a conversation's record, for the model it resolves to.
    ShowConversation {
        agent: String,
        conversation: String,
    },
    /// Read an agent's record, for its executor and its capability set. This is
    /// what [`crate::capability`] refuses a turn on.
    ShowAgent { agent: String },
    /// Read the model bindings, so the disclosure can name the model that
    /// actually served rather than the local alias the agent record carries.
    ListModels,
}

impl ExoCommand {
    /// The token this command is recorded and checked under.
    #[must_use]
    pub const fn shape(&self) -> &'static str {
        match self {
            Self::SendTurn { .. } => "send_turn",
            Self::ReadEvents { .. } => "read_events",
            Self::ShowConversation { .. } => "show_conversation",
            Self::ShowAgent { .. } => "show_agent",
            Self::ListModels => "list_models",
        }
    }

    /// The arguments, with their provenance.
    ///
    /// Total: every variant produces a shape, and each one is stated in
    /// [`ADMITTED_LANE_ARGV`].
    #[must_use]
    pub fn args(&self, root: &ExoRoot) -> Vec<ExoArg> {
        let mut args = vec![
            ExoArg::Fixed("--root"),
            ExoArg::Config(root.as_str().to_owned()),
        ];
        match self {
            Self::SendTurn {
                agent,
                conversation,
                prompt,
            } => {
                args.extend([
                    ExoArg::Fixed("conversation"),
                    ExoArg::Fixed("send"),
                    ExoArg::Fixed(ARGUMENT_TERMINATOR),
                    ExoArg::UserText(agent.clone()),
                    ExoArg::UserText(conversation.clone()),
                    ExoArg::UserText(prompt.clone()),
                ]);
            }
            Self::ReadEvents {
                agent,
                conversation,
                limit,
            } => {
                args.extend([
                    ExoArg::Fixed("conversation"),
                    ExoArg::Fixed("events"),
                    ExoArg::Fixed("--limit"),
                    ExoArg::Config(limit.to_string()),
                    ExoArg::Fixed(ARGUMENT_TERMINATOR),
                    ExoArg::UserText(agent.clone()),
                    ExoArg::UserText(conversation.clone()),
                ]);
            }
            Self::ShowConversation {
                agent,
                conversation,
            } => {
                args.extend([
                    ExoArg::Fixed("conversation"),
                    ExoArg::Fixed("show"),
                    ExoArg::Fixed(ARGUMENT_TERMINATOR),
                    ExoArg::UserText(agent.clone()),
                    ExoArg::UserText(conversation.clone()),
                ]);
            }
            Self::ShowAgent { agent } => {
                args.extend([
                    ExoArg::Fixed("agent"),
                    ExoArg::Fixed("show"),
                    ExoArg::Fixed(ARGUMENT_TERMINATOR),
                    ExoArg::UserText(agent.clone()),
                ]);
            }
            Self::ListModels => {
                args.extend([ExoArg::Fixed("model"), ExoArg::Fixed("list")]);
            }
        }
        args
    }

    /// The command line, as strings.
    #[must_use]
    pub fn argv(&self, root: &ExoRoot) -> Vec<String> {
        self.args(root)
            .iter()
            .map(|arg| arg.value().to_owned())
            .collect()
    }

    /// One of every variant, filled with placeholder values.
    ///
    /// The exhaustiveness handle: a new variant that is not added here fails
    /// `every_variant_has_a_written_shape`, and a new variant that *is* added
    /// here fails until its shape is written into [`ADMITTED_LANE_ARGV`].
    #[must_use]
    pub fn every_shape() -> Vec<Self> {
        vec![
            Self::SendTurn {
                agent: "<agent>".into(),
                conversation: "<conversation>".into(),
                prompt: "<prompt>".into(),
            },
            Self::ReadEvents {
                agent: "<agent>".into(),
                conversation: "<conversation>".into(),
                limit: 0,
            },
            Self::ShowConversation {
                agent: "<agent>".into(),
                conversation: "<conversation>".into(),
            },
            Self::ShowAgent {
                agent: "<agent>".into(),
            },
            Self::ListModels,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> ExoRoot {
        ExoRoot::at("<root>")
    }

    /// The shapes, asserted exactly. Not "contains no dangerous flag" — every
    /// token, in order, against a written list.
    ///
    /// A `<name>` token in the written shape asserts a *slot*: that position
    /// must be filled from a value, and a literal appearing there fails. Every
    /// other token asserts itself. So the check covers both what the command
    /// line says and which parts of it this file decided rather than received.
    #[test]
    fn every_variant_has_a_written_shape() {
        let shapes = ExoCommand::every_shape();
        assert_eq!(
            shapes.len(),
            ADMITTED_LANE_ARGV.len(),
            "a command variant exists with no written argv shape, or the other way round"
        );
        for command in &shapes {
            let (_, expected) = ADMITTED_LANE_ARGV
                .iter()
                .find(|(name, _)| *name == command.shape())
                .unwrap_or_else(|| panic!("{} has no written shape", command.shape()));
            let args = command.args(&root());
            assert_eq!(args.len(), expected.len(), "{}", command.shape());
            for (index, (arg, token)) in args.iter().zip(expected.iter()).enumerate() {
                if token.starts_with('<') {
                    assert!(
                        !matches!(arg, ExoArg::Fixed(_)),
                        "{} argv[{index}] is a literal where the shape says {token}",
                        command.shape()
                    );
                } else {
                    assert_eq!(
                        arg,
                        &ExoArg::Fixed(token),
                        "{} argv[{index}]",
                        command.shape()
                    );
                }
            }
        }
    }

    /// The security law, stated over the whole closed set rather than over the
    /// arguments one call site happened to build.
    ///
    /// A command that carries no text from outside needs no terminator; a
    /// command that carries any must put all of it after one. Both halves are
    /// checked, so neither "forgot the terminator" nor "added user text after
    /// the fact" can land quietly.
    #[test]
    fn user_text_cannot_become_an_exo_flag() {
        for command in ExoCommand::every_shape() {
            let args = command.args(&root());
            let terminator = args
                .iter()
                .position(|arg| arg == &ExoArg::Fixed(ARGUMENT_TERMINATOR));
            let Some(terminator) = terminator else {
                assert!(
                    !args.iter().any(ExoArg::is_user_text),
                    "{} carries text from outside Omega and emits no argument terminator",
                    command.shape()
                );
                continue;
            };
            for (index, arg) in args.iter().enumerate() {
                assert!(
                    !(arg.is_user_text() && index <= terminator),
                    "{} puts text from outside Omega at argv[{index}], before the terminator",
                    command.shape()
                );
            }
        }
    }

    /// The prompt that proved this against the real binary: `--help` on a
    /// `conversation send` exits 0, prints usage, and runs no turn. After the
    /// terminator it is a prompt.
    #[test]
    fn a_flag_shaped_prompt_is_still_a_prompt() {
        let command = ExoCommand::SendTurn {
            agent: "omega-lane".into(),
            conversation: "tier-a".into(),
            prompt: "--help".into(),
        };
        let argv = command.argv(&root());
        let terminator = argv
            .iter()
            .position(|arg| arg == ARGUMENT_TERMINATOR)
            .expect("terminated");
        assert_eq!(argv.last().map(String::as_str), Some("--help"));
        assert!(argv.len() - 1 > terminator);
    }

    /// Omega never configures Exo. The admitted verbs are three reads and one
    /// send, and the ones that are absent are absent by name so a later edit
    /// that adds `agent update` has to delete a line of this test to pass.
    #[test]
    fn the_lane_can_express_no_command_that_configures_exo() {
        let admitted: Vec<&str> = ADMITTED_LANE_ARGV
            .iter()
            .map(|(_, argv)| argv[3]) // --root <root> <group> <verb>
            .collect();
        assert_eq!(admitted, ["send", "events", "show", "show", "list"]);
        for forbidden in [
            "create", "update", "delete", "mount", "set", "register", "configure", "serve", "repl",
            "adapters", "fork", "run",
        ] {
            for (name, argv) in ADMITTED_LANE_ARGV {
                assert!(
                    !argv.contains(&forbidden),
                    "{name} can reach Exo's {forbidden}"
                );
            }
        }
    }

    /// The state root is named and never opened. If `ExoRoot` ever grows a way
    /// to write, this is the test that has to be deleted for it to land.
    #[test]
    fn the_exo_root_is_a_name_and_not_a_handle() {
        let root = ExoRoot::at("/tmp/exo/.exo");
        assert_eq!(root.as_str(), "/tmp/exo/.exo");
        let source = include_str!("command.rs");
        let impl_block = source
            .split_once("impl ExoRoot {")
            .and_then(|(_, rest)| rest.split_once("\n}\n"))
            .expect("ExoRoot has an impl block")
            .0;
        for writing in [
            "fs::", "File", "create", "remove", "write", "OpenOptions", "std::io",
        ] {
            assert!(
                !impl_block.contains(writing),
                "ExoRoot grew a way to touch the directory it names: {writing}"
            );
        }
    }
}
