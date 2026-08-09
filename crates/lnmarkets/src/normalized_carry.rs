use lnmarkets_data::FeatureSnapshot;
use plugin_api::{
    CarrySurface, CarrySurfaceError, CarrySurfaceInput, CarrySurfaceProvider, CarrySurfaceRequest,
    ContractKind, ExpectedFundingPayment, FeeSchedule, MeasurementWindow, PositionSide,
    SettlementCadence, normalize_carry,
};

use crate::{LnMarketsPlugin, MANIFEST};

pub const LN_MARKETS_FUNDING_SETTLEMENT_INTERVAL_MS: u64 = 8 * 60 * 60 * 1_000;

impl CarrySurfaceProvider for LnMarketsPlugin {
    type FeatureSnapshot = FeatureSnapshot;

    fn carry_surface(
        &self,
        features: &Self::FeatureSnapshot,
        fee_schedule: FeeSchedule,
        request: CarrySurfaceRequest,
    ) -> Result<CarrySurface, CarrySurfaceError> {
        if request.reporting_numeraire != "USD" {
            return Err(unavailable(format!(
                "reporting numeraire {} is unsupported; LN Markets features are measured in USD",
                request.reporting_numeraire
            )));
        }
        let index_price = features
            .index
            .current_price
            .ok_or_else(|| unavailable("the index price is missing"))?;
        if !index_price.is_finite() || index_price <= 0.0 {
            return Err(unavailable(
                "the index price is not a positive finite value",
            ));
        }
        let (expected_rate, expectation_source) = match features.funding.ema {
            Some(rate) => (rate, "lnmarkets_funding_ema"),
            None => (
                features
                    .funding
                    .current_rate
                    .ok_or_else(|| unavailable("the expected funding rate is missing"))?,
                "lnmarkets_current_funding_rate",
            ),
        };
        if !expected_rate.is_finite() || !(-1.0..=1.0).contains(&expected_rate) {
            return Err(unavailable(
                "the expected funding rate is outside the supported range",
            ));
        }
        let started_at_ms = features
            .funding
            .measurement_started_at_ms
            .ok_or_else(|| unavailable("the funding measurement start is missing"))?;
        let ended_at_ms = features
            .funding
            .measurement_ended_at_ms
            .ok_or_else(|| unavailable("the funding measurement end is missing"))?;
        let samples = u64::try_from(features.funding.samples)
            .map_err(|_| unavailable("the funding sample count exceeded the supported range"))?;
        let signed_rate = match request.position_side {
            PositionSide::Short => expected_rate,
            PositionSide::Long => -expected_rate,
        };
        let clip_size = request.expected_slippage.clip_size_in_reporting_numeraire;
        let amount_in_bitcoin = clip_size * signed_rate / index_price;

        normalize_carry(CarrySurfaceInput {
            venue_id: MANIFEST.id.into(),
            contract_kind: ContractKind::Inverse,
            settlement_cadence: SettlementCadence {
                interval_ms: LN_MARKETS_FUNDING_SETTLEMENT_INTERVAL_MS,
            },
            measurement_window: MeasurementWindow {
                started_at_ms,
                ended_at_ms,
                samples,
            },
            expectation_source: expectation_source.into(),
            expected_funding_payment: ExpectedFundingPayment {
                settlement_asset: "BTC".into(),
                amount_in_settlement_asset: amount_in_bitcoin,
                settlement_asset_price_in_reporting_numeraire: index_price,
            },
            fee_schedule,
            request,
        })
    }
}

fn unavailable(reason: impl Into<String>) -> CarrySurfaceError {
    CarrySurfaceError::ProviderUnavailable {
        venue_id: MANIFEST.id.into(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use lnmarkets_data::{
        FundingFeatures, FundingSign, IndexFeatures, LiquidityFeatures, VolatilityFeatures,
    };
    use plugin_api::{ExpectedSlippage, PositionSide};

    use super::*;

    #[test]
    fn plugin_normalizes_inverse_bitcoin_funding_into_usd_carry() {
        let features = FeatureSnapshot {
            schema: "omega.lnmarkets.features.v1".into(),
            as_of_ms: Some(300),
            index: IndexFeatures {
                current_price: Some(20_000.0),
                one_hour_move: None,
                six_hours_move: None,
                one_day_move: None,
                price_points: 3,
            },
            volatility: VolatilityFeatures {
                one_hour: None,
                six_hours: None,
                one_day: None,
                price_points: 0,
            },
            funding: FundingFeatures {
                current_rate: Some(0.000_04),
                ema: Some(0.000_03),
                sign: FundingSign::Positive,
                sign_flipped_at_ms: None,
                measurement_started_at_ms: Some(100),
                measurement_ended_at_ms: Some(300),
                samples: 3,
            },
            liquidity: LiquidityFeatures {
                best_bid: None,
                best_ask: None,
                spread: None,
                spread_bps: None,
                bid_depth: None,
                ask_depth: None,
                tier_count: 0,
            },
            account_drift: None,
        };
        let surface = LnMarketsPlugin
            .carry_surface(
                &features,
                FeeSchedule {
                    entry_fee_bps: 2.0,
                    exit_fee_bps: 2.0,
                },
                CarrySurfaceRequest {
                    network: "signet".into(),
                    instrument: "BTC-USD perpetual".into(),
                    reporting_numeraire: "USD".into(),
                    position_side: PositionSide::Short,
                    expected_holding_period_ms: 30 * 24 * 60 * 60 * 1_000,
                    expected_slippage: ExpectedSlippage {
                        clip_size_in_reporting_numeraire: 10_000.0,
                        round_trip_bps: 2.0,
                    },
                    hedge_cost_bps: 1.0,
                    collateral_conversion_cost_bps: 1.0,
                },
            )
            .expect("carry surface");

        assert_eq!(surface.venue_id, MANIFEST.id);
        assert_eq!(surface.contract_kind, ContractKind::Inverse);
        assert_eq!(
            surface.settlement_cadence.interval_ms,
            LN_MARKETS_FUNDING_SETTLEMENT_INTERVAL_MS
        );
        assert_eq!(surface.measurement_window.samples, 3);
        assert_close(surface.expected_funding_rate_per_settlement_bps, 0.3);
        assert!(surface.annualized_net_carry_bps < surface.annualized_expected_funding_bps);
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }
}
