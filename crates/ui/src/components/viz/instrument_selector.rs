use documented::Documented;

use crate::Chip;
use crate::components::viz::MarketTokens;
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrumentKind {
    Perpetual,
    Spot,
    Option,
    Prediction,
}
impl InstrumentKind {
    fn label(self) -> &'static str {
        match self {
            Self::Perpetual => "perpetual",
            Self::Spot => "spot",
            Self::Option => "option",
            Self::Prediction => "prediction",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Instrument {
    pub symbol: SharedString,
    pub name: SharedString,
    pub venue: SharedString,
    pub kind: InstrumentKind,
}
pub trait InstrumentCatalogSource {
    fn instruments(&self) -> Vec<Instrument>;
}
pub struct DemoInstrumentCatalogSource;
impl InstrumentCatalogSource for DemoInstrumentCatalogSource {
    fn instruments(&self) -> Vec<Instrument> {
        vec![
            Instrument {
                symbol: "BTC-PERP".into(),
                name: "Bitcoin perpetual".into(),
                venue: "Hyperliquid".into(),
                kind: InstrumentKind::Perpetual,
            },
            Instrument {
                symbol: "BTCUSD".into(),
                name: "Bitcoin inverse future".into(),
                venue: "LN Markets".into(),
                kind: InstrumentKind::Perpetual,
            },
            Instrument {
                symbol: "ETH-USDC".into(),
                name: "Ether spot".into(),
                venue: "Hyperliquid".into(),
                kind: InstrumentKind::Spot,
            },
            Instrument {
                symbol: "HYPE-PERP".into(),
                name: "Hype perpetual".into(),
                venue: "Hyperliquid".into(),
                kind: InstrumentKind::Perpetual,
            },
            Instrument {
                symbol: "BTC-100K".into(),
                name: "Bitcoin above 100K".into(),
                venue: "Prediction".into(),
                kind: InstrumentKind::Prediction,
            },
        ]
    }
}
fn fuzzy_score(candidate: &str, query: &str) -> Option<usize> {
    let query = query.to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let candidate = candidate.to_lowercase();
    let mut offset = 0usize;
    let mut score = 0usize;
    for character in query.chars() {
        let remainder = candidate.get(offset..)?;
        let position = remainder.find(character)?;
        score = score.saturating_add(position);
        offset = offset.saturating_add(position + character.len_utf8());
    }
    Some(score)
}
fn filtered(
    catalog: &[Instrument],
    query: &str,
    venue: Option<&str>,
    kind: Option<InstrumentKind>,
) -> Vec<Instrument> {
    let mut matches: Vec<_> = catalog
        .iter()
        .filter(|instrument| {
            venue.is_none_or(|venue| instrument.venue.as_ref() == venue)
                && kind.is_none_or(|kind| instrument.kind == kind)
        })
        .filter_map(|instrument| {
            let searchable = format!(
                "{} {} {}",
                instrument.symbol, instrument.name, instrument.venue
            );
            fuzzy_score(&searchable, query).map(|score| (score, instrument.clone()))
        })
        .collect();
    matches.sort_by_key(|(score, instrument)| (*score, instrument.symbol.clone()));
    matches
        .into_iter()
        .map(|(_, instrument)| instrument)
        .collect()
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Fuzzy instrument catalog with venue and instrument-kind facets.
pub struct InstrumentSelector {
    catalog: Vec<Instrument>,
    query: SharedString,
    venue: Option<SharedString>,
    kind: Option<InstrumentKind>,
    tokens: Option<MarketTokens>,
}
impl InstrumentSelector {
    pub fn from_source(source: &impl InstrumentCatalogSource) -> Self {
        Self {
            catalog: source.instruments(),
            query: "btc".into(),
            venue: None,
            kind: None,
            tokens: None,
        }
    }
    pub fn query(mut self, query: impl Into<SharedString>) -> Self {
        self.query = query.into();
        self
    }
    pub fn venue(mut self, venue: impl Into<SharedString>) -> Self {
        self.venue = Some(venue.into());
        self
    }
    pub fn kind(mut self, kind: InstrumentKind) -> Self {
        self.kind = Some(kind);
        self
    }
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}
impl RenderOnce for InstrumentSelector {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let matches = filtered(&self.catalog, &self.query, self.venue.as_deref(), self.kind);
        v_flex()
            .debug_selector(|| "market.instrument_selector".into())
            .w(px(520.))
            .gap_2()
            .p_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(Chip::new(format!("⌕ {}", self.query)))
                    .when_some(self.venue, |this, venue| {
                        this.child(Chip::new(format!("venue:{venue}")))
                    })
                    .when_some(self.kind, |this, kind| {
                        this.child(Chip::new(format!("kind:{}", kind.label())))
                    }),
            )
            .children(matches.into_iter().take(8).map(|instrument| {
                h_flex()
                    .justify_between()
                    .py_1()
                    .child(
                        v_flex()
                            .child(Label::new(instrument.symbol).size(LabelSize::Small))
                            .child(
                                Label::new(instrument.name)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(
                        h_flex().gap_2().child(Chip::new(instrument.venue)).child(
                            div()
                                .text_color(tokens.muted)
                                .child(instrument.kind.label()),
                        ),
                    )
            }))
    }
}
impl Component for InstrumentSelector {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }
    fn description() -> &'static str {
        Self::DOCS
    }
    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Instrument selector",
                vec![single_example(
                    "Fuzzy query with venue/kind facets",
                    InstrumentSelector::from_source(&DemoInstrumentCatalogSource)
                        .kind(InstrumentKind::Perpetual)
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Named facets carry every distinction",
                    InstrumentSelector::from_source(&DemoInstrumentCatalogSource)
                        .venue("Hyperliquid")
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
    fn fuzzy_search_and_facets_compose() {
        let catalog = DemoInstrumentCatalogSource.instruments();
        let results = filtered(
            &catalog,
            "btcp",
            Some("Hyperliquid"),
            Some(InstrumentKind::Perpetual),
        );
        assert_eq!(
            results.first().map(|instrument| instrument.symbol.as_ref()),
            Some("BTC-PERP")
        );
    }
}
