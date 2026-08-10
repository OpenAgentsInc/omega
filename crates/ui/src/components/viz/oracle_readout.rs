use documented::Documented;
use gpui::px;

use crate::Chip;
use crate::components::viz::{MarketTokens, format_with_decimals, market_number_font};
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttestationState {
    Verified,
    Pending,
    Invalid,
}
impl AttestationState {
    fn glyph(self) -> &'static str {
        match self {
            Self::Verified => "✓",
            Self::Pending => "◌",
            Self::Invalid => "×",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Pending => "pending",
            Self::Invalid => "invalid",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct OracleAttestation {
    pub oracle: SharedString,
    pub announced_price: f64,
    pub attested_price: Option<f64>,
    pub state: AttestationState,
    pub attested_at_ms: Option<i64>,
}
pub trait OracleAttestationSource {
    fn oracle_attestation(&self) -> OracleAttestation;
}
pub struct DemoOracleAttestationSource;
impl OracleAttestationSource for DemoOracleAttestationSource {
    fn oracle_attestation(&self) -> OracleAttestation {
        OracleAttestation {
            oracle: "Pyth BTC/USD".into(),
            announced_price: 116_420.50,
            attested_price: Some(116_419.92),
            state: AttestationState::Verified,
            attested_at_ms: Some(1_754_700_000_000),
        }
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Announced and attested oracle prices with structural verification state.
pub struct OracleAttestationReadout {
    attestation: OracleAttestation,
    tokens: Option<MarketTokens>,
}
impl OracleAttestationReadout {
    pub fn from_source(source: &impl OracleAttestationSource) -> Self {
        Self {
            attestation: source.oracle_attestation(),
            tokens: None,
        }
    }
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}
impl RenderOnce for OracleAttestationReadout {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let color = match self.attestation.state {
            AttestationState::Verified => tokens.up,
            AttestationState::Pending => tokens.flat,
            AttestationState::Invalid => tokens.down,
        };
        let price = |label, value: Option<f64>| {
            v_flex()
                .gap_0p5()
                .child(
                    Label::new(label)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .font(market_number_font(cx))
                        .text_size(px(11.))
                        .text_color(tokens.text)
                        .child(
                            value
                                .map(|value| format_with_decimals(value, 2))
                                .unwrap_or_else(|| "—".into()),
                        ),
                )
        };
        h_flex()
            .debug_selector(|| "market.oracle_attestation".into())
            .gap_4()
            .p_2()
            .child(Label::new(self.attestation.oracle).size(LabelSize::Small))
            .child(price("announced", Some(self.attestation.announced_price)))
            .child(price("attested", self.attestation.attested_price))
            .child(
                Chip::new(format!(
                    "{} {}",
                    self.attestation.state.glyph(),
                    self.attestation.state.label()
                ))
                .label_color(Color::Custom(color))
                .border_color(color.opacity(0.7))
                .bg_color(color.opacity(0.08)),
            )
    }
}
impl Component for OracleAttestationReadout {
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
                "Oracle attestation",
                vec![single_example(
                    "Announced and signed values",
                    OracleAttestationReadout::from_source(&DemoOracleAttestationSource)
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Verification glyph and label carry state",
                    OracleAttestationReadout::from_source(&DemoOracleAttestationSource)
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
    fn verified_demo_has_attested_value() {
        let value = DemoOracleAttestationSource.oracle_attestation();
        assert_eq!(value.state, AttestationState::Verified);
        assert!(value.attested_price.is_some());
    }
}
