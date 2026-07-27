//! What the Exo agent behind this lane is allowed to be. `OMEGA-DELTA-0042`,
//! omega#87.
//!
//! Exo's differentiator is that its agent can rewrite itself. The flagship Exo
//! agent runs unrestricted networked `shell` in a sandbox with Exo's own source
//! tree mounted read-write, plus a `guardian_action` tool that builds and
//! restarts Exo. There is no approval prompt anywhere in Exo — the security
//! model is sandbox isolation, and Exo's threat model assumes you *want* the
//! agent to modify itself.
//!
//! Omega supplies the authority gate Exo lacks. The ordinary lane refuses
//! self-modification. A separate, typed, one-use grant can authorize one exact
//! turn after a visible human confirmation. The lane never silently enables
//! Exo's self-modification tools.
//!
//! # Read the agent, do not assume it
//!
//! "Never enables" is easy and nearly worthless on its own: the lane emits no
//! configuring command (see [`crate::command`]), so of course it enables
//! nothing. The capability that matters is the one the agent *already has*,
//! configured by whoever set Exo up — and an Omega lane pointed at the flagship
//! self-improving agent would be surfacing exactly the capability this packet
//! excludes, without Omega having enabled anything.
//!
//! So the lane reads `exo agent show` before it sends a turn and refuses when
//! the agent carries self-modification capability. That is a live observation of
//! the agent that is about to run, not an assertion about Omega's own argv.
//!
//! Three capabilities are refused, and each is one of the three things
//! `docs/RSI.md` and the flagship agent's own configuration use to rewrite Exo:
//!
//! * **agent-authored tools** — `tool_creation: enabled`, the agent writing
//!   TypeScript into `.exo/agent-tools/` at runtime;
//! * **a tool module** — `typescript_tool_modules: N` where `N > 0`, which is
//!   how `guardian_action` is installed (`examples/exo/guardian-tools.ts`);
//! * **a read-write mount** — a `sandbox_mounts` entry in `rw` mode, which is
//!   how Exo's source tree gets into the sandbox to be edited.
//!
//! Networking is deliberately **not** in that list. An agent with a shell and a
//! network is a high-capability agent, and it is also what any useful coding
//! agent is; refusing it would make the lane refuse everything and would say
//! nothing about self-modification. It is reported, so a reader sees it, and it
//! does not refuse.

/// What `exo agent show` said about the agent this lane points at.
///
/// Every field is read out of Exo's own output. Nothing here is a default this
/// file invented: a record Omega could not parse becomes
/// [`ExoAgentReadError`], not a permissive record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExoAgent {
    /// The agent's durable id, when Exo printed one.
    ///
    /// `OMEGA-DELTA-0102`, omega#104. Exo addresses every request by *id*
    /// (`AgentId = Uuid7` at the pin), and the lane holds only slugs — so
    /// nothing that reads Exo's durable log could name the agent it was already
    /// running turns on. `exo agent show` prints `id:` as its first line and
    /// this drops it no longer.
    ///
    /// `Option`, not required: an older `exo` that does not print the line
    /// still parses, and a caller that needs the id gets `None` rather than a
    /// lane that refuses to start. A missing id means "cannot address the
    /// durable record", which is a smaller failure than "cannot run a turn".
    pub id: Option<String>,
    /// The agent slug, as Exo reports it.
    pub slug: String,
    /// The executor Exo will run the turn with: `basic`, `rlm`, `typescript`,
    /// `codex`, `claude-code`, `cursor`, or a module path.
    pub harness: String,
    /// The model binding the agent resolves to.
    pub model: String,
    /// Whether the agent may author its own tools at runtime.
    pub agent_authored_tools: bool,
    /// How many TypeScript tool modules are loaded into it.
    pub tool_modules: u32,
    /// Exact TypeScript tool-module paths reported by Exo.
    pub tool_module_paths: Vec<String>,
    /// Whether any sandbox mount is read-write.
    pub read_write_mount: bool,
    /// Exact agent-level sandbox mounts.
    pub mounts: Vec<ExoMount>,
    /// Whether the agent's sandbox has a network. Reported, never refused.
    pub networking: bool,
}

/// A mount observed in an Exo agent or conversation record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExoMount {
    pub host_path: String,
    pub mount_path: String,
    pub read_write: bool,
}

/// Capability-bearing fields from `exo conversation show`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExoConversation {
    /// The conversation's durable id, when Exo printed one. See
    /// [`ExoAgent::id`] for why it is optional.
    pub id: Option<String>,
    pub slug: String,
    pub mounts: Vec<ExoMount>,
}

/// Why the lane will not run a turn on this agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfModification {
    /// `tool_creation: enabled`.
    AgentAuthoredTools,
    /// One or more `--tool-module` modules, the `guardian_action` shape.
    ToolModule,
    /// A read-write sandbox mount, the source-tree shape.
    ReadWriteMount,
}

impl SelfModification {
    /// The stable token this refusal is recorded under.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::AgentAuthoredTools => "agent_authored_tools",
            Self::ToolModule => "tool_module",
            Self::ReadWriteMount => "read_write_mount",
        }
    }

    /// Every capability the lane refuses, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::AgentAuthoredTools,
            Self::ToolModule,
            Self::ReadWriteMount,
        ]
    }
}

impl std::fmt::Display for SelfModification {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::AgentAuthoredTools => {
                "this Exo agent may write its own tools at runtime; the Omega lane does not \
                 surface Exo's self-modification"
            }
            Self::ToolModule => {
                "this Exo agent has a tool module loaded, which is how guardian_action is \
                 installed; the Omega lane does not surface Exo's self-modification"
            }
            Self::ReadWriteMount => {
                "this Exo agent has a read-write mount, which is how Exo's source tree is \
                 edited from inside the sandbox; the Omega lane does not surface Exo's \
                 self-modification"
            }
        })
    }
}

impl std::error::Error for SelfModification {}

/// Why `exo agent show` could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExoAgentReadError {
    /// A field the decision depends on was not in the output.
    MissingField(&'static str),
    /// A field was present and this build could not read its value.
    UnreadableField(&'static str),
}

impl std::fmt::Display for ExoAgentReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(field) => {
                write!(formatter, "exo agent show did not report {field}")
            }
            Self::UnreadableField(field) => {
                write!(formatter, "exo agent show reported an unreadable {field}")
            }
        }
    }
}

impl std::error::Error for ExoAgentReadError {}

impl ExoAgent {
    /// Read `exo agent show` output.
    ///
    /// Fails closed on every shape it does not recognise. Exo's house rule is
    /// that it does not keep backwards compatibility, so the realistic failure
    /// is not a malicious record — it is upstream renaming `tool_creation` in a
    /// commit nobody read, and a parser that defaulted the missing field to
    /// `false` would answer "no self-modification" about an agent it could no
    /// longer see. That is the one wrong answer this whole file exists to
    /// prevent, so an absent field is an error and never a default.
    ///
    /// # Errors
    ///
    /// [`ExoAgentReadError`] when a load-bearing field is missing or unreadable.
    pub fn parse(output: &str) -> Result<Self, ExoAgentReadError> {
        let field = |name: &'static str| -> Result<&str, ExoAgentReadError> {
            output
                .lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
                .map(str::trim)
                .ok_or(ExoAgentReadError::MissingField(name))
        };

        let tool_creation = match field("tool_creation")? {
            "enabled" => true,
            "disabled" => false,
            _ => return Err(ExoAgentReadError::UnreadableField("tool_creation")),
        };
        let tool_modules: u32 = field("typescript_tool_modules")?
            .parse()
            .map_err(|_| ExoAgentReadError::UnreadableField("typescript_tool_modules"))?;
        let networking = match field("enable_networking")? {
            "true" => true,
            "false" => false,
            _ => return Err(ExoAgentReadError::UnreadableField("enable_networking")),
        };

        let mounts = mounts(output, "sandbox_mounts")?;
        Ok(Self {
            id: durable_id(output),
            slug: field("slug")?.to_owned(),
            harness: field("harness")?.to_owned(),
            model: field("model")?.to_owned(),
            agent_authored_tools: tool_creation,
            tool_modules,
            tool_module_paths: tool_module_paths(output, tool_modules)?,
            read_write_mount: mounts.iter().any(|mount| mount.read_write),
            mounts,
            networking,
        })
    }

    /// Whether the lane may run a turn on this agent.
    ///
    /// # Errors
    ///
    /// The first self-modification capability found, in
    /// [`SelfModification::all`] order.
    pub fn admits_lane_turn(&self) -> Result<(), SelfModification> {
        if self.agent_authored_tools {
            return Err(SelfModification::AgentAuthoredTools);
        }
        if self.tool_modules > 0 {
            return Err(SelfModification::ToolModule);
        }
        if self.read_write_mount {
            return Err(SelfModification::ReadWriteMount);
        }
        Ok(())
    }
}

impl ExoConversation {
    /// Parse the conversation record and retain exact mount paths.
    pub fn parse(output: &str) -> Result<Self, ExoAgentReadError> {
        let slug = output
            .lines()
            .find_map(|line| line.strip_prefix("slug:"))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(ExoAgentReadError::MissingField("slug"))?
            .to_owned();
        Ok(Self {
            id: durable_id(output),
            slug,
            mounts: mounts(output, "mounts")?,
        })
    }

    /// Whether the conversation itself can modify a mounted host path.
    pub fn admits_lane_turn(&self) -> Result<(), SelfModification> {
        if self.mounts.iter().any(|mount| mount.read_write) {
            Err(SelfModification::ReadWriteMount)
        } else {
            Ok(())
        }
    }
}

/// The `id:` line of an `exo agent show` or `exo conversation show`.
///
/// `OMEGA-DELTA-0102`, omega#104. Exact prefix, and only at the start of a
/// line: `latest_event_id:` and `sandbox_id:` are also ids and are not this
/// one. `None` when the line is absent or empty, so a caller that needs the id
/// decides what to do about it rather than inheriting a guess.
fn durable_id(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("id:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn tool_module_paths(output: &str, expected: u32) -> Result<Vec<String>, ExoAgentReadError> {
    let mut lines = output.lines();
    lines
        .find(|line| line.starts_with("typescript_tool_modules:"))
        .ok_or(ExoAgentReadError::MissingField("typescript_tool_modules"))?;
    let paths = lines
        .take_while(|line| line.starts_with("  - "))
        .map(|line| line.trim_start_matches("  - ").to_owned())
        .collect::<Vec<_>>();
    if paths.len() != expected as usize {
        return Err(ExoAgentReadError::UnreadableField(
            "typescript_tool_modules",
        ));
    }
    Ok(paths)
}

/// Read the `sandbox_mounts:` block.
///
/// Exo prints `  none` for an empty set and `  <host> -> <path> (ro)` or
/// `(rw[, internal])` per mount. A block this build cannot read is an error for
/// the same reason a missing field is.
fn mounts(output: &str, field: &'static str) -> Result<Vec<ExoMount>, ExoAgentReadError> {
    let mut lines = output.lines();
    lines
        .find(|line| line.trim_end() == format!("{field}:"))
        .ok_or(ExoAgentReadError::MissingField(field))?;

    let mut mounts = Vec::new();
    for line in lines {
        let Some(entry) = line.strip_prefix("  ") else {
            break;
        };
        let entry = entry.trim();
        if entry == "none" {
            return Ok(Vec::new());
        }
        let Some((paths, mode)) = entry.rsplit_once(" (") else {
            return Err(ExoAgentReadError::UnreadableField(field));
        };
        let mode = mode.trim_end_matches(')');
        let read_write = match mode.split(',').next().map(str::trim) {
            Some("rw") => true,
            Some("ro") => false,
            _ => return Err(ExoAgentReadError::UnreadableField(field)),
        };
        let Some((host_path, mount_path)) = paths.split_once(" -> ") else {
            return Err(ExoAgentReadError::UnreadableField(field));
        };
        mounts.push(ExoMount {
            host_path: host_path.to_owned(),
            mount_path: mount_path.to_owned(),
            read_write,
        });
    }
    Ok(mounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `exo agent show` output, captured from the pinned Exo at
    /// `baa07f67` on 2026-07-25. Not a hand-written fixture: this is what the
    /// binary printed for the agent the lane was driven against.
    const DRIVEN_AGENT: &str = "\
id: 019f9cc4-0bf8-7fa2-8a48-eb84460661c1
slug: omega-lane
name: OmegaLane
harness: basic
typescript_module: none
typescript_tool_modules: 0
tool_creation: disabled
sandbox_image: default
sandbox_provider: apple-container
sandbox_scope: conversation
enable_networking: false
sandbox_mounts:
  none
model: gpt5mini
max_output_tokens: none
max_tool_round_trips: none
braintrust: none
";

    #[test]
    fn the_agent_the_lane_was_driven_against_is_admitted() {
        let agent = ExoAgent::parse(DRIVEN_AGENT).expect("real output parses");
        assert_eq!(agent.slug, "omega-lane");
        assert_eq!(agent.harness, "basic");
        assert_eq!(agent.model, "gpt5mini");
        assert!(!agent.agent_authored_tools);
        assert_eq!(agent.tool_modules, 0);
        assert!(!agent.read_write_mount);
        assert!(!agent.networking);
        assert_eq!(agent.admits_lane_turn(), Ok(()));
    }

    /// The flagship Exo agent, which is the thing this refusal exists for: the
    /// source tree mounted read-write, `guardian-tools.ts` loaded, and runtime
    /// tool authoring on.
    #[test]
    fn the_self_improving_exo_agent_is_refused() {
        let self_improving = DRIVEN_AGENT
            .replace("tool_creation: disabled", "tool_creation: enabled")
            .replace(
                "typescript_tool_modules: 0",
                "typescript_tool_modules: 1\n  - /workspace/exo/examples/exo/guardian-tools.ts",
            )
            .replace(
                "sandbox_mounts:\n  none",
                "sandbox_mounts:\n  /Users/x/exo -> /workspace/exo (rw)",
            );
        let agent = ExoAgent::parse(&self_improving).expect("parses");
        assert_eq!(
            agent.admits_lane_turn(),
            Err(SelfModification::AgentAuthoredTools)
        );
    }

    /// Each capability refuses on its own, so removing any one of the three
    /// checks fails a test rather than being masked by the other two.
    #[test]
    fn each_self_modification_capability_refuses_on_its_own() {
        let cases = [
            (
                "tool_creation: disabled",
                "tool_creation: enabled",
                SelfModification::AgentAuthoredTools,
            ),
            (
                "typescript_tool_modules: 0",
                "typescript_tool_modules: 2\n  - /tools/one.ts\n  - /tools/two.ts",
                SelfModification::ToolModule,
            ),
            (
                "sandbox_mounts:\n  none",
                "sandbox_mounts:\n  /Users/x/exo -> /workspace/exo (rw, internal)",
                SelfModification::ReadWriteMount,
            ),
        ];
        assert_eq!(cases.len(), SelfModification::all().len());
        for (from, to, expected) in cases {
            let agent = ExoAgent::parse(&DRIVEN_AGENT.replace(from, to)).expect("parses");
            assert_eq!(agent.admits_lane_turn(), Err(expected), "{to}");
        }
    }

    /// A read-only mount is not self-modification. Refusing it would make the
    /// gate refuse an ordinary working directory and say nothing about Exo
    /// rewriting itself.
    #[test]
    fn a_read_only_mount_is_not_self_modification() {
        let agent = ExoAgent::parse(&DRIVEN_AGENT.replace(
            "sandbox_mounts:\n  none",
            "sandbox_mounts:\n  /Users/x/project -> /workspace/project (ro)",
        ))
        .expect("parses");
        assert!(!agent.read_write_mount);
        assert_eq!(agent.admits_lane_turn(), Ok(()));
    }

    /// Networking is reported and does not refuse. See the module docs.
    #[test]
    fn a_networked_agent_is_reported_and_not_refused() {
        let agent = ExoAgent::parse(
            &DRIVEN_AGENT.replace("enable_networking: false", "enable_networking: true"),
        )
        .expect("parses");
        assert!(agent.networking);
        assert_eq!(agent.admits_lane_turn(), Ok(()));
    }

    /// `OMEGA-DELTA-0102`, omega#104. The id Exo addresses by, kept.
    ///
    /// Exo's protocol takes `AgentId = Uuid7` — never a slug — so a durable-log
    /// reader holding only the lane's `agent: String` cannot name the agent the
    /// lane is already running turns on. `exo agent show` prints the id as its
    /// first line, this parser used to drop it, and the whole read path was
    /// unreachable because of that one field.
    #[test]
    fn the_agent_record_keeps_the_id_exo_addresses_it_by() {
        let agent = ExoAgent::parse(DRIVEN_AGENT).expect("real output parses");
        assert_eq!(
            agent.id.as_deref(),
            Some("019f9cc4-0bf8-7fa2-8a48-eb84460661c1")
        );

        // `latest_event_id:` is also an id, and it is not this one. A prefix
        // match rather than a line-start match would read the wrong field on a
        // `conversation show`, and address every later request at an event.
        let conversation = ExoConversation::parse(
            "\
id: 019f9cc4-1111-7fa2-8a48-eb84460661c2
slug: omega-lane
latest_event_id: 019f9cc4-2222-7fa2-8a48-eb84460661c3
mounts:
  none
",
        )
        .expect("conversation parses");
        assert_eq!(
            conversation.id.as_deref(),
            Some("019f9cc4-1111-7fa2-8a48-eb84460661c2")
        );
    }

    /// An Exo that does not print an id still runs a turn.
    ///
    /// The id is optional on purpose: it is what a durable-log *read* needs, and
    /// refusing to parse without it would turn "cannot show the history" into
    /// "cannot run the agent", which is the larger failure of the two.
    #[test]
    fn an_output_with_no_id_line_still_parses_and_still_runs() {
        let without = DRIVEN_AGENT
            .lines()
            .filter(|line| !line.starts_with("id:"))
            .collect::<Vec<_>>()
            .join("\n");
        let agent = ExoAgent::parse(&without).expect("an Exo with no id line still parses");
        assert_eq!(agent.id, None);
        assert_eq!(agent.slug, "omega-lane");
        assert_eq!(agent.admits_lane_turn(), Ok(()));
    }

    #[test]
    fn a_conversation_only_read_write_mount_is_refused() {
        let shown = "\
slug: omega-lane
mounts:
  /Users/x/exo -> /workspace/exo (rw)
";
        let conversation = ExoConversation::parse(shown).expect("conversation parses");
        assert_eq!(
            conversation.admits_lane_turn(),
            Err(SelfModification::ReadWriteMount)
        );
        assert_eq!(conversation.mounts[0].host_path, "/Users/x/exo");
    }

    /// The upstream-rename failure, which is the realistic one. A field the
    /// decision depends on going missing must not read as "no capability".
    #[test]
    fn a_renamed_field_refuses_to_parse_rather_than_defaulting_to_permissive() {
        for field in [
            "tool_creation",
            "typescript_tool_modules",
            "sandbox_mounts",
            "harness",
            "model",
            "slug",
            "enable_networking",
        ] {
            let renamed = DRIVEN_AGENT
                .lines()
                .filter(|line| !line.starts_with(&format!("{field}:")))
                .collect::<Vec<_>>()
                .join("\n");
            let read = ExoAgent::parse(&renamed);
            assert!(
                read.is_err(),
                "dropping {field} still parsed, and the record would claim no capability: {read:?}"
            );
        }
    }

    #[test]
    fn an_unreadable_value_is_refused_rather_than_read_as_false() {
        for (from, to) in [
            ("tool_creation: disabled", "tool_creation: maybe"),
            (
                "typescript_tool_modules: 0",
                "typescript_tool_modules: some",
            ),
            ("enable_networking: false", "enable_networking: yes"),
        ] {
            assert!(
                ExoAgent::parse(&DRIVEN_AGENT.replace(from, to)).is_err(),
                "{to}"
            );
        }
    }
}
