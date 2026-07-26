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
//! keychain, and it starts no processes — asking an agent to identify itself
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
}

/// The set Omega offers, in the order it prefers them.
///
/// Codex is first because it is what the first message routes to.
pub const CANDIDATES: &[AgentCandidate] = &[
    AgentCandidate {
        id: "codex-acp",
        name: "Codex",
        binaries: &["codex"],
    },
    AgentCandidate {
        id: "claude-acp",
        name: "Claude",
        binaries: &["claude"],
    },
    AgentCandidate {
        id: "github-copilot-cli",
        name: "GitHub Copilot",
        binaries: &["copilot"],
    },
    AgentCandidate {
        id: "cursor",
        name: "Cursor",
        binaries: &["cursor-agent"],
    },
];

/// An agent found on disk, with the file that was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedAgent {
    pub id: &'static str,
    pub name: &'static str,
    pub binary: PathBuf,
}

/// Everything in [`CANDIDATES`] present on the given `PATH`, in preference
/// order.
///
/// The order is [`CANDIDATES`]' order, not `PATH`'s, so the choice Omega makes
/// does not change with the shell that launched it.
pub fn detect_on_path(path_var: &str) -> Vec<DetectedAgent> {
    CANDIDATES
        .iter()
        .filter_map(|candidate| {
            candidate
                .binaries
                .iter()
                .find_map(|binary| lookup(binary, path_var))
                .map(|binary| DetectedAgent {
                    id: candidate.id,
                    name: candidate.name,
                    binary,
                })
        })
        .collect()
}

/// [`detect_on_path`] against the process's own `PATH`.
///
/// A missing `PATH` yields nothing rather than falling back to a default set of
/// directories. Guessing where binaries live would make the empty case
/// unreachable, and the empty case is the one a new person is in.
pub fn detect_from_env() -> Vec<DetectedAgent> {
    match std::env::var("PATH") {
        Ok(path_var) => detect_on_path(&path_var),
        Err(_) => Vec::new(),
    }
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
