//! Authority receipt inspector for the Sarah workroom (`OMEGA-SW-05`).
//!
//! This crate is record-agnostic projection logic for one activity/receipt row
//! and the room-header authority profile strip. It holds no durable state and
//! never returns raw tokens, private paths, or unbounded tool output.
//!
//! `workroom_ui` (OMEGA-SW-03) consumes these types when wiring the pane.

mod community_verification;
mod issue31_full_auto;
mod issue31_host;
mod public_ref;
mod receipt;
mod render;
mod room_header;

pub use community_verification::{
    AdmittedVerification, COMMUNITY_FEEDBACK_KIND, COMMUNITY_INDEPENDENT_VERIFICATION_PACKET,
    COMMUNITY_INDEPENDENT_VERIFICATION_SCHEMA, CommunityBinding,
    INDEPENDENT_VERIFICATION_FEEDBACK_TYPE, SHARED_FIXTURE_DIGESTS, VerificationEvent,
    VerificationRefusal, VerificationVerdict, admit_independent_verification,
};
pub use issue31_full_auto::{
    ISSUE31_EVIDENCE_HOPS, ISSUE31_FULL_AUTO_ADJUNCT_SCHEMA, Issue31EvidenceChain,
    Issue31EvidenceHop, Issue31EvidenceHopKind, Issue31EvidenceUnavailableReason,
    Issue31FullAutoAdjunct, Issue31FullAutoAdjunctError, Issue31FullAutoControl,
    Issue31FullAutoControlKind, Issue31FullAutoLifecycle, Issue31FullAutoRun,
    Issue31ProviderAccount, Issue31ProviderHandoff, Issue31ProviderHandoffState,
    Issue31ProviderQuota, Issue31ProviderReadiness, MAX_ISSUE31_FULL_AUTO_ACCOUNTS,
    MAX_ISSUE31_FULL_AUTO_CONTROLS, MAX_ISSUE31_FULL_AUTO_HANDOFFS, MAX_ISSUE31_FULL_AUTO_RUNS,
    MAX_ISSUE31_UNATTENDED_MS, build_issue31_full_auto_adjunct, decode_issue31_full_auto_adjunct,
    is_issue31_public_text, project_issue31_evidence_pair,
};
pub use issue31_host::{
    ISSUE31_HOST_ADJUNCT_SCHEMA, Issue31AbsentGap, Issue31CommandState, Issue31CommandStateInput,
    Issue31Gap, Issue31HostAdjunct, Issue31HostAdjunctError, Issue31HostProjection,
    Issue31HostProjectionInput, Issue31HostSources, Issue31ObservedGap,
    Issue31ProjectionCapability, Issue31ProjectionSource, Issue31Role, Issue31RoleInput,
    Issue31RoleKind, Issue31RoleStatus, Issue31SourceKind, Issue31TerminalState,
    MAX_ISSUE31_PROJECTION_REFS, MAX_ISSUE31_TIMESTAMP_MS, ProjectionFreshness,
    build_issue31_host_adjunct, decode_issue31_host_adjunct,
};
pub use public_ref::{PUBLIC_REF_MAX_LEN, PublicRef, is_public_safe_ref, sanitize_public_ref};
pub use receipt::{
    AuthorityBlockJson, AuthorityReceiptDetail, AuthorityStatus, MAX_BOUNDED_RESULT_REFS,
    RawAuthorityBlock, RefusalDetail, RefusalReasonClass,
};
pub use render::{
    CONVERSATION_HEADER_TITLE, InspectorField, ReceiptInspectorView, RoomHeaderAuthorityView,
    render_receipt_detail, render_room_header_authority,
};
pub use room_header::{DEFAULT_AUTHORITY_PROFILE_REF, RoomAuthorityHeader};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_is_receipt_inspector_not_full_workroom_pane() {
        let path = module_path!();
        assert!(path.contains("workroom_receipts"));
        assert!(!path.contains("workroom_ui"));
    }
}
