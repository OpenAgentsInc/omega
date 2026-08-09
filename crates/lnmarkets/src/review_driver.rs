use gpui::App;
use plugin_api::{ReviewCadence, ReviewTurnEvidence, ReviewTurnOutcome, SessionReviewDriver};

use crate::{
    PORTFOLIO_REVIEW_TOKEN_BUDGET, ReviewTurnOutcome as PortfolioReviewTurnOutcome, SoakReviewTurn,
    WakeupSource, acknowledge_portfolio_wakeup, pending_portfolio_wakeup, portfolio_review_cadence,
    portfolio_review_instruction, record_portfolio_review_turn, record_signet_soak_review_turn,
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
        let turn = SoakReviewTurn {
            at_ms: evidence.at_ms,
            transcript_label: evidence.source.transcript_label(),
            source: evidence.source,
            reasoning_note_present: evidence.reasoning_note_present,
            strategy_card_updates: evidence.tracked_tool_calls,
            tokens_used: evidence.tokens_used,
        };
        record_signet_soak_review_turn(session_id, turn, cx)
    }
}
