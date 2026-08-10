//! Confirm-before-send adapter for the Nautilus testnet command channel.
//! An intent becomes one typed command request exactly once; ambiguous results
//! remain terminal and are never retried.

use nautilus_sidecar::{
    CommandOutcome, CommandReceipt, CommandRequest, NautilusCommand, OrderSide,
};
use ui::{
    MarketEnvironment, OrderConfirmation, OrderConfirmationSource, OrderDraft, OrderKind,
    OrderSide as TicketOrderSide, OrderTicketSource, SizeMode, TimeInForce, VenueOrderRules,
};

const INSTRUMENT_ID: &str = "BTC-USD-PERP.HYPERLIQUID";
const ORDER_QUANTITY: f64 = 0.001;
const VENUE_TICK_SIZE: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NautilusOrderPreview {
    pub reference_price: f64,
    pub quantity: f64,
    pub limit_price: f64,
    pub notional_usd: f64,
    pub margin_usd: f64,
    pub liquidation_price: f64,
    pub available_margin_cents: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NautilusOrderIntent {
    pub command_id: String,
    pub client_order_id: String,
    pub preview: NautilusOrderPreview,
}

impl NautilusOrderIntent {
    /// Builds a deliberately non-marketable, post-only testnet bid. The user
    /// must still confirm the exact command before it crosses the boundary.
    pub fn testnet_probe(
        best_bid: f64,
        available_margin_cents: i64,
        sequence: u64,
    ) -> Option<Self> {
        if !best_bid.is_finite() || best_bid <= VENUE_TICK_SIZE || available_margin_cents <= 0 {
            return None;
        }
        let limit_price = ((best_bid * 0.9) / VENUE_TICK_SIZE).floor() * VENUE_TICK_SIZE;
        if !limit_price.is_finite() || limit_price <= 0.0 || limit_price >= best_bid {
            return None;
        }
        let notional_usd = ORDER_QUANTITY * limit_price;
        let available_margin_usd = available_margin_cents as f64 / 100.0;
        if available_margin_usd < notional_usd {
            return None;
        }
        let liquidation_price = (limit_price - available_margin_usd / ORDER_QUANTITY).max(0.0);
        let client_order_id = format!("omega-ui-{sequence}");
        Some(Self {
            command_id: format!("omega-ui-place-{sequence}"),
            client_order_id,
            preview: NautilusOrderPreview {
                reference_price: best_bid,
                quantity: ORDER_QUANTITY,
                limit_price,
                notional_usd,
                margin_usd: notional_usd,
                liquidation_price,
                available_margin_cents,
            },
        })
    }

    pub fn place_request(&self) -> CommandRequest {
        CommandRequest {
            command_id: self.command_id.clone(),
            command: NautilusCommand::PlaceOrder {
                client_order_id: self.client_order_id.clone(),
                instrument_id: INSTRUMENT_ID.to_owned(),
                side: OrderSide::Buy,
                quantity: format!("{:.3}", self.preview.quantity),
                price: format!("{:.1}", self.preview.limit_price),
                post_only: true,
                reduce_only: false,
            },
        }
    }

    pub fn cancel_request(&self, sequence: u64) -> CommandRequest {
        CommandRequest {
            command_id: format!("omega-ui-cancel-{sequence}"),
            command: NautilusCommand::CancelOrder {
                client_order_id: self.client_order_id.clone(),
            },
        }
    }
}

pub struct NautilusOrderTicketSource<'a> {
    intent: &'a NautilusOrderIntent,
}

impl<'a> NautilusOrderTicketSource<'a> {
    pub fn new(intent: &'a NautilusOrderIntent) -> Self {
        Self { intent }
    }
}

impl OrderTicketSource for NautilusOrderTicketSource<'_> {
    fn order_draft(&self) -> OrderDraft {
        OrderDraft {
            instrument: "BTC-PERP".into(),
            venue: "Hyperliquid".into(),
            kind: OrderKind::Limit,
            side: TicketOrderSide::Buy,
            size_mode: SizeMode::Units,
            size: self.intent.preview.quantity,
            reference_price: self.intent.preview.reference_price,
            limit_price: Some(self.intent.preview.limit_price),
            trigger_price: None,
            leverage: 1,
            reduce_only: false,
            time_in_force: TimeInForce::GoodTilCancelled,
            take_profit: None,
            stop_loss: None,
            available_margin_cents: self.intent.preview.available_margin_cents,
        }
    }

    fn venue_rules(&self) -> VenueOrderRules {
        VenueOrderRules {
            tick_size: VENUE_TICK_SIZE,
            lot_size: ORDER_QUANTITY,
            max_leverage: 1,
        }
    }
}

pub struct NautilusOrderConfirmationSource<'a> {
    intent: &'a NautilusOrderIntent,
}

impl<'a> NautilusOrderConfirmationSource<'a> {
    pub fn new(intent: &'a NautilusOrderIntent) -> Self {
        Self { intent }
    }
}

impl OrderConfirmationSource for NautilusOrderConfirmationSource<'_> {
    fn order_confirmation(&self) -> OrderConfirmation {
        let margin_bps = ((self.intent.preview.margin_usd * 100.0)
            / self.intent.preview.available_margin_cents.max(1) as f64
            * 10_000.0)
            .round()
            .clamp(0.0, u32::MAX as f64) as u32;
        OrderConfirmation {
            request_id: self.intent.command_id.clone().into(),
            exact_order: format!(
                "BUY {:.3} BTC-PERP · LIMIT {:.1} · GTC · POST ONLY",
                self.intent.preview.quantity, self.intent.preview.limit_price,
            )
            .into(),
            estimated_cost_cents: (self.intent.preview.notional_usd * 100.0).round() as i64,
            headroom_consumed_bps: margin_bps,
            counterparty: "Hyperliquid".into(),
            environment: MarketEnvironment::Testnet,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub enum LiveOrderState {
    #[default]
    Idle,
    Draft(NautilusOrderIntent),
    Review(NautilusOrderIntent),
    Sending {
        command_id: String,
    },
    Completed {
        intent: NautilusOrderIntent,
        receipt: CommandReceipt,
    },
    Failed {
        command_id: String,
        detail: String,
    },
}

impl LiveOrderState {
    pub fn accepted_intent(&self) -> Option<&NautilusOrderIntent> {
        let Self::Completed { intent, receipt } = self else {
            return None;
        };
        matches!(receipt.outcome, CommandOutcome::OrderAccepted { .. }).then_some(intent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_is_non_marketable_typed_and_exactly_reusable_for_confirmation() {
        let Some(intent) = NautilusOrderIntent::testnet_probe(65_000.0, 100_000, 42) else {
            assert!(false, "valid quote should produce a probe intent");
            return;
        };
        assert!(intent.preview.limit_price < 65_000.0);
        assert_eq!(intent.preview.limit_price % VENUE_TICK_SIZE, 0.0);
        let first = intent.place_request();
        let confirmed = intent.place_request();
        assert_eq!(first, confirmed);
        assert!(matches!(
            first.command,
            NautilusCommand::PlaceOrder {
                post_only: true,
                reduce_only: false,
                ..
            }
        ));

        let draft = NautilusOrderTicketSource::new(&intent).order_draft();
        assert_eq!(draft.instrument.as_ref(), "BTC-PERP");
        assert_eq!(draft.venue.as_ref(), "Hyperliquid");
        assert_eq!(draft.available_margin_cents, 100_000);
        assert_eq!(draft.limit_price, Some(intent.preview.limit_price));

        let confirmation = NautilusOrderConfirmationSource::new(&intent).order_confirmation();
        assert_eq!(confirmation.request_id.as_ref(), intent.command_id);
        assert_eq!(confirmation.counterparty.as_ref(), "Hyperliquid");
        assert_eq!(confirmation.environment, MarketEnvironment::Testnet);
        assert!(confirmation.exact_order.contains("POST ONLY"));
        assert_eq!(confirmation.headroom_consumed_bps, 585);
    }

    #[test]
    fn invalid_quotes_never_create_an_effectful_intent() {
        assert!(NautilusOrderIntent::testnet_probe(f64::NAN, 100, 1).is_none());
        assert!(NautilusOrderIntent::testnet_probe(0.0, 100, 1).is_none());
        assert!(NautilusOrderIntent::testnet_probe(65_000.0, 0, 1).is_none());
        assert!(NautilusOrderIntent::testnet_probe(65_000.0, 5_000, 1).is_none());
    }
}
