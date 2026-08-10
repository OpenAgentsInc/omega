use std::collections::BTreeSet;

use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, FontWeight, SharedString, Window, relative};
use trading_mandate::{AssetId, MandateSnapshot, ReviewCadence, TradingMandate, TradingNetwork};
use ui::prelude::*;

use crate::format::{format_countdown, format_percent_bps, format_sats, format_usd};

const WARN_FRACTION: f32 = 0.7;
const CRITICAL_FRACTION: f32 = 0.9;

/// Current usage against each mandate limit, gathered by the caller from
/// the ledger and venue state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MandateUsage {
    /// Balance held at the venue, in the mandate's collateral asset.
    pub venue_balance: u64,
    pub position_notional_usd: u64,
    /// Current effective leverage in hundredths (250 = 2.5x).
    pub leverage_hundredths: u32,
    /// Loss so far today, in the mandate's collateral asset.
    pub daily_loss: u64,
    pub orders_this_hour: u32,
    /// Distance to liquidation in basis points; `None` while flat.
    pub liquidation_distance_bps: Option<u32>,
}

/// The threshold zone a meter's fill sits in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeterZone {
    Safe,
    Warn,
    Critical,
}

impl MeterZone {
    pub fn from_fraction(fraction: f32) -> Self {
        if fraction >= CRITICAL_FRACTION {
            Self::Critical
        } else if fraction >= WARN_FRACTION {
            Self::Warn
        } else {
            Self::Safe
        }
    }

    pub fn color(self) -> Color {
        match self {
            Self::Safe => Color::Success,
            Self::Warn => Color::Warning,
            Self::Critical => Color::Error,
        }
    }
}

/// One per-limit headroom meter: how much of the limit is consumed.
#[derive(Clone, Debug)]
pub struct HeadroomMeter {
    pub label: SharedString,
    pub used_display: SharedString,
    pub limit_display: SharedString,
    /// Fraction of the limit consumed, unclamped so tests can see overshoot.
    pub fraction: f32,
}

impl HeadroomMeter {
    pub fn zone(&self) -> MeterZone {
        MeterZone::from_fraction(self.fraction)
    }
}

fn fraction(used: u64, limit: u64) -> f32 {
    if limit == 0 {
        return 1.0;
    }
    used as f32 / limit as f32
}

fn collateral_amount(amount: u64, asset: &AssetId) -> String {
    let signed = i64::try_from(amount).unwrap_or(i64::MAX);
    if asset.is_sats() {
        format_sats(signed)
    } else {
        format!("{signed} {asset}")
    }
}

/// The mandate status card: scope line, expiry countdown, and per-limit
/// headroom meters. Value-typed; build it from a [`MandateSnapshot`] plus a
/// [`MandateUsage`] via [`MandateStatusCard::from_snapshot`].
#[derive(IntoElement, RegisterComponent)]
pub struct MandateStatusCard {
    venue: SharedString,
    network: TradingNetwork,
    collateral: SharedString,
    objective: SharedString,
    revision: u64,
    expires_at_ms: i64,
    now_ms: i64,
    meters: Vec<HeadroomMeter>,
}

impl MandateStatusCard {
    pub fn new(mandate: &TradingMandate, usage: &MandateUsage, revision: u64, now_ms: i64) -> Self {
        Self {
            venue: mandate.venue.clone().into(),
            network: mandate.network,
            collateral: mandate.collateral_asset.to_string().into(),
            objective: mandate.objective.clone().into(),
            revision,
            expires_at_ms: mandate.expires_at_ms,
            now_ms,
            meters: Self::meters(mandate, usage),
        }
    }

    /// Adapter from the mandate store's snapshot for one (venue, network)
    /// scope. Returns `None` when no mandate governs that scope.
    pub fn from_snapshot(
        snapshot: &MandateSnapshot,
        venue: &str,
        network: TradingNetwork,
        usage: &MandateUsage,
        now_ms: i64,
    ) -> Option<Self> {
        let mandate = snapshot.mandate_for(venue, network)?;
        Some(Self::new(mandate, usage, snapshot.revision, now_ms))
    }

    pub fn meters(mandate: &TradingMandate, usage: &MandateUsage) -> Vec<HeadroomMeter> {
        let asset = &mandate.collateral_asset;
        let mut meters = vec![
            HeadroomMeter {
                label: "Venue balance".into(),
                used_display: collateral_amount(usage.venue_balance, asset).into(),
                limit_display: collateral_amount(mandate.max_venue_balance, asset).into(),
                fraction: fraction(usage.venue_balance, mandate.max_venue_balance),
            },
            HeadroomMeter {
                label: "Position notional".into(),
                used_display: format_usd(usage.position_notional_usd).into(),
                limit_display: format_usd(mandate.max_position_usd).into(),
                fraction: fraction(usage.position_notional_usd, mandate.max_position_usd),
            },
            HeadroomMeter {
                label: "Leverage".into(),
                used_display: format!(
                    "{}.{:02}x",
                    usage.leverage_hundredths / 100,
                    usage.leverage_hundredths % 100
                )
                .into(),
                limit_display: format!("{}x", mandate.max_leverage).into(),
                fraction: fraction(
                    u64::from(usage.leverage_hundredths),
                    u64::from(mandate.max_leverage) * 100,
                ),
            },
            HeadroomMeter {
                label: "Daily loss stop".into(),
                used_display: collateral_amount(usage.daily_loss, asset).into(),
                limit_display: collateral_amount(mandate.daily_loss_stop, asset).into(),
                fraction: fraction(usage.daily_loss, mandate.daily_loss_stop),
            },
            HeadroomMeter {
                label: "Order rate".into(),
                used_display: format!("{}/h", usage.orders_this_hour).into(),
                limit_display: format!("{}/h", mandate.max_orders_per_hour).into(),
                fraction: fraction(
                    u64::from(usage.orders_this_hour),
                    u64::from(mandate.max_orders_per_hour),
                ),
            },
        ];
        // The liquidation meter is inverted: the limit is a floor, so the
        // fill grows as the live distance shrinks toward the required
        // minimum buffer.
        let liquidation = match usage.liquidation_distance_bps {
            Some(distance_bps) => HeadroomMeter {
                label: "Liquidation buffer".into(),
                used_display: format_percent_bps(distance_bps).into(),
                limit_display: format!(
                    "min {}",
                    format_percent_bps(mandate.min_liquidation_buffer_bps)
                )
                .into(),
                fraction: fraction(
                    u64::from(mandate.min_liquidation_buffer_bps),
                    u64::from(distance_bps.max(1)),
                ),
            },
            None => HeadroomMeter {
                label: "Liquidation buffer".into(),
                used_display: "flat".into(),
                limit_display: format!(
                    "min {}",
                    format_percent_bps(mandate.min_liquidation_buffer_bps)
                )
                .into(),
                fraction: 0.0,
            },
        };
        meters.push(liquidation);
        meters
    }
}

fn meter_row(meter: &HeadroomMeter, cx: &App) -> AnyElement {
    let zone = meter.zone();
    let fill = meter.fraction.clamp(0.0, 1.0);
    v_flex()
        .w_full()
        .gap_0p5()
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .gap_2()
                .child(Label::new(meter.label.clone()).size(LabelSize::XSmall))
                .child(
                    Label::new(format!("{} / {}", meter.used_display, meter.limit_display))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        )
        .child(
            div()
                .relative()
                .w_full()
                .h_1p5()
                .rounded_full()
                .bg(cx.theme().colors().element_background)
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .h_full()
                        .rounded_full()
                        .w(relative(fill))
                        .bg(zone.color().color(cx)),
                )
                .children([WARN_FRACTION, CRITICAL_FRACTION].map(|threshold| {
                    div()
                        .absolute()
                        .top_0()
                        .left(relative(threshold))
                        .h_full()
                        .w(px(1.))
                        .bg(cx.theme().colors().border)
                })),
        )
        .into_any_element()
}

impl RenderOnce for MandateStatusCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let expired = self.expires_at_ms <= self.now_ms;
        let expires_soon = !expired && self.expires_at_ms - self.now_ms < 86_400_000;
        let expiry_color = if expired {
            Color::Error
        } else if expires_soon {
            Color::Warning
        } else {
            Color::Muted
        };
        let expiry = if expired {
            format!("expired {}", format_countdown(self.expires_at_ms, self.now_ms))
        } else {
            format!("expires {}", format_countdown(self.expires_at_ms, self.now_ms))
        };
        let network = match self.network {
            TradingNetwork::Signet => "signet",
            TradingNetwork::Mainnet => "mainnet",
        };

        v_flex()
            .id("command-center-mandate-card")
            .debug_selector(|| "command_center.mandate_card".into())
            .w_full()
            .max_w(px(520.))
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().surface_background)
            .overflow_hidden()
            .child(
                v_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_0p5()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Trading mandate"),
                            )
                            .child(
                                Label::new(expiry)
                                    .size(LabelSize::XSmall)
                                    .color(expiry_color),
                            ),
                    )
                    .child(
                        Label::new(format!(
                            "{} · {} · {} collateral · revision {}",
                            self.venue, network, self.collateral, self.revision
                        ))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    )
                    .child(
                        Label::new(self.objective)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .children(self.meters.iter().map(|meter| meter_row(meter, cx))),
            )
    }
}

/// Demo mandate at interesting fill levels for the component library.
pub(crate) fn demo_mandate() -> TradingMandate {
    TradingMandate {
        venue: "lnmarkets".into(),
        network: TradingNetwork::Signet,
        collateral_asset: AssetId::sats(),
        objective: "Bound automated carry and rebalance risk".into(),
        max_venue_balance: 250_000,
        max_position_usd: 500,
        max_leverage: 3,
        daily_loss_stop: 10_000,
        max_orders_per_hour: 12,
        min_liquidation_buffer_bps: 1_500,
        allowed_strategies: BTreeSet::from(["funding_carry".into(), "threshold_swing".into()]),
        review_cadence: ReviewCadence::Interval { seconds: 900 },
        expires_at_ms: 1_786_500_000_000,
    }
}

impl Component for MandateStatusCard {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        "Mandate scope, expiry countdown, and per-limit headroom meters with safe/warn/critical threshold zones."
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        let mandate = demo_mandate();
        let now_ms = 1_786_276_800_000_i64;
        let comfortable = MandateUsage {
            venue_balance: 96_000,
            position_notional_usd: 120,
            leverage_hundredths: 110,
            daily_loss: 900,
            orders_this_hour: 2,
            liquidation_distance_bps: Some(6_400),
        };
        let stressed = MandateUsage {
            venue_balance: 205_000,
            position_notional_usd: 460,
            leverage_hundredths: 280,
            daily_loss: 7_600,
            orders_this_hour: 11,
            liquidation_distance_bps: Some(1_650),
        };
        let flat = MandateUsage {
            venue_balance: 40_000,
            position_notional_usd: 0,
            leverage_hundredths: 0,
            daily_loss: 0,
            orders_this_hour: 0,
            liquidation_distance_bps: None,
        };
        let mut expiring = demo_mandate();
        expiring.expires_at_ms = now_ms + 3 * 3_600_000;

        v_flex()
            .gap_6()
            .child(
                example_group_with_title(
                    "Mandate status",
                    vec![
                        single_example(
                            "Comfortable",
                            MandateStatusCard::new(&mandate, &comfortable, 4, now_ms)
                                .into_any_element(),
                        )
                        .width(px(520.)),
                        single_example(
                            "Near limits",
                            MandateStatusCard::new(&mandate, &stressed, 4, now_ms)
                                .into_any_element(),
                        )
                        .width(px(520.)),
                        single_example(
                            "Flat, expiring soon",
                            MandateStatusCard::new(&expiring, &flat, 5, now_ms).into_any_element(),
                        )
                        .width(px(520.)),
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
    fn meter_zones_split_at_the_thresholds() {
        assert_eq!(MeterZone::from_fraction(0.0), MeterZone::Safe);
        assert_eq!(MeterZone::from_fraction(0.69), MeterZone::Safe);
        assert_eq!(MeterZone::from_fraction(0.7), MeterZone::Warn);
        assert_eq!(MeterZone::from_fraction(0.89), MeterZone::Warn);
        assert_eq!(MeterZone::from_fraction(0.9), MeterZone::Critical);
        assert_eq!(MeterZone::from_fraction(1.4), MeterZone::Critical);
    }

    #[test]
    fn meters_cover_every_mandate_limit() {
        let mandate = demo_mandate();
        let usage = MandateUsage {
            venue_balance: 125_000,
            position_notional_usd: 250,
            leverage_hundredths: 150,
            daily_loss: 5_000,
            orders_this_hour: 6,
            liquidation_distance_bps: Some(3_000),
        };
        let meters = MandateStatusCard::meters(&mandate, &usage);
        let labels: Vec<_> = meters.iter().map(|meter| meter.label.to_string()).collect();
        assert_eq!(
            labels,
            [
                "Venue balance",
                "Position notional",
                "Leverage",
                "Daily loss stop",
                "Order rate",
                "Liquidation buffer",
            ]
        );
        assert!((meters[0].fraction - 0.5).abs() < 1e-6);
        assert!((meters[2].fraction - 0.5).abs() < 1e-6);
        assert!((meters[4].fraction - 0.5).abs() < 1e-6);
        assert!((meters[5].fraction - 0.5).abs() < 1e-6, "liquidation meter inverts: min buffer over live distance");
    }

    #[test]
    fn flat_position_shows_an_empty_liquidation_meter() {
        let mandate = demo_mandate();
        let usage = MandateUsage::default();
        let meters = MandateStatusCard::meters(&mandate, &usage);
        let liquidation = meters.last().expect("liquidation meter is always present");
        assert_eq!(liquidation.fraction, 0.0);
        assert_eq!(liquidation.used_display.as_ref(), "flat");
    }

    #[test]
    fn from_snapshot_selects_the_scoped_mandate() {
        let snapshot = MandateSnapshot {
            revision: 9,
            mandates: vec![demo_mandate()],
        };
        let usage = MandateUsage::default();
        assert!(
            MandateStatusCard::from_snapshot(
                &snapshot,
                "lnmarkets",
                TradingNetwork::Signet,
                &usage,
                0,
            )
            .is_some()
        );
        assert!(
            MandateStatusCard::from_snapshot(
                &snapshot,
                "lnmarkets",
                TradingNetwork::Mainnet,
                &usage,
                0,
            )
            .is_none()
        );
    }
}
