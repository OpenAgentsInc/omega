//! The reset, its two halves, and the half that does not compose yet.
//! `OMEGA-DELTA-0090`, omega#103.
//!
//! omega#103 and the teardown both state that `conversation_fork` plus
//! `start_sandbox { snapshot_id }` is "a complete episode-reset mechanism" that
//! "needs no upstream changes at all". **The first half is true. The second is
//! not, at [`crate::EXO_PROTOCOL_PIN`].** This module is where that is written
//! down as a type rather than as a footnote, because the failure mode of
//! believing it is an episode that reports a reset it did not perform.
//!
//! # What fork actually copies
//!
//! `BasicConversationHandle::fork` (`crates/exoharness/src/basic.rs:2225`)
//! copies exactly four prefixes out of the source conversation's directory:
//! [`FORK_COPIES_PREFIXES`]. Snapshots are not among them. Exo's own
//! `docs/sandbox-snapshots.md` states the layout:
//!
//! ```text
//! agents/<agent_id>/conversations/<conversation_id>/snapshots/<snapshot_id>/
//! ├── manifest.json
//! └── payload.bin
//! ```
//!
//! `snapshots` is a sibling of `sandboxes`, and only `sandboxes` is copied. So a
//! fork inherits the sandbox *records* — including each record's
//! `latest_snapshot_id` — and inherits none of the payloads those ids name. The
//! fork therefore holds a dangling reference, and `start_sandbox` against it
//! fails in `start_sandbox_side_effect` while loading the manifest, with Exo's
//! own message: *"loading snapshot manifest for `<id>` (have you taken a
//! snapshot?)"*.
//!
//! Exo's documentation says the same thing in product terms, in its own words:
//! snapshots time-travel a sandbox "**without forking the conversation
//! itself**", and are "**not a conversation rewind** — use `conversation fork`
//! to rewind the conversation itself." They are presented as two alternatives.
//! Nothing upstream claims they compose.
//!
//! # The three scopes, and why each fails or works
//!
//! A sandbox is owned by an agent, a conversation, or a turn, and the owner's
//! directory is where its snapshots live.
//!
//! * **Conversation** and **turn** scope: the snapshot lives under the source
//!   conversation and the fork does not get it. [`SnapshotReach::LostByFork`].
//! * **Agent** scope: the snapshot lives under the agent directory, which every
//!   conversation of that agent — forks included — can reach.
//!   [`SnapshotReach::ReachableFromFork`].
//!
//! Agent scope reaches, and it does not isolate. There is one sandbox record
//! per agent-scoped id, so two forks restoring it are restoring the *same*
//! sandbox: one warm container, one `latest_snapshot_id`, whichever wrote last.
//! Two siblings that share a filesystem are not two episodes.
//!
//! # And a fork with no filesystem state at all
//!
//! `OMEGA-DELTA-0107`, omega#103. Even in the one combination that reaches its
//! snapshot, there has to *be* one. A sandbox that was never snapshotted is the
//! ordinary case for a fresh agent, and `start_sandbox` against it fails in
//! `start_sandbox_side_effect` with the same sentence a fork's dangling
//! reference produces: *"loading snapshot manifest for `<id>` (have you taken a
//! snapshot?)"*.
//!
//! Those are two different situations that Exo reports identically, and the
//! confusing one is the second: the operator reads a snapshot-manifest failure
//! and goes looking for the fork bug that is not there. So
//! [`admit_filesystem_reset`] takes [`SnapshotEvidence`] — whether anything
//! Omega actually read names a snapshot for this sandbox — and refuses without
//! it by name ([`ResetRefusal::NoSnapshotObserved`]), before a request is built.
//! The evidence is a value the caller supplies from the durable record, because
//! this crate has no filesystem and reads nothing itself.
//!
//! So [`admit_filesystem_reset`] is total over (scope, shape, evidence) and
//! issues its witness for exactly one combination — agent scope, one episode at
//! a time, with a snapshot Omega has seen — and names the reason for each
//! refusal. The table is the honest summary of what this primitive can do
//! today:
//!
//! | | one episode | two siblings |
//! | --- | --- | --- |
//! | agent scope, snapshot observed | admitted | refused: they share one sandbox |
//! | agent scope, no snapshot observed | refused: nothing to restore | refused: they share one sandbox |
//! | conversation scope | refused: snapshot lost by fork | refused: snapshot lost by fork |
//! | turn scope | refused: snapshot lost by fork | refused: snapshot lost by fork |
//!
//! # This is not fatal to the issue
//!
//! The conversation half of the reset — memory, tools, and the whole event log,
//! which is what a check runs against — forks completely and correctly today.
//! The filesystem half needs one upstream change: `fork` copying the
//! `snapshots` prefix as it already copies `sandboxes`. That is four lines
//! beside four identical ones, additive, and in upstream's own direction. It is
//! owner-gated like every other upstream contribution and it is not made here.
//!
//! # No step of any of this touches the working tree
//!
//! The falsification loop this replaces destroyed uncommitted files once, with
//! `git checkout --`. So this crate has no filesystem and no process: no
//! `std::fs`, no `std::process`, no `std::path`. Nothing here can revert
//! anything, because nothing here can reach anything.
//! `the_episode_crate_cannot_reach_the_working_tree` in `crates/omega_deltas`
//! reads the source and fails if that ever stops being true.

use crate::request::ForkedConversation;

/// The prefixes `fork` copies from the source conversation, at the pin.
///
/// Transcribed from the four `copy_prefix` calls in
/// `BasicConversationHandle::fork`, in source order.
pub const FORK_COPIES_PREFIXES: &[&str] = &["bindings", "secrets", "artifacts", "sandboxes"];

/// The prefix `fork` does not copy, which is the whole finding.
pub const FORK_DOES_NOT_COPY_PREFIXES: &[&str] = &["snapshots"];

/// Who owns a sandbox, which is also where its snapshots live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxScopeKind {
    /// Owned by the agent. Snapshots live under the agent directory.
    Agent,
    /// Owned by the conversation. Snapshots live under the conversation
    /// directory, which a fork does not copy.
    Conversation,
    /// Owned by one turn. Same directory as the conversation for this purpose.
    Turn,
}

impl SandboxScopeKind {
    /// The tag Exo's `SandboxScope` serializes under.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Conversation => "conversation",
            Self::Turn => "turn",
        }
    }
}

/// How many episodes run from one fork point at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpisodeShape {
    /// One fork, restored and run on its own.
    SingleEpisode,
    /// Two or more forks from one event, compared against each other.
    Siblings,
}

/// Whether a snapshot taken before a fork can be reached from inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotReach {
    /// The snapshot is under the agent directory, which the fork shares.
    ReachableFromFork,
    /// The snapshot is under the source conversation, which the fork copies
    /// four prefixes of, and `snapshots` is not one of them.
    LostByFork,
}

/// Whether a snapshot taken before a fork survives into it.
#[must_use]
pub const fn snapshot_reach(scope: SandboxScopeKind) -> SnapshotReach {
    match scope {
        SandboxScopeKind::Agent => SnapshotReach::ReachableFromFork,
        SandboxScopeKind::Conversation | SandboxScopeKind::Turn => SnapshotReach::LostByFork,
    }
}

/// Whether Omega has actually seen a snapshot to restore from.
///
/// `OMEGA-DELTA-0107`, omega#103. A value the caller supplies, read off the
/// durable record — a `sandbox_snapshotted` event, or a sandbox record whose
/// `latest_snapshot_id` is set. This crate has no filesystem and no socket, so
/// it cannot look; what it can do is refuse to build a request that has nothing
/// to restore.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotEvidence {
    /// Omega read something that names a snapshot for this sandbox.
    Observed,
    /// Nothing Omega read names one. The sandbox may never have been
    /// snapshotted, which is the ordinary state of a fresh agent.
    NoneObserved,
}

/// Why a filesystem reset is not available for this shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetRefusal {
    /// The snapshot is under the source conversation and the fork did not copy
    /// it. `start_sandbox` would fail loading the manifest.
    SnapshotLostByFork,
    /// The snapshot is reachable, and the sandbox behind it is one record
    /// shared by every conversation of the agent. Two siblings restoring it
    /// are one sandbox, not two episodes.
    SiblingsShareOneSandbox,
    /// Nothing Omega read names a snapshot for this sandbox, so there is no
    /// filesystem state to reset to.
    ///
    /// `OMEGA-DELTA-0107`. Exo reports this and [`Self::SnapshotLostByFork`]
    /// with the same sentence, which sends a reader looking for a fork bug that
    /// is not there. Refusing here separates them before a request exists.
    NoSnapshotObserved,
}

impl std::fmt::Display for ResetRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::SnapshotLostByFork => {
                "a fork copies bindings, secrets, artifacts and sandbox records, and not \
                 snapshots, so this snapshot does not exist inside the fork"
            }
            Self::SiblingsShareOneSandbox => {
                "an agent-scoped sandbox is one record for the whole agent, so two forks \
                 restoring it share a filesystem and are not two episodes"
            }
            Self::NoSnapshotObserved => {
                "nothing Omega read names a snapshot for this sandbox, so there is no \
                 filesystem state to restore — this fork has no environment to reset to, \
                 which is not the same failure as a snapshot a fork left behind, though \
                 Exo reports both by asking whether a snapshot was ever taken"
            }
        })
    }
}

impl std::error::Error for ResetRefusal {}

/// Proof that a filesystem reset of this shape is one that can work.
///
/// Issued only by [`admit_filesystem_reset`], and required by
/// [`crate::EpisodeRequest::RestoreSandbox`]. The field is private and there is
/// no other constructor, so a `start_sandbox` request cannot be built for a
/// shape that would fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilesystemReset {
    scope: SandboxScopeKind,
}

impl FilesystemReset {
    /// The scope this reset was admitted for.
    #[must_use]
    pub const fn scope(self) -> SandboxScopeKind {
        self.scope
    }

    /// Exo's `SandboxScope` for this reset, addressed at the fork.
    ///
    /// Only the agent variant is reachable today, because that is the only
    /// scope [`admit_filesystem_reset`] issues a witness for. The other two
    /// arms are written so the function stays total if the upstream `fork`
    /// gains the `snapshots` prefix and the table opens up.
    #[must_use]
    pub fn scope_json(self, fork: &ForkedConversation) -> serde_json::Value {
        match self.scope {
            SandboxScopeKind::Agent => serde_json::json!({
                "type": "agent",
                "agent_id": fork.agent().as_str(),
            }),
            SandboxScopeKind::Conversation | SandboxScopeKind::Turn => serde_json::json!({
                "type": "conversation",
                "agent_id": fork.agent().as_str(),
                "conversation_id": fork.conversation().as_str(),
            }),
        }
    }
}

/// Decide whether a filesystem reset of this shape can work.
///
/// Structure first, then evidence: the two shape refusals are properties of the
/// primitive and hold whatever Omega read, so they are decided before the
/// question of whether a snapshot exists is even asked. A caller that fixed its
/// evidence and still gets `SnapshotLostByFork` is being told the right thing.
///
/// # Errors
///
/// [`ResetRefusal`] for every shape but agent scope on a single episode with a
/// snapshot Omega has seen. See the module documentation for the table and the
/// reasons.
pub const fn admit_filesystem_reset(
    scope: SandboxScopeKind,
    shape: EpisodeShape,
    evidence: SnapshotEvidence,
) -> Result<FilesystemReset, ResetRefusal> {
    match (snapshot_reach(scope), shape) {
        (SnapshotReach::LostByFork, _) => Err(ResetRefusal::SnapshotLostByFork),
        (SnapshotReach::ReachableFromFork, EpisodeShape::Siblings) => {
            Err(ResetRefusal::SiblingsShareOneSandbox)
        }
        (SnapshotReach::ReachableFromFork, EpisodeShape::SingleEpisode) => match evidence {
            SnapshotEvidence::NoneObserved => Err(ResetRefusal::NoSnapshotObserved),
            SnapshotEvidence::Observed => Ok(FilesystemReset { scope }),
        },
    }
}

/// One step of the falsification loop, in the order it happens.
///
/// The order is a constant ([`FALSIFICATION_LOOP`]) rather than something a
/// caller assembles, because two of the orderings a caller could assemble are
/// exactly the failures omega#103 lists: mutating before forking, and reading a
/// check outcome before proving the mutation applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// `conversation_fork` at the chosen event, for the fork that will carry
    /// the mutation.
    ForkCandidate,
    /// `conversation_fork` at the same event, for the fork that will not.
    ForkControl,
    /// `conversation_get_events` on the candidate, before anything happens to
    /// it.
    ReadCandidateBaseline,
    /// `conversation_get_events` on the control, before anything happens to it.
    ReadControlBaseline,
    /// The two baselines must be the same state. This is the acceptance
    /// condition "two forks from one event start identical", decided by
    /// comparing rather than asserting.
    CompareStartingStates,
    /// The mutation. A turn sent into the candidate fork by
    /// `omega_exo_lane::ExoCommand::SendTurn` — the lane's one write, which
    /// goes to Exo's CLI and not to this crate's protocol client.
    ApplyMutationInCandidate,
    /// `conversation_get_events` on the candidate again.
    ReadCandidateAfterMutation,
    /// `conversation_get_events` on the control again.
    ReadControlAfterMutation,
    /// The candidate must have moved. This is the probe, and it comes before
    /// the check: an edit that silently did not apply produces a check that
    /// passes while testing nothing, which has happened here repeatedly.
    ProbeMutationApplied,
    /// The control must not have moved. This is "a mutation in one fork is
    /// absent from its sibling", again by comparison.
    CompareSiblingUnmutated,
    /// Run the named check against the mutated episode.
    RunNamedCheck,
    /// Read [`verdict`] from the probe and the check outcome.
    ReadVerdict,
}

/// The falsification loop, in order.
pub const FALSIFICATION_LOOP: &[Step] = &[
    Step::ForkCandidate,
    Step::ForkControl,
    Step::ReadCandidateBaseline,
    Step::ReadControlBaseline,
    Step::CompareStartingStates,
    Step::ApplyMutationInCandidate,
    Step::ReadCandidateAfterMutation,
    Step::ReadControlAfterMutation,
    Step::ProbeMutationApplied,
    Step::CompareSiblingUnmutated,
    Step::RunNamedCheck,
    Step::ReadVerdict,
];

/// Whether the mutation is present in the episode it was applied to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The candidate's state moved. The mutation is really there.
    MutationPresent,
    /// The candidate's state is what it was. Nothing was applied.
    MutationAbsent,
}

/// What the named check said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The check passed.
    Passed,
    /// The check failed.
    Failed,
}

/// What the loop proved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The mutation applied and the check failed. The check is worth trusting
    /// for this mutation.
    Falsified,
    /// The mutation applied and the check passed anyway. The check does not
    /// test what its name says it tests. This is the "green while testing
    /// nothing" outcome — on 2026-07-26 it happened because the mutated string
    /// also appeared in a second code path.
    CheckDidNotNotice,
    /// The mutation never applied, so the check's answer is about nothing. Not
    /// a failure of the check; a failure of the loop, and the reason the probe
    /// is a step rather than an assumption.
    MutationDidNotApply,
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Falsified => "the mutation applied and the check failed",
            Self::CheckDidNotNotice => {
                "the mutation applied and the check passed, so the check tests something else"
            }
            Self::MutationDidNotApply => {
                "the mutation never applied, so the check answered a question nobody asked"
            }
        })
    }
}

/// Read the verdict. Total over both inputs, and the probe wins.
///
/// A passing check under an absent mutation is the shape that reads like
/// success and means nothing, so it is not folded into
/// [`Verdict::CheckDidNotNotice`]: those two are different bugs, in different
/// places, and a loop that could not tell them apart would send somebody to
/// read the wrong file.
#[must_use]
pub const fn verdict(probe: ProbeOutcome, check: CheckOutcome) -> Verdict {
    match (probe, check) {
        (ProbeOutcome::MutationAbsent, _) => Verdict::MutationDidNotApply,
        (ProbeOutcome::MutationPresent, CheckOutcome::Passed) => Verdict::CheckDidNotNotice,
        (ProbeOutcome::MutationPresent, CheckOutcome::Failed) => Verdict::Falsified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conversation_scoped_snapshot_does_not_survive_a_fork() {
        assert_eq!(
            snapshot_reach(SandboxScopeKind::Conversation),
            SnapshotReach::LostByFork
        );
        assert_eq!(
            snapshot_reach(SandboxScopeKind::Turn),
            SnapshotReach::LostByFork
        );
        assert!(
            !FORK_COPIES_PREFIXES.contains(&"snapshots"),
            "if fork ever copies snapshots, this whole module's refusals are stale"
        );
        assert_eq!(FORK_DOES_NOT_COPY_PREFIXES, &["snapshots"]);
        assert_eq!(
            FORK_COPIES_PREFIXES,
            &["bindings", "secrets", "artifacts", "sandboxes"],
            "the four copy_prefix calls in BasicConversationHandle::fork, in source order"
        );
    }

    #[test]
    fn the_admission_table_is_the_one_the_module_documents() {
        use EpisodeShape::{Siblings, SingleEpisode};
        use SandboxScopeKind::{Agent, Conversation, Turn};
        use SnapshotEvidence::{NoneObserved, Observed};

        assert!(admit_filesystem_reset(Agent, SingleEpisode, Observed).is_ok());
        assert_eq!(
            admit_filesystem_reset(Agent, SingleEpisode, NoneObserved),
            Err(ResetRefusal::NoSnapshotObserved)
        );
        for evidence in [Observed, NoneObserved] {
            assert_eq!(
                admit_filesystem_reset(Agent, Siblings, evidence),
                Err(ResetRefusal::SiblingsShareOneSandbox),
                "the shape refusal holds whatever Omega read"
            );
            for scope in [Conversation, Turn] {
                for shape in [SingleEpisode, Siblings] {
                    assert_eq!(
                        admit_filesystem_reset(scope, shape, evidence),
                        Err(ResetRefusal::SnapshotLostByFork),
                        "{scope:?} / {shape:?} / {evidence:?} was admitted, and the \
                         restore would fail loading the manifest"
                    );
                }
            }
        }
    }

    /// `OMEGA-DELTA-0107`, omega#103. A fork with no filesystem state to reset
    /// to is refused by name, rather than confusingly.
    ///
    /// Exo answers a missing snapshot and a snapshot the fork left behind with
    /// the same sentence — *"have you taken a snapshot?"* — so an operator who
    /// hits the first goes looking for the second. Omega separates them here,
    /// before a request exists, and each refusal says which one it is.
    #[test]
    fn a_fork_with_no_snapshot_is_refused_for_having_none() {
        let refusal = admit_filesystem_reset(
            SandboxScopeKind::Agent,
            EpisodeShape::SingleEpisode,
            SnapshotEvidence::NoneObserved,
        )
        .expect_err("there is nothing to restore");
        assert_eq!(refusal, ResetRefusal::NoSnapshotObserved);

        let said = refusal.to_string();
        assert!(said.contains("no filesystem state to restore"), "{said}");
        assert!(
            said.contains("not the same failure"),
            "the refusal must separate itself from the fork finding, which is \
             the whole reason it exists: {said}"
        );
        assert_ne!(
            said,
            ResetRefusal::SnapshotLostByFork.to_string(),
            "two situations Exo reports identically must not read identically here"
        );
    }

    #[test]
    fn the_admitted_witness_carries_the_scope_it_was_admitted_for() {
        let admitted = admit_filesystem_reset(
            SandboxScopeKind::Agent,
            EpisodeShape::SingleEpisode,
            SnapshotEvidence::Observed,
        )
        .expect("agent scope, one episode, a snapshot Omega saw");
        assert_eq!(admitted.scope(), SandboxScopeKind::Agent);
        assert_eq!(admitted.scope().token(), "agent");
    }

    #[test]
    fn the_loop_forks_before_it_mutates_and_probes_before_it_reads_the_check() {
        let at = |step: Step| {
            FALSIFICATION_LOOP
                .iter()
                .position(|candidate| *candidate == step)
                .unwrap_or_else(|| panic!("{step:?} is a step of the loop"))
        };
        assert!(
            at(Step::ForkCandidate) < at(Step::ApplyMutationInCandidate),
            "forking after the mutation puts the mutation in the sibling, which is \
             omega#103's second falsifier"
        );
        assert!(at(Step::ForkControl) < at(Step::ApplyMutationInCandidate));
        assert!(
            at(Step::CompareStartingStates) < at(Step::ApplyMutationInCandidate),
            "the starting states are compared while they are still starting states"
        );
        assert!(
            at(Step::ProbeMutationApplied) < at(Step::RunNamedCheck),
            "the probe comes first; a check run against an unapplied mutation passes \
             while testing nothing"
        );
        assert!(at(Step::RunNamedCheck) < at(Step::ReadVerdict));
        assert_eq!(
            FALSIFICATION_LOOP.len(),
            12,
            "a step was added or removed without the ordering being re-argued"
        );
    }

    #[test]
    fn the_verdict_is_total_and_the_probe_wins() {
        assert_eq!(
            verdict(ProbeOutcome::MutationPresent, CheckOutcome::Failed),
            Verdict::Falsified
        );
        assert_eq!(
            verdict(ProbeOutcome::MutationPresent, CheckOutcome::Passed),
            Verdict::CheckDidNotNotice
        );
        assert_eq!(
            verdict(ProbeOutcome::MutationAbsent, CheckOutcome::Failed),
            Verdict::MutationDidNotApply
        );
        assert_eq!(
            verdict(ProbeOutcome::MutationAbsent, CheckOutcome::Passed),
            Verdict::MutationDidNotApply,
            "a check that passes under a mutation that never applied is not a check \
             that noticed anything"
        );
    }

    #[test]
    fn a_failing_check_is_the_only_thing_that_counts_as_falsified() {
        let verdicts = [
            verdict(ProbeOutcome::MutationPresent, CheckOutcome::Passed),
            verdict(ProbeOutcome::MutationAbsent, CheckOutcome::Passed),
            verdict(ProbeOutcome::MutationAbsent, CheckOutcome::Failed),
        ];
        assert!(
            verdicts
                .iter()
                .all(|verdict| *verdict != Verdict::Falsified),
            "only a present mutation with a failing check is a falsification"
        );
    }
}
