//! Render public-safe inspector and room-header field rows.
//!
//! Output is structured label/value pairs for GPUI or tests. Values never
//! include raw tokens, private paths, or unbounded tool content.

use crate::public_ref::PublicRef;
use crate::receipt::AuthorityReceiptDetail;
use crate::room_header::RoomAuthorityHeader;

/// Conversation header title per spec §9.6 — name only, no authority details.
pub const CONVERSATION_HEADER_TITLE: &str = "Sarah";

/// One label/value row in the inspector or room header strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectorField {
    pub label: &'static str,
    pub value: String,
}

impl InspectorField {
    pub fn new(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
        }
    }

    pub fn line(&self) -> String {
        format!("{}: {}", self.label, self.value)
    }
}

/// Details view for one activity/receipt row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptInspectorView {
    pub fields: Vec<InspectorField>,
    pub allowed: bool,
}

impl ReceiptInspectorView {
    pub fn lines(&self) -> Vec<String> {
        self.fields.iter().map(InspectorField::line).collect()
    }

    pub fn joined_text(&self) -> String {
        self.lines().join("\n")
    }

    pub fn field_value(&self, label: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map(|f| f.value.as_str())
    }
}

/// Room header area authority strip (separate from conversation header).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomHeaderAuthorityView {
    /// Always `Sarah` — must not include profile or revision.
    pub conversation_header: &'static str,
    pub fields: Vec<InspectorField>,
}

impl RoomHeaderAuthorityView {
    pub fn lines(&self) -> Vec<String> {
        self.fields.iter().map(InspectorField::line).collect()
    }

    pub fn joined_text(&self) -> String {
        self.lines().join("\n")
    }
}

/// Render the details view for one receipt row.
pub fn render_receipt_detail(detail: &AuthorityReceiptDetail) -> ReceiptInspectorView {
    let mut fields = vec![
        InspectorField::new("tool_ref", detail.tool_ref.as_str()),
        InspectorField::new("allowed", if detail.allowed { "true" } else { "false" }),
        InspectorField::new(
            "authority_receipt_ref",
            detail.authority_receipt_ref.as_str(),
        ),
        InspectorField::new("decision_ref", detail.decision_ref.as_str()),
        InspectorField::new(
            "bounded_result_ref",
            detail
                .bounded_result_ref
                .as_ref()
                .map(PublicRef::as_str)
                .unwrap_or("—"),
        ),
    ];

    if let Some(refusal) = &detail.refusal {
        fields.push(InspectorField::new(
            "reason_class",
            refusal.reason_class.as_label(),
        ));
        if let Some(category) = &refusal.reserved_category {
            fields.push(InspectorField::new("reserved_category", category.as_str()));
        }
    }

    ReceiptInspectorView {
        fields,
        allowed: detail.allowed,
    }
}

/// Render the room header authority strip.
///
/// Conversation header stays `Sarah`. Profile ref and revision live here only.
pub fn render_room_header_authority(header: &RoomAuthorityHeader) -> RoomHeaderAuthorityView {
    RoomHeaderAuthorityView {
        conversation_header: CONVERSATION_HEADER_TITLE,
        fields: vec![
            InspectorField::new(
                "authority_profile_ref",
                header.authority_profile_ref.as_str(),
            ),
            InspectorField::new("authority_revision", header.authority_revision.to_string()),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_ref::PublicRef;
    use crate::receipt::{
        AuthorityReceiptDetail, RawAuthorityBlock, RefusalDetail, RefusalReasonClass,
    };
    use crate::room_header::RoomAuthorityHeader;

    fn allowed_detail() -> AuthorityReceiptDetail {
        AuthorityReceiptDetail::from_raw(RawAuthorityBlock {
            tool_ref: "tool.sarah.list_full_auto_runs",
            allowed: true,
            authority_receipt_ref: "receipt.authority.sarah.tool.ok.1",
            decision_ref: "decision.sarah.tool.call-ok",
            result_refs: &["result.full_auto.list.page1"],
            bounded_result_ref: None,
            refusal_reason: None,
            reserved_category: None,
        })
        .expect("allowed")
    }

    fn refused_detail() -> AuthorityReceiptDetail {
        AuthorityReceiptDetail::from_raw(RawAuthorityBlock {
            tool_ref: "tool.sarah.export_secret",
            allowed: false,
            authority_receipt_ref: "receipt.authority.sarah.tool.refuse.1",
            decision_ref: "decision.sarah.tool.call-refuse",
            result_refs: &["blocker.sarah.authority_refused"],
            bounded_result_ref: None,
            refusal_reason: Some("reserved_action"),
            reserved_category: Some("reserved.secret_export"),
        })
        .expect("refused")
    }

    #[test]
    fn renders_allowed_receipt_fields() {
        let view = render_receipt_detail(&allowed_detail());
        assert!(view.allowed);
        assert_eq!(
            view.field_value("tool_ref"),
            Some("tool.sarah.list_full_auto_runs")
        );
        assert_eq!(view.field_value("allowed"), Some("true"));
        assert_eq!(
            view.field_value("authority_receipt_ref"),
            Some("receipt.authority.sarah.tool.ok.1")
        );
        assert_eq!(
            view.field_value("decision_ref"),
            Some("decision.sarah.tool.call-ok")
        );
        assert_eq!(
            view.field_value("bounded_result_ref"),
            Some("result.full_auto.list.page1")
        );
        assert!(view.field_value("reason_class").is_none());
        assert!(view.field_value("reserved_category").is_none());
        let text = view.joined_text();
        assert!(!text.contains("sk-"));
        assert!(!text.contains("/Users/"));
    }

    #[test]
    fn renders_refused_receipt_with_reason_class_and_reserved_category() {
        let view = render_receipt_detail(&refused_detail());
        assert!(!view.allowed);
        assert_eq!(view.field_value("allowed"), Some("false"));
        assert_eq!(view.field_value("reason_class"), Some("reserved_action"));
        assert_eq!(
            view.field_value("reserved_category"),
            Some("reserved.secret_export")
        );
        // Refusals are not errors in the label set — reason_class is explicit.
        let labels: Vec<_> = view.fields.iter().map(|f| f.label).collect();
        assert!(!labels.contains(&"error"));
        assert!(labels.contains(&"reason_class"));
    }

    #[test]
    fn room_header_holds_profile_conversation_header_is_sarah_only() {
        let header = RoomAuthorityHeader::from_raw("openagents.sarah-owner-orchestrator", 6)
            .expect("header");
        let view = render_room_header_authority(&header);
        assert_eq!(view.conversation_header, "Sarah");
        assert_eq!(view.conversation_header, CONVERSATION_HEADER_TITLE);
        assert_eq!(
            view.fields
                .iter()
                .find(|f| f.label == "authority_profile_ref")
                .map(|f| f.value.as_str()),
            Some("openagents.sarah-owner-orchestrator")
        );
        assert_eq!(
            view.fields
                .iter()
                .find(|f| f.label == "authority_revision")
                .map(|f| f.value.as_str()),
            Some("6")
        );
        // Conversation header must not carry authority text.
        assert!(!view.conversation_header.contains("openagents"));
        assert!(!view.conversation_header.contains("revision"));
        assert!(!view.conversation_header.contains("6"));
    }

    #[test]
    fn rendering_never_includes_dropped_unsafe_material() {
        let detail = AuthorityReceiptDetail {
            tool_ref: PublicRef::new("tool.sarah.safe").unwrap(),
            allowed: false,
            authority_receipt_ref: PublicRef::new("receipt.authority.sarah.tool.x").unwrap(),
            decision_ref: PublicRef::new("decision.sarah.tool.x").unwrap(),
            bounded_result_ref: Some(PublicRef::new("blocker.sarah.authority_refused").unwrap()),
            bounded_result_refs: vec![PublicRef::new("blocker.sarah.authority_refused").unwrap()],
            refusal: Some(RefusalDetail::new(
                RefusalReasonClass::ReservedAction,
                Some(PublicRef::new("reserved.secret_export").unwrap()),
            )),
        };
        let text = render_receipt_detail(&detail).joined_text();
        assert!(!text.contains("Bearer"));
        assert!(!text.contains("sk-"));
        assert!(!text.contains("/Users/"));
        assert!(!text.contains("OPENAGENTS_AGENT_TOKEN"));
        assert!(text.contains("reserved_action"));
        assert!(text.contains("reserved.secret_export"));
    }
}
