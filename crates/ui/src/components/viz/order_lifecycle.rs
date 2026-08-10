use documented::Documented;

use crate::Chip;
use crate::components::viz::MarketTokens;
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderLifecycleStage {
    Placed,
    Resting,
    PartiallyFilled,
    Filled,
    Cancelled,
}
impl OrderLifecycleStage {
    fn label(self) -> &'static str {
        match self {
            Self::Placed => "placed",
            Self::Resting => "resting",
            Self::PartiallyFilled => "partial",
            Self::Filled => "filled",
            Self::Cancelled => "cancelled",
        }
    }
    fn glyph(self) -> &'static str {
        match self {
            Self::Placed => "○",
            Self::Resting => "◉",
            Self::PartiallyFilled => "◐",
            Self::Filled => "✓",
            Self::Cancelled => "×",
        }
    }
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Placed, Self::Resting | Self::Cancelled)
                | (
                    Self::Resting,
                    Self::PartiallyFilled | Self::Filled | Self::Cancelled
                )
                | (
                    Self::PartiallyFilled,
                    Self::PartiallyFilled | Self::Filled | Self::Cancelled
                )
        )
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct OrderLifecycle {
    pub order_id: SharedString,
    pub stage: OrderLifecycleStage,
    pub filled_units: f64,
    pub total_units: f64,
}
pub trait OrderLifecycleSource {
    fn order_lifecycle(&self) -> OrderLifecycle;
}
pub struct DemoOrderLifecycleSource;
impl OrderLifecycleSource for DemoOrderLifecycleSource {
    fn order_lifecycle(&self) -> OrderLifecycle {
        OrderLifecycle {
            order_id: "cloid-0042".into(),
            stage: OrderLifecycleStage::PartiallyFilled,
            filled_units: 0.05,
            total_units: 0.08,
        }
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Compact order-state chip/toast with structural lifecycle glyphs.
pub struct OrderLifecycleToast {
    lifecycle: OrderLifecycle,
    tokens: Option<MarketTokens>,
}
impl OrderLifecycleToast {
    pub fn from_source(source: &impl OrderLifecycleSource) -> Self {
        Self {
            lifecycle: source.order_lifecycle(),
            tokens: None,
        }
    }
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}
impl RenderOnce for OrderLifecycleToast {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let color = match self.lifecycle.stage {
            OrderLifecycleStage::Filled => tokens.up,
            OrderLifecycleStage::Cancelled => tokens.down,
            _ => tokens.flat,
        };
        h_flex()
            .debug_selector(|| "market.order_lifecycle".into())
            .gap_2()
            .p_2()
            .child(
                Chip::new(format!(
                    "{} {}",
                    self.lifecycle.stage.glyph(),
                    self.lifecycle.stage.label()
                ))
                .label_color(Color::Custom(color))
                .border_color(color.opacity(0.7)),
            )
            .child(
                Label::new(self.lifecycle.order_id)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(format!(
                "{:.3} / {:.3}",
                self.lifecycle.filled_units, self.lifecycle.total_units
            ))
    }
}
impl Component for OrderLifecycleToast {
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
                "Order lifecycle",
                vec![single_example(
                    "Streaming partial fill",
                    OrderLifecycleToast::from_source(&DemoOrderLifecycleSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Glyph and label preserve the phase",
                    OrderLifecycleToast::from_source(&DemoOrderLifecycleSource)
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
    fn lifecycle_rejects_terminal_transitions() {
        assert!(OrderLifecycleStage::Resting.can_transition_to(OrderLifecycleStage::Filled));
        assert!(!OrderLifecycleStage::Filled.can_transition_to(OrderLifecycleStage::Cancelled));
    }
}
