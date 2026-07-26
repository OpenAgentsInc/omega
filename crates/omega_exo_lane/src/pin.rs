//! Which Exo. `OMEGA-DELTA-0042`, omega#87.
//!
//! Exo declares itself unstable and writes the house rule "do not write
//! fallback code or handle backwards compatibility" into its own `AGENTS.md`.
//! `0.1.0`, no tags, nothing published. A lane that said "whatever `exo` is on
//! `PATH`" would be a lane whose behaviour is decided by somebody else's next
//! push.
//!
//! So the lane names an exact commit, and it names the tree that commit points
//! at. Both, for the same reason `omega-effectd` is pinned by release tag *and*
//! asset digest in `script/prove-omega-rc-install`: a commit id alone is
//! satisfied by a force-pushed branch that still answers to the same forty
//! characters in a clone nobody re-fetched, and a tree alone produces a refusal
//! nobody can act on.
//!
//! # What this file does and does not pin
//!
//! [`EXO_PIN`] is an **identity**: upstream, commit, tree, version. It is the
//! same on every machine, it is checked in, and it is what a reader compares a
//! clone against.
//!
//! The **bytes that actually run** are a different fact, because Exo has no
//! release artifact — the install path is `curl setup.sh | bash`, which clones
//! and builds from source, so two hosts at the same commit hold two different
//! binaries. Those bytes go through the mechanism that already exists for
//! exactly this: [`omega_harness::HarnessPinLedger`], under the harness id
//! [`EXO_HARNESS_ID`], holding a [`MeasuredDigest`] the host computed from the
//! binary it is about to run.
//!
//! [`MeasuredDigest`]: omega_harness::MeasuredDigest
//!
//! That split is the whole point. The identity is a claim anyone can check
//! against upstream; the digest is a measurement only this host can make. They
//! must not share a type and they must not share a file.

use omega_harness::MeasuredDigest;

/// The harness id the Exo lane is frozen under in the pin ledger.
///
/// The same namespace `codex-acp` uses, because Exo is the same kind of thing:
/// an external harness Omega runs and does not own.
pub const EXO_HARNESS_ID: &str = "exo";

/// The Exo this lane drives.
///
/// **The maintained `OpenAgentsInc/exo` fork of `exoharness/exo`**, the
/// recursive-self-improvement agent harness. This is not exo labs'
/// `exo-explore/exo` cluster-inference appliance. The fork contains the ACP
/// transport that Omega needs while the upstream change is under review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExoPin {
    /// The repository, in full. See the type documentation for why.
    pub upstream: &'static str,
    /// The exact commit the lane was audited and driven against.
    pub source_commit: &'static str,
    /// The tree that commit points at.
    pub source_tree: &'static str,
    /// The workspace version at that commit, for a message a person can read.
    pub version: &'static str,
}

/// The pin. Audited in the openagents teardown
/// `docs/teardowns/2026-07-25-exoharness-exo-teardown.md` and driven for real
/// by omega#87.
pub const EXO_PIN: ExoPin = ExoPin {
    upstream: "https://github.com/OpenAgentsInc/exo",
    source_commit: "cd7c0d29db869e953fb7261d8390ca93007d36a6",
    source_tree: "c61846e3f44daaf445930d1a499432ca9b069306",
    version: "0.1.0",
};

/// Why a local Exo checkout is not the pinned one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExoPinMismatch {
    /// The clone is a different repository. This is the omega#86 failure: the
    /// other Exo.
    Upstream,
    /// The checkout is at a different commit.
    Commit,
    /// The checkout is at the pinned commit and the tree underneath it is not
    /// the pinned tree — a rewritten history, a dirty tree, or a substituted
    /// object store.
    Tree,
    /// The bytes about to run are not the bytes the owner froze.
    Bytes,
}

impl std::fmt::Display for ExoPinMismatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Upstream => "the Exo checkout is not the maintained exoharness/exo fork",
            Self::Commit => "the Exo checkout is not at the pinned commit",
            Self::Tree => "the Exo checkout's tree is not the pinned tree",
            Self::Bytes => "the Exo binary is not the bytes frozen in the pin ledger",
        })
    }
}

impl std::error::Error for ExoPinMismatch {}

/// What a host observed about a local Exo checkout.
///
/// Every field is something the host read: `git config --get remote.origin.url`,
/// `git rev-parse HEAD`, `git rev-parse HEAD^{tree}`. A record of observations,
/// which is why it is a separate type from [`ExoPin`], which is a record of
/// decisions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedExoCheckout {
    /// The remote the checkout came from.
    pub upstream: String,
    /// `HEAD`.
    pub commit: String,
    /// `HEAD^{tree}`.
    pub tree: String,
}

impl ExoPin {
    /// Whether an observed checkout is the Exo this lane admits.
    ///
    /// Upstream is compared after normalising a trailing `.git` and a trailing
    /// slash, because `https://github.com/exoharness/exo.git` and
    /// `https://github.com/exoharness/exo` are the same repository and refusing
    /// one of them would be a cosmetic refusal. `git@github.com:exoharness/exo`
    /// is *not* normalised into the same string; it is compared by its
    /// `owner/name` suffix, which is the part that carries the identity.
    ///
    /// # Errors
    ///
    /// Returns the first thing that differed, most-identifying first: a
    /// checkout of the wrong repository is reported as the wrong repository
    /// even though its commit also differs.
    pub fn admits(&self, observed: &ObservedExoCheckout) -> Result<(), ExoPinMismatch> {
        if !same_repository(self.upstream, &observed.upstream) {
            return Err(ExoPinMismatch::Upstream);
        }
        if !self
            .source_commit
            .eq_ignore_ascii_case(observed.commit.trim())
        {
            return Err(ExoPinMismatch::Commit);
        }
        if !self.source_tree.eq_ignore_ascii_case(observed.tree.trim()) {
            return Err(ExoPinMismatch::Tree);
        }
        Ok(())
    }
}

/// Whether the bytes this host measured are the bytes the owner froze.
///
/// Takes the frozen digest as a `&str` and the live one as a [`MeasuredDigest`]
/// for the reason that type exists: a pin read back from a file is a recorded
/// claim about a past measurement, and this process did not make it.
///
/// An **unfrozen** Exo is admitted, and that is deliberate rather than an
/// oversight. `codex-acp` behaves the same way: a pin is the owner freezing a
/// version, not a precondition for running anything at all. What is refused is
/// a *disagreement* — the case where the owner said "these bytes" and the host
/// is holding different ones.
///
/// # Errors
///
/// [`ExoPinMismatch::Bytes`] when a pin exists and the measurement differs.
pub fn admits_bytes(frozen: Option<&str>, measured: &MeasuredDigest) -> Result<(), ExoPinMismatch> {
    match frozen {
        Some(frozen) if !measured.matches_recorded(frozen) => Err(ExoPinMismatch::Bytes),
        Some(_) | None => Ok(()),
    }
}

/// Whether two remote strings name the same repository.
fn same_repository(pinned: &str, observed: &str) -> bool {
    normalize_remote(pinned) == normalize_remote(observed)
}

/// `owner/name`, lowercased, with the transport, host, trailing slash and
/// `.git` suffix removed.
fn normalize_remote(remote: &str) -> String {
    let remote = remote.trim().trim_end_matches('/');
    let remote = remote.strip_suffix(".git").unwrap_or(remote);
    let tail = remote.rsplit(['/', ':']).take(2).collect::<Vec<_>>();
    tail.into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("/")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pinned_checkout() -> ObservedExoCheckout {
        ObservedExoCheckout {
            upstream: EXO_PIN.upstream.to_string(),
            commit: EXO_PIN.source_commit.to_string(),
            tree: EXO_PIN.source_tree.to_string(),
        }
    }

    /// The commit and tree are forty hex characters each. A pin that had been
    /// filled in with a branch name, a tag, or a truncated id would name
    /// something that can move, which is the whole thing the pin exists to
    /// stop.
    #[test]
    fn the_pin_names_an_exact_commit_and_an_exact_tree() {
        for value in [EXO_PIN.source_commit, EXO_PIN.source_tree] {
            assert_eq!(value.len(), 40, "{value} is not a full object id");
            assert!(
                value
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "{value} is not a lowercase hexadecimal object id"
            );
        }
        assert_ne!(
            EXO_PIN.source_commit, EXO_PIN.source_tree,
            "a commit and the tree it points at are different objects"
        );
    }

    /// omega#86's failure, as a test. The two Exos share a name and nothing
    /// else, and the one this lane drives is the harness.
    #[test]
    fn the_pin_names_the_harness_exo_and_not_the_cluster_one() {
        assert_eq!(EXO_PIN.upstream, "https://github.com/OpenAgentsInc/exo");
        let cluster = ObservedExoCheckout {
            upstream: "https://github.com/exo-explore/exo".into(),
            ..pinned_checkout()
        };
        assert_eq!(EXO_PIN.admits(&cluster), Err(ExoPinMismatch::Upstream));
    }

    #[test]
    fn the_pinned_checkout_is_admitted() {
        assert_eq!(EXO_PIN.admits(&pinned_checkout()), Ok(()));
    }

    /// A remote that differs only in `.git`, a trailing slash, case, or SSH
    /// versus HTTPS transport is the same repository, and refusing it would be
    /// a refusal about punctuation.
    #[test]
    fn the_same_repository_written_four_ways_is_the_same_repository() {
        for spelling in [
            "https://github.com/OpenAgentsInc/exo.git",
            "https://github.com/OpenAgentsInc/exo/",
            "https://GitHub.com/OpenAgentsInc/Exo",
            "git@github.com:OpenAgentsInc/exo.git",
        ] {
            let observed = ObservedExoCheckout {
                upstream: spelling.into(),
                ..pinned_checkout()
            };
            assert_eq!(EXO_PIN.admits(&observed), Ok(()), "{spelling}");
        }
    }

    /// A moved branch that keeps the commit id it answers to is exactly the
    /// substitution a commit-only pin cannot see, so the tree is checked too.
    #[test]
    fn the_pinned_commit_over_a_different_tree_is_refused() {
        let rewritten = ObservedExoCheckout {
            tree: "0000000000000000000000000000000000000000".into(),
            ..pinned_checkout()
        };
        assert_eq!(EXO_PIN.admits(&rewritten), Err(ExoPinMismatch::Tree));
    }

    #[test]
    fn a_different_commit_is_refused() {
        let moved = ObservedExoCheckout {
            commit: "1111111111111111111111111111111111111111".into(),
            ..pinned_checkout()
        };
        assert_eq!(EXO_PIN.admits(&moved), Err(ExoPinMismatch::Commit));
    }

    /// The bytes half. An unfrozen harness runs; a frozen one that changed
    /// underneath the owner does not.
    #[test]
    fn frozen_bytes_that_changed_are_refused_and_unfrozen_bytes_run() {
        let measured = MeasuredDigest::measure(b"the exo binary this host built");
        assert_eq!(admits_bytes(None, &measured), Ok(()));
        assert_eq!(admits_bytes(Some(measured.as_str()), &measured), Ok(()));
        let other = MeasuredDigest::measure(b"a different exo binary");
        assert_eq!(
            admits_bytes(Some(other.as_str()), &measured),
            Err(ExoPinMismatch::Bytes)
        );
    }
}
