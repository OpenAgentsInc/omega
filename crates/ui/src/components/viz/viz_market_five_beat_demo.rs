use documented::Documented;
use gpui::{ClickEvent, px};

use crate::components::viz::viz_market_chat_demo::{agent_message, user_message};
use crate::components::viz::viz_rfq::demo_quote_set;
use crate::components::viz::viz_swap::demo_card;
use crate::components::viz::{
    NetworkCard, RfqComparisonCard, SwapStage, VizChip, VizChipTone, VizProgressRail,
    demo_shape_fixture,
};
use crate::prelude::*;

/// The number of beats in the walkthrough.
pub const BEAT_COUNT: usize = 5;

const BEAT_TITLES: [&str; BEAT_COUNT] = [
    "command center",
    "carry comparison",
    "order · confirm-gated",
    "ledger receipt",
    "swap lane",
];

/// The title of one beat; out-of-range indices clamp to the last beat, so a
/// stale stepper state can never panic the harness.
pub fn beat_title(beat: usize) -> &'static str {
    BEAT_TITLES[beat.min(BEAT_COUNT - 1)]
}

/// A dashed placeholder frame for a surface whose real components land in a
/// sibling lane of the #284 sprint. The marking is deliberate and loud: a
/// rehearsal audience must never mistake a stand-in for a shipped surface.
fn placeholder_frame(
    title: &'static str,
    note: &'static str,
    rows: Vec<gpui::AnyElement>,
    cx: &App,
) -> gpui::AnyElement {
    let colors = cx.theme().colors();
    v_flex()
        .w(px(420.))
        .my_1p5()
        .rounded_md()
        .border_1()
        .border_dashed()
        .border_color(colors.border)
        .bg(colors.editor_background)
        .overflow_hidden()
        .child(
            h_flex()
                .h_8()
                .w_full()
                .px_2()
                .justify_between()
                .bg(colors
                    .element_background
                    .blend(colors.editor_foreground.opacity(0.025)))
                .child(Label::new(title).size(LabelSize::Small))
                .child(
                    Label::new("placeholder")
                        .size(LabelSize::XSmall)
                        .color(Color::Warning),
                ),
        )
        .child(v_flex().px_3().py_2().gap_1().children(rows))
        .child(
            div()
                .px_3()
                .pb_2()
                .child(Label::new(note).size(LabelSize::XSmall).color(Color::Muted)),
        )
        .into_any_element()
}

fn fixture_row(label: &'static str, value: &'static str, cx: &App) -> gpui::AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .gap_2()
        .child(Label::new(label).size(LabelSize::Small).color(Color::Muted))
        .child(Label::new(value).size(LabelSize::Small).buffer_font(cx))
        .into_any_element()
}

/// The rehearsal walkthrough for tomorrow's market demo, named for its
/// organizing question: where should the money sit, and can Omega move it
/// there with receipts? Five beats — command center, carry comparison, a
/// confirm-gated order, the ledger receipt, and the RFQ/swap network lane —
/// advance on click, every value is a demo fixture, and nothing here places
/// an order or touches a live system, so the demo can be rehearsed and shown
/// even while the live lanes are busy.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct WhereShouldTheMoneySit {
    beat: usize,
}

impl WhereShouldTheMoneySit {
    pub fn new() -> Self {
        Self { beat: 0 }
    }

    pub fn beat(mut self, beat: usize) -> Self {
        self.beat = beat.min(BEAT_COUNT - 1);
        self
    }
}

impl Default for WhereShouldTheMoneySit {
    fn default() -> Self {
        Self::new()
    }
}

fn beat_command_center(cx: &App) -> Vec<gpui::AnyElement> {
    vec![
        user_message(0, "Morning. Where does the money sit right now?", cx),
        agent_message(
            0,
            "Here is the command center: portfolio, PnL, drawdown, and what \
             the agents are running.",
            cx,
        ),
        placeholder_frame(
            "command-center header",
            "placeholder frame · the live command-center components land in a \
             sibling lane of #284; this beat swaps to them on arrival",
            vec![
                fixture_row("portfolio", "12,840,000 sats", cx),
                fixture_row("today", "+38,200 sats · +0.30%", cx),
                fixture_row("30d max drawdown", "−2.1%", cx),
                fixture_row("strategies", "funding_carry active · rebalance idle", cx),
            ],
            cx,
        ),
    ]
}

fn beat_carry_comparison(cx: &App) -> Vec<gpui::AnyElement> {
    vec![
        user_message(
            1,
            "Where does the same sat earn the most carry tonight?",
            cx,
        ),
        agent_message(
            1,
            "Normalized to the same 8h window, LN Markets pays the best \
             funding carry right now, and the edge clears fees.",
            cx,
        ),
        placeholder_frame(
            "normalized carry · 8h window",
            "demo fixtures · live numbers come from the normalized carry \
             surface (#279)",
            vec![
                fixture_row("lnmarkets · BTCUSD perp", "+14.6 bps / 8h", cx),
                fixture_row("hyperliquid · BTC perp", "+9.1 bps / 8h", cx),
                fixture_row("cost floor (fees + spread)", "−3.8 bps", cx),
                fixture_row("net edge, best venue", "+10.8 bps / 8h", cx),
            ],
            cx,
        ),
        placeholder_frame(
            "prediction",
            "placeholder frame · the prediction card is its own #284 item; \
             the record shape is the pre-action prediction from #276",
            vec![
                fixture_row("instrument", "BTCUSD · funding", cx),
                fixture_row("direction", "funding stays positive", cx),
                fixture_row("confidence", "0.72", cx),
                fixture_row("horizon", "8h · resolves at next funding", cx),
            ],
            cx,
        ),
    ]
}

fn beat_confirm_gated_order(cx: &App) -> Vec<gpui::AnyElement> {
    let colors = cx.theme().colors();
    let order_card = v_flex()
        .w(px(420.))
        .my_1p5()
        .rounded_md()
        .border_1()
        .border_color(colors.border.opacity(0.8))
        .bg(colors.editor_background)
        .overflow_hidden()
        .child(
            h_flex()
                .h_8()
                .w_full()
                .px_2()
                .gap_1p5()
                .justify_between()
                .bg(colors
                    .element_background
                    .blend(colors.editor_foreground.opacity(0.025)))
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .child(
                            Icon::new(IconName::Info)
                                .size(IconSize::Small)
                                .color(Color::Info),
                        )
                        .child(Label::new("place_order · confirm required").size(LabelSize::Small)),
                )
                .child(
                    Label::new("testnet")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        )
        .child(
            v_flex()
                .px_3()
                .py_2()
                .gap_1()
                .child(fixture_row("venue", "lnmarkets · BTCUSD perp", cx))
                .child(fixture_row("order", "sell 0.02 BTC · market", cx))
                .child(fixture_row(
                    "mandate headroom",
                    "12% of order budget · 1.4× lev",
                    cx,
                ))
                .child(fixture_row(
                    "loss stop distance",
                    "untouched · 6.2% remaining",
                    cx,
                )),
        )
        .child(
            h_flex()
                .w_full()
                .px_3()
                .pb_2()
                .gap_2()
                .justify_between()
                .child(
                    VizChip::new("simulated · no live order")
                        .tone(VizChipTone::Warn)
                        .scale(1.0),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("five-beat-reject", "Reject").label_size(LabelSize::Small),
                        )
                        .child(
                            Button::new("five-beat-confirm", "Confirm order")
                                .style(ButtonStyle::Filled)
                                .label_size(LabelSize::Small),
                        ),
                ),
        )
        .into_any_element();

    vec![
        user_message(2, "Rotate 0.02 BTC into the carry position.", cx),
        agent_message(
            2,
            "This order is inside the mandate, but placement is confirm-gated \
             — nothing moves until you approve. The demo simulates the tool \
             result; no order leaves this screen.",
            cx,
        ),
        order_card,
    ]
}

fn beat_ledger_receipt(cx: &App) -> Vec<gpui::AnyElement> {
    vec![
        user_message(3, "Show me the receipt.", cx),
        agent_message(
            3,
            "Every action lands as a double-entry ledger entry with its \
             postings; the verification state is computed, never asserted.",
            cx,
        ),
        placeholder_frame(
            "ledger entry 4182 · order.filled",
            "demo fixtures · the live view reads the trading ledger; the \
             receipt viewer is a later inventory item (§5)",
            vec![
                fixture_row("debit", "margin:lnmarkets +0.02 BTC", cx),
                fixture_row("credit", "wallet:lightning −0.02 BTC", cx),
                fixture_row("fees", "fees:lnmarkets 210 sats", cx),
                fixture_row(
                    "verification",
                    "seq 4182 · hash chain verified (fixture)",
                    cx,
                ),
            ],
            cx,
        ),
    ]
}

fn beat_swap_lane(cx: &App) -> Vec<gpui::AnyElement> {
    vec![
        user_message(
            4,
            "Fund it — swap 50,000 sats from Lightning to on-chain.",
            cx,
        ),
        agent_message(
            4,
            "Five providers quoted the RFQ. provider-b wins on output net of \
             fees; the swap runs below, on the network it settles across.",
            cx,
        ),
        RfqComparisonCard::new(demo_quote_set())
            .selected(1)
            .into_any_element(),
        demo_card().stage(SwapStage::Executing).into_any_element(),
        NetworkCard::new(demo_shape_fixture())
            .time(14.0)
            .into_any_element(),
    ]
}

impl RenderOnce for WhereShouldTheMoneySit {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let beat = self.beat.min(BEAT_COUNT - 1);
        let content = match beat {
            0 => beat_command_center(cx),
            1 => beat_carry_comparison(cx),
            2 => beat_confirm_gated_order(cx),
            3 => beat_ledger_receipt(cx),
            _ => beat_swap_lane(cx),
        };

        v_flex()
            .gap_1()
            .max_w(px(560.))
            .child(
                VizProgressRail::new(BEAT_TITLES)
                    .completed(beat)
                    .active(beat)
                    .show_all_labels(true)
                    .scale(1.0),
            )
            .children(content)
    }
}

impl Component for WhereShouldTheMoneySit {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(window: &mut Window, cx: &mut App) -> AnyElement {
        let beat_state = window.use_keyed_state("market-five-beat-demo", cx, |_, _| 0usize);
        let beat = *beat_state.read(cx);

        let step = |state: gpui::Entity<usize>, delta: isize| {
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                state.update(cx, |beat, cx| {
                    *beat = beat.saturating_add_signed(delta).min(BEAT_COUNT - 1);
                    cx.notify();
                });
            }
        };

        let controls = h_flex()
            .gap_2()
            .items_center()
            .child(
                Button::new("five-beat-back", "Back")
                    .label_size(LabelSize::Small)
                    .disabled(beat == 0)
                    .on_click(step(beat_state.clone(), -1)),
            )
            .child(
                Button::new("five-beat-next", "Next beat")
                    .style(ButtonStyle::Filled)
                    .label_size(LabelSize::Small)
                    .disabled(beat + 1 == BEAT_COUNT)
                    .on_click(step(beat_state.clone(), 1)),
            )
            .child(
                Label::new(format!(
                    "beat {} of {BEAT_COUNT} · {}",
                    beat + 1,
                    beat_title(beat)
                ))
                .size(LabelSize::Small)
                .color(Color::Muted),
            )
            .child(
                Label::new("click the stage to advance · all data is fixtures")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );

        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Where should the money sit?",
                vec![single_example(
                    "The five-beat rehearsal: command center → carry → \
                     confirm-gated order → receipt → swap lane",
                    v_flex()
                        .gap_2()
                        .child(controls)
                        .child(
                            div()
                                .id("five-beat-stage")
                                .on_click(step(beat_state, 1))
                                .child(WhereShouldTheMoneySit::new().beat(beat)),
                        )
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
    fn the_walkthrough_has_five_named_beats() {
        assert_eq!(BEAT_COUNT, 5);
        assert_eq!(beat_title(0), "command center");
        assert_eq!(beat_title(4), "swap lane");
    }

    #[test]
    fn out_of_range_beats_clamp_instead_of_panicking() {
        assert_eq!(beat_title(99), "swap lane");
        let demo = WhereShouldTheMoneySit::new().beat(99);
        assert_eq!(demo.beat, BEAT_COUNT - 1);
    }
}
