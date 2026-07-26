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
//! **52**, and Omega's own `omega_exo_lane::endpoint` prose says 53.

pub mod client;
pub mod history;
pub mod query;
pub mod record;

pub use client::{EXO_REQUEST_PATH, ExoReadClient, ExoReadError, exo_default_port};
pub use history::{ExoArtifactSet, ExoBody, ExoHistory, ExoHistoryRow};
pub use query::{ExoEventWindow, ExoId, ExoQuery, ExoReadDirection, NotAnExoId};
pub use record::{
    ExoAgentRecord, ExoArtifact, ExoArtifactRef, ExoArtifactVersion, ExoConversation,
    ExoConversationRecord, ExoEvent, ExoEventBody, ExoEventPage, ExoResponseTag,
    HarnessReportedUsage,
};

/// Every request kind this crate can send, in one list.
///
/// The registry `OMEGA-DELTA-0091` reads. It is derived from [`ExoQuery`] by
/// hand and checked against it, rather than being the source: a list that were
/// the source could grow a write kind without a variant existing to send it.
pub const EXO_ADMITTED_READ_KINDS: &[&str] = &[
    "get_agent",
    "agent_list_artifacts",
    "agent_read_artifact",
    "get_conversation",
    "conversation_get_events",
    "conversation_get_event",
    "conversation_list_artifacts",
    "conversation_read_artifact",
];

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
        let mut declared: Vec<&str> = EXO_ADMITTED_READ_KINDS.to_vec();
        declared.sort_unstable();
        assert_eq!(named, declared);
        assert_eq!(named.len(), 8);
    }
}
