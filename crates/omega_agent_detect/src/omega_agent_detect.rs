//! Which coding agents are actually on this machine.
//!
//! omega#100. The onboarding screen decided whether an agent was "installed" by
//! asking the settings store:
//!
//! ```ignore
//! let installed_agents = cx.global::<SettingsStore>().get::<AllAgentServersSettings>(None);
//! let is_installed = installed_agents.contains_key(featured_agent.id);
//! ```
//!
//! That reads **configured**, not **present**. A key in that map means someone
//! wrote a setting, not that a binary exists. The two agree on a machine that
//! has been used for a while, which is exactly why the green check beside Codex
//! looked like working detection. They disagree in the case that matters: a
//! brand new `--user-data-dir` has no settings written, so a settings-keyed
//! check finds nothing however many agents are installed.
//!
//! This module answers the other question, and only that one. It looks for
//! executables on `PATH`. It does not read settings, it does not read the
//! credential storage, and it starts no processes — asking an agent to identify itself
//! costs a process launch on the startup path, and a file that is present and
//! executable is enough to offer it.
//!
//! `PATH` is a parameter rather than an ambient read, so the behaviour is
//! testable without a machine that happens to have the right binaries. That
//! matters more than usual here: the mode this feeds is entered by a
//! command-line flag, and a defect that made it completely unusable once
//! passed `cargo check`, `cargo test` and clippy together, because none of them
//! launches the binary.

pub mod exo;

use std::path::{Path, PathBuf};

/// How Omega starts this agent's ACP server once it has been detected.
///
/// OMEGA-DELTA-0203. Detection and launch were two independent registries, and
/// an agent could be (and `scv`, `cursor` and `github-copilot-cli` all were)
/// advertised as delegable while the launcher had no definition for it. This
/// field is what makes "listed" and "startable" one fact instead of two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentLaunch {
    /// Started through the agent-server store, which resolves the id from
    /// settings and the ACP registry. The id MUST have an entry in
    /// `assets/settings/default.json`'s `agent_servers`, or the store cannot
    /// start it; `omega_deltas` checks that mechanically.
    AgentServerStore,
    /// The detected executable IS the ACP server, started directly by the path
    /// detection resolved. Nothing in settings is required.
    DetectedBinary { args: &'static [&'static str] },
}

/// What a delegated task has to look like before this agent can answer it.
///
/// OMEGA-DELTA-0209. `AgentLaunch` made "offered" and "startable" one fact, and
/// the next one was still two: Omega sent every delegation as prose, because
/// every agent it had ever delegated to had a model to read prose with. `scv`
/// does not. It is a deterministic tool server, its prompt must *be* a JSON
/// tool request, and a prose task reached it as
/// `invalid tool request: expected value at line 1 column 1` — a launched
/// agent, a live session, and nothing it could do with what it was sent.
///
/// This field is the third half of the same law: an agent Omega offers to
/// delegate to must be one Omega can start **and** one Omega sends a shape it
/// accepts. There is no default and no third case, so a candidate added without
/// a contract does not compile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentPromptContract {
    /// A model-backed agent. The task is prose and the agent interprets it.
    Prose,
    /// A deterministic server with no model. The task must be exactly one JSON
    /// request, and nothing else will do.
    Structured(StructuredPrompt),
}

/// The exact request a [`AgentPromptContract::Structured`] agent answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StructuredPrompt {
    /// The tools the agent will answer, so a declaration that drifts from what
    /// the agent advertises over ACP can be caught rather than discovered by a
    /// delegation that fails.
    pub tools: &'static [&'static str],
    /// The request, spelled the way the delegating model is told to emit it.
    ///
    /// This is the string that reaches the model, so it is the shape and not a
    /// description of the shape: a model handed a sentence about JSON writes a
    /// sentence, and a model handed JSON writes JSON.
    pub request: &'static str,
}

/// An agent Omega knows how to drive, and the executables it might be called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentCandidate {
    /// The id used by `AllAgentServersSettings` and the onboarding grid, so a
    /// detected agent and a configured one can be compared without a mapping
    /// table in between.
    pub id: &'static str,
    pub name: &'static str,
    /// In preference order. Cursor ships its CLI as `cursor-agent`, and
    /// `cursor` is commonly a shell alias to it, which a `PATH` scan cannot
    /// see — so the real binary is named first.
    pub binaries: &'static [&'static str],
    /// OMEGA-DELTA-0203. How the launcher starts it once it is found.
    pub launch: AgentLaunch,
    /// OMEGA-DELTA-0209. What a delegated task must look like for it.
    pub prompt: AgentPromptContract,
}

/// The set Omega offers, in the order it prefers them.
///
/// Codex is first because it is what the first message routes to.
pub const CANDIDATES: &[AgentCandidate] = &[
    AgentCandidate {
        id: "codex-acp",
        name: "Codex",
        binaries: &["codex"],
        launch: AgentLaunch::AgentServerStore,
        prompt: AgentPromptContract::Prose,
    },
    AgentCandidate {
        id: "claude-acp",
        name: "Claude",
        binaries: &["claude"],
        launch: AgentLaunch::AgentServerStore,
        prompt: AgentPromptContract::Prose,
    },
    AgentCandidate {
        id: "grok",
        name: "Grok",
        binaries: &["grok"],
        launch: AgentLaunch::AgentServerStore,
        prompt: AgentPromptContract::Prose,
    },
    AgentCandidate {
        id: "github-copilot-cli",
        name: "GitHub Copilot",
        binaries: &["copilot"],
        launch: AgentLaunch::AgentServerStore,
        prompt: AgentPromptContract::Prose,
    },
    AgentCandidate {
        id: "cursor",
        name: "Cursor",
        binaries: &["cursor-agent"],
        launch: AgentLaunch::AgentServerStore,
        prompt: AgentPromptContract::Prose,
    },
    // `scv` is first-party and ships beside the `omega` executable, so it is
    // started as the file detection found rather than through a settings entry
    // naming a bare `scv` that only a shell `PATH` could resolve.
    // `scv` has no model, so it is the one candidate whose task is a request
    // rather than a sentence. `SCV_REQUEST` is `scv::PROMPT_REQUEST_SHAPE`,
    // spelled here because this crate is a leaf and the hyper-lightweight agent
    // must not gain a dependency on Omega's catalog to be described by it;
    // `omega_deltas` asserts the two spellings agree by calling both.
    AgentCandidate {
        id: "scv",
        name: "SCV",
        binaries: &["scv"],
        launch: AgentLaunch::DetectedBinary { args: &[] },
        prompt: AgentPromptContract::Structured(StructuredPrompt {
            tools: &["read"],
            request: SCV_REQUEST,
        }),
    },
];

/// SCV's request, as the delegating model is told to write it.
pub const SCV_REQUEST: &str =
    r#"{"tool":"read","arguments":{"path":"/absolute/path","offset":1,"limit":2000}}"#;

/// An agent found on disk, with the file that was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedAgent {
    pub id: &'static str,
    pub name: &'static str,
    pub binary: PathBuf,
    /// OMEGA-DELTA-0203. Carried from the candidate so a caller holding a
    /// `DetectedAgent` knows how to start it without a second lookup, which is
    /// where the two registries got to disagree.
    pub launch: AgentLaunch,
    /// OMEGA-DELTA-0209. Carried for the same reason as `launch`: the caller
    /// that is about to send this agent a task must not have to look up
    /// elsewhere what shape it takes.
    pub prompt: AgentPromptContract,
}

/// Everything in [`CANDIDATES`] present on the given `PATH`, in preference
/// order.
///
/// The order is [`CANDIDATES`]' order, not `PATH`'s, so the choice Omega makes
/// does not change with the shell that launched it.
pub fn detect_on_path(path_var: &str) -> Vec<DetectedAgent> {
    detect_with(None, path_var)
}

/// [`detect_on_path`], preferring `sibling_dir` over `path_var`.
///
/// OMEGA-DELTA-0203. A first-party agent such as `scv` ships beside the `omega`
/// executable inside the application bundle, so the running executable's own
/// directory is the authoritative place to find it: a packaged Omega has no
/// shell `PATH` to rely on, and a stale hand-copied binary on `PATH` must not
/// beat the one that was shipped and signed with this build.
pub fn detect_with(sibling_dir: Option<&Path>, path_var: &str) -> Vec<DetectedAgent> {
    CANDIDATES
        .iter()
        .filter_map(|candidate| {
            candidate
                .binaries
                .iter()
                .find_map(|binary| lookup_beside_then_on_path(binary, sibling_dir, path_var))
                .map(|binary| DetectedAgent {
                    id: candidate.id,
                    name: candidate.name,
                    binary,
                    launch: candidate.launch,
                    prompt: candidate.prompt,
                })
        })
        .collect()
}

/// [`detect_with`] against the running executable's directory and the process's
/// own `PATH`.
///
/// A missing `PATH` contributes no directories rather than falling back to a
/// default set. Guessing where binaries live would make the empty case
/// unreachable, and the empty case is the one a new person is in. The
/// executable's own directory is not a guess: it is where this build put the
/// agents it ships.
pub fn detect_from_env() -> Vec<DetectedAgent> {
    let executable_directory = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(Path::to_path_buf));
    let path_var = std::env::var("PATH").unwrap_or_default();
    detect_with(executable_directory.as_deref(), &path_var)
}

/// [`detect_from_env`], computed once for the life of the process.
///
/// omega#100. A surface asks this on every draw, and the answer costs one
/// `stat` per candidate binary per `PATH` entry. `PATH` does not change inside
/// a process, so a per-frame filesystem walk would be a syscall storm answering
/// the same question. The cost of the cache is that installing an agent while
/// Omega is running is not noticed until it restarts, which is the same
/// staleness a `PATH` read has anyway.
pub fn detected() -> &'static [DetectedAgent] {
    static DETECTED: std::sync::OnceLock<Vec<DetectedAgent>> = std::sync::OnceLock::new();
    DETECTED.get_or_init(detect_from_env)
}

/// The agent the first message should route to, if it is here.
///
/// Deliberately not "the first thing found": omega#100 asks for Codex
/// specifically, and silently routing to a different agent because Codex was
/// missing would be a worse answer than saying nothing was found.
pub fn preferred(detected: &[DetectedAgent]) -> Option<&DetectedAgent> {
    detected.iter().find(|agent| agent.id == "codex-acp")
}

/// Whether a configured launch program would resolve when spawned.
///
/// omega#169. The executor menu rendered a settings-declared custom agent as
/// a normal enabled row because the settings store answers **configured**,
/// not **present** — the row then failed honestly with exit 127 only after
/// the person selected it. This answers presence for an arbitrary configured
/// command the same way the spawning shell will:
///
/// - an absolute program is checked as that file,
/// - a bare name is searched on `path_var`,
/// - a relative program with a separator is assumed present, because it
///   resolves against the launch directory and this check cannot know that
///   directory. Claiming absence it cannot verify would dim a workable row,
///   which is the same dishonesty in the other direction.
pub fn command_resolves(program: &Path, path_var: &str) -> bool {
    if program.is_absolute() {
        is_executable_file(program)
    } else if program.components().count() > 1 {
        true
    } else {
        program
            .to_str()
            .is_some_and(|name| lookup(name, path_var).is_some())
    }
}

/// [`command_resolves`] against the process's own `PATH`.
///
/// A missing `PATH` resolves nothing, matching [`detect_from_env`]: guessing
/// directories would make the empty case unreachable, and the empty case is
/// exactly the stripped-PATH launch that surfaced omega#169.
pub fn command_resolves_from_env(program: &Path) -> bool {
    if program.is_absolute() || program.components().count() > 1 {
        return command_resolves(program, "");
    }
    match std::env::var("PATH") {
        Ok(path_var) => command_resolves(program, &path_var),
        Err(_) => false,
    }
}

/// The launch spec [`CANDIDATES`] declares for `id`, if it is one Omega knows.
pub fn launch_for(id: &str) -> Option<AgentLaunch> {
    CANDIDATES
        .iter()
        .find(|candidate| candidate.id == id)
        .map(|candidate| candidate.launch)
}

/// The prompt contract [`CANDIDATES`] declares for `id`, if Omega knows it.
///
/// `None` means Omega has never heard of this id, which is a different answer
/// from [`AgentPromptContract::Prose`]. An unknown id is not assumed to read
/// prose: the caller decides what to do with an agent nobody declared.
pub fn prompt_contract_for(id: &str) -> Option<AgentPromptContract> {
    CANDIDATES
        .iter()
        .find(|candidate| candidate.id == id)
        .map(|candidate| candidate.prompt)
}

/// Turn a task written for a [`AgentPromptContract::Structured`] agent into the
/// exact text that agent accepts, or say why it cannot be turned into one.
///
/// OMEGA-DELTA-0209. Shaping, not interpreting. A structured agent has no
/// model, so there is nothing here that could read a sentence and decide what
/// it meant — and inventing a request from prose would be worse than failing,
/// because a `read` of a file nobody asked for is an answer to a question
/// nobody asked. Exactly two things happen:
///
/// - surrounding whitespace and **one** Markdown code fence are removed, because
///   a model told to emit JSON very often emits JSON in a fence, and stripping a
///   fence is a deterministic unwrapping rather than a guess about meaning;
/// - what is left must parse as a JSON object.
///
/// Anything else is refused in one line that names the shape, which is the line
/// the delegating model reads. `expected value at line 1 column 1` — what the
/// owner was shown — says nothing a reader can act on.
pub fn shape_delegated_task(
    agent_name: &str,
    contract: &StructuredPrompt,
    task: &str,
) -> Result<String, String> {
    let refusal = || {
        format!(
            "{agent_name} takes a JSON tool request as the task, not prose. Send: {}",
            contract.request
        )
    };

    let text = strip_one_code_fence(task.trim()).trim();
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(_)) => Ok(text.to_owned()),
        Ok(_) | Err(_) => Err(refusal()),
    }
}

/// The body of a single Markdown code fence, or the input unchanged.
///
/// Only a fence that opens on the first line and closes on the last is removed,
/// so text that merely *contains* a fenced block is left alone: taking a block
/// out of the middle of prose would be choosing which part of a message was the
/// request, and that is interpretation.
fn strip_one_code_fence(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    let Some((_language, body)) = rest.split_once('\n') else {
        return text;
    };
    match body.trim_end().strip_suffix("```") {
        Some(body) => body,
        None => text,
    }
}

/// Find `binary` beside the running executable first, then on `path_var`.
fn lookup_beside_then_on_path(
    binary: &str,
    sibling_dir: Option<&Path>,
    path_var: &str,
) -> Option<PathBuf> {
    sibling_dir
        .map(|directory| directory.join(binary))
        .filter(|candidate| is_executable_file(candidate))
        .or_else(|| lookup(binary, path_var))
}

/// Find `binary` on `path_var`, returning the first executable match.
///
/// Empty entries are skipped. POSIX reads an empty `PATH` entry as the current
/// directory, and resolving an agent binary out of whatever directory Omega
/// happens to have been launched from is a way to run something unintended.
fn lookup(binary: &str, path_var: &str) -> Option<PathBuf> {
    path_var
        .split(':')
        .filter(|directory| !directory.is_empty())
        .map(|directory| Path::new(directory).join(binary))
        .find(|candidate| is_executable_file(candidate))
}

#[cfg(unix)]
pub(crate) fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub(crate) fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(unix)]
    fn write_executable(directory: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = directory.join(name);
        fs::write(&path, "#!/bin/sh\n").expect("the fixture binary is written");
        let mut permissions = fs::metadata(&path)
            .expect("the fixture binary exists")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("the fixture binary is executable");
        path
    }

    #[test]
    fn an_agent_on_the_path_is_found() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        write_executable(directory.path(), "codex");

        let detected = detect_on_path(&directory.path().to_string_lossy());

        assert_eq!(detected.len(), 1, "exactly the one written is found");
        assert_eq!(detected[0].id, "codex-acp");
        assert_eq!(detected[0].name, "Codex");
    }

    #[test]
    fn nothing_installed_finds_nothing() {
        let directory = tempfile::tempdir().expect("a temporary directory");

        assert!(
            detect_on_path(&directory.path().to_string_lossy()).is_empty(),
            "an empty directory yields no agents, which is the state a new \
             person is in and the one the composer has to say something about"
        );
    }

    /// The distinction the settings-keyed check could not make.
    #[test]
    fn a_file_that_is_not_executable_is_not_an_agent() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        fs::write(directory.path().join("codex"), "not a program")
            .expect("the fixture file is written");

        assert!(
            detect_on_path(&directory.path().to_string_lossy()).is_empty(),
            "a present but non-executable file is not an installed agent"
        );
    }

    #[test]
    fn the_order_is_the_candidate_order_not_the_path_order() {
        let first = tempfile::tempdir().expect("a temporary directory");
        let second = tempfile::tempdir().expect("a temporary directory");
        // Claude earlier on PATH than Codex, and Codex still leads.
        write_executable(first.path(), "claude");
        write_executable(second.path(), "codex");
        let path_var = format!("{}:{}", first.path().display(), second.path().display());

        let detected = detect_on_path(&path_var);

        assert_eq!(
            detected.iter().map(|agent| agent.id).collect::<Vec<_>>(),
            vec!["codex-acp", "claude-acp"],
            "the shell that launched Omega must not change which agent it picks"
        );
    }

    #[test]
    fn grok_on_the_path_is_found() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        write_executable(directory.path(), "grok");

        let detected = detect_on_path(&directory.path().to_string_lossy());

        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].id, "grok");
        assert_eq!(detected[0].name, "Grok");
    }

    #[test]
    fn an_empty_path_entry_is_not_the_current_directory() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        write_executable(directory.path(), "codex");
        let previous = std::env::current_dir().expect("a current directory");
        std::env::set_current_dir(directory.path()).expect("the current directory is set");

        let detected = detect_on_path("::");

        std::env::set_current_dir(previous).expect("the current directory is restored");
        assert!(
            detected.is_empty(),
            "an empty PATH entry must not resolve an agent out of the launch \
             directory"
        );
    }

    #[test]
    fn codex_is_preferred_and_a_missing_codex_is_not_substituted() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        write_executable(directory.path(), "claude");

        let detected = detect_on_path(&directory.path().to_string_lossy());

        assert_eq!(detected.len(), 1, "Claude is found");
        assert!(
            preferred(&detected).is_none(),
            "with Codex absent the first message must not be silently routed to \
             a different agent"
        );
    }

    #[test]
    fn a_bare_command_resolves_only_when_it_is_on_the_path() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        write_executable(directory.path(), "grok");
        let path_var = directory.path().to_string_lossy().to_string();

        assert!(command_resolves(Path::new("grok"), &path_var));
        assert!(
            !command_resolves(Path::new("grok"), ""),
            "omega#169: a configured agent whose command is not on PATH is \
             not present, however firmly the settings store believes in it"
        );
        assert!(!command_resolves(Path::new("absent-agent"), &path_var));
    }

    #[test]
    fn an_absolute_command_resolves_only_as_that_file() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let binary = write_executable(directory.path(), "grok");

        assert!(command_resolves(&binary, ""));
        assert!(!command_resolves(&directory.path().join("absent"), ""));
    }

    #[test]
    fn a_relative_command_with_a_separator_is_assumed_present() {
        assert!(
            command_resolves(Path::new("./agents/grok"), ""),
            "presence relative to an unknown launch directory cannot be \
             verified, and a false absence would dim a workable row"
        );
    }

    /// OMEGA-DELTA-0203. The shipped binary beside the application wins.
    ///
    /// `scv` is first-party and packaged into `Contents/MacOS`. A stale copy
    /// someone put on `PATH` by hand must not be preferred over the one this
    /// build shipped, because the two can be different programs.
    #[test]
    fn a_binary_beside_the_executable_wins_over_one_on_the_path() {
        let beside = tempfile::tempdir().expect("a temporary directory");
        let on_path = tempfile::tempdir().expect("a temporary directory");
        let shipped = write_executable(beside.path(), "scv");
        write_executable(on_path.path(), "scv");

        let detected = detect_with(Some(beside.path()), &on_path.path().to_string_lossy());

        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].id, "scv");
        assert_eq!(
            detected[0].binary, shipped,
            "the binary shipped with this build is the one that is started"
        );
    }

    #[test]
    fn a_missing_sibling_directory_falls_back_to_the_path() {
        let absent = tempfile::tempdir().expect("a temporary directory");
        let on_path = tempfile::tempdir().expect("a temporary directory");
        let installed = write_executable(on_path.path(), "scv");
        let sibling = absent.path().join("no-such-directory");

        let detected = detect_with(Some(&sibling), &on_path.path().to_string_lossy());

        assert_eq!(detected.len(), 1, "an unbundled build still finds an agent");
        assert_eq!(detected[0].binary, installed);
    }

    #[test]
    fn a_non_executable_file_beside_the_executable_is_not_accepted() {
        let beside = tempfile::tempdir().expect("a temporary directory");
        let on_path = tempfile::tempdir().expect("a temporary directory");
        fs::write(beside.path().join("scv"), "not a program").expect("the fixture file is written");
        let installed = write_executable(on_path.path(), "scv");

        let detected = detect_with(Some(beside.path()), &on_path.path().to_string_lossy());

        assert_eq!(
            detected.len(),
            1,
            "a present but non-executable sibling file is not an agent, and must \
             not shadow the one that can actually be started"
        );
        assert_eq!(detected[0].binary, installed);
    }

    /// OMEGA-DELTA-0203. Listing an agent and being able to start it are one
    /// fact. `scv` is started as the file that was found; Codex goes through
    /// the agent-server store, which requires a settings entry.
    #[test]
    fn every_candidate_declares_how_it_is_launched() {
        assert_eq!(
            launch_for("scv"),
            Some(AgentLaunch::DetectedBinary { args: &[] }),
            "SCV ships beside the application, so a settings entry naming a bare \
             `scv` would only ever resolve from a shell PATH the bundle does not \
             have"
        );
        assert_eq!(launch_for("codex-acp"), Some(AgentLaunch::AgentServerStore));
        assert_eq!(launch_for("not-an-agent"), None);
    }

    #[test]
    fn a_detected_agent_carries_its_launch_spec() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        write_executable(directory.path(), "scv");
        write_executable(directory.path(), "codex");

        let detected = detect_on_path(&directory.path().to_string_lossy());

        let launches: Vec<(&str, AgentLaunch)> = detected
            .iter()
            .map(|agent| (agent.id, agent.launch))
            .collect();
        assert_eq!(
            launches,
            vec![
                ("codex-acp", AgentLaunch::AgentServerStore),
                ("scv", AgentLaunch::DetectedBinary { args: &[] }),
            ],
            "a caller holding a DetectedAgent must not need a second lookup to \
             learn how to start it"
        );
    }

    /// OMEGA-DELTA-0209. Every candidate declares what a task must look like.
    ///
    /// The enum has no default, so this is not checking that the field is set —
    /// it cannot be unset. It is checking that both arms stay populated by the
    /// shipped catalog, because an arm no candidate takes is an arm nothing
    /// exercises.
    #[test]
    fn every_candidate_declares_the_shape_of_a_task() {
        assert_eq!(
            prompt_contract_for("scv"),
            Some(AgentPromptContract::Structured(StructuredPrompt {
                tools: &["read"],
                request: SCV_REQUEST,
            })),
            "SCV has no model, so a task it can answer is a request and not a \
             sentence"
        );
        assert_eq!(
            prompt_contract_for("codex-acp"),
            Some(AgentPromptContract::Prose)
        );
        assert_eq!(
            prompt_contract_for("not-an-agent"),
            None,
            "an id nobody declared must not be assumed to read prose"
        );

        let structured = CANDIDATES
            .iter()
            .filter(|candidate| matches!(candidate.prompt, AgentPromptContract::Structured(_)))
            .count();
        let prose = CANDIDATES.len() - structured;
        assert!(
            structured > 0 && prose > 0,
            "both prompt contracts must remain exercised by the shipped catalog"
        );
    }

    #[test]
    fn a_detected_agent_carries_its_prompt_contract() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        write_executable(directory.path(), "scv");

        let detected = detect_on_path(&directory.path().to_string_lossy());

        assert!(matches!(
            detected[0].prompt,
            AgentPromptContract::Structured(_)
        ));
    }

    fn scv_contract() -> StructuredPrompt {
        match prompt_contract_for("scv").expect("SCV is in the catalog") {
            AgentPromptContract::Structured(contract) => contract,
            AgentPromptContract::Prose => panic!("SCV has no model to read prose with"),
        }
    }

    #[test]
    fn a_json_task_is_sent_as_written() {
        let request = r#"{"tool":"read","arguments":{"path":"/tmp/a","limit":2}}"#;
        assert_eq!(
            shape_delegated_task("SCV", &scv_contract(), request),
            Ok(request.to_owned())
        );
    }

    /// A model told to emit JSON very often emits JSON in a fence.
    #[test]
    fn one_surrounding_code_fence_is_removed() {
        let request = r#"{"tool":"read","arguments":{"path":"/tmp/a"}}"#;
        for fenced in [
            format!("```json\n{request}\n```"),
            format!("```\n{request}\n```"),
            format!("  ```json\n{request}\n```  "),
        ] {
            assert_eq!(
                shape_delegated_task("SCV", &scv_contract(), &fenced),
                Ok(request.to_owned()),
                "{fenced}"
            );
        }
    }

    /// The owner's defect, at the seam that now catches it.
    ///
    /// The refusal names the shape. `expected value at line 1 column 1` was
    /// what a person and a model both got, and neither could act on it.
    #[test]
    fn prose_is_refused_by_naming_the_shape() {
        let error = shape_delegated_task(
            "SCV",
            &scv_contract(),
            "Perform a read-only test delegation: report the project root path \
             and list one or two top-level entries. Do not modify files.",
        )
        .expect_err("prose is not a tool request");

        assert!(error.contains(SCV_REQUEST), "{error}");
        assert_eq!(
            error.lines().count(),
            1,
            "one line, not a paragraph: {error}"
        );
    }

    /// Nothing is invented from prose that happens to hold a request.
    ///
    /// Choosing which part of a message was the request is interpretation, and
    /// a `read` of a file nobody asked for is an answer to a question nobody
    /// asked.
    #[test]
    fn a_request_buried_in_prose_is_not_dug_out() {
        let error = shape_delegated_task(
            "SCV",
            &scv_contract(),
            "Please run this for me:\n```json\n{\"tool\":\"read\"}\n```\nThanks.",
        )
        .expect_err("a fence in the middle of a message is not the message");
        assert!(error.contains(SCV_REQUEST), "{error}");
    }

    /// JSON that is not an object is not a tool request either.
    #[test]
    fn a_json_scalar_is_not_a_request() {
        for task in ["42", "\"read\"", "[1,2]", "null"] {
            assert!(
                shape_delegated_task("SCV", &scv_contract(), task).is_err(),
                "{task}"
            );
        }
    }

    #[test]
    fn codex_is_returned_when_it_is_present() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        write_executable(directory.path(), "codex");
        write_executable(directory.path(), "claude");

        let detected = detect_on_path(&directory.path().to_string_lossy());

        assert_eq!(
            preferred(&detected).map(|agent| agent.id),
            Some("codex-acp")
        );
    }
}
