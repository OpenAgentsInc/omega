use anyhow::{Context as _, Result, anyhow};
use convex::{FunctionResult, Value};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttentionRequest {
    pub request_id: String,
    pub kind: String,
    pub summary: String,
    pub recorded_at: f64,
}

/// The bounded shell row returned by `workShells:attentionInbox`.
///
/// Keep the complete web/mobile projection here even though the spike window
/// renders only its primary fields. That makes contract drift a decode failure
/// instead of silently creating a fourth, narrower desktop projection.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttentionRow {
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub label: String,
    pub identifier: Option<String>,
    pub href: Option<String>,
    pub status: String,
    pub attention_state: String,
    pub attention_rank: f64,
    pub pending_approval_count: f64,
    pub pending_input_count: f64,
    pub visibility: String,
    pub snoozed_at: Option<f64>,
    pub snoozed_until: Option<f64>,
    pub settled_at: Option<f64>,
    pub woke_at: Option<f64>,
    pub woke_reason: Option<String>,
    pub branch_summary: Option<String>,
    pub pull_request_summary: Option<String>,
    pub latest_turn_ref: Option<String>,
    pub generation: f64,
    pub last_command_id: String,
    pub created_at: f64,
    pub updated_at: f64,
    pub pending_requests: Vec<AttentionRequest>,
}

pub fn decode_attention_rows(result: FunctionResult) -> Result<Vec<AttentionRow>> {
    let value = match result {
        FunctionResult::Value(value) => value,
        FunctionResult::ErrorMessage(message) => {
            return Err(anyhow!("Convex inbox query failed: {message}"));
        }
        FunctionResult::ConvexError(error) => {
            return Err(anyhow!("Convex inbox query failed: {}", error.message));
        }
    };
    let json: serde_json::Value = Value::into(value);
    serde_json::from_value(json).context("Convex inbox projection did not match its Rust contract")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_complete_bounded_inbox_row() {
        let value = serde_json::json!([{
            "aggregateType": "issue",
            "aggregateId": "issue_42",
            "label": "Omega on Convex",
            "identifier": "PRO-42",
            "href": "/issues/PRO-42",
            "status": "started",
            "attentionState": "working",
            "attentionRank": 3.0,
            "pendingApprovalCount": 0.0,
            "pendingInputCount": 0.0,
            "visibility": "active",
            "snoozedAt": null,
            "snoozedUntil": null,
            "settledAt": null,
            "wokeAt": null,
            "wokeReason": null,
            "branchSummary": "codex/issue-42",
            "pullRequestSummary": null,
            "latestTurnRef": "turn_1",
            "generation": 7.0,
            "lastCommandId": "cmd_7",
            "createdAt": 1.0,
            "updatedAt": 2.0,
            "pendingRequests": []
        }]);
        let convex_value = Value::try_from(value).expect("fixture should be a Convex value");
        let rows = decode_attention_rows(FunctionResult::Value(convex_value))
            .expect("fixture should decode");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].identifier.as_deref(), Some("PRO-42"));
        assert_eq!(rows[0].generation, 7.0);
    }

    #[test]
    fn rejects_a_projection_that_drops_required_web_fields() {
        let convex_value = Value::try_from(serde_json::json!([{
            "aggregateType": "issue",
            "aggregateId": "issue_42",
            "label": "Omega on Convex"
        }]))
        .expect("fixture should be a Convex value");
        let error = decode_attention_rows(FunctionResult::Value(convex_value))
            .expect_err("incomplete projection must fail closed");
        assert!(error.to_string().contains("Rust contract"));
    }
}
