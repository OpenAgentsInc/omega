//! Venue-neutral carry normalization for allocator-facing comparisons.
//!
//! A carry surface converts a venue-native expected funding payment into a
//! common reporting numeraire, annualizes it using the venue's explicit
//! settlement cadence, and subtracts annualized execution and hedge costs at
//! a stated clip size and holding period.
//!
//! This contract deliberately excludes risk adjustment and counterparty
//! weighting. Those are allocator constraints backed by mandates and the
//! counterparty-exposure ledger; folding them into carry would make an
//! economically identical quote change when authority or venue balances
//! change. The surface is measurement, not allocation policy.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CARRY_SURFACE_SCHEMA: &str = "omega.carry_surface.v1";
pub const MILLISECONDS_PER_YEAR: u64 = 365 * 24 * 60 * 60 * 1_000;
const BASIS_POINTS_PER_UNIT: f64 = 10_000.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractKind {
    Inverse,
    Linear,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionSide {
    Long,
    Short,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettlementCadence {
    pub interval_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MeasurementWindow {
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    pub samples: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeeSchedule {
    pub entry_fee_bps: f64,
    pub exit_fee_bps: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExpectedSlippage {
    pub clip_size_in_reporting_numeraire: f64,
    pub round_trip_bps: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CarrySurfaceRequest {
    pub network: String,
    pub instrument: String,
    pub reporting_numeraire: String,
    pub position_side: PositionSide,
    pub expected_holding_period_ms: u64,
    pub expected_slippage: ExpectedSlippage,
    pub hedge_cost_bps: f64,
    pub collateral_conversion_cost_bps: f64,
}

/// One venue-native expected settlement payment and the conversion price that
/// makes it comparable in the requested reporting numeraire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExpectedFundingPayment {
    pub settlement_asset: String,
    pub amount_in_settlement_asset: f64,
    pub settlement_asset_price_in_reporting_numeraire: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CarrySurfaceInput {
    pub venue_id: String,
    pub contract_kind: ContractKind,
    pub settlement_cadence: SettlementCadence,
    pub measurement_window: MeasurementWindow,
    pub expectation_source: String,
    pub expected_funding_payment: ExpectedFundingPayment,
    pub fee_schedule: FeeSchedule,
    pub request: CarrySurfaceRequest,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CarryCostBreakdown {
    pub entry_fee_bps: f64,
    pub exit_fee_bps: f64,
    pub expected_round_trip_slippage_bps: f64,
    pub hedge_cost_bps: f64,
    pub collateral_conversion_cost_bps: f64,
    pub total_round_trip_cost_bps: f64,
    pub annualized_cost_bps: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CarrySurface {
    pub schema: String,
    pub venue_id: String,
    pub network: String,
    pub instrument: String,
    pub reporting_numeraire: String,
    pub position_side: PositionSide,
    pub contract_kind: ContractKind,
    pub settlement_cadence: SettlementCadence,
    pub measurement_window: MeasurementWindow,
    pub expectation_source: String,
    pub settlement_asset: String,
    pub clip_size_in_reporting_numeraire: f64,
    pub expected_holding_period_ms: u64,
    pub expected_funding_rate_per_settlement_bps: f64,
    pub annualized_expected_funding_bps: f64,
    pub annualized_expected_funding_in_reporting_numeraire: f64,
    pub costs: CarryCostBreakdown,
    pub annualized_net_carry_bps: f64,
    pub annualized_net_carry_in_reporting_numeraire: f64,
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum CarrySurfaceError {
    #[error("carry surface field {field} must not be blank")]
    BlankField { field: &'static str },
    #[error("carry surface field {field} must be a finite {requirement}")]
    InvalidNumber {
        field: &'static str,
        requirement: &'static str,
    },
    #[error("carry surface field {field} must be greater than zero")]
    ZeroValue { field: &'static str },
    #[error("carry measurement window ends before it starts")]
    ReversedMeasurementWindow,
    #[error("{venue_id} carry surface is unavailable: {reason}")]
    ProviderUnavailable { venue_id: String, reason: String },
}

/// A venue plugin implements this conversion against its typed feature
/// snapshot while sharing the fee and output contract with every other venue.
pub trait CarrySurfaceProvider {
    type FeatureSnapshot;

    fn carry_surface(
        &self,
        features: &Self::FeatureSnapshot,
        fee_schedule: FeeSchedule,
        request: CarrySurfaceRequest,
    ) -> Result<CarrySurface, CarrySurfaceError>;
}

pub fn normalize_carry(input: CarrySurfaceInput) -> Result<CarrySurface, CarrySurfaceError> {
    require_text(&input.venue_id, "venue_id")?;
    require_text(&input.request.network, "network")?;
    require_text(&input.request.instrument, "instrument")?;
    require_text(&input.request.reporting_numeraire, "reporting_numeraire")?;
    require_text(&input.expectation_source, "expectation_source")?;
    require_text(
        &input.expected_funding_payment.settlement_asset,
        "settlement_asset",
    )?;

    if input.settlement_cadence.interval_ms == 0 {
        return Err(CarrySurfaceError::ZeroValue {
            field: "settlement_cadence.interval_ms",
        });
    }
    if input.measurement_window.samples == 0 {
        return Err(CarrySurfaceError::ZeroValue {
            field: "measurement_window.samples",
        });
    }
    if input.measurement_window.ended_at_ms < input.measurement_window.started_at_ms {
        return Err(CarrySurfaceError::ReversedMeasurementWindow);
    }
    if input.request.expected_holding_period_ms == 0 {
        return Err(CarrySurfaceError::ZeroValue {
            field: "expected_holding_period_ms",
        });
    }

    require_positive_finite(
        input
            .request
            .expected_slippage
            .clip_size_in_reporting_numeraire,
        "expected_slippage.clip_size_in_reporting_numeraire",
    )?;
    require_finite(
        input.expected_funding_payment.amount_in_settlement_asset,
        "expected_funding_payment.amount_in_settlement_asset",
        "number",
    )?;
    require_positive_finite(
        input
            .expected_funding_payment
            .settlement_asset_price_in_reporting_numeraire,
        "expected_funding_payment.settlement_asset_price_in_reporting_numeraire",
    )?;
    for (value, field) in [
        (input.fee_schedule.entry_fee_bps, "entry_fee_bps"),
        (input.fee_schedule.exit_fee_bps, "exit_fee_bps"),
        (
            input.request.expected_slippage.round_trip_bps,
            "expected_slippage.round_trip_bps",
        ),
        (input.request.hedge_cost_bps, "hedge_cost_bps"),
        (
            input.request.collateral_conversion_cost_bps,
            "collateral_conversion_cost_bps",
        ),
    ] {
        require_non_negative_finite(value, field)?;
    }

    let clip_size = input
        .request
        .expected_slippage
        .clip_size_in_reporting_numeraire;
    let expected_payment_in_reporting_numeraire =
        input.expected_funding_payment.amount_in_settlement_asset
            * input
                .expected_funding_payment
                .settlement_asset_price_in_reporting_numeraire;
    require_finite(
        expected_payment_in_reporting_numeraire,
        "expected_funding_payment_in_reporting_numeraire",
        "number",
    )?;
    let funding_rate_per_settlement = expected_payment_in_reporting_numeraire / clip_size;
    require_finite(
        funding_rate_per_settlement,
        "expected_funding_rate_per_settlement",
        "number",
    )?;

    let settlements_per_year =
        MILLISECONDS_PER_YEAR as f64 / input.settlement_cadence.interval_ms as f64;
    let annualized_expected_funding_bps =
        funding_rate_per_settlement * settlements_per_year * BASIS_POINTS_PER_UNIT;
    let total_round_trip_cost_bps = input.fee_schedule.entry_fee_bps
        + input.fee_schedule.exit_fee_bps
        + input.request.expected_slippage.round_trip_bps
        + input.request.hedge_cost_bps
        + input.request.collateral_conversion_cost_bps;
    let holding_periods_per_year =
        MILLISECONDS_PER_YEAR as f64 / input.request.expected_holding_period_ms as f64;
    let annualized_cost_bps = total_round_trip_cost_bps * holding_periods_per_year;
    let annualized_net_carry_bps = annualized_expected_funding_bps - annualized_cost_bps;
    for (value, field) in [
        (
            annualized_expected_funding_bps,
            "annualized_expected_funding_bps",
        ),
        (annualized_cost_bps, "annualized_cost_bps"),
        (annualized_net_carry_bps, "annualized_net_carry_bps"),
    ] {
        require_finite(value, field, "number")?;
    }

    let annualized_expected_funding_in_reporting_numeraire =
        clip_size * annualized_expected_funding_bps / BASIS_POINTS_PER_UNIT;
    let annualized_net_carry_in_reporting_numeraire =
        clip_size * annualized_net_carry_bps / BASIS_POINTS_PER_UNIT;
    let costs = CarryCostBreakdown {
        entry_fee_bps: input.fee_schedule.entry_fee_bps,
        exit_fee_bps: input.fee_schedule.exit_fee_bps,
        expected_round_trip_slippage_bps: input.request.expected_slippage.round_trip_bps,
        hedge_cost_bps: input.request.hedge_cost_bps,
        collateral_conversion_cost_bps: input.request.collateral_conversion_cost_bps,
        total_round_trip_cost_bps,
        annualized_cost_bps,
    };

    Ok(CarrySurface {
        schema: CARRY_SURFACE_SCHEMA.into(),
        venue_id: input.venue_id,
        network: input.request.network,
        instrument: input.request.instrument,
        reporting_numeraire: input.request.reporting_numeraire,
        position_side: input.request.position_side,
        contract_kind: input.contract_kind,
        settlement_cadence: input.settlement_cadence,
        measurement_window: input.measurement_window,
        expectation_source: input.expectation_source,
        settlement_asset: input.expected_funding_payment.settlement_asset,
        clip_size_in_reporting_numeraire: clip_size,
        expected_holding_period_ms: input.request.expected_holding_period_ms,
        expected_funding_rate_per_settlement_bps: funding_rate_per_settlement
            * BASIS_POINTS_PER_UNIT,
        annualized_expected_funding_bps,
        annualized_expected_funding_in_reporting_numeraire,
        costs,
        annualized_net_carry_bps,
        annualized_net_carry_in_reporting_numeraire,
    })
}

fn require_text(value: &str, field: &'static str) -> Result<(), CarrySurfaceError> {
    if value.trim().is_empty() {
        return Err(CarrySurfaceError::BlankField { field });
    }
    Ok(())
}

fn require_finite(
    value: f64,
    field: &'static str,
    requirement: &'static str,
) -> Result<(), CarrySurfaceError> {
    if !value.is_finite() {
        return Err(CarrySurfaceError::InvalidNumber { field, requirement });
    }
    Ok(())
}

fn require_positive_finite(value: f64, field: &'static str) -> Result<(), CarrySurfaceError> {
    if !value.is_finite() || value <= 0.0 {
        return Err(CarrySurfaceError::InvalidNumber {
            field,
            requirement: "positive number",
        });
    }
    Ok(())
}

fn require_non_negative_finite(value: f64, field: &'static str) -> Result<(), CarrySurfaceError> {
    if !value.is_finite() || value < 0.0 {
        return Err(CarrySurfaceError::InvalidNumber {
            field,
            requirement: "non-negative number",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EIGHT_HOURS_MS: u64 = 8 * 60 * 60 * 1_000;
    const THIRTY_DAYS_MS: u64 = 30 * 24 * 60 * 60 * 1_000;

    fn request() -> CarrySurfaceRequest {
        CarrySurfaceRequest {
            network: "testnet".into(),
            instrument: "BTC-USD perpetual".into(),
            reporting_numeraire: "USD".into(),
            position_side: PositionSide::Short,
            expected_holding_period_ms: THIRTY_DAYS_MS,
            expected_slippage: ExpectedSlippage {
                clip_size_in_reporting_numeraire: 10_000.0,
                round_trip_bps: 2.0,
            },
            hedge_cost_bps: 1.0,
            collateral_conversion_cost_bps: 1.0,
        }
    }

    fn input(
        contract_kind: ContractKind,
        settlement_asset: &str,
        payment_amount: f64,
        asset_price: f64,
    ) -> CarrySurfaceInput {
        CarrySurfaceInput {
            venue_id: format!("{contract_kind:?}").to_lowercase(),
            contract_kind,
            settlement_cadence: SettlementCadence {
                interval_ms: EIGHT_HOURS_MS,
            },
            measurement_window: MeasurementWindow {
                started_at_ms: 1,
                ended_at_ms: 2,
                samples: 90,
            },
            expectation_source: "synthetic_fixture".into(),
            expected_funding_payment: ExpectedFundingPayment {
                settlement_asset: settlement_asset.into(),
                amount_in_settlement_asset: payment_amount,
                settlement_asset_price_in_reporting_numeraire: asset_price,
            },
            fee_schedule: FeeSchedule {
                entry_fee_bps: 2.0,
                exit_fee_bps: 2.0,
            },
            request: request(),
        }
    }

    #[test]
    fn inverse_and_linear_contracts_normalize_the_same_synthetic_market() {
        let inverse = normalize_carry(input(ContractKind::Inverse, "BTC", 0.000_015, 20_000.0))
            .expect("inverse carry");
        let linear =
            normalize_carry(input(ContractKind::Linear, "USDC", 0.3, 1.0)).expect("linear carry");

        assert_close(
            inverse.expected_funding_rate_per_settlement_bps,
            linear.expected_funding_rate_per_settlement_bps,
        );
        assert_close(
            inverse.annualized_expected_funding_bps,
            linear.annualized_expected_funding_bps,
        );
        assert_close(
            inverse.annualized_net_carry_bps,
            linear.annualized_net_carry_bps,
        );
        assert_eq!(inverse.contract_kind, ContractKind::Inverse);
        assert_eq!(linear.contract_kind, ContractKind::Linear);
    }

    #[test]
    fn settlement_cadence_changes_annualized_funding_without_hiding_the_cadence() {
        let eight_hour = normalize_carry(input(ContractKind::Linear, "USDC", 0.3, 1.0))
            .expect("eight-hour carry");
        let mut hourly_input = input(ContractKind::Linear, "USDC", 0.3, 1.0);
        hourly_input.settlement_cadence.interval_ms = 60 * 60 * 1_000;
        let hourly = normalize_carry(hourly_input).expect("hourly carry");

        assert_close(
            hourly.annualized_expected_funding_bps,
            eight_hour.annualized_expected_funding_bps * 8.0,
        );
        assert_eq!(hourly.settlement_cadence.interval_ms, 60 * 60 * 1_000);
        assert_eq!(eight_hour.settlement_cadence.interval_ms, EIGHT_HOURS_MS);
    }

    #[test]
    fn higher_fees_reduce_net_carry_and_remain_itemized() {
        let low_fee = normalize_carry(input(ContractKind::Inverse, "BTC", 0.000_015, 20_000.0))
            .expect("low-fee carry");
        let mut high_fee_input = input(ContractKind::Inverse, "BTC", 0.000_015, 20_000.0);
        high_fee_input.fee_schedule = FeeSchedule {
            entry_fee_bps: 7.0,
            exit_fee_bps: 7.0,
        };
        let high_fee = normalize_carry(high_fee_input).expect("high-fee carry");

        assert!(high_fee.annualized_net_carry_bps < low_fee.annualized_net_carry_bps);
        assert_close(high_fee.costs.entry_fee_bps, 7.0);
        assert_close(high_fee.costs.exit_fee_bps, 7.0);
        assert!(high_fee.costs.annualized_cost_bps > low_fee.costs.annualized_cost_bps);
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }
}
