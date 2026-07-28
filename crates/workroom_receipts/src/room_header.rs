//! Room header authority strip (not the conversation header).
//!
//! Spec §9.6: show the Sarah authority profile reference and revision in the
//! room header **area**. The conversation header says only `Sarah`.

use serde::{Deserialize, Serialize};

use crate::public_ref::{PublicRef, sanitize_public_ref};

/// Default profile ref from `@openagentsinc/sarah` / SARAH_AUTHORITY.md.
///
/// The live room projection supplies the authoritative value; this constant is
/// a public-safe fallback label for offline/degraded presentation only.
pub const DEFAULT_AUTHORITY_PROFILE_REF: &str = "openagents.sarah-owner-orchestrator";

/// Authority profile strip for the room header area.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomAuthorityHeader {
    pub authority_profile_ref: PublicRef,
    pub authority_revision: u32,
}

impl RoomAuthorityHeader {
    pub fn new(authority_profile_ref: PublicRef, authority_revision: u32) -> Self {
        Self {
            authority_profile_ref,
            authority_revision,
        }
    }

    /// Project from raw room bootstrap fields.
    pub fn from_raw(profile_ref: &str, revision: u32) -> Option<Self> {
        if revision == 0 {
            return None;
        }
        let authority_profile_ref = sanitize_public_ref(profile_ref)?;
        Some(Self {
            authority_profile_ref,
            authority_revision: revision,
        })
    }

    /// Public-safe fallback used when the room projection is degraded.
    pub fn default_admitted() -> Self {
        Self {
            authority_profile_ref: PublicRef::new(DEFAULT_AUTHORITY_PROFILE_REF)
                .expect("default profile ref is public-safe"),
            // Live package pin at time of OMEGA-SW-05; room projection overrides.
            authority_revision: 6,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_profile_ref() {
        assert!(RoomAuthorityHeader::from_raw("/Users/owner/profile", 6).is_none());
        assert!(RoomAuthorityHeader::from_raw("openagents.sarah-owner-orchestrator", 0).is_none());
    }

    #[test]
    fn accepts_live_profile() {
        let header =
            RoomAuthorityHeader::from_raw("openagents.sarah-owner-orchestrator", 6).expect("valid");
        assert_eq!(
            header.authority_profile_ref.as_str(),
            "openagents.sarah-owner-orchestrator"
        );
        assert_eq!(header.authority_revision, 6);
    }
}
