//! Typed ACP/provider transport bridge for live forensic task calls.
//!
//! This module only accepts structured JSON from a completed tool-call event.
//! It deliberately has no Markdown or assistant-message ingestion API.

use omega_forensics::{ForensicSourceCatalog, ForensicToolEvent, ForensicToolJournal};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibleForensicToolCallState {
    Pending,
    InProgress,
    Completed,
    Failed,
    Rejected,
    Cancelled,
}

/// Normalize a provider's structured argument envelope and ingest one
/// completed, first-party forensic tool call. Unknown tools remain ordinary
/// transcript events and do not mutate forensic state.
pub fn ingest_visible_forensic_tool_call(
    tool_name: &str,
    raw_input: &serde_json::Value,
    state: VisibleForensicToolCallState,
    journal: &mut ForensicToolJournal,
    sources: &ForensicSourceCatalog,
) -> anyhow::Result<Option<ForensicToolEvent>> {
    if state != VisibleForensicToolCallState::Completed {
        return Ok(None);
    }
    journal
        .ingest_visible_tool_json(tool_name, raw_input, sources)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use omega_forensics::{
        FORENSIC_TOOL_VERSION_V1, ForensicSourceCatalog, ForensicToolActorRole,
        ForensicToolCallBinding, ForensicToolJournal,
    };

    use super::*;

    fn journal() -> ForensicToolJournal {
        ForensicToolJournal::new(
            ForensicToolCallBinding {
                run_ref: "run:bridge:1".into(),
                task_ref: "task:bridge:1".into(),
                actor_ref: "actor:discovery:1".into(),
                actor_role: ForensicToolActorRole::Discovery,
                audience_ref: "audience:private:owner".into(),
                source_bundle_ref: "source-bundle:bridge:1".into(),
                source_bundle_digest: format!("sha256:{}", "a".repeat(64)),
                coverage_generation: 1,
                prompt_digest: format!("sha256:{}", "b".repeat(64)),
                model_route_ref: "model-route:bridge:1".into(),
                tool_version: FORENSIC_TOOL_VERSION_V1.into(),
                budget_ref: "budget:bridge:1".into(),
                expected_event_cursor: 0,
            },
            "actor:verifier:1".into(),
        )
        .expect("journal")
    }

    fn sources() -> ForensicSourceCatalog {
        ForensicSourceCatalog {
            audience_ref: "audience:private:owner".into(),
            source_bundle_ref: "source-bundle:bridge:1".into(),
            source_bundle_digest: format!("sha256:{}", "a".repeat(64)),
            coverage_generation: 1,
            missing_dependency_paths: Vec::new(),
            files: Vec::new(),
        }
    }

    fn query_call() -> serde_json::Value {
        serde_json::json!({
            "callRef": "call:bridge:1",
            "idempotencyRef": "idempotency:bridge:1",
            "binding": {
                "runRef": "run:bridge:1",
                "taskRef": "task:bridge:1",
                "actorRef": "actor:discovery:1",
                "actorRole": "discovery",
                "audienceRef": "audience:private:owner",
                "sourceBundleRef": "source-bundle:bridge:1",
                "sourceBundleDigest": format!("sha256:{}", "a".repeat(64)),
                "coverageGeneration": 1,
                "promptDigest": format!("sha256:{}", "b".repeat(64)),
                "modelRouteRef": "model-route:bridge:1",
                "toolVersion": FORENSIC_TOOL_VERSION_V1,
                "budgetRef": "budget:bridge:1",
                "expectedEventCursor": 0
            },
            "payload": {
                "tool": "query_prior_forensic_work",
                "input": { "query_ref": "query:bridge:1" }
            },
            "observedAt": "2026-08-03T23:00:00Z"
        })
    }

    #[test]
    fn provider_envelopes_normalize_to_the_same_canonical_event() {
        let direct = query_call();
        let encoded = direct.to_string();
        let variants = [
            direct.clone(),
            serde_json::json!({ "input": direct }),
            serde_json::json!({ "arguments": encoded }),
        ];
        let mut projections = Vec::new();
        for variant in variants {
            let mut journal = journal();
            let event = ingest_visible_forensic_tool_call(
                "query_prior_forensic_work",
                &variant,
                VisibleForensicToolCallState::Completed,
                &mut journal,
                &sources(),
            )
            .expect("bridge")
            .expect("forensic event");
            projections.push((event, journal));
        }
        assert!(projections.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn markdown_unknown_pending_and_mismatched_tools_create_no_state() {
        let mut journal = journal();
        let catalog = sources();
        assert!(
            ingest_visible_forensic_tool_call(
                "assistant_markdown",
                &serde_json::json!({ "text": "submit_forensic_finding" }),
                VisibleForensicToolCallState::Completed,
                &mut journal,
                &catalog,
            )
            .expect("unknown tool")
            .is_none()
        );
        assert!(
            ingest_visible_forensic_tool_call(
                "query_prior_forensic_work",
                &query_call(),
                VisibleForensicToolCallState::InProgress,
                &mut journal,
                &catalog,
            )
            .expect("pending")
            .is_none()
        );
        assert!(
            ingest_visible_forensic_tool_call(
                "get_forensic_work_by_ref",
                &query_call(),
                VisibleForensicToolCallState::Completed,
                &mut journal,
                &catalog,
            )
            .is_err()
        );
        assert!(journal.events.is_empty());
        assert_eq!(journal.command_digests, BTreeMap::new());
    }
}
