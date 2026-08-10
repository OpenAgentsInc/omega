use std::{collections::BTreeMap, sync::Arc};

use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, SharedString, Window, px};
use trading_ledger::{
    Counterparty, CounterpartyExposure, LedgerAccount, LedgerEntry, LedgerEntryKind, ProfitReport,
};
use ui::{MarketDirection, MarketTokens, Table, market_number_font, prelude::*};

use crate::format::format_wall_clock;

pub(crate) fn text_cell(text: impl Into<SharedString>) -> AnyElement {
    div()
        .text_size(px(11.0))
        .child(text.into())
        .into_any_element()
}

pub(crate) fn number_cell(
    text: impl Into<SharedString>,
    color: gpui::Hsla,
    cx: &App,
) -> AnyElement {
    div()
        .font(market_number_font(cx))
        .text_size(px(11.0))
        .text_color(color)
        .child(text.into())
        .into_any_element()
}

pub(crate) fn format_asset_amount(amount: i64, asset: &str) -> String {
    let sign = if amount < 0 { "-" } else { "" };
    format!(
        "{sign}{} {asset}",
        ui::format_grouped_u64(amount.unsigned_abs())
    )
}

fn counterparty_name(counterparty: &Counterparty) -> SharedString {
    match counterparty {
        Counterparty::Venue { venue } => venue.clone().into(),
        Counterparty::Provider { provider } => provider.clone().into(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BalanceMode {
    Unified,
    Isolated,
    VenueManaged,
    Unknown,
}

impl BalanceMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unified => "unified",
            Self::Isolated => "isolated",
            Self::VenueManaged => "venue-managed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BalanceRow {
    pub venue: SharedString,
    pub asset: SharedString,
    pub balance: i64,
    pub unrealized: i64,
    pub in_flight: i64,
    pub counterparty_exposure: i64,
    pub usable_margin: Option<i64>,
    pub mode: BalanceMode,
}

impl BalanceRow {
    pub fn from_exposure(
        exposure: &CounterpartyExposure,
        usable_margin: Option<i64>,
        mode: BalanceMode,
    ) -> Self {
        Self {
            venue: counterparty_name(&exposure.counterparty),
            asset: exposure.asset.as_str().to_owned().into(),
            balance: exposure.balance_held,
            unrealized: exposure.unrealized_claims,
            in_flight: exposure.in_flight_transfers,
            counterparty_exposure: exposure.counterparty_exposure,
            usable_margin,
            mode,
        }
    }
}

#[derive(IntoElement, RegisterComponent)]
pub struct BalancesTable {
    rows: Vec<BalanceRow>,
    tokens: Option<MarketTokens>,
}

impl BalancesTable {
    pub fn new(rows: Vec<BalanceRow>) -> Self {
        Self { rows, tokens: None }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for BalancesTable {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let rows = Arc::new(self.rows);
        let row_count = rows.len();
        let table = Table::new(8)
            .header(
                [
                    "Venue",
                    "Asset",
                    "Balance",
                    "Unrealized",
                    "In flight",
                    "Exposure",
                    "Usable",
                    "Mode",
                ]
                .into_iter()
                .map(text_cell)
                .collect(),
            )
            .uniform_list("portfolio-balances", row_count, move |range, _, cx| {
                range
                    .filter_map(|index| rows.get(index))
                    .map(|row| {
                        let asset = row.asset.as_ref();
                        let unrealized_direction = MarketDirection::of_i64(row.unrealized);
                        vec![
                            text_cell(row.venue.clone()),
                            text_cell(row.asset.clone()),
                            number_cell(format_asset_amount(row.balance, asset), tokens.text, cx),
                            number_cell(
                                format!(
                                    "{} {}",
                                    unrealized_direction.glyph(),
                                    format_asset_amount(row.unrealized, asset)
                                ),
                                tokens.direction_color(unrealized_direction),
                                cx,
                            ),
                            number_cell(format_asset_amount(row.in_flight, asset), tokens.text, cx),
                            number_cell(
                                format_asset_amount(row.counterparty_exposure, asset),
                                tokens.text,
                                cx,
                            ),
                            number_cell(
                                row.usable_margin.map_or_else(
                                    || "—".to_owned(),
                                    |amount| format_asset_amount(amount, asset),
                                ),
                                if row.usable_margin.is_some() {
                                    tokens.text
                                } else {
                                    tokens.muted
                                },
                                cx,
                            ),
                            text_cell(row.mode.label()),
                        ]
                    })
                    .collect()
            });
        div()
            .debug_selector(|| "command_center.balances_table".into())
            .h(px(210.0))
            .border_1()
            .border_color(tokens.grid)
            .bg(tokens.surface)
            .child(table)
    }
}

fn demo_balances() -> Vec<BalanceRow> {
    vec![
        BalanceRow {
            venue: "hyperliquid".into(),
            asset: "USDC".into(),
            balance: 125_000_00,
            unrealized: 42_500,
            in_flight: 0,
            counterparty_exposure: 125_425_00,
            usable_margin: Some(91_200_00),
            mode: BalanceMode::Unified,
        },
        BalanceRow {
            venue: "lnmarkets".into(),
            asset: "sats".into(),
            balance: 4_812_930,
            unrealized: -18_420,
            in_flight: 75_000,
            counterparty_exposure: 4_869_510,
            usable_margin: Some(3_990_000),
            mode: BalanceMode::VenueManaged,
        },
        BalanceRow {
            venue: "provider:npub1…9xq".into(),
            asset: "sats".into(),
            balance: 680_000,
            unrealized: 0,
            in_flight: 42_000,
            counterparty_exposure: 722_000,
            usable_margin: None,
            mode: BalanceMode::Unknown,
        },
    ]
}

impl Component for BalancesTable {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        "Per-venue and asset balances, claims, transfers, exposure, and mode-aware usable margin."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Balances",
                vec![single_example(
                    "Venue-neutral account modes and exact asset units",
                    BalancesTable::new(demo_balances()).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Signed glyphs preserve unrealized direction",
                    BalancesTable::new(demo_balances())
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationRow {
    pub venue: SharedString,
    pub asset: SharedString,
    pub ledger_balance: i64,
    pub engine_balance: i64,
    pub difference: i64,
    pub reconciled_at_ms: i64,
}

impl ReconciliationRow {
    pub fn is_matched(&self) -> bool {
        self.difference == 0 && self.engine_balance == self.ledger_balance
    }
}

#[derive(IntoElement, RegisterComponent)]
pub struct ReconciliationStatusTable {
    rows: Vec<ReconciliationRow>,
    tokens: Option<MarketTokens>,
}

impl ReconciliationStatusTable {
    pub fn new(rows: Vec<ReconciliationRow>) -> Self {
        Self { rows, tokens: None }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for ReconciliationStatusTable {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let rows = Arc::new(self.rows);
        let row_count = rows.len();
        let table = Table::new(7)
            .header(
                [
                    "Venue",
                    "Asset",
                    "Ledger",
                    "Engine",
                    "Difference",
                    "Checked",
                    "Chain",
                ]
                .into_iter()
                .map(text_cell)
                .collect(),
            )
            .uniform_list(
                "portfolio-reconciliation",
                row_count,
                move |range, _, cx| {
                    range
                        .filter_map(|index| rows.get(index))
                        .map(|row| {
                            let direction = MarketDirection::of_i64(row.difference);
                            let matched = row.is_matched();
                            vec![
                                text_cell(row.venue.clone()),
                                text_cell(row.asset.clone()),
                                number_cell(
                                    format_asset_amount(row.ledger_balance, row.asset.as_ref()),
                                    tokens.text,
                                    cx,
                                ),
                                number_cell(
                                    format_asset_amount(row.engine_balance, row.asset.as_ref()),
                                    tokens.text,
                                    cx,
                                ),
                                number_cell(
                                    format!(
                                        "{} {}",
                                        direction.glyph(),
                                        format_asset_amount(row.difference, row.asset.as_ref())
                                    ),
                                    tokens.direction_color(direction),
                                    cx,
                                ),
                                text_cell(format_wall_clock(row.reconciled_at_ms)),
                                number_cell(
                                    if matched { "✓" } else { "!" },
                                    if matched { tokens.up } else { tokens.down },
                                    cx,
                                ),
                            ]
                        })
                        .collect()
                },
            );
        div()
            .debug_selector(|| "command_center.reconciliation_table".into())
            .h(px(190.0))
            .border_1()
            .border_color(tokens.grid)
            .bg(tokens.surface)
            .child(table)
    }
}

fn demo_reconciliation() -> Vec<ReconciliationRow> {
    vec![
        ReconciliationRow {
            venue: "hyperliquid".into(),
            asset: "USDC".into(),
            ledger_balance: 12_500_000,
            engine_balance: 12_500_000,
            difference: 0,
            reconciled_at_ms: 1_754_700_000_000,
        },
        ReconciliationRow {
            venue: "lnmarkets".into(),
            asset: "sats".into(),
            ledger_balance: 4_812_930,
            engine_balance: 4_812_905,
            difference: -25,
            reconciled_at_ms: 1_754_699_970_000,
        },
    ]
}

impl Component for ReconciliationStatusTable {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        "Ledger-to-engine reconciliation by venue and asset, including the exact zero-difference check."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Reconciliation",
                vec![single_example(
                    "Zero and non-zero differences remain explicit",
                    ReconciliationStatusTable::new(demo_reconciliation()).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Check and warning glyphs carry verification state",
                    ReconciliationStatusTable::new(demo_reconciliation())
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeFundingRow {
    pub day_start_ms: i64,
    pub strategy: SharedString,
    pub venue: SharedString,
    pub asset: SharedString,
    pub fees_paid: i64,
    pub funding_collected: i64,
}

impl FeeFundingRow {
    pub fn net_cost(&self) -> i64 {
        self.fees_paid.saturating_sub(self.funding_collected)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FeeFundingData {
    pub rows: Vec<FeeFundingRow>,
}

fn venue_from_entry(entry: &LedgerEntry) -> SharedString {
    entry
        .postings
        .iter()
        .find_map(|posting| match &posting.account {
            LedgerAccount::VenueBalance { venue } => Some(venue.clone().into()),
            _ => None,
        })
        .unwrap_or_else(|| "external".into())
}

impl FeeFundingData {
    pub fn from_entries(entries: &[LedgerEntry]) -> Self {
        const DAY_MS: i64 = 86_400_000;
        let mut totals =
            BTreeMap::<(i64, SharedString, SharedString, SharedString), (i64, i64)>::new();
        for entry in entries {
            if !matches!(
                entry.kind,
                LedgerEntryKind::Fee | LedgerEntryKind::FundingSettlement
            ) {
                continue;
            }
            let venue = venue_from_entry(entry);
            for posting in &entry.postings {
                let (fee, funding) = match posting.account {
                    LedgerAccount::FeeExpense if posting.amount > 0 => (posting.amount, 0),
                    LedgerAccount::FundingIncome => (0, posting.amount.saturating_neg()),
                    _ => continue,
                };
                let key = (
                    entry.occurred_at_ms / DAY_MS * DAY_MS,
                    entry.strategy_id.clone().into(),
                    venue.clone(),
                    posting.asset.as_str().to_owned().into(),
                );
                let total = totals.entry(key).or_default();
                total.0 = total.0.saturating_add(fee);
                total.1 = total.1.saturating_add(funding);
            }
        }
        Self {
            rows: totals
                .into_iter()
                .map(
                    |((day_start_ms, strategy, venue, asset), (fees_paid, funding_collected))| {
                        FeeFundingRow {
                            day_start_ms,
                            strategy,
                            venue,
                            asset,
                            fees_paid,
                            funding_collected,
                        }
                    },
                )
                .collect(),
        }
    }

    pub fn from_report(
        report: &ProfitReport,
        day_start_ms: i64,
        venue: impl Into<SharedString>,
    ) -> Self {
        let venue = venue.into();
        let mut rows = Vec::new();
        for strategy in &report.strategies {
            for asset in &strategy.assets {
                rows.push(FeeFundingRow {
                    day_start_ms,
                    strategy: strategy.strategy_id.clone().into(),
                    venue: venue.clone(),
                    asset: asset.asset.as_str().to_owned().into(),
                    fees_paid: asset.fees_paid,
                    funding_collected: asset.funding_collected,
                });
            }
        }
        Self { rows }
    }

    pub fn export_csv(&self) -> String {
        let mut csv =
            String::from("day_ms,strategy,venue,asset,fees_paid,funding_collected,net_cost\n");
        for row in &self.rows {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                row.day_start_ms,
                row.strategy,
                row.venue,
                row.asset,
                row.fees_paid,
                row.funding_collected,
                row.net_cost(),
            ));
        }
        csv
    }
}

#[derive(IntoElement, RegisterComponent)]
pub struct FeeFundingBreakdown {
    data: FeeFundingData,
    tokens: Option<MarketTokens>,
}

impl FeeFundingBreakdown {
    pub fn new(data: FeeFundingData) -> Self {
        Self { data, tokens: None }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for FeeFundingBreakdown {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let rows = Arc::new(self.data.rows);
        let row_count = rows.len();
        let table = Table::new(7)
            .header(
                [
                    "Day", "Strategy", "Venue", "Asset", "Fees", "Funding", "Net cost",
                ]
                .into_iter()
                .map(text_cell)
                .collect(),
            )
            .uniform_list("portfolio-fee-funding", row_count, move |range, _, cx| {
                range
                    .filter_map(|index| rows.get(index))
                    .map(|row| {
                        let direction = MarketDirection::of_i64(row.net_cost().saturating_neg());
                        vec![
                            text_cell(format_wall_clock(row.day_start_ms)),
                            text_cell(row.strategy.clone()),
                            text_cell(row.venue.clone()),
                            text_cell(row.asset.clone()),
                            number_cell(
                                format_asset_amount(row.fees_paid, row.asset.as_ref()),
                                tokens.down,
                                cx,
                            ),
                            number_cell(
                                format_asset_amount(row.funding_collected, row.asset.as_ref()),
                                tokens.up,
                                cx,
                            ),
                            number_cell(
                                format!(
                                    "{} {}",
                                    direction.glyph(),
                                    format_asset_amount(row.net_cost(), row.asset.as_ref())
                                ),
                                tokens.direction_color(direction),
                                cx,
                            ),
                        ]
                    })
                    .collect()
            });
        div()
            .debug_selector(|| "command_center.fee_funding_breakdown".into())
            .h(px(210.0))
            .border_1()
            .border_color(tokens.grid)
            .bg(tokens.surface)
            .child(table)
    }
}

fn demo_fee_funding() -> FeeFundingData {
    FeeFundingData {
        rows: vec![
            FeeFundingRow {
                day_start_ms: 1_754_630_400_000,
                strategy: "funding-carry".into(),
                venue: "hyperliquid".into(),
                asset: "USDC".into(),
                fees_paid: 8_420,
                funding_collected: 21_850,
            },
            FeeFundingRow {
                day_start_ms: 1_754_630_400_000,
                strategy: "threshold-swing".into(),
                venue: "lnmarkets".into(),
                asset: "sats".into(),
                fees_paid: 1_140,
                funding_collected: 0,
            },
        ],
    }
}

impl Component for FeeFundingBreakdown {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        "Daily strategy and venue fee/funding attribution with exportable exact-unit totals."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Fees and funding",
                vec![single_example(
                    "Daily attributed costs and collections",
                    FeeFundingBreakdown::new(demo_fee_funding()).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Signs and glyphs repeat cost direction",
                    FeeFundingBreakdown::new(demo_fee_funding())
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balances_keep_mode_aware_margin_distinct_from_exposure() {
        let rows = demo_balances();
        assert_ne!(rows[0].usable_margin, Some(rows[0].counterparty_exposure));
        assert_eq!(rows[2].mode, BalanceMode::Unknown);
        assert_eq!(rows[2].usable_margin, None);
    }

    #[test]
    fn reconciliation_requires_the_exact_zero_identity() {
        let rows = demo_reconciliation();
        assert!(rows[0].is_matched());
        assert!(!rows[1].is_matched());
    }

    #[test]
    fn cost_report_export_carries_every_attribution_dimension() {
        let data = demo_fee_funding();
        let export = data.export_csv();
        assert!(export.contains("day_ms,strategy,venue,asset"));
        assert!(export.contains("funding-carry,hyperliquid,USDC"));
        assert!(export.contains("threshold-swing,lnmarkets,sats"));
    }
}
