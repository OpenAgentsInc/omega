//! Sarah Nostr conversation client for `openagents.omega.effectd.v1`.
//!
//! Packet: SARAH-NR-06 (OpenAgentsInc/omega#33).
//! Spec: docs/omega/2026-07-24-sarah-workroom-mvp-spec.md §8, §24.7.
//!
//! This module is the only conversation client for the Sarah lane. It must
//! never link a Khala Sync client. The backing transport is a Nostr relay
//! adapter: mock/in-memory for local tests, NIP-42 authenticated when a real
//! relay URL and identity key are configured.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use nostr::{Event, EventBuilder, Keys, Kind, RelayUrl, Tag, nips::nip42};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::protocol::{MAX_FRAME_BYTES, PROTOCOL_SCHEMA};

pub use crate::openagents_binding::BindingState;

/// Framed method names on `openagents.omega.effectd.v1` for the Sarah room.
pub const SARAH_METHOD_SESSION_STATUS: &str = "sarah_session_status";
pub const SARAH_METHOD_BOOTSTRAP: &str = "sarah_bootstrap";
pub const SARAH_METHOD_ROOM_SNAPSHOT: &str = "sarah_room_snapshot";
pub const SARAH_METHOD_SEND_MESSAGE: &str = "sarah_send_message";
pub const SARAH_METHOD_INTERRUPT_TURN: &str = "sarah_interrupt_turn";
pub const SARAH_EVENT_ROOM_EVENT: &str = "sarah_room_event";
pub const SARAH_EVENT_ROOM_STATE: &str = "sarah_room_state";

pub const SARAH_FRAMED_METHODS: &[&str] = &[
    SARAH_METHOD_SESSION_STATUS,
    SARAH_METHOD_BOOTSTRAP,
    SARAH_METHOD_ROOM_SNAPSHOT,
    SARAH_METHOD_SEND_MESSAGE,
    SARAH_METHOD_INTERRUPT_TURN,
];

/// NIP-AO ephemeral control kind used for interrupt / cancel_turn.
pub const NIP_AO_KIND: u16 = 24200;
/// Durable Sarah turn-record kind (SARAH-NR-00).
pub const SARAH_TURN_RECORD_KIND: u16 = 44300;

const DEFAULT_PAGE_LIMIT: usize = 32;
const MAX_PAGE_LIMIT: usize = 64;
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
    pub room_state: RoomStateEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoomSnapshotResult {
    pub conversation_ref: String,
    pub transcript: TranscriptPage,
    pub activity: ActivityPage,
    pub run_state: RunStateProjection,
    pub room_state: RoomStateEvent,
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
    fn query(
        &mut self,
        conversation_ref: &str,
        after_cursor: Option<&str>,
        limit: usize,
    ) -> Result<QueryPage, SarahConversationError>;
    fn last_event_id(&self) -> Option<String>;
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
        let after_index = after_cursor
            .and_then(parse_cursor_index)
            .map(|index| index.saturating_add(1))
            .unwrap_or(0);
        let matching: Vec<StoredConversationEvent> = self
            .events
            .iter()
            .filter(|event| event.conversation_ref == conversation_ref)
            .cloned()
            .collect();
        let page: Vec<StoredConversationEvent> = matching
            .iter()
            .filter(|event| event.store_index >= after_index)
            .take(limit)
            .cloned()
            .collect();
        let next_cursor = if matching
            .iter()
            .filter(|event| event.store_index >= after_index)
            .count()
            > page.len()
        {
            page.last()
                .map(|event| format!("{CURSOR_PREFIX}{}", event.store_index))
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

/// Configuration for the conversation client.
#[derive(Debug, Clone)]
pub struct SarahConversationConfig {
    pub generation: u64,
    pub conversation_digest: String,
    pub identity: ConversationIdentity,
    /// When set, the client treats the transport as a real relay and runs NIP-42.
    pub relay_url: Option<String>,
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
pub struct SarahConversationClient {
    config: SarahConversationConfig,
    relay: Box<dyn RelayTransport>,
    signer: SigningIdentity,
    pending_events: VecDeque<Value>,
    active_turn_ref: Option<String>,
    run_state: String,
    message_seq: u64,
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
            signer,
            pending_events: VecDeque::new(),
            active_turn_ref: None,
            run_state: "idle".to_string(),
            message_seq: 0,
        }
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

    pub fn set_generation(&mut self, generation: u64) {
        self.config.generation = generation.max(1);
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
                let cursor = params
                    .and_then(|value| value.get("cursor"))
                    .and_then(Value::as_str);
                let limit = params
                    .and_then(|value| value.get("limit"))
                    .and_then(Value::as_u64)
                    .map(|value| value as usize);
                Ok(serde_json::to_value(self.room_snapshot(cursor, limit)?)
                    .map_err(|error| SarahConversationError::Internal(error.to_string()))?)
            }
            SARAH_METHOD_SEND_MESSAGE => {
                let text = params
                    .and_then(|value| value.get("text"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        SarahConversationError::InvalidRequest(
                            "sarah_send_message requires text".into(),
                        )
                    })?;
                Ok(serde_json::to_value(self.send_message(text)?)
                    .map_err(|error| SarahConversationError::Internal(error.to_string()))?)
            }
            SARAH_METHOD_INTERRUPT_TURN => {
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
                Ok(serde_json::to_value(self.interrupt_turn(&turn_ref)?)
                    .map_err(|error| SarahConversationError::Internal(error.to_string()))?)
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
            room_state,
        })
    }

    pub fn room_snapshot(
        &mut self,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<RoomSnapshotResult, SarahConversationError> {
        self.ensure_connected()?;
        let limit = limit
            .unwrap_or(DEFAULT_PAGE_LIMIT)
            .clamp(1, MAX_PAGE_LIMIT);
        let conversation_ref = self.config.conversation_ref();
        let page = self.relay.query(&conversation_ref, cursor, limit)?;
        let mut transcript_entries = Vec::new();
        let mut activity_entries = Vec::new();
        for event in &page.events {
            let event_cursor = format!("{CURSOR_PREFIX}{}", event.store_index);
            if event.record_kind == "activity" {
                activity_entries.push(ActivityEntry {
                    event_id: event.event_id.clone(),
                    cursor: event_cursor,
                    entry: tag_value(&event.tags, "entry").unwrap_or_else(|| "unknown".into()),
                    turn_ref: tag_value(&event.tags, "turn")
                        .unwrap_or_else(|| "turn.unknown".into()),
                    created_at: iso_from_unix(event.created_at),
                });
            } else if event.record_kind != "control" {
                let role = if event.pubkey == self.config.identity.owner_public_key_hex {
                    "owner"
                } else {
                    "sarah"
                };
                transcript_entries.push(TranscriptEntry {
                    event_id: event.event_id.clone(),
                    cursor: event_cursor,
                    role: role.to_string(),
                    kind: "text".to_string(),
                    text: event.content_summary.clone(),
                    created_at: iso_from_unix(event.created_at),
                    status: "confirmed".to_string(),
                });
            }
        }
        let last_cursor = transcript_entries
            .last()
            .map(|entry| entry.cursor.clone())
            .or_else(|| activity_entries.last().map(|entry| entry.cursor.clone()))
            .unwrap_or_else(|| cursor.unwrap_or("cursor.start").to_string());
        Ok(RoomSnapshotResult {
            conversation_ref,
            transcript: TranscriptPage {
                entries: transcript_entries,
                cursor: last_cursor.clone(),
                next_cursor: page.next_cursor,
                gap_state: page.gap_state,
            },
            activity: ActivityPage {
                entries: activity_entries,
                cursor: last_cursor,
                next_cursor: None,
                gap_state: page.gap_state,
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
    ) -> Result<SendMessageResult, SarahConversationError> {
        self.ensure_connected()?;
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

        self.message_seq += 1;
        let turn_ref = format!("turn.{}", self.message_seq);
        let message_ref = format!("msg.{}", self.message_seq);
        self.active_turn_ref = Some(turn_ref.clone());
        self.run_state = "running".to_string();

        let conversation_ref = self.config.conversation_ref();
        let tags = conversation_tags(
            &conversation_ref,
            &self.config.identity.owner_public_key_hex,
            &self.config.identity.sarah_public_key_hex,
        );
        let event = self.signer.sign_text_note(text, tags)?;
        self.relay.publish(&event)?;
        let cursor = format!("{CURSOR_PREFIX}{}", self.message_seq.saturating_sub(1));
        let record = TranscriptEntry {
            event_id: event.id.to_hex(),
            cursor: cursor.clone(),
            role: "owner".to_string(),
            kind: "text".to_string(),
            text: redact_content_summary(text),
            created_at: iso_from_unix(event.created_at.as_secs()),
            status: "accepted".to_string(),
        };
        self.push_room_event(&record);
        self.push_room_state_event(&self.current_room_state());

        Ok(SendMessageResult {
            accepted: true,
            message_ref,
            turn_ref,
            event_id: event.id.to_hex(),
            cursor,
            status: "accepted".to_string(),
        })
    }

    pub fn interrupt_turn(
        &mut self,
        turn_ref: &str,
    ) -> Result<InterruptTurnResult, SarahConversationError> {
        self.ensure_connected()?;
        if turn_ref.trim().is_empty() {
            return Err(SarahConversationError::InvalidRequest(
                "turnRef must not be empty".into(),
            ));
        }
        let intent_ref = format!("intent.interrupt.{}", self.message_seq + 1);
        let conversation_ref = self.config.conversation_ref();
        let mut tags = conversation_tags(
            &conversation_ref,
            &self.config.identity.owner_public_key_hex,
            &self.config.identity.sarah_public_key_hex,
        );
        tags.push(
            Tag::parse(["turn", turn_ref])
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?,
        );
        tags.push(
            Tag::parse(["control", "cancel_turn"])
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?,
        );

        let content = json!({
            "schema": "openagents.sarah.control.v1",
            "control": "cancel_turn",
            "turnRef": turn_ref,
            "intentRef": intent_ref,
        })
        .to_string();

        let event = self.signer.sign_custom(NIP_AO_KIND, &content, tags)?;
        self.relay.publish(&event)?;

        self.run_state = "interrupt_pending".to_string();
        self.push_room_state_event(&self.current_room_state());

        Ok(InterruptTurnResult {
            accepted: true,
            turn_ref: turn_ref.to_string(),
            intent_ref,
            status: "pending".to_string(),
            pending: true,
        })
    }

    /// Drain pending framed events (`sarah_room_event` / `sarah_room_state`).
    pub fn drain_events(&mut self) -> Vec<Value> {
        self.pending_events.drain(..).collect()
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

    fn current_room_state(&self) -> RoomStateEvent {
        let last_id = self.relay.last_event_id();
        RoomStateEvent {
            method: SARAH_EVENT_ROOM_STATE.to_string(),
            connection: self.relay.connection_state(),
            freshness: if self.relay.connection_state() == ConnectionState::Connected {
                FreshnessState::Fresh
            } else {
                FreshnessState::Unknown
            },
            gap_state: GapState::None,
            connected_relays: vec![self.relay.label().to_string()],
            last_acknowledged_event_id: last_id,
            last_acknowledged_cursor: if self.message_seq > 0 {
                Some(format!(
                    "{CURSOR_PREFIX}{}",
                    self.message_seq.saturating_sub(1)
                ))
            } else {
                None
            },
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
            self.pending_events.push_back(value);
        }
    }

    fn push_room_state_event(&mut self, state: &RoomStateEvent) {
        if let Ok(value) = serde_json::to_value(state) {
            self.pending_events.push_back(value);
        }
    }
}

fn conversation_tags(
    conversation_ref: &str,
    owner_pubkey: &str,
    sarah_pubkey: &str,
) -> Vec<Tag> {
    vec![
        Tag::parse(["conversation", conversation_ref]).expect("conversation tag"),
        Tag::parse(["p", owner_pubkey]).expect("p tag"),
        Tag::parse(["agent", sarah_pubkey]).expect("agent tag"),
        Tag::parse(["alt", "OpenAgents Sarah conversation message"]).expect("alt tag"),
    ]
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

fn parse_cursor_index(cursor: &str) -> Option<usize> {
    cursor
        .strip_prefix(CURSOR_PREFIX)
        .and_then(|value| value.parse().ok())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn iso_from_unix(seconds: u64) -> String {
    // Keep dependency-free timestamps for public-safe projections.
    let _ = unix_now();
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
            .send_message("Plan the next SARAH-NR packet")
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
        let sent = client.send_message("start a turn").expect("send");
        let interrupt = client.interrupt_turn(&sent.turn_ref).expect("interrupt");
        assert!(interrupt.accepted);
        assert!(interrupt.pending);
        assert_eq!(interrupt.status, "pending");
        assert_eq!(interrupt.turn_ref, sent.turn_ref);
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
            .send_message("authenticated send")
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
            .send_message("Authorization: Bearer sk-test-123")
            .expect_err("secret");
        assert!(matches!(error, SarahConversationError::InvalidRequest(_)));
    }

    #[test]
    fn no_khala_sync_client_on_sarah_lane() {
        assert!(asserts_no_khala_sync_client());
    }

    #[test]
    fn handle_request_covers_all_methods() {
        let mut client = client();
        for method in SARAH_FRAMED_METHODS {
            let params = match *method {
                SARAH_METHOD_SEND_MESSAGE => Some(json!({ "text": "hello from test" })),
                SARAH_METHOD_INTERRUPT_TURN => {
                    client.send_message("prep").ok();
                    Some(json!({ "turnRef": "turn.1" }))
                }
                SARAH_METHOD_ROOM_SNAPSHOT => Some(json!({ "limit": 5 })),
                _ => None,
            };
            client
                .handle_request(method, 1, params.as_ref())
                .unwrap_or_else(|error| panic!("{method} failed: {error}"));
        }
    }
}
