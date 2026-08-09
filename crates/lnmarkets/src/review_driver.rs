use gpui::App;
use plugin_api::{ReviewCadence, ReviewTurnEvidence, ReviewTurnOutcome, SessionReviewDriver};
use review_accounting::{
    REVIEW_ACCOUNTING_SCHEMA_VERSION, ReviewCostRecord, ReviewDisposition, ReviewInterventionKind,
    ReviewToolCall,
};

use crate::{
    PORTFOLIO_REVIEW_TOKEN_BUDGET, ReviewTurnOutcome as PortfolioReviewTurnOutcome, SoakReviewTurn,
    WakeupSource, acknowledge_portfolio_wakeup, pending_portfolio_wakeup, portfolio_review_cadence,
    portfolio_review_instruction, record_portfolio_review_evidence, record_portfolio_review_turn,
    record_signet_soak_review_turn,
};

/// The plugin's portfolio-review integration with the agent's wakeup
/// scheduler, registered through the plugin registry so the agent never names
/// this crate.
pub struct LnMarketsReviewDriver;

impl SessionReviewDriver for LnMarketsReviewDriver {
    fn review_cadence(&self, session_id: &str, cx: &App) -> Result<Option<ReviewCadence>, String> {
        Ok(
            portfolio_review_cadence(session_id, cx)?.map(|cadence| match cadence {
                crate::ReviewCadence::FundingSettlement => ReviewCadence::EventDriven,
                crate::ReviewCadence::Interval { seconds } => ReviewCadence::Interval { seconds },
            }),
        )
    }

    fn review_token_budget(&self) -> u64 {
        PORTFOLIO_REVIEW_TOKEN_BUDGET
    }

    fn pending_wakeup(&self, session_id: &str, cx: &App) -> Option<(WakeupSource, String)> {
        pending_portfolio_wakeup(session_id, cx)
    }

    fn review_instruction(
        &self,
        session_id: &str,
        now_ms: i64,
        trigger: &str,
        cx: &App,
    ) -> Result<Option<String>, String> {
        portfolio_review_instruction(session_id, now_ms, trigger, cx)
    }

    fn acknowledge_wakeup(
        &self,
        session_id: &str,
        source: &WakeupSource,
        instruction: &str,
        cx: &App,
    ) -> bool {
        acknowledge_portfolio_wakeup(session_id, source, instruction, cx)
    }

    fn record_review_turn(
        &self,
        session_id: &str,
        at_ms: i64,
        source: WakeupSource,
        outcome: ReviewTurnOutcome,
        cx: &App,
    ) -> bool {
        let outcome = match outcome {
            ReviewTurnOutcome::Completed => PortfolioReviewTurnOutcome::Completed,
            ReviewTurnOutcome::Failed => PortfolioReviewTurnOutcome::Failed,
        };
        record_portfolio_review_turn(session_id, at_ms, source, outcome, cx)
    }

    fn evidence_tool_names(&self) -> &'static [&'static str] {
        &["lnmarkets_strategy"]
    }

    fn record_review_evidence(
        &self,
        session_id: &str,
        evidence: ReviewTurnEvidence,
        cx: &App,
    ) -> bool {
        let cost_record = ReviewCostRecord {
            schema_version: REVIEW_ACCOUNTING_SCHEMA_VERSION,
            turn_id: format!("{session_id}:{}", evidence.at_ms),
            session_id: session_id.to_string(),
            started_at_ms: evidence.at_ms,
            completed_at_ms: evidence.completed_at_ms,
            wall_clock_ms: evidence.wall_clock_ms,
            model_id: evidence.model_id,
            token_usage: evidence.token_usage,
            disposition: review_disposition(&evidence.tool_calls),
            tool_calls: evidence.tool_calls,
            source: evidence.source.clone(),
            venues: vec!["lnmarkets".to_string()],
            strategies: vec![
                "funding_carry".to_string(),
                "rebalance_to_target".to_string(),
                "threshold_swing".to_string(),
            ],
        };
        let turn = SoakReviewTurn {
            at_ms: evidence.at_ms,
            transcript_label: evidence.source.transcript_label(),
            source: evidence.source,
            reasoning_note_present: evidence.reasoning_note_present,
            strategy_card_updates: evidence.tracked_tool_calls,
            tokens_used: evidence.tokens_used,
        };
        let cost_recorded = record_portfolio_review_evidence(session_id, cost_record, cx);
        let soak_recorded = record_signet_soak_review_turn(session_id, turn, cx);
        cost_recorded && soak_recorded
    }
}

fn review_disposition(tool_calls: &[ReviewToolCall]) -> ReviewDisposition {
    let mut parameter_change = false;
    let mut intent = false;
    let mut halt_response = false;
    for tool_call in tool_calls {
        if tool_call.name != "lnmarkets_strategy" {
            continue;
        }
        let input = tool_call.input.get("value").unwrap_or(&tool_call.input);
        match input.get("action").and_then(serde_json::Value::as_str) {
            Some("adjust") => parameter_change = true,
            Some("start") => intent = true,
            Some("halt") => halt_response = true,
            _ => {}
        }
    }
    let mut kinds = Vec::new();
    if parameter_change {
        kinds.push(ReviewInterventionKind::ParameterChange);
    }
    if intent {
        kinds.push(ReviewInterventionKind::Intent);
    }
    if halt_response {
        kinds.push(ReviewInterventionKind::HaltResponse);
    }
    if kinds.is_empty() {
        ReviewDisposition::NoChange
    } else {
        ReviewDisposition::Intervention { kinds }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategy_actions_classify_interventions_without_treating_reads_as_changes() {
        let calls = [
            ReviewToolCall {
                name: "lnmarkets_strategy".to_string(),
                input: serde_json::json!({"type": "json", "value": {"action": "status"}}),
            },
            ReviewToolCall {
                name: "lnmarkets_strategy".to_string(),
                input: serde_json::json!({"type": "json", "value": {"action": "adjust"}}),
            },
            ReviewToolCall {
                name: "lnmarkets_strategy".to_string(),
                input: serde_json::json!({"type": "json", "value": {"action": "halt"}}),
            },
        ];
        assert_eq!(
            review_disposition(&calls),
            ReviewDisposition::Intervention {
                kinds: vec![
                    ReviewInterventionKind::ParameterChange,
                    ReviewInterventionKind::HaltResponse,
                ],
            }
        );
        assert_eq!(review_disposition(&calls[..1]), ReviewDisposition::NoChange);
    }
}
