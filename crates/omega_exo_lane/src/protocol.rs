//! Exo's request protocol, enumerated once. `OMEGA-DELTA-0102`, omega#103 and
//! omega#104.
//!
//! `exo serve` answers one unary endpoint, `POST /request`, with **no
//! authentication** — a client may send a bearer token and the server never
//! checks it. Behind that endpoint sits every request type Exo has, including
//! `get_secret`, `delete_agent`, and `turn_add_events`. Loopback keeps the
//! endpoint off the network ([`crate::LoopbackEndpoint`]); the crates above
//! this one keep the *authority* small once you are on it.
//!
//! # Why the enumeration is here and the decisions are not
//!
//! Two lanes landed on 2026-07-26 with a decision to make about this protocol,
//! and each transcribed all fifty-two variants to make it.
//! `omega_exo_episode::family` partitions them into admitted and refused
//! families; `omega_exo_log` splits them into the eight reads its client may
//! name and the forty-four it may not. Both transcriptions were correct and
//! they agreed exactly, which is the good case — and it is still one copy too
//! many, because the *next* upstream variant has to be noticed twice by two
//! people who each already believe their list is complete.
//!
//! So the list of request types lives here, once, and each crate above keeps
//! its own decision **as a total function over [`ExoRequestKind`]**. The two
//! decisions are deliberately not merged: they admit different subsets, for
//! different reasons, and collapsing them would mean one of the two got a
//! capability nobody granted it. `conversation_fork` is admitted by the episode
//! law because forking *is* the episode reset, and refused by the log client
//! because that client is read-only. Both are right.
//!
//! # A fifty-third variant cannot pass unclassified
//!
//! Every decision over this enum is written as a `match` with no wildcard arm,
//! so a variant added here does not compile until somebody classifies it in
//! each crate that decides about it. The failure is at the build, on the person
//! adding the variant, rather than at runtime on whoever happened to send it.
//!
//! [`ALL`] is held to the same standard from the other direction: it is
//! `[Self; EXO_REQUEST_KIND_COUNT]`, and [`ExoRequestKind::ordinal`] is a
//! wildcard-free `match` whose value is checked against each entry's position.
//! An array that forgot a variant, listed one twice, or grew without the count
//! growing fails.
//!
//! [`ALL`]: ExoRequestKind::ALL

/// How many request types `exo serve` answers at [`crate::EXO_PIN`].
///
/// Counted off `Request::kind` in `crates/exoharness/src/protocol.rs`, not
/// quoted from a teardown. The count is a separate constant from the enum so
/// that [`ExoRequestKind::ALL`] has a declared length to be checked against.
pub const EXO_REQUEST_KIND_COUNT: usize = 52;

/// One request type in Exo's protocol.
///
/// Transcribed from `Request::kind` in `crates/exoharness/src/protocol.rs` at
/// [`crate::EXO_PIN`], in declaration order, which is also the order the
/// upstream enum is written in. This is the *only* transcription of that list
/// in Omega; every other module that decides something about a request decides
/// it over this type.
///
/// The variant names are Omega's spelling of upstream's; the wire strings in
/// [`ExoRequestKind::wire`] are upstream's own, verbatim, and are what actually
/// goes in the `type` field of a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExoRequestKind {
    ListAgents,
    GetAgent,
    NewAgent,
    DeleteAgent,
    ListBindings,
    PutBinding,
    GetBinding,
    ListSecrets,
    PutSecret,
    GetSecret,
    ListConversations,
    GetConversation,
    NewConversation,
    DeleteConversation,
    AgentListArtifacts,
    AgentReadArtifact,
    AgentWriteArtifact,
    CreateSandbox,
    SnapshotSandbox,
    StartSandbox,
    StopSandbox,
    StartSandboxProcess,
    WriteSandboxProcessInput,
    CloseSandboxProcessInput,
    GetSandboxProcessEvents,
    WaitSandboxProcess,
    CancelSandboxProcess,
    AgentListBindings,
    AgentPutBinding,
    AgentGetBinding,
    AgentListSecrets,
    AgentPutSecret,
    AgentGetSecret,
    ConversationStartSession,
    ConversationEndSession,
    ConversationBeginTurn,
    ConversationGetEvents,
    ConversationGetEvent,
    ConversationAddEvents,
    ConversationFork,
    ConversationListArtifacts,
    ConversationReadArtifact,
    ConversationWriteArtifact,
    ConversationListBindings,
    ConversationPutBinding,
    ConversationGetBinding,
    ConversationListSecrets,
    ConversationPutSecret,
    ConversationGetSecret,
    TurnAddEvents,
    TurnWriteArtifact,
    TurnFinish,
}

impl ExoRequestKind {
    /// Every request type Exo has, in upstream's declaration order.
    ///
    /// The length is [`EXO_REQUEST_KIND_COUNT`], so an array that grew or shrank
    /// without the count moving does not compile, and
    /// `the_enumeration_lists_every_variant_exactly_once` holds the other
    /// direction.
    pub const ALL: [Self; EXO_REQUEST_KIND_COUNT] = [
        Self::ListAgents,
        Self::GetAgent,
        Self::NewAgent,
        Self::DeleteAgent,
        Self::ListBindings,
        Self::PutBinding,
        Self::GetBinding,
        Self::ListSecrets,
        Self::PutSecret,
        Self::GetSecret,
        Self::ListConversations,
        Self::GetConversation,
        Self::NewConversation,
        Self::DeleteConversation,
        Self::AgentListArtifacts,
        Self::AgentReadArtifact,
        Self::AgentWriteArtifact,
        Self::CreateSandbox,
        Self::SnapshotSandbox,
        Self::StartSandbox,
        Self::StopSandbox,
        Self::StartSandboxProcess,
        Self::WriteSandboxProcessInput,
        Self::CloseSandboxProcessInput,
        Self::GetSandboxProcessEvents,
        Self::WaitSandboxProcess,
        Self::CancelSandboxProcess,
        Self::AgentListBindings,
        Self::AgentPutBinding,
        Self::AgentGetBinding,
        Self::AgentListSecrets,
        Self::AgentPutSecret,
        Self::AgentGetSecret,
        Self::ConversationStartSession,
        Self::ConversationEndSession,
        Self::ConversationBeginTurn,
        Self::ConversationGetEvents,
        Self::ConversationGetEvent,
        Self::ConversationAddEvents,
        Self::ConversationFork,
        Self::ConversationListArtifacts,
        Self::ConversationReadArtifact,
        Self::ConversationWriteArtifact,
        Self::ConversationListBindings,
        Self::ConversationPutBinding,
        Self::ConversationGetBinding,
        Self::ConversationListSecrets,
        Self::ConversationPutSecret,
        Self::ConversationGetSecret,
        Self::TurnAddEvents,
        Self::TurnWriteArtifact,
        Self::TurnFinish,
    ];

    /// The `type` string this request carries on the wire.
    ///
    /// Upstream's own strings. Wildcard-free, so a variant added to the enum
    /// has to be given its wire spelling here before anything compiles.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::ListAgents => "list_agents",
            Self::GetAgent => "get_agent",
            Self::NewAgent => "new_agent",
            Self::DeleteAgent => "delete_agent",
            Self::ListBindings => "list_bindings",
            Self::PutBinding => "put_binding",
            Self::GetBinding => "get_binding",
            Self::ListSecrets => "list_secrets",
            Self::PutSecret => "put_secret",
            Self::GetSecret => "get_secret",
            Self::ListConversations => "list_conversations",
            Self::GetConversation => "get_conversation",
            Self::NewConversation => "new_conversation",
            Self::DeleteConversation => "delete_conversation",
            Self::AgentListArtifacts => "agent_list_artifacts",
            Self::AgentReadArtifact => "agent_read_artifact",
            Self::AgentWriteArtifact => "agent_write_artifact",
            Self::CreateSandbox => "create_sandbox",
            Self::SnapshotSandbox => "snapshot_sandbox",
            Self::StartSandbox => "start_sandbox",
            Self::StopSandbox => "stop_sandbox",
            Self::StartSandboxProcess => "start_sandbox_process",
            Self::WriteSandboxProcessInput => "write_sandbox_process_input",
            Self::CloseSandboxProcessInput => "close_sandbox_process_input",
            Self::GetSandboxProcessEvents => "get_sandbox_process_events",
            Self::WaitSandboxProcess => "wait_sandbox_process",
            Self::CancelSandboxProcess => "cancel_sandbox_process",
            Self::AgentListBindings => "agent_list_bindings",
            Self::AgentPutBinding => "agent_put_binding",
            Self::AgentGetBinding => "agent_get_binding",
            Self::AgentListSecrets => "agent_list_secrets",
            Self::AgentPutSecret => "agent_put_secret",
            Self::AgentGetSecret => "agent_get_secret",
            Self::ConversationStartSession => "conversation_start_session",
            Self::ConversationEndSession => "conversation_end_session",
            Self::ConversationBeginTurn => "conversation_begin_turn",
            Self::ConversationGetEvents => "conversation_get_events",
            Self::ConversationGetEvent => "conversation_get_event",
            Self::ConversationAddEvents => "conversation_add_events",
            Self::ConversationFork => "conversation_fork",
            Self::ConversationListArtifacts => "conversation_list_artifacts",
            Self::ConversationReadArtifact => "conversation_read_artifact",
            Self::ConversationWriteArtifact => "conversation_write_artifact",
            Self::ConversationListBindings => "conversation_list_bindings",
            Self::ConversationPutBinding => "conversation_put_binding",
            Self::ConversationGetBinding => "conversation_get_binding",
            Self::ConversationListSecrets => "conversation_list_secrets",
            Self::ConversationPutSecret => "conversation_put_secret",
            Self::ConversationGetSecret => "conversation_get_secret",
            Self::TurnAddEvents => "turn_add_events",
            Self::TurnWriteArtifact => "turn_write_artifact",
            Self::TurnFinish => "turn_finish",
        }
    }

    /// This variant's position in [`Self::ALL`].
    ///
    /// Exists so that "`ALL` is every variant, once" is a checkable claim rather
    /// than a careful reading. Wildcard-free: a fifty-third variant needs an
    /// ordinal, the only free ordinal is `52`, and `ALL` has no index `52` until
    /// [`EXO_REQUEST_KIND_COUNT`] and the array both grow.
    #[must_use]
    pub const fn ordinal(self) -> usize {
        match self {
            Self::ListAgents => 0,
            Self::GetAgent => 1,
            Self::NewAgent => 2,
            Self::DeleteAgent => 3,
            Self::ListBindings => 4,
            Self::PutBinding => 5,
            Self::GetBinding => 6,
            Self::ListSecrets => 7,
            Self::PutSecret => 8,
            Self::GetSecret => 9,
            Self::ListConversations => 10,
            Self::GetConversation => 11,
            Self::NewConversation => 12,
            Self::DeleteConversation => 13,
            Self::AgentListArtifacts => 14,
            Self::AgentReadArtifact => 15,
            Self::AgentWriteArtifact => 16,
            Self::CreateSandbox => 17,
            Self::SnapshotSandbox => 18,
            Self::StartSandbox => 19,
            Self::StopSandbox => 20,
            Self::StartSandboxProcess => 21,
            Self::WriteSandboxProcessInput => 22,
            Self::CloseSandboxProcessInput => 23,
            Self::GetSandboxProcessEvents => 24,
            Self::WaitSandboxProcess => 25,
            Self::CancelSandboxProcess => 26,
            Self::AgentListBindings => 27,
            Self::AgentPutBinding => 28,
            Self::AgentGetBinding => 29,
            Self::AgentListSecrets => 30,
            Self::AgentPutSecret => 31,
            Self::AgentGetSecret => 32,
            Self::ConversationStartSession => 33,
            Self::ConversationEndSession => 34,
            Self::ConversationBeginTurn => 35,
            Self::ConversationGetEvents => 36,
            Self::ConversationGetEvent => 37,
            Self::ConversationAddEvents => 38,
            Self::ConversationFork => 39,
            Self::ConversationListArtifacts => 40,
            Self::ConversationReadArtifact => 41,
            Self::ConversationWriteArtifact => 42,
            Self::ConversationListBindings => 43,
            Self::ConversationPutBinding => 44,
            Self::ConversationGetBinding => 45,
            Self::ConversationListSecrets => 46,
            Self::ConversationPutSecret => 47,
            Self::ConversationGetSecret => 48,
            Self::TurnAddEvents => 49,
            Self::TurnWriteArtifact => 50,
            Self::TurnFinish => 51,
        }
    }

    /// The request type this wire string names, if Exo has one at the pin.
    ///
    /// `None` for a string this build has never heard of. Every caller must
    /// treat that as refused rather than as harmless: an unknown request type is
    /// one somebody added upstream after this enum was written, and the safe
    /// reading of "we do not know what this does" is not "send it".
    #[must_use]
    pub fn from_wire(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.wire() == text)
    }
}

impl std::fmt::Display for ExoRequestKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.wire())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The claim [`ExoRequestKind::ALL`] makes, checked rather than read.
    ///
    /// `ordinal` is wildcard-free over the variants and `ALL` is fixed at
    /// [`EXO_REQUEST_KIND_COUNT`] entries, so an array whose entry at index `n`
    /// reports ordinal `n` for all `n` is exactly the variant set, once each. A
    /// duplicated entry reports the same ordinal twice and one of the two is in
    /// the wrong place; a missing one shifts everything after it.
    #[test]
    fn the_enumeration_lists_every_variant_exactly_once() {
        for (index, kind) in ExoRequestKind::ALL.into_iter().enumerate() {
            assert_eq!(
                kind.ordinal(),
                index,
                "{kind} sits at index {index} of ALL and reports ordinal {}. \
                 ALL is either missing a variant, listing one twice, or out of \
                 upstream's declaration order.",
                kind.ordinal()
            );
        }
        assert_eq!(ExoRequestKind::ALL.len(), EXO_REQUEST_KIND_COUNT);
    }

    /// Wire strings are upstream's, and they are distinct.
    ///
    /// Two variants sharing a spelling would make [`ExoRequestKind::from_wire`]
    /// answer one of them arbitrarily, and a decision taken on the answer would
    /// be a decision about the other request.
    #[test]
    fn every_variant_has_its_own_wire_spelling() {
        let mut spellings: Vec<&str> = ExoRequestKind::ALL.iter().map(|kind| kind.wire()).collect();
        spellings.sort_unstable();
        let before = spellings.len();
        spellings.dedup();
        assert_eq!(
            spellings.len(),
            before,
            "two request types share one wire spelling"
        );
        for kind in ExoRequestKind::ALL {
            assert_eq!(ExoRequestKind::from_wire(kind.wire()), Some(kind));
            assert!(
                !kind.wire().is_empty() && kind.wire().is_ascii(),
                "{kind} has a wire spelling Exo would not answer to"
            );
        }
    }

    #[test]
    fn a_request_type_this_build_has_never_heard_of_is_not_recognised() {
        assert_eq!(ExoRequestKind::from_wire("conversation_teleport"), None);
        assert_eq!(ExoRequestKind::from_wire(""), None);
        // Exo's event tags overlap its request kinds — the event
        // `conversation_forked` contains the request `conversation_fork` — and
        // this lookup is exact, so it does not confuse the two.
        assert_eq!(ExoRequestKind::from_wire("conversation_forked"), None);
        assert_eq!(
            ExoRequestKind::from_wire("conversation_fork"),
            Some(ExoRequestKind::ConversationFork)
        );
    }

    /// The three shapes every crate above this one asks about, spelled out once
    /// so a rename upstream fails here rather than in five decision tables.
    #[test]
    fn the_variants_the_crates_above_decide_about_keep_their_names() {
        assert_eq!(ExoRequestKind::ConversationFork.wire(), "conversation_fork");
        assert_eq!(ExoRequestKind::StartSandbox.wire(), "start_sandbox");
        assert_eq!(
            ExoRequestKind::ConversationGetEvents.wire(),
            "conversation_get_events"
        );
    }
}
