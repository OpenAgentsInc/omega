//! Exo's durable log, read. `OMEGA-DELTA-0091`, omega#104.
//!
//! Omega attaches to **`exoharness/exo`** over ACP (omega#87, PR #94) and sees
//! the live turn: text, tool calls, tool results as ACP framed them, and a
//! completion record. Beside that stream sits Exo's actual record — a durable,
//! replayable event log with versioned artifacts and sandbox snapshots — and
//! until this crate, Omega read none of it.
//!
//! The cost of that gap was paid before it was noticed. On 2026-07-26 a
//! `read_subagent_transcript` tool was built here from scratch because a parent
//! thread could see only a subagent's final message. That tool is still right
//! for Omega's own native subagents. For an Exo-backed thread it was a second,
//! weaker record beside a complete one on a socket Omega already talks to.
//!
//! # What this crate is allowed to do
//!
//! Read. Eight of Exo's fifty-two request variants, on loopback, with no
//! authentication asserted and no write, fork, sandbox, or secret authority.
//!
//! * [`ExoQuery`] is closed at those eight. A write cannot be *named*, so it
//!   cannot be sent. That is the shape this issue asked for: the wrong call is
//!   not refused at runtime, it does not exist.
//! * [`ExoReadClient`] is constructed from an address and fails on anything
//!   that is not this machine, with a reason. Exo's endpoint has no
//!   authentication — its own documentation says a bearer token is accepted and
//!   never checked — so loopback is the entire boundary, and the resolved
//!   socket address is checked a second time before a connection is opened.
//! * [`ExoHistory`] renders the record. Artifacts are what carry tool results,
//!   so a history built without artifact reads keeps every name and loses every
//!   body, and says so in the row.
//!
//! # What it must never do
//!
//! * Expose Exo's endpoint beyond loopback through any Omega surface.
//! * Treat [`HarnessReportedUsage`] as accounting truth. Exo never makes the
//!   model call through an attested path and says so itself. Receipts mark
//!   these numbers as harness-reported; this type converts into nothing.
//! * Reach Exo's secrets. The env-var injection paths are Exo's to run.
//! * Fork, snapshot, or start anything. That is omega#103 and it is scoped
//!   there.
//!
//! # Where the wire format came from
//!
//! Three witnesses in the pinned tree (`omega_exo_lane::EXO_PIN`), because
//! teardown prose is reference and not truth:
//! `crates/exoharness/src/protocol.rs` (the enums and their serde tags),
//! `typescript/harness/runner.ts` (the same objects written by hand by Exo's
//! own bridge), and `docs/exoharness-http.md` (the envelope, with examples).
//! The variant count was counted off `Request::kind` rather than quoted: it is
//! **52**.
//!
//! # One enumeration, two decisions
//!
//! `OMEGA-DELTA-0102`. That list of fifty-two now lives once, in
//! `omega_exo_lane::ExoRequestKind`, and [`admission`] is this crate's decision
//! over it. `omega_exo_episode::family` is the other decision over the same
//! enumeration, and the two are deliberately not merged: that one admits
//! `conversation_fork` because forking is the episode reset, this one refuses it
//! because this client is read-only. See [`admission`] for the ten reads this
//! crate also refuses and why.

pub mod admission;
pub mod client;
pub mod history;
pub mod query;
pub mod record;

pub use admission::{admitted_read_kinds, is_admitted_read, unadmitted_kinds};
pub use client::{EXO_REQUEST_PATH, ExoReadClient, ExoReadError, exo_default_port};
pub use history::{ExoArtifactSet, ExoBody, ExoHistory, ExoHistoryRow};
pub use query::{ExoEventWindow, ExoId, ExoQuery, ExoReadDirection, NotAnExoId};
pub use record::{
    ExoAgentRecord, ExoArtifact, ExoArtifactRef, ExoArtifactVersion, ExoConversation,
    ExoConversationRecord, ExoEvent, ExoEventBody, ExoEventPage, ExoResponseTag,
    HarnessReportedUsage,
};

/// Every request kind this crate can send, as wire strings.
///
/// `OMEGA-DELTA-0102`. Derived twice over — from
/// `omega_exo_lane::ExoRequestKind::ALL` through [`is_admitted_read`], and
/// spelled by `ExoRequestKind::wire` — so there is no hand-kept list here to
/// drift from the decision. It was a transcription until the two Exo crates were
/// found holding one protocol between them in two copies.
#[must_use]
pub fn exo_admitted_read_kinds() -> Vec<&'static str> {
    admitted_read_kinds()
        .into_iter()
        .map(omega_exo_lane::ExoRequestKind::wire)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The declared list and the type agree, in both directions.
    #[test]
    fn the_admitted_list_is_exactly_what_the_query_type_can_name() {
        let id = ExoId::parse("0198f3ec-1b7a-7c31-9f0e-6d2a4b8c1d55").expect("a UUID");
        let every = [
            ExoQuery::AgentShow { agent: id.clone() },
            ExoQuery::AgentArtifacts { agent: id.clone() },
            ExoQuery::AgentArtifact {
                agent: id.clone(),
                artifact: id.clone(),
                version: None,
            },
            ExoQuery::ConversationShow {
                agent: id.clone(),
                conversation: id.clone(),
            },
            ExoQuery::ConversationEvents {
                agent: id.clone(),
                conversation: id.clone(),
                window: ExoEventWindow::default(),
            },
            ExoQuery::ConversationEvent {
                agent: id.clone(),
                conversation: id.clone(),
                event: id.clone(),
            },
            ExoQuery::ConversationArtifacts {
                agent: id.clone(),
                conversation: id.clone(),
            },
            ExoQuery::ConversationArtifact {
                agent: id.clone(),
                conversation: id.clone(),
                artifact: id,
                version: None,
            },
        ];
        let mut named: Vec<&str> = every.iter().map(ExoQuery::wire_kind).collect();
        named.sort_unstable();
        let mut declared: Vec<&str> = exo_admitted_read_kinds();
        declared.sort_unstable();
        assert_eq!(named, declared);
        assert_eq!(named.len(), 8);
    }

    /// The published wire spellings are Exo's, not a rename.
    ///
    /// `exo_admitted_read_kinds` is derived, so it cannot disagree with the
    /// decision — but it could still be derived from a renamed enumeration. The
    /// three spellings this crate exists for are stated once here against the
    /// strings Exo actually answers to.
    #[test]
    fn the_published_kinds_are_the_strings_exo_answers_to() {
        let published = exo_admitted_read_kinds();
        for expected in [
            "get_agent",
            "agent_list_artifacts",
            "agent_read_artifact",
            "get_conversation",
            "conversation_get_events",
            "conversation_get_event",
            "conversation_list_artifacts",
            "conversation_read_artifact",
        ] {
            assert!(
                published.contains(&expected),
                "the client can no longer name `{expected}`: {published:?}"
            );
        }
        assert_eq!(published.len(), 8);
    }
}
