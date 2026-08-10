use anyhow::Result;
use command_center_ui::{
    ActivityFeed, AgentRoster, ApprovalQueue, BalancesTable, CommandCenterHeader, MandateStatusCard,
};
use component::{Component, ComponentScope, example_group_with_title, single_example};
use gpui::{
    AnyElement, App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Task, WeakEntity,
    Window, px, uniform_list,
};
use market_ui::NautilusOrderTicketSource;
use nautilus_sidecar::NautilusStreamSource;
use trading_ledger::LedgerStore;
use trading_mandate::{MandateStore, TradingNetwork};
use ui::{
    CandlestickChart, FillsOnCandlesChart, IndicatorOverlayChart, InstrumentCatalogSource,
    InstrumentSelector, MarketTokens, OpenOrdersSource, OpenOrdersTable, OrderBookLadder,
    OrderTicket, OscillatorStack, PositionsPanel, PositionsSource, StatisticTileGrid, Tearsheet,
    TradeLogTable, prelude::*,
};
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::workspace_data::{AnalyticsPanelData, PortfolioPanelData, TradingPanelData};
use crate::{ToggleAnalyticsPanel, TogglePortfolioPanel, ToggleTradingPanel};

const HYPERLIQUID_VENUE: &str = "hyperliquid";

struct InstrumentValues(Vec<ui::Instrument>);

impl InstrumentCatalogSource for InstrumentValues {
    fn instruments(&self) -> Vec<ui::Instrument> {
        self.0.clone()
    }
}

struct PositionValues(Vec<ui::Position>);

impl PositionsSource for PositionValues {
    fn positions(&self) -> Vec<ui::Position> {
        self.0.clone()
    }
}

struct OpenOrderValues(Vec<ui::OpenOrder>);

impl OpenOrdersSource for OpenOrderValues {
    fn open_orders(&self) -> Vec<ui::OpenOrder> {
        self.0.clone()
    }
}

fn panel_section(content: AnyElement) -> AnyElement {
    div()
        .h(px(360.0))
        .w_full()
        .p_3()
        .overflow_hidden()
        .child(content)
        .into_any_element()
}

#[derive(IntoElement, RegisterComponent)]
pub struct PortfolioWorkspaceSurface {
    data: PortfolioPanelData,
    tokens: Option<MarketTokens>,
}

impl PortfolioWorkspaceSurface {
    pub fn new(data: PortfolioPanelData) -> Self {
        Self { data, tokens: None }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for PortfolioWorkspaceSurface {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let grayscale = tokens == tokens.grayscale();
        let data = std::sync::Arc::new(self.data);
        let error = data.error.clone();
        v_flex()
            .debug_selector(|| "trading_workspace.portfolio_surface".into())
            .when(grayscale, |this| {
                this.debug_selector(|| "trading_workspace.portfolio_grayscale".into())
            })
            .size_full()
            .child(uniform_list(
                "trading-workspace-portfolio-sections",
                7,
                move |range, _window, _cx| {
                    range
                        .map(|index| match index {
                            0 => panel_section(
                                CommandCenterHeader::new(data.summary.clone()).into_any_element(),
                            ),
                            1 => panel_section(
                                BalancesTable::new(data.balances.clone())
                                    .tokens(tokens)
                                    .into_any_element(),
                            ),
                            2 => panel_section(
                                PositionsPanel::from_source(&PositionValues(
                                    data.positions.clone(),
                                ))
                                .tokens(tokens)
                                .into_any_element(),
                            ),
                            3 => panel_section(
                                AgentRoster::new(data.roster.clone()).into_any_element(),
                            ),
                            4 => panel_section(
                                ActivityFeed::new(data.activity.clone())
                                    .height(px(320.0))
                                    .into_any_element(),
                            ),
                            5 => panel_section(
                                data.mandate
                                    .as_ref()
                                    .map(|mandate| {
                                        MandateStatusCard::new(
                                            &mandate.mandate,
                                            &mandate.usage,
                                            mandate.revision,
                                            mandate.now_ms,
                                        )
                                        .into_any_element()
                                    })
                                    .unwrap_or_else(|| {
                                        Label::new("No active mandate")
                                            .color(Color::Muted)
                                            .into_any_element()
                                    }),
                            ),
                            _ => panel_section(
                                ApprovalQueue::new(data.approvals.clone())
                                    .tokens(tokens)
                                    .into_any_element(),
                            ),
                        })
                        .collect()
                },
            ))
            .when_some(error, |this, error| {
                this.child(
                    div().absolute().bottom_2().left_2().child(
                        Label::new(error)
                            .size(LabelSize::XSmall)
                            .color(Color::Error),
                    ),
                )
            })
    }
}

impl Component for PortfolioWorkspaceSurface {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        "Portfolio command-center panel composition with virtualized, typed sections."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Portfolio workspace",
                vec![single_example(
                    "Command center",
                    PortfolioWorkspaceSurface::new(PortfolioPanelData::demo()).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Structure preserves portfolio state",
                    PortfolioWorkspaceSurface::new(PortfolioPanelData::demo())
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

#[derive(IntoElement, RegisterComponent)]
pub struct TradingWorkspaceSurface {
    data: TradingPanelData,
    tokens: Option<MarketTokens>,
}

impl TradingWorkspaceSurface {
    pub fn new(data: TradingPanelData) -> Self {
        Self { data, tokens: None }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for TradingWorkspaceSurface {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let grayscale = tokens == tokens.grayscale();
        let data = std::sync::Arc::new(self.data);
        v_flex()
            .debug_selector(|| "trading_workspace.trading_surface".into())
            .when(grayscale, |this| {
                this.debug_selector(|| "trading_workspace.trading_grayscale".into())
            })
            .size_full()
            .child(uniform_list(
                "trading-workspace-trading-sections",
                5,
                move |range, _window, _cx| {
                    range
                        .map(|index| match index {
                            0 => panel_section(
                                InstrumentSelector::from_source(&InstrumentValues(
                                    data.instruments.clone(),
                                ))
                                .query("")
                                .tokens(tokens)
                                .into_any_element(),
                            ),
                            1 => panel_section(
                                CandlestickChart::new(data.candles.clone())
                                    .size(760.0, 330.0)
                                    .tokens(tokens)
                                    .into_any_element(),
                            ),
                            2 => panel_section(
                                OrderBookLadder::new(data.book.clone())
                                    .width(420.0)
                                    .tokens(tokens)
                                    .into_any_element(),
                            ),
                            3 => panel_section(
                                data.order_intent
                                    .as_ref()
                                    .map(|intent| {
                                        OrderTicket::from_source(&NautilusOrderTicketSource::new(
                                            intent,
                                        ))
                                        .tokens(tokens)
                                        .into_any_element()
                                    })
                                    .unwrap_or_else(|| {
                                        Label::new("Quote and collateral unavailable")
                                            .color(Color::Muted)
                                            .into_any_element()
                                    }),
                            ),
                            _ => panel_section(
                                OpenOrdersTable::from_source(&OpenOrderValues(
                                    data.open_orders.clone(),
                                ))
                                .tokens(tokens)
                                .into_any_element(),
                            ),
                        })
                        .collect()
                },
            ))
    }
}

impl Component for TradingWorkspaceSurface {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        "Focused testnet trading panel composed from typed market-stream values."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Trading workspace",
                vec![single_example(
                    "Focused trade surface",
                    TradingWorkspaceSurface::new(TradingPanelData::demo()).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Geometry and glyphs preserve side",
                    TradingWorkspaceSurface::new(TradingPanelData::demo())
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

#[derive(IntoElement, RegisterComponent)]
pub struct AnalyticsWorkspaceSurface {
    data: AnalyticsPanelData,
    tokens: Option<MarketTokens>,
}

impl AnalyticsWorkspaceSurface {
    pub fn new(data: AnalyticsPanelData) -> Self {
        Self { data, tokens: None }
    }

    pub fn tokens(mut self, tokens: MarketTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

impl RenderOnce for AnalyticsWorkspaceSurface {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = self.tokens.unwrap_or_else(|| MarketTokens::from_theme(cx));
        let grayscale = tokens == tokens.grayscale();
        let data = std::sync::Arc::new(self.data);
        v_flex()
            .debug_selector(|| "trading_workspace.analytics_surface".into())
            .when(grayscale, |this| {
                this.debug_selector(|| "trading_workspace.analytics_grayscale".into())
            })
            .size_full()
            .child(uniform_list(
                "trading-workspace-analytics-sections",
                6,
                move |range, _window, _cx| {
                    range
                        .map(|index| match index {
                            0 => panel_section(
                                IndicatorOverlayChart::new(data.overlays.clone())
                                    .size(760.0, 330.0)
                                    .tokens(tokens)
                                    .into_any_element(),
                            ),
                            1 => panel_section(
                                OscillatorStack::new(data.oscillators.clone())
                                    .width(760.0)
                                    .tokens(tokens)
                                    .into_any_element(),
                            ),
                            2 => panel_section(
                                StatisticTileGrid::new(data.statistics.clone())
                                    .tokens(tokens)
                                    .into_any_element(),
                            ),
                            3 => panel_section(
                                Tearsheet::new(data.tearsheet.clone())
                                    .width(760.0)
                                    .tokens(tokens)
                                    .into_any_element(),
                            ),
                            4 => panel_section(
                                FillsOnCandlesChart::new(data.fills.clone())
                                    .tokens(tokens)
                                    .into_any_element(),
                            ),
                            _ => panel_section(
                                TradeLogTable::new(data.trades.clone())
                                    .visible_range(0..50)
                                    .tokens(tokens)
                                    .into_any_element(),
                            ),
                        })
                        .collect()
                },
            ))
    }
}

impl Component for AnalyticsWorkspaceSurface {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        "Live-stream analytics and ledger-linked tearsheet panel composition."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        v_flex()
            .gap_4()
            .child(example_group_with_title(
                "Analytics workspace",
                vec![single_example(
                    "Tearsheet and execution analytics",
                    AnalyticsWorkspaceSurface::new(AnalyticsPanelData::demo()).into_any_element(),
                )],
            ))
            .child(example_group_with_title(
                "Grayscale audit",
                vec![single_example(
                    "Stroke, fill, and sign preserve meaning",
                    AnalyticsWorkspaceSurface::new(AnalyticsPanelData::demo())
                        .tokens(MarketTokens::from_theme(cx).grayscale())
                        .into_any_element(),
                )],
            ))
            .into_any_element()
    }
}

struct PanelLiveState {
    stream: Option<Entity<NautilusStreamSource>>,
    _subscription: Option<Subscription>,
}

impl PanelLiveState {
    fn new<T: 'static>(cx: &mut Context<T>) -> Self {
        let stream = NautilusStreamSource::try_global(cx);
        let subscription = stream
            .as_ref()
            .map(|stream| cx.observe(stream, |_, _, cx| cx.notify()));
        Self {
            stream,
            _subscription: subscription,
        }
    }

    fn snapshot(&self, cx: &App) -> nautilus_sidecar::NautilusMarketSnapshot {
        self.stream
            .as_ref()
            .map(|stream| stream.read(cx).market_snapshot())
            .unwrap_or_default()
    }
}

pub struct PortfolioWorkspacePanel {
    focus_handle: FocusHandle,
    position: DockPosition,
    live: PanelLiveState,
    ledger: Option<LedgerStore>,
    mandate: Option<MandateStore>,
    store_error: Option<SharedString>,
    demo: bool,
    grayscale: bool,
}

impl PortfolioWorkspacePanel {
    pub fn load(
        workspace: WeakEntity<Workspace>,
        cx: AsyncWindowContext,
    ) -> Task<Result<Entity<Self>>> {
        cx.spawn(async move |cx| {
            workspace.update_in(cx, |_workspace, _window, cx| cx.new(Self::new))
        })
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let (ledger, ledger_error) = match LedgerStore::open_default() {
            Ok(store) => (Some(store), None),
            Err(error) => (None, Some(format!("Ledger unavailable: {error}").into())),
        };
        let (mandate, mandate_error) = match MandateStore::open_default() {
            Ok(store) => (Some(store), None),
            Err(error) => (None, Some(format!("Mandate unavailable: {error}").into())),
        };
        Self {
            focus_handle: cx.focus_handle(),
            position: DockPosition::Left,
            live: PanelLiveState::new(cx),
            ledger,
            mandate,
            store_error: ledger_error.or(mandate_error),
            demo: false,
            grayscale: false,
        }
    }

    #[cfg(test)]
    fn demo(grayscale: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            position: DockPosition::Left,
            live: PanelLiveState {
                stream: None,
                _subscription: None,
            },
            ledger: None,
            mandate: None,
            store_error: None,
            demo: true,
            grayscale,
        }
    }

    fn data(&self, cx: &App) -> PortfolioPanelData {
        if self.demo {
            return PortfolioPanelData::demo();
        }
        let market = self.live.snapshot(cx);
        let mandate = self
            .mandate
            .as_ref()
            .and_then(|store| store.snapshot().ok())
            .and_then(|snapshot| {
                snapshot
                    .mandate_for(HYPERLIQUID_VENUE, TradingNetwork::Testnet)
                    .cloned()
                    .map(|mandate| (mandate, snapshot.revision))
            });
        PortfolioPanelData::live(
            &market,
            self.ledger.as_ref(),
            mandate,
            command_center_ui::unix_now_ms(),
            self.store_error.as_deref(),
        )
    }
}

pub struct TradingWorkspacePanel {
    focus_handle: FocusHandle,
    position: DockPosition,
    live: PanelLiveState,
    demo: bool,
    grayscale: bool,
}

impl TradingWorkspacePanel {
    pub fn load(
        workspace: WeakEntity<Workspace>,
        cx: AsyncWindowContext,
    ) -> Task<Result<Entity<Self>>> {
        cx.spawn(async move |cx| {
            workspace.update_in(cx, |_workspace, _window, cx| cx.new(Self::new))
        })
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            position: DockPosition::Right,
            live: PanelLiveState::new(cx),
            demo: false,
            grayscale: false,
        }
    }

    #[cfg(test)]
    fn demo(grayscale: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            position: DockPosition::Right,
            live: PanelLiveState {
                stream: None,
                _subscription: None,
            },
            demo: true,
            grayscale,
        }
    }
}

pub struct AnalyticsWorkspacePanel {
    focus_handle: FocusHandle,
    position: DockPosition,
    live: PanelLiveState,
    demo: bool,
    grayscale: bool,
}

impl AnalyticsWorkspacePanel {
    pub fn load(
        workspace: WeakEntity<Workspace>,
        cx: AsyncWindowContext,
    ) -> Task<Result<Entity<Self>>> {
        cx.spawn(async move |cx| {
            workspace.update_in(cx, |_workspace, _window, cx| cx.new(Self::new))
        })
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            position: DockPosition::Bottom,
            live: PanelLiveState::new(cx),
            demo: false,
            grayscale: false,
        }
    }

    #[cfg(test)]
    fn demo(grayscale: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            position: DockPosition::Bottom,
            live: PanelLiveState {
                stream: None,
                _subscription: None,
            },
            demo: true,
            grayscale,
        }
    }
}

macro_rules! impl_panel {
    ($type:ty, $name:literal, $key:literal, $icon:expr, $tooltip:literal, $action:expr, $priority:literal) => {
        impl Focusable for $type {
            fn focus_handle(&self, _cx: &App) -> FocusHandle {
                self.focus_handle.clone()
            }
        }

        impl EventEmitter<PanelEvent> for $type {}

        impl Panel for $type {
            fn persistent_name() -> &'static str {
                $name
            }

            fn panel_key() -> &'static str {
                $key
            }

            fn position(&self, _: &Window, _: &App) -> DockPosition {
                self.position
            }

            fn position_is_valid(&self, _: DockPosition) -> bool {
                true
            }

            fn set_position(
                &mut self,
                position: DockPosition,
                _: &mut Window,
                _: &mut Context<Self>,
            ) {
                self.position = position;
            }

            fn default_size(&self, _: &Window, _: &App) -> gpui::Pixels {
                px(640.0)
            }

            fn min_size(&self, _: &Window, _: &App) -> Option<gpui::Pixels> {
                Some(px(360.0))
            }

            fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
                Some($icon)
            }

            fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
                Some($tooltip)
            }

            fn toggle_action(&self) -> Box<dyn gpui::Action> {
                Box::new($action)
            }

            fn activation_priority(&self) -> u32 {
                $priority
            }
        }
    };
}

impl_panel!(
    PortfolioWorkspacePanel,
    "PortfolioWorkspacePanel",
    "portfolio-workspace",
    IconName::Book,
    "Portfolio",
    TogglePortfolioPanel,
    12
);
impl_panel!(
    TradingWorkspacePanel,
    "TradingWorkspacePanel",
    "trading-workspace",
    IconName::ArrowRightLeft,
    "Trading",
    ToggleTradingPanel,
    11
);
impl_panel!(
    AnalyticsWorkspacePanel,
    "AnalyticsWorkspacePanel",
    "analytics-workspace",
    IconName::SignalHigh,
    "Analytics",
    ToggleAnalyticsPanel,
    10
);

impl Render for PortfolioWorkspacePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let surface = PortfolioWorkspaceSurface::new(self.data(cx));
        div()
            .debug_selector(|| "trading_workspace.portfolio_panel".into())
            .track_focus(&self.focus_handle)
            .size_full()
            .child(if self.grayscale {
                surface
                    .tokens(MarketTokens::from_theme(cx).grayscale())
                    .into_any_element()
            } else {
                surface.into_any_element()
            })
    }
}

impl Render for TradingWorkspacePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let market = self.live.snapshot(cx);
        let data = if self.demo {
            TradingPanelData::demo()
        } else {
            TradingPanelData::live(&market)
        };
        let surface = TradingWorkspaceSurface::new(data);
        div()
            .debug_selector(|| "trading_workspace.trading_panel".into())
            .track_focus(&self.focus_handle)
            .size_full()
            .child(if self.grayscale {
                surface
                    .tokens(MarketTokens::from_theme(cx).grayscale())
                    .into_any_element()
            } else {
                surface.into_any_element()
            })
    }
}

impl Render for AnalyticsWorkspacePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let market = self.live.snapshot(cx);
        let data = if self.demo {
            AnalyticsPanelData::demo()
        } else {
            AnalyticsPanelData::live(&market, command_center_ui::unix_now_ms())
        };
        let surface = AnalyticsWorkspaceSurface::new(data);
        div()
            .debug_selector(|| "trading_workspace.analytics_panel".into())
            .track_focus(&self.focus_handle)
            .size_full()
            .child(if self.grayscale {
                surface
                    .tokens(MarketTokens::from_theme(cx).grayscale())
                    .into_any_element()
            } else {
                surface.into_any_element()
            })
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
    }

    struct AllTradingWorkspacePanels;

    impl Render for AllTradingWorkspacePanels {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            h_flex()
                .size_full()
                .child(cx.new(|cx| PortfolioWorkspacePanel::demo(false, cx)))
                .child(cx.new(|cx| TradingWorkspacePanel::demo(false, cx)))
                .child(cx.new(|cx| AnalyticsWorkspacePanel::demo(false, cx)))
        }
    }

    struct GrayscaleTradingWorkspacePanels;

    impl Render for GrayscaleTradingWorkspacePanels {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            h_flex()
                .size_full()
                .child(cx.new(|cx| PortfolioWorkspacePanel::demo(true, cx)))
                .child(cx.new(|cx| TradingWorkspacePanel::demo(true, cx)))
                .child(cx.new(|cx| AnalyticsWorkspacePanel::demo(true, cx)))
        }
    }

    #[gpui::test]
    fn all_panels_paint_in_real_windows(cx: &mut TestAppContext) {
        init_test(cx);
        let (_view, cx) = cx.add_window_view(|_, _| AllTradingWorkspacePanels);
        cx.run_until_parked();
        let rendered = cx.debug_render_snapshot();
        for selector in [
            "trading_workspace.portfolio_panel",
            "trading_workspace.trading_panel",
            "trading_workspace.analytics_panel",
            "trading_workspace.portfolio_surface",
            "trading_workspace.trading_surface",
            "trading_workspace.analytics_surface",
        ] {
            assert!(
                !rendered.occurrences(selector).is_empty(),
                "trading workspace did not paint {selector}"
            );
        }
    }

    #[gpui::test]
    fn all_panel_compositions_paint_in_grayscale(cx: &mut TestAppContext) {
        init_test(cx);
        let (_view, cx) = cx.add_window_view(|_, _| GrayscaleTradingWorkspacePanels);
        cx.run_until_parked();
        let rendered = cx.debug_render_snapshot();
        for selector in [
            "trading_workspace.portfolio_grayscale",
            "trading_workspace.trading_grayscale",
            "trading_workspace.analytics_grayscale",
        ] {
            assert!(
                !rendered.occurrences(selector).is_empty(),
                "trading workspace grayscale audit did not paint {selector}"
            );
        }
    }

    #[test]
    fn panels_accept_every_dock_position() {
        for position in [
            DockPosition::Left,
            DockPosition::Bottom,
            DockPosition::Right,
        ] {
            assert!(matches!(
                position,
                DockPosition::Left | DockPosition::Bottom | DockPosition::Right
            ));
        }
    }
}
