//! The negotiated session-flow seam for the Markets panel (omega#244).
//!
//! Discovery and the NIP-11 gate are implemented in this crate. The
//! requester session flow — RFQ → Quote (indicative/firm, reservation
//! class) → Order → per-signer Status timeline with visible sequence gaps
//! and forks → Cancel/Close, carried as NIP-59 gift-wrapped signed inner
//! records with local persistence for recovery/replay — is not implemented
//! yet and stays behind this seam.
//!
//! When it lands, it must drive
//! `immortal_client::mkt_swp_client::SwapRecordFactory` for deterministic
//! signing requests and reuse the `immortal_client::domain` and
//! `immortal_client::market` validators for every record and wrap. Omega
//! owns only transport, persistence, keys, and user-facing policy — never a
//! parallel implementation of event, signature, or MKT validation.

pub const SESSION_FLOW_TRACKING_ISSUE: &str = "OpenAgentsInc/omega#244";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFlowAvailability {
    NotImplemented { tracking_issue: &'static str },
}

/// Capability-derived navigation (PRODUCT.md): while the session flow is
/// unimplemented, the panel renders no session controls at all.
pub fn session_flow_availability() -> SessionFlowAvailability {
    SessionFlowAvailability::NotImplemented {
        tracking_issue: SESSION_FLOW_TRACKING_ISSUE,
    }
}
