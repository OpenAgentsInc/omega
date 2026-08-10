//! Minimal local number-formatting and up/down color shims.
//!
//! A shared financial theme-token and formatting kit is landing in
//! `crates/ui` as part of omega#284; when it does, these helpers get
//! replaced by that kit so every market surface agrees on formatting and
//! colorblind-safe up/down semantics. Until then the command-center
//! components format through this one module so the swap is a single edit.

use chrono::{TimeZone as _, Utc};
use ui::Color;

fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    let offset = digits.len() % 3;
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (index + 3 - offset).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(character);
    }
    grouped
}

pub fn format_sats(sats: i64) -> String {
    let sign = if sats < 0 { "-" } else { "" };
    format!("{sign}{} sats", group_thousands(sats.unsigned_abs()))
}

pub fn format_signed_sats(sats: i64) -> String {
    let sign = match sats {
        value if value > 0 => "+",
        value if value < 0 => "-",
        _ => "",
    };
    format!("{sign}{} sats", group_thousands(sats.unsigned_abs()))
}

pub fn format_btc(sats: i64) -> String {
    let sign = if sats < 0 { "-" } else { "" };
    let absolute = sats.unsigned_abs();
    format!(
        "{sign}{}.{:08} BTC",
        absolute / 100_000_000,
        absolute % 100_000_000
    )
}

pub fn format_usd(usd: u64) -> String {
    format!("${}", group_thousands(usd))
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

/// Sign coloring for PnL-like values. The colorblind-safe up/down pair is
/// the shared token kit's job; these map onto the theme's existing semantic
/// colors until it lands.
pub fn signed_color(value: i64) -> Color {
    match value {
        value if value > 0 => Color::Success,
        value if value < 0 => Color::Error,
        _ => Color::Muted,
    }
}

pub fn direction_color(direction: prediction_events::PredictedDirection) -> Color {
    use prediction_events::PredictedDirection;
    match direction {
        PredictedDirection::Up => Color::Success,
        PredictedDirection::Down => Color::Error,
        PredictedDirection::Flat => Color::Muted,
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
    fn signed_colors_map_to_semantic_theme_colors() {
        assert_eq!(signed_color(1), Color::Success);
        assert_eq!(signed_color(-1), Color::Error);
        assert_eq!(signed_color(0), Color::Muted);
    }
}
