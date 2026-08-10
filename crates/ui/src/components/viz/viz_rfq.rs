use std::time::Duration;

use documented::Documented;
use gpui::{Animation, AnimationExt as _, px};

use crate::components::viz::{SwapNetwork, VizChip, VizChipTone, VizPalette, format_sats};
use crate::prelude::*;

/// A provider-reputation placeholder. Receipts-backed reputation has not
/// shipped, so the chip renders either an honest `unrated` or a fixture fill
/// rate; nothing here pretends to be a verified track record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RfqReputation {
    Unrated,
    /// Percentage of past RFQs this provider filled, as a demo fixture until
    /// the reputation lane lands.
    FillRate(u8),
}

impl RfqReputation {
    fn label(self) -> String {
        match self {
            Self::Unrated => "unrated".to_owned(),
            Self::FillRate(percent) => format!("{percent}% fill"),
        }
    }

    fn tone(self) -> VizChipTone {
        match self {
            Self::Unrated => VizChipTone::Neutral,
            Self::FillRate(percent) if percent >= 90 => VizChipTone::Ok,
            Self::FillRate(_) => VizChipTone::Neutral,
        }
    }
}

/// One provider's answer to the RFQ, projected to the numbers a person
/// compares: what comes out, what it costs, and how long the offer stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RfqQuote {
    /// The provider's pubkey or label; rendered truncated.
    pub provider: SharedString,
    pub reputation: RfqReputation,
    pub output_sats: u64,
    pub fee_sats: u64,
    /// Omitted when the quote did not state a rate; the row then shows only
    /// the absolute fee instead of fabricating `0 bps`.
    pub fee_bps: Option<u32>,
    /// Seconds until the quote expires; zero or negative means expired.
    pub expires_in_secs: i64,
}

impl RfqQuote {
    pub fn usable(&self) -> bool {
        self.expires_in_secs > 0
    }
}

/// The full comparison input: one requested conversion and every quote
/// received for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteSet {
    pub from_ticker: SharedString,
    pub to_ticker: SharedString,
    pub input_sats: u64,
    pub network: SwapNetwork,
    pub quotes: Vec<RfqQuote>,
}

impl QuoteSet {
    /// The highlighted winner, mirroring the session-flow selection policy:
    /// among unexpired quotes, highest output, then lowest fee, then the
    /// lexicographically lowest provider. Expired quotes never win, no matter
    /// how good their terms were.
    pub fn best(&self) -> Option<usize> {
        self.quotes
            .iter()
            .enumerate()
            .filter(|(_, quote)| quote.usable())
            .max_by(|(_, a), (_, b)| {
                a.output_sats
                    .cmp(&b.output_sats)
                    .then_with(|| b.fee_sats.cmp(&a.fee_sats))
                    .then_with(|| b.provider.cmp(&a.provider))
            })
            .map(|(index, _)| index)
    }
}

fn countdown_label(secs: i64) -> String {
    if secs <= 0 {
        "expired".to_owned()
    } else if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m {:02}s", secs / 60, secs % 60)
    }
}

fn truncated_provider(provider: &str) -> String {
    if provider.chars().count() <= 10 {
        provider.to_owned()
    } else {
        let head: String = provider.chars().take(8).collect();
        format!("{head}…")
    }
}

/// An inline multi-provider RFQ comparison for one requested conversion:
/// provider (with a reputation placeholder chip), quoted output, fee, and a
/// live expiry countdown per row, with the policy-best quote highlighted by a
/// chip and an accent bar — never by color alone. Compact enough for the
/// transcript and the market panel alike.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct RfqComparisonCard {
    quote_set: QuoteSet,
    selected: Option<usize>,
    palette: Option<VizPalette>,
}

impl RfqComparisonCard {
    pub fn new(quote_set: QuoteSet) -> Self {
        Self {
            quote_set,
            selected: None,
            palette: None,
        }
    }

    /// Marks one quote as accepted by the session; independent of the best
    /// highlight, because the person may overrule policy.
    pub fn selected(mut self, selected: usize) -> Self {
        self.selected = Some(selected);
        self
    }

    /// Advances every countdown by `elapsed` seconds; the demo replay uses
    /// this to let quotes age and expire on screen.
    pub fn tick(mut self, elapsed: i64) -> Self {
        for quote in &mut self.quote_set.quotes {
            quote.expires_in_secs -= elapsed;
        }
        self
    }

    /// Overrides the theme palette; used by the grayscale audit preview.
    pub fn palette(mut self, palette: VizPalette) -> Self {
        self.palette = Some(palette);
        self
    }
}

impl RenderOnce for RfqComparisonCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.theme().colors();
        let status = cx.theme().status();
        let best = self.quote_set.best();
        let usable = self.quote_set.quotes.iter().filter(|q| q.usable()).count();
        // The tool-call card recipe from the transcript renderers.
        let card_border = colors.border.opacity(0.8);
        let header_background = colors
            .element_background
            .blend(colors.editor_foreground.opacity(0.025));

        let header = h_flex()
            .h_8()
            .w_full()
            .px_2()
            .justify_between()
            .bg(header_background)
            .child(
                Label::new(format!(
                    "RFQ · {} {} → {}",
                    format_sats(self.quote_set.input_sats),
                    self.quote_set.from_ticker,
                    self.quote_set.to_ticker
                ))
                .size(LabelSize::Custom(rems_from_px(13.)))
                .buffer_font(cx),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(
                        Label::new(self.quote_set.network.label())
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(format!(
                            "{usable} of {} quotes live",
                            self.quote_set.quotes.len()
                        ))
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                    ),
            );

        let column_headers = h_flex()
            .w_full()
            .px_2()
            .pt_1()
            .gap_2()
            .child(
                div().flex_1().min_w_0().child(
                    Label::new("provider")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
            )
            .child(
                h_flex().w(px(92.)).justify_end().child(
                    Label::new("output")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
            )
            .child(
                h_flex().w(px(96.)).justify_end().child(
                    Label::new("fee")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
            )
            .child(
                h_flex().w(px(52.)).justify_end().child(
                    Label::new("expires")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
            )
            .child(div().w(px(44.)));

        let selected = self.selected;
        let palette = self.palette;
        let rows = self
            .quote_set
            .quotes
            .iter()
            .enumerate()
            .map(|(index, quote)| {
                let is_best = best == Some(index);
                let is_selected = selected == Some(index);
                let expired = !quote.usable();
                let number_color = if expired {
                    Color::Disabled
                } else {
                    Color::Default
                };
                let expiry_color = if expired {
                    Color::Warning
                } else if quote.expires_in_secs <= 10 {
                    Color::Warning
                } else {
                    Color::Muted
                };

                let mut reputation_chip = VizChip::new(quote.reputation.label())
                    .tone(quote.reputation.tone())
                    .scale(1.0);
                if let Some(palette) = palette {
                    reputation_chip = reputation_chip.palette(palette);
                }

                let marker: gpui::AnyElement = if is_selected {
                    h_flex()
                        .gap_0p5()
                        .justify_end()
                        .child(
                            Icon::new(IconName::Check)
                                .size(IconSize::XSmall)
                                .color(Color::Success),
                        )
                        .child(
                            Label::new("sel")
                                .size(LabelSize::XSmall)
                                .color(Color::Success),
                        )
                        .into_any_element()
                } else if is_best {
                    let mut chip = VizChip::new("best").tone(VizChipTone::Ok).scale(1.0);
                    if let Some(palette) = palette {
                        chip = chip.palette(palette);
                    }
                    h_flex().justify_end().child(chip).into_any_element()
                } else {
                    div().into_any_element()
                };

                h_flex()
                    .w_full()
                    .px_2()
                    .py_0p5()
                    .gap_2()
                    .items_center()
                    .when(is_best, |row| {
                        row.border_l_2()
                            .border_color(status.success)
                            .bg(colors.element_background.opacity(0.5))
                    })
                    .when(!is_best, |row| {
                        row.border_l_2().border_color(gpui::transparent_black())
                    })
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1p5()
                            .items_center()
                            .child(
                                Label::new(truncated_provider(&quote.provider))
                                    .size(LabelSize::Small)
                                    .color(number_color)
                                    .buffer_font(cx)
                                    .truncate(),
                            )
                            .child(reputation_chip),
                    )
                    .child(
                        h_flex().w(px(92.)).justify_end().child(
                            Label::new(format_sats(quote.output_sats))
                                .size(LabelSize::Small)
                                .color(number_color)
                                .buffer_font(cx),
                        ),
                    )
                    .child(
                        h_flex().w(px(96.)).justify_end().child(
                            Label::new(match quote.fee_bps {
                                Some(bps) => format!("{} · {bps} bps", quote.fee_sats),
                                None => format!("{} sats", quote.fee_sats),
                            })
                            .size(LabelSize::XSmall)
                            .color(if expired {
                                Color::Disabled
                            } else {
                                Color::Muted
                            })
                            .buffer_font(cx),
                        ),
                    )
                    .child(
                        h_flex().w(px(52.)).justify_end().child(
                            Label::new(countdown_label(quote.expires_in_secs))
                                .size(LabelSize::XSmall)
                                .color(expiry_color)
                                .buffer_font(cx),
                        ),
                    )
                    .child(h_flex().w(px(44.)).justify_end().child(marker))
            })
            .collect::<Vec<_>>();

        let caption = if usable == 0 {
            "all quotes expired · request a fresh RFQ".to_owned()
        } else {
            "best = highest output, then lowest fee · quotes are provider claims until contracted"
                .to_owned()
        };

        v_flex()
            .w(px(420.))
            .my_1p5()
            .rounded_md()
            .border_1()
            .border_color(card_border)
            .bg(colors.editor_background)
            .overflow_hidden()
            .child(header)
            .child(column_headers)
            .child(v_flex().w_full().py_0p5().children(rows))
            .child(
                div().px_2().pb_1p5().child(
                    Label::new(caption)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
            )
    }
}

/// The demo fixture: five providers answering one 50,000-sat Lightning →
/// on-chain RFQ with staggered expiries. provider-b is the clear winner;
/// river-d posted a better output but has already expired, which is exactly
/// the trap the best-quote policy exists to refuse.
pub fn demo_quote_set() -> QuoteSet {
    QuoteSet {
        from_ticker: "LN".into(),
        to_ticker: "BTC".into(),
        input_sats: 50_000,
        network: SwapNetwork::Demo,
        quotes: vec![
            RfqQuote {
                provider: "provider-a".into(),
                reputation: RfqReputation::FillRate(94),
                output_sats: 49_870,
                fee_sats: 130,
                fee_bps: Some(26),
                expires_in_secs: 42,
            },
            RfqQuote {
                provider: "provider-b".into(),
                reputation: RfqReputation::FillRate(97),
                output_sats: 49_910,
                fee_sats: 90,
                fee_bps: Some(18),
                expires_in_secs: 28,
            },
            RfqQuote {
                provider: "provider-c".into(),
                reputation: RfqReputation::FillRate(88),
                output_sats: 49_850,
                fee_sats: 150,
                fee_bps: Some(30),
                expires_in_secs: 65,
            },
            RfqQuote {
                provider: "joiner".into(),
                reputation: RfqReputation::Unrated,
                output_sats: 49_780,
                fee_sats: 220,
                fee_bps: Some(44),
                expires_in_secs: 12,
            },
            RfqQuote {
                provider: "river-d".into(),
                reputation: RfqReputation::FillRate(91),
                output_sats: 49_930,
                fee_sats: 70,
                fee_bps: Some(14),
                expires_in_secs: -3,
            },
        ],
    }
}

impl Component for RfqComparisonCard {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Live countdowns",
                vec![single_example(
                    "Quotes age and expire; the best highlight follows the policy, not the clock",
                    RfqComparisonCard::new(demo_quote_set())
                        .with_animation(
                            "rfq-comparison-demo",
                            Animation::new(Duration::from_secs(30)).repeat(),
                            |card, delta| {
                                RfqComparisonCard::new(demo_quote_set())
                                    .tick((delta * 30.0) as i64)
                                    .palette_from(&card)
                            },
                        )
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Selection",
                vec![single_example(
                    "The accepted quote carries its own mark, independent of the best highlight",
                    RfqComparisonCard::new(demo_quote_set())
                        .selected(1)
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Everything expired",
                vec![single_example(
                    "No winner is ever picked from expired quotes",
                    RfqComparisonCard::new(demo_quote_set())
                        .tick(120)
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Best and selected survive without hue",
                    RfqComparisonCard::new(demo_quote_set())
                        .selected(1)
                        .palette(VizPalette::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

impl RfqComparisonCard {
    /// Copies the palette override from another card; the animation closure
    /// rebuilds the card each frame and must not drop a grayscale override.
    fn palette_from(mut self, other: &Self) -> Self {
        self.palette = other.palette;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_best_quote_maximizes_output_then_minimizes_fee() {
        let set = demo_quote_set();
        // river-d has the best terms but is expired; provider-b wins.
        assert_eq!(set.best(), Some(1));

        let mut tie = demo_quote_set();
        tie.quotes[0].output_sats = 49_910;
        tie.quotes[0].fee_sats = 80;
        // Same output as provider-b, lower fee: provider-a now wins.
        assert_eq!(tie.best(), Some(0));
    }

    #[test]
    fn expired_quotes_never_win() {
        let mut set = demo_quote_set();
        for quote in &mut set.quotes {
            quote.expires_in_secs = 0;
        }
        assert_eq!(set.best(), None);
    }

    #[test]
    fn provider_breaks_full_ties_deterministically() {
        let mut set = demo_quote_set();
        set.quotes.truncate(2);
        set.quotes[0].output_sats = 49_910;
        set.quotes[0].fee_sats = 90;
        // provider-a < provider-b lexicographically.
        assert_eq!(set.best(), Some(0));
    }

    #[test]
    fn countdowns_render_expiry_honestly() {
        assert_eq!(countdown_label(-3), "expired");
        assert_eq!(countdown_label(0), "expired");
        assert_eq!(countdown_label(9), "9s");
        assert_eq!(countdown_label(65), "1m 05s");
    }

    #[test]
    fn providers_truncate_without_losing_short_names() {
        assert_eq!(truncated_provider("provider-b"), "provider-b");
        assert_eq!(
            truncated_provider("232aa9c2d3642abf9ba89e4c9f704b018630acfaf3e2c9faa2faa2b708341b18"),
            "232aa9c2…"
        );
    }

    #[test]
    fn ticking_ages_every_quote() {
        let card = RfqComparisonCard::new(demo_quote_set()).tick(20);
        assert_eq!(card.quote_set.quotes[1].expires_in_secs, 8);
        assert_eq!(card.quote_set.quotes[3].expires_in_secs, -8);
        // joiner expired at 12s, so it can no longer win even a thin field.
        assert!(!card.quote_set.quotes[3].usable());
    }
}
