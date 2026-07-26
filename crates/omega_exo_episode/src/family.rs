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
//! # The families
//!
//! [`RequestFamily`] has five variants and [`EXO_REQUEST_FAMILIES`] assigns one
//! to each of the 52 request types at [`crate::EXO_PROTOCOL_PIN`]. Three are
//! admitted:
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

/// Every request type Exo's protocol has at [`crate::EXO_PROTOCOL_PIN`], with
/// the family it belongs to.
///
/// Transcribed from `Request::kind` in `crates/exoharness/src/protocol.rs`, in
/// declaration order, which is also the order the enum is written in. The count
/// is checked against [`EXO_REQUEST_TYPE_COUNT`] so a partial transcription
/// fails rather than quietly admitting the variants nobody copied.
pub const EXO_REQUEST_FAMILIES: &[(&str, RequestFamily)] = &[
    ("list_agents", RequestFamily::Query),
    ("get_agent", RequestFamily::Query),
    ("new_agent", RequestFamily::Write),
    ("delete_agent", RequestFamily::Write),
    ("list_bindings", RequestFamily::Query),
    ("put_binding", RequestFamily::Write),
    ("get_binding", RequestFamily::Query),
    ("list_secrets", RequestFamily::Secret),
    ("put_secret", RequestFamily::Secret),
    ("get_secret", RequestFamily::Secret),
    ("list_conversations", RequestFamily::Query),
    ("get_conversation", RequestFamily::Query),
    ("new_conversation", RequestFamily::Write),
    ("delete_conversation", RequestFamily::Write),
    ("agent_list_artifacts", RequestFamily::Query),
    ("agent_read_artifact", RequestFamily::Query),
    ("agent_write_artifact", RequestFamily::Write),
    ("create_sandbox", RequestFamily::Write),
    ("snapshot_sandbox", RequestFamily::Write),
    ("start_sandbox", RequestFamily::Reset),
    ("stop_sandbox", RequestFamily::Write),
    ("start_sandbox_process", RequestFamily::Write),
    ("write_sandbox_process_input", RequestFamily::Write),
    ("close_sandbox_process_input", RequestFamily::Write),
    ("get_sandbox_process_events", RequestFamily::Query),
    ("wait_sandbox_process", RequestFamily::Query),
    ("cancel_sandbox_process", RequestFamily::Write),
    ("agent_list_bindings", RequestFamily::Query),
    ("agent_put_binding", RequestFamily::Write),
    ("agent_get_binding", RequestFamily::Query),
    ("agent_list_secrets", RequestFamily::Secret),
    ("agent_put_secret", RequestFamily::Secret),
    ("agent_get_secret", RequestFamily::Secret),
    ("conversation_start_session", RequestFamily::Write),
    ("conversation_end_session", RequestFamily::Write),
    ("conversation_begin_turn", RequestFamily::Write),
    ("conversation_get_events", RequestFamily::Query),
    ("conversation_get_event", RequestFamily::Query),
    ("conversation_add_events", RequestFamily::Write),
    ("conversation_fork", RequestFamily::Fork),
    ("conversation_list_artifacts", RequestFamily::Query),
    ("conversation_read_artifact", RequestFamily::Query),
    ("conversation_write_artifact", RequestFamily::Write),
    ("conversation_list_bindings", RequestFamily::Query),
    ("conversation_put_binding", RequestFamily::Write),
    ("conversation_get_binding", RequestFamily::Query),
    ("conversation_list_secrets", RequestFamily::Secret),
    ("conversation_put_secret", RequestFamily::Secret),
    ("conversation_get_secret", RequestFamily::Secret),
    ("turn_add_events", RequestFamily::Write),
    ("turn_write_artifact", RequestFamily::Write),
    ("turn_finish", RequestFamily::Write),
];

/// How many request types Exo's protocol has at the pin.
///
/// Counted from `Request::kind`'s match arms. The teardown says 52 and this
/// build measured 52; `crates/omega_exo_lane/src/omega_exo_lane.rs` says 53 in
/// prose, which is prose. If a rebase of the reference clone moves this number
/// the transcription above is stale and [`family_of`] will start returning
/// `None` for real request types, which is the fail-closed direction.
pub const EXO_REQUEST_TYPE_COUNT: usize = 52;

/// Which family a request type belongs to.
///
/// `None` for a type this build has never heard of. A caller must treat that as
/// refused: an unclassified request is one somebody added upstream after this
/// table was written, and the safe reading of "we do not know what this does"
/// is not "send it".
#[must_use]
pub fn family_of(request_type: &str) -> Option<RequestFamily> {
    EXO_REQUEST_FAMILIES
        .iter()
        .find(|(name, _)| *name == request_type)
        .map(|(_, family)| *family)
}

/// Whether an episode may send this request type. Unknown types are refused.
#[must_use]
pub fn is_admitted(request_type: &str) -> bool {
    family_of(request_type).is_some_and(RequestFamily::is_admitted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_partition_covers_every_request_type_exactly_once() {
        assert_eq!(
            EXO_REQUEST_FAMILIES.len(),
            EXO_REQUEST_TYPE_COUNT,
            "the family table has {} rows and Exo's protocol has {EXO_REQUEST_TYPE_COUNT} \
             request types, so some variant is unclassified",
            EXO_REQUEST_FAMILIES.len()
        );
        let mut seen: Vec<&str> = EXO_REQUEST_FAMILIES.iter().map(|(name, _)| *name).collect();
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
        for (name, family) in EXO_REQUEST_FAMILIES {
            if name.contains("secret") {
                assert_eq!(
                    *family,
                    RequestFamily::Secret,
                    "{name} names secrets and is classified {family}"
                );
            }
        }
        let secrets = EXO_REQUEST_FAMILIES
            .iter()
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
        let forks: Vec<&str> = EXO_REQUEST_FAMILIES
            .iter()
            .filter(|(_, family)| *family == RequestFamily::Fork)
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(forks, ["conversation_fork"]);
        let resets: Vec<&str> = EXO_REQUEST_FAMILIES
            .iter()
            .filter(|(_, family)| *family == RequestFamily::Reset)
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(resets, ["start_sandbox"]);
    }

    #[test]
    fn the_calls_that_would_change_somebody_elses_history_are_refused() {
        for refused in [
            "conversation_add_events",
            "turn_add_events",
            "turn_finish",
            "conversation_begin_turn",
            "delete_conversation",
            "delete_agent",
            "new_agent",
            "snapshot_sandbox",
            "stop_sandbox",
            "start_sandbox_process",
            "get_secret",
            "agent_get_secret",
            "conversation_list_secrets",
        ] {
            assert!(
                !is_admitted(refused),
                "{refused} is admitted, and it should not be"
            );
        }
    }

    #[test]
    fn an_unknown_request_type_is_refused_rather_than_assumed_harmless() {
        assert_eq!(family_of("conversation_teleport"), None);
        assert!(!is_admitted("conversation_teleport"));
        assert!(!is_admitted(""));
    }

    #[test]
    fn the_admitted_set_is_the_one_the_issue_named() {
        let admitted: Vec<&str> = EXO_REQUEST_FAMILIES
            .iter()
            .filter(|(_, family)| family.is_admitted())
            .map(|(name, _)| *name)
            .collect();
        assert!(admitted.contains(&"conversation_fork"));
        assert!(admitted.contains(&"start_sandbox"));
        assert!(admitted.contains(&"conversation_get_events"));
        assert!(
            admitted.len() == 20,
            "the admitted set is {} types: {admitted:?}. If that number moved, \
             a family was reclassified and the reclassification is the change \
             worth reviewing.",
            admitted.len()
        );
    }
}
