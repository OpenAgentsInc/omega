//! Authority receipt detail for one activity row.
//!
//! An allowed decision is never proof that the target broker acted. The
//! inspector surfaces the authority block only: tool, allowed flag, receipt,
//! decision, bounded result refs, and refusal reason class when refused.

use serde::{Deserialize, Serialize};

use crate::public_ref::{PublicRef, is_public_safe_ref, sanitize_public_ref};

/// Cap on bounded result references kept on one inspector row.
pub const MAX_BOUNDED_RESULT_REFS: usize = 8;

/// Allowed vs refused authority decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityStatus {
    Allowed,
    Refused,
}

impl AuthorityStatus {
    pub fn from_allowed_flag(allowed: bool) -> Self {
        if allowed {
            Self::Allowed
        } else {
            Self::Refused
        }
    }

    pub fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed)
    }

    pub fn as_label(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Refused => "refused",
        }
    }
}

/// Reason class for a refused authority decision.
///
/// Matches the `@openagentsinc/authority` denial reasons plus a reserved
/// category when the refusal is a reserved-action deny.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "class")]
pub enum RefusalReasonClass {
    ProfileInactive,
    ReservedAction,
    GrantNotFound,
    ConditionMissing,
    ConditionFailed,
    ProfileInvalid,
    AuthorityRefused,
    /// Public-safe unknown class string after sanitization.
    Other {
        reason_class: PublicRef,
    },
}

impl RefusalReasonClass {
    /// Parse a public-safe reason class token from service/runtime output.
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        let normalized = trimmed.to_ascii_lowercase().replace('-', "_");
        match normalized.as_str() {
            "profile_inactive" => Some(Self::ProfileInactive),
            "reserved_action" | "reserved" | "needs_owner_reserved_action" => {
                Some(Self::ReservedAction)
            }
            "grant_not_found" => Some(Self::GrantNotFound),
            "condition_missing" => Some(Self::ConditionMissing),
            "condition_failed" => Some(Self::ConditionFailed),
            "profile_invalid" => Some(Self::ProfileInvalid),
            "authority_refused" | "denied" | "refused" => Some(Self::AuthorityRefused),
            other => sanitize_public_ref(other).map(|reason_class| Self::Other { reason_class }),
        }
    }

    pub fn as_label(&self) -> &str {
        match self {
            Self::ProfileInactive => "profile_inactive",
            Self::ReservedAction => "reserved_action",
            Self::GrantNotFound => "grant_not_found",
            Self::ConditionMissing => "condition_missing",
            Self::ConditionFailed => "condition_failed",
            Self::ProfileInvalid => "profile_invalid",
            Self::AuthorityRefused => "authority_refused",
            Self::Other { reason_class } => reason_class.as_str(),
        }
    }

    pub fn is_reserved(&self) -> bool {
        matches!(self, Self::ReservedAction)
    }
}

/// Refusal detail shown when `allowed` is false.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusalDetail {
    pub reason_class: RefusalReasonClass,
    /// Present when the refusal maps to a reserved category id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved_category: Option<PublicRef>,
}

impl RefusalDetail {
    pub fn new(reason_class: RefusalReasonClass, reserved_category: Option<PublicRef>) -> Self {
        // Only surface reserved_category when the reason class is reserved.
        let reserved_category = if reason_class.is_reserved() {
            reserved_category
        } else {
            None
        };
        Self {
            reason_class,
            reserved_category,
        }
    }
}

/// Public-safe details for one activity/receipt row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityReceiptDetail {
    pub tool_ref: PublicRef,
    pub allowed: bool,
    pub authority_receipt_ref: PublicRef,
    pub decision_ref: PublicRef,
    /// Primary bounded result reference (first safe result ref, if any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounded_result_ref: Option<PublicRef>,
    /// Additional bounded result references (already capped and redacted).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bounded_result_refs: Vec<PublicRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<RefusalDetail>,
}

impl AuthorityReceiptDetail {
    pub fn status(&self) -> AuthorityStatus {
        AuthorityStatus::from_allowed_flag(self.allowed)
    }

    /// Build a detail view from raw service fields, redacting unsafe values.
    ///
    /// Returns `None` when the required public refs cannot be sanitized.
    pub fn from_raw(input: RawAuthorityBlock<'_>) -> Option<Self> {
        let tool_ref = sanitize_public_ref(input.tool_ref)?;
        let authority_receipt_ref = sanitize_public_ref(input.authority_receipt_ref)?;
        let decision_ref = sanitize_public_ref(input.decision_ref)?;

        let mut bounded_result_refs: Vec<PublicRef> = input
            .result_refs
            .iter()
            .filter_map(|r| sanitize_public_ref(r))
            .take(MAX_BOUNDED_RESULT_REFS)
            .collect();
        // Prefer an explicit primary when provided and safe.
        if let Some(primary) = input.bounded_result_ref.and_then(sanitize_public_ref) {
            bounded_result_refs.retain(|r| r != &primary);
            bounded_result_refs.insert(0, primary);
            if bounded_result_refs.len() > MAX_BOUNDED_RESULT_REFS {
                bounded_result_refs.truncate(MAX_BOUNDED_RESULT_REFS);
            }
        }
        let bounded_result_ref = bounded_result_refs.first().cloned();

        let refusal = if input.allowed {
            None
        } else {
            let reason_class = input
                .refusal_reason
                .and_then(RefusalReasonClass::parse)
                .or_else(|| {
                    // Derive reserved from blocker / category hints.
                    if input
                        .reserved_category
                        .is_some_and(|c| is_public_safe_ref(c) || c.starts_with("reserved."))
                    {
                        Some(RefusalReasonClass::ReservedAction)
                    } else {
                        Some(RefusalReasonClass::AuthorityRefused)
                    }
                })
                .unwrap_or(RefusalReasonClass::AuthorityRefused);
            let reserved_category = input
                .reserved_category
                .and_then(sanitize_public_ref)
                .or_else(|| {
                    input.result_refs.iter().find_map(|r| {
                        let s = r.trim();
                        if s.starts_with("reserved.") {
                            sanitize_public_ref(s)
                        } else {
                            None
                        }
                    })
                });
            Some(RefusalDetail::new(reason_class, reserved_category))
        };

        Some(Self {
            tool_ref,
            allowed: input.allowed,
            authority_receipt_ref,
            decision_ref,
            bounded_result_ref,
            bounded_result_refs,
            refusal,
        })
    }
}

/// Raw authority block fields as they arrive from the service projection.
///
/// Callers pass references only; this type never owns secrets and drops any
/// field that fails public-safety checks during projection.
#[derive(Clone, Debug)]
pub struct RawAuthorityBlock<'a> {
    pub tool_ref: &'a str,
    pub allowed: bool,
    pub authority_receipt_ref: &'a str,
    pub decision_ref: &'a str,
    pub result_refs: &'a [&'a str],
    pub bounded_result_ref: Option<&'a str>,
    pub refusal_reason: Option<&'a str>,
    pub reserved_category: Option<&'a str>,
}

/// JSON-facing authority block (service event payload subset).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorityBlockJson {
    #[serde(default, alias = "tool_ref")]
    pub tool_ref: Option<String>,
    #[serde(default)]
    pub allowed: Option<bool>,
    #[serde(
        default,
        alias = "authority_receipt_ref",
        alias = "authorityRef",
        alias = "authority_ref"
    )]
    pub authority_receipt_ref: Option<String>,
    #[serde(default, alias = "decision_ref")]
    pub decision_ref: Option<String>,
    #[serde(default, alias = "result_refs", alias = "resultRefs")]
    pub result_refs: Vec<String>,
    #[serde(default, alias = "bounded_result_ref", alias = "boundedResultRef")]
    pub bounded_result_ref: Option<String>,
    #[serde(
        default,
        alias = "refusal_reason",
        alias = "refusalReason",
        alias = "reason"
    )]
    pub refusal_reason: Option<String>,
    #[serde(default, alias = "reserved_category", alias = "reservedCategory")]
    pub reserved_category: Option<String>,
    // Unknown keys (content, token, rawOutput, paths, etc.) are ignored by
    // serde's default and never become inspector fields.
}

impl AuthorityBlockJson {
    /// Project into a public-safe receipt detail, dropping unsafe fields.
    pub fn project(&self) -> Option<AuthorityReceiptDetail> {
        let tool_ref = self.tool_ref.as_deref()?;
        let authority_receipt_ref = self.authority_receipt_ref.as_deref()?;
        let decision_ref = self.decision_ref.as_deref()?;
        let result_ref_stores: Vec<&str> = self.result_refs.iter().map(String::as_str).collect();
        AuthorityReceiptDetail::from_raw(RawAuthorityBlock {
            tool_ref,
            allowed: self.allowed.unwrap_or(true),
            authority_receipt_ref,
            decision_ref,
            result_refs: &result_ref_stores,
            bounded_result_ref: self.bounded_result_ref.as_deref(),
            refusal_reason: self.refusal_reason.as_deref(),
            reserved_category: self.reserved_category.as_deref(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_receipt_has_no_refusal() {
        let detail = AuthorityReceiptDetail::from_raw(RawAuthorityBlock {
            tool_ref: "tool.sarah.list_full_auto_runs",
            allowed: true,
            authority_receipt_ref: "receipt.authority.sarah.tool.abc.list",
            decision_ref: "decision.sarah.tool.call-1",
            result_refs: &["result.full_auto.run.r1"],
            bounded_result_ref: None,
            refusal_reason: Some("should_be_ignored"),
            reserved_category: Some("reserved.secret_export"),
        })
        .expect("valid allowed receipt");
        assert!(detail.allowed);
        assert!(detail.refusal.is_none());
        assert_eq!(
            detail.bounded_result_ref.as_ref().map(PublicRef::as_str),
            Some("result.full_auto.run.r1")
        );
    }

    #[test]
    fn refused_receipt_exposes_reason_class_and_reserved_category() {
        let detail = AuthorityReceiptDetail::from_raw(RawAuthorityBlock {
            tool_ref: "tool.sarah.move_financial_value",
            allowed: false,
            authority_receipt_ref: "receipt.authority.sarah.tool.abc.refuse",
            decision_ref: "decision.sarah.tool.call-2",
            result_refs: &["blocker.sarah.authority_refused"],
            bounded_result_ref: None,
            refusal_reason: Some("reserved_action"),
            reserved_category: Some("reserved.financial_custody"),
        })
        .expect("valid refused receipt");
        assert!(!detail.allowed);
        let refusal = detail.refusal.expect("refusal present");
        assert_eq!(refusal.reason_class.as_label(), "reserved_action");
        assert_eq!(
            refusal.reserved_category.as_ref().map(PublicRef::as_str),
            Some("reserved.financial_custody")
        );
    }

    #[test]
    fn redacts_unsafe_result_refs_and_required_fields() {
        let detail = AuthorityReceiptDetail::from_raw(RawAuthorityBlock {
            tool_ref: "tool.sarah.read_release_state",
            allowed: true,
            authority_receipt_ref: "receipt.authority.sarah.tool.safe",
            decision_ref: "decision.sarah.tool.call-3",
            result_refs: &[
                "result.release.state.ok",
                "/Users/owner/.codex/auth.json",
                "sk-ant-api03-SECRETVALUE0000000000",
                "result.with spaces is bad",
            ],
            bounded_result_ref: Some("Bearer eyJhbGciOiJIUzI1NiJ9"),
            refusal_reason: None,
            reserved_category: None,
        })
        .expect("required refs valid");
        assert_eq!(detail.bounded_result_refs.len(), 1);
        assert_eq!(
            detail.bounded_result_ref.as_ref().map(PublicRef::as_str),
            Some("result.release.state.ok")
        );
    }

    #[test]
    fn rejects_when_required_ref_is_unsafe() {
        assert!(
            AuthorityReceiptDetail::from_raw(RawAuthorityBlock {
                tool_ref: "/tmp/evil",
                allowed: true,
                authority_receipt_ref: "receipt.authority.sarah.tool.safe",
                decision_ref: "decision.sarah.tool.call-4",
                result_refs: &[],
                bounded_result_ref: None,
                refusal_reason: None,
                reserved_category: None,
            })
            .is_none()
        );
    }

    #[test]
    fn json_projection_drops_unsafe_payload_fields() {
        let json = serde_json::json!({
            "toolRef": "tool.sarah.control_full_auto",
            "allowed": false,
            "authorityReceiptRef": "receipt.authority.sarah.tool.ctrl",
            "decisionRef": "decision.sarah.tool.call-5",
            "resultRefs": ["blocker.sarah.authority_refused"],
            "refusalReason": "reserved_action",
            "reservedCategory": "reserved.stable_release_without_direction",
            "content": "full tool dump with secrets sk-abc",
            "token": "OPENAGENTS_AGENT_TOKEN=should-not-appear",
            "rawOutput": {"path": "/Users/owner/private"}
        });
        let block: AuthorityBlockJson = serde_json::from_value(json).expect("parse");
        let detail = block.project().expect("project");
        let encoded = serde_json::to_string(&detail).expect("encode");
        assert!(!encoded.contains("sk-abc"));
        assert!(!encoded.contains("OPENAGENTS_AGENT_TOKEN"));
        assert!(!encoded.contains("/Users/owner"));
        assert!(!encoded.contains("full tool dump"));
        assert!(encoded.contains("reserved_action"));
        assert!(encoded.contains("reserved.stable_release_without_direction"));
        assert!(!detail.allowed);
    }
}
