use lnmarkets_data::{FeatureSnapshot, FundingSign};
use review_accounting::ReviewCostSummary;
use serde::Serialize;
use trading_ledger::{LedgerEntry, LedgerEntryKind, ProfitReport};
use trading_mandate::{LEGACY_VENUE, MandateSnapshot};

use crate::{BacktestReport, StrategyRuntimeSnapshot};

pub const PORTFOLIO_REVIEW_SCHEMA: &str = "omega.lnmarkets.portfolio_review.v1";
pub const PORTFOLIO_REVIEW_TOKEN_BUDGET: u64 = 1_024;
const ONE_HOUR_MS: i64 = 60 * 60 * 1_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityAvailability {
    Available,
    InputOnly,
    NotImplemented,
    NotAllowed,
    DataUnavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Opportunity {
    pub name: &'static str,
    pub signal: String,
    pub cost_hurdle: &'static str,
    pub risk: &'static str,
    pub availability: OpportunityAvailability,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StrategyLimitHeadroom {
    pub strategy_id: String,
    pub realized_loss_sats: u64,
    pub daily_loss_headroom_sats: u64,
    pub orders_last_hour: u32,
    pub order_headroom: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LimitHeadroom {
    pub mandate_expires_in_ms: i64,
    pub maximum_venue_balance_sats: u64,
    pub maximum_position_usd: u64,
    pub maximum_leverage: u8,
    pub minimum_liquidation_buffer_bps: u32,
    pub by_strategy: Vec<StrategyLimitHeadroom>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PortfolioReview {
    pub schema: &'static str,
    pub generated_at_ms: i64,
    pub trigger: String,
    pub feature_status: String,
    pub features: Option<FeatureSnapshot>,
    pub daily_report: ProfitReport,
    pub mandate: MandateSnapshot,
    pub limit_headroom: Option<LimitHeadroom>,
    pub strategies: Vec<StrategyRuntimeSnapshot>,
    pub backtests: Vec<BacktestReport>,
    pub opportunities: Vec<Opportunity>,
    pub review_costs: ReviewCostSummary,
    pub review_cost_summary_line: String,
}

impl PortfolioReview {
    pub fn build(
        generated_at_ms: i64,
        trigger: impl Into<String>,
        feature_status: impl Into<String>,
        features: Option<FeatureSnapshot>,
        daily_report: ProfitReport,
        hourly_entries: &[LedgerEntry],
        mandate: MandateSnapshot,
        strategies: Vec<StrategyRuntimeSnapshot>,
        backtests: Vec<BacktestReport>,
        review_costs: ReviewCostSummary,
    ) -> Self {
        let limit_headroom = lnmarkets_mandate(&mandate).map(|active| {
            let by_strategy = active
                .allowed_strategies
                .iter()
                .map(|strategy_id| {
                    let strategy_profit = daily_report
                        .strategies
                        .iter()
                        .find(|profit| &profit.strategy_id == strategy_id);
                    let realized_loss_sats = strategy_profit
                        .map(|profit| realized_loss(profit.profit_sats))
                        .unwrap_or(0);
                    let order_count = hourly_entries
                        .iter()
                        .filter(|entry| {
                            entry.strategy_id == *strategy_id
                                && matches!(entry.kind, LedgerEntryKind::Order)
                        })
                        .count();
                    let orders_last_hour = u32::try_from(order_count).unwrap_or(u32::MAX);
                    StrategyLimitHeadroom {
                        strategy_id: strategy_id.clone(),
                        realized_loss_sats,
                        daily_loss_headroom_sats: active
                            .daily_loss_stop
                            .saturating_sub(realized_loss_sats),
                        orders_last_hour,
                        order_headroom: active.max_orders_per_hour.saturating_sub(orders_last_hour),
                    }
                })
                .collect();
            LimitHeadroom {
                mandate_expires_in_ms: active.expires_at_ms.saturating_sub(generated_at_ms),
                maximum_venue_balance_sats: active.max_venue_balance,
                maximum_position_usd: active.max_position_usd,
                maximum_leverage: active.max_leverage,
                minimum_liquidation_buffer_bps: active.min_liquidation_buffer_bps,
                by_strategy,
            }
        });
        let opportunities = opportunity_inventory(features.as_ref(), &mandate);
        let review_cost_summary_line = review_cost_summary_line(&review_costs);
        Self {
            schema: PORTFOLIO_REVIEW_SCHEMA,
            generated_at_ms,
            trigger: trigger.into(),
            feature_status: feature_status.into(),
            features,
            daily_report,
            mandate,
            limit_headroom,
            strategies,
            backtests,
            opportunities,
            review_costs,
            review_cost_summary_line,
        }
    }

    pub fn instruction(&self) -> Result<String, serde_json::Error> {
        let context = serde_json::to_string(self)?;
        Ok(format!(
            "Run one bounded LN Markets portfolio review. The local context below already contains the feature and ledger reads; do not call remote market or account tools.\n\n{context}\n\nRank every opportunity against the supplied features, costs, risks, active strategy states, and limit headroom. Before any strategy action or reasoning conclusion, call lnmarkets_prediction exactly once with action=record, a fixed decision ID, confidence, horizon, and observation references. If the review makes no strategy change, record a flat prediction linked to an explicit no-change decision; abstention without a scored prediction is invalid. The strategy engine places orders; never place a raw order or swap in this turn. You may then call lnmarkets_strategy at most once to start, adjust, or halt a supported strategy, and only within the active mandate. A start or adjust must use the returned prediction_id and the same decision_id. The mandate cannot be widened from this turn. Then write one reasoning note of at most 120 words and end the turn. Include a daily report with ledger profit, fees paid, funding collected, worst drawdown, remaining limit headroom, and the supplied 24-hour review supervision cost summary. If data or authority is missing, record a flat prediction and make no change."
        ))
    }
}

fn review_cost_summary_line(summary: &ReviewCostSummary) -> String {
    let average_review_tokens = summary
        .cost_per_review
        .as_ref()
        .map_or(0, |cost| cost.total_tokens);
    let average_intervention_tokens = summary
        .cost_per_intervention
        .as_ref()
        .map_or(0, |cost| cost.total_tokens);
    let false_wakeup_rate = summary.false_wakeup_rate_bps.map_or_else(
        || "n/a".to_string(),
        |rate| format!("{}.{:02}%", rate / 100, rate % 100),
    );
    format!(
        "Review supervision (24h): {} reviews, {} interventions, {} tokens/review, {} tokens/intervention, {false_wakeup_rate} false event wakeups",
        summary.review_count,
        summary.intervention_count,
        average_review_tokens,
        average_intervention_tokens,
    )
}

fn realized_loss(profit_sats: i64) -> u64 {
    if profit_sats >= 0 {
        return 0;
    }
    profit_sats
        .checked_neg()
        .and_then(|loss| u64::try_from(loss).ok())
        .unwrap_or(u64::MAX)
}

fn opportunity_inventory(
    features: Option<&FeatureSnapshot>,
    mandate: &MandateSnapshot,
) -> Vec<Opportunity> {
    let allowed = |strategy_id: &str| {
        lnmarkets_mandate(mandate)
            .is_some_and(|active| active.allowed_strategies.contains(strategy_id))
    };
    let funding_availability = if !allowed("funding_carry") {
        OpportunityAvailability::NotAllowed
    } else if features.is_none() {
        OpportunityAvailability::DataUnavailable
    } else {
        OpportunityAvailability::Available
    };
    let rebalance_availability = if !allowed("rebalance_to_target") {
        OpportunityAvailability::NotAllowed
    } else if features
        .and_then(|snapshot| snapshot.account_drift.as_ref())
        .is_none()
    {
        OpportunityAvailability::DataUnavailable
    } else {
        OpportunityAvailability::Available
    };
    let funding_signal = features.map_or_else(
        || "funding features unavailable".to_string(),
        |snapshot| match snapshot.funding.sign {
            FundingSign::Positive => format!(
                "positive funding; current {:?}, EMA {:?}",
                snapshot.funding.current_rate, snapshot.funding.ema
            ),
            FundingSign::Neutral => "neutral funding".to_string(),
            FundingSign::Negative => format!(
                "negative funding; current {:?}, EMA {:?}",
                snapshot.funding.current_rate, snapshot.funding.ema
            ),
        },
    );
    let rebalance_signal = features
        .and_then(|snapshot| snapshot.account_drift.as_ref())
        .map(|drift| {
            format!(
                "BTC weight {:.4}, target {:.4}, drift {:.4}",
                drift.current_btc_weight, drift.target_btc_weight, drift.drift
            )
        })
        .unwrap_or_else(|| "account drift unavailable".to_string());
    vec![
        Opportunity {
            name: "funding_carry",
            signal: funding_signal,
            cost_hurdle: "entry and exit taker fees plus adverse funding",
            risk: "liquidation during rallies and funding sign flips",
            availability: funding_availability,
        },
        Opportunity {
            name: "rebalance_to_target",
            signal: rebalance_signal,
            cost_hurdle: "measured round-trip cost plus configured margin",
            risk: "opportunity cost while holding synthetic USD",
            availability: rebalance_availability,
        },
        Opportunity {
            name: "threshold_swing",
            signal: features
                .and_then(|snapshot| snapshot.volatility.one_hour)
                .map(|volatility| format!("one-hour realized volatility {volatility:.6}"))
                .unwrap_or_else(|| "volatility features unavailable".to_string()),
            cost_hurdle: "round-trip taker cost plus adverse trend",
            risk: "persistent trend against a bounded mean-reversion position",
            availability: OpportunityAvailability::NotImplemented,
        },
        Opportunity {
            name: "ladder_aware_sizing",
            signal: features.map_or_else(
                || "liquidity features unavailable".to_string(),
                |snapshot| {
                    format!(
                        "spread {:?} bps, bid depth {:?}, ask depth {:?}",
                        snapshot.liquidity.spread_bps,
                        snapshot.liquidity.bid_depth,
                        snapshot.liquidity.ask_depth
                    )
                },
            ),
            cost_hurdle: "none; this is an execution input",
            risk: "stale ladder data",
            availability: if features.is_some() {
                OpportunityAvailability::InputOnly
            } else {
                OpportunityAvailability::DataUnavailable
            },
        },
        Opportunity {
            name: "cross_venue_rebalancing",
            signal: "requires the accepted Immortal price-feed packet".to_string(),
            cost_hurdle: "both venues' fees and settlement timing",
            risk: "cross-venue settlement delay",
            availability: OpportunityAvailability::NotImplemented,
        },
        Opportunity {
            name: "immortal_spread_tuning",
            signal: "requires the accepted Immortal price-feed packet".to_string(),
            cost_hurdle: "none; quote improvement only",
            risk: "stale price feed can misprice quotes",
            availability: OpportunityAvailability::NotImplemented,
        },
    ]
}

fn lnmarkets_mandate(mandate: &MandateSnapshot) -> Option<&trading_mandate::TradingMandate> {
    mandate
        .mandates
        .iter()
        .find(|active| active.venue == LEGACY_VENUE)
}

pub fn hourly_start(now_ms: i64) -> i64 {
    now_ms.saturating_sub(ONE_HOUR_MS).max(0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use lnmarkets_data::{
        AccountDriftFeatures, FundingFeatures, IndexFeatures, LiquidityFeatures, VolatilityFeatures,
    };
    use trading_ledger::{LedgerAccount, LedgerEntry, LedgerEntryKind, LedgerPosting};
    use trading_mandate::{ReviewCadence, TradingMandate, TradingNetwork};

    use super::*;

    fn mandate() -> MandateSnapshot {
        MandateSnapshot {
            revision: 3,
            mandates: vec![TradingMandate {
                venue: LEGACY_VENUE.into(),
                network: TradingNetwork::Signet,
                collateral_asset: trading_mandate::AssetId::sats(),
                objective: "maximize ledger profit in sats".into(),
                max_venue_balance: 100_000,
                max_position_usd: 50,
                max_leverage: 2,
                daily_loss_stop: 500,
                max_orders_per_hour: 4,
                min_liquidation_buffer_bps: 2_000,
                allowed_strategies: BTreeSet::from([
                    "funding_carry".into(),
                    "rebalance_to_target".into(),
                ]),
                review_cadence: ReviewCadence::Interval { seconds: 300 },
                expires_at_ms: 20_000,
            }],
        }
    }

    fn features() -> FeatureSnapshot {
        FeatureSnapshot {
            schema: "omega.lnmarkets.features.v1".into(),
            as_of_ms: Some(9_900),
            index: IndexFeatures {
                current_price: Some(60_005.0),
                one_hour_move: Some(-0.01),
                six_hours_move: Some(0.02),
                one_day_move: Some(0.03),
                price_points: 100,
            },
            volatility: VolatilityFeatures {
                one_hour: Some(0.02),
                six_hours: Some(0.03),
                one_day: Some(0.04),
                price_points: 100,
            },
            funding: FundingFeatures {
                current_rate: Some(0.0001),
                ema: Some(0.00008),
                sign: FundingSign::Positive,
                sign_flipped_at_ms: None,
                measurement_started_at_ms: Some(1),
                measurement_ended_at_ms: Some(8),
                samples: 8,
            },
            liquidity: LiquidityFeatures {
                best_bid: Some(60_000.0),
                best_ask: Some(60_010.0),
                spread: Some(10.0),
                spread_bps: Some(1.67),
                bid_depth: Some(500.0),
                ask_depth: Some(400.0),
                tier_count: 5,
            },
            account_drift: Some(AccountDriftFeatures {
                btc_value_usd: 90.0,
                synthetic_usd: 10.0,
                current_btc_weight: 0.9,
                target_btc_weight: 0.5,
                drift: 0.4,
            }),
        }
    }

    fn order(strategy_id: &str) -> LedgerEntry {
        LedgerEntry {
            sequence: 1,
            event_id: format!("order-{strategy_id}"),
            occurred_at_ms: 9_000,
            strategy_id: strategy_id.into(),
            kind: LedgerEntryKind::Order,
            postings: vec![
                LedgerPosting::sats(LedgerAccount::FeeExpense, 1),
                LedgerPosting::sats(LedgerAccount::TradingProfit, -1),
            ],
            metadata: serde_json::json!({}),
            previous_hash: "00".into(),
            entry_hash: "01".into(),
        }
    }

    #[test]
    fn review_contains_local_context_inventory_and_exact_headroom() {
        let report = ProfitReport {
            total_profit_sats: -125,
            strategies: vec![trading_ledger::StrategyProfit {
                strategy_id: "funding_carry".into(),
                profit_sats: -125,
                ..Default::default()
            }],
            ..Default::default()
        };
        let review = PortfolioReview::build(
            10_000,
            "scheduled review",
            "ready",
            Some(features()),
            report,
            &[order("funding_carry")],
            mandate(),
            Vec::new(),
            Vec::new(),
            ReviewCostSummary::default(),
        );
        let headroom = review.limit_headroom.as_ref().expect("limit headroom");
        let funding = headroom
            .by_strategy
            .iter()
            .find(|item| item.strategy_id == "funding_carry")
            .expect("funding headroom");
        assert_eq!(funding.daily_loss_headroom_sats, 375);
        assert_eq!(funding.order_headroom, 3);
        assert_eq!(review.opportunities.len(), 6);
        assert!(
            review
                .review_cost_summary_line
                .starts_with("Review supervision (24h):")
        );
        assert_eq!(
            review.opportunities[0].availability,
            OpportunityAvailability::Available
        );
    }

    #[test]
    fn instruction_is_short_bounded_and_forbids_direct_orders() {
        let review = PortfolioReview::build(
            10_000,
            "event: strategy halt",
            "ready",
            Some(features()),
            ProfitReport::default(),
            &[],
            mandate(),
            Vec::new(),
            Vec::new(),
            ReviewCostSummary::default(),
        );
        let instruction = review.instruction().expect("serialize review");
        assert!(instruction.contains(PORTFOLIO_REVIEW_SCHEMA));
        assert!(instruction.contains("at most once"));
        assert!(instruction.contains("never place a raw order or swap"));
        assert!(instruction.contains("at most 120 words"));
        assert!(instruction.contains("review supervision cost summary"));
        assert!(!instruction.contains("lnmarkets_market_data"));
    }
}
