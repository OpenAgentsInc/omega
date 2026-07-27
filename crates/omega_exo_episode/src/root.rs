//! One writer per `.exo` root. `OMEGA-DELTA-0090`, omega#103.
//!
//! Exo's storage is a JSON file per record on an object store, and its
//! concurrency control is **one in-process async mutex**. The teardown is
//! explicit and so is Exo's own code: multiple processes on one root are not
//! the design. Two `exo serve` processes on one root do not conflict loudly —
//! they interleave. The head compare-and-set protects a turn writer and nothing
//! else, so a fork replayed by one process while another appends to the source
//! conversation produces a fork of a history that never existed, and nothing
//! anywhere reports an error.
//!
//! That is the worst possible failure for this crate specifically. An episode's
//! whole claim is "these two forks started from the same place". A second
//! writer makes that claim false while every check still passes.
//!
//! # Measured, 2026-07-27
//!
//! `OMEGA-DELTA-0120`. omega#103's third falsifier reads "point two processes
//! at one `.exo` root: this must be refused". It is not. Two `exo serve`
//! processes were started on one root at the pin, on two ports, on a throwaway
//! copy. Both came up. Both answered `GET /health` with `ok`. Neither said
//! anything about the other, and no lock file appeared.
//!
//! So the refusal below is Omega's, kept by Omega, for Omega. Nothing in the
//! substrate enforces it, and a second writer started by somebody else — a
//! person at a terminal, another lane, a stale daemon — is invisible from
//! here. That is the residual, stated rather than assumed away, and it is why
//! `script/exo-episode-live` insists on a copy of a root rather than a root.
//!
//! # What this can and cannot enforce
//!
//! [`ExoRoots`] is a registry of the roots one Omega process has claimed. It
//! refuses the second claim on a root. That is real for the case that actually
//! happens here — a lane that starts a second Exo against the root the first
//! one is already using — and it is not, and cannot be, a lock against a
//! process Omega did not start. This crate does no IO: no lock file, no
//! `flock`, no directory read. A claim is a claim about intent inside one
//! process.
//!
//! So the boundary is drawn where the crate can actually hold it, and the doc
//! says where it stops rather than implying more.
//!
//! # Claims are by spelling, and the spelling must be unambiguous
//!
//! Two different strings can name one directory: a relative path, a `..`, a
//! symlink. Resolving those needs the filesystem, which this crate does not
//! touch. Rather than compare loosely and hope, [`ExoRoots::claim`] refuses
//! anything but an absolute path with no `.` or `..` component — a spelling
//! that cannot be a second name for a root already claimed under a different
//! spelling, except through a symlink, which is stated as the residual and not
//! papered over.

use std::collections::BTreeSet;

/// Why a root could not be claimed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootRefusal {
    /// Another holder in this process has this root.
    ///
    /// Carries the root, because the useful thing to say is which one.
    AlreadyClaimed(String),
    /// The path was empty.
    Empty,
    /// The path is relative, so it names a different directory depending on
    /// where the caller happened to be.
    NotAbsolute,
    /// The path carries a `.` or `..` component, so two spellings of it could
    /// name one directory and the registry would not notice.
    Ambiguous,
}

impl std::fmt::Display for RootRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyClaimed(root) => write!(
                formatter,
                "{root} is already open in this process, and an Exo root is single-writer storage"
            ),
            Self::Empty => formatter.write_str("an Exo root path cannot be empty"),
            Self::NotAbsolute => {
                formatter.write_str("an Exo root must be named by an absolute path")
            }
            Self::Ambiguous => formatter.write_str(
                "an Exo root path must carry no `.` or `..` component, so that one root has one \
                 spelling",
            ),
        }
    }
}

impl std::error::Error for RootRefusal {}

/// A claim on one Exo root, held for as long as the episode uses it.
///
/// There is no way to build one except through [`ExoRoots::claim`], and it is
/// deliberately not [`Clone`]: a clone would be a second holder of a
/// single-writer resource, which is the exact thing the type exists to prevent.
#[derive(Debug, PartialEq, Eq)]
pub struct RootClaim {
    root: String,
}

impl RootClaim {
    /// The root this claim covers.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.root
    }
}

/// The Exo roots this process has open.
///
/// Deliberately not [`Clone`]: a cloned registry would hand out a second claim
/// on a root the original still holds, which is the exact thing it exists to
/// refuse.
#[derive(Debug, Default)]
pub struct ExoRoots {
    held: BTreeSet<String>,
}

impl ExoRoots {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim a root for one holder.
    ///
    /// # Errors
    ///
    /// [`RootRefusal`] when the path is not an unambiguous absolute path, or
    /// when this process already holds it.
    pub fn claim(&mut self, root: &str) -> Result<RootClaim, RootRefusal> {
        let root = normalize(root)?;
        if !self.held.insert(root.clone()) {
            return Err(RootRefusal::AlreadyClaimed(root));
        }
        Ok(RootClaim { root })
    }

    /// Give a root back.
    ///
    /// Takes the claim by value, so a released root cannot still be held.
    pub fn release(&mut self, claim: RootClaim) {
        self.held.remove(&claim.root);
    }

    /// Whether this process holds a root. For tests and diagnostics.
    #[must_use]
    pub fn holds(&self, root: &str) -> bool {
        normalize(root).is_ok_and(|root| self.held.contains(&root))
    }
}

/// Reduce a root path to the one spelling the registry compares, or refuse.
fn normalize(root: &str) -> Result<String, RootRefusal> {
    let root = root.trim();
    if root.is_empty() {
        return Err(RootRefusal::Empty);
    }
    if !root.starts_with('/') {
        return Err(RootRefusal::NotAbsolute);
    }
    if root
        .split('/')
        .any(|component| matches!(component, "." | ".."))
    {
        return Err(RootRefusal::Ambiguous);
    }
    // Collapse repeated separators and drop a trailing one, so `/a//b/` and
    // `/a/b` are one claim rather than two.
    let mut normalized = String::with_capacity(root.len());
    for component in root.split('/').filter(|component| !component.is_empty()) {
        normalized.push('/');
        normalized.push_str(component);
    }
    if normalized.is_empty() {
        normalized.push('/');
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_root_can_only_be_claimed_once() {
        let mut roots = ExoRoots::new();
        let first = roots.claim("/Users/someone/.exo").expect("the first claim");
        assert_eq!(
            roots.claim("/Users/someone/.exo"),
            Err(RootRefusal::AlreadyClaimed(
                "/Users/someone/.exo".to_owned()
            )),
            "two processes on one root is the failure that produces a fork of a history \
             that never existed"
        );
        roots.release(first);
        roots
            .claim("/Users/someone/.exo")
            .expect("the root is free again");
    }

    #[test]
    fn one_root_spelled_two_ways_is_still_one_claim() {
        let mut roots = ExoRoots::new();
        let _first = roots.claim("/Users/someone/.exo").expect("the first claim");
        for alias in [
            "/Users/someone/.exo/",
            "//Users//someone//.exo",
            "  /Users/someone/.exo  ",
        ] {
            assert!(
                matches!(roots.claim(alias), Err(RootRefusal::AlreadyClaimed(_))),
                "{alias} names the root already claimed, and the registry read it as new"
            );
        }
    }

    #[test]
    fn a_spelling_that_could_alias_is_refused_rather_than_compared_loosely() {
        let mut roots = ExoRoots::new();
        assert_eq!(roots.claim(".exo"), Err(RootRefusal::NotAbsolute));
        assert_eq!(roots.claim(""), Err(RootRefusal::Empty));
        assert_eq!(
            roots.claim("/Users/someone/../someone/.exo"),
            Err(RootRefusal::Ambiguous),
            "resolving `..` needs the filesystem, and this crate does not touch it"
        );
        assert_eq!(
            roots.claim("/Users/./someone/.exo"),
            Err(RootRefusal::Ambiguous)
        );
    }

    #[test]
    fn different_roots_do_not_collide() {
        let mut roots = ExoRoots::new();
        let _first = roots.claim("/Users/someone/.exo").expect("first");
        let _second = roots.claim("/Users/someone/.exo-episode").expect("second");
        assert!(roots.holds("/Users/someone/.exo"));
        assert!(roots.holds("/Users/someone/.exo-episode"));
        assert!(!roots.holds("/Users/someone/.exo-other"));
    }
}
