//! Financial theme tokens and the number-formatting kit shared by every
//! market surface (omega#284).
//!
//! One definition point so the candlestick chart, order-book ladder, command
//! center, mandate meters, RFQ comparison, and prediction cards all agree on
//! up/down semantics, environment labeling, numeral alignment, and number
//! formats. The viz rule applies here too: color repeats a meaning, it never
//! carries it alone — every signed helper returns the sign in text alongside
//! the direction, and the direction glyph never depends on hue.

use std::sync::Arc;
use std::time::Duration;

use documented::Documented;
use gpui::{
    Animation, AnimationElement, AnimationExt, App, ElementId, Font, FontFeatures, Hsla, rgb,
};
use theme::ActiveTheme;

use crate::prelude::*;

/// The sign of a market quantity, driving color and glyph together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketDirection {
    Up,
    Down,
    Flat,
}

impl MarketDirection {
    pub fn of_f64(value: f64) -> Self {
        if !value.is_finite() || value == 0.0 {
            Self::Flat
        } else if value > 0.0 {
            Self::Up
        } else {
            Self::Down
        }
    }

    pub fn of_i64(value: i64) -> Self {
        match value.cmp(&0) {
            std::cmp::Ordering::Greater => Self::Up,
            std::cmp::Ordering::Less => Self::Down,
            std::cmp::Ordering::Equal => Self::Flat,
        }
    }

    /// A glyph that repeats the direction without color.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Up => "▲",
            Self::Down => "▼",
            Self::Flat => "·",
        }
    }
}

/// The execution environment a market surface is acting against. Surfaces
/// label the environment in text and may add the matching tint; real-money
/// mainnet is the undecorated case so the tint always means "not real".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketEnvironment {
    Demo,
    Testnet,
    Mainnet,
}

impl MarketEnvironment {
    pub fn label(self) -> &'static str {
        match self {
            Self::Demo => "demo",
            Self::Testnet => "testnet",
            Self::Mainnet => "mainnet",
        }
    }
}

/// The resolved financial color tokens for a market surface.
///
/// Structure follows `VizPalette`: theme-derived where the theme already has
/// the meaning, single definition point for the market-specific accents. The
/// up/down pair is blue/orange — the colorblind-safe axis — never raw
/// red/green, and every consumer pairs the color with a sign, glyph, or fill
/// difference so meaning survives grayscale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketTokens {
    /// Positive change: bids, gains, upticks. Sourced from the theme's info
    /// blue so it tracks theme changes.
    pub up: Hsla,
    /// Negative change: asks, losses, downticks. An orange accent with this
    /// single definition point, distinct from the bitcoin asset accent.
    pub down: Hsla,
    /// No change / dormant readouts.
    pub flat: Hsla,
    /// The tint behind not-real environments (demo, testnet).
    pub environment_tint: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub grid: Hsla,
    pub surface: Hsla,
}

impl MarketTokens {
    pub fn from_theme(cx: &App) -> Self {
        let colors = cx.theme().colors();
        let status = cx.theme().status();
        Self {
            up: status.info,
            down: rgb(0xf0883e).into(),
            flat: colors.text_muted,
            environment_tint: status.warning.opacity(0.12),
            text: colors.text,
            muted: colors.text_muted,
            grid: colors.border.opacity(0.5),
            surface: colors.surface_background,
        }
    }

    pub fn direction_color(&self, direction: MarketDirection) -> Hsla {
        match direction {
            MarketDirection::Up => self.up,
            MarketDirection::Down => self.down,
            MarketDirection::Flat => self.flat,
        }
    }

    /// The tint a surface applies when acting against `environment`; mainnet
    /// stays undecorated so a tint always means "not real money".
    pub fn environment_tint(&self, environment: MarketEnvironment) -> Option<Hsla> {
        match environment {
            MarketEnvironment::Demo | MarketEnvironment::Testnet => Some(self.environment_tint),
            MarketEnvironment::Mainnet => None,
        }
    }

    /// Desaturates every token — the audit that proves direction stays
    /// legible through signs, glyphs, and fill differences when hue is gone.
    pub fn grayscale(self) -> Self {
        Self {
            up: self.up.grayscale(),
            down: self.down.grayscale(),
            flat: self.flat.grayscale(),
            environment_tint: self.environment_tint.grayscale(),
            text: self.text.grayscale(),
            muted: self.muted.grayscale(),
            grid: self.grid.grayscale(),
            surface: self.surface.grayscale(),
        }
    }
}

/// Returns `font` with the OpenType `tnum` feature enabled, so digits share
/// one advance width and columns of changing numbers stop shimmering.
pub fn with_tabular_numerals(font: Font) -> Font {
    let mut features: Vec<(String, u32)> = font.features.tag_value_list().to_vec();
    if !features.iter().any(|(tag, _)| tag == "tnum") {
        features.push(("tnum".to_string(), 1));
    }
    Font {
        features: FontFeatures(Arc::new(features)),
        ..font
    }
}

/// The aligned-numeral font for market readouts: the buffer font with
/// tabular numerals pinned on for fonts whose digits are proportional.
pub fn market_number_font(cx: &App) -> Font {
    with_tabular_numerals(theme::theme_settings(cx).buffer_font(cx).clone())
}

/// How long a flash-on-change highlight takes to decay.
pub const FLASH_DURATION: Duration = Duration::from_millis(450);

/// The overlay color for a flash that is `progress` (0..=1) through its
/// decay: full-strength tint at 0, transparent at 1, quadratic ease-out.
pub fn flash_overlay(tint: Hsla, progress: f32) -> Hsla {
    let progress = progress.clamp(0.0, 1.0);
    tint.opacity((1.0 - progress).powi(2) * 0.35)
}

/// Flash-on-change for row-shaped elements: the animation is keyed by
/// `epoch`, so bumping the epoch when the underlying value changes restarts
/// the one-shot flash. `apply` receives the current overlay color each frame.
pub trait FlashOnChangeExt: AnimationExt + Sized {
    fn with_change_flash(
        self,
        name: impl Into<SharedString>,
        epoch: u64,
        tint: Hsla,
        apply: impl Fn(Self, Hsla) -> Self + 'static,
    ) -> AnimationElement<Self> {
        self.with_animation(
            ElementId::NamedInteger(name.into(), epoch),
            Animation::new(FLASH_DURATION),
            move |element, delta| apply(element, flash_overlay(tint, delta)),
        )
    }
}

impl<T: AnimationExt> FlashOnChangeExt for T {}

pub const SATS_PER_BTC: u64 = 100_000_000;

fn group_digits(digits: &str) -> String {
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(character);
    }
    grouped
}

/// `1234567` → `"1,234,567"`.
pub fn format_grouped_u64(value: u64) -> String {
    group_digits(&value.to_string())
}

/// `50_000` → `"50,000 sats"`.
pub fn format_sats(sats: u64) -> String {
    format!("{} sats", format_grouped_u64(sats))
}

/// `123_456_789` sats → `"1.23456789 BTC"`. Always eight decimals so BTC
/// columns align under tabular numerals.
pub fn format_btc(sats: u64) -> String {
    let whole = sats / SATS_PER_BTC;
    let fraction = sats % SATS_PER_BTC;
    format!("{}.{:08} BTC", format_grouped_u64(whole), fraction)
}

/// Signed sats through the shared unsigned formatter.
pub fn format_signed_sats_amount(sats: i64) -> String {
    let sign = if sats < 0 { "-" } else { "" };
    format!("{sign}{}", format_sats(sats.unsigned_abs()))
}

/// Signed sats with an explicit `+` for positive changes.
pub fn format_sats_change(sats: i64) -> String {
    let sign = match sats.cmp(&0) {
        std::cmp::Ordering::Greater => "+",
        std::cmp::Ordering::Less => "-",
        std::cmp::Ordering::Equal => "",
    };
    format!("{sign}{}", format_sats(sats.unsigned_abs()))
}

/// Signed BTC with the kit's fixed eight-decimal representation.
pub fn format_signed_btc(sats: i64) -> String {
    let sign = if sats < 0 { "-" } else { "" };
    format!("{sign}{}", format_btc(sats.unsigned_abs()))
}

/// Whole-dollar USD with grouped thousands.
pub fn format_usd(usd: u64) -> String {
    format!("${}", format_grouped_u64(usd))
}

/// A micro-probability where one million represents certainty.
pub fn format_probability_micros(micros: u32) -> String {
    let tenths = u64::from(micros) / 1_000;
    if tenths.is_multiple_of(10) {
        format!("{}%", tenths / 10)
    } else {
        format!("{}.{}%", tenths / 10, tenths % 10)
    }
}

/// Basis points rendered as a percentage without dropping hundredths.
pub fn format_percent_bps(basis_points: u32) -> String {
    if basis_points.is_multiple_of(100) {
        format!("{}%", basis_points / 100)
    } else {
        format!("{}.{:02}%", basis_points / 100, basis_points % 100)
    }
}

/// Compact duration used by deadlines and high-density market rows.
pub fn format_duration_ms(milliseconds: i64) -> String {
    let total_seconds = milliseconds.max(0) / 1_000;
    let days = total_seconds / 86_400;
    let hours = (total_seconds % 86_400) / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;
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

/// A deadline relative to `now_ms`.
pub fn format_countdown(deadline_ms: i64, now_ms: i64) -> String {
    if deadline_ms >= now_ms {
        format!("in {}", format_duration_ms(deadline_ms - now_ms))
    } else {
        format!("{} ago", format_duration_ms(now_ms - deadline_ms))
    }
}

/// A local-time readout using the workspace's one locale-aware time path.
pub fn format_wall_clock(at_ms: i64) -> String {
    let nanos = i128::from(at_ms).saturating_mul(1_000_000);
    match time::OffsetDateTime::from_unix_timestamp_nanos(nanos) {
        Ok(timestamp) => time_format::format_time(timestamp),
        Err(_) => format!("{at_ms} ms"),
    }
}

pub fn sats_to_btc(sats: u64) -> f64 {
    sats as f64 / SATS_PER_BTC as f64
}

/// Rounds to the nearest sat; `None` for negative, non-finite, or
/// out-of-range inputs rather than panicking or wrapping.
pub fn btc_to_sats(btc: f64) -> Option<u64> {
    if !btc.is_finite() || btc < 0.0 {
        return None;
    }
    let sats = (btc * SATS_PER_BTC as f64).round();
    if sats > u64::MAX as f64 {
        return None;
    }
    Some(sats as u64)
}

/// USD from integer cents — the money path stays off floats. Negative
/// amounts render as `-$1,234.56`.
pub fn format_usd_cents(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let magnitude = cents.unsigned_abs();
    format!(
        "{sign}${}.{:02}",
        format_grouped_u64(magnitude / 100),
        magnitude % 100
    )
}

/// A decimal quantity with grouped thousands and a fixed number of decimals;
/// the per-asset formatting path (BTC 8, USD 2, contracts 0, …). Non-finite
/// values render as an em dash rather than propagating `NaN` into the UI.
pub fn format_with_decimals(value: f64, decimals: usize) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    let sign = if value < 0.0 { "-" } else { "" };
    let text = format!("{:.*}", decimals, value.abs());
    match text.split_once('.') {
        Some((whole, fraction)) => format!("{sign}{}.{fraction}", group_digits(whole)),
        None => format!("{sign}{}", group_digits(&text)),
    }
}

/// Compact notation for large magnitudes: `1_234_567.0` → `"1.2M"`. One
/// decimal, trimmed when zero, through K/M/B/T.
pub fn format_compact(value: f64) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    let sign = if value < 0.0 { "-" } else { "" };
    let magnitude = value.abs();
    let (scaled, suffix) = if magnitude >= 1e12 {
        (magnitude / 1e12, "T")
    } else if magnitude >= 1e9 {
        (magnitude / 1e9, "B")
    } else if magnitude >= 1e6 {
        (magnitude / 1e6, "M")
    } else if magnitude >= 1e3 {
        (magnitude / 1e3, "K")
    } else {
        return format!("{sign}{}", format_with_decimals(magnitude, 0));
    };
    let text = format!("{scaled:.1}");
    let trimmed = text.strip_suffix(".0").unwrap_or(&text);
    format!("{sign}{trimmed}{suffix}")
}

/// A signed change formatted for display next to its direction color: the
/// sign rides in the text so the meaning never depends on hue alone.
pub fn format_signed(value: f64, decimals: usize) -> (String, MarketDirection) {
    let direction = MarketDirection::of_f64(value);
    let sign = match direction {
        MarketDirection::Up => "+",
        _ => "",
    };
    (
        format!("{sign}{}", format_with_decimals(value, decimals)),
        direction,
    )
}

/// `0.0234` → `("+2.34%", Up)`. Takes a fraction, not percentage points.
pub fn format_signed_percent(fraction: f64, decimals: usize) -> (String, MarketDirection) {
    let (text, direction) = format_signed(fraction * 100.0, decimals);
    (format!("{text}%"), direction)
}

/// The catalog entry demonstrating the kit: tokens, environment tints,
/// tabular alignment, every formatter, and the flash-on-change decay.
#[derive(IntoElement, RegisterComponent, Documented)]
pub struct MarketKit {
    tokens: Option<MarketTokens>,
}

impl MarketKit {
    pub fn new() -> Self {
        Self { tokens: None }
    }

    /// Overrides the theme tokens; used by the grayscale audit preview.
    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl Default for MarketKit {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for MarketKit {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let number_font = market_number_font(cx);

        let direction_row = |direction: MarketDirection, sample: (String, MarketDirection)| {
            let color = tokens.direction_color(direction);
            h_flex()
                .gap_2()
                .items_center()
                .child(div().size(px(10.)).rounded_xs().bg(color))
                .child(
                    Label::new(format!("{} {}", direction.glyph(), sample.0))
                        .size(LabelSize::Small)
                        .color(Color::Custom(color))
                        .buffer_font(cx),
                )
        };

        let environment_row = |environment: MarketEnvironment| {
            let row = h_flex()
                .px_2()
                .py_1()
                .gap_2()
                .rounded_sm()
                .border_1()
                .border_color(cx.theme().colors().border)
                .child(Label::new(environment.label()).size(LabelSize::Small))
                .child(
                    Label::new(if tokens.environment_tint(environment).is_some() {
                        "tinted · not real"
                    } else {
                        "untinted · real money"
                    })
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
                );
            match tokens.environment_tint(environment) {
                Some(tint) => row.bg(tint),
                None => row,
            }
        };

        let format_row = |label: &'static str, value: String| {
            h_flex()
                .gap_2()
                .justify_between()
                .w(px(340.))
                .child(
                    Label::new(label)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    div()
                        .font(number_font.clone())
                        .text_size(px(12.))
                        .child(value),
                )
        };

        let (change_text, change_direction) = format_signed(1234.5, 2);
        let (percent_text, percent_direction) = format_signed_percent(-0.0234, 2);

        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Direction tokens",
                vec![single_example(
                    "Blue up, orange down — sign and glyph repeat the meaning",
                    v_flex()
                        .gap_1()
                        .child(direction_row(MarketDirection::Up, format_signed(1234.5, 2)))
                        .child(direction_row(
                            MarketDirection::Down,
                            format_signed(-1234.5, 2),
                        ))
                        .child(direction_row(MarketDirection::Flat, format_signed(0.0, 2)))
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Environment tint",
                vec![single_example(
                    "A tint always means not-real; mainnet is undecorated",
                    h_flex()
                        .gap_2()
                        .child(environment_row(MarketEnvironment::Demo))
                        .child(environment_row(MarketEnvironment::Testnet))
                        .child(environment_row(MarketEnvironment::Mainnet))
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Formatting kit",
                vec![single_example(
                    "Every formatter, aligned under tabular numerals",
                    v_flex()
                        .gap_0p5()
                        .child(format_row("format_sats", format_sats(50_000)))
                        .child(format_row("format_btc", format_btc(123_456_789)))
                        .child(format_row("format_usd_cents", format_usd_cents(123_456)))
                        .child(format_row(
                            "format_with_decimals",
                            format_with_decimals(65_432.1, 2),
                        ))
                        .child(format_row("format_compact", format_compact(1_234_567.0)))
                        .child(format_row("format_signed", change_text.clone()))
                        .child(format_row("format_signed_percent", percent_text.clone()))
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Signed with color",
                vec![single_example(
                    "Signed helpers pair text with a direction for the color",
                    h_flex()
                        .gap_4()
                        .child(
                            Label::new(change_text)
                                .size(LabelSize::Small)
                                .color(Color::Custom(tokens.direction_color(change_direction)))
                                .buffer_font(cx),
                        )
                        .child(
                            Label::new(percent_text)
                                .size(LabelSize::Small)
                                .color(Color::Custom(tokens.direction_color(percent_direction)))
                                .buffer_font(cx),
                        )
                        .into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Flash on change",
                vec![single_example(
                    "The decay curve of a size-change flash, replaying",
                    h_flex()
                        .gap_2()
                        .children([0.0f32, 0.25, 0.5, 0.75, 1.0].into_iter().map(|progress| {
                            div()
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .bg(flash_overlay(tokens.up, progress))
                                .child(
                                    Label::new(format!("t={progress:.2}"))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                        }))
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

impl Component for MarketKit {
    fn scope() -> ComponentScope {
        ComponentScope::Utilities
    }

    fn description() -> &'static str {
        Self::DOCS
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(MarketKit::new())
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Direction survives without hue via sign and glyph",
                    MarketKit::new()
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
    fn sats_group_thousands() {
        assert_eq!(format_sats(0), "0 sats");
        assert_eq!(format_sats(950), "950 sats");
        assert_eq!(format_sats(50_000), "50,000 sats");
        assert_eq!(format_sats(1_234_567), "1,234,567 sats");
    }

    #[test]
    fn btc_renders_eight_decimals_from_sats() {
        assert_eq!(format_btc(0), "0.00000000 BTC");
        assert_eq!(format_btc(1), "0.00000001 BTC");
        assert_eq!(format_btc(123_456_789), "1.23456789 BTC");
        assert_eq!(format_btc(250_000_000_000), "2,500.00000000 BTC");
    }

    #[test]
    fn sats_btc_conversions_round_trip() {
        assert_eq!(btc_to_sats(1.0), Some(SATS_PER_BTC));
        assert_eq!(btc_to_sats(0.00000001), Some(1));
        assert_eq!(btc_to_sats(sats_to_btc(123_456_789)), Some(123_456_789));
        assert_eq!(btc_to_sats(-1.0), None);
        assert_eq!(btc_to_sats(f64::NAN), None);
        assert_eq!(btc_to_sats(f64::INFINITY), None);
    }

    #[test]
    fn usd_cents_keep_money_off_floats() {
        assert_eq!(format_usd_cents(0), "$0.00");
        assert_eq!(format_usd_cents(5), "$0.05");
        assert_eq!(format_usd_cents(123_456), "$1,234.56");
        assert_eq!(format_usd_cents(-123_456), "-$1,234.56");
    }

    #[test]
    fn decimal_formatting_groups_and_survives_non_finite() {
        assert_eq!(format_with_decimals(65_432.1, 2), "65,432.10");
        assert_eq!(format_with_decimals(-0.5, 2), "-0.50");
        assert_eq!(format_with_decimals(1_234.0, 0), "1,234");
        assert_eq!(format_with_decimals(f64::NAN, 2), "—");
        assert_eq!(format_with_decimals(f64::INFINITY, 2), "—");
    }

    #[test]
    fn compact_notation_scales_and_trims() {
        assert_eq!(format_compact(950.0), "950");
        assert_eq!(format_compact(1_000.0), "1K");
        assert_eq!(format_compact(1_234_567.0), "1.2M");
        assert_eq!(format_compact(2_000_000_000.0), "2B");
        assert_eq!(format_compact(3_400_000_000_000.0), "3.4T");
        assert_eq!(format_compact(-12_500.0), "-12.5K");
        assert_eq!(format_compact(f64::NAN), "—");
    }

    #[test]
    fn signed_helpers_carry_the_sign_in_text() {
        assert_eq!(
            format_signed(1234.5, 2),
            ("+1,234.50".to_string(), MarketDirection::Up)
        );
        assert_eq!(
            format_signed(-1234.5, 2),
            ("-1,234.50".to_string(), MarketDirection::Down)
        );
        assert_eq!(
            format_signed(0.0, 2),
            ("0.00".to_string(), MarketDirection::Flat)
        );
        assert_eq!(
            format_signed_percent(0.0234, 2),
            ("+2.34%".to_string(), MarketDirection::Up)
        );
        assert_eq!(
            format_signed_percent(-0.0234, 2),
            ("-2.34%".to_string(), MarketDirection::Down)
        );
    }

    #[test]
    fn shared_deadline_formats_stay_compact() {
        assert_eq!(format_duration_ms(42_000), "42s");
        assert_eq!(format_duration_ms(3 * 60_000 + 5_000), "3m 05s");
        assert_eq!(format_countdown(10_000, 5_000), "in 5s");
        assert_eq!(format_countdown(5_000, 10_000), "5s ago");
    }

    #[test]
    fn direction_classifies_values() {
        assert_eq!(MarketDirection::of_f64(0.1), MarketDirection::Up);
        assert_eq!(MarketDirection::of_f64(-0.1), MarketDirection::Down);
        assert_eq!(MarketDirection::of_f64(0.0), MarketDirection::Flat);
        assert_eq!(MarketDirection::of_f64(f64::NAN), MarketDirection::Flat);
        assert_eq!(MarketDirection::of_i64(5), MarketDirection::Up);
        assert_eq!(MarketDirection::of_i64(-5), MarketDirection::Down);
        assert_eq!(MarketDirection::of_i64(0), MarketDirection::Flat);
    }

    #[test]
    fn tabular_numerals_are_added_once() {
        let font = gpui::font("Zed Plex Mono");
        let tabular = with_tabular_numerals(font);
        assert_eq!(
            tabular
                .features
                .tag_value_list()
                .iter()
                .filter(|(tag, _)| tag == "tnum")
                .count(),
            1
        );
        let again = with_tabular_numerals(tabular);
        assert_eq!(
            again
                .features
                .tag_value_list()
                .iter()
                .filter(|(tag, _)| tag == "tnum")
                .count(),
            1
        );
    }

    #[test]
    fn flash_overlay_decays_to_transparent() {
        let tint = gpui::rgb(0x74ade8).into();
        assert!(flash_overlay(tint, 0.0).a > flash_overlay(tint, 0.5).a);
        assert!(flash_overlay(tint, 0.5).a > flash_overlay(tint, 1.0).a);
        assert_eq!(flash_overlay(tint, 1.0).a, 0.0);
        // Out-of-range progress clamps instead of over-brightening.
        assert_eq!(flash_overlay(tint, 2.0).a, 0.0);
        assert_eq!(flash_overlay(tint, -1.0).a, flash_overlay(tint, 0.0).a);
    }
}
