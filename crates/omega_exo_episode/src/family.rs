//! Which of Exo's 52 requests an episode may send. `OMEGA-DELTA-0090`,
//! omega#103.
//!
//! `exo serve` answers one unary endpoint, `POST /request`, with **no
//! authentication** — a client may send a bearer token and the server never
//! checks it. Behind that endpoint sits the whole protocol, including
//! `get_secret`, `delete_agent`, and `turn_add_events`. Loopback keeps the
//! endpoint off the network (`omega_exo_lane::LoopbackEndpoint`); this module
//! keeps the *authority* small once you are on it.
//!
//! The boundary is stated as a partition of the entire protocol rather than as
//! an allowlist of the calls this crate happens to make. An allowlist answers
//! "is this call permitted"; a partition answers "what is every call, and
//! which side is it on" — and it fails when upstream adds a 53rd variant
//! nobody classified, which is the case an allowlist is silent about.
//!
//! # One enumeration, two decisions
//!
//! `OMEGA-DELTA-0102`. The list of request types is **not** here. It is
//! `omega_exo_lane::ExoRequestKind`, written once, and this module is one of
//! two decisions taken over it — the other being `omega_exo_log`'s read
//! admission. Both crates originally transcribed the fifty-two variants for
//! themselves; the transcriptions agreed exactly, and that was still one copy
//! too many, because the *next* upstream variant would have to be noticed twice
//! by two people who each already believed their list was complete.
//!
//! The two decisions are deliberately not merged. They admit different subsets,
//! and `conversation_fork` is the clearest case: admitted here because forking
//! *is* the episode reset, refused there because that client is read-only.
//! Merging them would hand one of the two a capability nobody granted it.
//!
//! [`family_of`] is a `match` with no wildcard arm, so a 53rd variant added to
//! `ExoRequestKind` does not compile until somebody classifies it here — and
//! the same is independently true in the log crate.
//!
//! # The families
//!
//! [`RequestFamily`] has five variants and [`family_of`] assigns one to each of
//! the 52 request types at [`crate::EXO_PROTOCOL_PIN`]. Three are admitted:
//!
//! * [`RequestFamily::Query`] — reads. They change nothing.
//! * [`RequestFamily::Fork`] — `conversation_fork`, the episode reset itself.
//! * [`RequestFamily::Reset`] — `start_sandbox`, the filesystem half.
//!
//! and two are not:
//!
//! * [`RequestFamily::Write`] — everything that appends to somebody's history,
//!   creates or deletes a record, or drives a process.
//! * [`RequestFamily::Secret`] — the secret family, including the *list* calls.
//!   Listing returns metadata rather than plaintext, and it is still refused:
//!   the names of an operator's secrets are the operator's business, and an
//!   episode has no use for them.
//!
//! # Fork and Reset are not Query, and the crate says so
//!
//! `conversation_fork` writes. It creates a conversation record, copies
//! bindings, secrets, artifacts and sandbox records into it, replays every
//! event under a fresh id, and appends a `conversation_forked` event. It is
//! admitted because it is the mechanism, not because it is harmless, and it is
//! its own family so that nobody reads "queries only" and believes the episode
//! leaves Exo's storage untouched. It does not. It adds one conversation.
//!
//! `start_sandbox` writes too: it flips a stored sandbox record to running,
//! records the snapshot it was restored from, and appends a sandbox lifecycle
//! event. See [`crate::reset`] for the reason Omega cannot actually reach a
//! useful one of these at this pin.

use omega_exo_lane::ExoRequestKind;

/// What a request does to Exo, coarsely enough to decide with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestFamily {
    /// A read. Admitted.
    Query,
    /// `conversation_fork`. Admitted; it is the episode reset.
    Fork,
    /// `start_sandbox`. Admitted; it is the filesystem half of the reset.
    Reset,
    /// Anything that appends history, creates or deletes a record, or drives a
    /// process. Refused.
    Write,
    /// The secret family, listing included. Refused.
    Secret,
}

impl RequestFamily {
    /// Whether an episode may send a request of this family.
    #[must_use]
    pub const fn is_admitted(self) -> bool {
        matches!(self, Self::Query | Self::Fork | Self::Reset)
    }

    /// The stable token this family is recorded under.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Fork => "fork",
            Self::Reset => "reset",
            Self::Write => "write",
            Self::Secret => "secret",
        }
    }
}

impl std::fmt::Display for RequestFamily {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.token())
    }
}

/// The episode law's decision, for every request type Exo has.
///
/// Total by construction: no wildcard arm, so upstream's 53rd variant is a
/// build failure here rather than an unclassified request somebody sends.
#[must_use]
pub const fn family_of(kind: ExoRequestKind) -> RequestFamily {
    match kind {
        ExoRequestKind::ListAgents => RequestFamily::Query,
        ExoRequestKind::GetAgent => RequestFamily::Query,
        ExoRequestKind::NewAgent => RequestFamily::Write,
        ExoRequestKind::DeleteAgent => RequestFamily::Write,
        ExoRequestKind::ListBindings => RequestFamily::Query,
        ExoRequestKind::PutBinding => RequestFamily::Write,
        ExoRequestKind::GetBinding => RequestFamily::Query,
        ExoRequestKind::ListSecrets => RequestFamily::Secret,
        ExoRequestKind::PutSecret => RequestFamily::Secret,
        ExoRequestKind::GetSecret => RequestFamily::Secret,
        ExoRequestKind::ListConversations => RequestFamily::Query,
        ExoRequestKind::GetConversation => RequestFamily::Query,
        ExoRequestKind::NewConversation => RequestFamily::Write,
        ExoRequestKind::DeleteConversation => RequestFamily::Write,
        ExoRequestKind::AgentListArtifacts => RequestFamily::Query,
        ExoRequestKind::AgentReadArtifact => RequestFamily::Query,
        ExoRequestKind::AgentWriteArtifact => RequestFamily::Write,
        ExoRequestKind::CreateSandbox => RequestFamily::Write,
        ExoRequestKind::SnapshotSandbox => RequestFamily::Write,
        ExoRequestKind::StartSandbox => RequestFamily::Reset,
        ExoRequestKind::StopSandbox => RequestFamily::Write,
        ExoRequestKind::StartSandboxProcess => RequestFamily::Write,
        ExoRequestKind::WriteSandboxProcessInput => RequestFamily::Write,
        ExoRequestKind::CloseSandboxProcessInput => RequestFamily::Write,
        ExoRequestKind::GetSandboxProcessEvents => RequestFamily::Query,
        ExoRequestKind::WaitSandboxProcess => RequestFamily::Query,
        ExoRequestKind::CancelSandboxProcess => RequestFamily::Write,
        ExoRequestKind::AgentListBindings => RequestFamily::Query,
        ExoRequestKind::AgentPutBinding => RequestFamily::Write,
        ExoRequestKind::AgentGetBinding => RequestFamily::Query,
        ExoRequestKind::AgentListSecrets => RequestFamily::Secret,
        ExoRequestKind::AgentPutSecret => RequestFamily::Secret,
        ExoRequestKind::AgentGetSecret => RequestFamily::Secret,
        ExoRequestKind::ConversationStartSession => RequestFamily::Write,
        ExoRequestKind::ConversationEndSession => RequestFamily::Write,
        ExoRequestKind::ConversationBeginTurn => RequestFamily::Write,
        ExoRequestKind::ConversationGetEvents => RequestFamily::Query,
        ExoRequestKind::ConversationGetEvent => RequestFamily::Query,
        ExoRequestKind::ConversationAddEvents => RequestFamily::Write,
        ExoRequestKind::ConversationFork => RequestFamily::Fork,
        ExoRequestKind::ConversationListArtifacts => RequestFamily::Query,
        ExoRequestKind::ConversationReadArtifact => RequestFamily::Query,
        ExoRequestKind::ConversationWriteArtifact => RequestFamily::Write,
        ExoRequestKind::ConversationListBindings => RequestFamily::Query,
        ExoRequestKind::ConversationPutBinding => RequestFamily::Write,
        ExoRequestKind::ConversationGetBinding => RequestFamily::Query,
        ExoRequestKind::ConversationListSecrets => RequestFamily::Secret,
        ExoRequestKind::ConversationPutSecret => RequestFamily::Secret,
        ExoRequestKind::ConversationGetSecret => RequestFamily::Secret,
        ExoRequestKind::TurnAddEvents => RequestFamily::Write,
        ExoRequestKind::TurnWriteArtifact => RequestFamily::Write,
        ExoRequestKind::TurnFinish => RequestFamily::Write,
    }
}

/// Whether an episode may send this request type.
#[must_use]
pub const fn is_admitted(kind: ExoRequestKind) -> bool {
    family_of(kind).is_admitted()
}

/// Which family a wire request type belongs to.
///
/// `None` for a type this build has never heard of. A caller must treat that as
/// refused: an unclassified request is one somebody added upstream after
/// `omega_exo_lane::ExoRequestKind` was written, and the safe reading of "we do
/// not know what this does" is not "send it".
#[must_use]
pub fn family_of_wire(request_type: &str) -> Option<RequestFamily> {
    ExoRequestKind::from_wire(request_type).map(family_of)
}

/// Whether an episode may send this wire request type. Unknown types are
/// refused.
#[must_use]
pub fn is_admitted_wire(request_type: &str) -> bool {
    family_of_wire(request_type).is_some_and(RequestFamily::is_admitted)
}

/// Every request type Exo has at the pin, with the family it belongs to.
///
/// Derived from the single enumeration and this module's decision, never
/// transcribed. Built rather than stored as a constant so there is no second
/// list to drift: the only way to change a row is to change [`family_of`].
#[must_use]
pub fn exo_request_families() -> Vec<(ExoRequestKind, RequestFamily)> {
    ExoRequestKind::ALL
        .into_iter()
        .map(|kind| (kind, family_of(kind)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_partition_covers_every_request_type_exactly_once() {
        let classified = exo_request_families();
        assert_eq!(
            classified.len(),
            omega_exo_lane::EXO_REQUEST_KIND_COUNT,
            "the partition has {} rows and Exo's protocol has {} request types, \
             so some variant is unclassified",
            classified.len(),
            omega_exo_lane::EXO_REQUEST_KIND_COUNT
        );
        let mut seen: Vec<ExoRequestKind> = classified.iter().map(|(kind, _)| *kind).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(
            seen.len(),
            before,
            "a request type is classified twice, so one of the two rows is unreachable"
        );
    }

    #[test]
    fn every_secret_bearing_request_is_in_the_secret_family() {
        for (kind, family) in exo_request_families() {
            if kind.wire().contains("secret") {
                assert_eq!(
                    family,
                    RequestFamily::Secret,
                    "{kind} names secrets and is classified {family}"
                );
            }
        }
        let secrets = exo_request_families()
            .into_iter()
            .filter(|(_, family)| *family == RequestFamily::Secret)
            .count();
        assert_eq!(
            secrets, 9,
            "Exo has three secret scopes with list/put/get in each; \
             {secrets} rows is not that shape"
        );
    }

    #[test]
    fn exactly_two_request_types_are_admitted_beyond_reading() {
        let of_family = |wanted: RequestFamily| -> Vec<ExoRequestKind> {
            exo_request_families()
                .into_iter()
                .filter(|(_, family)| *family == wanted)
                .map(|(kind, _)| kind)
                .collect()
        };
        assert_eq!(
            of_family(RequestFamily::Fork),
            [ExoRequestKind::ConversationFork]
        );
        assert_eq!(
            of_family(RequestFamily::Reset),
            [ExoRequestKind::StartSandbox]
        );
    }

    #[test]
    fn the_calls_that_would_change_somebody_elses_history_are_refused() {
        for refused in [
            ExoRequestKind::ConversationAddEvents,
            ExoRequestKind::TurnAddEvents,
            ExoRequestKind::TurnFinish,
            ExoRequestKind::ConversationBeginTurn,
            ExoRequestKind::DeleteConversation,
            ExoRequestKind::DeleteAgent,
            ExoRequestKind::NewAgent,
            ExoRequestKind::SnapshotSandbox,
            ExoRequestKind::StopSandbox,
            ExoRequestKind::StartSandboxProcess,
            ExoRequestKind::GetSecret,
            ExoRequestKind::AgentGetSecret,
            ExoRequestKind::ConversationListSecrets,
        ] {
            assert!(
                !is_admitted(refused),
                "{refused} is admitted, and it should not be"
            );
        }
    }

    #[test]
    fn an_unknown_request_type_is_refused_rather_than_assumed_harmless() {
        assert_eq!(family_of_wire("conversation_teleport"), None);
        assert!(!is_admitted_wire("conversation_teleport"));
        assert!(!is_admitted_wire(""));
        // Exo's event tags overlap its request kinds, and this lookup is exact.
        assert_eq!(family_of_wire("conversation_forked"), None);
    }

    #[test]
    fn the_admitted_set_is_the_one_the_issue_named() {
        let admitted: Vec<ExoRequestKind> = exo_request_families()
            .into_iter()
            .filter(|(_, family)| family.is_admitted())
            .map(|(kind, _)| kind)
            .collect();
        assert!(admitted.contains(&ExoRequestKind::ConversationFork));
        assert!(admitted.contains(&ExoRequestKind::StartSandbox));
        assert!(admitted.contains(&ExoRequestKind::ConversationGetEvents));
        assert!(
            admitted.len() == 20,
            "the admitted set is {} types: {admitted:?}. If that number moved, \
             a family was reclassified and the reclassification is the change \
             worth reviewing.",
            admitted.len()
        );
    }

    /// This decision keeps the two capabilities the read-only log client
    /// refuses. `OMEGA-DELTA-0102`.
    ///
    /// The two decisions over one enumeration are meant to disagree; the whole
    /// argument for keeping them separate is that merging them would hand one
    /// side a capability nobody granted it. `omega_exo_log` is not a dependency
    /// of this crate, so the disagreement is asserted from each side here and
    /// checked *across* both in `OMEGA-DELTA-0102`.
    #[test]
    fn the_episode_law_keeps_the_two_capabilities_a_reader_must_not_have() {
        assert_eq!(
            family_of(ExoRequestKind::ConversationFork),
            RequestFamily::Fork,
            "the episode law refuses the fork that is its own mechanism"
        );
        assert_eq!(
            family_of(ExoRequestKind::StartSandbox),
            RequestFamily::Reset,
            "the episode law refuses the restore that is the filesystem half"
        );
    }
}
