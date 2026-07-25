//! Authority receipt inspector for the Sarah workroom (`OMEGA-SW-05`).
//!
//! This crate is record-agnostic projection logic for one activity/receipt row
//! and the room-header authority profile strip. It holds no durable state and
//! never returns raw tokens, private paths, or unbounded tool output.
//!
//! `workroom_ui` (OMEGA-SW-03) consumes these types when wiring the pane.

mod issue31_host;
mod public_ref;
mod receipt;
mod render;
mod room_header;

pub use issue31_host::{
    decode_issue31_host_adjunct, Issue31CommandState, Issue31Gap, Issue31HostAdjunct,
    Issue31HostAdjunctError, Issue31HostProjection, Issue31ProjectionCapability,
    Issue31ProjectionSource, Issue31Role, Issue31RoleKind, Issue31RoleStatus,
    Issue31SourceKind, Issue31TerminalState, ProjectionFreshness,
    ISSUE31_HOST_ADJUNCT_SCHEMA, MAX_ISSUE31_PROJECTION_REFS, MAX_ISSUE31_TIMESTAMP_MS,
};
pub use public_ref::{is_public_safe_ref, sanitize_public_ref, PublicRef, PUBLIC_REF_MAX_LEN};
pub use receipt::{
    AuthorityBlockJson, AuthorityReceiptDetail, AuthorityStatus, RawAuthorityBlock, RefusalDetail,
    RefusalReasonClass, MAX_BOUNDED_RESULT_REFS,
};
pub use render::{
    render_receipt_detail, render_room_header_authority, InspectorField, ReceiptInspectorView,
    RoomHeaderAuthorityView, CONVERSATION_HEADER_TITLE,
};
pub use room_header::{RoomAuthorityHeader, DEFAULT_AUTHORITY_PROFILE_REF};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_is_receipt_inspector_not_full_workroom_pane() {
        let path = module_path!();
        assert!(path.contains("workroom_receipts"));
        assert!(!path.contains("workroom_ui"));
    }
}
