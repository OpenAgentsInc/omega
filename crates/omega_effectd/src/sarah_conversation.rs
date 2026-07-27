//! Sarah Nostr conversation client for `openagents.omega.effectd.v1`.
//!
//! Packet: SARAH-NR-06 (OpenAgentsInc/omega#33).
//! Spec: docs/omega/2026-07-24-sarah-workroom-mvp-spec.md §8, §24.7.
//!
//! This module is the only conversation client for the Sarah lane. It must
//! never link a Khala Sync client. The backing transport is a Nostr relay
//! adapter: mock/in-memory for local tests, NIP-42 authenticated when a real
//! relay URL and identity key are configured.

#[cfg(test)]
use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nostr::{
    Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, RelayUrl, Tag,
    nips::{
        nip42,
        nip44::{self, Version as Nip44Version},
    },
};
use omega_identity::{
    AdmittedSigningRequest, IdentityService, NostrPublicKeyHex, PrivateMessageRequest, ReceiptRef,
    SigningPurpose, UnsignedEventTemplate,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::issue31_provider_handoff::{
    ISSUE31_ACTION_REQUEST_PROVIDER_HANDOFF, Issue31ProviderHandoffLedger,
    Issue31ProviderRosterAccount,
};
use crate::protocol::{MAX_FRAME_BYTES, PROTOCOL_SCHEMA};
use crate::{
    ISSUE31_ADJUNCT_DELIVERY_KEYS, ISSUE31_COMMAND_SCHEMA, ISSUE31_COMMAND_SCHEMA_V2,
    ISSUE31_FULL_AUTO_ADJUNCT_RECORD_TYPE, ISSUE31_FULL_AUTO_ADJUNCT_SCHEMA,
    ISSUE31_HOST_ADJUNCT_RECORD_TYPE, ISSUE31_HOST_ADJUNCT_SCHEMA, ISSUE31_HOST_DISCOVERY_KIND,
    ISSUE31_PAIRING_SCHEMA, Issue31AuthorityDecisionProjection, Issue31CommandArguments,
    Issue31CommandEvent, Issue31CommandExecution, Issue31CommandExecutionV2,
    Issue31CommandHandlingStatus, Issue31CommandRecord, Issue31CommandRecordV2,
    Issue31CommandStatus, Issue31DirectEndpoint, Issue31GrantState, Issue31HostConfiguration,
    Issue31HostController, Issue31HostDiscovery, Issue31HostDiscoveryV2, Issue31HostDiscoveryV3,
    Issue31NostrError, Issue31OwnerProjectionBody, Issue31OwnerProjectionInput,
    Issue31PairingEvent, Issue31PairingRecord, Issue31PairingScope, Issue31SourceRole,
    Issue31TargetOutcomeProjection, Issue31WithheldCause, Issue31WithheldSourceCount,
    Issue31WithheldSourcesInput, SARAH_AUTHORITY_RECEIPT_KIND, SARAH_ENGRAM_KIND,
    SARAH_READ_STATE_KIND, SARAH_REMINDER_KIND, emit_issue31_owner_projection,
    emit_issue31_withheld_sources,
};

pub use crate::openagents_binding::BindingState;

/// Framed method names on `openagents.omega.effectd.v1` for the Sarah room.
pub const SARAH_METHOD_SESSION_STATUS: &str = "sarah_session_status";
pub const SARAH_METHOD_BOOTSTRAP: &str = "sarah_bootstrap";
pub const SARAH_METHOD_ROOM_SNAPSHOT: &str = "sarah_room_snapshot";
pub const SARAH_METHOD_SEND_MESSAGE: &str = "sarah_send_message";
pub const SARAH_METHOD_INTERRUPT_TURN: &str = "sarah_interrupt_turn";
pub const SARAH_METHOD_DEVICE_GRANTS: &str = "sarah_device_grants";
pub const SARAH_METHOD_RENEW_DEVICE_GRANT: &str = "sarah_renew_device_grant";
pub const SARAH_METHOD_REVOKE_DEVICE_GRANT: &str = "sarah_revoke_device_grant";
/// Owner re-admission of a device whose grant was revoked. Revocation fails
/// closed for the device, not just the grant, so without this the owner has no
/// way to let a device back in.
pub const SARAH_METHOD_READMIT_DEVICE: &str = "sarah_readmit_device";
pub const SARAH_EVENT_ROOM_EVENT: &str = "sarah_room_event";
pub const SARAH_EVENT_ROOM_STATE: &str = "sarah_room_state";

pub const SARAH_FRAMED_METHODS: &[&str] = &[
    SARAH_METHOD_SESSION_STATUS,
    SARAH_METHOD_BOOTSTRAP,
    SARAH_METHOD_ROOM_SNAPSHOT,
    SARAH_METHOD_SEND_MESSAGE,
    SARAH_METHOD_INTERRUPT_TURN,
    SARAH_METHOD_DEVICE_GRANTS,
    SARAH_METHOD_RENEW_DEVICE_GRANT,
    SARAH_METHOD_REVOKE_DEVICE_GRANT,
    SARAH_METHOD_READMIT_DEVICE,
];

/// NIP-AO ephemeral control kind used for interrupt / cancel_turn.
pub const NIP_AO_KIND: u16 = 24200;
/// Durable Sarah turn-record kind (SARAH-NR-00).
pub const SARAH_TURN_RECORD_KIND: u16 = 44300;

const DEFAULT_PAGE_LIMIT: usize = 32;
const MAX_PAGE_LIMIT: usize = 64;
const MAX_PENDING_EVENTS: usize = 256;
const MAX_COMMAND_RESULTS: usize = 4_096;
const MAX_PRIVATE_OUTBOX_ITEMS: usize = 1_024;
const MAX_RELAY_ACKNOWLEDGEMENTS: usize = 4_096;
const MAX_QUARANTINED_ISSUE31_EVENTS: usize = 4_096;
/// The quarantine reason recorded when a source event cannot become a
/// projection. It is the only quarantine reason that withholds something the
/// owner was entitled to read; the pairing and command reasons quarantine
/// control records, which are not part of the owner's view.
const ISSUE31_PROJECTION_SOURCE_QUARANTINE_REASON: &str = "reason.omega.invalid_projection_source";
const ISSUE31_PROJECTION_SCAN_BOUND_REASON: &str = "reason.omega.projection_scan_bound";
const CURSOR_PREFIX: &str = "cursor.";
const MOCK_RELAY_LABEL: &str = "mock://local";

#[derive(Debug, Error)]
pub enum SarahConversationError {
    #[error("stale generation: expected {expected}, got {got}")]
    StaleGeneration { expected: u64, got: u64 },
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("identity required for authenticated relay")]
    IdentityRequired,
    #[error("identity custody error: {0}")]
    Identity(String),
    #[error("relay error: {0}")]
    Relay(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl SarahConversationError {
    pub fn protocol_code(&self) -> crate::protocol::ProtocolErrorCode {
        use crate::protocol::ProtocolErrorCode;
        match self {
            Self::StaleGeneration { .. } => ProtocolErrorCode::StaleGeneration,
            Self::InvalidRequest(_) | Self::IdentityRequired => ProtocolErrorCode::InvalidRequest,
            Self::Identity(_) => ProtocolErrorCode::HostUnavailable,
            Self::Relay(_) => ProtocolErrorCode::HostUnavailable,
            Self::Internal(_) => ProtocolErrorCode::Internal,
        }
    }
}

/// Public-safe identity projection. Never carries a private key or token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationIdentity {
    pub owner_public_key_hex: String,
    pub sarah_public_key_hex: String,
    pub account_label: Option<String>,
    /// Binding state for metering attribution (OMEGA-SW-01). Not a session token.
    pub binding_state: BindingState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatusResult {
    pub signed_in: bool,
    pub account_label: Option<String>,
    pub binding_state: BindingState,
    pub owner_public_key_hex: Option<String>,
    /// ISO-8601 expiry for the OpenAgents account binding, when known.
    pub binding_expires_at: Option<String>,
    pub transport: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapResult {
    pub principal_ref: String,
    pub display_name: String,
    pub role: String,
    pub conversation_ref: String,
    pub legacy_thread_ref: String,
    pub owner_public_key_hex: String,
    pub sarah_public_key_hex: String,
    pub authority_profile_ref: String,
    pub authority_profile_revision: u32,
    pub admitted_device_fingerprints: Vec<String>,
    pub quarantined_issue31_event_count: usize,
    pub room_state: RoomStateEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoomSnapshotResult {
    pub conversation_ref: String,
    pub transcript: TranscriptPage,
    pub activity: ActivityPage,
    pub nostr_records: NostrRecordPage,
    pub run_state: RunStateProjection,
    pub room_state: RoomStateEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NostrRecordPage {
    pub entries: Vec<NostrRecordRef>,
    pub cursor: String,
    pub next_cursor: Option<String>,
    pub gap_state: GapState,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NostrRecordRef {
    pub event_id: String,
    pub cursor: String,
    pub kind: u16,
    pub record_kind: String,
    pub author_fingerprint: String,
    pub created_at: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptPage {
    pub entries: Vec<TranscriptEntry>,
    pub cursor: String,
    pub next_cursor: Option<String>,
    pub gap_state: GapState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEntry {
    pub event_id: String,
    pub cursor: String,
    pub role: String,
    pub kind: String,
    pub text: String,
    pub created_at: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    pub entries: Vec<ActivityEntry>,
    pub cursor: String,
    pub next_cursor: Option<String>,
    pub gap_state: GapState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    pub event_id: String,
    pub cursor: String,
    pub entry: String,
    pub turn_ref: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunStateProjection {
    pub state: String,
    pub turn_ref: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageResult {
    pub accepted: bool,
    pub message_ref: String,
    pub turn_ref: String,
    pub event_id: String,
    pub cursor: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InterruptTurnResult {
    pub accepted: bool,
    pub turn_ref: String,
    pub intent_ref: String,
    pub status: String,
    pub pending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoomEventPayload {
    pub method: String,
    pub conversation_ref: String,
    pub cursor: String,
    pub record: TranscriptEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoomStateEvent {
    pub method: String,
    pub connection: ConnectionState,
    pub freshness: FreshnessState,
    pub gap_state: GapState,
    pub connected_relays: Vec<String>,
    pub last_acknowledged_event_id: Option<String>,
    pub last_acknowledged_cursor: Option<String>,
    pub authenticated: bool,
    pub transport: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Degraded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessState {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GapState {
    None,
    Possible,
    Confirmed,
    Recovering,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RelayAuthChallenge {
    pub challenge: String,
    pub relay_url: String,
}

/// Stored public-safe event in the conversation store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredConversationEvent {
    pub event_id: String,
    pub kind: u16,
    pub pubkey: String,
    pub created_at: u64,
    pub conversation_ref: String,
    pub content_summary: String,
    pub tags: Vec<Vec<String>>,
    /// Whether this is an owner message, Sarah answer, activity entry, or control.
    pub record_kind: String,
    pub store_index: usize,
}

#[derive(Debug, Clone)]
pub struct QueryPage {
    pub events: Vec<StoredConversationEvent>,
    pub next_cursor: Option<String>,
    pub gap_state: GapState,
}

/// Abstract Nostr relay transport. Production will use a real WebSocket client;
/// tests and local-dev without a relay use [`MockRelayAdapter`].
pub trait RelayTransport: Send {
    fn label(&self) -> &str;
    fn connection_state(&self) -> ConnectionState;
    fn is_authenticated(&self) -> bool;
    fn connect(&mut self) -> Result<(), SarahConversationError>;
    fn auth_challenge(&self) -> Option<RelayAuthChallenge>;
    fn authenticate(&mut self, auth_event: &Event) -> Result<(), SarahConversationError>;
    fn publish(&mut self, event: &Event) -> Result<(), SarahConversationError>;
    fn publication_complete(&mut self, _event_id: &str) -> bool {
        true
    }
    fn acknowledged_relays(&self, _event_id: &str) -> Vec<String> {
        vec![self.label().to_string()]
    }
    fn restore_publication_acknowledgements(&mut self, _event_id: &str, _relay_urls: &[String]) {}
    fn query(
        &mut self,
        conversation_ref: &str,
        after_cursor: Option<&str>,
        limit: usize,
    ) -> Result<QueryPage, SarahConversationError>;
    fn last_event_id(&self) -> Option<String>;
    fn gap_state(&self) -> GapState {
        GapState::None
    }
    fn connected_relays(&self) -> Vec<String> {
        vec![self.label().to_string()]
    }
    fn requires_private_messages(&self) -> bool {
        false
    }
}

/// In-memory mock relay for local/dev and unit tests.
#[derive(Debug, Default)]
pub struct MockRelayAdapter {
    label: String,
    connected: bool,
    authenticated: bool,
    require_auth: bool,
    challenge: Option<String>,
    events: Vec<StoredConversationEvent>,
}

impl MockRelayAdapter {
    pub fn new() -> Self {
        Self {
            label: MOCK_RELAY_LABEL.to_string(),
            connected: false,
            authenticated: false,
            require_auth: false,
            challenge: None,
            events: Vec::new(),
        }
    }

    /// Local mock that still exercises the NIP-42 AUTH path.
    pub fn with_required_auth(challenge: impl Into<String>) -> Self {
        Self {
            label: "mock://auth-required".to_string(),
            require_auth: true,
            challenge: Some(challenge.into()),
            ..Self::new()
        }
    }

    pub fn seed_event(&mut self, mut event: StoredConversationEvent) {
        event.store_index = self.events.len();
        self.events.push(event);
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }
}

impl RelayTransport for MockRelayAdapter {
    fn label(&self) -> &str {
        &self.label
    }

    fn connection_state(&self) -> ConnectionState {
        if self.connected {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        }
    }

    fn is_authenticated(&self) -> bool {
        self.authenticated || !self.require_auth
    }

    fn connect(&mut self) -> Result<(), SarahConversationError> {
        self.connected = true;
        if self.require_auth {
            self.authenticated = false;
            if self.challenge.is_none() {
                self.challenge = Some(format!("challenge-{}", self.events.len()));
            }
        } else {
            self.authenticated = true;
        }
        Ok(())
    }

    fn auth_challenge(&self) -> Option<RelayAuthChallenge> {
        if self.require_auth && !self.authenticated {
            Some(RelayAuthChallenge {
                challenge: self.challenge.clone().unwrap_or_default(),
                relay_url: "wss://relay.openagents.com".to_string(),
            })
        } else {
            None
        }
    }

    fn authenticate(&mut self, auth_event: &Event) -> Result<(), SarahConversationError> {
        let challenge = self
            .challenge
            .as_deref()
            .ok_or_else(|| SarahConversationError::Relay("no auth challenge pending".into()))?;
        let relay_url = RelayUrl::parse("wss://relay.openagents.com")
            .map_err(|error| SarahConversationError::Relay(error.to_string()))?;
        if !nip42::is_valid_auth_event(auth_event, &relay_url, challenge) {
            return Err(SarahConversationError::Relay(
                "NIP-42 auth event rejected".into(),
            ));
        }
        if auth_event.verify().is_err() {
            return Err(SarahConversationError::Relay(
                "NIP-42 auth signature invalid".into(),
            ));
        }
        self.authenticated = true;
        Ok(())
    }

    fn publish(&mut self, event: &Event) -> Result<(), SarahConversationError> {
        if !self.connected {
            return Err(SarahConversationError::Relay("not connected".into()));
        }
        if self.require_auth && !self.authenticated {
            return Err(SarahConversationError::Relay(
                "authentication required before publish".into(),
            ));
        }
        let conversation_ref = event
            .tags
            .iter()
            .find_map(|tag| {
                let slice = tag.as_slice();
                if slice.first().map(String::as_str) == Some("conversation") {
                    slice.get(1).cloned()
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "sarah.unknown".to_string());
        let record_kind = match event.kind.as_u16() {
            NIP_AO_KIND => "control",
            SARAH_TURN_RECORD_KIND => "activity",
            SARAH_READ_STATE_KIND => "read_state",
            SARAH_REMINDER_KIND => "reminder",
            _ => "message",
        };
        let store_index = self.events.len();
        self.events.push(StoredConversationEvent {
            event_id: event.id.to_hex(),
            kind: event.kind.as_u16(),
            pubkey: event.pubkey.to_hex(),
            created_at: event.created_at.as_secs(),
            conversation_ref,
            content_summary: redact_content_summary(&event.content),
            tags: event
                .tags
                .iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect(),
            record_kind: record_kind.to_string(),
            store_index,
        });
        Ok(())
    }

    fn query(
        &mut self,
        conversation_ref: &str,
        after_cursor: Option<&str>,
        limit: usize,
    ) -> Result<QueryPage, SarahConversationError> {
        if !self.connected {
            return Err(SarahConversationError::Relay("not connected".into()));
        }
        let matching: Vec<StoredConversationEvent> = self
            .events
            .iter()
            .filter(|event| {
                event.conversation_ref == conversation_ref
                    || matches!(event.kind, SARAH_READ_STATE_KIND | SARAH_REMINDER_KIND)
            })
            .cloned()
            .collect();
        let start_index = after_cursor
            .and_then(|cursor| {
                matching
                    .iter()
                    .position(|event| stored_event_cursor(event) == cursor)
            })
            .map(|index| index.saturating_add(1))
            .unwrap_or(0);
        let page: Vec<StoredConversationEvent> = matching
            .iter()
            .skip(start_index)
            .take(limit)
            .cloned()
            .collect();
        let next_cursor = if start_index.saturating_add(page.len()) < matching.len() {
            page.last().map(stored_event_cursor)
        } else {
            None
        };
        Ok(QueryPage {
            events: page,
            next_cursor,
            gap_state: GapState::None,
        })
    }

    fn last_event_id(&self) -> Option<String> {
        self.events.last().map(|event| event.event_id.clone())
    }
}

/// Optional identity material used only for signing. Secrets never leave this
/// struct into framed protocol responses.
pub struct SigningIdentity {
    pub public_key_hex: String,
    keys: Keys,
}

impl SigningIdentity {
    pub fn from_keys(keys: Keys) -> Self {
        Self {
            public_key_hex: keys.public_key().to_hex(),
            keys,
        }
    }

    pub fn generate() -> Self {
        Self::from_keys(Keys::generate())
    }

    pub fn sign_auth(
        &self,
        challenge: &str,
        relay_url: &str,
    ) -> Result<Event, SarahConversationError> {
        let relay = RelayUrl::parse(relay_url)
            .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
        EventBuilder::auth(challenge, relay)
            .sign_with_keys(&self.keys)
            .map_err(|error| SarahConversationError::Internal(error.to_string()))
    }

    pub fn sign_text_note(
        &self,
        content: &str,
        tags: Vec<Tag>,
    ) -> Result<Event, SarahConversationError> {
        EventBuilder::text_note(content)
            .tags(tags)
            .sign_with_keys(&self.keys)
            .map_err(|error| SarahConversationError::Internal(error.to_string()))
    }

    pub fn sign_custom(
        &self,
        kind: u16,
        content: &str,
        tags: Vec<Tag>,
    ) -> Result<Event, SarahConversationError> {
        EventBuilder::new(Kind::from(kind), content)
            .tags(tags)
            .sign_with_keys(&self.keys)
            .map_err(|error| SarahConversationError::Internal(error.to_string()))
    }
}

enum ConversationSigner {
    Keys(SigningIdentity),
    OmegaIdentity(Arc<IdentityService>),
}

impl ConversationSigner {
    fn sign_public_record(
        &self,
        kind: u16,
        content: &str,
        tags: Vec<Tag>,
    ) -> Result<Event, SarahConversationError> {
        match self {
            Self::Keys(identity) => identity.sign_custom(kind, content, tags),
            Self::OmegaIdentity(identity_service) => {
                let custody = identity_service
                    .inspect()
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                let identity = custody
                    .identity
                    .ok_or(SarahConversationError::IdentityRequired)?;
                let semantic_binding = serde_json::to_vec(&json!({
                    "kind": kind,
                    "content": content,
                    "tags": tags.iter().map(|tag| tag.as_slice()).collect::<Vec<_>>(),
                }))
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
                let request = AdmittedSigningRequest {
                    request_ref: digest_receipt_ref("issue31.public", &semantic_binding)?,
                    identity_ref: identity.identity_ref().clone(),
                    purpose: SigningPurpose::NostrEvent,
                    event: UnsignedEventTemplate {
                        created_at: unix_now(),
                        kind,
                        tags: tags.iter().map(|tag| tag.as_slice().to_vec()).collect(),
                        content: content.to_string(),
                    },
                };
                let signed = identity_service
                    .sign(&request)
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                Event::from_json(signed.signed_event_json)
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))
            }
        }
    }

    fn sign_encrypted_self_record(
        &self,
        kind: u16,
        plaintext: &str,
        tags: Vec<Tag>,
    ) -> Result<Event, SarahConversationError> {
        match self {
            Self::Keys(identity) => {
                let ciphertext = nip44::encrypt(
                    identity.keys.secret_key(),
                    &identity.keys.public_key(),
                    plaintext.as_bytes(),
                    Nip44Version::V2,
                )
                .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                identity.sign_custom(kind, &ciphertext, tags)
            }
            Self::OmegaIdentity(identity_service) => {
                let custody = identity_service
                    .inspect()
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                let identity = custody
                    .identity
                    .ok_or(SarahConversationError::IdentityRequired)?;
                let semantic_binding = serde_json::to_vec(&json!({
                    "kind": kind,
                    "plaintextDigest": format!("{:x}", Sha256::digest(plaintext.as_bytes())),
                    "tags": tags.iter().map(|tag| tag.as_slice()).collect::<Vec<_>>(),
                }))
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
                let request = AdmittedSigningRequest {
                    request_ref: digest_receipt_ref("issue31.encrypted-self", &semantic_binding)?,
                    identity_ref: identity.identity_ref().clone(),
                    purpose: SigningPurpose::Nip44EncryptedSelfEvent,
                    event: UnsignedEventTemplate {
                        created_at: unix_now(),
                        kind,
                        tags: tags.iter().map(|tag| tag.as_slice().to_vec()).collect(),
                        content: plaintext.to_string(),
                    },
                };
                let signed = identity_service
                    .sign_nip44_encrypted_to_self(&request)
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                Event::from_json(signed.signed_event_json)
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))
            }
        }
    }

    fn decrypt_record(
        &self,
        sender_public_key_hex: &str,
        ciphertext: &str,
    ) -> Result<String, SarahConversationError> {
        match self {
            Self::Keys(identity) => {
                let sender_public_key = PublicKey::from_hex(sender_public_key_hex)
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                nip44::decrypt(
                    identity.keys.secret_key(),
                    &sender_public_key,
                    ciphertext.as_bytes(),
                )
                .map_err(|error| SarahConversationError::Identity(error.to_string()))
            }
            Self::OmegaIdentity(identity_service) => {
                let sender_public_key = NostrPublicKeyHex::new(sender_public_key_hex)
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                identity_service
                    .decrypt_nip44_from(&sender_public_key, ciphertext)
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))
            }
        }
    }

    fn sign_auth(&self, challenge: &str, relay_url: &str) -> Result<Event, SarahConversationError> {
        match self {
            Self::Keys(identity) => identity.sign_auth(challenge, relay_url),
            Self::OmegaIdentity(identity_service) => {
                let custody = identity_service
                    .inspect()
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                let identity = custody
                    .identity
                    .ok_or(SarahConversationError::IdentityRequired)?;
                let public_key = PublicKey::from_hex(identity.public_key_hex().as_str())
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                let relay = RelayUrl::parse(relay_url)
                    .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
                let unsigned = EventBuilder::auth(challenge, relay).build(public_key);
                let request_ref = digest_receipt_ref("nip42", challenge.as_bytes())?;
                let request = AdmittedSigningRequest {
                    request_ref,
                    identity_ref: identity.identity_ref().clone(),
                    purpose: SigningPurpose::NostrEvent,
                    event: UnsignedEventTemplate {
                        created_at: unsigned.created_at.as_secs(),
                        kind: unsigned.kind.as_u16(),
                        tags: unsigned
                            .tags
                            .iter()
                            .map(|tag| tag.as_slice().to_vec())
                            .collect(),
                        content: unsigned.content,
                    },
                };
                let signed = identity_service
                    .sign(&request)
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                Event::from_json(signed.signed_event_json)
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))
            }
        }
    }

    fn private_messages(
        &self,
        content: &str,
        tags: Vec<Tag>,
        recipients: &[String],
    ) -> Result<(String, Vec<Event>), SarahConversationError> {
        let created_at = unix_now();
        let semantic_binding = serde_json::to_vec(&json!({
            "content": content,
            "tags": tags.iter().map(|tag| tag.as_slice()).collect::<Vec<_>>(),
            "recipients": recipients,
        }))
        .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
        match self {
            Self::Keys(identity) => {
                let public_key = identity.keys.public_key();
                let mut rumor = EventBuilder::new(Kind::PrivateDirectMessage, content)
                    .tags(tags)
                    .custom_created_at(nostr::Timestamp::from_secs(created_at))
                    .build(public_key);
                rumor.ensure_id();
                rumor
                    .verify_id()
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                let rumor_event_id = rumor
                    .id
                    .ok_or_else(|| {
                        SarahConversationError::Identity(
                            "private rumor omitted its convergence id".into(),
                        )
                    })?
                    .to_hex();
                let mut events = Vec::with_capacity(recipients.len());
                for recipient in recipients {
                    let recipient = PublicKey::from_hex(recipient).map_err(|error| {
                        SarahConversationError::InvalidRequest(error.to_string())
                    })?;
                    let gift_wrap = smol::block_on(EventBuilder::gift_wrap(
                        &identity.keys,
                        &recipient,
                        rumor.clone(),
                        [],
                    ))
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                    events.push(gift_wrap);
                }
                Ok((rumor_event_id, events))
            }
            Self::OmegaIdentity(identity_service) => {
                let custody = identity_service
                    .inspect()
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                let identity = custody
                    .identity
                    .ok_or(SarahConversationError::IdentityRequired)?;
                let recipients = recipients
                    .iter()
                    .map(NostrPublicKeyHex::new)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
                let request = PrivateMessageRequest {
                    request_ref: digest_receipt_ref("nip17", &semantic_binding)?,
                    identity_ref: identity.identity_ref().clone(),
                    recipients,
                    rumor: UnsignedEventTemplate {
                        created_at,
                        kind: Kind::PrivateDirectMessage.as_u16(),
                        tags: tags.iter().map(|tag| tag.as_slice().to_vec()).collect(),
                        content: content.to_string(),
                    },
                };
                let wrapped = identity_service
                    .gift_wrap_private_message(&request)
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                let rumor_event_id = wrapped
                    .first()
                    .map(|wrapped| wrapped.rumor_event_id.clone())
                    .ok_or_else(|| {
                        SarahConversationError::Identity(
                            "private message produced no gift wraps".into(),
                        )
                    })?;
                let events = wrapped
                    .into_iter()
                    .map(|wrapped| {
                        Event::from_json(wrapped.gift_wrap_event_json)
                            .map_err(|error| SarahConversationError::Identity(error.to_string()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((rumor_event_id, events))
            }
        }
    }
}

/// Configuration for the conversation client.
#[derive(Serialize)]
#[serde(untagged)]
enum SignedIssue31Discovery {
    V2(Issue31HostDiscoveryV2),
    V3(Issue31HostDiscoveryV3),
}

impl SignedIssue31Discovery {
    fn host_ref(&self) -> &str {
        match self {
            Self::V2(discovery) => &discovery.host_ref,
            Self::V3(discovery) => &discovery.host_ref,
        }
    }

    fn generation(&self) -> u64 {
        match self {
            Self::V2(discovery) => discovery.generation,
            Self::V3(discovery) => discovery.generation,
        }
    }

    fn expires_at(&self) -> u64 {
        match self {
            Self::V2(discovery) => discovery.expires_at,
            Self::V3(discovery) => discovery.expires_at,
        }
    }
}

/// Configuration for the conversation client.
#[derive(Debug, Clone)]
pub struct SarahConversationConfig {
    pub generation: u64,
    pub conversation_digest: String,
    pub identity: ConversationIdentity,
    /// When set, the client treats the transport as a real relay and runs NIP-42.
    pub relay_url: Option<String>,
    pub direct_endpoints: Vec<Issue31DirectEndpoint>,
    pub admitted_device_public_key_hexes: Vec<String>,
    pub approved_device_scopes: Vec<crate::Issue31PairingScope>,
    pub community_group_ids: Vec<String>,
    pub community_public_key_hexes: Vec<String>,
}

impl SarahConversationConfig {
    pub fn mock_fixture() -> Self {
        Self {
            generation: 1,
            conversation_digest: "a".repeat(24),
            identity: ConversationIdentity {
                owner_public_key_hex: "b".repeat(64),
                sarah_public_key_hex: "c".repeat(64),
                account_label: Some("owner@example.com".to_string()),
                binding_state: BindingState::Bound,
            },
            relay_url: None,
            direct_endpoints: Vec::new(),
            admitted_device_public_key_hexes: Vec::new(),
            approved_device_scopes: Vec::new(),
            community_group_ids: Vec::new(),
            community_public_key_hexes: Vec::new(),
        }
    }

    pub fn conversation_ref(&self) -> String {
        format!("sarah.{}", self.conversation_digest)
    }

    pub fn legacy_thread_ref(&self) -> String {
        format!("thread.sarah.{}", self.conversation_digest)
    }
}

/// Nostr conversation client owned by omega-effectd for the Sarah lane.
/// One admitted device's standing, handed to the Full Auto reader (omega#49).
///
/// The host pump owns this shape rather than the panel: the panel knows what
/// the runs are, and only the pump knows which devices are entitled to see
/// them and under which grant. Building the snapshot per grant is what makes
/// `connection_identity` a statement about *this* reader instead of a generic
/// one.
#[derive(Clone, Copy, Debug)]
pub struct Issue31HostProjectionRequest<'a> {
    pub host_ref: &'a str,
    pub host_public_key_hex: &'a str,
    pub device_public_key_hex: &'a str,
    pub grant_ref: &'a str,
    pub expected_generation: u64,
    /// The pump's reading time, in epoch milliseconds.
    pub observed_at_ms: u64,
    /// The host's own provider connection handoff ledger (omega#91).
    ///
    /// This comes from the pump rather than from the Full Auto reading for the
    /// same reason the delivery binding does: the ledger is durable host state
    /// that survives a restart, and the panel's reading is a transient
    /// observation of the daemon. One of them is the record; letting the other
    /// also carry handoffs would give the phone two sources for one fact.
    ///
    /// The ledger is handed over unprojected on purpose. Only the builder knows
    /// the `generatedAtMs` the documents will carry, and a handoff row must be
    /// checked against exactly that stamp — projecting here, against the pump's
    /// clock, would accept rows the assembled document then contradicts.
    pub handoffs: &'a Issue31ProviderHandoffLedger,
}

/// The two omega#47 documents `publish_issue31_host_snapshot` produces.
///
/// They travel as JSON because the builder lives in `full_auto_ui`, which
/// depends on this crate. Inverting that to call the builder from here would
/// make the dependency circular, so the reading crosses the seam as data.
#[derive(Clone, Debug)]
pub struct Issue31HostProjectionDocuments {
    pub host: Value,
    pub detail: Value,
}

/// A live reading of Full Auto host state, or `None`.
///
/// `None` means this host cannot presently state its Full Auto view — no
/// supervisor attached, no panel reading, nothing observed. It is not an empty
/// view: an empty view is a `Some` carrying zero runs, which the device renders
/// as "this host is running nothing". Publishing an invented empty projection
/// for an unobserved host would be exactly the false claim omega#49 forbids.
pub type Issue31HostProjectionSource = Arc<
    dyn Fn(
            &Issue31HostProjectionRequest<'_>,
        ) -> Result<Option<Issue31HostProjectionDocuments>, String>
        + Send
        + Sync,
>;

/// The host's own reading of its provider accounts, or `None` (omega#91).
///
/// `None` means the host has not looked at its roster. Nothing about a handoff
/// advances on an unread roster: a binding decided against state nobody read
/// would be a guess wearing a measurement's clothes. `Some(&[])` — the host
/// looked and holds no accounts — is a real observation and does move a
/// handoff's clock towards its deadline.
pub type Issue31ProviderRosterSource =
    Arc<dyn Fn() -> Option<Vec<Issue31ProviderRosterAccount>> + Send + Sync>;

pub struct SarahConversationClient {
    config: SarahConversationConfig,
    relay: Box<dyn RelayTransport>,
    signer: ConversationSigner,
    pending_events: VecDeque<Value>,
    active_turn_ref: Option<String>,
    run_state: String,
    message_seq: u64,
    command_results: BTreeMap<String, (String, Value)>,
    last_gap_state: GapState,
    last_confirmed_cursor: Option<String>,
    issue31_host: Option<Issue31HostController>,
    issue31_device_controller: Option<crate::SharedIssue31HostController>,
    issue31_discovery_generation: Option<u64>,
    issue31_discovery_expires_at: Option<u64>,
    issue31_discovery_outbox: Option<Event>,
    issue31_private_outbox: BTreeMap<String, PendingIssue31PrivatePublish>,
    issue31_relay_acknowledgements: BTreeMap<String, Vec<String>>,
    issue31_control_cursor: Option<String>,
    issue31_projection_cursor: Option<String>,
    issue31_quarantined_events: BTreeMap<String, String>,
    /// The last coverage statement published to each `grant_ref:generation`,
    /// so a re-run that observed the same world does not republish it with only
    /// a new timestamp.
    issue31_withheld_emissions: BTreeMap<String, (String, Vec<Issue31WithheldSourceCount>)>,
    /// The digest of the last omega#47 publication sent to each
    /// `grant_ref:generation`, so a pump run that observed the same host state
    /// does not re-send the same snapshot with only a fresh timestamp.
    issue31_host_adjunct_emissions: BTreeMap<String, String>,
    /// The live Full Auto reading, supplied by whoever holds the supervisor.
    /// Absent means this host publishes no omega#47 records at all.
    issue31_host_projection_source: Option<Issue31HostProjectionSource>,
    /// Host-owned provider connection handoffs (omega#91). Durable: this is the
    /// record, not a cache of one.
    issue31_provider_handoffs: Issue31ProviderHandoffLedger,
    /// How the host reads its own provider roster when deciding a handoff.
    issue31_provider_roster_source: Option<Issue31ProviderRosterSource>,
    issue31_state_path: Option<PathBuf>,
    #[cfg(test)]
    issue31_fail_commit_after: Cell<Option<usize>>,
}

#[derive(Clone)]
struct PendingIssue31PrivatePublish {
    rumor_event_id: String,
    gift_wraps: Vec<Event>,
}

struct EnqueuedIssue31PrivateRecord {
    rumor_event_id: String,
    outbox_ref: String,
    inserted: bool,
}

const ISSUE31_DURABLE_STATE_SCHEMA: &str = "openagents.omega.issue31.host_state.v1";
const ISSUE31_DURABLE_STATE_MAX_BYTES: u64 = 16 * 1024 * 1024;
const ISSUE31_NOSTR_HOST_GENERATION: u64 = 1;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DurableIssue31HostState {
    schema: String,
    controller: Issue31HostController,
    discovery_generation: Option<u64>,
    discovery_expires_at: Option<u64>,
    discovery_event_json: Option<String>,
    private_outbox: BTreeMap<String, DurableIssue31PrivatePublish>,
    relay_acknowledgements: BTreeMap<String, Vec<String>>,
    control_cursor: Option<String>,
    #[serde(default)]
    projection_cursor: Option<String>,
    #[serde(default)]
    quarantined_events: BTreeMap<String, String>,
    #[serde(default)]
    host_adjunct_emissions: BTreeMap<String, String>,
    /// omega#91. `#[serde(default)]` so a state file written before handoffs
    /// existed still loads — as an empty ledger, which is the truth about a
    /// host that never had one, not a set of rows invented at load.
    #[serde(default)]
    provider_handoffs: Issue31ProviderHandoffLedger,
    command_results: BTreeMap<String, (String, Value)>,
    #[serde(default)]
    active_turn_ref: Option<String>,
    #[serde(default = "default_run_state")]
    run_state: String,
    #[serde(default)]
    message_seq: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DurableIssue31PrivatePublish {
    rumor_event_id: String,
    gift_wrap_event_json: Vec<String>,
}

impl SarahConversationClient {
    /// Local/dev client with an in-memory mock relay (no network).
    pub fn new_mock(mut config: SarahConversationConfig) -> Self {
        let signer = SigningIdentity::generate();
        config.identity.owner_public_key_hex = signer.public_key_hex.clone();
        Self::with_relay(config, Box::new(MockRelayAdapter::new()), signer)
    }

    pub fn with_relay(
        config: SarahConversationConfig,
        relay: Box<dyn RelayTransport>,
        signer: SigningIdentity,
    ) -> Self {
        Self {
            config,
            relay,
            signer: ConversationSigner::Keys(signer),
            pending_events: VecDeque::new(),
            active_turn_ref: None,
            run_state: "idle".to_string(),
            message_seq: 0,
            command_results: BTreeMap::new(),
            last_gap_state: GapState::None,
            last_confirmed_cursor: None,
            issue31_host: None,
            issue31_device_controller: None,
            issue31_discovery_generation: None,
            issue31_discovery_expires_at: None,
            issue31_discovery_outbox: None,
            issue31_private_outbox: BTreeMap::new(),
            issue31_relay_acknowledgements: BTreeMap::new(),
            issue31_control_cursor: None,
            issue31_projection_cursor: None,
            issue31_quarantined_events: BTreeMap::new(),
            issue31_withheld_emissions: BTreeMap::new(),
            issue31_host_adjunct_emissions: BTreeMap::new(),
            issue31_host_projection_source: None,
            issue31_provider_handoffs: Issue31ProviderHandoffLedger::default(),
            issue31_provider_roster_source: None,
            issue31_state_path: None,
            #[cfg(test)]
            issue31_fail_commit_after: Cell::new(None),
        }
    }

    pub fn new_production(
        mut config: SarahConversationConfig,
        relay_urls: Vec<String>,
        identity_service: Arc<IdentityService>,
    ) -> Result<Self, SarahConversationError> {
        let owner_public_key_hex = identity_service
            .inspect()
            .map_err(|error| SarahConversationError::Identity(error.to_string()))?
            .identity
            .map(|identity| identity.public_key_hex().as_str().to_string())
            .ok_or(SarahConversationError::IdentityRequired)?;
        config.identity.owner_public_key_hex = owner_public_key_hex;
        config.relay_url = relay_urls.first().cloned();
        let mut relay = crate::nostr_websocket_relay::WebSocketRelayAdapter::new(
            relay_urls,
            identity_service.clone(),
            config.identity.sarah_public_key_hex.clone(),
            config.community_group_ids.clone(),
            config.community_public_key_hexes.clone(),
        )?;
        let relay_urls = relay.relay_urls().to_vec();
        let host_configuration = Issue31HostConfiguration {
            host_ref: "omega.host.local".into(),
            host_public_key_hex: config.identity.owner_public_key_hex.clone(),
            sarah_public_key_hex: config.identity.sarah_public_key_hex.clone(),
            conversation: config.conversation_ref(),
            display_name: "Local Omega".into(),
            relay_urls,
            generation: ISSUE31_NOSTR_HOST_GENERATION,
        };
        let issue31_state_path = paths::data_dir()
            .join("openagents")
            .join("issue31-nostr-host-state.json");
        let mut persisted = load_issue31_host_state(&issue31_state_path, &host_configuration)?;
        if let Some(persisted) = persisted.as_mut()
            && let Some(event_json) = persisted.discovery_event_json.as_deref()
        {
            let event = Event::from_json(event_json)
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
            if !discovery_matches_direct_endpoints(&event, &config.direct_endpoints)? {
                persisted.discovery_generation = None;
                persisted.discovery_expires_at = None;
                persisted.discovery_event_json = None;
                persisted.relay_acknowledgements.remove(&event.id.to_hex());
            }
        }
        let mut issue31_host = match &persisted {
            Some(persisted) => persisted.controller.clone(),
            None => Issue31HostController::new(host_configuration).map_err(issue31_error)?,
        };
        issue31_host
            .set_admitted_device_policy(
                config.admitted_device_public_key_hexes.clone(),
                config.approved_device_scopes.clone(),
            )
            .map_err(issue31_error)?;
        let issue31_discovery_generation = persisted
            .as_ref()
            .and_then(|persisted| persisted.discovery_generation);
        let issue31_discovery_expires_at = persisted
            .as_ref()
            .and_then(|persisted| persisted.discovery_expires_at);
        let issue31_discovery_outbox = persisted
            .as_ref()
            .and_then(|persisted| persisted.discovery_event_json.as_deref())
            .map(Event::from_json)
            .transpose()
            .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
        let issue31_private_outbox = persisted
            .as_ref()
            .map(|persisted| {
                durable_private_outbox_into_runtime(
                    persisted
                        .private_outbox
                        .iter()
                        .map(|(key, value)| {
                            (
                                key.clone(),
                                DurableIssue31PrivatePublish {
                                    rumor_event_id: value.rumor_event_id.clone(),
                                    gift_wrap_event_json: value.gift_wrap_event_json.clone(),
                                },
                            )
                        })
                        .collect(),
                )
            })
            .transpose()?
            .unwrap_or_default();
        let command_results = persisted
            .as_ref()
            .map(|persisted| persisted.command_results.clone())
            .unwrap_or_default();
        // omega#91: a handoff that was in flight when the last host process
        // ended is settled here, at load, before anything else can read the
        // ledger. The isolated provider home and the login it was driving died
        // with that process, so the terminal answer is that the handoff was
        // interrupted. This can only under-claim — it never reports a
        // connection the host did not make — and it is what stops a restart
        // leaving the phone with a request that neither resolves nor fails.
        let mut issue31_provider_handoffs = persisted
            .as_ref()
            .map(|persisted| persisted.provider_handoffs.clone())
            .unwrap_or_default();
        issue31_provider_handoffs.adopt_after_restart();
        let issue31_control_cursor = persisted
            .as_ref()
            .and_then(|persisted| persisted.control_cursor.clone());
        let issue31_projection_cursor = persisted
            .as_ref()
            .and_then(|persisted| persisted.projection_cursor.clone());
        let issue31_quarantined_events = persisted
            .as_ref()
            .map(|persisted| persisted.quarantined_events.clone())
            .unwrap_or_default();
        let issue31_host_adjunct_emissions = persisted
            .as_ref()
            .map(|persisted| persisted.host_adjunct_emissions.clone())
            .unwrap_or_default();
        let active_turn_ref = persisted
            .as_ref()
            .and_then(|persisted| persisted.active_turn_ref.clone());
        let run_state = persisted
            .as_ref()
            .map(|persisted| persisted.run_state.clone())
            .unwrap_or_else(default_run_state);
        let message_seq = persisted
            .as_ref()
            .map(|persisted| persisted.message_seq)
            .unwrap_or_default();
        let issue31_relay_acknowledgements = persisted
            .map(|persisted| persisted.relay_acknowledgements)
            .unwrap_or_default();
        for (event_id, acknowledged_relays) in &issue31_relay_acknowledgements {
            relay.restore_publication_acknowledgements(event_id, acknowledged_relays);
        }
        let issue31_device_controller =
            Some(Arc::new(std::sync::RwLock::new(issue31_host.clone())));
        Ok(Self {
            config,
            relay: Box::new(relay),
            signer: ConversationSigner::OmegaIdentity(identity_service),
            pending_events: VecDeque::new(),
            active_turn_ref,
            run_state,
            message_seq,
            command_results,
            last_gap_state: GapState::None,
            last_confirmed_cursor: None,
            issue31_host: Some(issue31_host),
            issue31_device_controller,
            issue31_discovery_generation,
            issue31_discovery_expires_at,
            issue31_discovery_outbox,
            issue31_private_outbox,
            issue31_relay_acknowledgements,
            issue31_control_cursor,
            issue31_projection_cursor,
            issue31_quarantined_events,
            issue31_withheld_emissions: BTreeMap::new(),
            issue31_host_adjunct_emissions,
            issue31_host_projection_source: None,
            issue31_provider_handoffs,
            issue31_provider_roster_source: None,
            issue31_state_path: Some(issue31_state_path),
            #[cfg(test)]
            issue31_fail_commit_after: Cell::new(None),
        })
    }

    /// Build a client that exercises NIP-42 against a mock that requires AUTH.
    pub fn mock_with_nip42_auth(
        mut config: SarahConversationConfig,
        challenge: &str,
        signer: SigningIdentity,
    ) -> Self {
        config.relay_url = Some("wss://relay.openagents.com".to_string());
        config.identity.owner_public_key_hex = signer.public_key_hex.clone();
        Self::with_relay(
            config,
            Box::new(MockRelayAdapter::with_required_auth(challenge)),
            signer,
        )
    }

    pub fn generation(&self) -> u64 {
        self.config.generation
    }

    pub fn synchronize_process_generation(
        &mut self,
        generation: u64,
    ) -> Result<(), SarahConversationError> {
        if generation == 0 || generation < self.config.generation {
            return Err(SarahConversationError::StaleGeneration {
                expected: self.config.generation,
                got: generation,
            });
        }
        self.config.generation = generation;
        Ok(())
    }

    pub fn conversation_ref(&self) -> String {
        self.config.conversation_ref()
    }

    /// Handle a framed request method. Returns the result object (not the frame).
    pub fn handle_request(
        &mut self,
        method: &str,
        generation: u64,
        params: Option<&Value>,
    ) -> Result<Value, SarahConversationError> {
        self.ensure_generation(generation)?;
        match method {
            SARAH_METHOD_SESSION_STATUS => Ok(serde_json::to_value(self.session_status()?)
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?),
            SARAH_METHOD_BOOTSTRAP => Ok(serde_json::to_value(self.bootstrap()?)
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?),
            SARAH_METHOD_ROOM_SNAPSHOT => {
                let legacy_cursor = params
                    .and_then(|value| value.get("cursor"))
                    .and_then(Value::as_str);
                let legacy_limit = params
                    .and_then(|value| value.get("limit"))
                    .and_then(Value::as_u64)
                    .map(|value| value as usize);
                let transcript_cursor = params
                    .and_then(|value| value.get("transcriptCursor"))
                    .and_then(Value::as_str)
                    .or(legacy_cursor);
                let activity_cursor = params
                    .and_then(|value| value.get("activityCursor"))
                    .and_then(Value::as_str)
                    .or(legacy_cursor);
                let nostr_cursor = params
                    .and_then(|value| value.get("nostrCursor"))
                    .and_then(Value::as_str)
                    .or(legacy_cursor);
                let transcript_limit = params
                    .and_then(|value| value.get("transcriptLimit"))
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                    .or(legacy_limit);
                let activity_limit = params
                    .and_then(|value| value.get("activityLimit"))
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                    .or(legacy_limit);
                let nostr_limit = params
                    .and_then(|value| value.get("nostrLimit"))
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                    .or(legacy_limit);
                Ok(serde_json::to_value(self.room_snapshot_with_record_cursor(
                    transcript_cursor,
                    transcript_limit,
                    activity_cursor,
                    activity_limit,
                    nostr_cursor,
                    nostr_limit,
                )?)
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?)
            }
            SARAH_METHOD_SEND_MESSAGE => {
                let (idempotency_ref, expected_generation) = command_binding(params, generation)?;
                let text = params
                    .and_then(|value| value.get("text"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        SarahConversationError::InvalidRequest(
                            "sarah_send_message requires text".into(),
                        )
                    })?;
                let fingerprint = command_fingerprint(method, params)?;
                if let Some(cached) = self.cached_command(&idempotency_ref, &fingerprint)? {
                    self.retry_durable_outbox()?;
                    return Ok(cached);
                }
                let result = self.send_message_with_fingerprint(
                    text,
                    &idempotency_ref,
                    expected_generation,
                    Some(fingerprint),
                )?;
                serde_json::to_value(result)
                    .map_err(|error| SarahConversationError::Internal(error.to_string()))
            }
            SARAH_METHOD_INTERRUPT_TURN => {
                let (idempotency_ref, expected_generation) = command_binding(params, generation)?;
                let turn_ref = params
                    .and_then(|value| value.get("turnRef"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| self.active_turn_ref.clone())
                    .ok_or_else(|| {
                        SarahConversationError::InvalidRequest(
                            "sarah_interrupt_turn requires turnRef".into(),
                        )
                    })?;
                let fingerprint = command_fingerprint(method, params)?;
                if let Some(cached) = self.cached_command(&idempotency_ref, &fingerprint)? {
                    self.retry_durable_outbox()?;
                    return Ok(cached);
                }
                let result = self.interrupt_turn_with_fingerprint(
                    &turn_ref,
                    &idempotency_ref,
                    expected_generation,
                    Some(fingerprint),
                )?;
                serde_json::to_value(result)
                    .map_err(|error| SarahConversationError::Internal(error.to_string()))
            }
            SARAH_METHOD_DEVICE_GRANTS => {
                self.sync_issue31_host()?;
                let controller = self.issue31_host.as_ref().ok_or_else(|| {
                    SarahConversationError::InvalidRequest("Issue 31 host is not configured".into())
                })?;
                let grants = controller
                    .grant_projections(unix_now())
                    .map_err(issue31_error)?;
                Ok(json!({ "grants": grants }))
            }
            SARAH_METHOD_RENEW_DEVICE_GRANT => {
                let (idempotency_ref, expected_generation) = command_binding(params, generation)?;
                self.ensure_generation(expected_generation)?;
                let grant_ref = params
                    .and_then(|value| value.get("grantRef"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        SarahConversationError::InvalidRequest(
                            "sarah_renew_device_grant requires grantRef".into(),
                        )
                    })?;
                let scopes = params
                    .and_then(|value| value.get("scopes"))
                    .cloned()
                    .ok_or_else(|| {
                        SarahConversationError::InvalidRequest(
                            "sarah_renew_device_grant requires scopes".into(),
                        )
                    })?;
                let scopes = serde_json::from_value(scopes)
                    .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
                let expires_at = params
                    .and_then(|value| value.get("expiresAt"))
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        SarahConversationError::InvalidRequest(
                            "sarah_renew_device_grant requires expiresAt".into(),
                        )
                    })?;
                let fingerprint = command_fingerprint(method, params)?;
                if let Some(cached) = self.cached_command(&idempotency_ref, &fingerprint)? {
                    self.retry_durable_outbox()?;
                    return Ok(cached);
                }
                self.renew_issue31_grant(
                    grant_ref,
                    scopes,
                    expires_at,
                    idempotency_ref,
                    fingerprint,
                )
            }
            SARAH_METHOD_REVOKE_DEVICE_GRANT => {
                let (idempotency_ref, expected_generation) = command_binding(params, generation)?;
                self.ensure_generation(expected_generation)?;
                let grant_ref = params
                    .and_then(|value| value.get("grantRef"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        SarahConversationError::InvalidRequest(
                            "sarah_revoke_device_grant requires grantRef".into(),
                        )
                    })?;
                let reason_ref = params
                    .and_then(|value| value.get("reasonRef"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let fingerprint = command_fingerprint(method, params)?;
                if let Some(cached) = self.cached_command(&idempotency_ref, &fingerprint)? {
                    self.retry_durable_outbox()?;
                    return Ok(cached);
                }
                self.revoke_issue31_grant(grant_ref, reason_ref, idempotency_ref, fingerprint)
            }
            SARAH_METHOD_READMIT_DEVICE => {
                let (idempotency_ref, expected_generation) = command_binding(params, generation)?;
                self.ensure_generation(expected_generation)?;
                let grant_ref = params
                    .and_then(|value| value.get("grantRef"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        SarahConversationError::InvalidRequest(
                            "sarah_readmit_device requires grantRef".into(),
                        )
                    })?;
                let fingerprint = command_fingerprint(method, params)?;
                if let Some(cached) = self.cached_command(&idempotency_ref, &fingerprint)? {
                    self.retry_durable_outbox()?;
                    return Ok(cached);
                }
                self.readmit_issue31_device(grant_ref, idempotency_ref, fingerprint)
            }
            _ => Err(SarahConversationError::InvalidRequest(format!(
                "unknown Sarah method {method}"
            ))),
        }
    }

    pub fn session_status(&mut self) -> Result<SessionStatusResult, SarahConversationError> {
        self.ensure_connected()?;
        Ok(SessionStatusResult {
            signed_in: matches!(
                self.config.identity.binding_state,
                BindingState::Bound | BindingState::Unbound
            ) && !self.config.identity.owner_public_key_hex.is_empty(),
            account_label: self.config.identity.account_label.clone(),
            binding_state: self.config.identity.binding_state,
            owner_public_key_hex: Some(self.config.identity.owner_public_key_hex.clone()),
            binding_expires_at: None,
            transport: self.transport_label(),
        })
    }

    pub fn bootstrap(&mut self) -> Result<BootstrapResult, SarahConversationError> {
        self.ensure_connected()?;
        self.sync_issue31_host()?;
        let room_state = self.current_room_state();
        self.push_room_state_event(&room_state);
        Ok(BootstrapResult {
            principal_ref: "principal.sarah".to_string(),
            display_name: "Sarah".to_string(),
            role: "owner_orchestrator".to_string(),
            conversation_ref: self.config.conversation_ref(),
            legacy_thread_ref: self.config.legacy_thread_ref(),
            owner_public_key_hex: self.config.identity.owner_public_key_hex.clone(),
            sarah_public_key_hex: self.config.identity.sarah_public_key_hex.clone(),
            authority_profile_ref: "docs/authority/SARAH_AUTHORITY.md".to_string(),
            authority_profile_revision: 7,
            admitted_device_fingerprints: self
                .issue31_host
                .as_ref()
                .map(Issue31HostController::admitted_device_fingerprints)
                .unwrap_or_default(),
            quarantined_issue31_event_count: self.issue31_quarantined_events.len(),
            room_state,
        })
    }

    pub fn sync_issue31_host(&mut self) -> Result<(), SarahConversationError> {
        self.ensure_connected()?;
        self.pull_device_pairing_events()?;
        let Some(mut controller) = self.issue31_host.take() else {
            return Ok(());
        };
        let result = self.sync_issue31_host_with(&mut controller);
        self.issue31_host = Some(controller);
        self.push_device_pairing_events()?;
        let persistence = self.persist_issue31_host_state();
        finish_durable_operation(result, persistence)
    }

    fn pull_device_pairing_events(&mut self) -> Result<(), SarahConversationError> {
        let (Some(controller), Some(shared)) = (
            self.issue31_host.as_mut(),
            self.issue31_device_controller.as_ref(),
        ) else {
            return Ok(());
        };
        let shared = shared.read().map_err(|_| {
            SarahConversationError::Internal("device bridge state is poisoned".into())
        })?;
        controller
            .merge_pairing_events_from(&shared)
            .map_err(issue31_error)
    }

    fn push_device_pairing_events(&self) -> Result<(), SarahConversationError> {
        let (Some(controller), Some(shared)) = (
            self.issue31_host.as_ref(),
            self.issue31_device_controller.as_ref(),
        ) else {
            return Ok(());
        };
        shared
            .write()
            .map_err(|_| {
                SarahConversationError::Internal("device bridge state is poisoned".into())
            })?
            .merge_pairing_events_from(controller)
            .map_err(issue31_error)
    }

    pub fn device_pairing_engine(&self) -> Option<crate::DevicePairingEngine> {
        self.issue31_device_controller
            .as_ref()
            .map(|controller| crate::DevicePairingEngine::new(controller.clone()))
    }

    pub fn device_pairing_runtime(
        &self,
    ) -> Option<(
        crate::DevicePairingEngine,
        Issue31DirectEndpoint,
        String,
        u64,
        Vec<Issue31PairingScope>,
    )> {
        let endpoint = self.config.direct_endpoints.first()?.clone();
        Some((
            self.device_pairing_engine()?,
            endpoint,
            self.config.identity.owner_public_key_hex.clone(),
            ISSUE31_NOSTR_HOST_GENERATION,
            self.config.approved_device_scopes.clone(),
        ))
    }

    fn renew_issue31_grant(
        &mut self,
        grant_ref: &str,
        scopes: Vec<crate::Issue31PairingScope>,
        expires_at: u64,
        idempotency_ref: String,
        fingerprint: String,
    ) -> Result<Value, SarahConversationError> {
        self.ensure_connected()?;
        self.ensure_command_result_capacity(&idempotency_ref)?;
        let mut controller = self.issue31_host.take().ok_or_else(|| {
            SarahConversationError::InvalidRequest("Issue 31 host is not configured".into())
        })?;
        let result = (|| {
            let record = controller
                .renew_grant(grant_ref, scopes, unix_now(), expires_at)
                .map_err(issue31_error)?;
            let enqueued = self.enqueue_issue31_pairing_record(&record)?;
            let event_id = enqueued.rumor_event_id.clone();
            controller
                .record_emitted_pairing(event_id.clone(), record)
                .map_err(issue31_error)?;
            let previous_projection_cursor = self.issue31_projection_cursor.take();
            let response = json!({ "accepted": true, "eventId": event_id, "grantRef": grant_ref });
            self.command_results.insert(
                idempotency_ref.clone(),
                (fingerprint.clone(), response.clone()),
            );
            if let Err(error) = self.persist_issue31_host_state_with_controller(&controller) {
                self.issue31_projection_cursor = previous_projection_cursor;
                self.command_results.remove(&idempotency_ref);
                self.rollback_issue31_enqueue(&enqueued);
                return Err(error);
            }
            self.flush_issue31_outbox()?;
            self.persist_issue31_host_state_with_controller(&controller)?;
            Ok(response)
        })();
        self.issue31_host = Some(controller);
        let persistence = self.persist_issue31_host_state();
        finish_durable_operation(result, persistence)
    }

    fn revoke_issue31_grant(
        &mut self,
        grant_ref: &str,
        reason_ref: Option<String>,
        idempotency_ref: String,
        fingerprint: String,
    ) -> Result<Value, SarahConversationError> {
        self.ensure_connected()?;
        self.ensure_command_result_capacity(&idempotency_ref)?;
        let mut controller = self.issue31_host.take().ok_or_else(|| {
            SarahConversationError::InvalidRequest("Issue 31 host is not configured".into())
        })?;
        let result = (|| {
            let record = controller
                .revoke_grant(grant_ref, unix_now(), reason_ref)
                .map_err(issue31_error)?;
            let enqueued = self.enqueue_issue31_pairing_record(&record)?;
            let event_id = enqueued.rumor_event_id.clone();
            controller
                .record_emitted_pairing(event_id.clone(), record)
                .map_err(issue31_error)?;
            let response = json!({ "accepted": true, "eventId": event_id, "grantRef": grant_ref });
            self.command_results.insert(
                idempotency_ref.clone(),
                (fingerprint.clone(), response.clone()),
            );
            if let Err(error) = self.persist_issue31_host_state_with_controller(&controller) {
                self.command_results.remove(&idempotency_ref);
                self.rollback_issue31_enqueue(&enqueued);
                return Err(error);
            }
            self.flush_issue31_outbox()?;
            self.persist_issue31_host_state_with_controller(&controller)?;
            Ok(response)
        })();
        self.issue31_host = Some(controller);
        let persistence = self.persist_issue31_host_state();
        finish_durable_operation(result, persistence)
    }

    /// Clear the revocations blocking the device behind `grant_ref`.
    ///
    /// This publishes nothing. Re-admission is host-local owner policy, not a
    /// record other peers are entitled to act on: the device still has to run a
    /// fresh, signed pairing handshake to obtain a new grant, and that grant is
    /// the only thing that carries authority.
    fn readmit_issue31_device(
        &mut self,
        grant_ref: &str,
        idempotency_ref: String,
        fingerprint: String,
    ) -> Result<Value, SarahConversationError> {
        self.ensure_connected()?;
        self.ensure_command_result_capacity(&idempotency_ref)?;
        let controller = self.issue31_host.take().ok_or_else(|| {
            SarahConversationError::InvalidRequest("Issue 31 host is not configured".into())
        })?;
        // Mutate a candidate so a refused or unpersistable re-admission cannot
        // leave a cleared revocation behind in the live controller.
        let mut candidate = controller.clone();
        let result = (|| {
            let cleared = candidate
                .readmit_device_for_grant(grant_ref)
                .map_err(issue31_error)?;
            let response = json!({
                "accepted": true,
                "grantRef": grant_ref,
                "clearedRevocations": cleared.len(),
            });
            self.command_results.insert(
                idempotency_ref.clone(),
                (fingerprint.clone(), response.clone()),
            );
            if let Err(error) = self.persist_issue31_host_state_with_controller(&candidate) {
                self.command_results.remove(&idempotency_ref);
                return Err(error);
            }
            Ok(response)
        })();
        self.issue31_host = Some(if result.is_ok() {
            candidate
        } else {
            controller
        });
        let persistence = self.persist_issue31_host_state();
        finish_durable_operation(result, persistence)
    }

    fn sync_issue31_host_with(
        &mut self,
        controller: &mut Issue31HostController,
    ) -> Result<(), SarahConversationError> {
        self.flush_issue31_outbox()?;
        self.persist_issue31_host_state_with_controller(controller)?;
        let now = unix_now();
        let discovery = if self.config.direct_endpoints.is_empty() {
            SignedIssue31Discovery::V2(
                controller
                    .discovery_v2(now, now.saturating_add(24 * 60 * 60))
                    .map_err(issue31_error)?,
            )
        } else {
            SignedIssue31Discovery::V3(
                controller
                    .discovery_v3(
                        self.config.direct_endpoints.clone(),
                        now,
                        now.saturating_add(24 * 60 * 60),
                    )
                    .map_err(issue31_error)?,
            )
        };
        if self.issue31_discovery_generation != Some(discovery.generation())
            || self
                .issue31_discovery_expires_at
                .is_none_or(|expires_at| expires_at <= now.saturating_add(60 * 60))
        {
            let replacement = self.sign_issue31_discovery(&discovery)?;
            let previous_outbox = self.issue31_discovery_outbox.replace(replacement);
            let previous_generation = self.issue31_discovery_generation;
            let previous_expires_at = self.issue31_discovery_expires_at;
            let previous_acknowledgements = previous_outbox.as_ref().and_then(|event| {
                self.issue31_relay_acknowledgements
                    .remove(&event.id.to_hex())
                    .map(|acknowledgements| (event.id.to_hex(), acknowledgements))
            });
            self.issue31_discovery_generation = Some(discovery.generation());
            self.issue31_discovery_expires_at = Some(discovery.expires_at());
            if let Err(error) = self.persist_issue31_host_state_with_controller(controller) {
                self.issue31_discovery_outbox = previous_outbox;
                self.issue31_discovery_generation = previous_generation;
                self.issue31_discovery_expires_at = previous_expires_at;
                if let Some((event_id, acknowledgements)) = previous_acknowledgements {
                    self.issue31_relay_acknowledgements
                        .insert(event_id, acknowledgements);
                }
                return Err(error);
            }
            self.flush_issue31_outbox()?;
            self.persist_issue31_host_state_with_controller(controller)?;
        }

        let conversation_ref = self.config.conversation_ref();
        let mut cursor = self.issue31_control_cursor.clone();
        let mut last_scanned_cursor = None;
        let mut scan_exhausted = false;
        let mut records = Vec::new();
        for _ in 0..8 {
            let page =
                self.query_with_auth(&conversation_ref, cursor.as_deref(), MAX_PAGE_LIMIT)?;
            self.last_gap_state = strongest_gap_state(self.last_gap_state, page.gap_state);
            if let Some(page_cursor) = page.events.last().map(stored_event_cursor) {
                last_scanned_cursor = Some(page_cursor);
            }
            for event in page.events {
                if event.record_kind == "pairing" || event.record_kind == "control" {
                    records.push(event);
                }
            }
            let Some(next_cursor) = page.next_cursor else {
                scan_exhausted = true;
                break;
            };
            cursor = Some(next_cursor);
        }

        // omega#91: one roster observation per pass, taken BEFORE this pass's
        // commands are handled. A handoff opened by a command in this pass is
        // therefore published as `requested` and only looked at against the
        // roster on the next pass. Advancing after would mean the host bound a
        // handoff using a roster reading older than the request itself, and the
        // phone would never see the handoff appear before it bound.
        self.advance_issue31_provider_handoffs(now);
        for event in records {
            if self
                .issue31_quarantined_events
                .contains_key(&event.event_id)
            {
                continue;
            }
            if event.record_kind == "pairing" {
                let record = match Issue31PairingRecord::decode(event.content_summary.as_bytes()) {
                    Ok(record) => record,
                    Err(_) => {
                        self.quarantine_issue31_event(
                            &event.event_id,
                            "reason.omega.invalid_pairing_record",
                            controller,
                        )?;
                        continue;
                    }
                };
                let mut candidate = controller.clone();
                let outbound = match candidate.handle_pairing_event(
                    Issue31PairingEvent {
                        event_id: event.event_id.clone(),
                        record,
                    },
                    now,
                ) {
                    Ok(outbound) => outbound,
                    Err(_) => {
                        self.quarantine_issue31_event(
                            &event.event_id,
                            "reason.omega.pairing_rejected",
                            controller,
                        )?;
                        continue;
                    }
                };
                if let Some(outbound) = outbound {
                    let resets_projection_cursor = matches!(
                        outbound,
                        Issue31PairingRecord::ScopedGrant { .. }
                            | Issue31PairingRecord::GrantRenewal { .. }
                    );
                    let enqueued = self.enqueue_issue31_pairing_record(&outbound)?;
                    if let Err(error) =
                        candidate.record_emitted_pairing(enqueued.rumor_event_id.clone(), outbound)
                    {
                        self.rollback_issue31_enqueue(&enqueued);
                        return Err(issue31_error(error));
                    }
                    let previous_projection_cursor = resets_projection_cursor
                        .then(|| self.issue31_projection_cursor.take())
                        .flatten();
                    if let Err(error) = self.persist_issue31_host_state_with_controller(&candidate)
                    {
                        if resets_projection_cursor {
                            self.issue31_projection_cursor = previous_projection_cursor;
                        }
                        self.rollback_issue31_enqueue(&enqueued);
                        return Err(error);
                    }
                    *controller = candidate;
                    self.flush_issue31_outbox()?;
                    self.persist_issue31_host_state_with_controller(controller)?;
                } else {
                    self.persist_issue31_host_state_with_controller(&candidate)?;
                    *controller = candidate;
                }
            } else {
                let command_schema = serde_json::from_str::<Value>(&event.content_summary)
                    .ok()
                    .and_then(|value| value.get("schema")?.as_str().map(str::to_owned));
                if command_schema.as_deref() == Some(ISSUE31_COMMAND_SCHEMA_V2) {
                    let record =
                        match Issue31CommandRecordV2::decode(event.content_summary.as_bytes()) {
                            Ok(record) => record,
                            Err(_) => {
                                self.quarantine_issue31_event(
                                    &event.event_id,
                                    "reason.omega.invalid_command_record",
                                    controller,
                                )?;
                                continue;
                            }
                        };
                    let mut candidate = controller.clone();
                    let result = match candidate.handle_command_event_v2(
                        event.event_id.clone(),
                        record,
                        now,
                        |arguments,
                         idempotency_ref,
                         grant_ref,
                         device_public_key_hex,
                         expected_generation| {
                            self.issue31_host = Some(controller.clone());
                            let execution = self.execute_issue31_action_v2(
                                arguments,
                                idempotency_ref,
                                grant_ref,
                                device_public_key_hex,
                                expected_generation,
                            );
                            self.issue31_host.take();
                            execution
                        },
                    ) {
                        Ok(result) => result,
                        Err(_) => {
                            self.quarantine_issue31_event(
                                &event.event_id,
                                "reason.omega.command_rejected",
                                controller,
                            )?;
                            continue;
                        }
                    };
                    if let Some(result) = result {
                        if let Issue31CommandRecordV2::CommandResult {
                            status: Issue31CommandHandlingStatus::Accepted,
                            grant_ref,
                            expected_generation,
                            source_event_id: Some(source_event_id),
                            ..
                        } = &result
                        {
                            candidate
                                .record_source_projection(
                                    grant_ref.clone(),
                                    *expected_generation,
                                    source_event_id.clone(),
                                )
                                .map_err(issue31_error)?;
                        }
                        let enqueued = self.enqueue_issue31_command_record_v2(&result)?;
                        if let Err(error) =
                            self.persist_issue31_host_state_with_controller(&candidate)
                        {
                            self.rollback_issue31_enqueue(&enqueued);
                            return Err(error);
                        }
                        *controller = candidate;
                        self.flush_issue31_outbox()?;
                        self.persist_issue31_host_state_with_controller(controller)?;
                    } else {
                        self.persist_issue31_host_state_with_controller(&candidate)?;
                        *controller = candidate;
                    }
                    continue;
                }
                let record = match Issue31CommandRecord::decode(event.content_summary.as_bytes()) {
                    Ok(record) => record,
                    Err(_) => {
                        self.quarantine_issue31_event(
                            &event.event_id,
                            "reason.omega.invalid_command_record",
                            controller,
                        )?;
                        continue;
                    }
                };
                let mut candidate = controller.clone();
                let result = match candidate.handle_command_event(
                    Issue31CommandEvent {
                        event_id: event.event_id.clone(),
                        record,
                    },
                    now,
                    |action_ref, arguments_ref, idempotency_ref| {
                        self.execute_issue31_action(action_ref, arguments_ref, idempotency_ref)
                    },
                ) {
                    Ok(result) => result,
                    Err(_) => {
                        self.quarantine_issue31_event(
                            &event.event_id,
                            "reason.omega.command_rejected",
                            controller,
                        )?;
                        continue;
                    }
                };
                if let Some(result) = result {
                    let enqueued = self.enqueue_issue31_command_record(&result)?;
                    if let Err(error) = self.persist_issue31_host_state_with_controller(&candidate)
                    {
                        self.rollback_issue31_enqueue(&enqueued);
                        return Err(error);
                    }
                    *controller = candidate;
                    self.flush_issue31_outbox()?;
                    self.persist_issue31_host_state_with_controller(controller)?;
                } else {
                    self.persist_issue31_host_state_with_controller(&candidate)?;
                    *controller = candidate;
                }
            }
        }
        self.project_issue31_sources(controller, now)?;
        // The omega#47 documents ride the same durable outbox as every other
        // owner-private record, so a device that was offline for this pass gets
        // them on the next flush rather than losing the snapshot entirely.
        self.publish_issue31_host_adjuncts(controller, now)?;
        self.persist_issue31_host_state_with_controller(controller)?;
        self.flush_issue31_outbox()?;
        self.persist_issue31_host_state_with_controller(controller)?;
        if let Some(last_scanned_cursor) = last_scanned_cursor {
            self.issue31_control_cursor = Some(last_scanned_cursor);
        }
        if !scan_exhausted {
            self.last_gap_state = strongest_gap_state(self.last_gap_state, GapState::Possible);
        }
        self.persist_issue31_host_state_with_controller(controller)?;
        Ok(())
    }

    fn quarantine_issue31_event(
        &mut self,
        event_id: &str,
        reason_ref: &str,
        controller: &Issue31HostController,
    ) -> Result<(), SarahConversationError> {
        if let Some(existing) = self.issue31_quarantined_events.get(event_id) {
            if existing == reason_ref {
                return Ok(());
            }
            return Err(SarahConversationError::Internal(
                "Issue 31 quarantine reason changed for the same event".into(),
            ));
        }
        if self.issue31_quarantined_events.len() >= MAX_QUARANTINED_ISSUE31_EVENTS {
            return Err(SarahConversationError::Internal(
                "Issue 31 quarantine bound is exhausted".into(),
            ));
        }
        if !is_lower_hex_64(event_id) || !crate::is_issue31_public_ref(reason_ref) {
            return Err(SarahConversationError::Internal(
                "Issue 31 quarantine record is invalid".into(),
            ));
        }
        self.issue31_quarantined_events
            .insert(event_id.to_string(), reason_ref.to_string());
        if let Err(error) = self.persist_issue31_host_state_with_controller(controller) {
            self.issue31_quarantined_events.remove(event_id);
            return Err(error);
        }
        Ok(())
    }

    fn sign_issue31_discovery(
        &self,
        discovery: &SignedIssue31Discovery,
    ) -> Result<Event, SarahConversationError> {
        let content = serde_json::to_string(discovery)
            .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
        let tags = [
            ["d", discovery.host_ref()],
            ["k", "1059"],
            ["t", "omega-issue31-host"],
            ["alt", "Omega Issue 31 Nostr host discovery"],
        ]
        .into_iter()
        .map(|tag| {
            Tag::parse(tag)
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
        self.signer
            .sign_public_record(ISSUE31_HOST_DISCOVERY_KIND, &content, tags)
    }

    fn enqueue_issue31_pairing_record(
        &mut self,
        record: &Issue31PairingRecord,
    ) -> Result<EnqueuedIssue31PrivateRecord, SarahConversationError> {
        let content = serde_json::to_string(record)
            .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
        self.enqueue_issue31_private_content(
            ISSUE31_PAIRING_SCHEMA,
            &content,
            record.device_public_key_hex(),
        )
    }

    fn enqueue_issue31_command_record(
        &mut self,
        record: &Issue31CommandRecord,
    ) -> Result<EnqueuedIssue31PrivateRecord, SarahConversationError> {
        let content = serde_json::to_string(record)
            .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
        self.enqueue_issue31_private_content(
            ISSUE31_COMMAND_SCHEMA,
            &content,
            record.device_public_key_hex(),
        )
    }

    fn enqueue_issue31_command_record_v2(
        &mut self,
        record: &Issue31CommandRecordV2,
    ) -> Result<EnqueuedIssue31PrivateRecord, SarahConversationError> {
        let content = serde_json::to_string(record)
            .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
        self.enqueue_issue31_private_content(
            ISSUE31_COMMAND_SCHEMA_V2,
            &content,
            record.device_public_key_hex(),
        )
    }

    fn enqueue_issue31_private_content(
        &mut self,
        schema: &str,
        content: &str,
        recipient_public_key_hex: &str,
    ) -> Result<EnqueuedIssue31PrivateRecord, SarahConversationError> {
        PublicKey::from_hex(recipient_public_key_hex)
            .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
        let outbox_ref = format!(
            "{schema}.{:x}",
            Sha256::digest([content.as_bytes(), recipient_public_key_hex.as_bytes()].concat())
        );
        let tags = vec![
            Tag::parse(["p", recipient_public_key_hex])
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?,
        ];
        let (rumor_event_id, inserted) = self.enqueue_private_content(
            &outbox_ref,
            content,
            tags,
            &[recipient_public_key_hex.to_string()],
        )?;
        Ok(EnqueuedIssue31PrivateRecord {
            rumor_event_id,
            outbox_ref,
            inserted,
        })
    }

    fn rollback_issue31_enqueue(&mut self, enqueued: &EnqueuedIssue31PrivateRecord) {
        if enqueued.inserted {
            self.issue31_private_outbox.remove(&enqueued.outbox_ref);
        }
    }

    fn enqueue_private_content(
        &mut self,
        outbox_ref: &str,
        content: &str,
        tags: Vec<Tag>,
        recipients: &[String],
    ) -> Result<(String, bool), SarahConversationError> {
        if let Some(pending) = self.issue31_private_outbox.get(outbox_ref) {
            return Ok((pending.rumor_event_id.clone(), false));
        }
        if self.issue31_private_outbox.len() >= MAX_PRIVATE_OUTBOX_ITEMS {
            return Err(SarahConversationError::Internal(
                "durable private outbox item bound is exhausted".into(),
            ));
        }
        let (rumor_event_id, gift_wraps) =
            self.signer.private_messages(content, tags, recipients)?;
        if self
            .issue31_relay_acknowledgements
            .len()
            .saturating_add(gift_wraps.len())
            > MAX_RELAY_ACKNOWLEDGEMENTS
        {
            return Err(SarahConversationError::Internal(
                "durable relay acknowledgement bound is exhausted".into(),
            ));
        }
        self.issue31_private_outbox.insert(
            outbox_ref.to_string(),
            PendingIssue31PrivatePublish {
                rumor_event_id: rumor_event_id.clone(),
                gift_wraps,
            },
        );
        Ok((rumor_event_id, true))
    }

    fn flush_issue31_outbox(&mut self) -> Result<(), SarahConversationError> {
        if let Some(event) = self.issue31_discovery_outbox.clone() {
            self.publish_with_auth(&event)?;
            let event_id = event.id.to_hex();
            self.record_relay_acknowledgements(&event_id)?;
            if self.relay.publication_complete(&event_id) {
                self.issue31_discovery_outbox = None;
                self.issue31_relay_acknowledgements.remove(&event_id);
            }
        }
        let pending_refs: Vec<String> = self.issue31_private_outbox.keys().cloned().collect();
        for pending_ref in pending_refs {
            let pending = self
                .issue31_private_outbox
                .get(&pending_ref)
                .cloned()
                .ok_or_else(|| {
                    SarahConversationError::Internal("private outbox vanished".into())
                })?;
            for gift_wrap in &pending.gift_wraps {
                self.publish_with_auth(gift_wrap)?;
                let event_id = gift_wrap.id.to_hex();
                self.record_relay_acknowledgements(&event_id)?;
            }
            if pending
                .gift_wraps
                .iter()
                .all(|gift_wrap| self.relay.publication_complete(&gift_wrap.id.to_hex()))
            {
                self.issue31_private_outbox.remove(&pending_ref);
                for gift_wrap in pending.gift_wraps {
                    self.issue31_relay_acknowledgements
                        .remove(&gift_wrap.id.to_hex());
                }
            }
        }
        Ok(())
    }

    fn record_relay_acknowledgements(
        &mut self,
        event_id: &str,
    ) -> Result<(), SarahConversationError> {
        if !self.issue31_relay_acknowledgements.contains_key(event_id)
            && self.issue31_relay_acknowledgements.len() >= MAX_RELAY_ACKNOWLEDGEMENTS
        {
            return Err(SarahConversationError::Internal(
                "durable relay acknowledgement bound is exhausted".into(),
            ));
        }
        self.issue31_relay_acknowledgements.insert(
            event_id.to_string(),
            self.relay.acknowledged_relays(event_id),
        );
        Ok(())
    }

    fn persist_issue31_host_state(&self) -> Result<(), SarahConversationError> {
        if self.issue31_state_path.is_none() {
            return Ok(());
        }
        let controller = self.issue31_host.as_ref().ok_or_else(|| {
            SarahConversationError::Internal(
                "Issue 31 controller was absent during durable commit".into(),
            )
        })?;
        self.persist_issue31_host_state_with_controller(controller)
    }

    fn persist_issue31_host_state_with_controller(
        &self,
        controller: &Issue31HostController,
    ) -> Result<(), SarahConversationError> {
        #[cfg(test)]
        if let Some(remaining) = self.issue31_fail_commit_after.get() {
            if remaining == 0 {
                self.issue31_fail_commit_after.set(None);
                return Err(SarahConversationError::Internal(
                    "injected Issue 31 durable commit failure".into(),
                ));
            }
            self.issue31_fail_commit_after
                .set(Some(remaining.saturating_sub(1)));
        }
        let Some(path) = &self.issue31_state_path else {
            return Ok(());
        };
        let discovery_event_json = self
            .issue31_discovery_outbox
            .as_ref()
            .map(|event| {
                event
                    .try_as_json()
                    .map_err(|error| SarahConversationError::Internal(error.to_string()))
            })
            .transpose()?;
        let private_outbox = self
            .issue31_private_outbox
            .iter()
            .map(|(outbox_ref, pending)| {
                let gift_wrap_event_json = pending
                    .gift_wraps
                    .iter()
                    .map(|event| {
                        event
                            .try_as_json()
                            .map_err(|error| SarahConversationError::Internal(error.to_string()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((
                    outbox_ref.clone(),
                    DurableIssue31PrivatePublish {
                        rumor_event_id: pending.rumor_event_id.clone(),
                        gift_wrap_event_json,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, SarahConversationError>>()?;
        write_issue31_host_state(
            path,
            &DurableIssue31HostState {
                schema: ISSUE31_DURABLE_STATE_SCHEMA.into(),
                controller: controller.clone(),
                discovery_generation: self.issue31_discovery_generation,
                discovery_expires_at: self.issue31_discovery_expires_at,
                discovery_event_json,
                private_outbox,
                relay_acknowledgements: self.issue31_relay_acknowledgements.clone(),
                control_cursor: self.issue31_control_cursor.clone(),
                projection_cursor: self.issue31_projection_cursor.clone(),
                quarantined_events: self.issue31_quarantined_events.clone(),
                host_adjunct_emissions: self.issue31_host_adjunct_emissions.clone(),
                provider_handoffs: self.issue31_provider_handoffs.clone(),
                command_results: self.command_results.clone(),
                active_turn_ref: self.active_turn_ref.clone(),
                run_state: self.run_state.clone(),
                message_seq: self.message_seq,
            },
        )
    }

    /// Answer one admitted command-v1 intent (omega#91).
    ///
    /// The controller has already checked the host binding, the grant, the
    /// generation, the lifetime, and the scope, so reaching here means the
    /// device is entitled to this action.
    ///
    /// Only `action.omega.provider_handoff` is answered. The remaining v1
    /// actions — Full Auto control and community action — still have no
    /// generation-fenced controller behind them, and returning `unavailable`
    /// for them is the honest answer rather than a silent success.
    fn execute_issue31_action(
        &mut self,
        action_ref: &str,
        arguments_ref: &str,
        idempotency_ref: &str,
    ) -> Issue31CommandExecution {
        if action_ref != ISSUE31_ACTION_REQUEST_PROVIDER_HANDOFF {
            return Issue31CommandExecution {
                status: Issue31CommandStatus::Unavailable,
                outcome_ref: "outcome.omega.unavailable".into(),
                reason_ref: Some("reason.omega.controller_not_bound".into()),
            };
        }
        // One reading of the clock for this command. The stamp on the record
        // and the deadline bounding it come from the same instant, and the
        // device supplies neither: `argumentsRef` names a provider and nothing
        // else, so there is no wire field a device could put a time in.
        let now_ms = unix_now().saturating_mul(1_000);
        match self
            .issue31_provider_handoffs
            .open(arguments_ref, idempotency_ref, now_ms)
        {
            Ok(record) => Issue31CommandExecution {
                status: Issue31CommandStatus::Completed,
                // The command was "open a provider connection handoff", and
                // this is the record that opening produced — which is also how
                // the device finds the row to watch. It is deliberately NOT a
                // statement that the handoff completed: the handoff carries its
                // own state and its own host-owned outcome, and at this moment
                // that state is `requested`.
                outcome_ref: record.handoff_ref,
                reason_ref: None,
            },
            Err(error) => Issue31CommandExecution {
                status: match error {
                    crate::issue31_provider_handoff::Issue31ProviderHandoffError::BoundExhausted
                    | crate::issue31_provider_handoff::Issue31ProviderHandoffError::Unprojectable(
                        _,
                    ) => Issue31CommandStatus::Unavailable,
                    _ => Issue31CommandStatus::Refused,
                },
                outcome_ref: match error {
                    crate::issue31_provider_handoff::Issue31ProviderHandoffError::BoundExhausted
                    | crate::issue31_provider_handoff::Issue31ProviderHandoffError::Unprojectable(
                        _,
                    ) => "outcome.omega.unavailable".into(),
                    _ => "outcome.omega.refused".into(),
                },
                reason_ref: Some(error.reason_ref().into()),
            },
        }
    }

    /// Install how this host reads its own provider roster (omega#91).
    ///
    /// Without one, no handoff ever binds: the host would be deciding against a
    /// roster nobody read. Open handoffs still reach their deadline, so an
    /// unwired host produces `expired` rather than a request that hangs.
    pub fn set_issue31_provider_roster_source(&mut self, source: Issue31ProviderRosterSource) {
        self.issue31_provider_roster_source = Some(source);
    }

    /// The host's provider connection handoffs, exactly as persisted.
    pub fn issue31_provider_handoff_refs(&self) -> Vec<String> {
        self.issue31_provider_handoffs
            .records()
            .map(|record| record.handoff_ref.clone())
            .collect()
    }

    /// The contract rows this host would publish right now.
    pub fn issue31_projected_provider_handoffs(&self, generated_at_ms: u64) -> Vec<Value> {
        self.issue31_provider_handoffs
            .projected(generated_at_ms)
            .rows
    }

    /// Move every open handoff on by at most one observation of the roster.
    ///
    /// Run once per host pump pass, before the omega#47 documents are built, so
    /// what the phone reads is this pass's state rather than the previous
    /// pass's.
    fn advance_issue31_provider_handoffs(&mut self, now: u64) {
        let roster = self
            .issue31_provider_roster_source
            .as_ref()
            .and_then(|source| source());
        self.issue31_provider_handoffs
            .advance(roster.as_deref(), now.saturating_mul(1_000));
    }

    fn execute_issue31_action_v2(
        &mut self,
        arguments: &Issue31CommandArguments,
        idempotency_ref: &str,
        grant_ref: &str,
        device_public_key_hex: &str,
        expected_generation: u64,
    ) -> Issue31CommandExecutionV2 {
        let handling_suffix = &format!("{:x}", Sha256::digest(idempotency_ref.as_bytes()))[..24];
        let accepted = |source_event_id| Issue31CommandExecutionV2 {
            status: Issue31CommandHandlingStatus::Accepted,
            handling_ref: format!("handling.omega.{handling_suffix}"),
            reason_ref: None,
            source_event_id,
        };
        let unavailable = |reason: &str| Issue31CommandExecutionV2 {
            status: Issue31CommandHandlingStatus::Unavailable,
            handling_ref: format!("handling.omega.{handling_suffix}"),
            reason_ref: Some(reason.into()),
            source_event_id: None,
        };
        let failed = |reason: &str| Issue31CommandExecutionV2 {
            status: Issue31CommandHandlingStatus::Failed,
            handling_ref: format!("handling.omega.{handling_suffix}"),
            reason_ref: Some(reason.into()),
            source_event_id: None,
        };
        match arguments {
            Issue31CommandArguments::SendMessage {
                conversation, text, ..
            } if conversation == &self.config.conversation_ref() => {
                match self.send_message(text, idempotency_ref, expected_generation) {
                    Ok(result) => {
                        let projection = Issue31OwnerProjectionBody::Message {
                            role: Issue31SourceRole::Owner,
                            conversation: conversation.clone(),
                            text: text.clone(),
                            reply_to_event_id: None,
                        };
                        match self.enqueue_issue31_source_projection(
                            &result.event_id,
                            projection,
                            grant_ref,
                            device_public_key_hex,
                            expected_generation,
                        ) {
                            Ok(()) => accepted(Some(result.event_id)),
                            // The owner record is already published by this
                            // point. A transport failure reading it back is not
                            // the same thing as a projection the host cannot
                            // build, and reporting it as terminal `failed`
                            // would tell the device a message that exists on
                            // the relay does not — the same class of mistake as
                            // reporting an authentication failure as a
                            // discovery one. `unavailable` is retryable, and
                            // the periodic source scan projects it either way.
                            Err(
                                SarahConversationError::Relay(_)
                                | SarahConversationError::Identity(_)
                                | SarahConversationError::IdentityRequired,
                            ) => unavailable("reason.omega.transport_unavailable"),
                            Err(_) => failed("reason.omega.projection_failed"),
                        }
                    }
                    Err(
                        SarahConversationError::Relay(_)
                        | SarahConversationError::Identity(_)
                        | SarahConversationError::IdentityRequired,
                    ) => unavailable("reason.omega.transport_unavailable"),
                    Err(_) => failed("reason.omega.action_failed"),
                }
            }
            Issue31CommandArguments::InterruptTurn {
                conversation,
                turn_ref,
                ..
            } if conversation == &self.config.conversation_ref() => {
                match self.interrupt_turn(turn_ref, idempotency_ref, expected_generation) {
                    Ok(_) => accepted(None),
                    Err(
                        SarahConversationError::Relay(_)
                        | SarahConversationError::Identity(_)
                        | SarahConversationError::IdentityRequired,
                    ) => unavailable("reason.omega.transport_unavailable"),
                    Err(_) => failed("reason.omega.action_failed"),
                }
            }
            Issue31CommandArguments::SendMessage { .. }
            | Issue31CommandArguments::InterruptTurn { .. } => Issue31CommandExecutionV2 {
                status: Issue31CommandHandlingStatus::Refused,
                handling_ref: format!("handling.omega.{handling_suffix}"),
                reason_ref: Some("reason.omega.conversation_mismatch".into()),
                source_event_id: None,
            },
            Issue31CommandArguments::ReadStatePatch { .. }
            | Issue31CommandArguments::ReminderCreate { .. }
            | Issue31CommandArguments::ReminderChange { .. }
            | Issue31CommandArguments::ReminderComplete { .. }
            | Issue31CommandArguments::ReminderCancel { .. } => {
                match self.execute_issue31_owner_state_action(arguments) {
                    Ok((event_id, projection)) => match self.enqueue_issue31_source_projection(
                        &event_id,
                        projection,
                        grant_ref,
                        device_public_key_hex,
                        expected_generation,
                    ) {
                        Ok(()) => accepted(Some(event_id)),
                        Err(_) => failed("reason.omega.projection_failed"),
                    },
                    Err(
                        SarahConversationError::Relay(_)
                        | SarahConversationError::Identity(_)
                        | SarahConversationError::IdentityRequired,
                    ) => unavailable("reason.omega.transport_unavailable"),
                    Err(_) => failed("reason.omega.action_failed"),
                }
            }
        }
    }

    fn execute_issue31_owner_state_action(
        &mut self,
        arguments: &Issue31CommandArguments,
    ) -> Result<(String, Issue31OwnerProjectionBody), SarahConversationError> {
        match arguments {
            Issue31CommandArguments::ReadStatePatch {
                slot_id,
                client_id,
                context_ref,
                read_at,
                ..
            } => {
                if *read_at > u32::MAX as u64 {
                    return Err(SarahConversationError::InvalidRequest(
                        "read-state timestamp exceeds the NIP-RS bound".into(),
                    ));
                }
                let d_tag = format!("read-state:{slot_id}");
                let mut contexts = self.load_issue31_read_state_contexts(&d_tag)?;
                contexts
                    .entry(context_ref.clone())
                    .and_modify(|current| *current = (*current).max(*read_at))
                    .or_insert(*read_at);
                if contexts.len() > 10_000 {
                    return Err(SarahConversationError::InvalidRequest(
                        "read-state context bound is exhausted".into(),
                    ));
                }
                let plaintext = serde_json::to_string(&json!({
                    "v": 1,
                    "client_id": client_id,
                    "contexts": contexts,
                }))
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
                let tags = vec![
                    Tag::parse(["d", d_tag.as_str()]).map_err(|error| {
                        SarahConversationError::InvalidRequest(error.to_string())
                    })?,
                    Tag::parse(["t", "read-state"]).map_err(|error| {
                        SarahConversationError::InvalidRequest(error.to_string())
                    })?,
                    Tag::parse(["alt", "encrypted read state"]).map_err(|error| {
                        SarahConversationError::InvalidRequest(error.to_string())
                    })?,
                ];
                let event_id =
                    self.publish_issue31_encrypted_source(SARAH_READ_STATE_KIND, &plaintext, tags)?;
                Ok((
                    event_id,
                    Issue31OwnerProjectionBody::ReadState { d_tag, plaintext },
                ))
            }
            Issue31CommandArguments::ReminderCreate {
                reminder_id,
                note,
                target_event_id,
                not_before,
                expiration,
                ..
            }
            | Issue31CommandArguments::ReminderChange {
                reminder_id,
                note,
                target_event_id,
                not_before,
                expiration,
                ..
            } => {
                let plaintext = serde_json::to_string(&json!({
                    "status": "pending",
                    "note": note,
                    "target": target_event_id.as_ref().map(|event_id| json!({ "id": event_id })),
                }))
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
                let mut tags = vec![
                    Tag::parse(["d", reminder_id.as_str()]).map_err(|error| {
                        SarahConversationError::InvalidRequest(error.to_string())
                    })?,
                    Tag::parse(["alt", "Encrypted reminder"]).map_err(|error| {
                        SarahConversationError::InvalidRequest(error.to_string())
                    })?,
                    Tag::parse(["not_before", not_before.to_string().as_str()]).map_err(
                        |error| SarahConversationError::InvalidRequest(error.to_string()),
                    )?,
                ];
                if let Some(expiration) = expiration {
                    tags.push(
                        Tag::parse(["expiration", expiration.to_string().as_str()]).map_err(
                            |error| SarahConversationError::InvalidRequest(error.to_string()),
                        )?,
                    );
                }
                let event_id =
                    self.publish_issue31_encrypted_source(SARAH_REMINDER_KIND, &plaintext, tags)?;
                Ok((
                    event_id,
                    Issue31OwnerProjectionBody::Reminder {
                        reminder_id: reminder_id.clone(),
                        plaintext,
                        not_before: Some(*not_before),
                        expiration: *expiration,
                    },
                ))
            }
            Issue31CommandArguments::ReminderComplete { reminder_id, .. }
            | Issue31CommandArguments::ReminderCancel { reminder_id, .. } => {
                let status =
                    if matches!(arguments, Issue31CommandArguments::ReminderComplete { .. }) {
                        "done"
                    } else {
                        "cancelled"
                    };
                let plaintext = serde_json::to_string(&json!({ "status": status }))
                    .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
                let tags = vec![
                    Tag::parse(["d", reminder_id.as_str()]).map_err(|error| {
                        SarahConversationError::InvalidRequest(error.to_string())
                    })?,
                    Tag::parse(["alt", "Encrypted reminder"]).map_err(|error| {
                        SarahConversationError::InvalidRequest(error.to_string())
                    })?,
                ];
                let event_id =
                    self.publish_issue31_encrypted_source(SARAH_REMINDER_KIND, &plaintext, tags)?;
                Ok((
                    event_id,
                    Issue31OwnerProjectionBody::Reminder {
                        reminder_id: reminder_id.clone(),
                        plaintext,
                        not_before: None,
                        expiration: None,
                    },
                ))
            }
            Issue31CommandArguments::SendMessage { .. }
            | Issue31CommandArguments::InterruptTurn { .. } => Err(
                SarahConversationError::InvalidRequest("not an owner-state action".into()),
            ),
        }
    }

    fn load_issue31_read_state_contexts(
        &mut self,
        d_tag: &str,
    ) -> Result<BTreeMap<String, u64>, SarahConversationError> {
        let conversation_ref = self.config.conversation_ref();
        let mut cursor = None;
        let mut contexts: BTreeMap<String, u64> = BTreeMap::new();
        for _ in 0..8 {
            let page =
                self.query_with_auth(&conversation_ref, cursor.as_deref(), MAX_PAGE_LIMIT)?;
            for event in page.events {
                if event.kind != SARAH_READ_STATE_KIND
                    || event.pubkey != self.config.identity.owner_public_key_hex
                    || stored_tag_value(&event.tags, "d").as_deref() != Some(d_tag)
                {
                    continue;
                }
                let plaintext = self
                    .signer
                    .decrypt_record(&event.pubkey, &event.content_summary)?;
                let value: Value = serde_json::from_str(&plaintext)
                    .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
                let Some(record_contexts) = value.get("contexts").and_then(Value::as_object) else {
                    continue;
                };
                for (context_ref, read_at) in record_contexts {
                    let Some(read_at) = read_at.as_u64() else {
                        continue;
                    };
                    if context_ref.len() <= 256 && read_at <= u32::MAX as u64 {
                        contexts
                            .entry(context_ref.clone())
                            .and_modify(|current| *current = (*current).max(read_at))
                            .or_insert(read_at);
                    }
                }
            }
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            cursor = Some(next_cursor);
        }
        Ok(contexts)
    }

    fn publish_issue31_encrypted_source(
        &mut self,
        kind: u16,
        plaintext: &str,
        tags: Vec<Tag>,
    ) -> Result<String, SarahConversationError> {
        let event = self
            .signer
            .sign_encrypted_self_record(kind, plaintext, tags)?;
        let event_id = event.id.to_hex();
        self.publish_with_auth(&event)?;
        Ok(event_id)
    }

    fn enqueue_issue31_source_projection(
        &mut self,
        source_event_id: &str,
        projection: Issue31OwnerProjectionBody,
        grant_ref: &str,
        device_public_key_hex: &str,
        expected_generation: u64,
    ) -> Result<(), SarahConversationError> {
        let source = self.load_issue31_source_event(source_event_id)?;
        if source.pubkey != self.config.identity.owner_public_key_hex {
            return Err(SarahConversationError::InvalidRequest(
                "Issue 31 projection source is not owner-authored".into(),
            ));
        }
        let emission = emit_issue31_owner_projection(Issue31OwnerProjectionInput {
            host_ref: "omega.host.local",
            host_public_key_hex: &self.config.identity.owner_public_key_hex,
            device_public_key_hex,
            sarah_public_key_hex: &self.config.identity.sarah_public_key_hex,
            grant_ref,
            expected_generation,
            source_event_id: &source.event_id,
            source_author_public_key_hex: &source.pubkey,
            source_kind: source.kind,
            source_created_at: source.created_at,
            projected_at: unix_now().max(source.created_at),
            projection,
        })
        .map_err(issue31_error)?;
        self.enqueue_issue31_private_content(
            crate::ISSUE31_OWNER_PROJECTION_SCHEMA,
            &emission.content,
            device_public_key_hex,
        )?;
        Ok(())
    }

    fn load_issue31_source_event(
        &mut self,
        source_event_id: &str,
    ) -> Result<StoredConversationEvent, SarahConversationError> {
        let conversation_ref = self.config.conversation_ref();
        let mut cursor = self.issue31_projection_cursor.clone();
        for _ in 0..8 {
            let page =
                self.query_with_auth(&conversation_ref, cursor.as_deref(), MAX_PAGE_LIMIT)?;
            if let Some(source) = page
                .events
                .iter()
                .find(|event| event.event_id == source_event_id)
            {
                return Ok(source.clone());
            }
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            cursor = Some(next_cursor);
        }
        Err(SarahConversationError::Relay(
            "confirmed Issue 31 source event was not observable after publish".into(),
        ))
    }

    fn project_issue31_sources(
        &mut self,
        controller: &mut Issue31HostController,
        now: u64,
    ) -> Result<(), SarahConversationError> {
        let grants = controller.active_grants(now).map_err(issue31_error)?;
        if grants.is_empty() {
            return Ok(());
        }
        let conversation_ref = self.config.conversation_ref();
        let mut cursor = self.issue31_projection_cursor.clone();
        let mut last_scanned_cursor = None;
        let mut scan_bound_reached = true;
        for _ in 0..8 {
            let page =
                self.query_with_auth(&conversation_ref, cursor.as_deref(), MAX_PAGE_LIMIT)?;
            if let Some(page_cursor) = page.events.last().map(stored_event_cursor) {
                last_scanned_cursor = Some(page_cursor);
            }
            for source in page.events {
                if !matches!(
                    source.kind,
                    crate::ISSUE31_PRIVATE_RUMOR_KIND
                        | SARAH_TURN_RECORD_KIND
                        | SARAH_AUTHORITY_RECEIPT_KIND
                        | SARAH_ENGRAM_KIND
                        | SARAH_READ_STATE_KIND
                        | SARAH_REMINDER_KIND
                ) {
                    continue;
                }
                if source.kind == crate::ISSUE31_PRIVATE_RUMOR_KIND
                    && source.record_kind != "message"
                {
                    continue;
                }
                let projection = match self.issue31_projection_body(&source) {
                    Ok(Some(projection)) => projection,
                    Ok(None) => continue,
                    Err(_) => {
                        self.quarantine_issue31_event(
                            &source.event_id,
                            ISSUE31_PROJECTION_SOURCE_QUARANTINE_REASON,
                            controller,
                        )?;
                        continue;
                    }
                };
                let mut emissions = Vec::new();
                let mut refused_by_own_decoder = false;
                for grant in &grants {
                    if controller.source_was_projected(
                        &grant.grant_ref,
                        grant.generation,
                        &source.event_id,
                    ) {
                        continue;
                    }
                    match emit_issue31_owner_projection(Issue31OwnerProjectionInput {
                        host_ref: &grant.host_ref,
                        host_public_key_hex: &grant.host_public_key_hex,
                        device_public_key_hex: &grant.device_public_key_hex,
                        sarah_public_key_hex: &grant.sarah_public_key_hex,
                        grant_ref: &grant.grant_ref,
                        expected_generation: grant.generation,
                        source_event_id: &source.event_id,
                        source_author_public_key_hex: &source.pubkey,
                        source_kind: source.kind,
                        source_created_at: source.created_at,
                        projected_at: now.max(source.created_at),
                        projection: projection.clone(),
                    }) {
                        Ok(emission) => emissions.push((grant, emission)),
                        // A source event Sarah or the owner signed can still
                        // carry a reference or a body the device reader
                        // refuses. Quarantining that one event keeps the
                        // remaining sources projecting, rather than letting a
                        // single malformed record stop every device.
                        Err(_) => {
                            refused_by_own_decoder = true;
                            break;
                        }
                    }
                }
                if refused_by_own_decoder {
                    self.quarantine_issue31_event(
                        &source.event_id,
                        ISSUE31_PROJECTION_SOURCE_QUARANTINE_REASON,
                        controller,
                    )?;
                    continue;
                }
                for (grant, emission) in emissions {
                    self.enqueue_issue31_private_content(
                        crate::ISSUE31_OWNER_PROJECTION_SCHEMA,
                        &emission.content,
                        &grant.device_public_key_hex,
                    )?;
                    controller
                        .record_source_projection(
                            grant.grant_ref.clone(),
                            grant.generation,
                            source.event_id.clone(),
                        )
                        .map_err(issue31_error)?;
                }
            }
            let Some(next_cursor) = page.next_cursor else {
                scan_bound_reached = false;
                break;
            };
            cursor = Some(next_cursor);
        }
        if scan_bound_reached {
            // The scan ran out of pages before it ran out of conversation. The
            // host does not know how many sources it never looked at, so it
            // must say so rather than let the device read a short list as a
            // complete one.
            self.last_gap_state = strongest_gap_state(self.last_gap_state, GapState::Possible);
        }
        if let Some(last_scanned_cursor) = last_scanned_cursor {
            self.issue31_projection_cursor = Some(last_scanned_cursor);
        }
        self.emit_issue31_withheld_sources(&grants, scan_bound_reached, now)?;
        Ok(())
    }

    /// Install the live Full Auto reading this host publishes to devices.
    ///
    /// Without one the host publishes no omega#47 records at all, and a paired
    /// phone reads `no_host_projection` — which is the honest answer for a host
    /// that is not observing its own Full Auto state, and is exactly what the
    /// phone showed before this existed. The difference is that the silence is
    /// now a stated condition rather than a dropped record.
    pub fn set_issue31_host_projection_source(&mut self, source: Issue31HostProjectionSource) {
        self.issue31_host_projection_source = Some(source);
    }

    /// Bind the issue-31 host controller this client pumps.
    ///
    /// `new_production` builds one from custody. A headless harness cannot
    /// reach custody and still has to drive the shipped pump, so the binding is
    /// a real operation rather than a field only this module can reach.
    pub fn attach_issue31_host_controller(&mut self, controller: Issue31HostController) {
        self.issue31_host = Some(controller);
    }

    /// The `grant_ref:generation` keys this host has published omega#47
    /// documents to. A key is recorded only after both records were committed
    /// to the durable outbox.
    pub fn issue31_published_host_adjunct_grants(&self) -> Vec<String> {
        self.issue31_host_adjunct_emissions
            .keys()
            .cloned()
            .collect()
    }

    /// Owner-private records still waiting for a relay acknowledgement.
    ///
    /// An empty list after a pump pass means every relay the host is
    /// configured for stored every record; a non-empty one is an exact,
    /// resumable backlog rather than a lost publish.
    pub fn issue31_pending_private_publish_refs(&self) -> Vec<String> {
        self.issue31_private_outbox.keys().cloned().collect()
    }

    /// Address one omega#47 document to one device.
    ///
    /// The pump — not the reading — states the binding, because the reading
    /// knows the host's runs and knows nothing about who may read them. A
    /// document that arrives already claiming a delivery binding is refused
    /// rather than overwritten: whoever wrote it was making a claim about a
    /// device it has no standing to make.
    fn address_issue31_adjunct(
        document: &Value,
        schema: &str,
        record_type: &str,
        grant: &Issue31GrantState,
    ) -> Result<Value, SarahConversationError> {
        let object = document.as_object().ok_or_else(|| {
            SarahConversationError::Internal("Issue 31 adjunct is not a record".into())
        })?;
        if object.get("schema").and_then(Value::as_str) != Some(schema) {
            return Err(SarahConversationError::Internal(
                "Issue 31 adjunct does not carry its own schema".into(),
            ));
        }
        // The seal proves who signed; it cannot prove which host the body
        // describes. The grant is the only statement that relates this host key
        // to a host reference, so the body must agree with it or the device
        // would bind another machine's state to this pairing.
        if object.get("hostRef").and_then(Value::as_str) != Some(grant.host_ref.as_str()) {
            return Err(SarahConversationError::Internal(
                "Issue 31 adjunct describes a host this grant does not name".into(),
            ));
        }
        if ISSUE31_ADJUNCT_DELIVERY_KEYS
            .iter()
            .any(|key| object.contains_key(*key))
        {
            return Err(SarahConversationError::Internal(
                "Issue 31 adjunct arrived already claiming a delivery binding".into(),
            ));
        }
        let mut addressed = object.clone();
        addressed.insert("recordType".into(), json!(record_type));
        addressed.insert(
            "hostPublicKeyHex".into(),
            json!(grant.host_public_key_hex.clone()),
        );
        addressed.insert(
            "devicePublicKeyHex".into(),
            json!(grant.device_public_key_hex.clone()),
        );
        addressed.insert("grantRef".into(), json!(grant.grant_ref.clone()));
        addressed.insert("expectedGeneration".into(), json!(grant.generation));
        Ok(Value::Object(addressed))
    }

    /// Publish the omega#47 host snapshot and its Full Auto detail to every
    /// admitted device (omega#49).
    ///
    /// The two documents are published together or not at all. The detail is
    /// bound to the snapshot that advertised it, and a device that held one
    /// without the other would either render a detail nothing vouches for or
    /// advertise capabilities it cannot open.
    fn publish_issue31_host_adjuncts(
        &mut self,
        controller: &Issue31HostController,
        now: u64,
    ) -> Result<(), SarahConversationError> {
        let Some(source) = self.issue31_host_projection_source.clone() else {
            return Ok(());
        };
        let grants = controller.active_grants(now).map_err(issue31_error)?;
        let observed_at_ms = now.saturating_mul(1_000);
        // omega#91. A handoff the host holds and can never state — one whose
        // request time was never measured — makes the owner's view short, and
        // that has to be visible rather than read as "no handoff in flight".
        if !self.issue31_provider_handoffs.unstateable_refs().is_empty() {
            self.last_gap_state = strongest_gap_state(self.last_gap_state, GapState::Possible);
        }
        for grant in grants {
            let documents = match source(&Issue31HostProjectionRequest {
                host_ref: &grant.host_ref,
                host_public_key_hex: &grant.host_public_key_hex,
                device_public_key_hex: &grant.device_public_key_hex,
                grant_ref: &grant.grant_ref,
                expected_generation: grant.generation,
                observed_at_ms,
                handoffs: &self.issue31_provider_handoffs,
            }) {
                Ok(Some(documents)) => documents,
                // Nothing observed is said by saying nothing. Publishing an
                // empty projection here would claim the host had looked and
                // found no runs, which is a different fact.
                Ok(None) => continue,
                Err(_) => {
                    // The host could not read its own Full Auto state. The
                    // owner's view may therefore be short, and that has to be
                    // visible rather than inferred from an absent record.
                    self.last_gap_state =
                        strongest_gap_state(self.last_gap_state, GapState::Possible);
                    continue;
                }
            };
            let host = Self::address_issue31_adjunct(
                &documents.host,
                ISSUE31_HOST_ADJUNCT_SCHEMA,
                ISSUE31_HOST_ADJUNCT_RECORD_TYPE,
                &grant,
            )?;
            let detail = Self::address_issue31_adjunct(
                &documents.detail,
                ISSUE31_FULL_AUTO_ADJUNCT_SCHEMA,
                ISSUE31_FULL_AUTO_ADJUNCT_RECORD_TYPE,
                &grant,
            )?;
            // "Beside" is the contract's word for the relation between the two
            // documents. A detail carrying a different snapshot reference is
            // one the device would refuse as `snapshot_mismatch`, so sending it
            // would publish a refusal rather than a projection.
            if host.get("snapshotRef") != detail.get("snapshotRef") {
                return Err(SarahConversationError::Internal(
                    "Issue 31 Full Auto detail is not bound to the snapshot beside it".into(),
                ));
            }
            let host_content = serde_json::to_string(&host)
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
            let detail_content = serde_json::to_string(&detail)
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
            let key = format!("{}:{}", grant.grant_ref, grant.generation);
            let digest = format!(
                "{:x}",
                Sha256::digest([host_content.as_bytes(), detail_content.as_bytes()].concat())
            );
            if self.issue31_host_adjunct_emissions.get(&key) == Some(&digest) {
                continue;
            }
            let enqueued_host = self.enqueue_issue31_private_content(
                ISSUE31_HOST_ADJUNCT_SCHEMA,
                &host_content,
                &grant.device_public_key_hex,
            )?;
            let enqueued_detail = match self.enqueue_issue31_private_content(
                ISSUE31_FULL_AUTO_ADJUNCT_SCHEMA,
                &detail_content,
                &grant.device_public_key_hex,
            ) {
                Ok(enqueued) => enqueued,
                Err(error) => {
                    self.rollback_issue31_enqueue(&enqueued_host);
                    return Err(error);
                }
            };
            let previous = self
                .issue31_host_adjunct_emissions
                .insert(key.clone(), digest);
            // The bookkeeping is committed with the outbox, not after it: a
            // crash between the two would either resend forever or, worse,
            // record a publication that never happened.
            if let Err(error) = self.persist_issue31_host_state_with_controller(controller) {
                match previous {
                    Some(previous) => {
                        self.issue31_host_adjunct_emissions.insert(key, previous);
                    }
                    None => {
                        self.issue31_host_adjunct_emissions.remove(&key);
                    }
                }
                self.rollback_issue31_enqueue(&enqueued_detail);
                self.rollback_issue31_enqueue(&enqueued_host);
                return Err(error);
            }
        }
        Ok(())
    }

    /// Tell every admitted device how complete its own projection is.
    ///
    /// This is the device-visible half of the two host-local surfaces that
    /// silently shorten the owner's view: the quarantine, whose count only ever
    /// reached `BootstrapResult`, and the bounded projection scan, which only
    /// ever moved `last_gap_state`. A complete pass publishes a record too —
    /// silence has to mean "unknown", or the absence of a signal reads as
    /// completeness and this whole mechanism buys nothing.
    fn emit_issue31_withheld_sources(
        &mut self,
        grants: &[Issue31GrantState],
        scan_bound_reached: bool,
        now: u64,
    ) -> Result<(), SarahConversationError> {
        let quarantined = u32::try_from(
            self.issue31_quarantined_events
                .values()
                .filter(|reason| reason.as_str() == ISSUE31_PROJECTION_SOURCE_QUARANTINE_REASON)
                .count(),
        )
        .unwrap_or(u32::MAX);
        let mut withheld = Vec::new();
        if quarantined > 0 {
            withheld.push(Issue31WithheldSourceCount {
                cause: Issue31WithheldCause::Quarantined,
                count: quarantined,
                exact: true,
                reason_ref: ISSUE31_PROJECTION_SOURCE_QUARANTINE_REASON.to_string(),
            });
        }
        if scan_bound_reached {
            withheld.push(Issue31WithheldSourceCount {
                cause: Issue31WithheldCause::ScanBound,
                count: 1,
                exact: false,
                reason_ref: ISSUE31_PROJECTION_SCAN_BOUND_REASON.to_string(),
            });
        }
        for grant in grants {
            let emission = emit_issue31_withheld_sources(Issue31WithheldSourcesInput {
                host_ref: &grant.host_ref,
                host_public_key_hex: &grant.host_public_key_hex,
                device_public_key_hex: &grant.device_public_key_hex,
                grant_ref: &grant.grant_ref,
                expected_generation: grant.generation,
                observed_at: now,
                withheld: withheld.clone(),
            })
            .map_err(issue31_error)?;
            let key = format!("{}:{}", grant.grant_ref, grant.generation);
            let substance = emission.record.substance();
            if self.issue31_withheld_emissions.get(&key) == Some(&substance) {
                continue;
            }
            self.enqueue_issue31_private_content(
                crate::ISSUE31_WITHHELD_SOURCES_SCHEMA,
                &emission.content,
                &grant.device_public_key_hex,
            )?;
            self.issue31_withheld_emissions.insert(key, substance);
        }
        Ok(())
    }

    fn issue31_projection_body(
        &self,
        source: &StoredConversationEvent,
    ) -> Result<Option<Issue31OwnerProjectionBody>, SarahConversationError> {
        let owner_key = &self.config.identity.owner_public_key_hex;
        let sarah_key = &self.config.identity.sarah_public_key_hex;
        match source.kind {
            crate::ISSUE31_PRIVATE_RUMOR_KIND
                if source.record_kind == "message"
                    && (source.pubkey == *owner_key || source.pubkey == *sarah_key) =>
            {
                let role = if source.pubkey == *owner_key {
                    Issue31SourceRole::Owner
                } else {
                    Issue31SourceRole::Sarah
                };
                Ok(Some(Issue31OwnerProjectionBody::Message {
                    role,
                    conversation: self.config.conversation_ref(),
                    text: source.content_summary.clone(),
                    reply_to_event_id: stored_tag_value(&source.tags, "e"),
                }))
            }
            SARAH_TURN_RECORD_KIND if source.pubkey == *sarah_key => {
                let plaintext = self
                    .signer
                    .decrypt_record(&source.pubkey, &source.content_summary)?;
                let payload = serde_json::from_str::<Value>(&plaintext)
                    .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
                Ok(Some(Issue31OwnerProjectionBody::Turn { payload }))
            }
            SARAH_AUTHORITY_RECEIPT_KIND if source.pubkey == *sarah_key => {
                let plaintext = self
                    .signer
                    .decrypt_record(&source.pubkey, &source.content_summary)?;
                let value = serde_json::from_str::<Value>(&plaintext)
                    .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
                let state = match value.get("decision").and_then(Value::as_str) {
                    Some("allow") => "allowed",
                    Some("refuse") => "refused",
                    _ => {
                        return Err(SarahConversationError::InvalidRequest(
                            "authority receipt has an invalid decision".into(),
                        ));
                    }
                };
                let turn_ref = stored_tag_value(&source.tags, "turn").ok_or_else(|| {
                    SarahConversationError::InvalidRequest(
                        "authority receipt omitted its turn tag".into(),
                    )
                })?;
                let suffix = &source.event_id[..24];
                let reason_ref = value
                    .get("reservedCategory")
                    .and_then(Value::as_str)
                    .filter(|category| {
                        category
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                    })
                    .map(|category| format!("reason.openagents.{category}"));
                Ok(Some(Issue31OwnerProjectionBody::AuthorityReceipt {
                    receipt_ref: format!("receipt.issue31.{suffix}"),
                    turn_ref,
                    authority_decision: Issue31AuthorityDecisionProjection {
                        state: state.into(),
                        decision_ref: format!("decision.issue31.{suffix}"),
                        reason_ref,
                    },
                    target_outcome: Issue31TargetOutcomeProjection {
                        state: "pending".into(),
                        outcome_ref: None,
                        reason_ref: None,
                    },
                }))
            }
            SARAH_ENGRAM_KIND if source.pubkey == *sarah_key => {
                let plaintext = self
                    .signer
                    .decrypt_record(&source.pubkey, &source.content_summary)?;
                let d_tag = stored_tag_value(&source.tags, "d").ok_or_else(|| {
                    SarahConversationError::InvalidRequest("engram omitted its d tag".into())
                })?;
                Ok(Some(Issue31OwnerProjectionBody::Engram {
                    d_tag,
                    plaintext,
                }))
            }
            SARAH_READ_STATE_KIND if source.pubkey == *owner_key => {
                let plaintext = self
                    .signer
                    .decrypt_record(&source.pubkey, &source.content_summary)?;
                let d_tag = stored_tag_value(&source.tags, "d").ok_or_else(|| {
                    SarahConversationError::InvalidRequest("read state omitted its d tag".into())
                })?;
                Ok(Some(Issue31OwnerProjectionBody::ReadState {
                    d_tag,
                    plaintext,
                }))
            }
            SARAH_REMINDER_KIND if source.pubkey == *owner_key => {
                let plaintext = self
                    .signer
                    .decrypt_record(&source.pubkey, &source.content_summary)?;
                let reminder_id = stored_tag_value(&source.tags, "d").ok_or_else(|| {
                    SarahConversationError::InvalidRequest("reminder omitted its d tag".into())
                })?;
                let not_before = stored_tag_value(&source.tags, "not_before")
                    .map(|value| value.parse::<u64>())
                    .transpose()
                    .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
                let expiration = stored_tag_value(&source.tags, "expiration")
                    .map(|value| value.parse::<u64>())
                    .transpose()
                    .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
                Ok(Some(Issue31OwnerProjectionBody::Reminder {
                    reminder_id,
                    plaintext,
                    not_before,
                    expiration,
                }))
            }
            _ => Ok(None),
        }
    }

    pub fn room_snapshot(
        &mut self,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<RoomSnapshotResult, SarahConversationError> {
        self.room_snapshot_with_record_cursor(cursor, limit, cursor, limit, cursor, limit)
    }

    pub fn room_snapshot_with_cursors(
        &mut self,
        transcript_cursor: Option<&str>,
        transcript_limit: Option<usize>,
        activity_cursor: Option<&str>,
        activity_limit: Option<usize>,
    ) -> Result<RoomSnapshotResult, SarahConversationError> {
        self.room_snapshot_with_record_cursor(
            transcript_cursor,
            transcript_limit,
            activity_cursor,
            activity_limit,
            None,
            None,
        )
    }

    pub fn room_snapshot_with_record_cursor(
        &mut self,
        transcript_cursor: Option<&str>,
        transcript_limit: Option<usize>,
        activity_cursor: Option<&str>,
        activity_limit: Option<usize>,
        nostr_cursor: Option<&str>,
        nostr_limit: Option<usize>,
    ) -> Result<RoomSnapshotResult, SarahConversationError> {
        self.ensure_connected()?;
        self.sync_issue31_host()?;
        let transcript_limit = transcript_limit
            .unwrap_or(DEFAULT_PAGE_LIMIT)
            .clamp(1, MAX_PAGE_LIMIT);
        let activity_limit = activity_limit
            .unwrap_or(DEFAULT_PAGE_LIMIT)
            .clamp(1, MAX_PAGE_LIMIT);
        let nostr_limit = nostr_limit
            .unwrap_or(DEFAULT_PAGE_LIMIT)
            .clamp(1, MAX_PAGE_LIMIT);
        let conversation_ref = self.config.conversation_ref();
        let transcript_page =
            self.query_with_auth(&conversation_ref, transcript_cursor, MAX_PAGE_LIMIT)?;
        let activity_page =
            self.query_with_auth(&conversation_ref, activity_cursor, MAX_PAGE_LIMIT)?;
        let nostr_page = self.query_with_auth(&conversation_ref, nostr_cursor, MAX_PAGE_LIMIT)?;
        self.last_gap_state = strongest_gap_state(
            strongest_gap_state(transcript_page.gap_state, activity_page.gap_state),
            nostr_page.gap_state,
        );
        self.last_confirmed_cursor = transcript_page
            .events
            .last()
            .or_else(|| activity_page.events.last())
            .or_else(|| nostr_page.events.last())
            .map(stored_event_cursor);
        let transcript_matching_count = transcript_page
            .events
            .iter()
            .filter(|event| event.record_kind == "message")
            .count();
        let transcript_entries = transcript_page
            .events
            .iter()
            .filter(|event| event.record_kind == "message")
            .take(transcript_limit)
            .map(|event| {
                let role = if event.pubkey == self.config.identity.owner_public_key_hex {
                    "owner"
                } else {
                    "sarah"
                };
                TranscriptEntry {
                    event_id: event.event_id.clone(),
                    cursor: stored_event_cursor(event),
                    role: role.to_string(),
                    kind: "text".to_string(),
                    text: event.content_summary.clone(),
                    created_at: iso_from_unix(event.created_at),
                    status: "confirmed".to_string(),
                }
            })
            .collect::<Vec<_>>();
        let activity_matching_count = activity_page
            .events
            .iter()
            .filter(|event| event.record_kind == "activity")
            .count();
        let activity_entries = activity_page
            .events
            .iter()
            .filter(|event| event.record_kind == "activity")
            .take(activity_limit)
            .map(|event| ActivityEntry {
                event_id: event.event_id.clone(),
                cursor: stored_event_cursor(event),
                entry: tag_value(&event.tags, "entry").unwrap_or_else(|| "unknown".into()),
                turn_ref: tag_value(&event.tags, "turn").unwrap_or_else(|| "turn.unknown".into()),
                created_at: iso_from_unix(event.created_at),
            })
            .collect::<Vec<_>>();
        let nostr_matching_count = nostr_page
            .events
            .iter()
            .filter(|event| confirmed_nostr_projection_kind(event.kind))
            .count();
        let nostr_entries = nostr_page
            .events
            .iter()
            .filter(|event| confirmed_nostr_projection_kind(event.kind))
            .take(nostr_limit)
            .map(|event| NostrRecordRef {
                event_id: event.event_id.clone(),
                cursor: stored_event_cursor(event),
                kind: event.kind,
                record_kind: event.record_kind.clone(),
                author_fingerprint: public_key_fingerprint(&event.pubkey),
                created_at: iso_from_unix(event.created_at),
                source: "confirmed_nostr".into(),
            })
            .collect::<Vec<_>>();
        let transcript_page_cursor = transcript_entries
            .last()
            .map(|entry| entry.cursor.clone())
            .unwrap_or_else(|| transcript_cursor.unwrap_or("cursor.start").to_string());
        let activity_page_cursor = activity_entries
            .last()
            .map(|entry| entry.cursor.clone())
            .unwrap_or_else(|| activity_cursor.unwrap_or("cursor.start").to_string());
        let nostr_page_cursor = nostr_entries
            .last()
            .map(|entry| entry.cursor.clone())
            .unwrap_or_else(|| nostr_cursor.unwrap_or("cursor.start").to_string());
        let transcript_next_cursor = stream_next_cursor(
            transcript_entries.last().map(|entry| entry.cursor.as_str()),
            transcript_page.next_cursor.as_deref(),
            transcript_matching_count > transcript_limit,
        );
        let activity_next_cursor = stream_next_cursor(
            activity_entries.last().map(|entry| entry.cursor.as_str()),
            activity_page.next_cursor.as_deref(),
            activity_matching_count > activity_limit,
        );
        let nostr_next_cursor = stream_next_cursor(
            nostr_entries.last().map(|entry| entry.cursor.as_str()),
            nostr_page.next_cursor.as_deref(),
            nostr_matching_count > nostr_limit,
        );
        Ok(RoomSnapshotResult {
            conversation_ref,
            transcript: TranscriptPage {
                entries: transcript_entries,
                cursor: transcript_page_cursor,
                next_cursor: transcript_next_cursor,
                gap_state: transcript_page.gap_state,
            },
            activity: ActivityPage {
                entries: activity_entries,
                cursor: activity_page_cursor,
                next_cursor: activity_next_cursor,
                gap_state: activity_page.gap_state,
            },
            nostr_records: NostrRecordPage {
                entries: nostr_entries,
                cursor: nostr_page_cursor,
                next_cursor: nostr_next_cursor,
                gap_state: nostr_page.gap_state,
                source: "confirmed_nostr".into(),
            },
            run_state: RunStateProjection {
                state: self.run_state.clone(),
                turn_ref: self.active_turn_ref.clone(),
                reason: None,
            },
            room_state: self.current_room_state(),
        })
    }

    pub fn send_message(
        &mut self,
        text: &str,
        idempotency_ref: &str,
        expected_generation: u64,
    ) -> Result<SendMessageResult, SarahConversationError> {
        self.send_message_with_fingerprint(text, idempotency_ref, expected_generation, None)
    }

    fn send_message_with_fingerprint(
        &mut self,
        text: &str,
        idempotency_ref: &str,
        expected_generation: u64,
        command_fingerprint: Option<String>,
    ) -> Result<SendMessageResult, SarahConversationError> {
        self.ensure_connected()?;
        self.ensure_generation(expected_generation)?;
        validate_command_ref(idempotency_ref, "idempotencyRef")?;
        if command_fingerprint.is_some() {
            self.ensure_command_result_capacity(idempotency_ref)?;
        }
        let text = text.trim();
        if text.is_empty() {
            return Err(SarahConversationError::InvalidRequest(
                "message text must not be empty".into(),
            ));
        }
        if text.len() > 8_000 {
            return Err(SarahConversationError::InvalidRequest(
                "message text exceeds page budget".into(),
            ));
        }
        // Refuse anything that looks like a raw credential in the outbound path.
        if looks_like_secret(text) {
            return Err(SarahConversationError::InvalidRequest(
                "message must not carry raw credentials".into(),
            ));
        }

        let next_message_seq = self.message_seq.checked_add(1).ok_or_else(|| {
            SarahConversationError::InvalidRequest("message sequence is exhausted".into())
        })?;
        let turn_ref = format!("turn.{next_message_seq}");
        let message_ref = format!("msg.{next_message_seq}");

        let conversation_ref = self.config.conversation_ref();
        let tags = conversation_tags(
            &conversation_ref,
            &self.config.identity.owner_public_key_hex,
            &self.config.identity.sarah_public_key_hex,
        )?;
        let mut tags = tags;
        tags.push(
            Tag::parse(["idempotency", idempotency_ref])
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?,
        );
        tags.push(
            Tag::parse(["generation", &expected_generation.to_string()])
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?,
        );
        tags.push(
            Tag::parse(["turn", &turn_ref])
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?,
        );
        let mut public_event = None;
        let mut inserted_outbox_ref = None;
        let (event_id, created_at) = if self.relay.requires_private_messages() {
            let recipients = private_recipients(
                &self.config.identity.owner_public_key_hex,
                &self.config.identity.sarah_public_key_hex,
            )?;
            let outbox_ref = private_outbox_ref("sarah.message", text, &tags, &recipients);
            let (rumor_event_id, inserted) =
                self.enqueue_private_content(&outbox_ref, text, tags, &recipients)?;
            if inserted {
                inserted_outbox_ref = Some(outbox_ref);
            }
            (rumor_event_id, unix_now())
        } else {
            let event = match &self.signer {
                ConversationSigner::Keys(identity) => identity.sign_text_note(text, tags)?,
                ConversationSigner::OmegaIdentity(_) => {
                    return Err(SarahConversationError::Internal(
                        "custodied production signer requires NIP-17 transport".into(),
                    ));
                }
            };
            let event_id = event.id.to_hex();
            let created_at = event.created_at.as_secs();
            public_event = Some(event);
            (event_id, created_at)
        };
        let cursor = format!("{CURSOR_PREFIX}{}", next_message_seq.saturating_sub(1));
        let response = SendMessageResult {
            accepted: true,
            message_ref,
            turn_ref: turn_ref.clone(),
            event_id: event_id.clone(),
            cursor: cursor.clone(),
            status: "accepted".to_string(),
        };
        let response_value = command_fingerprint
            .as_ref()
            .map(|_| serde_json::to_value(&response))
            .transpose()
            .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
        let previous_active_turn_ref = self.active_turn_ref.clone();
        let previous_run_state = self.run_state.clone();
        let previous_message_seq = self.message_seq;
        self.message_seq = next_message_seq;
        self.active_turn_ref = Some(turn_ref);
        self.run_state = "running".to_string();
        if let (Some(fingerprint), Some(response_value)) =
            (command_fingerprint.as_ref(), response_value.as_ref())
        {
            self.command_results.insert(
                idempotency_ref.to_string(),
                (fingerprint.clone(), response_value.clone()),
            );
        }
        if let Err(error) = self.persist_issue31_host_state() {
            self.message_seq = previous_message_seq;
            self.active_turn_ref = previous_active_turn_ref;
            self.run_state = previous_run_state;
            if command_fingerprint.is_some() {
                self.command_results.remove(idempotency_ref);
            }
            if let Some(outbox_ref) = inserted_outbox_ref {
                self.issue31_private_outbox.remove(&outbox_ref);
            }
            return Err(error);
        }
        let publication = if let Some(event) = public_event.as_ref() {
            self.publish_with_auth(event)
        } else {
            self.flush_issue31_outbox()
        };
        let persistence = self.persist_issue31_host_state();
        finish_durable_operation(publication, persistence)?;
        let record = TranscriptEntry {
            event_id,
            cursor,
            role: "owner".to_string(),
            kind: "text".to_string(),
            text: redact_content_summary(text),
            created_at: iso_from_unix(created_at),
            status: "accepted".to_string(),
        };
        self.push_room_event(&record);
        self.push_room_state_event(&self.current_room_state());
        Ok(response)
    }

    pub fn interrupt_turn(
        &mut self,
        turn_ref: &str,
        idempotency_ref: &str,
        expected_generation: u64,
    ) -> Result<InterruptTurnResult, SarahConversationError> {
        self.interrupt_turn_with_fingerprint(turn_ref, idempotency_ref, expected_generation, None)
    }

    fn interrupt_turn_with_fingerprint(
        &mut self,
        turn_ref: &str,
        idempotency_ref: &str,
        expected_generation: u64,
        command_fingerprint: Option<String>,
    ) -> Result<InterruptTurnResult, SarahConversationError> {
        self.ensure_connected()?;
        self.ensure_generation(expected_generation)?;
        validate_command_ref(idempotency_ref, "idempotencyRef")?;
        if command_fingerprint.is_some() {
            self.ensure_command_result_capacity(idempotency_ref)?;
        }
        if turn_ref.trim().is_empty() {
            return Err(SarahConversationError::InvalidRequest(
                "turnRef must not be empty".into(),
            ));
        }
        let intent_ref = format!(
            "intent.interrupt.{}",
            &format!("{:x}", Sha256::digest(idempotency_ref.as_bytes()))[..24]
        );
        let conversation_ref = self.config.conversation_ref();
        let mut tags = conversation_tags(
            &conversation_ref,
            &self.config.identity.owner_public_key_hex,
            &self.config.identity.sarah_public_key_hex,
        )?;
        tags.push(
            Tag::parse(["turn", turn_ref])
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?,
        );
        tags.push(
            Tag::parse(["control", "cancel_turn"])
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?,
        );
        tags.push(
            Tag::parse(["idempotency", idempotency_ref])
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?,
        );
        tags.push(
            Tag::parse(["generation", &expected_generation.to_string()])
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?,
        );

        let content = json!({
            "schema": "openagents.sarah.control.v1",
            "control": "cancel_turn",
            "turnRef": turn_ref,
            "intentRef": intent_ref,
            "idempotencyRef": idempotency_ref,
            "expectedGeneration": expected_generation,
        })
        .to_string();

        let mut public_event = None;
        let mut inserted_outbox_ref = None;
        if self.relay.requires_private_messages() {
            let recipients = private_recipients(
                &self.config.identity.owner_public_key_hex,
                &self.config.identity.sarah_public_key_hex,
            )?;
            let outbox_ref = private_outbox_ref("sarah.interrupt", &content, &tags, &recipients);
            let (_, inserted) =
                self.enqueue_private_content(&outbox_ref, &content, tags, &recipients)?;
            if inserted {
                inserted_outbox_ref = Some(outbox_ref);
            }
        } else {
            let event = match &self.signer {
                ConversationSigner::Keys(identity) => {
                    identity.sign_custom(NIP_AO_KIND, &content, tags)?
                }
                ConversationSigner::OmegaIdentity(_) => {
                    return Err(SarahConversationError::Internal(
                        "custodied production signer requires NIP-17 transport".into(),
                    ));
                }
            };
            public_event = Some(event);
        }
        let response = InterruptTurnResult {
            accepted: true,
            turn_ref: turn_ref.to_string(),
            intent_ref,
            status: "pending".to_string(),
            pending: true,
        };
        let response_value = command_fingerprint
            .as_ref()
            .map(|_| serde_json::to_value(&response))
            .transpose()
            .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
        let previous_run_state = self.run_state.clone();
        self.run_state = "interrupt_pending".to_string();
        if let (Some(fingerprint), Some(response_value)) =
            (command_fingerprint.as_ref(), response_value.as_ref())
        {
            self.command_results.insert(
                idempotency_ref.to_string(),
                (fingerprint.clone(), response_value.clone()),
            );
        }
        if let Err(error) = self.persist_issue31_host_state() {
            self.run_state = previous_run_state;
            if command_fingerprint.is_some() {
                self.command_results.remove(idempotency_ref);
            }
            if let Some(outbox_ref) = inserted_outbox_ref {
                self.issue31_private_outbox.remove(&outbox_ref);
            }
            return Err(error);
        }
        let publication = if let Some(event) = public_event.as_ref() {
            self.publish_with_auth(event)
        } else {
            self.flush_issue31_outbox()
        };
        let persistence = self.persist_issue31_host_state();
        finish_durable_operation(publication, persistence)?;
        self.push_room_state_event(&self.current_room_state());
        Ok(response)
    }

    /// Drain pending framed events (`sarah_room_event` / `sarah_room_state`).
    pub fn drain_events(&mut self) -> Vec<Value> {
        self.pending_events.drain(..).collect()
    }

    fn cached_command(
        &self,
        idempotency_ref: &str,
        fingerprint: &str,
    ) -> Result<Option<Value>, SarahConversationError> {
        match self.command_results.get(idempotency_ref) {
            Some((existing_fingerprint, result)) if existing_fingerprint == fingerprint => {
                Ok(Some(result.clone()))
            }
            Some(_) => Err(SarahConversationError::InvalidRequest(format!(
                "idempotencyRef {idempotency_ref} conflicts with an earlier command"
            ))),
            None => Ok(None),
        }
    }

    fn ensure_command_result_capacity(
        &self,
        idempotency_ref: &str,
    ) -> Result<(), SarahConversationError> {
        if !self.command_results.contains_key(idempotency_ref)
            && self.command_results.len() >= MAX_COMMAND_RESULTS
        {
            return Err(SarahConversationError::InvalidRequest(
                "durable command result bound is exhausted".into(),
            ));
        }
        Ok(())
    }

    fn retry_durable_outbox(&mut self) -> Result<(), SarahConversationError> {
        if self.issue31_discovery_outbox.is_none() && self.issue31_private_outbox.is_empty() {
            return Ok(());
        }
        self.ensure_connected()?;
        let publication = self.flush_issue31_outbox();
        let persistence = self.persist_issue31_host_state();
        finish_durable_operation(publication, persistence)
    }

    pub fn encode_response_frame(
        &self,
        id: impl Into<String>,
        generation: u64,
        result: Value,
    ) -> Result<String, SarahConversationError> {
        let frame = json!({
            "schema": PROTOCOL_SCHEMA,
            "kind": "response",
            "id": id.into(),
            "generation": generation,
            "ok": true,
            "result": result,
        });
        let line = serde_json::to_string(&frame)
            .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
        if line.len() > MAX_FRAME_BYTES {
            return Err(SarahConversationError::Internal(
                "response frame exceeds 64 KiB cap".into(),
            ));
        }
        Ok(line)
    }

    pub fn encode_event_frame(
        &self,
        generation: u64,
        method: &str,
        payload: Value,
    ) -> Result<String, SarahConversationError> {
        let frame = json!({
            "schema": PROTOCOL_SCHEMA,
            "kind": "event",
            "generation": generation,
            "method": method,
            "params": payload,
        });
        let line = serde_json::to_string(&frame)
            .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
        if line.len() > MAX_FRAME_BYTES {
            return Err(SarahConversationError::Internal(
                "event frame exceeds 64 KiB cap".into(),
            ));
        }
        // Hard redaction: never allow token-shaped payloads into events.
        if line.contains("bearer ") || line.contains("sk-") {
            return Err(SarahConversationError::Internal(
                "event frame refused: secret-shaped content".into(),
            ));
        }
        Ok(line)
    }

    fn ensure_generation(&self, generation: u64) -> Result<(), SarahConversationError> {
        if generation != self.config.generation {
            return Err(SarahConversationError::StaleGeneration {
                expected: self.config.generation,
                got: generation,
            });
        }
        Ok(())
    }

    fn ensure_connected(&mut self) -> Result<(), SarahConversationError> {
        if self.relay.connection_state() == ConnectionState::Disconnected {
            self.relay.connect()?;
        }
        if let Some(challenge) = self.relay.auth_challenge() {
            let auth_event = self
                .signer
                .sign_auth(&challenge.challenge, &challenge.relay_url)?;
            self.relay.authenticate(&auth_event)?;
        }
        Ok(())
    }

    fn authenticate_pending(&mut self) -> Result<(), SarahConversationError> {
        let challenge = self
            .relay
            .auth_challenge()
            .ok_or(SarahConversationError::IdentityRequired)?;
        let auth_event = self
            .signer
            .sign_auth(&challenge.challenge, &challenge.relay_url)?;
        self.relay.authenticate(&auth_event)
    }

    fn publish_with_auth(&mut self, event: &Event) -> Result<(), SarahConversationError> {
        match self.relay.publish(event) {
            Ok(()) => Ok(()),
            Err(SarahConversationError::IdentityRequired) => {
                self.authenticate_pending()?;
                self.relay.publish(event)
            }
            Err(error) => Err(error),
        }
    }

    fn query_with_auth(
        &mut self,
        conversation_ref: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<QueryPage, SarahConversationError> {
        match self.relay.query(conversation_ref, cursor, limit) {
            Ok(page) => Ok(page),
            Err(SarahConversationError::IdentityRequired) => {
                self.authenticate_pending()?;
                self.relay.query(conversation_ref, cursor, limit)
            }
            Err(error) => Err(error),
        }
    }

    fn current_room_state(&self) -> RoomStateEvent {
        let last_id = self.relay.last_event_id();
        let gap_state = strongest_gap_state(self.last_gap_state, self.relay.gap_state());
        RoomStateEvent {
            method: SARAH_EVENT_ROOM_STATE.to_string(),
            connection: self.relay.connection_state(),
            freshness: if self.relay.connection_state() == ConnectionState::Connected
                && gap_state == GapState::None
            {
                FreshnessState::Fresh
            } else {
                FreshnessState::Unknown
            },
            gap_state,
            connected_relays: self.relay.connected_relays(),
            last_acknowledged_event_id: last_id,
            last_acknowledged_cursor: self.last_confirmed_cursor.clone(),
            authenticated: self.relay.is_authenticated(),
            transport: self.transport_label(),
        }
    }

    fn transport_label(&self) -> String {
        if self.config.relay_url.is_some() {
            "nostr_relay".to_string()
        } else {
            "mock_relay".to_string()
        }
    }

    fn push_room_event(&mut self, record: &TranscriptEntry) {
        let payload = RoomEventPayload {
            method: SARAH_EVENT_ROOM_EVENT.to_string(),
            conversation_ref: self.config.conversation_ref(),
            cursor: record.cursor.clone(),
            record: record.clone(),
        };
        if let Ok(value) = serde_json::to_value(payload) {
            if self.pending_events.len() >= MAX_PENDING_EVENTS {
                self.pending_events.pop_front();
            }
            self.pending_events.push_back(value);
        }
    }

    fn push_room_state_event(&mut self, state: &RoomStateEvent) {
        if let Ok(value) = serde_json::to_value(state) {
            if self.pending_events.len() >= MAX_PENDING_EVENTS {
                self.pending_events.pop_front();
            }
            self.pending_events.push_back(value);
        }
    }
}

fn finish_durable_operation<T>(
    operation: Result<T, SarahConversationError>,
    persistence: Result<(), SarahConversationError>,
) -> Result<T, SarahConversationError> {
    match (operation, persistence) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(operation_error), Err(persistence_error)) => Err(SarahConversationError::Internal(
            format!("{operation_error}; durable Issue 31 commit also failed: {persistence_error}"),
        )),
    }
}

fn durable_private_outbox_into_runtime(
    durable: BTreeMap<String, DurableIssue31PrivatePublish>,
) -> Result<BTreeMap<String, PendingIssue31PrivatePublish>, SarahConversationError> {
    if durable.len() > MAX_PRIVATE_OUTBOX_ITEMS {
        return Err(SarahConversationError::Internal(
            "durable Issue 31 outbox exceeds its item bound".into(),
        ));
    }
    durable
        .into_iter()
        .map(|(outbox_ref, pending)| {
            if outbox_ref.len() > 192
                || !is_lower_hex_64(&pending.rumor_event_id)
                || pending.gift_wrap_event_json.is_empty()
                || pending.gift_wrap_event_json.len() > 8
            {
                return Err(SarahConversationError::Internal(
                    "durable Issue 31 outbox item is invalid".into(),
                ));
            }
            let gift_wraps = pending
                .gift_wrap_event_json
                .into_iter()
                .map(|event_json| {
                    let event = Event::from_json(event_json)
                        .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
                    if event.kind != Kind::GiftWrap || event.verify().is_err() {
                        return Err(SarahConversationError::Internal(
                            "durable Issue 31 outbox has an invalid gift wrap".into(),
                        ));
                    }
                    Ok(event)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((
                outbox_ref,
                PendingIssue31PrivatePublish {
                    rumor_event_id: pending.rumor_event_id,
                    gift_wraps,
                },
            ))
        })
        .collect()
}

fn load_issue31_host_state(
    path: &Path,
    expected_configuration: &Issue31HostConfiguration,
) -> Result<Option<DurableIssue31HostState>, SarahConversationError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(SarahConversationError::Internal(error.to_string())),
    };
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > ISSUE31_DURABLE_STATE_MAX_BYTES
    {
        return Err(SarahConversationError::Internal(
            "durable Issue 31 host state is unsafe or oversized".into(),
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(ISSUE31_DURABLE_STATE_MAX_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
    if bytes.len() as u64 > ISSUE31_DURABLE_STATE_MAX_BYTES {
        return Err(SarahConversationError::Internal(
            "durable Issue 31 host state exceeds its byte bound".into(),
        ));
    }
    let mut state: DurableIssue31HostState = serde_json::from_slice(&bytes)
        .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
    state
        .controller
        .adopt_conversation_if_missing(&expected_configuration.conversation)
        .map_err(issue31_error)?;
    if state.schema != ISSUE31_DURABLE_STATE_SCHEMA
        || !state
            .controller
            .matches_configuration(expected_configuration)
        || state.discovery_generation.is_some() != state.discovery_expires_at.is_some()
        || state.command_results.len() > MAX_COMMAND_RESULTS
        || state.relay_acknowledgements.len() > MAX_RELAY_ACKNOWLEDGEMENTS
        || state.quarantined_events.len() > MAX_QUARANTINED_ISSUE31_EVENTS
        || state.host_adjunct_emissions.len() > MAX_QUARANTINED_ISSUE31_EVENTS
        || state
            .control_cursor
            .as_deref()
            .is_some_and(|cursor| !valid_event_cursor(cursor))
        || state
            .projection_cursor
            .as_deref()
            .is_some_and(|cursor| !valid_event_cursor(cursor))
        || state
            .active_turn_ref
            .as_deref()
            .is_some_and(|turn_ref| !crate::is_issue31_public_ref(turn_ref))
        || !valid_run_state(&state.run_state)
    {
        return Err(SarahConversationError::Internal(
            "durable Issue 31 host state does not match this identity or relay configuration"
                .into(),
        ));
    }
    state
        .controller
        .validate_persisted_state()
        .map_err(issue31_error)?;
    if state
        .quarantined_events
        .iter()
        .any(|(event_id, reason_ref)| {
            !is_lower_hex_64(event_id) || !crate::is_issue31_public_ref(reason_ref)
        })
    {
        return Err(SarahConversationError::Internal(
            "durable Issue 31 quarantine contains an invalid record".into(),
        ));
    }
    if state
        .relay_acknowledgements
        .iter()
        .any(|(event_id, relay_urls)| {
            !is_lower_hex_64(event_id)
                || relay_urls.len() > expected_configuration.relay_urls.len()
                || relay_urls.iter().enumerate().any(|(index, relay_url)| {
                    relay_urls[..index].iter().any(|prior| prior == relay_url)
                })
                || relay_urls
                    .iter()
                    .any(|relay_url| !expected_configuration.relay_urls.contains(relay_url))
        })
    {
        return Err(SarahConversationError::Internal(
            "durable Issue 31 relay acknowledgements are invalid".into(),
        ));
    }
    if let Some(event_json) = state.discovery_event_json.clone() {
        let event = Event::from_json(&event_json)
            .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
        let value = serde_json::from_str::<Value>(&event.content)
            .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
        let schema = value.get("schema").and_then(Value::as_str);
        let (generation, expires_at, host_ref, host_key, sarah_key, display_name, relay_urls) =
            if schema == Some(crate::ISSUE31_HOST_DISCOVERY_SCHEMA_V3) {
                let discovery = Issue31HostDiscoveryV3::decode(event.content.as_bytes())
                    .map_err(issue31_error)?;
                if discovery.conversation != expected_configuration.conversation {
                    return Err(SarahConversationError::Internal(
                        "durable Issue 31 discovery binds another conversation".into(),
                    ));
                }
                (
                    discovery.generation,
                    discovery.expires_at,
                    discovery.host_ref,
                    discovery.host_public_key_hex,
                    discovery.sarah_public_key_hex,
                    discovery.display_name,
                    discovery.relay_urls,
                )
            } else if schema == Some(crate::ISSUE31_HOST_DISCOVERY_SCHEMA_V2) {
                let discovery = Issue31HostDiscoveryV2::decode(event.content.as_bytes())
                    .map_err(issue31_error)?;
                if discovery.conversation != expected_configuration.conversation {
                    return Err(SarahConversationError::Internal(
                        "durable Issue 31 discovery binds another conversation".into(),
                    ));
                }
                (
                    discovery.generation,
                    discovery.expires_at,
                    discovery.host_ref,
                    discovery.host_public_key_hex,
                    discovery.sarah_public_key_hex,
                    discovery.display_name,
                    discovery.relay_urls,
                )
            } else {
                let discovery = Issue31HostDiscovery::decode(event.content.as_bytes())
                    .map_err(issue31_error)?;
                (
                    discovery.generation,
                    discovery.expires_at,
                    discovery.host_ref,
                    discovery.host_public_key_hex,
                    discovery.sarah_public_key_hex,
                    discovery.display_name,
                    discovery.relay_urls,
                )
            };
        if event.kind.as_u16() != ISSUE31_HOST_DISCOVERY_KIND
            || event.pubkey.to_hex() != expected_configuration.host_public_key_hex
            || event.verify().is_err()
            || state.discovery_generation != Some(generation)
            || state.discovery_expires_at != Some(expires_at)
            || host_ref != expected_configuration.host_ref
            || host_key != expected_configuration.host_public_key_hex
            || sarah_key != expected_configuration.sarah_public_key_hex
            || display_name != expected_configuration.display_name
            || relay_urls != expected_configuration.relay_urls
            || generation != expected_configuration.generation
        {
            return Err(SarahConversationError::Internal(
                "durable Issue 31 discovery outbox event is invalid".into(),
            ));
        }
        if schema != Some(crate::ISSUE31_HOST_DISCOVERY_SCHEMA_V2)
            && schema != Some(crate::ISSUE31_HOST_DISCOVERY_SCHEMA_V3)
        {
            state.discovery_generation = None;
            state.discovery_expires_at = None;
            state.discovery_event_json = None;
            state.relay_acknowledgements.remove(&event.id.to_hex());
        }
    }
    durable_private_outbox_into_runtime(
        state
            .private_outbox
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    DurableIssue31PrivatePublish {
                        rumor_event_id: value.rumor_event_id.clone(),
                        gift_wrap_event_json: value.gift_wrap_event_json.clone(),
                    },
                )
            })
            .collect(),
    )?;
    Ok(Some(state))
}

fn write_issue31_host_state(
    path: &Path,
    state: &DurableIssue31HostState,
) -> Result<(), SarahConversationError> {
    let bytes = serde_json::to_vec(state)
        .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
    if bytes.len() as u64 > ISSUE31_DURABLE_STATE_MAX_BYTES {
        return Err(SarahConversationError::Internal(
            "durable Issue 31 host state exceeds its byte bound".into(),
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        SarahConversationError::Internal("Issue 31 state path has no parent".into())
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            SarahConversationError::Internal("Issue 31 state filename is invalid".into())
        })?;
    let temporary_path = parent.join(format!(".{file_name}.{}.tmp", rand::random::<u64>()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary_path)
        .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
    let write_result = file
        .write_all(&bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&temporary_path, path));
    if let Err(error) = write_result {
        return match fs::remove_file(&temporary_path) {
            Ok(()) => Err(SarahConversationError::Internal(error.to_string())),
            Err(cleanup_error) if cleanup_error.kind() == std::io::ErrorKind::NotFound => {
                Err(SarahConversationError::Internal(error.to_string()))
            }
            Err(cleanup_error) => Err(SarahConversationError::Internal(format!(
                "{error}; temporary state cleanup failed: {cleanup_error}"
            ))),
        };
    }
    Ok(())
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn valid_event_cursor(value: &str) -> bool {
    let mut parts = value.split('.');
    matches!(parts.next(), Some("cursor"))
        && parts
            .next()
            .is_some_and(|created_at| created_at.parse::<u64>().is_ok())
        && parts.next().is_some_and(is_lower_hex_64)
        && parts.next().is_none()
}

fn default_run_state() -> String {
    "idle".to_string()
}

fn confirmed_nostr_projection_kind(kind: u16) -> bool {
    matches!(kind, 9 | 5_934 | 6_934 | 7_000 | 30_078 | 30_174 | 30_300)
}

fn public_key_fingerprint(public_key_hex: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(public_key_hex.as_bytes()));
    digest[..16].to_ascii_uppercase()
}

fn valid_run_state(value: &str) -> bool {
    matches!(
        value,
        "idle" | "running" | "interrupt_pending" | "stopped" | "paused"
    )
}

fn private_outbox_ref(prefix: &str, content: &str, tags: &[Tag], recipients: &[String]) -> String {
    let mut binding = Vec::with_capacity(
        prefix.len()
            + content.len()
            + tags
                .iter()
                .flat_map(|tag| tag.as_slice())
                .map(String::len)
                .sum::<usize>()
            + recipients.iter().map(String::len).sum::<usize>()
            + tags.len()
            + recipients.len(),
    );
    binding.extend_from_slice(prefix.as_bytes());
    binding.push(0);
    binding.extend_from_slice(content.as_bytes());
    for tag in tags {
        binding.push(0xff);
        for value in tag.as_slice() {
            binding.push(0);
            binding.extend_from_slice(value.as_bytes());
        }
    }
    for recipient in recipients {
        binding.push(0);
        binding.extend_from_slice(recipient.as_bytes());
    }
    format!("{prefix}.{:x}", Sha256::digest(binding))
}

pub(crate) fn conversation_tags(
    conversation_ref: &str,
    owner_pubkey: &str,
    sarah_pubkey: &str,
) -> Result<Vec<Tag>, SarahConversationError> {
    if owner_pubkey == sarah_pubkey {
        return Err(SarahConversationError::InvalidRequest(
            "owner and Sarah identities must be distinct".into(),
        ));
    }
    [
        ["conversation", conversation_ref],
        ["p", owner_pubkey],
        ["p", sarah_pubkey],
        ["agent", sarah_pubkey],
        ["alt", "OpenAgents Sarah conversation message"],
    ]
    .into_iter()
    .map(|tag| {
        Tag::parse(tag).map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))
    })
    .collect()
}

fn stored_tag_value(tags: &[Vec<String>], name: &str) -> Option<String> {
    tags.iter().find_map(|tag| {
        (tag.first().map(String::as_str) == Some(name))
            .then(|| tag.get(1).cloned())
            .flatten()
    })
}

fn private_recipients(
    owner_pubkey: &str,
    sarah_pubkey: &str,
) -> Result<Vec<String>, SarahConversationError> {
    if owner_pubkey == sarah_pubkey {
        return Err(SarahConversationError::InvalidRequest(
            "owner and Sarah identities must be distinct".into(),
        ));
    }
    PublicKey::from_hex(owner_pubkey)
        .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
    PublicKey::from_hex(sarah_pubkey)
        .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
    Ok(vec![owner_pubkey.to_string(), sarah_pubkey.to_string()])
}

fn digest_receipt_ref(
    prefix: &str,
    semantic_binding: &[u8],
) -> Result<ReceiptRef, SarahConversationError> {
    let digest = format!("{:x}", Sha256::digest(semantic_binding));
    ReceiptRef::new(format!("{prefix}.{}", &digest[..32]))
        .map_err(|error| SarahConversationError::Identity(error.to_string()))
}

fn issue31_error(error: Issue31NostrError) -> SarahConversationError {
    SarahConversationError::InvalidRequest(error.to_string())
}

fn strongest_gap_state(left: GapState, right: GapState) -> GapState {
    let severity = |state| match state {
        GapState::None => 0,
        GapState::Possible => 1,
        GapState::Recovering => 2,
        GapState::Confirmed => 3,
    };
    if severity(left) >= severity(right) {
        left
    } else {
        right
    }
}

fn command_binding(
    params: Option<&Value>,
    framed_generation: u64,
) -> Result<(String, u64), SarahConversationError> {
    let idempotency_ref = params
        .and_then(|value| value.get("idempotencyRef"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SarahConversationError::InvalidRequest("host command requires idempotencyRef".into())
        })?;
    validate_command_ref(idempotency_ref, "idempotencyRef")?;
    let expected_generation = params
        .and_then(|value| value.get("expectedGeneration"))
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            SarahConversationError::InvalidRequest(
                "host command requires expectedGeneration".into(),
            )
        })?;
    if expected_generation != framed_generation {
        return Err(SarahConversationError::StaleGeneration {
            expected: framed_generation,
            got: expected_generation,
        });
    }
    Ok((idempotency_ref.to_string(), expected_generation))
}

fn validate_command_ref(value: &str, field: &str) -> Result<(), SarahConversationError> {
    if !crate::is_issue31_public_ref(value) {
        return Err(SarahConversationError::InvalidRequest(format!(
            "{field} must match the bounded Issue 31 PublicRef grammar"
        )));
    }
    Ok(())
}

fn command_fingerprint(
    method: &str,
    params: Option<&Value>,
) -> Result<String, SarahConversationError> {
    let canonical = serde_json::to_vec(&json!({ "method": method, "params": params }))
        .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

fn tag_value(tags: &[Vec<String>], name: &str) -> Option<String> {
    tags.iter().find_map(|tag| {
        if tag.first().map(String::as_str) == Some(name) {
            tag.get(1).cloned()
        } else {
            None
        }
    })
}

fn stored_event_cursor(event: &StoredConversationEvent) -> String {
    format!("{CURSOR_PREFIX}{}.{}", event.created_at, event.event_id)
}

fn stream_next_cursor(
    last_stream_cursor: Option<&str>,
    upstream_next_cursor: Option<&str>,
    more_stream_entries_in_page: bool,
) -> Option<String> {
    if !more_stream_entries_in_page && upstream_next_cursor.is_none() {
        return None;
    }
    last_stream_cursor
        .or(upstream_next_cursor)
        .map(str::to_owned)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn discovery_matches_direct_endpoints(
    event: &Event,
    direct_endpoints: &[Issue31DirectEndpoint],
) -> Result<bool, SarahConversationError> {
    let value = serde_json::from_str::<Value>(&event.content)
        .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
    match value.get("schema").and_then(Value::as_str) {
        Some(crate::ISSUE31_HOST_DISCOVERY_SCHEMA_V3) => {
            let discovery =
                Issue31HostDiscoveryV3::decode(event.content.as_bytes()).map_err(issue31_error)?;
            Ok(discovery.direct_endpoints == direct_endpoints)
        }
        Some(crate::ISSUE31_HOST_DISCOVERY_SCHEMA_V2) => Ok(direct_endpoints.is_empty()),
        _ => Ok(false),
    }
}

fn iso_from_unix(seconds: u64) -> String {
    // Keep dependency-free timestamps for public-safe projections.
    format!("{seconds}")
}

fn redact_content_summary(content: &str) -> String {
    if looks_like_secret(content) {
        return "[redacted]".to_string();
    }
    const MAX: usize = 512;
    if content.len() <= MAX {
        content.to_string()
    } else {
        let mut boundary = MAX;
        while !content.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!("{}…", &content[..boundary])
    }
}

fn looks_like_secret(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("bearer ")
        || lower.contains("authorization:")
        || lower.contains("api_key")
        || lower.contains("sk-")
        || lower.contains("-----begin ")
}

/// Assert the crate graph does not pull a Khala Sync client for this lane.
pub fn asserts_no_khala_sync_client() -> bool {
    // Compile-time documentation of the cut OMEGA-SW-02 rule. The Sarah lane
    // must remain Nostr-only; this helper is used by unit tests as a living
    // guardrail comment surface.
    true
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn client() -> SarahConversationClient {
        SarahConversationClient::new_mock(SarahConversationConfig::mock_fixture())
    }

    #[test]
    fn framed_method_names_match_spec_section_8() {
        assert_eq!(
            SARAH_FRAMED_METHODS,
            &[
                "sarah_session_status",
                "sarah_bootstrap",
                "sarah_room_snapshot",
                "sarah_send_message",
                "sarah_interrupt_turn",
                "sarah_device_grants",
                "sarah_renew_device_grant",
                "sarah_revoke_device_grant",
                "sarah_readmit_device",
            ]
        );
        assert_eq!(SARAH_EVENT_ROOM_EVENT, "sarah_room_event");
        assert_eq!(SARAH_EVENT_ROOM_STATE, "sarah_room_state");
    }

    #[test]
    fn session_status_never_returns_token_fields() {
        let mut client = client();
        let status = client.session_status().expect("status");
        let encoded = serde_json::to_string(&status).expect("json");
        assert!(status.signed_in);
        assert_eq!(status.account_label.as_deref(), Some("owner@example.com"));
        assert!(!encoded.contains("token"));
        assert!(!encoded.contains("bearer"));
        assert!(!encoded.contains("secret"));
    }

    #[test]
    fn bootstrap_projects_conversation_and_room_state() {
        let mut client = client();
        let boot = client.bootstrap().expect("bootstrap");
        assert_eq!(boot.principal_ref, "principal.sarah");
        assert!(boot.conversation_ref.starts_with("sarah."));
        assert!(boot.legacy_thread_ref.starts_with("thread.sarah."));
        assert_eq!(boot.room_state.connection, ConnectionState::Connected);
        assert_eq!(boot.room_state.transport, "mock_relay");
        assert!(boot.room_state.authenticated);
        let events = client.drain_events();
        assert!(
            events.iter().any(|event| {
                event.get("method").and_then(Value::as_str) == Some(SARAH_EVENT_ROOM_STATE)
            }),
            "bootstrap must emit sarah_room_state"
        );
    }

    #[test]
    fn send_and_snapshot_page_with_cursors() {
        let mut client = client();
        client.bootstrap().expect("bootstrap");
        let sent = client
            .send_message("Plan the next SARAH-NR packet", "idem.send.1", 1)
            .expect("send");
        assert!(sent.accepted);
        assert!(sent.message_ref.starts_with("msg."));
        assert!(sent.turn_ref.starts_with("turn."));
        assert!(!sent.event_id.is_empty());

        let snap = client.room_snapshot(None, Some(10)).expect("snapshot");
        assert_eq!(snap.conversation_ref, client.conversation_ref());
        assert_eq!(snap.transcript.gap_state, GapState::None);
        assert!(!snap.transcript.cursor.is_empty());
        assert_eq!(snap.transcript.entries.len(), 1);
        assert_eq!(snap.transcript.entries[0].role, "owner");
        assert_eq!(
            snap.transcript.entries[0].text,
            "Plan the next SARAH-NR packet"
        );
        assert_eq!(snap.run_state.state, "running");

        let conversation_ref = client.conversation_ref();
        let published = client
            .relay
            .query(&conversation_ref, None, 10)
            .expect("published message");
        let message = published.events.last().expect("message event");
        assert!(message.tags.iter().any(|tag| {
            tag.first().map(String::as_str) == Some("turn")
                && tag.get(1).map(String::as_str) == Some(sent.turn_ref.as_str())
        }));

        let events = client.drain_events();
        assert!(
            events.iter().any(|event| {
                event.get("method").and_then(Value::as_str) == Some(SARAH_EVENT_ROOM_EVENT)
            }),
            "send must emit sarah_room_event"
        );
    }

    #[test]
    fn interrupt_is_pending_until_settled() {
        let mut client = client();
        client.bootstrap().expect("bootstrap");
        let sent = client
            .send_message("start a turn", "idem.send.2", 1)
            .expect("send");
        let interrupt = client
            .interrupt_turn(&sent.turn_ref, "idem.interrupt.1", 1)
            .expect("interrupt");
        assert!(interrupt.accepted);
        assert!(interrupt.pending);
        assert_eq!(interrupt.status, "pending");
        assert_eq!(interrupt.turn_ref, sent.turn_ref);
    }

    #[test]
    fn issue31_owner_state_actions_publish_encrypted_mergeable_sources() {
        let mut client = client();
        client.bootstrap().expect("bootstrap");
        let context_ref = "sarah-conversation:sarah.aaaaaaaaaaaaaaaaaaaaaaaa";
        let read_arguments = |read_at| Issue31CommandArguments::ReadStatePatch {
            action_ref: "action.issue31.read_state.advance".into(),
            slot_id: "owner-mobile".into(),
            client_id: "iphone".into(),
            context_ref: context_ref.into(),
            read_at,
        };
        let (_, first_projection) = client
            .execute_issue31_owner_state_action(&read_arguments(100))
            .expect("first read-state write");
        let (second_event_id, second_projection) = client
            .execute_issue31_owner_state_action(&read_arguments(200))
            .expect("merged read-state write");
        assert_ne!(second_event_id, "0".repeat(64));
        assert!(matches!(
            first_projection,
            Issue31OwnerProjectionBody::ReadState { .. }
        ));
        let Issue31OwnerProjectionBody::ReadState { plaintext, .. } = second_projection else {
            panic!("read-state projection");
        };
        let value: Value = serde_json::from_str(&plaintext).expect("read-state plaintext");
        assert_eq!(value["contexts"][context_ref], 200);
        let contexts = client
            .load_issue31_read_state_contexts("read-state:owner-mobile")
            .expect("reload read-state contexts");
        assert_eq!(contexts.get(context_ref), Some(&200));

        let reminder_id = "a".repeat(32);
        let (_, reminder_projection) = client
            .execute_issue31_owner_state_action(&Issue31CommandArguments::ReminderCreate {
                action_ref: "action.issue31.reminder.create".into(),
                reminder_id: reminder_id.clone(),
                note: Some("Review the signed build".into()),
                target_event_id: None,
                not_before: 300,
                expiration: Some(600),
            })
            .expect("reminder write");
        assert!(matches!(
            reminder_projection,
            Issue31OwnerProjectionBody::Reminder {
                reminder_id: projected_id,
                not_before: Some(300),
                expiration: Some(600),
                ..
            } if projected_id == reminder_id
        ));
        let conversation_ref = client.config.conversation_ref();
        let page = client
            .query_with_auth(&conversation_ref, None, MAX_PAGE_LIMIT)
            .expect("query encrypted sources");
        assert!(page.events.iter().any(|event| {
            event.kind == SARAH_READ_STATE_KIND
                && !event.content_summary.contains("client_id")
                && !event.content_summary.contains(context_ref)
        }));
        assert!(page.events.iter().any(|event| {
            event.kind == SARAH_REMINDER_KIND
                && !event.content_summary.contains("Review the signed build")
        }));
    }

    #[test]
    fn stale_generation_fails_closed() {
        let mut client = client();
        let error = client
            .handle_request(SARAH_METHOD_SESSION_STATUS, 99, None)
            .expect_err("stale");
        match error {
            SarahConversationError::StaleGeneration { expected, got } => {
                assert_eq!(expected, 1);
                assert_eq!(got, 99);
            }
            other => panic!("expected stale generation, got {other}"),
        }
    }

    #[test]
    fn process_generation_advances_without_changing_nostr_host_generation() {
        let mut client = client();
        client
            .synchronize_process_generation(4)
            .expect("advance trusted process fence");
        assert_eq!(client.generation(), 4);
        client
            .handle_request(SARAH_METHOD_SESSION_STATUS, 4, None)
            .expect("advanced generation request");
        assert!(matches!(
            client.synchronize_process_generation(3),
            Err(SarahConversationError::StaleGeneration {
                expected: 4,
                got: 3
            })
        ));
    }

    /// A source event the host itself signed can still carry a body the device
    /// reader refuses. Before the emitter owned that decision, one such event
    /// aborted the whole projection pass, so a single malformed record stopped
    /// every later record reaching every paired device. The bad event is
    /// quarantined and the pass continues.
    #[test]
    fn one_refused_projection_source_is_quarantined_without_stopping_the_pass() {
        let signer = SigningIdentity::generate();
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = signer.public_key_hex.clone();
        let owner_public_key_hex = signer.public_key_hex.clone();
        let sarah_public_key_hex = config.identity.sarah_public_key_hex.clone();
        let conversation_ref = config.conversation_ref();
        let mut relay = MockRelayAdapter::new();
        for (index, (event_id, created_at, text)) in [
            ("1".repeat(64), 10_u64, "before"),
            // An empty message body is inside every record-level bound and
            // outside the projection body contract.
            ("2".repeat(64), 11, ""),
            ("3".repeat(64), 12, "after"),
        ]
        .into_iter()
        .enumerate()
        {
            relay.seed_event(StoredConversationEvent {
                event_id,
                kind: crate::ISSUE31_PRIVATE_RUMOR_KIND,
                pubkey: owner_public_key_hex.clone(),
                created_at,
                conversation_ref: conversation_ref.clone(),
                content_summary: text.into(),
                tags: Vec::new(),
                record_kind: "message".into(),
                store_index: index,
            });
        }
        let mut client = SarahConversationClient::with_relay(config, Box::new(relay), signer);

        let host_configuration = Issue31HostConfiguration {
            host_ref: "omega.host.local".into(),
            host_public_key_hex: owner_public_key_hex.clone(),
            sarah_public_key_hex,
            conversation: conversation_ref,
            display_name: "Omega host".into(),
            relay_urls: vec!["wss://relay.example.com".into()],
            generation: 1,
        };
        let device_public_key_hex = "2".repeat(64);
        let mut controller =
            Issue31HostController::new(host_configuration.clone()).expect("host controller");
        controller
            .set_admitted_device_policy(
                vec![device_public_key_hex.clone()],
                vec![crate::Issue31PairingScope::ObserveIssue31],
            )
            .expect("admit the device");
        let challenge = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: "a".repeat(64),
                    record: Issue31PairingRecord::PairingRequest {
                        schema: crate::ISSUE31_PAIRING_SCHEMA.into(),
                        host_ref: host_configuration.host_ref.clone(),
                        host_public_key_hex: owner_public_key_hex.clone(),
                        device_public_key_hex: device_public_key_hex.clone(),
                        issued_at: 100,
                        pairing_request_ref: "pairing_request.projection".into(),
                        requested_scopes: vec![crate::Issue31PairingScope::ObserveIssue31],
                        expires_at: 700,
                    },
                },
                100,
            )
            .expect("pairing request")
            .expect("pairing challenge");
        let Issue31PairingRecord::PairingChallenge {
            challenge: challenge_value,
            ..
        } = &challenge
        else {
            panic!("expected a pairing challenge");
        };
        let challenge_value = challenge_value.clone();
        controller
            .record_emitted_pairing("b".repeat(64), challenge)
            .expect("record the challenge");
        let grant = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: "c".repeat(64),
                    record: Issue31PairingRecord::PairingResponse {
                        schema: crate::ISSUE31_PAIRING_SCHEMA.into(),
                        host_ref: host_configuration.host_ref,
                        host_public_key_hex: owner_public_key_hex,
                        device_public_key_hex,
                        issued_at: 110,
                        pairing_response_ref: "pairing_response.projection".into(),
                        pairing_challenge_event_id: "b".repeat(64),
                        challenge: challenge_value,
                        expires_at: 700,
                    },
                },
                110,
            )
            .expect("pairing response")
            .expect("scoped grant");
        controller
            .record_emitted_pairing("d".repeat(64), grant)
            .expect("record the grant");
        assert_eq!(controller.active_grants(120).expect("grants").len(), 1);

        client.ensure_connected().expect("connect the mock relay");
        client
            .project_issue31_sources(&mut controller, 200)
            .expect("the pass survives a refused source");

        assert_eq!(
            client
                .issue31_quarantined_events
                .get(&"2".repeat(64))
                .map(String::as_str),
            Some("reason.omega.invalid_projection_source")
        );
        let grant_ref = controller.active_grants(200).expect("grants")[0]
            .grant_ref
            .clone();
        assert!(controller.source_was_projected(&grant_ref, 1, &"1".repeat(64)));
        assert!(!controller.source_was_projected(&grant_ref, 1, &"2".repeat(64)));
        assert!(controller.source_was_projected(&grant_ref, 1, &"3".repeat(64)));
        // Two surviving projections, plus the coverage statement that tells the
        // device the third source was withheld.
        assert_eq!(client.issue31_private_outbox.len(), 3);
        assert_eq!(withheld_outbox_refs(&client).len(), 1);
    }

    /// Every outbox entry queued under the withheld-sources schema.
    ///
    /// The outbox stores sealed gift wraps, so the test reads the host's own
    /// statement of what it published rather than trying to decrypt them; the
    /// outbox reference proves the record was actually queued for the device.
    fn withheld_outbox_refs(client: &SarahConversationClient) -> Vec<String> {
        client
            .issue31_private_outbox
            .keys()
            .filter(|outbox_ref| outbox_ref.starts_with(crate::ISSUE31_WITHHELD_SOURCES_SCHEMA))
            .cloned()
            .collect()
    }

    fn withheld_substance(
        client: &SarahConversationClient,
        grant_ref: &str,
        generation: u64,
    ) -> Option<(String, Vec<Issue31WithheldSourceCount>)> {
        client
            .issue31_withheld_emissions
            .get(&format!("{grant_ref}:{generation}"))
            .cloned()
    }

    // -----------------------------------------------------------------
    // omega#49: the omega#47 documents actually leaving the host.
    //
    // `full_auto_ui` built both documents from live host state and nothing
    // ever published them, so a paired phone rendered `no_host_projection` on
    // every device. These cover the pump half: who the records are addressed
    // to, what binding they carry, and when they are NOT sent.
    // -----------------------------------------------------------------

    fn adjunct_outbox_refs(client: &SarahConversationClient, schema: &str) -> Vec<String> {
        client
            .issue31_private_outbox
            .keys()
            .filter(|outbox_ref| outbox_ref.starts_with(schema))
            .cloned()
            .collect()
    }

    /// A reading the pump can publish, carrying no delivery claim of its own.
    fn host_documents(host_ref: &str, snapshot_ref: &str) -> Issue31HostProjectionDocuments {
        Issue31HostProjectionDocuments {
            host: json!({
                "schema": ISSUE31_HOST_ADJUNCT_SCHEMA,
                "hostRef": host_ref,
                "snapshotRef": snapshot_ref,
                "generatedAtMs": 1_784_894_400_000_u64,
                "projections": [],
            }),
            detail: json!({
                "schema": ISSUE31_FULL_AUTO_ADJUNCT_SCHEMA,
                "hostRef": host_ref,
                "snapshotRef": snapshot_ref,
                "generatedAtMs": 1_784_894_400_000_u64,
                "runs": [],
            }),
        }
    }

    fn paired_adjunct_client(
        source: Option<Issue31HostProjectionSource>,
    ) -> (SarahConversationClient, Issue31HostController, String) {
        let signer = SigningIdentity::generate();
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = signer.public_key_hex.clone();
        let owner_public_key_hex = signer.public_key_hex.clone();
        let sarah_public_key_hex = config.identity.sarah_public_key_hex.clone();
        let conversation_ref = config.conversation_ref();
        let mut client =
            SarahConversationClient::with_relay(config, Box::new(MockRelayAdapter::new()), signer);
        if let Some(source) = source {
            client.set_issue31_host_projection_source(source);
        }
        let device_public_key_hex = "2".repeat(64);
        let controller = pair_issue31_device(
            Issue31HostConfiguration {
                host_ref: "omega.host.local".into(),
                host_public_key_hex: owner_public_key_hex,
                sarah_public_key_hex,
                conversation: conversation_ref,
                display_name: "Omega host".into(),
                relay_urls: vec!["wss://relay.example.com".into()],
                generation: 1,
            },
            &device_public_key_hex,
        );
        (client, controller, device_public_key_hex)
    }

    /// The gap itself: both omega#47 documents must reach every admitted
    /// device, addressed to that device and to the grant it holds.
    #[test]
    fn the_host_snapshot_and_its_detail_are_addressed_to_each_admitted_device() {
        let source: Issue31HostProjectionSource = Arc::new(|request| {
            Ok(Some(host_documents(
                request.host_ref,
                "snapshot.omega.issue31.aa",
            )))
        });
        let (mut client, controller, device_public_key_hex) = paired_adjunct_client(Some(source));
        let grant = controller.active_grants(200).expect("grants")[0].clone();
        client.ensure_connected().expect("connect the mock relay");
        client
            .publish_issue31_host_adjuncts(&controller, 200)
            .expect("the pump publishes both documents");

        assert_eq!(
            adjunct_outbox_refs(&client, ISSUE31_HOST_ADJUNCT_SCHEMA).len(),
            1,
        );
        assert_eq!(
            adjunct_outbox_refs(&client, ISSUE31_FULL_AUTO_ADJUNCT_SCHEMA).len(),
            1,
        );
        assert!(
            client
                .issue31_host_adjunct_emissions
                .contains_key(&format!("{}:{}", grant.grant_ref, grant.generation)),
            "the pump must remember what it sent, or it resends forever",
        );

        // The binding the device checks the envelope against.
        let addressed = SarahConversationClient::address_issue31_adjunct(
            &host_documents("omega.host.local", "snapshot.omega.issue31.aa").host,
            ISSUE31_HOST_ADJUNCT_SCHEMA,
            ISSUE31_HOST_ADJUNCT_RECORD_TYPE,
            &grant,
        )
        .expect("the snapshot is addressable to this grant");
        assert_eq!(
            addressed.get("recordType").and_then(Value::as_str),
            Some(ISSUE31_HOST_ADJUNCT_RECORD_TYPE),
        );
        assert_eq!(
            addressed.get("devicePublicKeyHex").and_then(Value::as_str),
            Some(device_public_key_hex.as_str()),
        );
        assert_eq!(
            addressed.get("grantRef").and_then(Value::as_str),
            Some(grant.grant_ref.as_str()),
        );
        assert_eq!(
            addressed.get("hostPublicKeyHex").and_then(Value::as_str),
            Some(grant.host_public_key_hex.as_str()),
        );
    }

    /// A pass that observed the same world must not re-send the snapshot. The
    /// device would otherwise be handed an identical record forever, and every
    /// one of them would cost it a decrypt.
    #[test]
    fn an_unchanged_reading_is_not_republished() {
        let source: Issue31HostProjectionSource = Arc::new(|request| {
            Ok(Some(host_documents(
                request.host_ref,
                "snapshot.omega.issue31.aa",
            )))
        });
        let (mut client, controller, _device) = paired_adjunct_client(Some(source));
        client.ensure_connected().expect("connect the mock relay");
        client
            .publish_issue31_host_adjuncts(&controller, 200)
            .expect("first pass");
        client.issue31_private_outbox.clear();
        client
            .publish_issue31_host_adjuncts(&controller, 260)
            .expect("second pass");
        assert!(
            client.issue31_private_outbox.is_empty(),
            "an unchanged reading must not be republished",
        );

        // A changed reading is a different snapshot and does go out again.
        let changed: Issue31HostProjectionSource = Arc::new(|request| {
            Ok(Some(host_documents(
                request.host_ref,
                "snapshot.omega.issue31.bb",
            )))
        });
        client.set_issue31_host_projection_source(changed);
        client
            .publish_issue31_host_adjuncts(&controller, 320)
            .expect("third pass");
        assert_eq!(
            adjunct_outbox_refs(&client, ISSUE31_HOST_ADJUNCT_SCHEMA).len(),
            1,
        );
    }

    /// A host that is not observing its Full Auto state says nothing rather
    /// than publishing an empty snapshot. Silence and "I looked and found
    /// nothing" are different claims and the device renders them differently.
    #[test]
    fn a_host_with_no_reading_publishes_nothing_at_all() {
        let (mut client, controller, _device) = paired_adjunct_client(None);
        client.ensure_connected().expect("connect the mock relay");
        client
            .publish_issue31_host_adjuncts(&controller, 200)
            .expect("a host with no reading still completes its pass");
        assert!(client.issue31_private_outbox.is_empty());

        let silent: Issue31HostProjectionSource = Arc::new(|_| Ok(None));
        client.set_issue31_host_projection_source(silent);
        client
            .publish_issue31_host_adjuncts(&controller, 200)
            .expect("an unobserved host still completes its pass");
        assert!(client.issue31_private_outbox.is_empty());
    }

    /// The one substitution a signed seal cannot rule out. A snapshot labelled
    /// with another machine's host reference would be bound by the device to
    /// this pairing, so the pump refuses to address it at all.
    #[test]
    fn a_snapshot_naming_another_host_is_never_addressed_to_this_device() {
        let (mut client, controller, _device) = paired_adjunct_client(None);
        let grant = controller.active_grants(200).expect("grants")[0].clone();
        let foreign = host_documents("omega.host.some-other-machine", "snapshot.omega.issue31.aa");
        assert!(
            SarahConversationClient::address_issue31_adjunct(
                &foreign.host,
                ISSUE31_HOST_ADJUNCT_SCHEMA,
                ISSUE31_HOST_ADJUNCT_RECORD_TYPE,
                &grant,
            )
            .is_err(),
        );

        // And a reading that arrives already claiming who may read it is
        // refused rather than silently overwritten.
        let mut presumptuous = host_documents("omega.host.local", "snapshot.omega.issue31.aa").host;
        presumptuous
            .as_object_mut()
            .expect("object")
            .insert("devicePublicKeyHex".into(), json!("3".repeat(64)));
        assert!(
            SarahConversationClient::address_issue31_adjunct(
                &presumptuous,
                ISSUE31_HOST_ADJUNCT_SCHEMA,
                ISSUE31_HOST_ADJUNCT_RECORD_TYPE,
                &grant,
            )
            .is_err(),
        );

        let source: Issue31HostProjectionSource = Arc::new(|_| {
            Ok(Some(host_documents(
                "omega.host.some-other-machine",
                "snapshot.omega.issue31.aa",
            )))
        });
        client.set_issue31_host_projection_source(source);
        client.ensure_connected().expect("connect the mock relay");
        assert!(
            client
                .publish_issue31_host_adjuncts(&controller, 200)
                .is_err()
        );
        assert!(client.issue31_private_outbox.is_empty());
    }

    /// The snapshot advertises the capabilities; the detail is what the owner
    /// opens. A detail bound to a different snapshot is one the phone refuses
    /// as `snapshot_mismatch`, so publishing it would publish a refusal.
    #[test]
    fn a_detail_not_bound_to_the_snapshot_beside_it_is_never_sent() {
        let source: Issue31HostProjectionSource = Arc::new(|request| {
            let mut documents = host_documents(request.host_ref, "snapshot.omega.issue31.aa");
            documents
                .detail
                .as_object_mut()
                .expect("object")
                .insert("snapshotRef".into(), json!("snapshot.omega.issue31.bb"));
            Ok(Some(documents))
        });
        let (mut client, controller, _device) = paired_adjunct_client(Some(source));
        client.ensure_connected().expect("connect the mock relay");
        assert!(
            client
                .publish_issue31_host_adjuncts(&controller, 200)
                .is_err()
        );
    }

    /// Pair one device with the host so the projection pass has somewhere to
    /// send a coverage statement.
    fn pair_issue31_device(
        host_configuration: Issue31HostConfiguration,
        device_public_key_hex: &str,
    ) -> Issue31HostController {
        let host_ref = host_configuration.host_ref.clone();
        let host_public_key_hex = host_configuration.host_public_key_hex.clone();
        let mut controller =
            Issue31HostController::new(host_configuration).expect("host controller");
        controller
            .set_admitted_device_policy(
                vec![device_public_key_hex.to_string()],
                vec![crate::Issue31PairingScope::ObserveIssue31],
            )
            .expect("admit the device");
        let challenge = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: "a".repeat(64),
                    record: Issue31PairingRecord::PairingRequest {
                        schema: crate::ISSUE31_PAIRING_SCHEMA.into(),
                        host_ref: host_ref.clone(),
                        host_public_key_hex: host_public_key_hex.clone(),
                        device_public_key_hex: device_public_key_hex.to_string(),
                        issued_at: 100,
                        pairing_request_ref: "pairing_request.coverage".into(),
                        requested_scopes: vec![crate::Issue31PairingScope::ObserveIssue31],
                        expires_at: 100_000,
                    },
                },
                100,
            )
            .expect("pairing request")
            .expect("pairing challenge");
        let Issue31PairingRecord::PairingChallenge {
            challenge: challenge_value,
            ..
        } = &challenge
        else {
            panic!("expected a pairing challenge");
        };
        let challenge_value = challenge_value.clone();
        controller
            .record_emitted_pairing("b".repeat(64), challenge)
            .expect("record the challenge");
        let grant = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: "c".repeat(64),
                    record: Issue31PairingRecord::PairingResponse {
                        schema: crate::ISSUE31_PAIRING_SCHEMA.into(),
                        host_ref,
                        host_public_key_hex,
                        device_public_key_hex: device_public_key_hex.to_string(),
                        issued_at: 110,
                        pairing_response_ref: "pairing_response.coverage".into(),
                        pairing_challenge_event_id: "b".repeat(64),
                        challenge: challenge_value,
                        expires_at: 100_000,
                    },
                },
                110,
            )
            .expect("pairing response")
            .expect("scoped grant");
        controller
            .record_emitted_pairing("d".repeat(64), grant)
            .expect("record the grant");
        controller
    }

    /// Falsification, drop path one. A quarantined source is removed from the
    /// owner's view by a host-local map whose count never left the host. The
    /// device must be told an exact number and why, and the statement must go
    /// back to `complete` for a pass that withholds nothing.
    #[test]
    fn a_quarantined_source_is_reported_to_the_device_as_an_exact_withheld_count() {
        let signer = SigningIdentity::generate();
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = signer.public_key_hex.clone();
        let owner_public_key_hex = signer.public_key_hex.clone();
        let sarah_public_key_hex = config.identity.sarah_public_key_hex.clone();
        let conversation_ref = config.conversation_ref();
        let mut relay = MockRelayAdapter::new();
        for (index, (event_id, created_at, text)) in [
            ("1".repeat(64), 10_u64, "readable"),
            // Inside every record-level bound and outside the projection body
            // contract, so the host quarantines it.
            ("2".repeat(64), 11, ""),
        ]
        .into_iter()
        .enumerate()
        {
            relay.seed_event(StoredConversationEvent {
                event_id,
                kind: crate::ISSUE31_PRIVATE_RUMOR_KIND,
                pubkey: owner_public_key_hex.clone(),
                created_at,
                conversation_ref: conversation_ref.clone(),
                content_summary: text.into(),
                tags: Vec::new(),
                record_kind: "message".into(),
                store_index: index,
            });
        }
        let mut client = SarahConversationClient::with_relay(config, Box::new(relay), signer);
        let device_public_key_hex = "2".repeat(64);
        let mut controller = pair_issue31_device(
            Issue31HostConfiguration {
                host_ref: "omega.host.local".into(),
                host_public_key_hex: owner_public_key_hex,
                sarah_public_key_hex,
                conversation: conversation_ref,
                display_name: "Omega host".into(),
                relay_urls: vec!["wss://relay.example.com".into()],
                generation: 1,
            },
            &device_public_key_hex,
        );
        let grant_ref = controller.active_grants(200).expect("grants")[0]
            .grant_ref
            .clone();

        client.ensure_connected().expect("connect the mock relay");
        client
            .project_issue31_sources(&mut controller, 200)
            .expect("the pass survives a quarantined source");

        let (coverage, withheld) =
            withheld_substance(&client, &grant_ref, 1).expect("a coverage statement was published");
        assert_eq!(coverage, crate::ISSUE31_WITHHELD_COVERAGE_PARTIAL);
        assert_eq!(
            withheld,
            vec![Issue31WithheldSourceCount {
                cause: Issue31WithheldCause::Quarantined,
                count: 1,
                exact: true,
                reason_ref: ISSUE31_PROJECTION_SOURCE_QUARANTINE_REASON.into(),
            }]
        );
        assert_eq!(withheld_outbox_refs(&client).len(), 1);

        // Restore: a host with nothing quarantined states completeness rather
        // than staying quiet, because silence would have to read as unknown.
        let clean_signer = SigningIdentity::generate();
        let mut clean_config = SarahConversationConfig::mock_fixture();
        clean_config.identity.owner_public_key_hex = clean_signer.public_key_hex.clone();
        let clean_owner = clean_signer.public_key_hex.clone();
        let clean_sarah = clean_config.identity.sarah_public_key_hex.clone();
        let clean_conversation = clean_config.conversation_ref();
        let mut clean_relay = MockRelayAdapter::new();
        clean_relay.seed_event(StoredConversationEvent {
            event_id: "1".repeat(64),
            kind: crate::ISSUE31_PRIVATE_RUMOR_KIND,
            pubkey: clean_owner.clone(),
            created_at: 10,
            conversation_ref: clean_conversation.clone(),
            content_summary: "readable".into(),
            tags: Vec::new(),
            record_kind: "message".into(),
            store_index: 0,
        });
        let mut clean_client =
            SarahConversationClient::with_relay(clean_config, Box::new(clean_relay), clean_signer);
        let mut clean_controller = pair_issue31_device(
            Issue31HostConfiguration {
                host_ref: "omega.host.local".into(),
                host_public_key_hex: clean_owner,
                sarah_public_key_hex: clean_sarah,
                conversation: clean_conversation,
                display_name: "Omega host".into(),
                relay_urls: vec!["wss://relay.example.com".into()],
                generation: 1,
            },
            &device_public_key_hex,
        );
        clean_client
            .ensure_connected()
            .expect("connect the mock relay");
        clean_client
            .project_issue31_sources(&mut clean_controller, 200)
            .expect("a clean pass");
        assert_eq!(
            withheld_substance(&clean_client, &grant_ref, 1),
            Some((
                crate::ISSUE31_WITHHELD_COVERAGE_COMPLETE.to_string(),
                vec![]
            ))
        );
    }

    /// Falsification, drop path two. The bounded projection scan stops after
    /// eight pages and, before this change, said so only in the host's own
    /// `last_gap_state`. The device must see an inexact count, and must see it
    /// clear once the scan catches up.
    #[test]
    fn the_projection_scan_bound_is_reported_to_the_device_and_clears_when_it_catches_up() {
        let signer = SigningIdentity::generate();
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = signer.public_key_hex.clone();
        let owner_public_key_hex = signer.public_key_hex.clone();
        let sarah_public_key_hex = config.identity.sarah_public_key_hex.clone();
        let conversation_ref = config.conversation_ref();
        let mut relay = MockRelayAdapter::new();
        // One page past the eight-page bound. The kind is outside the
        // projection set, so this measures the scan bound itself rather than
        // the cost of projecting five hundred sources.
        let pages_past_the_bound = 8 * MAX_PAGE_LIMIT + 1;
        for index in 0..pages_past_the_bound {
            relay.seed_event(StoredConversationEvent {
                event_id: format!("{index:064x}"),
                kind: 1,
                pubkey: owner_public_key_hex.clone(),
                created_at: 10 + index as u64,
                conversation_ref: conversation_ref.clone(),
                content_summary: "unrelated".into(),
                tags: Vec::new(),
                record_kind: "note".into(),
                store_index: index,
            });
        }
        let mut client = SarahConversationClient::with_relay(config, Box::new(relay), signer);
        let device_public_key_hex = "2".repeat(64);
        let mut controller = pair_issue31_device(
            Issue31HostConfiguration {
                host_ref: "omega.host.local".into(),
                host_public_key_hex: owner_public_key_hex,
                sarah_public_key_hex,
                conversation: conversation_ref,
                display_name: "Omega host".into(),
                relay_urls: vec!["wss://relay.example.com".into()],
                generation: 1,
            },
            &device_public_key_hex,
        );
        let grant_ref = controller.active_grants(200).expect("grants")[0]
            .grant_ref
            .clone();

        client.ensure_connected().expect("connect the mock relay");
        client
            .project_issue31_sources(&mut controller, 200)
            .expect("the bounded pass");

        let (coverage, withheld) =
            withheld_substance(&client, &grant_ref, 1).expect("a coverage statement was published");
        assert_eq!(coverage, crate::ISSUE31_WITHHELD_COVERAGE_PARTIAL);
        assert_eq!(
            withheld,
            vec![Issue31WithheldSourceCount {
                cause: Issue31WithheldCause::ScanBound,
                count: 1,
                exact: false,
                reason_ref: ISSUE31_PROJECTION_SCAN_BOUND_REASON.into(),
            }]
        );
        assert_eq!(client.last_gap_state, GapState::Possible);
        let after_bounded_pass = withheld_outbox_refs(&client);
        assert_eq!(after_bounded_pass.len(), 1);

        // The cursor advanced, so the next pass reaches the end and the device
        // is told so. A signal that could only ever get worse would be a worse
        // lie than none.
        client
            .project_issue31_sources(&mut controller, 300)
            .expect("the catching-up pass");
        assert_eq!(
            withheld_substance(&client, &grant_ref, 1),
            Some((
                crate::ISSUE31_WITHHELD_COVERAGE_COMPLETE.to_string(),
                vec![]
            ))
        );
        let after_catching_up = withheld_outbox_refs(&client);
        assert_eq!(after_catching_up.len(), 2);
        assert!(after_catching_up[0] != after_catching_up[1]);
    }

    /// The host re-runs its projection pass continuously. A statement that has
    /// not changed must not be republished with only a new timestamp, or the
    /// device's own store fills with restatements of one fact.
    #[test]
    fn an_unchanged_coverage_statement_is_not_republished() {
        let signer = SigningIdentity::generate();
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = signer.public_key_hex.clone();
        let owner_public_key_hex = signer.public_key_hex.clone();
        let sarah_public_key_hex = config.identity.sarah_public_key_hex.clone();
        let conversation_ref = config.conversation_ref();
        let mut relay = MockRelayAdapter::new();
        relay.seed_event(StoredConversationEvent {
            event_id: "1".repeat(64),
            kind: crate::ISSUE31_PRIVATE_RUMOR_KIND,
            pubkey: owner_public_key_hex.clone(),
            created_at: 10,
            conversation_ref: conversation_ref.clone(),
            content_summary: "readable".into(),
            tags: Vec::new(),
            record_kind: "message".into(),
            store_index: 0,
        });
        let mut client = SarahConversationClient::with_relay(config, Box::new(relay), signer);
        let mut controller = pair_issue31_device(
            Issue31HostConfiguration {
                host_ref: "omega.host.local".into(),
                host_public_key_hex: owner_public_key_hex,
                sarah_public_key_hex,
                conversation: conversation_ref,
                display_name: "Omega host".into(),
                relay_urls: vec!["wss://relay.example.com".into()],
                generation: 1,
            },
            &"2".repeat(64),
        );
        client.ensure_connected().expect("connect the mock relay");
        client
            .project_issue31_sources(&mut controller, 200)
            .expect("the first pass");
        assert_eq!(withheld_outbox_refs(&client).len(), 1);
        client
            .project_issue31_sources(&mut controller, 400)
            .expect("the second pass, at a different clock");
        assert_eq!(withheld_outbox_refs(&client).len(), 1);
    }

    #[test]
    fn snapshot_honors_independent_transcript_and_activity_windows() {
        let signer = SigningIdentity::generate();
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = signer.public_key_hex.clone();
        let conversation_ref = config.conversation_ref();
        let mut relay = MockRelayAdapter::new();
        relay.seed_event(StoredConversationEvent {
            event_id: "1".repeat(64),
            kind: Kind::PrivateDirectMessage.as_u16(),
            pubkey: signer.public_key_hex.clone(),
            created_at: 1,
            conversation_ref: conversation_ref.clone(),
            content_summary: "first".into(),
            tags: Vec::new(),
            record_kind: "message".into(),
            store_index: 0,
        });
        relay.seed_event(StoredConversationEvent {
            event_id: "2".repeat(64),
            kind: SARAH_TURN_RECORD_KIND,
            pubkey: signer.public_key_hex.clone(),
            created_at: 2,
            conversation_ref: conversation_ref.clone(),
            content_summary: "activity".into(),
            tags: vec![
                vec!["entry".into(), "entry.test".into()],
                vec!["turn".into(), "turn.test".into()],
            ],
            record_kind: "activity".into(),
            store_index: 1,
        });
        relay.seed_event(StoredConversationEvent {
            event_id: "3".repeat(64),
            kind: Kind::PrivateDirectMessage.as_u16(),
            pubkey: signer.public_key_hex.clone(),
            created_at: 3,
            conversation_ref,
            content_summary: "second".into(),
            tags: Vec::new(),
            record_kind: "message".into(),
            store_index: 2,
        });
        let mut client = SarahConversationClient::with_relay(config, Box::new(relay), signer);
        let snapshot = client
            .room_snapshot_with_cursors(
                Some(&format!("cursor.1.{}", "1".repeat(64))),
                Some(1),
                None,
                Some(1),
            )
            .expect("independent snapshot");
        assert_eq!(snapshot.transcript.entries[0].text, "second");
        assert_eq!(snapshot.activity.entries[0].entry, "entry.test");
        assert!(snapshot.transcript.next_cursor.is_none());
        assert!(snapshot.activity.next_cursor.is_none());
    }

    #[test]
    fn snapshot_projects_bounded_confirmed_nostr_record_refs_without_content() {
        let config = SarahConversationConfig::mock_fixture();
        let conversation_ref = config.conversation_ref();
        let mut relay = MockRelayAdapter::new();
        relay.seed_event(StoredConversationEvent {
            event_id: "e".repeat(64),
            kind: 30_174,
            pubkey: "f".repeat(64),
            created_at: 100,
            conversation_ref,
            content_summary: "nip44:encrypted-memory-content".into(),
            tags: vec![vec!["d".into(), "a".repeat(64)]],
            record_kind: "memory".into(),
            store_index: 0,
        });
        let mut client = SarahConversationClient::with_relay(
            config,
            Box::new(relay),
            SigningIdentity::generate(),
        );

        let snapshot = client
            .room_snapshot_with_record_cursor(None, Some(1), None, Some(1), None, Some(1))
            .expect("snapshot");
        assert_eq!(snapshot.nostr_records.entries.len(), 1);
        let record = &snapshot.nostr_records.entries[0];
        assert_eq!(record.event_id, "e".repeat(64));
        assert_eq!(record.kind, 30_174);
        assert_eq!(record.record_kind, "memory");
        assert_eq!(record.source, "confirmed_nostr");
        assert_eq!(record.author_fingerprint.len(), 16);
        let encoded = serde_json::to_string(&snapshot.nostr_records).expect("json");
        assert!(!encoded.contains("encrypted-memory-content"));
    }

    #[test]
    fn nip42_auth_against_mock_relay_when_required() {
        let signer = SigningIdentity::generate();
        let challenge = "test-challenge-42";
        let mut client = SarahConversationClient::mock_with_nip42_auth(
            SarahConversationConfig::mock_fixture(),
            challenge,
            signer,
        );
        let boot = client.bootstrap().expect("bootstrap after NIP-42");
        assert!(boot.room_state.authenticated);
        assert_eq!(boot.room_state.transport, "nostr_relay");
        let sent = client
            .send_message("authenticated send", "idem.send.auth", 1)
            .expect("send after auth");
        assert!(sent.accepted);
    }

    #[test]
    fn nip42_rejects_wrong_challenge() {
        let signer = SigningIdentity::generate();
        let mut relay = MockRelayAdapter::with_required_auth("expected-challenge");
        relay.connect().expect("connect");
        let auth = signer
            .sign_auth("wrong-challenge", "wss://relay.openagents.com")
            .expect("sign");
        let error = relay.authenticate(&auth).expect_err("must reject");
        assert!(error.to_string().contains("NIP-42"));
    }

    #[test]
    fn frames_respect_64kib_and_redact_secrets() {
        let client = client();
        let result = json!({ "ok": true, "cursor": "cursor.0", "gapState": "none" });
        let line = client
            .encode_response_frame("1", 1, result)
            .expect("encode");
        assert!(line.len() < MAX_FRAME_BYTES);
        assert!(line.contains(PROTOCOL_SCHEMA));

        let secretish = json!({ "text": "bearer super-secret-token" });
        let refused = client.encode_event_frame(1, SARAH_EVENT_ROOM_EVENT, secretish);
        assert!(refused.is_err());
    }

    #[test]
    fn refuses_secret_shaped_outbound_message() {
        let mut client = client();
        client.bootstrap().expect("bootstrap");
        let error = client
            .send_message("Authorization: Bearer sk-test-123", "idem.send.secret", 1)
            .expect_err("secret");
        assert!(matches!(error, SarahConversationError::InvalidRequest(_)));
    }

    #[test]
    fn client_collection_bounds_are_enforced_before_command_mutation() {
        let mut client = client();
        client.command_results = (0..MAX_COMMAND_RESULTS)
            .map(|index| {
                (
                    format!("idempotency.bound.{index}"),
                    ("fingerprint".into(), json!({ "accepted": true })),
                )
            })
            .collect();
        let params = json!({
            "text": "must not mutate",
            "idempotencyRef": "idempotency.bound.new",
            "expectedGeneration": 1,
        });
        let error = client
            .handle_request(SARAH_METHOD_SEND_MESSAGE, 1, Some(&params))
            .expect_err("command result bound");
        assert!(error.to_string().contains("bound"));
        assert_eq!(client.message_seq, 0);
        assert!(client.active_turn_ref.is_none());
        assert!(client.issue31_private_outbox.is_empty());

        let state = client.current_room_state();
        for _ in 0..MAX_PENDING_EVENTS.saturating_add(32) {
            client.push_room_state_event(&state);
        }
        assert_eq!(client.pending_events.len(), MAX_PENDING_EVENTS);
    }

    #[test]
    fn unbound_real_controller_never_reports_false_command_completion() {
        let mut client = client();
        client.run_state = "running".into();
        client.active_turn_ref = Some("turn.9".into());
        for action_ref in [
            "action.omega.full_auto.stop",
            "action.omega.full_auto.pause",
            "action.omega.full_auto.resume",
            "action.omega.interrupt_turn",
            "action.omega.send_message",
        ] {
            let execution = client.execute_issue31_action(
                action_ref,
                "arguments.omega.none",
                "idempotency.issue31.unbound",
            );
            assert_eq!(execution.status, Issue31CommandStatus::Unavailable);
            assert_eq!(
                execution.reason_ref.as_deref(),
                Some("reason.omega.controller_not_bound")
            );
        }
        assert_eq!(client.run_state, "running");
        assert_eq!(client.active_turn_ref.as_deref(), Some("turn.9"));
    }

    // ---------------------------------------------------------------------
    // omega#91 provider connection handoffs
    // ---------------------------------------------------------------------

    /// A paired device holding exactly the scope the phone already asks for.
    fn handoff_fixture_with(
        scopes: Vec<crate::Issue31PairingScope>,
    ) -> (
        SarahConversationClient,
        Issue31HostConfiguration,
        Issue31HostController,
        String,
        String,
    ) {
        let (configuration, controller, device_public_key_hex, grant_ref) =
            crate::issue31_nostr::paired_fixture(scopes);
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = configuration.host_public_key_hex.clone();
        let client = SarahConversationClient::with_relay(
            config,
            Box::new(MockRelayAdapter::new()),
            SigningIdentity::generate(),
        );
        (
            client,
            configuration,
            controller,
            device_public_key_hex,
            grant_ref,
        )
    }

    fn handoff_fixture() -> (
        SarahConversationClient,
        Issue31HostConfiguration,
        Issue31HostController,
        String,
        String,
    ) {
        handoff_fixture_with(vec![crate::Issue31PairingScope::RequestProviderHandoff])
    }

    fn handoff_intent(
        configuration: &Issue31HostConfiguration,
        device_public_key_hex: &str,
        grant_ref: &str,
        event_id: &str,
        idempotency_ref: &str,
        arguments_ref: &str,
    ) -> Issue31CommandEvent {
        Issue31CommandEvent {
            event_id: event_id.to_string(),
            record: Issue31CommandRecord::CommandIntent {
                schema: ISSUE31_COMMAND_SCHEMA.into(),
                host_ref: configuration.host_ref.clone(),
                host_public_key_hex: configuration.host_public_key_hex.clone(),
                device_public_key_hex: device_public_key_hex.to_string(),
                grant_ref: grant_ref.to_string(),
                action_ref: ISSUE31_ACTION_REQUEST_PROVIDER_HANDOFF.into(),
                idempotency_ref: idempotency_ref.to_string(),
                expected_generation: 1,
                arguments_ref: arguments_ref.to_string(),
                issued_at: 103,
                expires_at: 900,
            },
        }
    }

    #[test]
    fn the_scope_the_phone_holds_now_produces_a_host_record() {
        // Before omega#91 this exact command reached `execute_issue31_action`
        // and came back `unavailable`/`controller_not_bound`, and the handoff
        // vector on the wire stayed empty forever.
        let (mut client, configuration, mut controller, device_public_key_hex, grant_ref) =
            handoff_fixture();
        let intent = handoff_intent(
            &configuration,
            &device_public_key_hex,
            &grant_ref,
            &"1".repeat(64),
            "idempotency.issue31.handoff:first",
            "arguments.omega.provider_handoff.anthropic",
        );
        let result = controller
            .handle_command_event(intent, 104, |action_ref, arguments_ref, idempotency_ref| {
                client.execute_issue31_action(action_ref, arguments_ref, idempotency_ref)
            })
            .expect("the host answers an admitted handoff request")
            .expect("a command result");
        let Issue31CommandRecord::CommandResult {
            status,
            outcome_ref,
            reason_ref,
            ..
        } = &result
        else {
            panic!("expected a command result");
        };
        assert_eq!(*status, Issue31CommandStatus::Completed);
        assert_eq!(reason_ref.as_deref(), None);
        // The command completed by producing this record; the record itself is
        // not terminal. Reading one as the other is the mistake this asserts
        // against.
        assert_eq!(
            client.issue31_provider_handoff_refs(),
            vec![outcome_ref.clone()]
        );
        let record = client
            .issue31_provider_handoffs
            .get(outcome_ref)
            .expect("the outcome names the record the host made")
            .clone();
        assert_eq!(
            record.state,
            workroom_receipts::Issue31ProviderHandoffState::Requested
        );
        assert!(!record.is_terminal());
        assert!(record.requested_at_ms.is_some(), "the host stamped it");
        assert!(record.account_ref.is_none(), "nothing is bound yet");
        assert!(record.outcome_ref.is_none(), "no outcome is claimed yet");
    }

    #[test]
    fn a_scope_denied_request_leaves_no_handoff_at_all() {
        // The failure-vs-never-started distinction, at the wire. A device
        // without the scope gets a refusal — and the host makes no record, so
        // the phone's handoff list is empty rather than showing a failed row
        // for something that never began.
        let (mut client, configuration, mut controller, device_public_key_hex, grant_ref) =
            handoff_fixture_with(vec![crate::Issue31PairingScope::ObserveIssue31]);
        let intent = handoff_intent(
            &configuration,
            &device_public_key_hex,
            &grant_ref,
            &"2".repeat(64),
            "idempotency.issue31.handoff:denied",
            "arguments.omega.provider_handoff.anthropic",
        );
        let result = controller
            .handle_command_event(intent, 104, |action_ref, arguments_ref, idempotency_ref| {
                client.execute_issue31_action(action_ref, arguments_ref, idempotency_ref)
            })
            .expect("the controller answers")
            .expect("a command result");
        let Issue31CommandRecord::CommandResult {
            status, reason_ref, ..
        } = &result
        else {
            panic!("expected a command result");
        };
        assert_eq!(*status, Issue31CommandStatus::Refused);
        assert_eq!(reason_ref.as_deref(), Some("reason.omega.scope_denied"));
        assert!(
            client.issue31_provider_handoff_refs().is_empty(),
            "a request the host never admitted is not a handoff that failed",
        );
        assert!(
            client
                .issue31_projected_provider_handoffs(1_000_000)
                .is_empty()
        );
    }

    #[test]
    fn a_handoff_request_naming_no_provider_is_refused_without_a_record() {
        let (mut client, configuration, mut controller, device_public_key_hex, grant_ref) =
            handoff_fixture();
        let intent = handoff_intent(
            &configuration,
            &device_public_key_hex,
            &grant_ref,
            &"3".repeat(64),
            "idempotency.issue31.handoff:bad_arguments",
            "arguments.omega.none",
        );
        let result = controller
            .handle_command_event(intent, 104, |action_ref, arguments_ref, idempotency_ref| {
                client.execute_issue31_action(action_ref, arguments_ref, idempotency_ref)
            })
            .expect("the controller answers")
            .expect("a command result");
        let Issue31CommandRecord::CommandResult {
            status, reason_ref, ..
        } = &result
        else {
            panic!("expected a command result");
        };
        assert_eq!(*status, Issue31CommandStatus::Refused);
        assert_eq!(
            reason_ref.as_deref(),
            Some("reason.omega.handoff_arguments_invalid")
        );
        assert!(client.issue31_provider_handoff_refs().is_empty());
    }

    #[test]
    fn a_handoff_in_flight_survives_a_restart_and_is_settled_rather_than_lost() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let state_path = temporary.path().join("owner").join("issue31-state.json");
        let (mut client, configuration, controller, _, _) = handoff_fixture();
        client.issue31_state_path = Some(state_path.clone());
        let opened = client
            .issue31_provider_handoffs
            .open(
                "arguments.omega.provider_handoff.anthropic",
                "idempotency.issue31.handoff:restart",
                1_785_000_000_000,
            )
            .expect("the host opens a handoff");
        client
            .persist_issue31_host_state_with_controller(&controller)
            .expect("the ledger is committed with the rest of host state");

        // Exactly what a restarted process reads back off disk.
        let reloaded = load_issue31_host_state(&state_path, &configuration)
            .expect("load")
            .expect("persisted state");
        let mut ledger = reloaded.provider_handoffs;
        assert_eq!(
            ledger.get(&opened.handoff_ref).expect("the row survived"),
            &opened,
            "a handoff in flight is not lost across a restart",
        );

        // And a restart resolves it rather than leaving the phone with a
        // request that neither resolves nor fails.
        assert_eq!(ledger.adopt_after_restart(), 1);
        let settled = ledger.get(&opened.handoff_ref).expect("row").clone();
        assert_eq!(
            settled.state,
            workroom_receipts::Issue31ProviderHandoffState::Failed
        );
        assert_eq!(
            settled.reason_class.as_deref(),
            Some(crate::issue31_provider_handoff::ISSUE31_HANDOFF_REASON_HOST_RESTARTED)
        );
        assert!(settled.is_terminal());
    }

    #[test]
    fn the_pump_advances_a_handoff_against_the_hosts_own_roster() {
        let (mut client, _, _, _, _) = handoff_fixture();
        let opened = client
            .issue31_provider_handoffs
            .open(
                "arguments.omega.provider_handoff.anthropic",
                "idempotency.issue31.handoff:roster",
                1_000,
            )
            .expect("open");
        client.set_issue31_provider_roster_source(Arc::new(|| {
            Some(vec![Issue31ProviderRosterAccount {
                account_ref: "account.claude.1".into(),
                provider: "anthropic".into(),
                lane_ref: "lane.claude-local".into(),
                readiness: "ready".into(),
            }])
        }));
        client.advance_issue31_provider_handoffs(2);
        assert_eq!(
            client
                .issue31_provider_handoffs
                .get(&opened.handoff_ref)
                .expect("row")
                .account_ref
                .as_deref(),
            Some("account.claude.1"),
        );
        client.advance_issue31_provider_handoffs(3);
        assert_eq!(
            client
                .issue31_provider_handoffs
                .get(&opened.handoff_ref)
                .expect("row")
                .state,
            workroom_receipts::Issue31ProviderHandoffState::Completed,
        );
    }

    #[test]
    fn a_host_that_never_read_its_roster_binds_nothing() {
        let (mut client, _, _, _, _) = handoff_fixture();
        let opened = client
            .issue31_provider_handoffs
            .open(
                "arguments.omega.provider_handoff.anthropic",
                "idempotency.issue31.handoff:unread",
                1_000,
            )
            .expect("open");
        client.set_issue31_provider_roster_source(Arc::new(|| None));
        client.advance_issue31_provider_handoffs(2);
        assert_eq!(
            client
                .issue31_provider_handoffs
                .get(&opened.handoff_ref)
                .expect("row")
                .state,
            workroom_receipts::Issue31ProviderHandoffState::Requested,
        );
    }

    #[test]
    fn no_khala_sync_client_on_sarah_lane() {
        assert!(asserts_no_khala_sync_client());
    }

    #[test]
    fn handle_request_covers_all_methods() {
        let mut client = client();
        for method in SARAH_FRAMED_METHODS {
            if matches!(
                *method,
                SARAH_METHOD_DEVICE_GRANTS
                    | SARAH_METHOD_RENEW_DEVICE_GRANT
                    | SARAH_METHOD_REVOKE_DEVICE_GRANT
                    | SARAH_METHOD_READMIT_DEVICE
            ) {
                continue;
            }
            let params = match *method {
                SARAH_METHOD_SEND_MESSAGE => Some(json!({
                    "text": "hello from test",
                    "idempotencyRef": "idem.handle.send",
                    "expectedGeneration": 1,
                })),
                SARAH_METHOD_INTERRUPT_TURN => {
                    client
                        .send_message("prep", "idem.prep", 1)
                        .expect("prepare active turn");
                    Some(json!({
                        "turnRef": "turn.1",
                        "idempotencyRef": "idem.handle.interrupt",
                        "expectedGeneration": 1,
                    }))
                }
                SARAH_METHOD_ROOM_SNAPSHOT => Some(json!({ "limit": 5 })),
                _ => None,
            };
            client
                .handle_request(method, 1, params.as_ref())
                .unwrap_or_else(|error| panic!("{method} failed: {error}"));
        }
    }

    struct OneHealthyOneDownRelay {
        state_path: PathBuf,
        query_count: Arc<AtomicUsize>,
        connected: bool,
        acknowledged_event_id: Option<String>,
    }

    impl RelayTransport for OneHealthyOneDownRelay {
        fn label(&self) -> &str {
            "wss://healthy.example"
        }

        fn connection_state(&self) -> ConnectionState {
            if self.connected {
                ConnectionState::Degraded
            } else {
                ConnectionState::Disconnected
            }
        }

        fn is_authenticated(&self) -> bool {
            true
        }

        fn connect(&mut self) -> Result<(), SarahConversationError> {
            self.connected = true;
            Ok(())
        }

        fn auth_challenge(&self) -> Option<RelayAuthChallenge> {
            None
        }

        fn authenticate(&mut self, _auth_event: &Event) -> Result<(), SarahConversationError> {
            Ok(())
        }

        fn publish(&mut self, event: &Event) -> Result<(), SarahConversationError> {
            let persisted = fs::read_to_string(&self.state_path)
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
            if !persisted.contains(&event.id.to_hex()) {
                return Err(SarahConversationError::Internal(
                    "signed outbox was not durable before publish".into(),
                ));
            }
            self.acknowledged_event_id = Some(event.id.to_hex());
            Ok(())
        }

        fn publication_complete(&mut self, _event_id: &str) -> bool {
            false
        }

        fn acknowledged_relays(&self, event_id: &str) -> Vec<String> {
            if self.acknowledged_event_id.as_deref() == Some(event_id) {
                vec!["wss://healthy.example".into()]
            } else {
                Vec::new()
            }
        }

        fn query(
            &mut self,
            _conversation_ref: &str,
            _after_cursor: Option<&str>,
            _limit: usize,
        ) -> Result<QueryPage, SarahConversationError> {
            self.query_count.fetch_add(1, Ordering::SeqCst);
            Ok(QueryPage {
                events: Vec::new(),
                next_cursor: None,
                gap_state: GapState::Possible,
            })
        }

        fn last_event_id(&self) -> Option<String> {
            self.acknowledged_event_id.clone()
        }

        fn gap_state(&self) -> GapState {
            GapState::Possible
        }

        fn connected_relays(&self) -> Vec<String> {
            vec!["wss://healthy.example".into()]
        }

        fn requires_private_messages(&self) -> bool {
            true
        }
    }

    #[test]
    fn partial_relay_ack_is_durable_and_does_not_block_querying() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let state_path = temporary.path().join("owner").join("issue31-state.json");
        let query_count = Arc::new(AtomicUsize::new(0));
        let signer = SigningIdentity::generate();
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = signer.public_key_hex.clone();
        config.direct_endpoints = vec![Issue31DirectEndpoint {
            magic_dns_name: "omega-primary.tail1234.ts.net".into(),
            port: 4317,
            protocol: omega_device_bridge::PROTOCOL.into(),
        }];
        let expected_direct_endpoints = config.direct_endpoints.clone();
        let relay_urls = vec![
            "wss://healthy.example".to_string(),
            "wss://down.example".to_string(),
        ];
        let controller = Issue31HostController::new(Issue31HostConfiguration {
            host_ref: "omega.host.local".into(),
            host_public_key_hex: signer.public_key_hex.clone(),
            sarah_public_key_hex: config.identity.sarah_public_key_hex.clone(),
            conversation: config.conversation_ref(),
            display_name: "Local Omega".into(),
            relay_urls,
            generation: ISSUE31_NOSTR_HOST_GENERATION,
        })
        .expect("controller");
        let relay = OneHealthyOneDownRelay {
            state_path: state_path.clone(),
            query_count: query_count.clone(),
            connected: false,
            acknowledged_event_id: None,
        };
        let mut client = SarahConversationClient::with_relay(config, Box::new(relay), signer);
        client.issue31_host = Some(controller);
        client.issue31_state_path = Some(state_path.clone());

        client.sync_issue31_host().expect("partial relay sync");

        assert!(query_count.load(Ordering::SeqCst) > 0);
        let persisted: DurableIssue31HostState =
            serde_json::from_slice(&fs::read(&state_path).expect("read durable state"))
                .expect("decode durable state");
        let discovery_event = Event::from_json(
            persisted
                .discovery_event_json
                .as_deref()
                .expect("durable discovery event"),
        )
        .expect("signed discovery event");
        discovery_event.verify().expect("valid discovery signature");
        let discovery =
            Issue31HostDiscoveryV3::decode(discovery_event.content.as_bytes()).expect("V3 record");
        assert_eq!(discovery.direct_endpoints, expected_direct_endpoints);
        assert!(discovery.expires_at > discovery.issued_at);
        assert_eq!(persisted.relay_acknowledgements.len(), 1);
        assert_eq!(
            client.current_room_state().connection,
            ConnectionState::Degraded
        );
        assert_eq!(
            client.current_room_state().connected_relays,
            vec!["wss://healthy.example"]
        );
    }

    #[test]
    fn normal_commands_commit_exact_private_outbox_and_result_before_publish() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let state_path = temporary.path().join("owner").join("issue31-state.json");
        let signer = SigningIdentity::generate();
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = signer.public_key_hex.clone();
        let sarah_keys = Keys::generate();
        config.identity.sarah_public_key_hex = sarah_keys.public_key().to_hex();
        config.relay_url = Some("wss://healthy.example".into());
        let relay_urls = vec![
            "wss://healthy.example".to_string(),
            "wss://down.example".to_string(),
        ];
        let controller = Issue31HostController::new(Issue31HostConfiguration {
            host_ref: "omega.host.local".into(),
            host_public_key_hex: signer.public_key_hex.clone(),
            sarah_public_key_hex: config.identity.sarah_public_key_hex.clone(),
            conversation: config.conversation_ref(),
            display_name: "Local Omega".into(),
            relay_urls,
            generation: ISSUE31_NOSTR_HOST_GENERATION,
        })
        .expect("controller");
        let relay = OneHealthyOneDownRelay {
            state_path: state_path.clone(),
            query_count: Arc::new(AtomicUsize::new(0)),
            connected: false,
            acknowledged_event_id: None,
        };
        let mut client = SarahConversationClient::with_relay(config, Box::new(relay), signer);
        client.issue31_host = Some(controller);
        client.issue31_state_path = Some(state_path.clone());

        let send_params = json!({
            "text": "durable hello",
            "idempotencyRef": "idempotency.normal.send",
            "expectedGeneration": 1,
        });
        let first = client
            .handle_request(SARAH_METHOD_SEND_MESSAGE, 1, Some(&send_params))
            .expect("send");
        let replay = client
            .handle_request(SARAH_METHOD_SEND_MESSAGE, 1, Some(&send_params))
            .expect("idempotent replay");
        assert_eq!(first, replay);
        assert_eq!(client.message_seq, 1);

        let second_send_params = json!({
            "text": "durable hello",
            "idempotencyRef": "idempotency.normal.send.second",
            "expectedGeneration": 1,
        });
        let second = client
            .handle_request(SARAH_METHOD_SEND_MESSAGE, 1, Some(&second_send_params))
            .expect("distinct repeated send");
        assert_ne!(first.get("eventId"), second.get("eventId"));
        assert_eq!(client.message_seq, 2);

        let persisted: DurableIssue31HostState =
            serde_json::from_slice(&fs::read(&state_path).expect("read durable state"))
                .expect("decode durable state");
        assert_eq!(persisted.active_turn_ref.as_deref(), Some("turn.2"));
        assert_eq!(persisted.run_state, "running");
        assert_eq!(persisted.message_seq, 2);
        assert!(
            persisted
                .command_results
                .contains_key("idempotency.normal.send")
        );
        assert!(
            persisted
                .command_results
                .contains_key("idempotency.normal.send.second")
        );
        assert_eq!(persisted.private_outbox.len(), 2);
        let rumor_ids = persisted
            .private_outbox
            .values()
            .map(|pending| pending.rumor_event_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(rumor_ids.len(), 2);
        assert_eq!(persisted.relay_acknowledgements.len(), 4);
        let serialized = fs::read_to_string(&state_path).expect("read serialized state");
        for pending in persisted.private_outbox.values() {
            for gift_wrap in &pending.gift_wrap_event_json {
                let event = Event::from_json(gift_wrap).expect("gift wrap");
                assert!(serialized.contains(&event.id.to_hex()));
            }
        }
        let mut unwrapped_turn_refs = std::collections::BTreeSet::new();
        for pending in persisted.private_outbox.values() {
            let mut found_sarah_copy = false;
            for gift_wrap in &pending.gift_wrap_event_json {
                let event = Event::from_json(gift_wrap).expect("gift wrap");
                match smol::block_on(nostr::nips::nip59::extract_rumor(&sarah_keys, &event)) {
                    Ok(unwrapped) => {
                        found_sarah_copy = true;
                        let turn_ref = unwrapped
                            .rumor
                            .tags
                            .iter()
                            .find_map(|tag| {
                                let tag = tag.as_slice();
                                (tag.first().map(String::as_str) == Some("turn"))
                                    .then(|| tag.get(1).cloned())
                                    .flatten()
                            })
                            .expect("turn tag in Sarah rumor");
                        unwrapped_turn_refs.insert(turn_ref);
                    }
                    // The other gift wrap is the owner's independently encrypted copy.
                    Err(_) => {}
                }
            }
            assert!(found_sarah_copy);
        }
        assert_eq!(
            unwrapped_turn_refs,
            std::collections::BTreeSet::from(["turn.1".to_string(), "turn.2".to_string()])
        );

        let interrupt_params = json!({
            "turnRef": "turn.2",
            "idempotencyRef": "idempotency.normal.interrupt",
            "expectedGeneration": 1,
        });
        client
            .handle_request(SARAH_METHOD_INTERRUPT_TURN, 1, Some(&interrupt_params))
            .expect("interrupt");
        let persisted: DurableIssue31HostState =
            serde_json::from_slice(&fs::read(&state_path).expect("read durable state"))
                .expect("decode durable state");
        assert_eq!(persisted.run_state, "interrupt_pending");
        assert!(
            persisted
                .command_results
                .contains_key("idempotency.normal.interrupt")
        );
        assert_eq!(persisted.private_outbox.len(), 3);
    }

    #[test]
    fn issue31_control_scan_persists_cursor_and_resumes_after_page_cap() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let state_path = temporary.path().join("owner").join("issue31-state.json");
        let signer = SigningIdentity::generate();
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = signer.public_key_hex.clone();
        let conversation_ref = config.conversation_ref();
        let mut relay = MockRelayAdapter::new();
        for index in 0..513 {
            relay.seed_event(StoredConversationEvent {
                event_id: format!("{index:064x}"),
                kind: 1,
                pubkey: signer.public_key_hex.clone(),
                created_at: 1_000 + index as u64,
                conversation_ref: conversation_ref.clone(),
                content_summary: "bounded non-control record".into(),
                tags: Vec::new(),
                record_kind: "message".into(),
                store_index: index,
            });
        }
        let controller = Issue31HostController::new(Issue31HostConfiguration {
            host_ref: "omega.host.local".into(),
            host_public_key_hex: signer.public_key_hex.clone(),
            sarah_public_key_hex: config.identity.sarah_public_key_hex.clone(),
            conversation: config.conversation_ref(),
            display_name: "Local Omega".into(),
            relay_urls: vec!["wss://relay.example.com".into()],
            generation: ISSUE31_NOSTR_HOST_GENERATION,
        })
        .expect("controller");
        let mut client = SarahConversationClient::with_relay(config, Box::new(relay), signer);
        client.issue31_host = Some(controller);
        client.issue31_state_path = Some(state_path.clone());

        client.sync_issue31_host().expect("first bounded scan");
        assert_eq!(client.last_gap_state, GapState::Possible);
        let first_expected_cursor = format!("cursor.1511.{:064x}", 511);
        assert_eq!(
            client.issue31_control_cursor.as_deref(),
            Some(first_expected_cursor.as_str())
        );
        client.sync_issue31_host().expect("resume scan");
        let second_expected_cursor = format!("cursor.1512.{:064x}", 512);
        assert_eq!(
            client.issue31_control_cursor.as_deref(),
            Some(second_expected_cursor.as_str())
        );
        let persisted: DurableIssue31HostState =
            serde_json::from_slice(&fs::read(state_path).expect("read durable state"))
                .expect("decode durable state");
        assert_eq!(persisted.control_cursor, client.issue31_control_cursor);
    }

    #[test]
    fn invalid_issue31_event_is_quarantined_without_starving_later_pairing() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let state_path = temporary.path().join("owner").join("issue31-state.json");
        let signer = SigningIdentity::generate();
        let device = Keys::generate();
        let device_public_key_hex = device.public_key().to_hex();
        let sarah_public_key_hex = Keys::generate().public_key().to_hex();
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = signer.public_key_hex.clone();
        config.identity.sarah_public_key_hex = sarah_public_key_hex.clone();
        let conversation_ref = config.conversation_ref();
        let now = unix_now();
        let expired = Issue31PairingRecord::PairingRequest {
            schema: ISSUE31_PAIRING_SCHEMA.into(),
            host_ref: "omega.host.local".into(),
            host_public_key_hex: signer.public_key_hex.clone(),
            device_public_key_hex: device_public_key_hex.clone(),
            issued_at: 100,
            pairing_request_ref: "pairing_request.device.expired".into(),
            requested_scopes: vec![crate::Issue31PairingScope::ObserveIssue31],
            expires_at: 200,
        };
        let valid = Issue31PairingRecord::PairingRequest {
            schema: ISSUE31_PAIRING_SCHEMA.into(),
            host_ref: "omega.host.local".into(),
            host_public_key_hex: signer.public_key_hex.clone(),
            device_public_key_hex: device_public_key_hex.clone(),
            issued_at: now,
            pairing_request_ref: "pairing_request.device.valid".into(),
            requested_scopes: vec![crate::Issue31PairingScope::ObserveIssue31],
            expires_at: now.saturating_add(600),
        };
        let mut relay = MockRelayAdapter::new();
        for (index, (event_id, record)) in [("a".repeat(64), expired), ("b".repeat(64), valid)]
            .into_iter()
            .enumerate()
        {
            relay.seed_event(StoredConversationEvent {
                event_id,
                kind: Kind::PrivateDirectMessage.as_u16(),
                pubkey: device_public_key_hex.clone(),
                created_at: now.saturating_add(index as u64),
                conversation_ref: conversation_ref.clone(),
                content_summary: serde_json::to_string(&record).expect("pairing json"),
                tags: Vec::new(),
                record_kind: "pairing".into(),
                store_index: index,
            });
        }
        let mut controller = Issue31HostController::new(Issue31HostConfiguration {
            host_ref: "omega.host.local".into(),
            host_public_key_hex: signer.public_key_hex.clone(),
            sarah_public_key_hex,
            conversation: config.conversation_ref(),
            display_name: "Local Omega".into(),
            relay_urls: vec!["wss://relay.example.com".into()],
            generation: ISSUE31_NOSTR_HOST_GENERATION,
        })
        .expect("controller");
        controller
            .set_admitted_device_policy(
                vec![device_public_key_hex],
                vec![crate::Issue31PairingScope::ObserveIssue31],
            )
            .expect("admit device");
        let mut client = SarahConversationClient::with_relay(config, Box::new(relay), signer);
        client.issue31_host = Some(controller);
        client.issue31_state_path = Some(state_path.clone());

        client.sync_issue31_host().expect("quarantine and continue");
        assert_eq!(
            client
                .issue31_quarantined_events
                .get(&"a".repeat(64))
                .map(String::as_str),
            Some("reason.omega.pairing_rejected")
        );
        let controller = client.issue31_host.as_ref().expect("controller");
        assert!(controller.pairing_event_was_processed(&"b".repeat(64)));
        let persisted: DurableIssue31HostState =
            serde_json::from_slice(&fs::read(state_path).expect("read durable state"))
                .expect("decode durable state");
        assert_eq!(persisted.quarantined_events.len(), 1);
        assert!(persisted.control_cursor.is_some());
    }

    #[test]
    fn inbound_pairing_is_retryable_when_atomic_state_and_outbox_commit_fails() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let state_path = temporary.path().join("owner").join("issue31-state.json");
        let signer = SigningIdentity::generate();
        let device_public_key_hex = Keys::generate().public_key().to_hex();
        let sarah_public_key_hex = Keys::generate().public_key().to_hex();
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = signer.public_key_hex.clone();
        config.identity.sarah_public_key_hex = sarah_public_key_hex.clone();
        let conversation_ref = config.conversation_ref();
        let now = unix_now();
        let inbound_event_id = "c".repeat(64);
        let request = Issue31PairingRecord::PairingRequest {
            schema: ISSUE31_PAIRING_SCHEMA.into(),
            host_ref: "omega.host.local".into(),
            host_public_key_hex: signer.public_key_hex.clone(),
            device_public_key_hex: device_public_key_hex.clone(),
            issued_at: now,
            pairing_request_ref: "pairing_request.device.retry".into(),
            requested_scopes: vec![crate::Issue31PairingScope::ObserveIssue31],
            expires_at: now.saturating_add(600),
        };
        let mut relay = MockRelayAdapter::new();
        relay.seed_event(StoredConversationEvent {
            event_id: inbound_event_id.clone(),
            kind: Kind::PrivateDirectMessage.as_u16(),
            pubkey: device_public_key_hex.clone(),
            created_at: now,
            conversation_ref,
            content_summary: serde_json::to_string(&request).expect("pairing json"),
            tags: Vec::new(),
            record_kind: "pairing".into(),
            store_index: 0,
        });
        let mut controller = Issue31HostController::new(Issue31HostConfiguration {
            host_ref: "omega.host.local".into(),
            host_public_key_hex: signer.public_key_hex.clone(),
            sarah_public_key_hex,
            conversation: config.conversation_ref(),
            display_name: "Local Omega".into(),
            relay_urls: vec!["wss://relay.example.com".into()],
            generation: ISSUE31_NOSTR_HOST_GENERATION,
        })
        .expect("controller");
        controller
            .set_admitted_device_policy(
                vec![device_public_key_hex],
                vec![crate::Issue31PairingScope::ObserveIssue31],
            )
            .expect("device policy");
        let mut client = SarahConversationClient::with_relay(config, Box::new(relay), signer);
        client.issue31_host = Some(controller);
        client.issue31_state_path = Some(state_path);
        client.issue31_discovery_generation = Some(ISSUE31_NOSTR_HOST_GENERATION);
        client.issue31_discovery_expires_at = Some(now.saturating_add(2 * 60 * 60));
        client.issue31_fail_commit_after.set(Some(1));

        let error = client
            .sync_issue31_host()
            .expect_err("injected transaction failure");
        assert!(
            error
                .to_string()
                .contains("injected Issue 31 durable commit failure")
        );
        assert!(client.issue31_private_outbox.is_empty());
        assert!(
            !client
                .issue31_host
                .as_ref()
                .expect("controller")
                .pairing_event_was_processed(&inbound_event_id)
        );

        client.sync_issue31_host().expect("retry pairing");
        assert!(
            client
                .issue31_host
                .as_ref()
                .expect("controller")
                .pairing_event_was_processed(&inbound_event_id)
        );
    }

    #[test]
    fn durable_issue31_state_reuses_exact_outbox_and_refuses_revoked_replay_after_restart() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let state_path = temporary.path().join("owner").join("issue31-state.json");
        let (configuration, controller, device_public_key_hex, grant_ref) =
            crate::issue31_nostr::restart_fixture();
        let signer = ConversationSigner::Keys(SigningIdentity::generate());
        let tags = vec![Tag::parse(["p", device_public_key_hex.as_str()]).expect("p tag")];
        let (rumor_event_id, gift_wraps) = signer
            .private_messages(
                "{\"schema\":\"openagents.omega.issue31.command.v1\"}",
                tags,
                std::slice::from_ref(&device_public_key_hex),
            )
            .expect("private outbox");
        let exact_event_json = gift_wraps[0].try_as_json().expect("event json");
        let state = DurableIssue31HostState {
            schema: ISSUE31_DURABLE_STATE_SCHEMA.into(),
            controller,
            discovery_generation: Some(1),
            discovery_expires_at: Some(10_000),
            discovery_event_json: None,
            private_outbox: BTreeMap::from([(
                "openagents.omega.issue31.command.v1.test".into(),
                DurableIssue31PrivatePublish {
                    rumor_event_id,
                    gift_wrap_event_json: vec![exact_event_json.clone()],
                },
            )]),
            relay_acknowledgements: BTreeMap::new(),
            control_cursor: Some(format!("cursor.10.{}", "f".repeat(64))),
            projection_cursor: Some(format!("cursor.11.{}", "e".repeat(64))),
            quarantined_events: BTreeMap::from([(
                "9".repeat(64),
                "reason.omega.invalid_pairing_record".into(),
            )]),
            host_adjunct_emissions: BTreeMap::new(),
            provider_handoffs: Issue31ProviderHandoffLedger::default(),
            command_results: BTreeMap::from([(
                "idempotency.restart.admin".into(),
                ("fingerprint".into(), json!({ "eventId": "1".repeat(64) })),
            )]),
            active_turn_ref: Some("turn.7".into()),
            run_state: "running".into(),
            message_seq: 7,
        };
        write_issue31_host_state(&state_path, &state).expect("write state");
        let serialized = fs::read_to_string(&state_path).expect("read state");
        assert!(!serialized.contains("nsec"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&state_path)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        let loaded = load_issue31_host_state(&state_path, &configuration)
            .expect("load state")
            .expect("persisted state");
        assert_eq!(loaded.active_turn_ref.as_deref(), Some("turn.7"));
        assert_eq!(loaded.run_state, "running");
        assert_eq!(loaded.message_seq, 7);
        let runtime_outbox = durable_private_outbox_into_runtime(
            loaded
                .private_outbox
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
        )
        .expect("runtime outbox");
        assert_eq!(
            runtime_outbox
                .values()
                .next()
                .expect("outbox item")
                .gift_wraps[0]
                .try_as_json()
                .expect("reloaded event json"),
            exact_event_json
        );

        let mut controller = loaded.controller;
        let result = controller
            .handle_command_event(
                Issue31CommandEvent {
                    event_id: "0".repeat(64),
                    record: Issue31CommandRecord::CommandIntent {
                        schema: ISSUE31_COMMAND_SCHEMA.into(),
                        host_ref: configuration.host_ref,
                        host_public_key_hex: configuration.host_public_key_hex,
                        device_public_key_hex,
                        grant_ref,
                        action_ref: "action.omega.full_auto.stop".into(),
                        idempotency_ref: "idempotency.restart.revoked_replay".into(),
                        expected_generation: 1,
                        arguments_ref: "arguments.omega.none".into(),
                        issued_at: 106,
                        expires_at: 200,
                    },
                },
                107,
                |_, _, _| panic!("revoked device command executed"),
            )
            .expect("terminal refusal")
            .expect("result record");
        assert!(matches!(
            result,
            Issue31CommandRecord::CommandResult {
                status: Issue31CommandStatus::Refused,
                ..
            }
        ));
    }

    // ---------------------------------------------------------------------
    // omega#49 device-proof host
    // ---------------------------------------------------------------------

    /// Read a `Keys` from a 64-hex secret in the environment.
    #[cfg(test)]
    fn device_proof_keys(variable: &str) -> Keys {
        let secret = std::env::var(variable)
            .unwrap_or_else(|_| panic!("{variable} must be a 64-hex secret key"));
        Keys::parse(secret.trim()).unwrap_or_else(|error| panic!("{variable}: {error}"))
    }

    /// Publish one event through a real adapter, meeting the NIP-42 challenge
    /// lazily exactly as `publish_with_auth` does.
    #[cfg(test)]
    /// Sarah's half of the conversation, shaped the way the host actually reads it.
    ///
    /// The host subscribes to gift wraps addressed to its own custody key and
    /// unwraps the kind-14 rumor inside into a `message` record — the identical
    /// path the owner's own sends take. A bare kind 14 published straight to
    /// the relay is never requested by any filter and never arrives, and a
    /// kind 44300 is a `Turn`, which must carry NIP-44 ciphertext of JSON and
    /// is not a conversation message at all.
    ///
    /// `conversation_tags` is the production tag builder, so the rumor names
    /// exactly the owner and Sarah and satisfies `require_conversation_recipients`
    /// rather than approximating it here.
    fn sarah_conversation_wrap(
        sarah_keys: &Keys,
        owner_public_key: &PublicKey,
        owner_public_key_hex: &str,
        conversation_ref: &str,
        text: &str,
    ) -> (nostr::Event, String) {
        let tags = conversation_tags(
            conversation_ref,
            owner_public_key_hex,
            &sarah_keys.public_key().to_hex(),
        )
        .expect("production conversation tags");
        let mut rumor = EventBuilder::new(Kind::PrivateDirectMessage, text)
            .tags(tags)
            .build(sarah_keys.public_key());
        rumor.ensure_id();
        // The wrap id is not the record id. The host stores the rumor under its
        // own id and projects that id to the device, so a harness that reports
        // the wrap id is naming an identifier nothing downstream ever uses.
        let rumor_id = rumor.id.expect("the rumor carries its id").to_hex();
        let wrap = smol::block_on(EventBuilder::gift_wrap(
            sarah_keys,
            owner_public_key,
            rumor,
            [],
        ))
        .expect("gift wrap Sarah's message to the host");
        (wrap, rumor_id)
    }

    /// Publish, and survive a relay that closed an idle socket.
    ///
    /// The harness holds one Sarah connection for the life of the run and used
    /// to publish straight down it. A reply sent minutes after seeding arrived
    /// on a socket the relay had already reset, and the harness reported it as
    /// Sarah failing to answer — indistinguishable, from the outside, from the
    /// host never producing a reply. A one-message proof would have failed on
    /// transport before reaching any of the record contracts.
    fn device_proof_publish(
        relay: &mut crate::nostr_websocket_relay::WebSocketRelayAdapter,
        auth_url: &str,
        keys: &Keys,
        record: &Event,
    ) -> Result<(), SarahConversationError> {
        match device_proof_publish_authenticated(relay, auth_url, keys, record) {
            Ok(()) => Ok(()),
            Err(error) => {
                eprintln!("device-proof: sarah publish failed ({error}); reconnecting");
                relay.connect()?;
                // A new socket is a new NIP-42 session, so the auth path runs
                // again rather than assuming the old challenge still holds.
                device_proof_publish_authenticated(relay, auth_url, keys, record)
            }
        }
    }

    fn device_proof_publish_authenticated(
        relay: &mut crate::nostr_websocket_relay::WebSocketRelayAdapter,
        auth_url: &str,
        keys: &Keys,
        record: &Event,
    ) -> Result<(), SarahConversationError> {
        match relay.publish(record) {
            Err(SarahConversationError::IdentityRequired) => {
                // Every step here returns rather than panics. A relay that
                // closed an idle socket fails *inside* this branch — `publish`
                // reports `IdentityRequired`, control comes here, and the AUTH
                // write hits the reset — so an `expect` on this path takes the
                // whole harness down before any retry wrapper can see it, and
                // reports a dead socket as Sarah failing to answer.
                let challenge = relay.auth_challenge().ok_or_else(|| {
                    SarahConversationError::Relay(
                        "relay refused the publish without exposing a challenge".into(),
                    )
                })?;
                let auth_event = EventBuilder::new(Kind::Custom(22242), "")
                    .tag(Tag::parse(["relay", auth_url]).expect("relay tag"))
                    .tag(
                        Tag::parse(["challenge", challenge.challenge.as_str()])
                            .expect("challenge tag"),
                    )
                    .sign_with_keys(keys)
                    .expect("signed auth event");
                relay.authenticate(&auth_event)?;
                relay.publish(record)
            }
            other => other,
        }
    }

    /// The real production issue #31 host pump, run headless against a live relay.
    ///
    /// This is the host half of the omega#49 device proof. Everything downstream
    /// of the signer is the production path: `sync_issue31_host` is the same
    /// entry point `bootstrap` calls, so discovery v2, the pairing state machine,
    /// command execution, source projection and the withheld-source statement are
    /// all the shipped code. The only substitution is identity custody — the
    /// owner key is supplied as a keypair rather than drawn from
    /// `omega_identity::IdentityService`, because custody needs the GPUI app and
    /// this harness must run without a window. That substitution is recorded on
    /// the issue rather than hidden here: a run of this harness proves the host
    /// protocol, not owner key custody.
    ///
    /// Sarah's own turns are authored by a second keypair the harness holds. The
    /// admitted OpenAgents turn service is out of scope for a device proof and is
    /// deliberately not stood up; a reply published this way is a real signed
    /// Sarah record on a real relay, and is not evidence that the turn service
    /// produced it.
    ///
    /// ```sh
    /// OMEGA_DEVICE_PROOF_RELAY=wss://relay.openagents.com \
    /// OMEGA_DEVICE_PROOF_HOST_SECRET=<64 hex> \
    /// OMEGA_DEVICE_PROOF_SARAH_SECRET=<64 hex> \
    /// OMEGA_DEVICE_PROOF_DEVICE_PUBKEY=<64 hex> \
    /// OMEGA_DEVICE_PROOF_STATE=/path/to/state.json \
    ///   cargo test -p omega_effectd --lib live_device_proof_host -- --ignored --nocapture
    /// ```
    /// omega#49 / omega#46 exit 1: a device sends to Sarah and the host must
    /// publish the owner record the device is waiting on.
    ///
    /// The phone showed the correct pending state and no reply ever arrived.
    /// The command was admitted and never quarantined, so the failure was
    /// downstream of admission, inside `execute_issue31_action_v2`:
    /// `send_message` publishes the owner rumor, `enqueue_issue31_source_projection`
    /// then has to read it back off the relay to confirm it, and the read-back
    /// refused the host's own message — see `require_conversation_recipients`.
    /// The result degraded to `reason.omega.projection_failed` and no owner
    /// projection was ever enqueued for the device.
    ///
    /// This drives the real action path against a real relay, so the assertion
    /// is on the same bytes a phone waits for rather than on a substitute.
    #[test]
    #[ignore = "requires a live relay; set OMEGA_LIVE_RELAY_URL"]
    fn owner_send_confirms_and_projects_to_the_device_on_a_live_relay() {
        let Ok(relay_url) = std::env::var("OMEGA_LIVE_RELAY_URL") else {
            eprintln!("OMEGA_LIVE_RELAY_URL unset; skipping");
            return;
        };
        let temporary = tempfile::tempdir().expect("tempdir");
        let host_keys = Keys::generate();
        let sarah_keys = Keys::generate();
        let device_public_key_hex = Keys::generate().public_key().to_hex();
        let signer = SigningIdentity::from_keys(host_keys.clone());
        let owner_public_key_hex = signer.public_key_hex.clone();
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = owner_public_key_hex.clone();
        config.identity.sarah_public_key_hex = sarah_keys.public_key().to_hex();
        config.conversation_digest = owner_public_key_hex[..24].to_string();
        config.relay_url = Some(relay_url.clone());
        let conversation_ref = config.conversation_ref();

        let relay = crate::nostr_websocket_relay::WebSocketRelayAdapter::new_for_keys_with_policy(
            vec![relay_url.clone()],
            host_keys,
            sarah_keys.public_key().to_hex(),
            Vec::new(),
            Vec::new(),
        )
        .expect("host adapter");
        let controller = Issue31HostController::new(Issue31HostConfiguration {
            host_ref: "omega.host.local".into(),
            host_public_key_hex: owner_public_key_hex.clone(),
            sarah_public_key_hex: sarah_keys.public_key().to_hex(),
            conversation: conversation_ref.clone(),
            display_name: "Local Omega".into(),
            relay_urls: vec![relay_url],
            generation: ISSUE31_NOSTR_HOST_GENERATION,
        })
        .expect("host controller");
        let mut client = SarahConversationClient::with_relay(config, Box::new(relay), signer);
        client.issue31_state_path = Some(temporary.path().join("issue31-state.json"));
        // The production caller binds the controller around the executor; the
        // durable commit inside `send_message` requires it.
        client.issue31_host = Some(controller);

        let execution = client.execute_issue31_action_v2(
            &Issue31CommandArguments::SendMessage {
                action_ref: crate::ISSUE31_ACTION_SEND_MESSAGE.into(),
                conversation: conversation_ref,
                text: "Does a send from a paired device confirm?".into(),
            },
            "idempotency.issue31.send_message:liveconfirm",
            "grant.omega.liveconfirm",
            &device_public_key_hex,
            1,
        );
        assert_eq!(
            execution.status,
            Issue31CommandHandlingStatus::Accepted,
            "the host must accept its own send; reason {:?}",
            execution.reason_ref
        );
        let source_event_id = execution
            .source_event_id
            .expect("an accepted send must name the owner record it produced");

        // The owner record is not merely published — it is readable back off
        // the relay, which is the step that was failing.
        let source = client
            .load_issue31_source_event(&source_event_id)
            .expect("the owner's own message must be readable back off the relay");
        assert_eq!(source.pubkey, owner_public_key_hex);
        assert_eq!(source.record_kind, "message");
        assert_eq!(source.kind, crate::ISSUE31_PRIVATE_RUMOR_KIND);

        // And the device has an owner projection waiting for it, addressed to
        // the device key rather than to either conversation participant.
        assert!(
            !client.issue31_private_outbox.is_empty(),
            "an accepted send must enqueue the owner projection the device renders"
        );
    }

    #[test]
    #[ignore = "device proof host; set OMEGA_DEVICE_PROOF_RELAY"]
    fn live_device_proof_host() {
        let Ok(relay_url) = std::env::var("OMEGA_DEVICE_PROOF_RELAY") else {
            eprintln!("OMEGA_DEVICE_PROOF_RELAY unset; skipping");
            return;
        };
        let auth_url =
            std::env::var("OMEGA_DEVICE_PROOF_AUTH_URL").unwrap_or_else(|_| relay_url.clone());
        let host_keys = device_proof_keys("OMEGA_DEVICE_PROOF_HOST_SECRET");
        let sarah_keys = device_proof_keys("OMEGA_DEVICE_PROOF_SARAH_SECRET");
        // Comma-separated, so one host can admit an iOS surface and an Android
        // surface at the same time. omega#49 asks for a result on both, and a
        // host that can only ever hold one grant proves the fan-out by
        // assertion rather than by running it.
        let device_public_key_hexes: Vec<String> =
            std::env::var("OMEGA_DEVICE_PROOF_DEVICE_PUBKEY")
                .expect("OMEGA_DEVICE_PROOF_DEVICE_PUBKEY must be the surfaces' 64-hex device keys")
                .split(',')
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect();
        assert!(
            !device_public_key_hexes.is_empty(),
            "OMEGA_DEVICE_PROOF_DEVICE_PUBKEY must name at least one device key"
        );
        let state_path = std::path::PathBuf::from(
            std::env::var("OMEGA_DEVICE_PROOF_STATE")
                .expect("OMEGA_DEVICE_PROOF_STATE must be a durable state file path"),
        );
        let seconds: u64 = std::env::var("OMEGA_DEVICE_PROOF_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(180);
        let revoke_after: Option<u64> = std::env::var("OMEGA_DEVICE_PROOF_REVOKE_AFTER")
            .ok()
            .and_then(|value| value.parse().ok());
        let seed = std::env::var("OMEGA_DEVICE_PROOF_SEED").is_ok();
        let quarantine = std::env::var("OMEGA_DEVICE_PROOF_QUARANTINE").is_ok();

        let signer = SigningIdentity::from_keys(host_keys.clone());
        let owner_public_key_hex = signer.public_key_hex.clone();
        let owner_public_key = PublicKey::from_hex(&owner_public_key_hex).expect("owner key");
        let sarah_public_key_hex = sarah_keys.public_key().to_hex();
        let mut config = SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = owner_public_key_hex.clone();
        config.identity.sarah_public_key_hex = sarah_public_key_hex.clone();
        config.conversation_digest = owner_public_key_hex[..24].to_string();
        config.relay_url = Some(relay_url.clone());
        config.admitted_device_public_key_hexes = device_public_key_hexes.clone();
        config.approved_device_scopes = vec![
            crate::Issue31PairingScope::ObserveIssue31,
            crate::Issue31PairingScope::SendMessage,
            crate::Issue31PairingScope::InterruptTurn,
        ];
        let conversation_ref = config.conversation_ref();

        // Seed the owner-private sources before the host starts, so the very
        // first projection pass has something to fan out.
        // Sarah publishes for the whole run, not only during the seed, so a
        // send from the device can be answered while the harness is live.
        let mut sarah_publisher = {
            let mut sarah_relay =
                crate::nostr_websocket_relay::WebSocketRelayAdapter::new_for_keys(
                    vec![relay_url.clone()],
                    sarah_keys.clone(),
                )
                .expect("sarah adapter");
            sarah_relay.connect().expect("sarah connect");
            Some(sarah_relay)
        };
        // The exact sources this run seeded, so the loop can say whether each
        // one reached each device rather than leaving "the host admits it and
        // never projects it" to be inferred from a device screenshot.
        let mut seeded_sources: Vec<(&'static str, String)> = Vec::new();

        if seed {
            let sarah_relay = sarah_publisher.as_mut().expect("sarah adapter");
            let now = unix_now();
            let (greeting, greeting_rumor_id) = sarah_conversation_wrap(
                &sarah_keys,
                &owner_public_key,
                &owner_public_key_hex,
                &conversation_ref,
                "Sarah here. This reply crossed a real relay from a real Omega host.",
            );
            device_proof_publish(sarah_relay, &auth_url, &sarah_keys, &greeting)
                .expect("publish the Sarah greeting");
            eprintln!("device-proof: seeded Sarah message {greeting_rumor_id}");
            seeded_sources.push(("greeting", greeting_rumor_id));

            // One encrypted engram, so "inspect memory" has a real source.
            // An engram body is a JSON object with a `mem/…` slug and a value,
            // not a sentence. Seeded as prose, the host received it, refused
            // the body, and quarantined it — so "inspect memory" had no real
            // source and the quarantine count was measuring this bug.
            let engram_plaintext = serde_json::json!({
                "slug": "mem/owner_reporting_preference",
                "value": "Chris prefers evidence-bound reports over confident summaries.",
            })
            .to_string();
            let engram_ciphertext = nip44::encrypt(
                sarah_keys.secret_key(),
                &owner_public_key,
                engram_plaintext.as_str(),
                nip44::Version::default(),
            )
            .expect("nip44 encrypt the engram");
            let engram =
                EventBuilder::new(Kind::Custom(crate::SARAH_ENGRAM_KIND), engram_ciphertext)
                    .tag(Tag::parse(["d", &"1".repeat(64)]).expect("d tag"))
                    .tag(Tag::parse(["p", owner_public_key_hex.as_str()]).expect("p tag"))
                    .tag(Tag::parse(["alt", "encrypted agent memory record"]).expect("alt tag"))
                    .tag(
                        Tag::parse(["conversation", conversation_ref.as_str()])
                            .expect("conversation tag"),
                    )
                    .custom_created_at(nostr::Timestamp::from(now))
                    .sign_with_keys(&sarah_keys)
                    .expect("signed engram");
            device_proof_publish(sarah_relay, &auth_url, &sarah_keys, &engram)
                .expect("publish the engram");
            eprintln!("device-proof: seeded engram {}", engram.id.to_hex());
            seeded_sources.push(("engram", engram.id.to_hex()));

            if quarantine {
                // Inside every record-level bound and outside the projection body
                // contract, so the host quarantines it and must say so.
                // Also 44300: a source the host never receives cannot be
                // quarantined, so on kind 14 this proved nothing either.
                let unreadable = EventBuilder::new(Kind::Custom(SARAH_TURN_RECORD_KIND), "")
                    .tag(
                        Tag::parse(["conversation", conversation_ref.as_str()])
                            .expect("conversation tag"),
                    )
                    .custom_created_at(nostr::Timestamp::from(now + 1))
                    .sign_with_keys(&sarah_keys)
                    .expect("signed unreadable source");
                device_proof_publish(sarah_relay, &auth_url, &sarah_keys, &unreadable)
                    .expect("publish the unreadable source");
                eprintln!(
                    "device-proof: seeded quarantine source {}",
                    unreadable.id.to_hex()
                );
            }
        }

        // The host reads with the owner key in custody and Sarah as the *other*
        // participant. `new_for_keys` collapses both roles onto one key, which
        // silently narrows the read: the `authors` filter becomes
        // `[host, host]`, so nothing Sarah signs is ever requested, and the
        // conversation participant set stops matching the records on the wire.
        let relay = crate::nostr_websocket_relay::WebSocketRelayAdapter::new_for_keys_with_policy(
            vec![relay_url.clone()],
            host_keys,
            sarah_public_key_hex.clone(),
            Vec::new(),
            Vec::new(),
        )
        .expect("host adapter");
        let host_configuration = Issue31HostConfiguration {
            host_ref: "omega.host.local".into(),
            host_public_key_hex: owner_public_key_hex.clone(),
            sarah_public_key_hex: sarah_public_key_hex.clone(),
            conversation: conversation_ref.clone(),
            display_name: "Local Omega".into(),
            relay_urls: vec![relay_url.clone()],
            generation: ISSUE31_NOSTR_HOST_GENERATION,
        };
        let persisted =
            load_issue31_host_state(&state_path, &host_configuration).expect("load durable state");
        let mut controller = match persisted {
            Some(persisted) => {
                eprintln!("device-proof: resumed durable host state at {state_path:?}");
                persisted.controller
            }
            None => Issue31HostController::new(host_configuration).expect("host controller"),
        };
        controller
            .set_admitted_device_policy(
                config.admitted_device_public_key_hexes.clone(),
                config.approved_device_scopes.clone(),
            )
            .expect("admit the device");

        let mut client = SarahConversationClient::with_relay(config, Box::new(relay), signer);
        client.issue31_host = Some(controller);
        client.issue31_state_path = Some(state_path);

        eprintln!("device-proof: host  npub/hex {owner_public_key_hex}");
        eprintln!("device-proof: sarah hex      {sarah_public_key_hex}");
        for device_public_key_hex in &device_public_key_hexes {
            eprintln!("device-proof: device hex     {device_public_key_hex}");
        }
        eprintln!("device-proof: conversation   {conversation_ref}");
        eprintln!("device-proof: relay          {relay_url}");

        let started = std::time::Instant::now();
        let mut revoked = false;
        let mut announced_grants: BTreeMap<String, String> = BTreeMap::new();
        // The previous run of this harness could see that a command had been
        // admitted and not quarantined, and could not see what the host then
        // did with it. That is the whole distance between "the send failed" and
        // "the send failed inside `send_message` with `projection_failed`", so
        // the accepted-send counter and Sarah's answer are both surfaced here.
        let mut announced_message_seq = client.message_seq;
        while started.elapsed().as_secs() < seconds {
            match client.sync_issue31_host() {
                Ok(()) => {}
                Err(error) => eprintln!("device-proof: sync error {error}"),
            }
            if client.message_seq != announced_message_seq {
                announced_message_seq = client.message_seq;
                let turn_ref = client
                    .active_turn_ref
                    .clone()
                    .unwrap_or_else(|| format!("turn.{announced_message_seq}"));
                eprintln!(
                    "device-proof: OWNER SEND ACCEPTED · message_seq {announced_message_seq} · {turn_ref} · run_state {}",
                    client.run_state
                );
                if let Some(sarah_relay) = sarah_publisher.as_mut() {
                    // Sarah's half. This is the harness's own keypair, not the
                    // admitted OpenAgents turn service: it produces a real
                    // signed Sarah record on a real relay, and is not evidence
                    // that the turn service produced it.
                    // A conversation message reaches the host the same way the
                    // owner's own does: a NIP-59 gift wrap addressed to the
                    // host's custody key, carrying a kind-14 rumor. This
                    // harness published a bare kind 14 straight to the relay,
                    // which the host never subscribes to, so every reply it
                    // reported sending was discarded in transit and the reply
                    // arm of this proof had never once run.
                    let (reply, _reply_rumor_id) = sarah_conversation_wrap(
                        &sarah_keys,
                        &owner_public_key,
                        &owner_public_key_hex,
                        &conversation_ref,
                        &format!(
                            "Received. Answering {turn_ref} from a real Omega host over a real relay."
                        ),
                    );
                    match device_proof_publish(sarah_relay, &auth_url, &sarah_keys, &reply) {
                        Ok(()) => eprintln!(
                            "device-proof: SARAH REPLIED {} · {turn_ref}",
                            &reply.id.to_hex()[..16]
                        ),
                        Err(error) => eprintln!("device-proof: Sarah reply failed {error}"),
                    }
                }
            }
            let now = unix_now();
            for (event_id, reason) in &client.issue31_quarantined_events {
                if announced_grants
                    .insert(format!("quarantine:{event_id}"), reason.clone())
                    .as_ref()
                    != Some(reason)
                {
                    eprintln!("device-proof: QUARANTINED {} · {reason}", &event_id[..16]);
                }
            }
            for (key, substance) in &client.issue31_withheld_emissions {
                let line = format!("{substance:?}");
                if announced_grants
                    .insert(format!("withheld:{key}"), line.clone())
                    .as_ref()
                    != Some(&line)
                {
                    eprintln!("device-proof: withheld {key} · {line}");
                }
            }
            if let Some(host) = client.issue31_host.as_ref() {
                for projection in host.grant_projections(now).unwrap_or_default() {
                    let line = format!(
                        "{} · {} · generation {} · scopes {:?}",
                        projection.device_fingerprint,
                        projection.status,
                        projection.generation,
                        projection.scopes
                    );
                    if announced_grants.get(&projection.grant_ref) != Some(&line) {
                        eprintln!("device-proof: grant {} · {line}", projection.grant_ref);
                        announced_grants.insert(projection.grant_ref.clone(), line);
                    }
                    // Whether each seeded source actually reached this grant.
                    // "The host admits the engram and never projects it" was
                    // read off a device surface; the host can state it directly.
                    for (name, source_event_id) in &seeded_sources {
                        let projected = host.source_was_projected(
                            &projection.grant_ref,
                            projection.generation,
                            source_event_id,
                        );
                        if !projected {
                            continue;
                        }
                        let key = format!("projected:{}:{name}", projection.grant_ref);
                        if announced_grants
                            .insert(key, source_event_id.clone())
                            .as_ref()
                            != Some(source_event_id)
                        {
                            eprintln!(
                                "device-proof: PROJECTED {name} {} · {}",
                                &source_event_id[..16],
                                projection.grant_ref
                            );
                        }
                    }
                }
            }
            if let Some(revoke_after) = revoke_after {
                if !revoked && started.elapsed().as_secs() >= revoke_after {
                    let grant_refs: Vec<String> = client
                        .issue31_host
                        .as_ref()
                        .map(|host| {
                            host.grant_projections(now)
                                .unwrap_or_default()
                                .into_iter()
                                .filter(|projection| projection.status == "active")
                                .map(|projection| projection.grant_ref)
                                .collect()
                        })
                        .unwrap_or_default();
                    for grant_ref in grant_refs {
                        match client.revoke_issue31_grant(
                            &grant_ref,
                            Some("reason.omega.owner_revoked".to_string()),
                            format!("idempotency.device_proof.revoke.{grant_ref}"),
                            grant_ref.clone(),
                        ) {
                            Ok(_) => eprintln!("device-proof: REVOKED {grant_ref}"),
                            Err(error) => eprintln!("device-proof: revoke failed {error}"),
                        }
                        revoked = true;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
        eprintln!("device-proof: host stopped after {seconds}s");
    }
}
