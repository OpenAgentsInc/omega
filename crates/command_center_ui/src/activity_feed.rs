use agent_wakeup::WakeupSource;
use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{AnyElement, App, SharedString, Window, uniform_list};
use strategy_engine::StrategyLifecycleEvent;
use ui::{Indicator, prelude::*};

use crate::format::format_wall_clock;

/// The chat1-style roster state chip for one agent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentActivityState {
    Researching,
    Watching,
    Monitoring,
    Thinking,
    Halted,
}

impl AgentActivityState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Researching => "researching",
            Self::Watching => "watching",
            Self::Monitoring => "monitoring",
            Self::Thinking => "thinking",
            Self::Halted => "halted",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Self::Researching => Color::Accent,
            Self::Watching => Color::Info,
            Self::Monitoring => Color::Success,
            Self::Thinking => Color::Modified,
            Self::Halted => Color::Error,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AgentRosterEntry {
    pub name: SharedString,
    pub state: AgentActivityState,
    pub detail: Option<SharedString>,
}

/// The agent roster: one row per agent with a live state chip.
#[derive(IntoElement, RegisterComponent)]
pub struct AgentRoster {
    entries: Vec<AgentRosterEntry>,
}

impl AgentRoster {
    pub fn new(entries: Vec<AgentRosterEntry>) -> Self {
        Self { entries }
    }
}

impl RenderOnce for AgentRoster {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .id("command-center-roster")
            .debug_selector(|| "command_center.roster".into())
            .w_full()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().surface_background)
            .overflow_hidden()
            .when(self.entries.is_empty(), |this| {
                this.child(
                    div().px_3().py_2().child(
                        Label::new("No agents")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
                )
            })
            .children(self.entries.into_iter().enumerate().map(|(index, entry)| {
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .when(index > 0, |this| {
                        this.border_t_1()
                            .border_color(cx.theme().colors().border_variant)
                    })
                    .child(Indicator::dot().color(entry.state.color()))
                    .child(Label::new(entry.name).size(LabelSize::Small))
                    .child(
                        Label::new(entry.state.label())
                            .size(LabelSize::XSmall)
                            .color(entry.state.color()),
                    )
                    .when_some(entry.detail, |this, detail| {
                        this.child(
                            div().flex_1().overflow_hidden().text_right().child(
                                Label::new(detail)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                        )
                    })
            }))
    }
}

/// The type tag for one feed row, used for icon and color selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityEventKind {
    Wakeup,
    Review,
    Lifecycle,
    Order,
    Halt,
    Ledger,
    Prediction,
}

impl ActivityEventKind {
    pub fn icon(self) -> IconName {
        match self {
            Self::Wakeup => IconName::BellRing,
            Self::Review => IconName::Eye,
            Self::Lifecycle => IconName::PlayFilled,
            Self::Order => IconName::ArrowRightLeft,
            Self::Halt => IconName::Stop,
            Self::Ledger => IconName::Book,
            Self::Prediction => IconName::OmegaPredict,
        }
    }

    pub fn color(self) -> Color {
        match self {
            Self::Wakeup => Color::Accent,
            Self::Review => Color::Info,
            Self::Lifecycle => Color::Success,
            Self::Order => Color::Default,
            Self::Halt => Color::Error,
            Self::Ledger => Color::Muted,
            Self::Prediction => Color::Modified,
        }
    }
}

/// One timestamped row in the activity feed.
#[derive(Clone, Debug)]
pub struct ActivityEvent {
    pub at_ms: i64,
    pub kind: ActivityEventKind,
    pub title: SharedString,
    pub detail: Option<SharedString>,
}

impl ActivityEvent {
    /// Adapter from a strategy-engine lifecycle event. Variants that do not
    /// carry their own timestamp use `observed_at_ms`.
    pub fn from_lifecycle(event: &StrategyLifecycleEvent, observed_at_ms: i64) -> Self {
        match event {
            StrategyLifecycleEvent::Started { strategy_id, at_ms } => Self {
                at_ms: *at_ms,
                kind: ActivityEventKind::Lifecycle,
                title: format!("{strategy_id} started").into(),
                detail: None,
            },
            StrategyLifecycleEvent::TickProcessed {
                strategy_id,
                at_ms,
                intent_count,
            } => Self {
                at_ms: *at_ms,
                kind: ActivityEventKind::Lifecycle,
                title: format!("{strategy_id} tick").into(),
                detail: Some(format!("{intent_count} intents").into()),
            },
            StrategyLifecycleEvent::OrderAuthorized {
                strategy_id,
                intent_id,
                mandate_revision,
            } => Self {
                at_ms: observed_at_ms,
                kind: ActivityEventKind::Order,
                title: format!("{strategy_id} order authorized").into(),
                detail: Some(format!("{intent_id} · mandate r{mandate_revision}").into()),
            },
            StrategyLifecycleEvent::OrderSubmitted {
                strategy_id,
                intent_id,
                venue_order_id,
            } => Self {
                at_ms: observed_at_ms,
                kind: ActivityEventKind::Order,
                title: format!("{strategy_id} order submitted").into(),
                detail: Some(format!("{intent_id} → {venue_order_id}").into()),
            },
            StrategyLifecycleEvent::CancelResolved {
                strategy_id,
                intent_id,
                outcome,
                ..
            } => Self {
                at_ms: observed_at_ms,
                kind: ActivityEventKind::Order,
                title: format!("{strategy_id} cancel resolved").into(),
                detail: Some(format!("{intent_id} · {outcome:?}").into()),
            },
            StrategyLifecycleEvent::BacktestApproved {
                strategy_id,
                report_digest,
            } => Self {
                at_ms: observed_at_ms,
                kind: ActivityEventKind::Review,
                title: format!("{strategy_id} backtest approved").into(),
                detail: Some(report_digest.clone().into()),
            },
            StrategyLifecycleEvent::StateUpdated {
                strategy_id, at_ms, ..
            } => Self {
                at_ms: *at_ms,
                kind: ActivityEventKind::Lifecycle,
                title: format!("{strategy_id} state updated").into(),
                detail: None,
            },
            StrategyLifecycleEvent::LedgerEntryAppended {
                strategy_id,
                event_id,
                sequence,
            } => Self {
                at_ms: observed_at_ms,
                kind: ActivityEventKind::Ledger,
                title: format!("{strategy_id} ledger entry").into(),
                detail: Some(format!("{event_id} · seq {sequence}").into()),
            },
            StrategyLifecycleEvent::Halted {
                strategy_id,
                at_ms,
                reason,
            } => Self {
                at_ms: *at_ms,
                kind: ActivityEventKind::Halt,
                title: format!("{strategy_id} halted").into(),
                detail: Some(format!("{reason:?}").into()),
            },
        }
    }

    /// Adapter from an agent-wakeup source.
    pub fn from_wakeup(source: &WakeupSource, at_ms: i64) -> Self {
        Self {
            at_ms,
            kind: ActivityEventKind::Wakeup,
            title: source.transcript_label().into(),
            detail: None,
        }
    }
}

/// The timestamped "LATEST" stream: wakeups, review outcomes, strategy
/// lifecycle events, orders, and halts, newest first, virtualized.
#[derive(IntoElement, RegisterComponent)]
pub struct ActivityFeed {
    events: Vec<ActivityEvent>,
    height: Pixels,
}

impl ActivityFeed {
    /// Rows are sorted newest first regardless of input order.
    pub fn new(mut events: Vec<ActivityEvent>) -> Self {
        events.sort_by_key(|event| std::cmp::Reverse(event.at_ms));
        Self {
            events,
            height: px(240.),
        }
    }

    pub fn height(mut self, height: Pixels) -> Self {
        self.height = height;
        self
    }
}

fn feed_row(event: &ActivityEvent) -> AnyElement {
    h_flex()
        .w_full()
        .px_3()
        .py_1()
        .gap_2()
        .child(
            Label::new(format_wall_clock(event.at_ms))
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .child(
            Icon::new(event.kind.icon())
                .size(IconSize::XSmall)
                .color(event.kind.color()),
        )
        .child(Label::new(event.title.clone()).size(LabelSize::Small))
        .when_some(event.detail.clone(), |this, detail| {
            this.child(
                div().flex_1().overflow_hidden().text_right().child(
                    Label::new(detail)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
            )
        })
        .into_any_element()
}

impl RenderOnce for ActivityFeed {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let count = self.events.len();
        let events = self.events;
        v_flex()
            .id("command-center-activity-feed")
            .debug_selector(|| "command_center.activity_feed".into())
            .w_full()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().surface_background)
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .px_3()
                    .py_1p5()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(Label::new("Latest").size(LabelSize::Small))
                    .child(
                        Label::new(format!("{count} events"))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .when(count == 0, |this| {
                this.child(
                    div().px_3().py_2().child(
                        Label::new("No activity yet")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
                )
            })
            .when(count > 0, |this| {
                this.child(
                    uniform_list(
                        "command-center-activity-feed-rows",
                        count,
                        move |range, _window, _cx| {
                            range
                                .filter_map(|index| Some(feed_row(events.get(index)?)))
                                .collect()
                        },
                    )
                    .h(self.height)
                    .w_full(),
                )
            })
    }
}

/// Demo roster used by the component-library preview.
pub fn demo_roster_entries() -> Vec<AgentRosterEntry> {
    vec![
        AgentRosterEntry {
            name: "carry-agent".into(),
            state: AgentActivityState::Monitoring,
            detail: Some("funding_carry · signet".into()),
        },
        AgentRosterEntry {
            name: "swing-agent".into(),
            state: AgentActivityState::Thinking,
            detail: Some("threshold_swing · reviewing tick".into()),
        },
        AgentRosterEntry {
            name: "research-agent".into(),
            state: AgentActivityState::Researching,
            detail: Some("scanning funding history".into()),
        },
        AgentRosterEntry {
            name: "rebalance-agent".into(),
            state: AgentActivityState::Watching,
            detail: Some("rebalance_to_target · idle".into()),
        },
        AgentRosterEntry {
            name: "old-swing-agent".into(),
            state: AgentActivityState::Halted,
            detail: Some("daily loss stop".into()),
        },
    ]
}

/// Demo mixed event stream used by the component-library preview.
pub fn demo_activity_events() -> Vec<ActivityEvent> {
    let base_ms = 1_786_276_800_000_i64;
    let mut events = vec![
        ActivityEvent::from_wakeup(
            &WakeupSource::ScheduledReview {
                cadence: "every 15m".into(),
            },
            base_ms - 15 * 60_000,
        ),
        ActivityEvent::from_wakeup(
            &WakeupSource::FundingSignFlip {
                previous_bps: 3,
                current_bps: -2,
            },
            base_ms - 11 * 60_000,
        ),
        ActivityEvent::from_lifecycle(
            &StrategyLifecycleEvent::Started {
                strategy_id: "funding_carry".into(),
                at_ms: base_ms - 10 * 60_000,
            },
            base_ms,
        ),
        ActivityEvent::from_lifecycle(
            &StrategyLifecycleEvent::OrderAuthorized {
                strategy_id: "funding_carry".into(),
                intent_id: "intent-104".into(),
                mandate_revision: 4,
            },
            base_ms - 9 * 60_000,
        ),
        ActivityEvent::from_lifecycle(
            &StrategyLifecycleEvent::OrderSubmitted {
                strategy_id: "funding_carry".into(),
                intent_id: "intent-104".into(),
                venue_order_id: "ord-77120".into(),
            },
            base_ms - 9 * 60_000 + 1_800,
        ),
        ActivityEvent {
            at_ms: base_ms - 7 * 60_000,
            kind: ActivityEventKind::Review,
            title: "review turn completed".into(),
            detail: Some("no action · 1,204 tokens".into()),
        },
        ActivityEvent {
            at_ms: base_ms - 6 * 60_000,
            kind: ActivityEventKind::Prediction,
            title: "prediction emitted".into(),
            detail: Some("BTCUSD up · 72% · 1h horizon".into()),
        },
        ActivityEvent::from_lifecycle(
            &StrategyLifecycleEvent::LedgerEntryAppended {
                strategy_id: "funding_carry".into(),
                event_id: "fill-ord-77120".into(),
                sequence: 3_212,
            },
            base_ms - 4 * 60_000,
        ),
        ActivityEvent::from_lifecycle(
            &StrategyLifecycleEvent::Halted {
                strategy_id: "threshold_swing".into(),
                at_ms: base_ms - 2 * 60_000,
                reason: strategy_engine::StrategyHaltReason::Manual {
                    reason: "operator paused for review".into(),
                },
            },
            base_ms,
        ),
    ];
    events.sort_by_key(|event| std::cmp::Reverse(event.at_ms));
    events
}

impl Component for AgentRoster {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        "Per-agent roster rows with live state chips (researching, watching, monitoring, thinking, halted)."
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        v_flex()
            .gap_6()
            .child(
                example_group_with_title(
                    "Agent roster",
                    vec![
                        single_example(
                            "Mixed states",
                            AgentRoster::new(demo_roster_entries()).into_any_element(),
                        )
                        .width(px(520.)),
                        single_example("Empty", AgentRoster::new(Vec::new()).into_any_element())
                            .width(px(520.)),
                    ],
                )
                .vertical(),
            )
            .into_any_element()
    }
}

impl Component for ActivityFeed {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn description() -> &'static str {
        "Timestamped activity stream of wakeups, review outcomes, strategy lifecycle events, orders, and halts, newest first and virtualized."
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        v_flex()
            .gap_6()
            .child(
                example_group_with_title(
                    "Activity feed",
                    vec![
                        single_example(
                            "Mixed stream",
                            ActivityFeed::new(demo_activity_events()).into_any_element(),
                        )
                        .width(px(640.)),
                        single_example("Empty", ActivityFeed::new(Vec::new()).into_any_element())
                            .width(px(640.)),
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
    use strategy_engine::StrategyHaltReason;

    #[test]
    fn lifecycle_events_map_to_typed_feed_rows() {
        let halted = ActivityEvent::from_lifecycle(
            &StrategyLifecycleEvent::Halted {
                strategy_id: "funding_carry".into(),
                at_ms: 42_000,
                reason: StrategyHaltReason::Manual {
                    reason: "paused".into(),
                },
            },
            99_000,
        );
        assert_eq!(halted.kind, ActivityEventKind::Halt);
        assert_eq!(halted.at_ms, 42_000);
        assert_eq!(halted.title.as_ref(), "funding_carry halted");

        let authorized = ActivityEvent::from_lifecycle(
            &StrategyLifecycleEvent::OrderAuthorized {
                strategy_id: "funding_carry".into(),
                intent_id: "intent-1".into(),
                mandate_revision: 7,
            },
            99_000,
        );
        assert_eq!(authorized.kind, ActivityEventKind::Order);
        assert_eq!(
            authorized.at_ms, 99_000,
            "untimestamped lifecycle variants use the observation time"
        );
    }

    #[test]
    fn wakeups_use_their_transcript_label() {
        let event = ActivityEvent::from_wakeup(
            &WakeupSource::DrawdownLimitApproach {
                drawdown_sats: 900,
                limit_sats: 1_000,
            },
            5_000,
        );
        assert_eq!(event.kind, ActivityEventKind::Wakeup);
        assert_eq!(event.title.as_ref(), "event: drawdown limit approach");
    }

    #[test]
    fn feed_sorts_newest_first() {
        let feed = ActivityFeed::new(vec![
            ActivityEvent {
                at_ms: 1,
                kind: ActivityEventKind::Ledger,
                title: "old".into(),
                detail: None,
            },
            ActivityEvent {
                at_ms: 9,
                kind: ActivityEventKind::Ledger,
                title: "new".into(),
                detail: None,
            },
        ]);
        let titles: Vec<_> = feed
            .events
            .iter()
            .map(|event| event.title.to_string())
            .collect();
        assert_eq!(titles, ["new", "old"]);
    }
}
