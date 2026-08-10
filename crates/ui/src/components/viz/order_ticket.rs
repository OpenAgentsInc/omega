use std::sync::Arc;

use documented::Documented;
use gpui::px;

use crate::Chip;
use crate::components::viz::{
    Gauge, HeadroomMeter, MarketDirection, MarketTokens, format_usd_cents, format_with_decimals,
    market_number_font,
};
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderKind {
    Market,
    Limit,
    Trigger,
}
impl OrderKind {
    fn label(self) -> &'static str {
        match self {
            Self::Market => "market",
            Self::Limit => "limit",
            Self::Trigger => "trigger",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderSide {
    Buy,
    Sell,
}
impl OrderSide {
    pub fn direction(self) -> MarketDirection {
        match self {
            Self::Buy => MarketDirection::Up,
            Self::Sell => MarketDirection::Down,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeMode {
    Units,
    Notional,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeInForce {
    GoodTilCancelled,
    ImmediateOrCancel,
    FillOrKill,
}
impl TimeInForce {
    fn label(self) -> &'static str {
        match self {
            Self::GoodTilCancelled => "GTC",
            Self::ImmediateOrCancel => "IOC",
            Self::FillOrKill => "FOK",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VenueOrderRules {
    pub tick_size: f64,
    pub lot_size: f64,
    pub max_leverage: u8,
}
#[derive(Debug, Clone, PartialEq)]
pub struct OrderDraft {
    pub instrument: SharedString,
    pub venue: SharedString,
    pub kind: OrderKind,
    pub side: OrderSide,
    pub size_mode: SizeMode,
    pub size: f64,
    pub reference_price: f64,
    pub limit_price: Option<f64>,
    pub trigger_price: Option<f64>,
    pub leverage: u8,
    pub reduce_only: bool,
    pub time_in_force: TimeInForce,
    pub take_profit: Option<f64>,
    pub stop_loss: Option<f64>,
    pub available_margin_cents: i64,
}
#[derive(Debug, Clone, PartialEq)]
pub enum OrderValidation {
    Valid(OrderPreview),
    Invalid(Vec<&'static str>),
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrderPreview {
    pub notional_cents: i64,
    pub margin_cents: i64,
    pub liquidation_price: f64,
    pub margin_fraction: f32,
}
#[derive(Debug, Clone, PartialEq)]
pub struct SubmitOrderIntent {
    pub draft: OrderDraft,
    pub preview: OrderPreview,
}
pub trait OrderTicketSource {
    fn order_draft(&self) -> OrderDraft;
    fn venue_rules(&self) -> VenueOrderRules;
}
pub struct DemoOrderTicketSource;
impl OrderTicketSource for DemoOrderTicketSource {
    fn order_draft(&self) -> OrderDraft {
        OrderDraft {
            instrument: "BTC-PERP".into(),
            venue: "Hyperliquid".into(),
            kind: OrderKind::Limit,
            side: OrderSide::Buy,
            size_mode: SizeMode::Units,
            size: 0.08,
            reference_price: 116_420.0,
            limit_price: Some(116_400.0),
            trigger_price: None,
            leverage: 3,
            reduce_only: false,
            time_in_force: TimeInForce::GoodTilCancelled,
            take_profit: Some(120_000.0),
            stop_loss: Some(112_000.0),
            available_margin_cents: 500_000,
        }
    }
    fn venue_rules(&self) -> VenueOrderRules {
        VenueOrderRules {
            tick_size: 0.5,
            lot_size: 0.001,
            max_leverage: 5,
        }
    }
}
fn aligned(value: f64, step: f64) -> bool {
    step.is_finite() && step > 0.0 && ((value / step) - (value / step).round()).abs() < 1e-8
}
impl OrderDraft {
    pub fn validate(&self, rules: VenueOrderRules) -> OrderValidation {
        let mut errors = Vec::new();
        if !self.size.is_finite() || self.size <= 0.0 {
            errors.push("size must be positive");
        }
        if self.size_mode == SizeMode::Units && !aligned(self.size, rules.lot_size) {
            errors.push("size violates venue lot size");
        }
        let order_price = match self.kind {
            OrderKind::Market => Some(self.reference_price),
            OrderKind::Limit => self.limit_price,
            OrderKind::Trigger => self.trigger_price,
        };
        let Some(price) = order_price.filter(|price| price.is_finite() && *price > 0.0) else {
            errors.push("order price is required");
            return OrderValidation::Invalid(errors);
        };
        if self.kind != OrderKind::Market && !aligned(price, rules.tick_size) {
            errors.push("price violates venue tick size");
        }
        if self.leverage == 0 || self.leverage > rules.max_leverage {
            errors.push("leverage exceeds venue limit");
        }
        let notional = match self.size_mode {
            SizeMode::Units => self.size * price,
            SizeMode::Notional => self.size,
        };
        let notional_cents = (notional * 100.0).round() as i64;
        let margin_cents = notional_cents.div_euclid(i64::from(self.leverage.max(1)));
        if margin_cents > self.available_margin_cents {
            errors.push("margin exceeds available balance");
        }
        if !errors.is_empty() {
            return OrderValidation::Invalid(errors);
        }
        let distance = price / f64::from(self.leverage.max(1));
        let liquidation_price = match self.side {
            OrderSide::Buy => price - distance,
            OrderSide::Sell => price + distance,
        };
        OrderValidation::Valid(OrderPreview {
            notional_cents,
            margin_cents,
            liquidation_price,
            margin_fraction: (margin_cents as f64 / self.available_margin_cents.max(1) as f64)
                .clamp(0.0, 1.0) as f32,
        })
    }
}

#[derive(IntoElement, RegisterComponent, Documented)]
/// Venue-validated order ticket with an exact live risk preview.
pub struct OrderTicket {
    draft: OrderDraft,
    rules: VenueOrderRules,
    tokens: Option<MarketTokens>,
    on_review: Option<Arc<dyn Fn(SubmitOrderIntent, &mut Window, &mut App) + 'static>>,
}
impl OrderTicket {
    pub fn from_source(source: &impl OrderTicketSource) -> Self {
        Self {
            draft: source.order_draft(),
            rules: source.venue_rules(),
            tokens: None,
            on_review: None,
        }
    }
    pub fn submit_intent(&self) -> Option<SubmitOrderIntent> {
        match self.draft.validate(self.rules) {
            OrderValidation::Valid(preview) => Some(SubmitOrderIntent {
                draft: self.draft.clone(),
                preview,
            }),
            OrderValidation::Invalid(_) => None,
        }
    }
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
    pub fn on_review(
        mut self,
        handler: impl Fn(SubmitOrderIntent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_review = Some(Arc::new(handler));
        self
    }
}
impl RenderOnce for OrderTicket {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let validation = self.draft.validate(self.rules);
        let preview = match &validation {
            OrderValidation::Valid(preview) => Some(*preview),
            OrderValidation::Invalid(_) => None,
        };
        let validation_error = match validation {
            OrderValidation::Valid(_) => None,
            OrderValidation::Invalid(errors) => Some(errors.join(" · ")),
        };
        let review_intent = preview.map(|preview| SubmitOrderIntent {
            draft: self.draft.clone(),
            preview,
        });
        let review_enabled = review_intent.is_some();
        let tab = |kind: OrderKind| {
            Chip::new(kind.label()).label_color(Color::Custom(if kind == self.draft.kind {
                tokens.text
            } else {
                tokens.muted
            }))
        };
        let field = |label, value: String| {
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
                        .child(value),
                )
        };
        v_flex()
            .debug_selector(|| "market.order_ticket".into())
            .w(px(520.))
            .gap_3()
            .p_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(tab(OrderKind::Market))
                    .child(tab(OrderKind::Limit))
                    .child(tab(OrderKind::Trigger)),
            )
            .child(
                h_flex()
                    .gap_4()
                    .child(field("instrument", self.draft.instrument.to_string()))
                    .child(field(
                        "side",
                        format!(
                            "{} {:?}",
                            self.draft.side.direction().glyph(),
                            self.draft.side
                        ),
                    ))
                    .child(field(
                        "size",
                        format!(
                            "{} {:?}",
                            format_with_decimals(self.draft.size, 3),
                            self.draft.size_mode
                        ),
                    ))
                    .child(field("leverage", format!("{}×", self.draft.leverage))),
            )
            .child(
                h_flex()
                    .gap_4()
                    .child(field("TIF", self.draft.time_in_force.label().into()))
                    .child(field(
                        "reduce only",
                        if self.draft.reduce_only {
                            "✓ on"
                        } else {
                            "○ off"
                        }
                        .into(),
                    ))
                    .child(field(
                        "take profit",
                        self.draft
                            .take_profit
                            .map(|value| format_with_decimals(value, 2))
                            .unwrap_or_else(|| "—".into()),
                    ))
                    .child(field(
                        "stop loss",
                        self.draft
                            .stop_loss
                            .map(|value| format_with_decimals(value, 2))
                            .unwrap_or_else(|| "—".into()),
                    )),
            )
            .when_some(preview, |this, preview| {
                this.child(
                    Gauge::new(HeadroomMeter {
                        label: "margin allocation".into(),
                        used_display: format_usd_cents(preview.margin_cents).into(),
                        limit_display: format_usd_cents(self.draft.available_margin_cents).into(),
                        fraction: preview.margin_fraction,
                    })
                    .width(480.)
                    .tokens(tokens),
                )
                .child(
                    h_flex()
                        .gap_4()
                        .child(field("notional", format_usd_cents(preview.notional_cents)))
                        .child(field("margin", format_usd_cents(preview.margin_cents)))
                        .child(field(
                            "liq preview",
                            format_with_decimals(preview.liquidation_price, 2),
                        )),
                )
            })
            .when_some(validation_error, |this, error| {
                this.child(
                    Label::new(error)
                        .size(LabelSize::XSmall)
                        .color(Color::Error),
                )
            })
            .child(
                Button::new("order-ticket-review", "Review order")
                    .style(ButtonStyle::Filled)
                    .disabled(!review_enabled)
                    .when_some(
                        self.on_review.zip(review_intent),
                        |button, (handler, intent)| {
                            button.on_click(move |_, window, cx| {
                                handler(intent.clone(), window, cx);
                            })
                        },
                    ),
            )
    }
}
impl Component for OrderTicket {
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
                "Order ticket",
                vec![single_example(
                    "Limit order with venue validation",
                    OrderTicket::from_source(&DemoOrderTicketSource).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Signs, labels, and meter thresholds preserve risk",
                    OrderTicket::from_source(&DemoOrderTicketSource)
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
    fn valid_ticket_produces_typed_intent() {
        let ticket = OrderTicket::from_source(&DemoOrderTicketSource);
        assert!(ticket.submit_intent().is_some());
    }
    #[test]
    fn rejects_off_tick_prices() {
        let mut draft = DemoOrderTicketSource.order_draft();
        draft.limit_price = Some(116_400.23);
        assert!(matches!(
            draft.validate(DemoOrderTicketSource.venue_rules()),
            OrderValidation::Invalid(_)
        ));
    }
}
