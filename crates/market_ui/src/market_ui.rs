//! NIP-MKT market panel on the Immortal transport-neutral client core
//! (omega#244).
//!
//! Omega owns the WebSocket, the NIP-11 fetch, and presentation. All event,
//! signature, and MKT-record validation is delegated to the pinned
//! `immortal-client` crate. The panel is a development surface and only
//! registers when `OMEGA_MARKET_PANEL=1`.

mod discovery;
mod panel;
mod session_flow;

use gpui::{App, actions};
use workspace::Workspace;

pub use discovery::{
    ConnectionState, DEFAULT_DEV_RELAY_URL, IngestOutcome, MKT_SWP_EXTENSION, MarketDiscovery,
    MarketDiscoveryConfig, MarketRelayGate, NIP_MKT_EXTENSION, NIP11_ACCEPT_MEDIA_TYPE,
    OfferingListing, ProviderListing, RELAY_URL_ENVIRONMENT_VARIABLE, SUBSCRIPTION_ID,
    validate_market_relay_information,
};
pub use panel::MarketPanel;
pub use session_flow::{
    SESSION_FLOW_TRACKING_ISSUE, SessionFlowAvailability, session_flow_availability,
};

actions!(
    market,
    [
        /// Toggles focus on the markets panel.
        ToggleFocus,
        /// Reconnects the markets panel to its configured relay.
        Reconnect
    ]
);

pub const MARKET_PANEL_ENVIRONMENT_VARIABLE: &str = "OMEGA_MARKET_PANEL";

/// Explicit development gate (PRODUCT.md): the Markets panel is absent from
/// normal builds until the negotiated session flow behind
/// `session_flow::session_flow_availability` is implemented.
pub fn market_panel_enabled() -> bool {
    std::env::var(MARKET_PANEL_ENVIRONMENT_VARIABLE).as_deref() == Ok("1")
}

pub fn init(cx: &mut App) {
    if !market_panel_enabled() {
        return;
    }
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
            workspace.toggle_panel_focus::<MarketPanel>(window, cx);
        });
    })
    .detach();
}
