//! NIP-MKT market panel on the Immortal transport-neutral client core
//! (omega#244).
//!
//! Omega owns the WebSocket, the NIP-11 fetch, and presentation. All event,
//! signature, and MKT-record validation is delegated to the pinned
//! `immortal-client` crate. The panel is a development surface and only
//! registers when `OMEGA_MARKET_PANEL=1`.

mod components;
#[cfg(not(target_arch = "wasm32"))]
mod discovery;
#[cfg(not(target_arch = "wasm32"))]
mod nautilus_live;
#[cfg(not(target_arch = "wasm32"))]
mod nautilus_order;
#[cfg(not(target_arch = "wasm32"))]
mod network_transport;
#[cfg(not(target_arch = "wasm32"))]
mod panel;
#[cfg(not(target_arch = "wasm32"))]
mod receipt_ledger;
#[cfg(not(target_arch = "wasm32"))]
mod session_flow;
#[cfg(not(target_arch = "wasm32"))]
mod session_transport;
mod view_model;

#[cfg(not(target_arch = "wasm32"))]
use gpui::{App, actions};
#[cfg(not(target_arch = "wasm32"))]
use workspace::Workspace;

pub use components::{
    CustodyStrip, ExitPackageBadge, ExpiryCountdown, FeeBreakdown, OfferingCard,
    PriceFeedProvenance, ProviderBadge, QuoteCompareTable, ReceiptCard, ReservationBadge,
    RungLabel, SessionTimeline, SwapFlow, TypedErrorMessage, VerifyChecklist,
};
#[cfg(not(target_arch = "wasm32"))]
pub use discovery::{
    ConnectionState, DEFAULT_DEV_RELAY_URL, IngestOutcome, MKT_SWP_EXTENSION, MarketDiscovery,
    MarketDiscoveryConfig, MarketRelayGate, NIP_MKT_EXTENSION, NIP11_ACCEPT_MEDIA_TYPE,
    OfferingListing, OfferingSideListing, ProviderListing, RELAY_URL_ENVIRONMENT_VARIABLE,
    SUBSCRIPTION_ID, validate_market_relay_information,
};
#[cfg(not(target_arch = "wasm32"))]
pub use nautilus_live::{
    NautilusAccountSummary, NautilusBookSource, NautilusCandleSource, NautilusLiveSnapshot,
};
#[cfg(not(target_arch = "wasm32"))]
pub use nautilus_order::{
    LiveOrderState, NautilusOrderConfirmationSource, NautilusOrderIntent, NautilusOrderPreview,
    NautilusOrderTicketSource,
};
#[cfg(not(target_arch = "wasm32"))]
pub use network_transport::{
    MultiRelayStatus, ProviderNetworkState, RelayAvailability, RelaySetPlan, fanout_exact_event,
};
#[cfg(not(target_arch = "wasm32"))]
pub use panel::MarketPanel;
#[cfg(not(target_arch = "wasm32"))]
pub use receipt_ledger::{
    RECEIPT_EXPORT_DIRECTORY, RECEIPT_EXPORT_SCHEMA, RECEIPT_LEDGER_SCHEMA,
    RECEIPT_LEDGER_STRATEGY, ReceiptVerification, export_verified_receipt,
    persist_verified_receipt, receipt_ledger_drafts, verify_receipts,
    verify_receipts_with_provider_keys,
};
#[cfg(not(target_arch = "wasm32"))]
pub use session_flow::{
    AcknowledgmentEntry, AdmitOutcome, CancelEntry, CloseEntry, IntentProgress, MarketSession,
    QuoteCandidate, SESSION_FLOW_TRACKING_ISSUE, SESSION_STORE_DIRECTORY, SESSION_STORE_SCHEMA,
    SessionFlowAvailability, SessionPhase, StatusEntry, StatusLane, StatusSlot, fold_status_lanes,
    load_stored_records, rfq_quote_set, select_quote, session_flow_availability,
    swp_profile_support, throwaway_session_signer, wrap_for_transport,
};
#[cfg(not(target_arch = "wasm32"))]
pub use session_transport::{
    SESSION_NETWORK_SUBSCRIPTION_ID, SESSION_SUBSCRIPTION_ID, SessionInbox, SessionSocketEvent,
    run_session_socket,
};
pub use view_model::{
    AssetView, CustodyView, EvidenceRung, ExitPackageView, FeeBreakdownView,
    MARKET_FIXTURE_CORPUS_SCHEMA, MARKET_SESSION_VIEW_SCHEMA, MKT_SWP_ERROR_CODES,
    MarketSessionViewModel, OfferingSideView, OfferingView, PriceFeedView, ProviderAssertionView,
    ProviderView, QuoteView, ReceiptView, ReservationProofClass, ReservationView, TimelineLaneView,
    TimelineSlotState, TimelineSlotView, TypedErrorView, VerifyChecklistView, VerifyRowView,
    VerifyState, asset_view, reservation_proof_class, typed_error_message,
};

#[cfg(not(target_arch = "wasm32"))]
actions!(
    market,
    [
        /// Toggles focus on the markets panel.
        ToggleFocus,
        /// Reconnects the markets panel to its configured relay.
        Reconnect
    ]
);

#[cfg(not(target_arch = "wasm32"))]
pub const MARKET_PANEL_ENVIRONMENT_VARIABLE: &str = "OMEGA_MARKET_PANEL";

/// Explicit development gate (PRODUCT.md): the Markets panel stays absent
/// from normal builds. The negotiated session flow is implemented, but the
/// panel targets the local Immortal dev relay and its no-spend provider, so
/// it remains a development surface.
#[cfg(not(target_arch = "wasm32"))]
pub fn market_panel_enabled() -> bool {
    std::env::var(MARKET_PANEL_ENVIRONMENT_VARIABLE).as_deref() == Ok("1")
}

#[cfg(not(target_arch = "wasm32"))]
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
