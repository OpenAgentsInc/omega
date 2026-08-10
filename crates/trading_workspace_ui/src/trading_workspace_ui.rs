//! Capability-gated portfolio, trading, and analytics dock panels.

mod panels;
mod workspace_data;

use gpui::{App, actions};
use nautilus_sidecar::NautilusStreamSource;
use workspace::Workspace;

pub use panels::{
    AnalyticsWorkspacePanel, AnalyticsWorkspaceSurface, PortfolioWorkspacePanel,
    PortfolioWorkspaceSurface, TradingWorkspacePanel, TradingWorkspaceSurface,
};
pub use workspace_data::{
    AnalyticsPanelData, MandateCardData, PortfolioPanelData, TradingPanelData,
};

actions!(
    trading_workspace,
    [
        /// Toggles focus on the portfolio command-center panel.
        TogglePortfolioPanel,
        /// Toggles focus on the focused trading panel.
        ToggleTradingPanel,
        /// Toggles focus on the analytics and tearsheet panel.
        ToggleAnalyticsPanel
    ]
);

pub fn enabled(cx: &App) -> bool {
    NautilusStreamSource::try_global(cx).is_some()
}

pub fn init(cx: &mut App) {
    if !enabled(cx) {
        return;
    }
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace.register_action(|workspace, _: &TogglePortfolioPanel, window, cx| {
            workspace.toggle_panel_focus::<PortfolioWorkspacePanel>(window, cx);
        });
        workspace.register_action(|workspace, _: &ToggleTradingPanel, window, cx| {
            workspace.toggle_panel_focus::<TradingWorkspacePanel>(window, cx);
        });
        workspace.register_action(|workspace, _: &ToggleAnalyticsPanel, window, cx| {
            workspace.toggle_panel_focus::<AnalyticsWorkspacePanel>(window, cx);
        });
    })
    .detach();
}
