//! Venue-neutral command-center components for the market UI sprint
//! (omega#284): the command-center header, agent roster + activity feed,
//! mandate status card with headroom meters, and the prediction card.
//!
//! Every component takes value-typed inputs and registers a demo-data
//! preview in the component library; live wiring happens through the small
//! adapter constructors that read from the real trading crates
//! (`trading_ledger`, `trading_mandate`, `strategy_engine`, `agent_wakeup`,
//! `prediction_events`).
//!
//! The number formatting and up/down color helpers in [`format`] are local
//! shims: they get swapped for the shared financial theme tokens and
//! formatting kit in `crates/ui` once that lands (tracked on omega#284).

mod activity_feed;
mod command_center_header;
mod format;
mod mandate_status_card;
mod prediction_card;

pub use activity_feed::{
    ActivityEvent, ActivityEventKind, ActivityFeed, AgentActivityState, AgentRoster,
    AgentRosterEntry, demo_activity_events, demo_roster_entries,
};
pub use command_center_header::{CommandCenterHeader, PortfolioSummary};
pub use format::{
    direction_color, format_btc, format_countdown, format_duration_ms, format_percent_bps,
    format_probability_micros, format_sats, format_signed_sats, format_usd, format_wall_clock,
    signed_color,
};
pub use mandate_status_card::{HeadroomMeter, MandateStatusCard, MandateUsage, MeterZone};
pub use prediction_card::{
    ForecastDisplay, PredictionCard, PredictionCardData, PredictionList, PredictionResolution,
};

pub fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod paint_tests {
    use component::Component as _;
    use gpui::TestAppContext;
    use ui::prelude::*;

    use crate::{
        ActivityFeed, AgentRoster, CommandCenterHeader, MandateStatusCard, PredictionCard,
    };

    struct AllCommandCenterPreviews;

    impl gpui::Render for AllCommandCenterPreviews {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            v_flex()
                .size_full()
                .child(CommandCenterHeader::preview(window, cx))
                .child(AgentRoster::preview(window, cx))
                .child(ActivityFeed::preview(window, cx))
                .child(MandateStatusCard::preview(window, cx))
                .child(PredictionCard::preview(window, cx))
        }
    }

    /// Paints every command-center preview gallery in a real test window so
    /// the full layout and tessellation path runs, matching the viz preview
    /// discipline from omega#247.
    #[gpui::test]
    async fn every_command_center_preview_paints_without_panicking(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
        let (_view, cx) = cx.add_window_view(|_, _| AllCommandCenterPreviews);
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();

        let rendered = cx.debug_render_snapshot();
        for selector in [
            "command_center.header",
            "command_center.roster",
            "command_center.activity_feed",
            "command_center.mandate_card",
            "command_center.prediction_card",
            "command_center.prediction_list",
        ] {
            assert!(
                !rendered.occurrences(selector).is_empty(),
                "command-center previews did not paint {selector}"
            );
        }
    }
}
