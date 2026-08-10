//! Command-center number and color helpers.
//!
//! The number grouping, sats/BTC/USD formatting, and colorblind-safe up/down
//! semantics all come from the shared financial kit in `crates/ui`
//! (`ui::MarketTokens`, `ui::MarketDirection`, `ui::format_sats`, …) so every
//! market surface agrees on one formatting and color path. The helpers here
//! are re-exports of that one path. This module only retains the `App`-resolved
//! direction mapping from prediction domain types into the kit's tokens.

use gpui::App;
use ui::{Color, MarketDirection, MarketTokens};

pub use ui::{
    format_countdown, format_duration_ms, format_percent_bps, format_probability_micros,
    format_sats_change as format_signed_sats, format_signed_btc as format_btc,
    format_signed_sats_amount as format_sats, format_usd, format_wall_clock,
};

/// The shared kit's colorblind-safe token for a signed quantity's direction:
/// blue up for gains, orange down for losses, muted when flat.
pub fn signed_color(value: i64, cx: &App) -> Color {
    token_color(MarketDirection::of_i64(value), cx)
}

/// The same up/down token for a prediction's directional call.
pub fn direction_color(direction: prediction_events::PredictedDirection, cx: &App) -> Color {
    token_color(market_direction(direction), cx)
}

fn token_color(direction: MarketDirection, cx: &App) -> Color {
    Color::Custom(MarketTokens::from_theme(cx).direction_color(direction))
}

fn market_direction(direction: prediction_events::PredictedDirection) -> MarketDirection {
    use prediction_events::PredictedDirection;
    match direction {
        PredictedDirection::Up => MarketDirection::Up,
        PredictedDirection::Down => MarketDirection::Down,
        PredictedDirection::Flat => MarketDirection::Flat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sats_formatting_groups_thousands_and_keeps_sign() {
        assert_eq!(format_sats(0), "0 sats");
        assert_eq!(format_sats(1_234_567), "1,234,567 sats");
        assert_eq!(format_sats(-980), "-980 sats");
        assert_eq!(format_signed_sats(42_000), "+42,000 sats");
        assert_eq!(format_signed_sats(-1_000_000), "-1,000,000 sats");
        assert_eq!(format_signed_sats(0), "0 sats");
    }

    #[test]
    fn btc_formatting_keeps_eight_decimals() {
        assert_eq!(format_btc(150_000_000), "1.50000000 BTC");
        assert_eq!(format_btc(-1), "-0.00000001 BTC");
    }

    #[test]
    fn usd_groups_thousands() {
        assert_eq!(format_usd(0), "$0");
        assert_eq!(format_usd(1_234_567), "$1,234,567");
    }

    #[test]
    fn probability_and_bps_formatting() {
        assert_eq!(format_probability_micros(720_000), "72%");
        assert_eq!(format_probability_micros(333_000), "33.3%");
        assert_eq!(format_percent_bps(25), "0.25%");
        assert_eq!(format_percent_bps(1_500), "15%");
    }

    #[test]
    fn durations_and_countdowns() {
        assert_eq!(format_duration_ms(42_000), "42s");
        assert_eq!(format_duration_ms(3 * 60_000 + 5_000), "3m 05s");
        assert_eq!(format_duration_ms(2 * 3_600_000 + 14 * 60_000), "2h 14m");
        assert_eq!(format_duration_ms(3 * 86_400_000 + 4 * 3_600_000), "3d 4h");
        assert_eq!(format_countdown(10_000, 5_000), "in 5s");
        assert_eq!(format_countdown(5_000, 10_000), "5s ago");
    }

    #[test]
    fn direction_maps_onto_the_shared_kit() {
        assert_eq!(
            market_direction(prediction_events::PredictedDirection::Up),
            MarketDirection::Up
        );
        assert_eq!(
            market_direction(prediction_events::PredictedDirection::Down),
            MarketDirection::Down
        );
        assert_eq!(
            market_direction(prediction_events::PredictedDirection::Flat),
            MarketDirection::Flat
        );
    }
}
