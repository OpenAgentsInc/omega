//! Which directory a thread should be able to see.
//!
//! `OMEGA-DELTA-0054`, omega#100. Zero base opened no project. The comment in
//! `crates/omega/src/zed.rs` said so plainly — "no project is opened, so there is
//! no buffer for them to show" — and the consequence was not a missing buffer.
//! It was that the workspace had no worktrees, so `grep`, `find_path`,
//! `list_directory`, `read_file` and `terminal` all had nothing to operate on.
//! The owner ran several searches, every one returned no matches, and the agent
//! reported that the workspace appeared to be empty. That is literally correct
//! and completely useless.
//!
//! The awkward part is that Omega is started two ways and they disagree. From a
//! terminal the working directory is meaningful and is almost certainly what
//! the person means. From Finder or the Dock it is `/` and means nothing. So
//! the question this module answers is not "what is the working directory" but
//! "is the working directory something a person chose".
//!
//! # Why this is a plausibility test and not a project test
//!
//! It would be easy to require a marker — a `.git` directory, a `Cargo.toml` —
//! and it would be wrong. A plain folder of files is a legitimate thing to
//! point an agent at, and a rule that refused it would refuse the case the
//! owner is most likely to be in. So the test is the other way round: reject
//! the directories that cannot have been meant, and accept the rest.
//!
//! # Why it takes its inputs as parameters
//!
//! The same reason `omega_agent_detect` does. This decides what a shipped
//! binary does on startup, and startup is the one path no test in this
//! repository reaches — `cargo check`, `cargo test` and clippy were all green
//! across a defect that made the mode completely unusable. A predicate that
//! reads the ambient process state can only be tested on a machine that happens
//! to be in the right state.

#![deny(missing_docs)]

use std::path::{Path, PathBuf};

/// Directory prefixes a launcher hands over, and a person never means.
///
/// Not a general "system directory" list. Every entry is somewhere Omega is
/// actually started from by something other than a person in a shell: `/` is
/// what Finder and the Dock give, `/private/var/folders` is a sandbox scratch
/// root, and the rest are where the bundle and its libraries live. Anything
/// else is accepted, because a directory this list does not name is one
/// somebody chose to be in.
pub const LAUNCHER_PREFIXES: &[&str] = &[
    "/Applications",
    "/Library",
    "/System",
    "/bin",
    "/dev",
    "/etc",
    "/private/var/folders",
    "/sbin",
    "/usr",
];

/// Why a working directory was not opened as a project.
///
/// Typed rather than a `bool` so the surface can say which of these happened.
/// "No folder is open" and "the folder you are in is the whole disk" are
/// different sentences, and a person who reads the wrong one goes looking in
/// the wrong place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotAProjectRoot {
    /// The path does not exist, or is not a directory.
    NotADirectory,
    /// The path is relative, so it names nothing on its own.
    NotAbsolute,
    /// The filesystem root. What Finder and the Dock hand over.
    FilesystemRoot,
    /// The person's home directory itself.
    ///
    /// Rejected rather than accepted, because opening a home directory as a
    /// project means scanning everything a person owns to answer one question.
    HomeDirectory,
    /// Inside one of [`LAUNCHER_PREFIXES`].
    LauncherDirectory,
}

/// Is this working directory something a person chose to be in?
///
/// `home` is a parameter rather than a read of `$HOME`, so the home rule can be
/// tested without a machine whose home directory is in the right place.
///
/// # Errors
///
/// Returns the reason it is not, so a caller can say which one in one line.
pub fn plausible_project_root(cwd: &Path, home: Option<&Path>) -> Result<PathBuf, NotAProjectRoot> {
    if !cwd.is_absolute() {
        return Err(NotAProjectRoot::NotAbsolute);
    }
    if cwd.parent().is_none() {
        return Err(NotAProjectRoot::FilesystemRoot);
    }
    if home.is_some_and(|home| home == cwd) {
        return Err(NotAProjectRoot::HomeDirectory);
    }
    if LAUNCHER_PREFIXES
        .iter()
        .any(|prefix| cwd.starts_with(prefix))
    {
        return Err(NotAProjectRoot::LauncherDirectory);
    }
    if !cwd.is_dir() {
        return Err(NotAProjectRoot::NotADirectory);
    }
    Ok(cwd.to_path_buf())
}

/// [`plausible_project_root`] against the process's own state.
///
/// A working directory that cannot be read is treated exactly like an
/// implausible one: the caller opens no project and says so. Guessing a
/// directory here would put an agent's file tools somewhere nobody named.
///
/// # Errors
///
/// Returns the reason, as [`plausible_project_root`] does.
pub fn from_env() -> Result<PathBuf, NotAProjectRoot> {
    let cwd = std::env::current_dir().map_err(|_| NotAProjectRoot::NotADirectory)?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    plausible_project_root(&cwd, home.as_deref())
}

/// The project root a path *argument* names.
///
/// `OMEGA-DELTA-0116`, omega#111. `omega <path>` used to mean "open the
/// editor", and that made the most ordinary thing a person types a way out of a
/// mode the owner asked to have no way out. A path argument now names the
/// **project** and never the **mode**, so this is the function that turns the
/// argument into the directory the thread will be able to see.
///
/// It is [`plausible_project_root`] with two more jobs, both of which exist
/// because a command line is not a working directory:
///
/// - **A relative argument is resolved against `cwd`.** `omega .` and
///   `omega ../sibling` are what a person in a shell types; the rule that
///   rejects relative paths is about a *working directory* that names nothing
///   on its own, and an argument is named relative to somewhere.
/// - **A file argument becomes the directory that holds it.** `omega
///   src/main.rs` in zero base cannot show a buffer, so the useful reading of
///   that command is "work in `src`" — which the thread's `grep`, `read_file`
///   and `terminal` can all act on — rather than a single-file worktree the
///   agent can see one file through.
///
/// A path that does not exist is refused rather than climbed. Falling back to
/// the parent of a typo would silently open a directory nobody named, which is
/// the failure [`plausible_project_root`] exists to prevent.
///
/// `.` components are dropped so the opened root reads as a person wrote it
/// (`/a/b`, not `/a/b/.`). `..` is left alone: resolving it means resolving
/// symlinks, and that turns a path a person recognises into one they do not.
///
/// Inputs are parameters for this module's usual reason: startup is the path no
/// test in this repository reaches.
///
/// # Errors
///
/// Returns the reason, as [`plausible_project_root`] does.
pub fn project_root_named(
    path: &Path,
    cwd: &Path,
    home: Option<&Path>,
) -> Result<PathBuf, NotAProjectRoot> {
    let named = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let named: PathBuf = named.components().collect();
    let directory = if named.is_dir() {
        named
    } else if named.is_file() {
        named
            .parent()
            .ok_or(NotAProjectRoot::NotADirectory)?
            .to_path_buf()
    } else {
        return Err(NotAProjectRoot::NotADirectory);
    };
    plausible_project_root(&directory, home)
}

/// How a working directory is written where a person reads it.
///
/// `OMEGA-DELTA-0116`, omega#111. The owner asked an agent which directory it
/// was in and got an answer he did not expect, because nothing in the window
/// named it. This is the one spelling of that fact, so the header and anything
/// that comes after it cannot disagree about the same directory.
///
/// `$HOME` becomes `~`, because the distinguishing part of a checkout path is
/// its tail and a home prefix spends the width that tail needs. Everything else
/// is absolute and unabbreviated: a directory a person is being asked to trust
/// an agent with is not a place to be clever.
#[must_use]
pub fn display_for_person(root: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Ok(rest) = root.strip_prefix(home)
    {
        if rest.as_os_str().is_empty() {
            return "~".to_string();
        }
        return format!("~/{}", rest.display());
    }
    root.display().to_string()
}

/// [`display_for_person`], shortened from the front to fit a header.
///
/// `OMEGA-DELTA-0116`, omega#111. The header has a line's width and a working
/// directory can be much longer than that, so something has to give — and
/// *which end* gives is the whole point. The two directories the owner
/// confused were a checkout and a build worktree; the build worktree's head is
/// `/private/tmp/claude-501/-Users-…`, which says nothing, and its tail is the
/// only part that identifies it. An end-truncated label would therefore have
/// been most misleading in exactly the case that produced this delta.
///
/// So the front goes and the tail stays, marked with a leading `…` so nobody
/// reads the result as an absolute path. The whole path is still available
/// unabbreviated wherever this is used; this is the glance, not the record.
#[must_use]
pub fn short_display_for_person(root: &Path, home: Option<&Path>, max_components: usize) -> String {
    let written = display_for_person(root, home);
    let parts: Vec<&str> = written.split('/').collect();
    if max_components == 0 || parts.len() <= max_components {
        return written;
    }
    format!("…/{}", parts[parts.len() - max_components..].join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_a_person_is_in_is_a_project_root() {
        let directory = tempfile::tempdir().expect("a temporary directory");

        assert_eq!(
            plausible_project_root(directory.path(), None),
            Ok(directory.path().to_path_buf()),
            "an ordinary directory is the case the owner is most likely in, and \
             it must not need a `.git` or a manifest to qualify"
        );
    }

    /// What Finder and the Dock hand over.
    #[test]
    fn the_filesystem_root_is_not_a_project() {
        assert_eq!(
            plausible_project_root(Path::new("/"), None),
            Err(NotAProjectRoot::FilesystemRoot)
        );
    }

    #[test]
    fn the_home_directory_itself_is_not_a_project() {
        let home = tempfile::tempdir().expect("a temporary directory");

        assert_eq!(
            plausible_project_root(home.path(), Some(home.path())),
            Err(NotAProjectRoot::HomeDirectory),
            "opening a home directory as a project scans everything a person \
             owns to answer one question"
        );
    }

    /// The rule is about the home directory, not about being under it. Almost
    /// every real checkout is under `$HOME`.
    #[test]
    fn a_directory_under_home_is_still_a_project() {
        let home = tempfile::tempdir().expect("a temporary directory");
        let project = home.path().join("code");
        std::fs::create_dir(&project).expect("the fixture directory is created");

        assert_eq!(
            plausible_project_root(&project, Some(home.path())),
            Ok(project)
        );
    }

    #[test]
    fn a_launcher_directory_is_not_a_project() {
        for launcher in ["/Applications", "/System/Library", "/usr/local/bin"] {
            assert_eq!(
                plausible_project_root(Path::new(launcher), None),
                Err(NotAProjectRoot::LauncherDirectory),
                "{launcher} is somewhere Omega is started from, not somewhere a \
                 person chose to be"
            );
        }
    }

    #[test]
    fn a_relative_path_names_nothing_on_its_own() {
        assert_eq!(
            plausible_project_root(Path::new("code"), None),
            Err(NotAProjectRoot::NotAbsolute)
        );
    }

    /// omega#111. The owner's command, minus the mode change.
    #[test]
    fn a_directory_argument_is_the_project_it_names() {
        let elsewhere = tempfile::tempdir().expect("a temporary directory");
        let named = tempfile::tempdir().expect("a temporary directory");

        assert_eq!(
            project_root_named(named.path(), elsewhere.path(), None),
            Ok(named.path().to_path_buf()),
            "`omega <directory>` must open the directory it names, not the one \
             the launcher happened to be in"
        );
    }

    #[test]
    fn a_relative_argument_is_resolved_against_the_working_directory() {
        let cwd = tempfile::tempdir().expect("a temporary directory");
        let project = cwd.path().join("code");
        std::fs::create_dir(&project).expect("the fixture directory is created");

        assert_eq!(
            project_root_named(Path::new("code"), cwd.path(), None),
            Ok(project),
            "`omega code` is what a person in a shell types, and the rule about \
             relative paths is about a working directory, not an argument"
        );
        assert_eq!(
            project_root_named(Path::new("."), cwd.path(), None),
            Ok(cwd.path().to_path_buf()),
            "`omega .` must open the directory a person is standing in, spelled \
             the way they would recognise it"
        );
    }

    /// Zero base draws no buffer, so the useful reading of a file argument is
    /// the directory that holds it.
    #[test]
    fn a_file_argument_names_the_directory_that_holds_it() {
        let project = tempfile::tempdir().expect("a temporary directory");
        let file = project.path().join("main.rs");
        std::fs::write(&file, "fn main() {}").expect("the fixture file is written");

        assert_eq!(
            project_root_named(&file, Path::new("/"), None),
            Ok(project.path().to_path_buf()),
            "a single-file worktree gives the thread's `grep` and `terminal` \
             one file to work on, which is barely better than none"
        );
    }

    #[test]
    fn an_argument_that_names_nothing_is_refused_rather_than_climbed() {
        let cwd = tempfile::tempdir().expect("a temporary directory");

        assert_eq!(
            project_root_named(Path::new("typo.rs"), cwd.path(), None),
            Err(NotAProjectRoot::NotADirectory),
            "climbing to the parent of a typo opens a directory nobody named, \
             which is the whole failure this module exists to prevent"
        );
    }

    /// The argument gets no exemption from the rules a working directory obeys.
    #[test]
    fn an_argument_is_still_refused_when_it_is_not_a_project() {
        let home = tempfile::tempdir().expect("a temporary directory");

        assert_eq!(
            project_root_named(Path::new("/"), Path::new("/"), None),
            Err(NotAProjectRoot::FilesystemRoot)
        );
        assert_eq!(
            project_root_named(home.path(), Path::new("/"), Some(home.path())),
            Err(NotAProjectRoot::HomeDirectory)
        );
        assert_eq!(
            project_root_named(Path::new("/Applications"), Path::new("/"), None),
            Err(NotAProjectRoot::LauncherDirectory)
        );
    }

    #[test]
    fn a_working_directory_is_written_the_way_a_person_wrote_it() {
        assert_eq!(
            display_for_person(Path::new("/Users/someone/work/omega"), None),
            "/Users/someone/work/omega"
        );
        assert_eq!(
            display_for_person(
                Path::new("/Users/someone/work/omega"),
                Some(Path::new("/Users/someone"))
            ),
            "~/work/omega",
            "the distinguishing part of a checkout path is its tail, and a home \
             prefix spends the width that tail needs"
        );
        assert_eq!(
            display_for_person(
                Path::new("/Users/someone"),
                Some(Path::new("/Users/someone"))
            ),
            "~"
        );
        assert_eq!(
            display_for_person(Path::new("/opt/src"), Some(Path::new("/Users/someone"))),
            "/opt/src",
            "a directory outside home is named in full; there is nothing to \
             abbreviate and guessing would be worse than not"
        );
    }

    /// The two directories the owner actually confused.
    #[test]
    fn shortening_keeps_the_end_that_identifies_a_directory() {
        let home = Path::new("/Users/someone");

        assert_eq!(
            short_display_for_person(Path::new("/Users/someone/work/omega"), Some(home), 3),
            "~/work/omega",
            "a path that already fits is not abbreviated"
        );
        assert_eq!(
            short_display_for_person(
                Path::new("/private/tmp/claude-501/-Users-someone-work/scratchpad/wt-99baselines"),
                Some(home),
                3,
            ),
            "…/-Users-someone-work/scratchpad/wt-99baselines",
            "the head of a build worktree says nothing and its tail says \
             everything, so end-truncation would be most misleading in exactly \
             the case that produced this delta"
        );
        // The budget counts components from the end, so the two directories the
        // owner confused stay distinguishable at a glance whichever of them the
        // window is showing.
        assert_ne!(
            short_display_for_person(
                Path::new("/private/tmp/claude-501/-Users-someone-work/scratchpad/wt-99baselines"),
                Some(home),
                3,
            ),
            short_display_for_person(Path::new("/Users/someone/work/omega"), Some(home), 3),
        );
        assert_eq!(
            short_display_for_person(Path::new("/Users/someone/work/omega"), Some(home), 0),
            "~/work/omega",
            "a zero budget is not a request for an empty label"
        );
    }

    #[test]
    fn a_path_that_is_not_a_directory_is_not_a_project() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let file = directory.path().join("main.rs");
        std::fs::write(&file, "fn main() {}").expect("the fixture file is written");

        assert_eq!(
            plausible_project_root(&file, None),
            Err(NotAProjectRoot::NotADirectory)
        );
        assert_eq!(
            plausible_project_root(&directory.path().join("absent"), None),
            Err(NotAProjectRoot::NotADirectory)
        );
    }
}
