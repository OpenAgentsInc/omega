//! Command-center number and color helpers.
//!
//! The number grouping, sats/BTC/USD formatting, and colorblind-safe up/down
//! semantics all come from the shared financial kit in `crates/ui`
//! (`ui::MarketTokens`, `ui::MarketDirection`, `ui::format_sats`, …) so every
//! market surface agrees on one formatting and color path. The helpers here
//! are the thin command-center-specific layer on top: signed money wrappers,
//! the prediction/probability/basis-point/duration formatters the kit does
//! not carry, and the `App`-resolved direction colors that turn a signed
//! quantity into one of the kit's tokens.

use chrono::{TimeZone as _, Utc};
use gpui::App;
use ui::{
    Color, MarketDirection, MarketTokens, format_btc as kit_format_btc, format_grouped_u64,
    format_sats as kit_format_sats,
};

/// Unsigned sats with a leading `-` when negative: `"1,234,567 sats"`.
pub fn format_sats(sats: i64) -> String {
    let sign = if sats < 0 { "-" } else { "" };
    format!("{sign}{}", kit_format_sats(sats.unsigned_abs()))
}

/// Sats that always show their direction: `"+42,000 sats"`, `"-1,000 sats"`,
/// `"0 sats"`.
pub fn format_signed_sats(sats: i64) -> String {
    let sign = match sats {
        value if value > 0 => "+",
        value if value < 0 => "-",
        _ => "",
    };
    format!("{sign}{}", kit_format_sats(sats.unsigned_abs()))
}

/// BTC with a leading `-` when negative; eight decimals via the kit.
pub fn format_btc(sats: i64) -> String {
    let sign = if sats < 0 { "-" } else { "" };
    format!("{sign}{}", kit_format_btc(sats.unsigned_abs()))
}

/// Whole-dollar USD with grouped thousands: `"$1,234"`.
pub fn format_usd(usd: u64) -> String {
    format!("${}", format_grouped_u64(usd))
}

/// Formats a micro-probability (`prediction_events::PROBABILITY_SCALE`) as a
/// percentage.
pub fn format_probability_micros(micros: u32) -> String {
    let tenths = u64::from(micros) / 1_000;
    if tenths % 10 == 0 {
        format!("{}%", tenths / 10)
    } else {
        format!("{}.{}%", tenths / 10, tenths % 10)
    }
}

pub fn format_percent_bps(bps: u32) -> String {
    if bps.is_multiple_of(100) {
        format!("{}%", bps / 100)
    } else {
        format!("{}.{:02}%", bps / 100, bps % 100)
    }
}

pub fn format_duration_ms(milliseconds: i64) -> String {
    let total_seconds = milliseconds.max(0) / 1_000;
    let (days, hours, minutes, seconds) = (
        total_seconds / 86_400,
        (total_seconds % 86_400) / 3_600,
        (total_seconds % 3_600) / 60,
        total_seconds % 60,
    );
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// A deadline relative to now: "in 2h 14m" before it, "3m 05s ago" after it.
pub fn format_countdown(deadline_ms: i64, now_ms: i64) -> String {
    if deadline_ms >= now_ms {
        format!("in {}", format_duration_ms(deadline_ms - now_ms))
    } else {
        format!("{} ago", format_duration_ms(now_ms - deadline_ms))
    }
}

/// A UTC wall-clock timestamp for feed rows.
pub fn format_wall_clock(at_ms: i64) -> String {
    match Utc.timestamp_millis_opt(at_ms).single() {
        Some(time) => time.format("%H:%M:%S").to_string(),
        None => format!("{at_ms} ms"),
    }
}

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
