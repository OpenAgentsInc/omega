use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use gpui::{
    App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ScrollHandle, Task, WeakEntity, Window, actions, px,
};
use lnmarkets_data::{CollectorHealth, CollectorStatus};
use plugin_api::{VenueActionStatus, VenueCapabilityReport, VenueCapabilityVerificationStatus};
use trading_ledger::ProfitReport;
use trading_mandate::{MandateSnapshot, ReviewCadence};
use ui::{Divider, Indicator, prelude::*};
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

const PANEL_KEY: &str = "lnmarkets-operator";
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

actions!(
    lnmarkets_operator,
    [
        /// Toggles focus on the LN Markets operator panel.
        ToggleFocus,
        /// Refreshes local LN Markets operator state.
        Refresh
    ]
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorStrategySnapshot {
    pub strategy_id: String,
    pub status: String,
    pub state: Option<String>,
    pub last_action: Option<String>,
    pub daily_loss_headroom_sats: Option<u64>,
    pub order_headroom: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorReviewTurn {
    pub at_ms: i64,
    pub trigger: String,
    pub outcome: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorBacktestSnapshot {
    pub strategy_id: String,
    pub outcome: String,
    pub created_at_ms: i64,
    pub trade_count: u64,
    pub expectancy_millisats: i64,
    pub maximum_drawdown_sats: u64,
    pub parameter_digest: String,
}

#[derive(Clone, Debug)]
pub struct OperatorConsoleSnapshot {
    pub generated_at_ms: i64,
    pub collector: Option<CollectorHealth>,
    pub venue_capabilities: Option<VenueCapabilityReport>,
    pub strategies: Vec<OperatorStrategySnapshot>,
    pub backtests: Vec<OperatorBacktestSnapshot>,
    pub ledger: Option<ProfitReport>,
    pub mandate: Option<MandateSnapshot>,
    pub review_cadence: Option<ReviewCadence>,
    pub pending_wakeups: usize,
    pub review_history: Vec<OperatorReviewTurn>,
    pub runtime_error: Option<String>,
}

impl OperatorConsoleSnapshot {
    pub fn unavailable(generated_at_ms: i64, error: impl Into<String>) -> Self {
        Self {
            generated_at_ms,
            collector: None,
            venue_capabilities: None,
            strategies: Vec::new(),
            backtests: Vec::new(),
            ledger: None,
            mandate: None,
            review_cadence: None,
            pending_wakeups: 0,
            review_history: Vec::new(),
            runtime_error: Some(error.into()),
        }
    }
}

pub trait OperatorConsoleSource: Send + Sync {
    fn snapshot(&self, now_ms: i64) -> OperatorConsoleSnapshot;
    fn narrow_mandate(&self, changed_at_ms: i64) -> Result<()>;
    fn revoke_mandate(&self, changed_at_ms: i64) -> Result<()>;
}

pub struct LnMarketsOperatorPanel {
    focus_handle: FocusHandle,
    source: Arc<dyn OperatorConsoleSource>,
    snapshot: OperatorConsoleSnapshot,
    operation: Option<Task<()>>,
    operation_error: Option<String>,
    scroll_handle: ScrollHandle,
    _refresh_task: Task<()>,
}

impl LnMarketsOperatorPanel {
    pub fn load(
        workspace: WeakEntity<Workspace>,
        source: Arc<dyn OperatorConsoleSource>,
        cx: AsyncWindowContext,
    ) -> Task<Result<Entity<Self>>> {
        cx.spawn(async move |cx| {
            workspace.update_in(cx, |_workspace, _window, cx| {
                cx.new(|cx| Self::new(source, true, cx))
            })
        })
    }

    fn new(source: Arc<dyn OperatorConsoleSource>, poll: bool, cx: &mut Context<Self>) -> Self {
        let snapshot = source.snapshot(unix_now_ms());
        let refresh_task = if poll {
            cx.spawn({
                let source = source.clone();
                async move |this, cx| loop {
                    cx.background_executor().timer(REFRESH_INTERVAL).await;
                    let snapshot = cx
                        .background_spawn({
                            let source = source.clone();
                            async move { source.snapshot(unix_now_ms()) }
                        })
                        .await;
                    if this
                        .update(cx, |this, cx| {
                            this.snapshot = snapshot;
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            })
        } else {
            Task::ready(())
        };
        Self {
            focus_handle: cx.focus_handle(),
            source,
            snapshot,
            operation: None,
            operation_error: None,
            scroll_handle: ScrollHandle::new(),
            _refresh_task: refresh_task,
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.snapshot = self.source.snapshot(unix_now_ms());
        cx.notify();
    }

    fn narrow_mandate(&mut self, cx: &mut Context<Self>) {
        self.run_mandate_action(MandateAction::Narrow, cx);
    }

    fn revoke_mandate(&mut self, cx: &mut Context<Self>) {
        self.run_mandate_action(MandateAction::Revoke, cx);
    }

    fn run_mandate_action(&mut self, action: MandateAction, cx: &mut Context<Self>) {
        if self.operation.is_some() {
            return;
        }
        self.operation_error = None;
        let source = self.source.clone();
        self.operation = Some(cx.spawn(async move |this, cx| {
            let (result, snapshot) = cx
                .background_spawn(async move {
                    let changed_at_ms = unix_now_ms();
                    let result = match action {
                        MandateAction::Narrow => source.narrow_mandate(changed_at_ms),
                        MandateAction::Revoke => source.revoke_mandate(changed_at_ms),
                    };
                    let snapshot = source.snapshot(unix_now_ms());
                    (result, snapshot)
                })
                .await;
            this.update(cx, |this, cx| {
                this.operation = None;
                this.snapshot = snapshot;
                this.operation_error = result.err().map(|error| error.to_string());
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn render_collector(&self) -> gpui::AnyElement {
        let content = match &self.snapshot.collector {
            Some(health) => {
                let status = format!("{:?}", health.status).to_lowercase();
                let backfill = format!(
                    "Backfill {}/{} surfaces · {} rows · {} stored events",
                    health.backfill_completed_surfaces,
                    health.backfill_total_surfaces,
                    health.backfill_rows,
                    health.stored_events
                );
                let generated_at_ms = self.snapshot.generated_at_ms;
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Indicator::dot().color(collector_color(health.status)))
                            .child(Label::new(status))
                            .child(
                                Label::new(format!("{:?}", health.network))
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(Label::new(backfill).size(LabelSize::Small))
                    .when_some(health.last_error.clone(), |this, error| {
                        this.child(Label::new(error).size(LabelSize::Small).color(Color::Error))
                    })
                    .children(health.subscribed_topics.iter().map(move |topic| {
                        let lag = health.last_event_by_topic_ms.get(topic).map_or_else(
                            || "waiting".to_string(),
                            |at_ms| format_duration(generated_at_ms.saturating_sub(*at_ms).max(0)),
                        );
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .child(Label::new(topic.clone()).size(LabelSize::Small))
                            .child(Label::new(lag).size(LabelSize::Small).color(Color::Muted))
                    }))
                    .into_any_element()
            }
            None => Label::new("Collector is starting")
                .size(LabelSize::Small)
                .color(Color::Muted)
                .into_any_element(),
        };
        section("lnmarkets.operator.collector", "Collector", content)
    }

    fn render_strategies(&self) -> gpui::AnyElement {
        let strategies = self
            .snapshot
            .strategies
            .iter()
            .enumerate()
            .map(|(index, strategy)| {
                let selector = format!("lnmarkets.operator.strategy.{}", strategy.strategy_id);
                v_flex()
                    .id(("lnmarkets-operator-strategy", index))
                    .debug_selector(move || selector)
                    .p_2()
                    .gap_1()
                    .border_1()
                    .rounded_sm()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(Label::new(strategy.strategy_id.clone()))
                            .child(
                                Label::new(strategy.status.clone())
                                    .size(LabelSize::Small)
                                    .color(strategy_color(&strategy.status)),
                            ),
                    )
                    .when_some(strategy.state.clone(), |this, state| {
                        this.child(
                            Label::new(format!("State {state}"))
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                    })
                    .when_some(strategy.last_action.clone(), |this, action| {
                        this.child(
                            Label::new(action)
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                    })
                    .child(
                        Label::new(format!(
                            "Loss headroom {} · order headroom {}",
                            optional_number(strategy.daily_loss_headroom_sats),
                            optional_number(strategy.order_headroom)
                        ))
                        .size(LabelSize::Small),
                    )
            });
        section(
            "lnmarkets.operator.strategies",
            "Active strategies",
            v_flex()
                .gap_2()
                .when(self.snapshot.strategies.is_empty(), |this| {
                    this.child(
                        Label::new("No strategies")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                })
                .children(strategies)
                .into_any_element(),
        )
    }

    fn render_venue_capabilities(&self) -> gpui::AnyElement {
        let content = match &self.snapshot.venue_capabilities {
            Some(report) => {
                let verification = &report.verification;
                let verified = verification.status == VenueCapabilityVerificationStatus::Verified;
                let status = if verified { "verified" } else { "unverified" };
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(Label::new(status).color(if verified {
                                Color::Success
                            } else {
                                Color::Warning
                            }))
                            .when_some(verification.newest_probed_at_ms, |this, probed_at_ms| {
                                this.child(
                                    Label::new(format!(
                                        "Probed {}",
                                        format_timestamp(probed_at_ms)
                                    ))
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                                )
                            }),
                    )
                    .when(verification.stale, |this| {
                        this.child(
                            Label::new("Capability probe is stale")
                                .size(LabelSize::Small)
                                .color(Color::Warning),
                        )
                    })
                    .children(verification.reasons.iter().map(|reason| {
                        Label::new(reason.clone())
                            .size(LabelSize::Small)
                            .color(Color::Warning)
                    }))
                    .when_some(report.capabilities.as_ref(), |this, capabilities| {
                        this.child(
                            Label::new(format!(
                                "Account {:?} ({}) · margin {:?} ({})",
                                capabilities.account_mode.value.typed,
                                capabilities.account_mode.value.raw,
                                capabilities.margin_mode.value.typed,
                                capabilities.margin_mode.value.raw,
                            ))
                            .size(LabelSize::Small),
                        )
                        .children(capabilities.actions.iter().map(|action| {
                            let status = match &action.value.status {
                                VenueActionStatus::Supported => "supported".to_string(),
                                VenueActionStatus::Disabled { reason } => {
                                    format!("disabled: {reason}")
                                }
                                VenueActionStatus::Unknown { raw } => {
                                    format!("unknown: {raw}")
                                }
                            };
                            Label::new(format!(
                                "{} {status} · probed {}",
                                action.value.action_class,
                                format_timestamp(action.probed_at_ms),
                            ))
                            .size(LabelSize::Small)
                            .color(Color::Muted)
                        }))
                    })
                    .into_any_element()
            }
            None => Label::new("Venue capabilities are unavailable")
                .size(LabelSize::Small)
                .color(Color::Warning)
                .into_any_element(),
        };
        section(
            "lnmarkets.operator.capabilities",
            "Venue capabilities",
            content,
        )
    }

    fn render_ledger(&self) -> gpui::AnyElement {
        let content = match &self.snapshot.ledger {
            Some(report) => v_flex()
                .gap_1()
                .child(Label::new(format!(
                    "Profit {} sats",
                    report.total_profit_sats
                )))
                .child(
                    Label::new(format!(
                        "Fees {} sats · funding {} sats · drawdown {} sats",
                        report.total_fees_paid_sats,
                        report.total_funding_collected_sats,
                        report.worst_drawdown_sats
                    ))
                    .size(LabelSize::Small),
                )
                .children(report.strategies.iter().map(|strategy| {
                    h_flex()
                        .justify_between()
                        .child(Label::new(strategy.strategy_id.clone()).size(LabelSize::Small))
                        .child(
                            Label::new(format!("{} sats", strategy.profit_sats))
                                .size(LabelSize::Small)
                                .color(if strategy.profit_sats < 0 {
                                    Color::Error
                                } else {
                                    Color::Success
                                }),
                        )
                }))
                .into_any_element(),
            None => Label::new("Ledger is unavailable")
                .size(LabelSize::Small)
                .color(Color::Muted)
                .into_any_element(),
        };
        section(
            "lnmarkets.operator.ledger",
            "Ledger · last 24 hours",
            content,
        )
    }

    fn render_backtests(&self) -> gpui::AnyElement {
        let reports = self
            .snapshot
            .backtests
            .iter()
            .enumerate()
            .map(|(index, report)| {
                v_flex()
                    .id(("lnmarkets-operator-backtest", index))
                    .p_2()
                    .gap_1()
                    .border_1()
                    .rounded_sm()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(Label::new(report.strategy_id.clone()))
                            .child(
                                Label::new(report.outcome.clone())
                                    .size(LabelSize::Small)
                                    .color(if report.outcome == "passed" {
                                        Color::Success
                                    } else {
                                        Color::Error
                                    }),
                            ),
                    )
                    .child(
                        Label::new(format!(
                            "{} trades · {} millisats expectancy · {} sats max drawdown",
                            report.trade_count,
                            report.expectancy_millisats,
                            report.maximum_drawdown_sats
                        ))
                        .size(LabelSize::Small),
                    )
                    .child(
                        Label::new(format!(
                            "Created {} · parameters {}",
                            format_timestamp(report.created_at_ms),
                            report
                                .parameter_digest
                                .get(..12)
                                .unwrap_or(&report.parameter_digest)
                        ))
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                    )
            });
        section(
            "lnmarkets.operator.backtests",
            "Backtest reports",
            v_flex()
                .gap_2()
                .when(self.snapshot.backtests.is_empty(), |this| {
                    this.child(
                        Label::new("No backtest reports")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                })
                .children(reports)
                .into_any_element(),
        )
    }

    fn render_mandate(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let busy = self.operation.is_some();
        let content = match self
            .snapshot
            .mandate
            .as_ref()
            .and_then(|snapshot| snapshot.mandates.first())
        {
            Some(mandate) => v_flex()
                .gap_1()
                .child(Label::new(format!(
                    "Expires {}",
                    format_timestamp(mandate.expires_at_ms)
                )))
                .child(
                    Label::new(format!(
                        "{} sats venue · {} USD position · {}x · {} orders/hour",
                        mandate.max_venue_balance,
                        mandate.max_position_usd,
                        mandate.max_leverage,
                        mandate.max_orders_per_hour
                    ))
                    .size(LabelSize::Small),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("lnmarkets-narrow-mandate", "Narrow limits 50%")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.narrow_mandate(cx))),
                        )
                        .child(
                            Button::new("lnmarkets-revoke-mandate", "Revoke")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.revoke_mandate(cx))),
                        ),
                )
                .into_any_element(),
            None => Label::new("No active mandate")
                .size(LabelSize::Small)
                .color(Color::Muted)
                .into_any_element(),
        };
        section("lnmarkets.operator.mandate", "Mandate", content)
    }

    fn render_wakeups(&self) -> gpui::AnyElement {
        let cadence = self
            .snapshot
            .review_cadence
            .as_ref()
            .map(review_cadence_label)
            .unwrap_or_else(|| "No schedule".to_string());
        section(
            "lnmarkets.operator.wakeups",
            "Review turns",
            v_flex()
                .gap_1()
                .child(Label::new(cadence))
                .child(
                    Label::new(format!(
                        "{} pending event wakeups",
                        self.snapshot.pending_wakeups
                    ))
                    .size(LabelSize::Small),
                )
                .when(self.snapshot.review_history.is_empty(), |this| {
                    this.child(
                        Label::new("No completed review turns")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                })
                .children(self.snapshot.review_history.iter().take(8).map(|turn| {
                    h_flex()
                        .justify_between()
                        .gap_2()
                        .child(
                            Label::new(format!(
                                "{} · {}",
                                format_timestamp(turn.at_ms),
                                turn.trigger
                            ))
                            .size(LabelSize::Small),
                        )
                        .child(
                            Label::new(turn.outcome.clone())
                                .size(LabelSize::Small)
                                .color(if turn.outcome == "completed" {
                                    Color::Success
                                } else {
                                    Color::Error
                                }),
                        )
                }))
                .into_any_element(),
        )
    }
}

#[derive(Clone, Copy)]
enum MandateAction {
    Narrow,
    Revoke,
}

impl Render for LnMarketsOperatorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("lnmarkets-operator-panel")
            .debug_selector(|| "lnmarkets.operator.panel".into())
            .key_context("LnMarketsOperatorPanel")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll_handle)
            .p_2()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(Label::new("LN Markets").size(LabelSize::Large))
                    .child(
                        Button::new("lnmarkets-operator-refresh", "Refresh")
                            .label_size(LabelSize::Small)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
            )
            .when_some(self.snapshot.runtime_error.clone(), |this, error| {
                this.child(Label::new(error).size(LabelSize::Small).color(Color::Error))
            })
            .when_some(self.operation_error.clone(), |this, error| {
                this.child(Label::new(error).size(LabelSize::Small).color(Color::Error))
            })
            .child(self.render_collector())
            .child(self.render_venue_capabilities())
            .child(self.render_strategies())
            .child(self.render_backtests())
            .child(self.render_ledger())
            .child(self.render_mandate(cx))
            .child(self.render_wakeups())
    }
}

impl Focusable for LnMarketsOperatorPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PanelEvent> for LnMarketsOperatorPanel {}

impl Panel for LnMarketsOperatorPanel {
    fn persistent_name() -> &'static str {
        "LnMarketsOperatorPanel"
    }

    fn panel_key() -> &'static str {
        PANEL_KEY
    }

    fn position(&self, _: &Window, _: &App) -> DockPosition {
        DockPosition::Right
    }

    fn position_is_valid(&self, _: DockPosition) -> bool {
        true
    }

    fn set_position(&mut self, _: DockPosition, _: &mut Window, _: &mut Context<Self>) {}

    fn default_size(&self, _: &Window, _: &App) -> gpui::Pixels {
        px(460.)
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::SignalHigh)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        Some("LN Markets")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        9
    }
}

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
            workspace.toggle_panel_focus::<LnMarketsOperatorPanel>(window, cx);
        });
    })
    .detach();
}

fn section(
    selector: &'static str,
    title: &'static str,
    content: gpui::AnyElement,
) -> gpui::AnyElement {
    v_flex()
        .debug_selector(move || selector.into())
        .p_2()
        .gap_2()
        .border_1()
        .rounded_md()
        .child(Label::new(title).size(LabelSize::Small).color(Color::Muted))
        .child(Divider::horizontal())
        .child(content)
        .into_any_element()
}

fn collector_color(status: CollectorStatus) -> Color {
    match status {
        CollectorStatus::Streaming => Color::Success,
        CollectorStatus::Backfilling | CollectorStatus::Connecting => Color::Accent,
        CollectorStatus::Degraded => Color::Warning,
        CollectorStatus::Starting | CollectorStatus::Stopped => Color::Muted,
    }
}

fn strategy_color(status: &str) -> Color {
    match status {
        "running" => Color::Success,
        "halted" | "error" => Color::Error,
        _ => Color::Muted,
    }
}

fn optional_number(number: Option<impl std::fmt::Display>) -> String {
    number
        .map(|number| number.to_string())
        .unwrap_or_else(|| "—".into())
}

fn review_cadence_label(cadence: &ReviewCadence) -> String {
    match cadence {
        ReviewCadence::FundingSettlement => "At each funding settlement".to_string(),
        ReviewCadence::Interval { seconds } => format!("Every {seconds} seconds"),
    }
}

fn format_duration(milliseconds: i64) -> String {
    if milliseconds < 1_000 {
        return format!("{milliseconds} ms");
    }
    if milliseconds < 60_000 {
        return format!("{} s", milliseconds / 1_000);
    }
    format!("{} min", milliseconds / 60_000)
}

fn format_timestamp(milliseconds: i64) -> String {
    format!("{milliseconds} ms")
}

fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use gpui::TestAppContext;
    use lnmarkets_client::Network;
    use trading_ledger::{ProfitReport, StrategyProfit};
    use trading_mandate::{AssetId, ReviewCadence, TradingMandate, TradingNetwork};

    use super::*;

    struct TestSource {
        snapshot: OperatorConsoleSnapshot,
        narrow_count: AtomicUsize,
        revoke_count: AtomicUsize,
    }

    impl OperatorConsoleSource for TestSource {
        fn snapshot(&self, _now_ms: i64) -> OperatorConsoleSnapshot {
            self.snapshot.clone()
        }

        fn narrow_mandate(&self, _changed_at_ms: i64) -> Result<()> {
            self.narrow_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn revoke_mandate(&self, _changed_at_ms: i64) -> Result<()> {
            self.revoke_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn test_source() -> Arc<TestSource> {
        Arc::new(TestSource {
            snapshot: OperatorConsoleSnapshot {
                generated_at_ms: 10_000,
                collector: Some(CollectorHealth {
                    network: Network::Signet,
                    status: CollectorStatus::Streaming,
                    authenticated: true,
                    subscribed_topics: BTreeSet::from(["ticker".into()]),
                    last_event_by_topic_ms: BTreeMap::from([("ticker".into(), 9_500)]),
                    last_backfill_at_ms: Some(9_000),
                    backfill_completed_surfaces: 3,
                    backfill_total_surfaces: 3,
                    backfill_rows: 42,
                    last_stream_event_at_ms: Some(9_500),
                    lag_ms: Some(500),
                    stored_events: 100,
                    last_error: None,
                }),
                venue_capabilities: None,
                strategies: vec![OperatorStrategySnapshot {
                    strategy_id: "funding_carry".into(),
                    status: "running".into(),
                    state: Some("monitoring funding settlement".into()),
                    last_action: Some("processed 1 intent".into()),
                    daily_loss_headroom_sats: Some(4_000),
                    order_headroom: Some(8),
                }],
                backtests: vec![OperatorBacktestSnapshot {
                    strategy_id: "funding_carry".into(),
                    outcome: "passed".into(),
                    created_at_ms: 8_000,
                    trade_count: 12,
                    expectancy_millisats: 2_500,
                    maximum_drawdown_sats: 75,
                    parameter_digest:
                        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
                }],
                ledger: Some(ProfitReport {
                    strategies: vec![StrategyProfit {
                        strategy_id: "funding_carry".into(),
                        profit_sats: 125,
                        ..Default::default()
                    }],
                    total_profit_sats: 125,
                    ..Default::default()
                }),
                mandate: Some(MandateSnapshot {
                    revision: 2,
                    mandates: vec![TradingMandate {
                        venue: "lnmarkets".into(),
                        network: TradingNetwork::Signet,
                        collateral_asset: AssetId::sats(),
                        objective: "Bounded profit".into(),
                        max_venue_balance: 100_000,
                        max_position_usd: 500,
                        max_leverage: 3,
                        daily_loss_stop: 5_000,
                        max_orders_per_hour: 12,
                        min_liquidation_buffer_bps: 1_500,
                        allowed_strategies: BTreeSet::from(["funding_carry".into()]),
                        review_cadence: ReviewCadence::Interval { seconds: 3_600 },
                        expires_at_ms: 86_400_000,
                    }],
                }),
                review_cadence: Some(ReviewCadence::Interval { seconds: 3_600 }),
                pending_wakeups: 1,
                review_history: vec![OperatorReviewTurn {
                    at_ms: 9_000,
                    trigger: "scheduled review".into(),
                    outcome: "completed".into(),
                }],
                runtime_error: None,
            },
            narrow_count: AtomicUsize::new(0),
            revoke_count: AtomicUsize::new(0),
        })
    }

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
    }

    #[gpui::test]
    fn operator_console_paints_every_operational_section(cx: &mut TestAppContext) {
        init_test(cx);
        let source = test_source();
        let (_panel, cx) =
            cx.add_window_view(|_window, cx| LnMarketsOperatorPanel::new(source, false, cx));
        cx.run_until_parked();
        let rendered = cx.debug_render_snapshot();

        for selector in [
            "lnmarkets.operator.panel",
            "lnmarkets.operator.collector",
            "lnmarkets.operator.capabilities",
            "lnmarkets.operator.strategies",
            "lnmarkets.operator.strategy.funding_carry",
            "lnmarkets.operator.backtests",
            "lnmarkets.operator.ledger",
            "lnmarkets.operator.mandate",
            "lnmarkets.operator.wakeups",
        ] {
            assert_eq!(
                rendered.occurrences(selector).len(),
                1,
                "operator console did not paint {selector} exactly once"
            );
        }
    }

    #[gpui::test]
    async fn mandate_reductions_run_without_an_approval_prompt(cx: &mut TestAppContext) {
        init_test(cx);
        let source = test_source();
        let (panel, cx) = cx.add_window_view({
            let source = source.clone();
            move |_window, cx| LnMarketsOperatorPanel::new(source, false, cx)
        });

        panel.update(cx, |panel, cx| panel.narrow_mandate(cx));
        cx.run_until_parked();
        assert_eq!(source.narrow_count.load(Ordering::SeqCst), 1);
        panel.update(cx, |panel, cx| panel.revoke_mandate(cx));
        cx.run_until_parked();
        assert_eq!(source.revoke_count.load(Ordering::SeqCst), 1);
    }
}
