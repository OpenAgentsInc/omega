use std::sync::Arc;

use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, SharedString, Window, px};
use trading_ledger::{LedgerAccount, LedgerEntry, LedgerEntryKind, LedgerQuery, LedgerStore};
use ui::{MarketDirection, MarketTokens, Table, Tooltip, prelude::*};

use crate::format::format_wall_clock;
use crate::portfolio_accounting::{format_asset_amount, number_cell, text_cell};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerChainState {
    Verified,
    SequenceGap,
    HashMismatch,
}

impl LedgerChainState {
    fn glyph(self) -> &'static str {
        match self {
            Self::Verified => "✓",
            Self::SequenceGap | Self::HashMismatch => "!",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::SequenceGap => "gap",
            Self::HashMismatch => "mismatch",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LedgerBrowserData {
    pub entries: Vec<LedgerEntry>,
    pub chain_state: LedgerChainState,
    pub strategy_filter: Option<SharedString>,
}

impl LedgerBrowserData {
    pub fn from_store(store: &LedgerStore, query: &LedgerQuery) -> anyhow::Result<Self> {
        store.verify()?;
        Ok(Self {
            entries: store.entries(query)?,
            chain_state: LedgerChainState::Verified,
            strategy_filter: query.strategy_id.clone().map(Into::into),
        })
    }

    fn visible_entries(&self) -> Vec<LedgerEntry> {
        self.entries
            .iter()
            .filter(|entry| {
                self.strategy_filter
                    .as_ref()
                    .is_none_or(|strategy| entry.strategy_id == strategy.as_ref())
            })
            .cloned()
            .collect()
    }
}

fn entry_kind_label(kind: &LedgerEntryKind) -> &'static str {
    match kind {
        LedgerEntryKind::Order => "order",
        LedgerEntryKind::Cancel => "cancel",
        LedgerEntryKind::Fill => "fill",
        LedgerEntryKind::Fee => "fee",
        LedgerEntryKind::FundingSettlement => "funding",
        LedgerEntryKind::Deposit => "deposit",
        LedgerEntryKind::Withdrawal => "withdrawal",
        LedgerEntryKind::BalanceAdjustment => "adjustment",
        LedgerEntryKind::ReconciliationMismatch(_) => "reconciliation",
    }
}

fn account_label(account: &LedgerAccount) -> SharedString {
    match account {
        LedgerAccount::VenueBalance { venue } => format!("venue:{venue}").into(),
        LedgerAccount::MarketParticipant { role, participant } => {
            format!("{role}:{participant}").into()
        }
        LedgerAccount::TradingProfit => "trading profit".into(),
        LedgerAccount::FeeExpense => "fee expense".into(),
        LedgerAccount::FundingIncome => "funding income".into(),
        LedgerAccount::External => "external".into(),
        LedgerAccount::BalanceAdjustment => "balance adjustment".into(),
    }
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(10).collect()
}

type LedgerSelectHandler = Arc<dyn Fn(u64, &mut Window, &mut App) + 'static>;

#[derive(IntoElement, RegisterComponent)]
pub struct LedgerBrowser {
    data: LedgerBrowserData,
    selected_sequence: Option<u64>,
    on_select: Option<LedgerSelectHandler>,
    tokens: Option<MarketTokens>,
}

impl LedgerBrowser {
    pub fn new(data: LedgerBrowserData) -> Self {
        Self {
            data,
            selected_sequence: None,
            on_select: None,
            tokens: None,
        }
    }

    pub fn selected_sequence(mut self, sequence: u64) -> Self {
        self.selected_sequence = Some(sequence);
        self
    }

    pub fn on_select(mut self, handler: impl Fn(u64, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Arc::new(handler));
        self
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for LedgerBrowser {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let entries = self.data.visible_entries();
        let selected = self
            .selected_sequence
            .and_then(|sequence| entries.iter().find(|entry| entry.sequence == sequence))
            .cloned();
        let entries = Arc::new(entries);
        let row_count = entries.len();
        let handler = self.on_select;
        let table = Table::new(7)
            .header(
                [
                    "Sequence", "Time", "Strategy", "Kind", "Event", "Previous", "Hash",
                ]
                .into_iter()
                .map(text_cell)
                .collect(),
            )
            .uniform_list(
                "portfolio-ledger-entries",
                row_count,
                move |range, _, cx| {
                    range
                        .filter_map(|index| entries.get(index))
                        .map(|entry| {
                            let sequence = entry.sequence;
                            let handler = handler.clone();
                            vec![
                                div()
                                    .id(("ledger-sequence", sequence))
                                    .cursor_pointer()
                                    .font(ui::market_number_font(cx))
                                    .text_size(px(11.0))
                                    .text_color(tokens.up)
                                    .child(format!("#{}", entry.sequence))
                                    .when_some(handler, move |this, handler| {
                                        this.on_click(move |_, window, cx| {
                                            handler(sequence, window, cx)
                                        })
                                    })
                                    .into_any_element(),
                                text_cell(format_wall_clock(entry.occurred_at_ms)),
                                text_cell(entry.strategy_id.clone()),
                                text_cell(entry_kind_label(&entry.kind)),
                                text_cell(entry.event_id.clone()),
                                number_cell(short_hash(&entry.previous_hash), tokens.muted, cx),
                                number_cell(short_hash(&entry.entry_hash), tokens.text, cx),
                            ]
                        })
                        .collect()
                },
            );
        let chain_color = if self.data.chain_state == LedgerChainState::Verified {
            tokens.up
        } else {
            tokens.down
        };
        let chain = h_flex()
            .id("ledger-chain-state")
            .gap_1()
            .child(number_cell(self.data.chain_state.glyph(), chain_color, cx))
            .child(
                Label::new("Chain")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .tooltip(Tooltip::text(self.data.chain_state.tooltip()));
        let filter = self
            .data
            .strategy_filter
            .unwrap_or_else(|| "all strategies".into());
        v_flex()
            .debug_selector(|| "command_center.ledger_browser".into())
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new(filter).size(LabelSize::Small))
                    .child(chain),
            )
            .child(
                div()
                    .h(px(220.0))
                    .border_1()
                    .border_color(tokens.grid)
                    .bg(tokens.surface)
                    .child(table),
            )
            .when_some(selected, |this, entry| {
                let posting_count = entry.postings.len();
                let mut postings = Table::new(3).header(
                    ["Account", "Amount", "Asset"]
                        .into_iter()
                        .map(text_cell)
                        .collect(),
                );
                for posting in entry.postings {
                    let direction = MarketDirection::of_i64(posting.amount);
                    postings = postings.row(vec![
                        text_cell(account_label(&posting.account)),
                        number_cell(
                            format!(
                                "{} {}",
                                direction.glyph(),
                                format_asset_amount(posting.amount, posting.asset.as_str())
                            ),
                            tokens.direction_color(direction),
                            cx,
                        ),
                        text_cell(posting.asset.as_str().to_owned()),
                    ]);
                }
                this.child(
                    v_flex()
                        .gap_1()
                        .child(
                            Label::new(format!(
                                "Entry #{} · {} postings",
                                entry.sequence, posting_count
                            ))
                            .size(LabelSize::Small),
                        )
                        .child(postings),
                )
            })
    }
}

fn demo_entry(sequence: u64, kind: LedgerEntryKind, strategy: &str, amount: i64) -> LedgerEntry {
    LedgerEntry {
        sequence,
        event_id: format!("event-{sequence:03}"),
        occurred_at_ms: 1_754_700_000_000 + sequence as i64 * 10_000,
        strategy_id: strategy.to_owned(),
        kind,
        postings: vec![
            trading_ledger::LedgerPosting::sats(
                LedgerAccount::VenueBalance {
                    venue: "lnmarkets".into(),
                },
                amount,
            ),
            trading_ledger::LedgerPosting::sats(LedgerAccount::TradingProfit, -amount),
        ],
        metadata: Default::default(),
        previous_hash: format!("{:064x}", sequence.saturating_sub(1)),
        entry_hash: format!("{sequence:064x}"),
    }
}

fn demo_ledger() -> LedgerBrowserData {
    LedgerBrowserData {
        entries: vec![
            demo_entry(1, LedgerEntryKind::Fill, "funding-carry", 4_200),
            demo_entry(2, LedgerEntryKind::Fee, "funding-carry", -180),
            demo_entry(3, LedgerEntryKind::FundingSettlement, "funding-carry", 950),
            demo_entry(4, LedgerEntryKind::Fill, "threshold-swing", -2_100),
        ],
        chain_state: LedgerChainState::Verified,
        strategy_filter: None,
    }
}

impl Component for LedgerBrowser {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        "Verified hash-chain entries with strategy filtering and posting drill-down."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Ledger browser",
                vec![single_example(
                    "Entry sequence and double-entry posting detail",
                    LedgerBrowser::new(demo_ledger())
                        .selected_sequence(2)
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Chain and posting state survives without hue",
                    LedgerBrowser::new(demo_ledger())
                        .selected_sequence(3)
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
    fn strategy_filter_limits_entries_without_changing_the_chain_state() {
        let mut data = demo_ledger();
        data.strategy_filter = Some("funding-carry".into());
        assert_eq!(data.visible_entries().len(), 3);
        assert_eq!(data.chain_state, LedgerChainState::Verified);
    }

    #[test]
    fn selected_entry_postings_balance() {
        let data = demo_ledger();
        let entry = &data.entries[1];
        assert_eq!(
            entry
                .postings
                .iter()
                .map(|posting| posting.amount)
                .sum::<i64>(),
            0
        );
    }
}
