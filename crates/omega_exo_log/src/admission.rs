//! Which of Exo's 52 requests this read-only client may name.
//! `OMEGA-DELTA-0091` and `OMEGA-DELTA-0102`, omega#104.
//!
//! # One enumeration, two decisions
//!
//! The list of request types is **not** here. It is
//! `omega_exo_lane::ExoRequestKind`, written once, and this module is one of two
//! decisions taken over it — the other being `omega_exo_episode::family`, which
//! partitions the same fifty-two into admitted and refused *families* for the
//! episode reset. Both crates originally transcribed the protocol for
//! themselves; the transcriptions agreed exactly, and that was still one copy
//! too many.
//!
//! The two decisions are not merged, and they must not be. They admit different
//! subsets for different reasons:
//!
//! * `conversation_fork` and `start_sandbox` are **admitted** by the episode
//!   law — forking *is* the episode reset, and the restore is its filesystem
//!   half — and **refused** here, because this client is read-only and
//!   omega#104 says so in as many words.
//! * `list_agents`, `list_conversations`, `get_sandbox_process_events`,
//!   `wait_sandbox_process`, and the six binding list-and-get variants are
//!   **admitted** by the episode law, which classifies them `Query` because
//!   they change nothing, and **refused** here, because omega#104 scoped this
//!   client to *a conversation's own record* and a list of every agent on the
//!   host is not that.
//!
//! Collapsing the two would hand one side a capability nobody granted it. Ten
//! reads is the smaller cost; the fork is the larger one.
//!
//! # A fifty-third variant cannot pass unclassified
//!
//! [`is_admitted_read`] is a `match` with no wildcard arm. A variant added to
//! `ExoRequestKind` does not compile until somebody decides about it *here*,
//! and independently until somebody decides about it in the episode crate. The
//! decision is forced on the person adding the variant rather than discovered at
//! runtime by whoever sent it.
//!
//! Refusal is spelled out per-variant rather than as a `_ => false` arm on
//! purpose. `false` by default would make a new upstream variant silently
//! refused, which is safe — and would also mean nobody ever reads it, which is
//! how the fork stayed unnoticed in two places for a day.

use omega_exo_lane::ExoRequestKind;

/// Whether this crate's client may name this request type.
///
/// Eight of Exo's fifty-two. Total by construction: no wildcard arm.
#[must_use]
pub const fn is_admitted_read(kind: ExoRequestKind) -> bool {
    match kind {
        // The eight this client exists for. Every one of them reads a
        // conversation's own durable record, or the agent record above it.
        ExoRequestKind::GetAgent
        | ExoRequestKind::AgentListArtifacts
        | ExoRequestKind::AgentReadArtifact
        | ExoRequestKind::GetConversation
        | ExoRequestKind::ConversationGetEvents
        | ExoRequestKind::ConversationGetEvent
        | ExoRequestKind::ConversationListArtifacts
        | ExoRequestKind::ConversationReadArtifact => true,

        // Reads, and refused anyway: they are about the *host*, not about the
        // thread being rendered. A denylist of writes would have admitted all
        // ten of these silently.
        ExoRequestKind::ListAgents
        | ExoRequestKind::ListConversations
        | ExoRequestKind::ListBindings
        | ExoRequestKind::GetBinding
        | ExoRequestKind::AgentListBindings
        | ExoRequestKind::AgentGetBinding
        | ExoRequestKind::ConversationListBindings
        | ExoRequestKind::ConversationGetBinding
        | ExoRequestKind::GetSandboxProcessEvents
        | ExoRequestKind::WaitSandboxProcess => false,

        // The fork and the restore. Admitted by `omega_exo_episode`, refused
        // here: omega#104 grants no write, fork, or sandbox authority and says
        // forking is omega#103's and is scoped there.
        ExoRequestKind::ConversationFork | ExoRequestKind::StartSandbox => false,

        // Writes.
        ExoRequestKind::NewAgent
        | ExoRequestKind::DeleteAgent
        | ExoRequestKind::PutBinding
        | ExoRequestKind::NewConversation
        | ExoRequestKind::DeleteConversation
        | ExoRequestKind::AgentWriteArtifact
        | ExoRequestKind::CreateSandbox
        | ExoRequestKind::SnapshotSandbox
        | ExoRequestKind::StopSandbox
        | ExoRequestKind::StartSandboxProcess
        | ExoRequestKind::WriteSandboxProcessInput
        | ExoRequestKind::CloseSandboxProcessInput
        | ExoRequestKind::CancelSandboxProcess
        | ExoRequestKind::AgentPutBinding
        | ExoRequestKind::ConversationStartSession
        | ExoRequestKind::ConversationEndSession
        | ExoRequestKind::ConversationBeginTurn
        | ExoRequestKind::ConversationAddEvents
        | ExoRequestKind::ConversationWriteArtifact
        | ExoRequestKind::ConversationPutBinding
        | ExoRequestKind::TurnAddEvents
        | ExoRequestKind::TurnWriteArtifact
        | ExoRequestKind::TurnFinish => false,

        // Secrets, listing included. Exo's env-var injection paths are Exo's to
        // run, not Omega's to reach through, and the names of an operator's
        // secrets are the operator's business.
        ExoRequestKind::ListSecrets
        | ExoRequestKind::PutSecret
        | ExoRequestKind::GetSecret
        | ExoRequestKind::AgentListSecrets
        | ExoRequestKind::AgentPutSecret
        | ExoRequestKind::AgentGetSecret
        | ExoRequestKind::ConversationListSecrets
        | ExoRequestKind::ConversationPutSecret
        | ExoRequestKind::ConversationGetSecret => false,
    }
}

/// Every request type this client may name.
///
/// Derived from the single enumeration and [`is_admitted_read`], never
/// transcribed. Built rather than stored as a constant so there is no second
/// list to drift: the only way to change it is to change the decision.
#[must_use]
pub fn admitted_read_kinds() -> Vec<ExoRequestKind> {
    ExoRequestKind::ALL
        .into_iter()
        .filter(|kind| is_admitted_read(*kind))
        .collect()
}

/// Every request type this client may not name.
#[must_use]
pub fn unadmitted_kinds() -> Vec<ExoRequestKind> {
    ExoRequestKind::ALL
        .into_iter()
        .filter(|kind| !is_admitted_read(*kind))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{ExoEventWindow, ExoId, ExoQuery};

    fn every_query() -> Vec<ExoQuery> {
        let id = ExoId::parse("0198f3ec-1b7a-7c31-9f0e-6d2a4b8c1d55").expect("a UUID");
        vec![
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
        ]
    }

    /// The decision and the type agree, in both directions.
    ///
    /// This is the load-bearing pair. A kind admitted here that no variant can
    /// build is a permission nobody can use; a variant that builds a kind this
    /// decision refuses is a capability nobody granted.
    #[test]
    fn the_admitted_set_is_exactly_what_the_query_type_can_name() {
        let mut named: Vec<ExoRequestKind> = every_query().iter().map(ExoQuery::kind).collect();
        named.sort_unstable();
        named.dedup();
        assert_eq!(
            named,
            admitted_read_kinds(),
            "the request kinds the query type can produce are no longer exactly \
             the admitted reads. A kind that vanished is a read the workspace can \
             no longer perform; a kind that appeared is a capability nobody \
             granted."
        );
        assert_eq!(named.len(), 8);
    }

    #[test]
    fn the_two_halves_partition_exos_protocol() {
        assert_eq!(
            admitted_read_kinds().len() + unadmitted_kinds().len(),
            omega_exo_lane::EXO_REQUEST_KIND_COUNT
        );
        assert_eq!(unadmitted_kinds().len(), 44);
        for kind in admitted_read_kinds() {
            assert!(
                !unadmitted_kinds().contains(&kind),
                "{kind} is on both sides"
            );
        }
    }

    /// The refusals this issue named, and the ten that a denylist of writes
    /// would have admitted by accident.
    #[test]
    fn the_client_refuses_the_writes_the_secrets_and_the_host_wide_reads() {
        for refused in [
            ExoRequestKind::ConversationFork,
            ExoRequestKind::StartSandbox,
            ExoRequestKind::ConversationAddEvents,
            ExoRequestKind::TurnAddEvents,
            ExoRequestKind::TurnFinish,
            ExoRequestKind::DeleteAgent,
            ExoRequestKind::GetSecret,
            ExoRequestKind::AgentListSecrets,
        ] {
            assert!(!is_admitted_read(refused), "{refused} is admitted");
        }
        for host_wide in [
            ExoRequestKind::ListAgents,
            ExoRequestKind::ListConversations,
            ExoRequestKind::ListBindings,
            ExoRequestKind::GetBinding,
            ExoRequestKind::AgentListBindings,
            ExoRequestKind::AgentGetBinding,
            ExoRequestKind::ConversationListBindings,
            ExoRequestKind::ConversationGetBinding,
            ExoRequestKind::GetSandboxProcessEvents,
            ExoRequestKind::WaitSandboxProcess,
        ] {
            assert!(
                !is_admitted_read(host_wide),
                "{host_wide} reads, and reading is not the boundary: this client \
                 is scoped to one conversation's own record"
            );
        }
    }

    /// The disagreement with `omega_exo_episode`, asserted from this side.
    ///
    /// `OMEGA-DELTA-0102` checks it across both crates; this states the half
    /// this crate is responsible for, so a merge of the two decisions fails here
    /// too rather than only in the registry.
    #[test]
    fn the_reader_refuses_the_two_capabilities_the_episode_law_holds() {
        assert!(!is_admitted_read(ExoRequestKind::ConversationFork));
        assert!(!is_admitted_read(ExoRequestKind::StartSandbox));
    }
}
