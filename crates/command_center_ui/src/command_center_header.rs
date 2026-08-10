use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, FontWeight, Window};
use trading_ledger::{LedgerQuery, LedgerStore, ProfitReport};
use ui::prelude::*;

use crate::format::{format_sats, format_signed_sats, signed_color};

const DAY_MS: i64 = 86_400_000;

/// The value-typed input for the command-center header: what the portfolio
/// is worth and how the machine is doing, independent of any venue.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PortfolioSummary {
    pub portfolio_value_sats: i64,
    pub pnl_today_sats: i64,
    pub pnl_30d_sats: i64,
    /// Worst drawdown over the 30-day window, as a non-positive number.
    pub max_drawdown_sats: i64,
    pub active_strategy_count: usize,
}

impl PortfolioSummary {
    /// Builds the summary from already-computed ledger profit reports.
    pub fn from_reports(
        portfolio_value_sats: i64,
        today: &ProfitReport,
        thirty_day: &ProfitReport,
        active_strategy_count: usize,
    ) -> Self {
        Self {
            portfolio_value_sats,
            pnl_today_sats: today.total_profit_sats,
            pnl_30d_sats: thirty_day.total_profit_sats,
            max_drawdown_sats: thirty_day.worst_drawdown_sats,
            active_strategy_count,
        }
    }

    /// Live adapter: reads venue balances and trailing 24h/30d profit
    /// reports straight from the trading ledger.
    pub fn from_ledger(
        ledger: &LedgerStore,
        venues: &[&str],
        active_strategy_count: usize,
        now_ms: i64,
    ) -> anyhow::Result<Self> {
        let mut portfolio_value_sats = 0_i64;
        for venue in venues {
            portfolio_value_sats =
                portfolio_value_sats.saturating_add(ledger.venue_balance(venue)?);
        }
        let today = ledger.profit_report(&LedgerQuery {
            from_ms: Some((now_ms - DAY_MS).max(0)),
            ..LedgerQuery::default()
        })?;
        let thirty_day = ledger.profit_report(&LedgerQuery {
            from_ms: Some((now_ms - 30 * DAY_MS).max(0)),
            ..LedgerQuery::default()
        })?;
        Ok(Self::from_reports(
            portfolio_value_sats,
            &today,
            &thirty_day,
            active_strategy_count,
        ))
    }
}

/// The dense top strip of the command center: portfolio value, today/30d
/// PnL, max drawdown, and the active-strategy count.
#[derive(IntoElement, RegisterComponent)]
pub struct CommandCenterHeader {
    summary: PortfolioSummary,
}

impl CommandCenterHeader {
    pub fn new(summary: PortfolioSummary) -> Self {
        Self { summary }
    }
}

fn stat_cell(label: &'static str, value: String, color: Color) -> AnyElement {
    v_flex()
        .gap_0p5()
        .min_w_24()
        .child(
            Label::new(label)
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(Label::new(value).color(color)),
        )
        .into_any_element()
}

impl RenderOnce for CommandCenterHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let summary = self.summary;
        h_flex()
            .id("command-center-header")
            .debug_selector(|| "command_center.header".into())
            .w_full()
            .gap_6()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .bg(cx.theme().colors().surface_background)
            .child(stat_cell(
                "Portfolio",
                format_sats(summary.portfolio_value_sats),
                Color::Default,
            ))
            .child(stat_cell(
                "PnL today",
                format_signed_sats(summary.pnl_today_sats),
                signed_color(summary.pnl_today_sats, cx),
            ))
            .child(stat_cell(
                "PnL 30d",
                format_signed_sats(summary.pnl_30d_sats),
                signed_color(summary.pnl_30d_sats, cx),
            ))
            .child(stat_cell(
                "Max drawdown",
                format_sats(summary.max_drawdown_sats),
                if summary.max_drawdown_sats < 0 {
                    signed_color(summary.max_drawdown_sats, cx)
                } else {
                    Color::Muted
                },
            ))
            .child(stat_cell(
                "Active strategies",
                summary.active_strategy_count.to_string(),
                Color::Default,
            ))
    }
}

impl Component for CommandCenterHeader {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        "Command-center top strip: portfolio value, today/30d PnL, max drawdown, and active strategies from the trading ledger."
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        v_flex()
            .gap_6()
            .child(
                example_group_with_title(
                    "Command-center header",
                    vec![
                        single_example(
                            "Profitable",
                            CommandCenterHeader::new(PortfolioSummary {
                                portfolio_value_sats: 4_812_930,
                                pnl_today_sats: 12_480,
                                pnl_30d_sats: 291_552,
                                max_drawdown_sats: -48_211,
                                active_strategy_count: 3,
                            })
                            .into_any_element(),
                        )
                        .width(px(760.)),
                        single_example(
                            "Drawdown day",
                            CommandCenterHeader::new(PortfolioSummary {
                                portfolio_value_sats: 4_612_002,
                                pnl_today_sats: -88_412,
                                pnl_30d_sats: 90_624,
                                max_drawdown_sats: -190_928,
                                active_strategy_count: 2,
                            })
                            .into_any_element(),
                        )
                        .width(px(760.)),
                        single_example(
                            "Flat start",
                            CommandCenterHeader::new(PortfolioSummary {
                                portfolio_value_sats: 1_000_000,
                                pnl_today_sats: 0,
                                pnl_30d_sats: 0,
                                max_drawdown_sats: 0,
                                active_strategy_count: 0,
                            })
                            .into_any_element(),
                        )
                        .width(px(760.)),
                    ],
                )
                .vertical(),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_from_reports_uses_window_totals() {
        let today = ProfitReport {
            total_profit_sats: 150,
            worst_drawdown_sats: -40,
            ..ProfitReport::default()
        };
        let thirty_day = ProfitReport {
            total_profit_sats: 2_600,
            worst_drawdown_sats: -900,
            ..ProfitReport::default()
        };
        let summary = PortfolioSummary::from_reports(10_000, &today, &thirty_day, 2);
        assert_eq!(summary.portfolio_value_sats, 10_000);
        assert_eq!(summary.pnl_today_sats, 150);
        assert_eq!(summary.pnl_30d_sats, 2_600);
        assert_eq!(summary.max_drawdown_sats, -900);
        assert_eq!(summary.active_strategy_count, 2);
    }
}
