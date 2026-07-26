//! Which directory a thread should be able to see.
//!
//! `OMEGA-DELTA-0054`, omega#100. Zero base opened no project. The comment in
//! `crates/zed/src/zed.rs` said so plainly — "no project is opened, so there is
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
