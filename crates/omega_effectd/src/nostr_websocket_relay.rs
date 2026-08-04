use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_io::Timer;
use async_tungstenite::WebSocketStream;
use async_tungstenite::async_std::{ConnectStream, connect_async};
use async_tungstenite::tungstenite::Message;
use futures::{FutureExt, StreamExt, pin_mut, select};
#[cfg(any(test, feature = "test-support"))]
use nostr::Keys;
#[cfg(any(test, feature = "test-support"))]
use nostr::nips::nip59;
use nostr::{Event, EventBuilder, JsonUtil, Kind, PublicKey, RelayUrl};

#[cfg(any(test, feature = "test-support"))]
async fn nip59_extract_rumor(
    keys: &Keys,
    event: &Event,
) -> Result<nip59::UnwrappedGift, nostr::nips::nip59::Error> {
    nip59::extract_rumor(keys, event).await
}
use omega_identity::{
    AdmittedSigningRequest, IdentityService, NostrPublicKeyHex, ProofRef, ReceiptRef,
    RelayAuthenticationProjection, RelayAuthenticationReceipt, RelayAuthenticationRefusal,
    RelayConnectionAuthenticationState, SigningPurpose, UnsignedEventTemplate,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::sarah_conversation::{
    ConnectionState, GapState, QueryPage, RelayAuthChallenge, RelayTransport,
    SarahConversationError, StoredConversationEvent,
};
use crate::{
    ISSUE31_COMMAND_SCHEMA, ISSUE31_COMMAND_SCHEMA_V2, ISSUE31_HOST_DISCOVERY_KIND,
    ISSUE31_HOST_DISCOVERY_SCHEMA_V2, ISSUE31_HOST_DISCOVERY_SCHEMA_V3,
    ISSUE31_OWNER_PROJECTION_SCHEMA, ISSUE31_PAIRING_SCHEMA, Issue31CommandRecord,
    Issue31CommandRecordV2, Issue31HostDiscovery, Issue31HostDiscoveryV2, Issue31HostDiscoveryV3,
    Issue31OwnerProjectionRecord, Issue31PairingRecord,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(8);
const QUERY_TIMEOUT: Duration = Duration::from_secs(12);
const QUERY_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONNECT_ROUNDS: usize = 2;
const INITIAL_BACKOFF: Duration = Duration::from_millis(100);
const MAX_BACKOFF: Duration = Duration::from_millis(800);
const MAX_PENDING_PUBLICATIONS: usize = 4_096;
const MAX_CACHED_EVENTS: usize = 8_192;
const MAX_CACHED_CONTENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_CONTROL_FRAMES_PER_READ: usize = 64;
const MAX_AUTH_CHALLENGE_BYTES: usize = 4_096;
const MAX_RECENT_AUTH_PROOFS: usize = 4_096;
const NIP42_FRESHNESS_WINDOW: u64 = 60;

static RELAY_AUTHENTICATION_RECEIPTS: OnceLock<
    Mutex<BTreeMap<(String, String), RelayAuthenticationReceipt>>,
> = OnceLock::new();
static NEXT_RELAY_CONNECTION_GENERATION: AtomicU64 = AtomicU64::new(1);

#[allow(clippy::disallowed_methods)]
fn network_timer(duration: Duration) -> Timer {
    // The standalone omega-effectd child has no GPUI application context to supply a timer.
    Timer::after(duration)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

const NIP_AE_KIND: u16 = 30174;
const NIP_RS_KIND: u16 = 30078;
const NIP_ER_KIND: u16 = 30300;
const NIP_29_GROUP_CHAT_KIND: u16 = 9;
const LBR_AGENTIC_CODING_REQUEST_KIND: u16 = 5934;
const LBR_AGENTIC_CODING_RESULT_KIND: u16 = 6934;
const LBR_FEEDBACK_KIND: u16 = 7000;
const LBR_KINDS: &[u16] = &[
    LBR_AGENTIC_CODING_REQUEST_KIND,
    LBR_AGENTIC_CODING_RESULT_KIND,
    LBR_FEEDBACK_KIND,
];
const ISSUE31_COMMUNITY_KINDS: &[u16] = &[
    NIP_29_GROUP_CHAT_KIND,
    LBR_AGENTIC_CODING_REQUEST_KIND,
    LBR_AGENTIC_CODING_RESULT_KIND,
    LBR_FEEDBACK_KIND,
];
const SARAH_RECORD_KINDS: &[u16] = &[24200, 44200, 44300, 44301];
/// Block NIP-AO (24200) and NIP-AM (44200) are recipient-private: the relay
/// refuses a read scoped only by `authors` and requires `#p` = the
/// authenticated pubkey. Keep them in their own filter.
const SARAH_RECIPIENT_PRIVATE_KINDS: &[u16] = &[24200, 44200];
/// The remaining Sarah record kinds are author-scoped and read normally.
const SARAH_AUTHORED_RECORD_KINDS: &[u16] = &[44300, 44301];

enum IncomingMessage {
    Json(Value),
    TimedOut,
}

pub fn relay_authentication_projection() -> RelayAuthenticationProjection {
    let receipts = RELAY_AUTHENTICATION_RECEIPTS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    RelayAuthenticationProjection {
        relays: receipts.values().cloned().collect(),
    }
}

#[derive(Debug, Clone)]
struct PendingRelayAuthentication {
    challenge: String,
    challenge_ref: ProofRef,
    connection_generation: u64,
}

/// A bounded, failover-capable NIP-01 relay transport for the Sarah workroom.
///
/// The surrounding effectd protocol is synchronous today, so each operation
/// drives the non-blocking socket to a bounded completion. Relay `OK` messages
/// are transport acknowledgements only. They are never projected as command
/// completion.
pub struct WebSocketRelayAdapter {
    relay_urls: Vec<String>,
    active_relay_index: usize,
    active_label: String,
    socket: Option<WebSocketStream<ConnectStream>>,
    custody: RelayCustody,
    owner_public_key_hex: String,
    owner_public_key: NostrPublicKeyHex,
    sarah_public_key_hex: String,
    community_group_ids: Vec<String>,
    community_public_key_hexes: Vec<String>,
    authenticated: bool,
    auth_challenge: Option<PendingRelayAuthentication>,
    connection_generation: u64,
    relay_authentication_receipts: BTreeMap<String, RelayAuthenticationReceipt>,
    recent_auth_event_ids: HashSet<String>,
    recent_auth_event_order: VecDeque<String>,
    acknowledged_event_id: Option<String>,
    publish_acknowledgements: BTreeMap<String, HashSet<String>>,
    healthy_relays: HashSet<String>,
    events: BTreeMap<String, StoredConversationEvent>,
    subscription_sequence: u64,
    gap_state: GapState,
    event_cache_truncated: bool,
}

enum RelayCustody {
    Omega(Arc<IdentityService>),
    #[cfg(any(test, feature = "test-support"))]
    Keys(Keys),
}

impl WebSocketRelayAdapter {
    pub fn new(
        relay_urls: Vec<String>,
        identity_service: Arc<IdentityService>,
        sarah_public_key_hex: String,
        community_group_ids: Vec<String>,
        community_public_key_hexes: Vec<String>,
    ) -> Result<Self, SarahConversationError> {
        let owner_public_key_hex = identity_service
            .inspect()
            .map_err(|error| SarahConversationError::Identity(error.to_string()))?
            .identity
            .map(|identity| identity.public_key_hex().as_str().to_string())
            .ok_or(SarahConversationError::IdentityRequired)?;
        Self::with_custody(
            relay_urls,
            RelayCustody::Omega(identity_service),
            owner_public_key_hex,
            sarah_public_key_hex,
            community_group_ids,
            community_public_key_hexes,
        )
    }

    /// A relay adapter whose owner key is supplied directly.
    ///
    /// Production custody goes through `new`, which reads the key from
    /// `omega_identity::IdentityService`. This exists for proof harnesses that
    /// must run without the GPUI app, and is compiled only under `test` or the
    /// explicit `test-support` feature so it can never reach a shipped build.
    #[cfg(any(test, feature = "test-support"))]
    pub fn new_for_keys(
        relay_urls: Vec<String>,
        keys: Keys,
    ) -> Result<Self, SarahConversationError> {
        let public_key_hex = keys.public_key().to_hex();
        Self::with_custody(
            relay_urls,
            RelayCustody::Keys(keys),
            public_key_hex.clone(),
            public_key_hex,
            Vec::new(),
            Vec::new(),
        )
    }

    /// The same headless adapter, with Sarah and community policy stated.
    #[cfg(any(test, feature = "test-support"))]
    pub fn new_for_keys_with_policy(
        relay_urls: Vec<String>,
        owner_keys: Keys,
        sarah_public_key_hex: String,
        community_group_ids: Vec<String>,
        community_public_key_hexes: Vec<String>,
    ) -> Result<Self, SarahConversationError> {
        let owner_public_key_hex = owner_keys.public_key().to_hex();
        Self::with_custody(
            relay_urls,
            RelayCustody::Keys(owner_keys),
            owner_public_key_hex,
            sarah_public_key_hex,
            community_group_ids,
            community_public_key_hexes,
        )
    }

    fn with_custody(
        relay_urls: Vec<String>,
        custody: RelayCustody,
        owner_public_key_hex: String,
        sarah_public_key_hex: String,
        community_group_ids: Vec<String>,
        community_public_key_hexes: Vec<String>,
    ) -> Result<Self, SarahConversationError> {
        let mut unique = HashSet::new();
        let mut normalized_relay_urls = Vec::new();
        for relay_url in relay_urls {
            let relay_url = relay_url.trim();
            if relay_url.is_empty() {
                continue;
            }
            let relay_url = normalize_relay_url(relay_url)?;
            if unique.insert(relay_url.clone()) {
                normalized_relay_urls.push(relay_url);
            }
        }
        let relay_urls = normalized_relay_urls;
        if relay_urls.is_empty() {
            return Err(SarahConversationError::InvalidRequest(
                "at least one relay URL is required".into(),
            ));
        }
        if relay_urls.len() > 8 {
            return Err(SarahConversationError::InvalidRequest(
                "at most eight relay URLs are supported".into(),
            ));
        }
        if PublicKey::from_hex(&owner_public_key_hex).is_err()
            || PublicKey::from_hex(&sarah_public_key_hex).is_err()
        {
            return Err(SarahConversationError::InvalidRequest(
                "relay subscription requires valid owner and Sarah public keys".into(),
            ));
        }
        let community_group_ids = normalized_group_ids(community_group_ids)?;
        let community_public_key_hexes =
            normalized_public_keys(community_public_key_hexes, "community author")?;
        let owner_public_key = NostrPublicKeyHex::new(&owner_public_key_hex)
            .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
        let active_label = relay_urls
            .first()
            .cloned()
            .ok_or_else(|| SarahConversationError::Internal("relay list disappeared".into()))?;
        Ok(Self {
            relay_urls,
            active_relay_index: 0,
            active_label,
            socket: None,
            custody,
            owner_public_key_hex,
            owner_public_key,
            sarah_public_key_hex,
            community_group_ids,
            community_public_key_hexes,
            authenticated: false,
            auth_challenge: None,
            connection_generation: 0,
            relay_authentication_receipts: BTreeMap::new(),
            recent_auth_event_ids: HashSet::new(),
            recent_auth_event_order: VecDeque::new(),
            acknowledged_event_id: None,
            publish_acknowledgements: BTreeMap::new(),
            healthy_relays: HashSet::new(),
            events: BTreeMap::new(),
            subscription_sequence: 0,
            gap_state: GapState::None,
            event_cache_truncated: false,
        })
    }

    fn custody_public_key_hex(&self) -> Result<String, SarahConversationError> {
        match &self.custody {
            RelayCustody::Omega(identity_service) => identity_service
                .inspect()
                .map_err(|error| SarahConversationError::Identity(error.to_string()))?
                .identity
                .map(|identity| identity.public_key_hex().as_str().to_string())
                .ok_or(SarahConversationError::IdentityRequired),
            #[cfg(any(test, feature = "test-support"))]
            RelayCustody::Keys(keys) => Ok(keys.public_key().to_hex()),
        }
    }

    fn prune_publication_acknowledgements(&mut self, preserved_event_id: &str) {
        while self.publish_acknowledgements.len() > MAX_PENDING_PUBLICATIONS {
            let Some(oldest_event_id) = self
                .publish_acknowledgements
                .keys()
                .find(|event_id| event_id.as_str() != preserved_event_id)
                .cloned()
            else {
                break;
            };
            self.publish_acknowledgements.remove(&oldest_event_id);
        }
    }

    pub fn relay_urls(&self) -> &[String] {
        &self.relay_urls
    }

    pub fn relay_authentication_projection(&self) -> RelayAuthenticationProjection {
        RelayAuthenticationProjection {
            relays: self
                .relay_authentication_receipts
                .values()
                .cloned()
                .collect(),
        }
    }

    fn record_relay_authentication(
        &mut self,
        state: RelayConnectionAuthenticationState,
        challenge_ref: Option<ProofRef>,
        auth_event_id: Option<String>,
        refusal: Option<RelayAuthenticationRefusal>,
    ) {
        self.relay_authentication_receipts.insert(
            self.active_label.clone(),
            RelayAuthenticationReceipt {
                relay_url: self.active_label.clone(),
                account_public_key_hex: self.owner_public_key.clone(),
                connection_generation: self.connection_generation.max(1),
                state,
                challenge_ref,
                auth_event_id,
                refusal,
                observed_at: unix_now().max(1),
            },
        );
        if let Some(receipt) = self
            .relay_authentication_receipts
            .get(&self.active_label)
            .cloned()
        {
            let mut receipts = RELAY_AUTHENTICATION_RECEIPTS
                .get_or_init(|| Mutex::new(BTreeMap::new()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            receipts.insert(
                (
                    self.owner_public_key.as_str().to_string(),
                    self.active_label.clone(),
                ),
                receipt,
            );
        }
    }

    fn remember_auth_event(&mut self, event_id: String) -> bool {
        if !self.recent_auth_event_ids.insert(event_id.clone()) {
            return false;
        }
        self.recent_auth_event_order.push_back(event_id);
        while self.recent_auth_event_order.len() > MAX_RECENT_AUTH_PROOFS {
            if let Some(expired) = self.recent_auth_event_order.pop_front() {
                self.recent_auth_event_ids.remove(&expired);
            }
        }
        true
    }

    fn connect_active(&mut self) -> Result<(), SarahConversationError> {
        self.connect_active_until(Instant::now() + CONNECT_TIMEOUT)
    }

    fn connect_active_until(&mut self, deadline: Instant) -> Result<(), SarahConversationError> {
        let relay_url = self
            .relay_urls
            .get(self.active_relay_index)
            .cloned()
            .ok_or_else(|| SarahConversationError::Internal("active relay disappeared".into()))?;
        let timeout_duration = deadline.saturating_duration_since(Instant::now());
        if timeout_duration.is_zero() {
            return Err(SarahConversationError::Relay(
                "relay operation deadline elapsed before connect".into(),
            ));
        }
        let connection = smol::block_on(async {
            let connect = connect_async(relay_url.as_str()).fuse();
            let timeout = FutureExt::fuse(network_timer(timeout_duration.min(CONNECT_TIMEOUT)));
            pin_mut!(connect, timeout);
            select! {
                result = connect => Some(result),
                _ = timeout => None,
            }
        });
        let (socket, _) = connection
            .ok_or_else(|| SarahConversationError::Relay("relay connect timed out".into()))?
            .map_err(|error| SarahConversationError::Relay(error.to_string()))?;
        self.active_label = relay_url;
        self.socket = Some(socket);
        self.connection_generation =
            NEXT_RELAY_CONNECTION_GENERATION.fetch_add(1, Ordering::Relaxed);
        if self.connection_generation == 0 {
            self.connection_generation =
                NEXT_RELAY_CONNECTION_GENERATION.fetch_add(1, Ordering::Relaxed);
        }
        self.authenticated = false;
        self.auth_challenge = None;
        Ok(())
    }

    fn disconnect_and_advance(&mut self) {
        self.close_active_connection();
        self.gap_state = GapState::Recovering;
        self.active_relay_index = (self.active_relay_index + 1) % self.relay_urls.len().max(1);
    }

    fn close_active_connection(&mut self) {
        if self.connection_generation > 0 {
            self.record_relay_authentication(
                RelayConnectionAuthenticationState::Disconnected,
                None,
                None,
                None,
            );
        }
        self.socket = None;
        self.authenticated = false;
        self.auth_challenge = None;
    }

    fn send_json(&mut self, payload: Value) -> Result<(), SarahConversationError> {
        self.send_json_until(payload, Instant::now() + NETWORK_TIMEOUT)
    }

    fn send_json_until(
        &mut self,
        payload: Value,
        deadline: Instant,
    ) -> Result<(), SarahConversationError> {
        let socket = self
            .socket
            .as_mut()
            .ok_or_else(|| SarahConversationError::Relay("not connected".into()))?;
        let text = serde_json::to_string(&payload)
            .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
        let timeout_duration = deadline.saturating_duration_since(Instant::now());
        if timeout_duration.is_zero() {
            return Err(SarahConversationError::Relay(
                "relay operation deadline elapsed before write".into(),
            ));
        }
        let result = smol::block_on(async {
            let send = socket.send(Message::Text(text.into())).fuse();
            let timeout = FutureExt::fuse(network_timer(timeout_duration.min(NETWORK_TIMEOUT)));
            pin_mut!(send, timeout);
            select! {
                result = send => Some(result),
                _ = timeout => None,
            }
        });
        result
            .ok_or_else(|| SarahConversationError::Relay("relay write timed out".into()))?
            .map_err(|error| SarahConversationError::Relay(error.to_string()))
    }

    fn next_message_until(
        &mut self,
        deadline: Instant,
    ) -> Result<IncomingMessage, SarahConversationError> {
        let mut control_frames = 0_usize;
        loop {
            let socket = self
                .socket
                .as_mut()
                .ok_or_else(|| SarahConversationError::Relay("not connected".into()))?;
            let timeout_duration = deadline.saturating_duration_since(Instant::now());
            if timeout_duration.is_zero() {
                return Ok(IncomingMessage::TimedOut);
            }
            let message = smol::block_on(async {
                let read = socket.next().fuse();
                let timeout = FutureExt::fuse(network_timer(timeout_duration));
                pin_mut!(read, timeout);
                select! {
                    result = read => Some(result),
                    _ = timeout => None,
                }
            });
            let Some(message) = message else {
                return Ok(IncomingMessage::TimedOut);
            };
            let message = message
                .ok_or_else(|| SarahConversationError::Relay("relay closed connection".into()))?
                .map_err(|error| SarahConversationError::Relay(error.to_string()))?;
            match message {
                Message::Text(text) => {
                    if text.len() > 2 * 1024 * 1024 {
                        return Err(SarahConversationError::Relay(
                            "relay frame exceeds the inbound budget".into(),
                        ));
                    }
                    let value = serde_json::from_str(text.as_str())
                        .map_err(|error| SarahConversationError::Relay(error.to_string()))?;
                    return Ok(IncomingMessage::Json(value));
                }
                Message::Binary(bytes) => {
                    if bytes.len() > 2 * 1024 * 1024 {
                        return Err(SarahConversationError::Relay(
                            "relay frame exceeds the inbound budget".into(),
                        ));
                    }
                    let text = std::str::from_utf8(&bytes).map_err(|error| {
                        SarahConversationError::Relay(format!(
                            "relay sent non-UTF-8 binary frame: {error}"
                        ))
                    })?;
                    let value = serde_json::from_str(text)
                        .map_err(|error| SarahConversationError::Relay(error.to_string()))?;
                    return Ok(IncomingMessage::Json(value));
                }
                Message::Ping(payload) => {
                    record_control_frame(&mut control_frames)?;
                    let socket = self.socket.as_mut().ok_or_else(|| {
                        SarahConversationError::Relay("relay disconnected during ping".into())
                    })?;
                    let result = smol::block_on(async {
                        let send = socket.send(Message::Pong(payload)).fuse();
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        let timeout =
                            FutureExt::fuse(network_timer(remaining.min(NETWORK_TIMEOUT)));
                        pin_mut!(send, timeout);
                        select! {
                            result = send => Some(result),
                            _ = timeout => None,
                        }
                    });
                    result
                        .ok_or_else(|| {
                            SarahConversationError::Relay("relay pong timed out".into())
                        })?
                        .map_err(|error| SarahConversationError::Relay(error.to_string()))?;
                }
                Message::Pong(_) | Message::Frame(_) => {
                    record_control_frame(&mut control_frames)?;
                }
                Message::Close(_) => {
                    return Err(SarahConversationError::Relay(
                        "relay closed connection".into(),
                    ));
                }
            }
        }
    }

    fn record_auth_challenge(&mut self, value: &Value) -> Result<bool, SarahConversationError> {
        let Some(array) = value.as_array() else {
            return Ok(false);
        };
        if array.first().and_then(Value::as_str) != Some("AUTH") {
            return Ok(false);
        }
        let Some(challenge) = (array.len() == 2)
            .then(|| array.get(1).and_then(Value::as_str))
            .flatten()
            .filter(|challenge| {
                !challenge.is_empty() && challenge.len() <= MAX_AUTH_CHALLENGE_BYTES
            })
        else {
            self.auth_challenge = None;
            self.authenticated = false;
            let challenge_ref = challenge_reference(
                &self.active_label,
                self.connection_generation,
                &value.to_string(),
            )?;
            self.record_relay_authentication(
                RelayConnectionAuthenticationState::Refused,
                Some(challenge_ref),
                None,
                Some(RelayAuthenticationRefusal::MalformedChallenge),
            );
            return Err(SarahConversationError::Relay(
                "relay sent a malformed NIP-42 challenge".into(),
            ));
        };
        let challenge_ref =
            challenge_reference(&self.active_label, self.connection_generation, challenge)?;
        self.auth_challenge = Some(PendingRelayAuthentication {
            challenge: challenge.to_owned(),
            challenge_ref: challenge_ref.clone(),
            connection_generation: self.connection_generation,
        });
        self.authenticated = false;
        self.record_relay_authentication(
            RelayConnectionAuthenticationState::ChallengePending,
            Some(challenge_ref),
            None,
            None,
        );
        Ok(true)
    }

    fn publish_once(&mut self, event: &Event) -> Result<(), SarahConversationError> {
        self.send_json(json!(["EVENT", event]))?;
        let deadline = Instant::now() + NETWORK_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(SarahConversationError::Relay(
                    "relay acknowledgement timed out".into(),
                ));
            }
            match self.next_message_until(deadline)? {
                IncomingMessage::TimedOut => {
                    return Err(SarahConversationError::Relay(
                        "relay acknowledgement timed out".into(),
                    ));
                }
                IncomingMessage::Json(value) => {
                    if self.record_auth_challenge(&value)? {
                        return Err(SarahConversationError::IdentityRequired);
                    }
                    let Some(array) = value.as_array() else {
                        continue;
                    };
                    if array.first().and_then(Value::as_str) != Some("OK")
                        || array.get(1).and_then(Value::as_str) != Some(event.id.to_hex().as_str())
                    {
                        continue;
                    }
                    if array.get(2).and_then(Value::as_bool) == Some(true) {
                        self.acknowledged_event_id = Some(event.id.to_hex());
                        return Ok(());
                    }
                    let reason = array
                        .get(3)
                        .and_then(Value::as_str)
                        .unwrap_or("relay rejected event");
                    return Err(SarahConversationError::Relay(reason.to_string()));
                }
            }
        }
    }

    fn query_community_once(
        &mut self,
        coordinate: &str,
    ) -> Result<Vec<Event>, SarahConversationError> {
        self.subscription_sequence = self.subscription_sequence.saturating_add(1);
        let subscription_id = format!("omega-community-{}", self.subscription_sequence);
        self.send_json(json!([
            "REQ",
            subscription_id,
            {
                "kinds": [1111],
                "#A": [coordinate],
                "limit": 128
            }
        ]))?;
        let deadline = Instant::now() + QUERY_TIMEOUT;
        let mut events = BTreeMap::new();
        loop {
            match self.next_message_until(deadline)? {
                IncomingMessage::TimedOut => {
                    return Err(SarahConversationError::Relay(
                        "community relay query timed out".into(),
                    ));
                }
                IncomingMessage::Json(value) => {
                    if self.record_auth_challenge(&value)? {
                        return Err(SarahConversationError::IdentityRequired);
                    }
                    let Some(frame) = value.as_array() else {
                        continue;
                    };
                    match frame.first().and_then(Value::as_str) {
                        Some("EVENT")
                            if frame.get(1).and_then(Value::as_str)
                                == Some(subscription_id.as_str()) =>
                        {
                            let Some(event) = frame.get(2) else {
                                continue;
                            };
                            let event = Event::from_json(event.to_string()).map_err(|error| {
                                SarahConversationError::Relay(format!(
                                    "community relay returned an invalid event: {error}"
                                ))
                            })?;
                            if event.verify().is_ok() {
                                events.entry(event.id.to_hex()).or_insert(event);
                            }
                        }
                        Some("EOSE")
                            if frame.get(1).and_then(Value::as_str)
                                == Some(subscription_id.as_str()) =>
                        {
                            self.send_json(json!(["CLOSE", subscription_id]))?;
                            return Ok(events.into_values().collect());
                        }
                        Some("CLOSED")
                            if frame.get(1).and_then(Value::as_str)
                                == Some(subscription_id.as_str()) =>
                        {
                            let reason = frame
                                .get(2)
                                .and_then(Value::as_str)
                                .unwrap_or("relay closed the community subscription");
                            return Err(SarahConversationError::Relay(reason.to_string()));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// The order in which `publish` walks the configured relays.
    ///
    /// Normally left to right. But `publish` is the one operation that hands
    /// `IdentityRequired` back to its caller, and the caller answers the NIP-42
    /// challenge on whichever relay raised it and then retries exactly once.
    /// Restarting that retry at relay 0 would tear down the socket it just
    /// authenticated in order to re-attempt a relay that is down — and since a
    /// reconnect earns a fresh challenge, the retry would raise
    /// `IdentityRequired` again and the publish could never complete. A healthy
    /// authenticating relay listed after a dead one would be unreachable.
    ///
    /// So while an authenticated session is open, that relay is tried first and
    /// the rest follow. Every relay is still attempted; only the order moves.
    fn publish_relay_order(&self) -> Vec<usize> {
        publish_relay_order(
            self.relay_urls.len(),
            (self.authenticated && self.socket.is_some()).then_some(self.active_relay_index),
        )
    }

    fn query_once_until(
        &mut self,
        conversation_ref: &str,
        deadline: Instant,
    ) -> Result<GapState, SarahConversationError> {
        self.subscription_sequence = self.subscription_sequence.saturating_add(1);
        let subscription_id = format!("omega-issue31-{}", self.subscription_sequence);
        let mut request = vec![Value::String("REQ".into()), json!(subscription_id)];
        request.push(json!({
            "kinds": [Kind::GiftWrap.as_u16()],
            "#p": [self.custody_public_key_hex()?],
            "limit": 256
        }));
        request.push(json!({
            "kinds": SARAH_RECIPIENT_PRIVATE_KINDS,
            "#p": [self.custody_public_key_hex()?],
            "limit": 256
        }));
        request.push(json!({
            "kinds": SARAH_AUTHORED_RECORD_KINDS
                .iter()
                .copied()
                .chain([ISSUE31_HOST_DISCOVERY_KIND])
                .collect::<Vec<_>>(),
            "authors": [self.owner_public_key_hex.as_str(), self.sarah_public_key_hex.as_str()],
            "limit": 256
        }));
        request.push(json!({
            "kinds": [NIP_AE_KIND, NIP_ER_KIND],
            "authors": [self.owner_public_key_hex.as_str(), self.sarah_public_key_hex.as_str()],
            "limit": 256
        }));
        request.push(json!({
            "kinds": [NIP_RS_KIND],
            "authors": [self.owner_public_key_hex.as_str()],
            "limit": 256
        }));
        if !self.community_group_ids.is_empty() {
            request.push(json!({
                "kinds": [NIP_29_GROUP_CHAT_KIND],
                "#h": self.community_group_ids,
                "limit": 256
            }));
        }
        if !self.community_public_key_hexes.is_empty() {
            request.push(json!({
                "kinds": LBR_KINDS,
                "authors": self.community_public_key_hexes,
                "limit": 256
            }));
        }
        self.send_json_until(Value::Array(request), deadline)?;

        let mut admitted = Vec::new();
        let mut admission_truncated = false;
        let mut notice: Option<String> = None;
        let mut closed_reason: Option<String> = None;
        let query_result = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break GapState::Possible;
            }
            match self.next_message_until(deadline)? {
                IncomingMessage::TimedOut => break GapState::Possible,
                IncomingMessage::Json(value) => {
                    if self.record_auth_challenge(&value)? {
                        return Err(SarahConversationError::IdentityRequired);
                    }
                    let Some(array) = value.as_array() else {
                        continue;
                    };
                    match array.first().and_then(Value::as_str) {
                        Some("EVENT")
                            if array.get(1).and_then(Value::as_str)
                                == Some(subscription_id.as_str()) =>
                        {
                            let event_value = array.get(2).ok_or_else(|| {
                                SarahConversationError::Relay(
                                    "relay EVENT frame omitted event".into(),
                                )
                            })?;
                            let event =
                                Event::from_json(event_value.to_string()).map_err(|error| {
                                    SarahConversationError::Relay(error.to_string())
                                })?;
                            if !event_within_bounds(&event) || event.verify().is_err() {
                                continue;
                            }
                            match self.admit_event(&event, conversation_ref) {
                                Ok(Some(stored)) if admitted.len() < 256 => admitted.push(stored),
                                Ok(Some(_)) => admission_truncated = true,
                                Ok(None) | Err(_) => {}
                            }
                        }
                        Some("EOSE")
                            if array.get(1).and_then(Value::as_str)
                                == Some(subscription_id.as_str()) =>
                        {
                            break query_gap_after_eose(admitted.len(), admission_truncated);
                        }
                        // NIP-01: the relay has ended our subscription. There is
                        // no EOSE coming, so waiting out the deadline would only
                        // turn a clear answer into an opaque timeout.
                        Some("CLOSED")
                            if array.get(1).and_then(Value::as_str)
                                == Some(subscription_id.as_str()) =>
                        {
                            closed_reason = array
                                .get(2)
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                                .or_else(|| Some("relay closed the subscription".into()));
                            break GapState::Possible;
                        }
                        // A NOTICE is not subscription-scoped, so it does not end
                        // the read. But if this query goes on to time out, the
                        // relay's own words are a better explanation than a
                        // generic deadline message.
                        Some("NOTICE") => {
                            notice = array
                                .get(1)
                                .and_then(Value::as_str)
                                .map(|text| text.chars().take(200).collect::<String>());
                        }
                        _ => {}
                    }
                }
            }
        };
        if let Some(reason) = closed_reason {
            return Err(SarahConversationError::Relay(format!(
                "relay closed the query subscription: {reason}"
            )));
        }
        // Closing is courtesy. A relay that has stopped answering must not turn
        // a completed read into an error, and a query that timed out already
        // carries its own gap state — reporting a write failure instead would
        // hide the gap behind a transport message.
        if self
            .send_json_until(json!(["CLOSE", subscription_id]), deadline)
            .is_err()
            && admitted.is_empty()
            && query_result != GapState::None
            && let Some(notice) = notice
        {
            return Err(SarahConversationError::Relay(format!(
                "relay query failed: {notice}"
            )));
        }

        for event in admitted {
            self.events.entry(event.event_id.clone()).or_insert(event);
        }
        self.reindex_events();
        Ok(query_result)
    }

    fn query_active_relay_until(
        &mut self,
        conversation_ref: &str,
        deadline: Instant,
    ) -> Result<GapState, SarahConversationError> {
        let mut authentication_attempted = false;
        loop {
            match self.query_once_until(conversation_ref, deadline) {
                Err(SarahConversationError::IdentityRequired) => {
                    begin_authentication_retry(&mut authentication_attempted)?;
                    let auth_event = self.sign_active_auth_event()?;
                    self.authenticate_until(&auth_event, deadline)?;
                }
                result => return result,
            }
        }
    }

    fn sign_active_auth_event(&self) -> Result<Event, SarahConversationError> {
        let pending = self.auth_challenge.as_ref().ok_or_else(|| {
            SarahConversationError::Relay("no relay AUTH challenge is pending".into())
        })?;
        if pending.connection_generation != self.connection_generation {
            return Err(SarahConversationError::Relay(
                "relay AUTH challenge belongs to a stale connection".into(),
            ));
        }
        let challenge = pending.challenge.as_str();
        let relay = RelayUrl::parse(&self.active_label)
            .map_err(|error| SarahConversationError::Relay(error.to_string()))?;
        match &self.custody {
            RelayCustody::Omega(identity_service) => {
                let custody = identity_service
                    .inspect()
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                let identity = custody
                    .identity
                    .ok_or(SarahConversationError::IdentityRequired)?;
                let public_key = PublicKey::from_hex(identity.public_key_hex().as_str())
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                let unsigned = EventBuilder::auth(challenge, relay).build(public_key);
                let semantic_binding = serde_json::to_vec(&json!({
                    "relay": self.active_label,
                    "challenge": challenge,
                    "createdAt": unsigned.created_at.as_secs(),
                }))
                .map_err(|error| SarahConversationError::Internal(error.to_string()))?;
                let digest = format!("{:x}", Sha256::digest(semantic_binding));
                let request_ref = ReceiptRef::new(format!("nip42.{}", &digest[..32]))
                    .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
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
            #[cfg(any(test, feature = "test-support"))]
            RelayCustody::Keys(keys) => EventBuilder::auth(challenge, relay)
                .sign_with_keys(keys)
                .map_err(|error| SarahConversationError::Identity(error.to_string())),
        }
    }

    fn authenticate_until(
        &mut self,
        auth_event: &Event,
        deadline: Instant,
    ) -> Result<(), SarahConversationError> {
        let pending = self.auth_challenge.clone().ok_or_else(|| {
            SarahConversationError::Relay("no relay AUTH challenge is pending".into())
        })?;
        if pending.connection_generation != self.connection_generation {
            self.record_relay_authentication(
                RelayConnectionAuthenticationState::Stale,
                Some(pending.challenge_ref),
                Some(auth_event.id.to_hex()),
                Some(RelayAuthenticationRefusal::StaleConnection),
            );
            return Err(SarahConversationError::Relay(
                "relay AUTH proof belongs to a stale connection".into(),
            ));
        }
        let relay_url = RelayUrl::parse(&self.active_label)
            .map_err(|error| SarahConversationError::Relay(error.to_string()))?;
        let event_id = auth_event.id.to_hex();
        let challenge_ref = pending.challenge_ref.clone();
        let refusal = validate_nip42_auth_event(
            auth_event,
            &relay_url,
            &pending.challenge,
            &self.custody_public_key_hex()?,
            unix_now(),
        )
        .err();
        if let Some(refusal) = refusal {
            self.record_relay_authentication(
                RelayConnectionAuthenticationState::Refused,
                Some(challenge_ref),
                Some(event_id),
                Some(refusal),
            );
            return Err(SarahConversationError::Relay(
                "NIP-42 auth event failed local validation".into(),
            ));
        }
        if !self.remember_auth_event(event_id.clone()) {
            self.record_relay_authentication(
                RelayConnectionAuthenticationState::Refused,
                Some(challenge_ref),
                Some(event_id),
                Some(RelayAuthenticationRefusal::ReplayedProof),
            );
            return Err(SarahConversationError::Relay(
                "NIP-42 auth event was already used".into(),
            ));
        }
        let connection_generation = self.connection_generation;
        self.auth_challenge = None;
        if let Err(error) = self.send_json_until(json!(["AUTH", auth_event]), deadline) {
            self.record_relay_authentication(
                RelayConnectionAuthenticationState::Refused,
                Some(challenge_ref),
                Some(event_id),
                Some(RelayAuthenticationRefusal::AcknowledgementMissing),
            );
            return Err(error);
        }
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.record_relay_authentication(
                    RelayConnectionAuthenticationState::Refused,
                    Some(challenge_ref),
                    Some(event_id),
                    Some(RelayAuthenticationRefusal::AcknowledgementMissing),
                );
                return Err(SarahConversationError::Relay(
                    "NIP-42 acknowledgement timed out".into(),
                ));
            }
            let incoming = match self.next_message_until(deadline) {
                Ok(incoming) => incoming,
                Err(error) => {
                    self.record_relay_authentication(
                        RelayConnectionAuthenticationState::Refused,
                        Some(challenge_ref),
                        Some(event_id),
                        Some(RelayAuthenticationRefusal::AcknowledgementMissing),
                    );
                    return Err(error);
                }
            };
            match incoming {
                IncomingMessage::TimedOut => {
                    self.record_relay_authentication(
                        RelayConnectionAuthenticationState::Refused,
                        Some(challenge_ref),
                        Some(event_id),
                        Some(RelayAuthenticationRefusal::AcknowledgementMissing),
                    );
                    return Err(SarahConversationError::Relay(
                        "NIP-42 acknowledgement timed out".into(),
                    ));
                }
                IncomingMessage::Json(value) => {
                    let Some(array) = value.as_array() else {
                        continue;
                    };
                    if array.first().and_then(Value::as_str) != Some("OK")
                        || array.get(1).and_then(Value::as_str) != Some(event_id.as_str())
                    {
                        continue;
                    }
                    if array.len() != 4
                        || array.get(2).and_then(Value::as_bool).is_none()
                        || array.get(3).and_then(Value::as_str).is_none()
                    {
                        self.record_relay_authentication(
                            RelayConnectionAuthenticationState::Refused,
                            Some(challenge_ref),
                            Some(event_id),
                            Some(RelayAuthenticationRefusal::RelayRejected),
                        );
                        return Err(SarahConversationError::Relay(
                            "relay sent a malformed NIP-42 acknowledgement".into(),
                        ));
                    }
                    if array.get(2).and_then(Value::as_bool) != Some(true) {
                        self.record_relay_authentication(
                            RelayConnectionAuthenticationState::Refused,
                            Some(challenge_ref),
                            Some(event_id),
                            Some(RelayAuthenticationRefusal::RelayRejected),
                        );
                        return Err(SarahConversationError::Relay(
                            "NIP-42 authentication rejected".into(),
                        ));
                    }
                    if self.connection_generation != connection_generation
                        || self.active_label != relay_url.as_str()
                    {
                        self.record_relay_authentication(
                            RelayConnectionAuthenticationState::Stale,
                            Some(challenge_ref),
                            Some(event_id),
                            Some(RelayAuthenticationRefusal::StaleConnection),
                        );
                        return Err(SarahConversationError::Relay(
                            "NIP-42 acknowledgement belongs to a stale connection".into(),
                        ));
                    }
                    self.authenticated = true;
                    self.record_relay_authentication(
                        RelayConnectionAuthenticationState::Authenticated,
                        Some(challenge_ref),
                        Some(event_id),
                        None,
                    );
                    return Ok(());
                }
            }
        }
    }

    fn admit_event(
        &self,
        event: &Event,
        conversation_ref: &str,
    ) -> Result<Option<StoredConversationEvent>, SarahConversationError> {
        if event.kind == Kind::GiftWrap {
            let gift_wrap_event_json = event
                .try_as_json()
                .map_err(|error| SarahConversationError::Relay(error.to_string()))?;
            let (rumor_event_id, sender_public_key_hex, created_at, tags, content) =
                match &self.custody {
                    RelayCustody::Omega(identity_service) => {
                        let gift = identity_service
                            .unwrap_private_message(&gift_wrap_event_json)
                            .map_err(|error| SarahConversationError::Identity(error.to_string()))?;
                        (
                            gift.rumor_event_id,
                            gift.sender_public_key_hex,
                            gift.created_at,
                            gift.tags,
                            gift.content,
                        )
                    }
                    #[cfg(any(test, feature = "test-support"))]
                    RelayCustody::Keys(keys) => {
                        let gift = smol::block_on(
                            crate::nostr_websocket_relay::nip59_extract_rumor(keys, event),
                        )
                        .map_err(|error| SarahConversationError::Relay(error.to_string()))?;
                        if gift.rumor.kind != Kind::PrivateDirectMessage {
                            return Ok(None);
                        }
                        gift.rumor
                            .verify_id()
                            .map_err(|error| SarahConversationError::Relay(error.to_string()))?;
                        let rumor_event_id = gift.rumor.id.ok_or_else(|| {
                            SarahConversationError::Relay(
                                "NIP-59 rumor omitted its convergence id".into(),
                            )
                        })?;
                        (
                            rumor_event_id.to_hex(),
                            gift.sender.to_hex(),
                            gift.rumor.created_at.as_secs(),
                            gift.rumor
                                .tags
                                .iter()
                                .map(|tag| tag.as_slice().to_vec())
                                .collect(),
                            gift.rumor.content,
                        )
                    }
                };
            let recipient_public_key_hex = self.custody_public_key_hex()?;
            let record_kind = private_record_kind(
                &content,
                &sender_public_key_hex,
                &recipient_public_key_hex,
                &self.owner_public_key_hex,
                &self.sarah_public_key_hex,
                &tags,
            )?;
            if record_kind == "message" {
                require_conversation_recipients(
                    &tags,
                    &sender_public_key_hex,
                    &self.owner_public_key_hex,
                    &self.sarah_public_key_hex,
                )?;
                if (sender_public_key_hex != self.owner_public_key_hex
                    && sender_public_key_hex != self.sarah_public_key_hex)
                    || tag_value(&tags, "conversation").as_deref() != Some(conversation_ref)
                {
                    return Ok(None);
                }
            }
            return Ok(Some(StoredConversationEvent {
                event_id: rumor_event_id,
                kind: Kind::PrivateDirectMessage.as_u16(),
                pubkey: sender_public_key_hex,
                created_at,
                conversation_ref: conversation_ref.to_string(),
                content_summary: content,
                tags,
                record_kind: record_kind.to_string(),
                store_index: 0,
            }));
        }

        let author = event.pubkey.to_hex();
        let kind = event.kind.as_u16();
        let tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        if kind == ISSUE31_HOST_DISCOVERY_KIND {
            let value = serde_json::from_str::<Value>(&event.content)
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
            let schema = value.get("schema").and_then(Value::as_str);
            let (host_ref, host_public_key_hex) = if schema
                == Some(ISSUE31_HOST_DISCOVERY_SCHEMA_V3)
            {
                let discovery = Issue31HostDiscoveryV3::decode(event.content.as_bytes())
                    .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
                if discovery.conversation != conversation_ref || !discovery.is_live_at(unix_now()) {
                    return Ok(None);
                }
                (discovery.host_ref, discovery.host_public_key_hex)
            } else if schema == Some(ISSUE31_HOST_DISCOVERY_SCHEMA_V2) {
                let discovery = Issue31HostDiscoveryV2::decode(event.content.as_bytes())
                    .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
                if discovery.conversation != conversation_ref {
                    return Ok(None);
                }
                (discovery.host_ref, discovery.host_public_key_hex)
            } else {
                let discovery = Issue31HostDiscovery::decode(event.content.as_bytes())
                    .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
                (discovery.host_ref, discovery.host_public_key_hex)
            };
            if author != self.owner_public_key_hex
                || author != self.custody_public_key_hex()?
                || host_public_key_hex != author
                || tag_value(&tags, "d").as_deref() != Some(host_ref.as_str())
                || tag_value(&tags, "k").as_deref() != Some("1059")
                || tag_value(&tags, "t").as_deref() != Some("omega-issue31-host")
                || tag_value(&tags, "alt").as_deref() != Some("Omega device mirror host discovery")
            {
                return Ok(None);
            }
        } else if SARAH_RECORD_KINDS.contains(&kind) {
            if (author != self.owner_public_key_hex && author != self.sarah_public_key_hex)
                || tag_value(&tags, "conversation").as_deref() != Some(conversation_ref)
            {
                return Ok(None);
            }
        } else if kind == NIP_AE_KIND {
            if author != self.sarah_public_key_hex
                || !valid_nip_ae_tags(&tags, &self.owner_public_key_hex)
            {
                return Ok(None);
            }
        } else if kind == NIP_RS_KIND {
            if author != self.owner_public_key_hex || !valid_nip_rs_tags(&tags) {
                return Ok(None);
            }
        } else if kind == NIP_ER_KIND {
            if (author != self.owner_public_key_hex && author != self.sarah_public_key_hex)
                || !valid_nip_er_tags(&tags)
            {
                return Ok(None);
            }
        } else if kind == NIP_29_GROUP_CHAT_KIND {
            if !configured_group_tag(&tags, &self.community_group_ids) {
                return Ok(None);
            }
        } else if LBR_KINDS.contains(&kind) {
            if !self
                .community_public_key_hexes
                .iter()
                .any(|public_key| public_key == &author)
            {
                return Ok(None);
            }
        } else {
            return Ok(None);
        }
        Ok(Some(StoredConversationEvent {
            event_id: event.id.to_hex(),
            kind,
            pubkey: author,
            created_at: event.created_at.as_secs(),
            conversation_ref: conversation_ref.to_string(),
            content_summary: if SARAH_RECORD_KINDS.contains(&kind)
                || matches!(kind, NIP_AE_KIND | NIP_RS_KIND | NIP_ER_KIND)
            {
                event.content.clone()
            } else {
                bounded_summary(&event.content)
            },
            tags,
            record_kind: public_record_kind(event.kind.as_u16()).to_string(),
            store_index: 0,
        }))
    }

    fn reindex_events(&mut self) {
        let mut events: Vec<StoredConversationEvent> = self.events.values().cloned().collect();
        events.sort_by(|left, right| {
            (left.created_at, left.event_id.as_str())
                .cmp(&(right.created_at, right.event_id.as_str()))
        });
        if events.len() > MAX_CACHED_EVENTS {
            events.drain(..events.len().saturating_sub(MAX_CACHED_EVENTS));
            self.gap_state = GapState::Possible;
            self.event_cache_truncated = true;
        }
        let mut content_bytes = events
            .iter()
            .map(|event| event.content_summary.len())
            .sum::<usize>();
        while content_bytes > MAX_CACHED_CONTENT_BYTES && events.len() > 1 {
            let removed = events.remove(0);
            content_bytes = content_bytes.saturating_sub(removed.content_summary.len());
            self.gap_state = GapState::Possible;
            self.event_cache_truncated = true;
        }
        self.events.clear();
        for (store_index, mut event) in events.into_iter().enumerate() {
            event.store_index = store_index;
            self.events.insert(event.event_id.clone(), event);
        }
    }

    fn page(
        &self,
        conversation_ref: &str,
        after_cursor: Option<&str>,
        limit: usize,
        query_gap_state: GapState,
    ) -> QueryPage {
        let mut matching: Vec<StoredConversationEvent> = self
            .events
            .values()
            .filter(|event| event.conversation_ref == conversation_ref)
            .cloned()
            .collect();
        matching.sort_by(|left, right| {
            (left.created_at, left.event_id.as_str())
                .cmp(&(right.created_at, right.event_id.as_str()))
        });

        let mut gap_state = query_gap_state;
        let start_index = match after_cursor {
            Some(cursor) => match matching
                .iter()
                .position(|event| event_cursor(event) == cursor)
            {
                Some(index) => index.saturating_add(1),
                None => {
                    gap_state = GapState::Confirmed;
                    0
                }
            },
            None => 0,
        };
        let events: Vec<StoredConversationEvent> = matching
            .iter()
            .skip(start_index)
            .take(limit)
            .cloned()
            .collect();
        let next_cursor = if start_index.saturating_add(events.len()) < matching.len() {
            events.last().map(event_cursor)
        } else {
            None
        };
        QueryPage {
            events,
            next_cursor,
            gap_state,
        }
    }
}

impl RelayTransport for WebSocketRelayAdapter {
    fn label(&self) -> &str {
        &self.active_label
    }

    fn connection_state(&self) -> ConnectionState {
        if self.socket.is_none() && self.healthy_relays.is_empty() {
            ConnectionState::Disconnected
        } else if self.gap_state == GapState::None
            && self.healthy_relays.len() == self.relay_urls.len()
        {
            ConnectionState::Connected
        } else {
            ConnectionState::Degraded
        }
    }

    fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    fn connect(&mut self) -> Result<(), SarahConversationError> {
        if self.socket.is_some() {
            return Ok(());
        }
        let attempts = self.relay_urls.len().saturating_mul(MAX_CONNECT_ROUNDS);
        let mut last_error = None;
        for attempt in 0..attempts {
            match self.connect_active() {
                Ok(()) => return Ok(()),
                Err(error) => last_error = Some(error),
            }
            self.disconnect_and_advance();
            let multiplier = 1_u32 << attempt.min(3);
            let backoff = INITIAL_BACKOFF.saturating_mul(multiplier).min(MAX_BACKOFF);
            smol::block_on(network_timer(backoff));
        }
        Err(last_error.unwrap_or_else(|| {
            SarahConversationError::Relay("relay connection attempts exhausted".into())
        }))
    }

    fn auth_challenge(&self) -> Option<RelayAuthChallenge> {
        self.auth_challenge
            .as_ref()
            .map(|pending| RelayAuthChallenge {
                challenge: pending.challenge.clone(),
                relay_url: self.active_label.clone(),
            })
    }

    fn authenticate(&mut self, auth_event: &Event) -> Result<(), SarahConversationError> {
        self.authenticate_until(auth_event, Instant::now() + NETWORK_TIMEOUT)
    }

    fn publish(&mut self, event: &Event) -> Result<(), SarahConversationError> {
        let event_id = event.id.to_hex();
        let mut last_error = None;
        for relay_index in self.publish_relay_order() {
            let relay_url = self.relay_urls[relay_index].clone();
            if self
                .publish_acknowledgements
                .get(&event_id)
                .is_some_and(|relays| relays.contains(&relay_url))
            {
                continue;
            }
            if self.socket.is_none() || self.active_label != relay_url {
                self.close_active_connection();
                self.active_relay_index = relay_index;
                if let Err(error) = self.connect_active() {
                    last_error = Some(error);
                    self.healthy_relays.remove(&relay_url);
                    continue;
                }
            }
            match self.publish_once(event) {
                Ok(()) => {
                    self.healthy_relays.insert(relay_url.clone());
                    self.publish_acknowledgements
                        .entry(event_id.clone())
                        .or_default()
                        .insert(relay_url);
                }
                Err(SarahConversationError::IdentityRequired) => {
                    return Err(SarahConversationError::IdentityRequired);
                }
                Err(error) => {
                    last_error = Some(error);
                    self.healthy_relays.remove(&relay_url);
                    self.close_active_connection();
                }
            }
        }
        let acknowledged_relays = self
            .publish_acknowledgements
            .get(&event_id)
            .map_or(0, HashSet::len);
        self.prune_publication_acknowledgements(&event_id);
        if acknowledged_relays > 0 {
            self.acknowledged_event_id = Some(event_id);
            if acknowledged_relays < self.relay_urls.len() {
                self.gap_state = GapState::Possible;
            } else if self.healthy_relays.len() == self.relay_urls.len()
                && !self.event_cache_truncated
            {
                self.gap_state = GapState::None;
            }
            Ok(())
        } else {
            self.gap_state = GapState::Possible;
            Err(last_error.unwrap_or_else(|| {
                SarahConversationError::Relay(
                    "event has not been acknowledged by every configured relay".into(),
                )
            }))
        }
    }

    fn publication_complete(&mut self, event_id: &str) -> bool {
        let complete = self
            .publish_acknowledgements
            .get(event_id)
            .is_some_and(|relays| relays.len() == self.relay_urls.len());
        if complete {
            self.publish_acknowledgements.remove(event_id);
        }
        complete
    }

    fn acknowledged_relays(&self, event_id: &str) -> Vec<String> {
        let mut relay_urls = self
            .publish_acknowledgements
            .get(event_id)
            .map(|relays| relays.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        relay_urls.sort();
        relay_urls
    }

    fn restore_publication_acknowledgements(&mut self, event_id: &str, relay_urls: &[String]) {
        let configured_relays = &self.relay_urls;
        let acknowledgements = self
            .publish_acknowledgements
            .entry(event_id.to_string())
            .or_default();
        acknowledgements.extend(
            relay_urls
                .iter()
                .filter(|relay_url| configured_relays.contains(relay_url))
                .cloned(),
        );
        self.prune_publication_acknowledgements(event_id);
    }

    fn query(
        &mut self,
        conversation_ref: &str,
        after_cursor: Option<&str>,
        limit: usize,
    ) -> Result<QueryPage, SarahConversationError> {
        let operation_deadline = Instant::now() + QUERY_OPERATION_TIMEOUT;
        let mut query_gap_state = GapState::None;
        let mut successful_relays = 0_usize;
        let mut last_error = None;
        for relay_index in 0..self.relay_urls.len() {
            let relay_url = self.relay_urls[relay_index].clone();
            let relay_deadline = operation_deadline.min(Instant::now() + QUERY_TIMEOUT);
            if relay_deadline <= Instant::now() {
                query_gap_state = GapState::Possible;
                last_error = Some(SarahConversationError::Relay(
                    "multi-relay query deadline elapsed".into(),
                ));
                break;
            }
            if self.socket.is_none() || self.active_label != relay_url {
                self.close_active_connection();
                self.active_relay_index = relay_index;
                if let Err(error) = self.connect_active_until(relay_deadline) {
                    last_error = Some(error);
                    query_gap_state = GapState::Possible;
                    self.healthy_relays.remove(&relay_url);
                    continue;
                }
            }
            match self.query_active_relay_until(conversation_ref, relay_deadline) {
                Ok(gap_state) => {
                    successful_relays = successful_relays.saturating_add(1);
                    self.healthy_relays.insert(relay_url);
                    if gap_state != GapState::None {
                        query_gap_state = strongest_gap_state(query_gap_state, gap_state);
                    }
                }
                Err(error) => {
                    last_error = Some(error);
                    query_gap_state = GapState::Possible;
                    self.healthy_relays.remove(&relay_url);
                    self.close_active_connection();
                }
            }
        }
        if successful_relays == 0 {
            return Err(last_error.unwrap_or_else(|| {
                SarahConversationError::Relay("all relay queries failed".into())
            }));
        }
        if self.event_cache_truncated
            || self
                .publish_acknowledgements
                .values()
                .any(|relays| !relays.is_empty() && relays.len() < self.relay_urls.len())
        {
            query_gap_state = strongest_gap_state(query_gap_state, GapState::Possible);
        }
        self.gap_state = query_gap_state;
        Ok(self.page(conversation_ref, after_cursor, limit, query_gap_state))
    }

    fn last_event_id(&self) -> Option<String> {
        self.acknowledged_event_id.clone()
    }

    fn gap_state(&self) -> GapState {
        self.gap_state
    }

    fn connected_relays(&self) -> Vec<String> {
        let mut relays = self.healthy_relays.iter().cloned().collect::<Vec<_>>();
        relays.sort();
        relays
    }

    fn requires_private_messages(&self) -> bool {
        true
    }
}

/// Publish one already-signed event through Omega's authenticated relay
/// transport.
///
/// The event is never re-signed. A NIP-42 challenge signs only the relay's
/// authentication event, then the exact caller-supplied bytes are retried.
pub fn publish_community_event(
    relay_urls: Vec<String>,
    event: &Event,
) -> Result<(), SarahConversationError> {
    let identity_service = Arc::new(IdentityService::system(*app_identity::CHANNEL));
    let owner_public_key_hex = identity_service
        .inspect()
        .map_err(|error| SarahConversationError::Identity(error.to_string()))?
        .identity
        .map(|identity| identity.public_key_hex().as_str().to_string())
        .ok_or(SarahConversationError::IdentityRequired)?;
    let mut relay = WebSocketRelayAdapter::new(
        relay_urls.clone(),
        identity_service,
        owner_public_key_hex,
        Vec::new(),
        Vec::new(),
    )?;
    relay.connect()?;
    for _ in 0..=relay_urls.len() {
        match relay.publish(event) {
            Err(SarahConversationError::IdentityRequired) => {
                let auth_event = relay.sign_active_auth_event()?;
                relay.authenticate(&auth_event)?;
            }
            result => return result,
        }
    }
    Err(SarahConversationError::Relay(
        "relay authentication did not settle".into(),
    ))
}

/// Read signed NIP-22 messages bound to one Forge repository.
///
/// This transport proves relay identity with NIP-42 when challenged. The
/// caller still has to enforce repository binding before displaying a record.
pub fn query_community_events(
    relay_urls: Vec<String>,
    coordinate: &str,
) -> Result<Vec<Event>, SarahConversationError> {
    let identity_service = Arc::new(IdentityService::system(*app_identity::CHANNEL));
    let owner_public_key_hex = identity_service
        .inspect()
        .map_err(|error| SarahConversationError::Identity(error.to_string()))?
        .identity
        .map(|identity| identity.public_key_hex().as_str().to_string())
        .ok_or(SarahConversationError::IdentityRequired)?;
    let mut relay = WebSocketRelayAdapter::new(
        relay_urls.clone(),
        identity_service,
        owner_public_key_hex,
        Vec::new(),
        Vec::new(),
    )?;
    relay.connect()?;
    for _ in 0..=relay_urls.len() {
        match relay.query_community_once(coordinate) {
            Err(SarahConversationError::IdentityRequired) => {
                let auth_event = relay.sign_active_auth_event()?;
                relay.authenticate(&auth_event)?;
            }
            result => return result,
        }
    }
    Err(SarahConversationError::Relay(
        "relay authentication did not settle".into(),
    ))
}

/// Order the relays a publish should attempt, given an open authenticated
/// session on `resume_at` if there is one.
///
/// Every relay is always attempted exactly once; only the order changes.
fn publish_relay_order(relay_count: usize, resume_at: Option<usize>) -> Vec<usize> {
    let Some(resume_at) = resume_at.filter(|index| *index < relay_count) else {
        return (0..relay_count).collect();
    };
    std::iter::once(resume_at)
        .chain((0..relay_count).filter(move |index| *index != resume_at))
        .collect()
}

fn normalize_relay_url(relay_url: &str) -> Result<String, SarahConversationError> {
    let parsed = url::Url::parse(relay_url)
        .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
    if !matches!(parsed.scheme(), "ws" | "wss")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(SarahConversationError::InvalidRequest(
            "relay URL must be a credential-free ws:// or wss:// endpoint".into(),
        ));
    }
    let mut normalized = parsed.to_string();
    if normalized.ends_with('/') {
        normalized.pop();
    }
    Ok(normalized)
}

fn challenge_reference(
    relay_url: &str,
    connection_generation: u64,
    challenge: &str,
) -> Result<ProofRef, SarahConversationError> {
    let mut digest = Sha256::new();
    digest.update(relay_url.as_bytes());
    digest.update([0]);
    digest.update(connection_generation.to_be_bytes());
    digest.update([0]);
    digest.update(challenge.as_bytes());
    let digest = format!("{:x}", digest.finalize());
    ProofRef::new(format!("nip42.{}", &digest[..32]))
        .map_err(|error| SarahConversationError::Internal(error.to_string()))
}

fn validate_nip42_auth_event(
    auth_event: &Event,
    relay_url: &RelayUrl,
    challenge: &str,
    expected_public_key_hex: &str,
    now: u64,
) -> Result<(), RelayAuthenticationRefusal> {
    if auth_event.pubkey.to_hex() != expected_public_key_hex {
        return Err(RelayAuthenticationRefusal::WrongAccount);
    }
    if auth_event.created_at.as_secs().abs_diff(now) > NIP42_FRESHNESS_WINDOW {
        return Err(RelayAuthenticationRefusal::StaleEvent);
    }

    let mut relay_tags = 0_usize;
    let mut challenge_tags = 0_usize;
    for tag in auth_event.tags.iter() {
        let values = tag.as_slice();
        match values.first().map(String::as_str) {
            Some("relay") => {
                relay_tags = relay_tags.saturating_add(1);
                if values.len() != 2
                    || values.get(1).map(String::as_str) != Some(relay_url.as_str())
                {
                    return Err(RelayAuthenticationRefusal::InvalidEvent);
                }
            }
            Some("challenge") => {
                challenge_tags = challenge_tags.saturating_add(1);
                if values.len() != 2 || values.get(1).map(String::as_str) != Some(challenge) {
                    return Err(RelayAuthenticationRefusal::InvalidEvent);
                }
            }
            _ => {}
        }
    }
    if relay_tags != 1 || challenge_tags != 1 {
        return Err(RelayAuthenticationRefusal::InvalidEvent);
    }
    if !nostr::nips::nip42::is_valid_auth_event(auth_event, relay_url, challenge)
        || auth_event.verify().is_err()
    {
        return Err(RelayAuthenticationRefusal::InvalidEvent);
    }
    Ok(())
}

fn begin_authentication_retry(attempted: &mut bool) -> Result<(), SarahConversationError> {
    if std::mem::replace(attempted, true) {
        return Err(SarahConversationError::Relay(
            "relay repeated NIP-42 challenge after authentication".into(),
        ));
    }
    Ok(())
}

fn record_control_frame(count: &mut usize) -> Result<(), SarahConversationError> {
    *count = count.saturating_add(1);
    if *count > MAX_CONTROL_FRAMES_PER_READ {
        return Err(SarahConversationError::Relay(
            "relay exceeded the control-frame budget".into(),
        ));
    }
    Ok(())
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

fn event_cursor(event: &StoredConversationEvent) -> String {
    format!("cursor.{}.{}", event.created_at, event.event_id)
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

fn private_record_kind(
    content: &str,
    sender_public_key_hex: &str,
    recipient_public_key_hex: &str,
    owner_public_key_hex: &str,
    sarah_public_key_hex: &str,
    tags: &[Vec<String>],
) -> Result<&'static str, SarahConversationError> {
    let schema = serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|value| {
            value
                .get("schema")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    match schema.as_deref() {
        Some(ISSUE31_COMMAND_SCHEMA) => {
            require_single_private_recipient(tags, recipient_public_key_hex)?;
            let record = Issue31CommandRecord::decode(content.as_bytes())
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
            record
                .validate_private_binding(sender_public_key_hex, recipient_public_key_hex)
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
            Ok("control")
        }
        Some(ISSUE31_COMMAND_SCHEMA_V2) => {
            require_single_private_recipient(tags, recipient_public_key_hex)?;
            let record = Issue31CommandRecordV2::decode(content.as_bytes())
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
            record
                .validate_private_binding(sender_public_key_hex, recipient_public_key_hex)
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
            Ok("control")
        }
        Some(ISSUE31_OWNER_PROJECTION_SCHEMA) => {
            require_single_private_recipient(tags, recipient_public_key_hex)?;
            let record = Issue31OwnerProjectionRecord::decode(content.as_bytes())
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
            if record.host_public_key_hex != owner_public_key_hex {
                return Err(SarahConversationError::InvalidRequest(
                    "owner projection targets another host".into(),
                ));
            }
            record
                .validate_private_binding(
                    sender_public_key_hex,
                    recipient_public_key_hex,
                    sarah_public_key_hex,
                )
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
            Ok("owner_projection")
        }
        Some(ISSUE31_PAIRING_SCHEMA) => {
            require_single_private_recipient(tags, recipient_public_key_hex)?;
            let record = Issue31PairingRecord::decode(content.as_bytes())
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
            record
                .validate_private_binding(sender_public_key_hex, recipient_public_key_hex)
                .map_err(|error| SarahConversationError::InvalidRequest(error.to_string()))?;
            Ok("pairing")
        }
        Some(schema) if schema.starts_with("openagents.omega.issue31.") => Err(
            SarahConversationError::InvalidRequest("unknown device mirror private schema".into()),
        ),
        _ => Ok("message"),
    }
}

fn require_single_private_recipient(
    tags: &[Vec<String>],
    recipient_public_key_hex: &str,
) -> Result<(), SarahConversationError> {
    let recipients: Vec<&str> = tags
        .iter()
        .filter(|tag| tag.first().map(String::as_str) == Some("p"))
        .filter_map(|tag| tag.get(1).map(String::as_str))
        .collect();
    if recipients != [recipient_public_key_hex] {
        return Err(SarahConversationError::InvalidRequest(
            "device mirror private rumor must have exactly one local p tag".into(),
        ));
    }
    Ok(())
}

/// A Sarah conversation message is a two-party record, unlike every Issue 31
/// control record around it, and its author is never in its own `p` set.
///
/// `conversation_tags` writes a `p` tag for the owner *and* one for Sarah, but
/// `EventBuilder::build` defaults `allow_self_tagging` to false and strips the
/// `p` tag matching the event author. So an owner-authored rumor ships as
/// `p: [sarah]` and a Sarah-authored one as `p: [owner]`. That is correct
/// NIP-17 — a rumor's `p` tags are its receivers — and it is exactly what the
/// old 1:1 `require_single_private_recipient` rule could not express.
///
/// Holding conversation rumors to that rule refused every message the host
/// itself authored. The owner's own send was unreadable back off the relay,
/// `load_issue31_source_event` could never confirm it, and the command came
/// back `reason.omega.projection_failed` with no owner projection ever
/// published — a phone on omega#49 saw the send publish, the host admit the
/// command, and no reply arrive, three times. Nothing in the repository caught
/// it because every admission test hand-wrote a single-`p` rumor instead of
/// building one through the production path.
///
/// The bound the single-recipient rule was buying is kept. The author plus the
/// `p` set must be exactly the conversation's own two participants, so a rumor
/// naming any third recipient, or addressed to another pair entirely, is still
/// refused. Being the addressed reader is already proven by NIP-59: this custody
/// key decrypted the seal.
fn require_conversation_recipients(
    tags: &[Vec<String>],
    sender_public_key_hex: &str,
    owner_public_key_hex: &str,
    sarah_public_key_hex: &str,
) -> Result<(), SarahConversationError> {
    let mut named: Vec<&str> = tags
        .iter()
        .filter(|tag| tag.first().map(String::as_str) == Some("p"))
        .filter_map(|tag| tag.get(1).map(String::as_str))
        .collect();
    named.push(sender_public_key_hex);
    named.sort_unstable();
    named.dedup();
    let mut participants = [owner_public_key_hex, sarah_public_key_hex];
    participants.sort_unstable();
    if named != participants {
        return Err(SarahConversationError::InvalidRequest(
            "device mirror conversation rumor must name exactly the owner and Sarah".into(),
        ));
    }
    Ok(())
}

fn configured_group_tag(tags: &[Vec<String>], configured_group_ids: &[String]) -> bool {
    let group_ids: Vec<&str> = tags
        .iter()
        .filter(|tag| tag.first().map(String::as_str) == Some("h"))
        .filter_map(|tag| tag.get(1).map(String::as_str))
        .collect();
    matches!(group_ids.as_slice(), [group_id] if configured_group_ids.iter().any(|configured| configured == group_id))
}

fn valid_nip_ae_tags(tags: &[Vec<String>], owner_public_key_hex: &str) -> bool {
    exact_tag_value(tags, "d").is_some_and(is_lower_hex_64)
        && exact_tag_value(tags, "p") == Some(owner_public_key_hex)
        && exact_tag_value(tags, "alt") == Some("encrypted agent memory record")
}

fn valid_nip_rs_tags(tags: &[Vec<String>]) -> bool {
    exact_tag_value(tags, "d").is_some_and(|value| {
        value
            .strip_prefix("read-state:")
            .is_some_and(|slot_id| !slot_id.is_empty() && slot_id.len() <= 64 && slot_id.is_ascii())
    }) && exact_tag_value(tags, "t") == Some("read-state")
        && exact_tag_value(tags, "alt") == Some("encrypted read state")
}

fn valid_nip_er_tags(tags: &[Vec<String>]) -> bool {
    let Some(identifier) = exact_tag_value(tags, "d") else {
        return false;
    };
    if identifier.is_empty() || identifier.len() > 128 {
        return false;
    }
    if exact_tag_value(tags, "alt") != Some("Encrypted reminder") {
        return false;
    }
    let not_before = optional_ascii_u64_tag(tags, "not_before");
    let expiration = optional_ascii_u64_tag(tags, "expiration");
    if not_before.is_err() || expiration.is_err() {
        return false;
    }
    match (not_before.ok().flatten(), expiration.ok().flatten()) {
        (Some(not_before), Some(expiration)) => expiration > not_before,
        _ => true,
    }
}

fn exact_tag_value<'a>(tags: &'a [Vec<String>], name: &str) -> Option<&'a str> {
    let mut values = tags
        .iter()
        .filter(|tag| tag.first().map(String::as_str) == Some(name))
        .filter_map(|tag| tag.get(1).map(String::as_str));
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}

fn optional_ascii_u64_tag(tags: &[Vec<String>], name: &str) -> Result<Option<u64>, ()> {
    let values = tags
        .iter()
        .filter(|tag| tag.first().map(String::as_str) == Some(name))
        .collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(None),
        [tag] => {
            let value = tag.get(1).ok_or(())?;
            if value.is_empty()
                || !value.bytes().all(|byte| byte.is_ascii_digit())
                || (value.len() > 1 && value.starts_with('0'))
            {
                return Err(());
            }
            value.parse().map(Some).map_err(|_| ())
        }
        _ => Err(()),
    }
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn normalized_group_ids(group_ids: Vec<String>) -> Result<Vec<String>, SarahConversationError> {
    if group_ids.len() > 64
        || group_ids
            .iter()
            .any(|group_id| group_id.is_empty() || group_id.len() > 256)
    {
        return Err(SarahConversationError::InvalidRequest(
            "community group policy must contain at most 64 bounded group ids".into(),
        ));
    }
    let unique = group_ids.iter().collect::<HashSet<_>>();
    if unique.len() != group_ids.len() {
        return Err(SarahConversationError::InvalidRequest(
            "community group policy contains duplicates".into(),
        ));
    }
    Ok(group_ids)
}

fn normalized_public_keys(
    public_keys: Vec<String>,
    label: &str,
) -> Result<Vec<String>, SarahConversationError> {
    if public_keys.len() > 64
        || public_keys.iter().any(|public_key| {
            !is_lower_hex_64(public_key) || PublicKey::from_hex(public_key).is_err()
        })
    {
        return Err(SarahConversationError::InvalidRequest(format!(
            "{label} policy must contain at most 64 lowercase public keys"
        )));
    }
    let unique = public_keys.iter().collect::<HashSet<_>>();
    if unique.len() != public_keys.len() {
        return Err(SarahConversationError::InvalidRequest(format!(
            "{label} policy contains duplicates"
        )));
    }
    Ok(public_keys)
}

fn event_within_bounds(event: &Event) -> bool {
    event.content.len() <= 1024 * 1024
        && event.tags.len() <= 128
        && event.tags.iter().all(|tag| {
            let values = tag.as_slice();
            values.len() <= 16 && values.iter().all(|value| value.len() <= 1024)
        })
}

fn query_gap_after_eose(admitted_events: usize, admission_truncated: bool) -> GapState {
    if admission_truncated || admitted_events == 256 {
        GapState::Possible
    } else {
        GapState::None
    }
}

fn public_record_kind(kind: u16) -> &'static str {
    if ISSUE31_COMMUNITY_KINDS.contains(&kind) {
        "community"
    } else if SARAH_RECORD_KINDS.contains(&kind) {
        "activity"
    } else {
        "memory"
    }
}

fn bounded_summary(content: &str) -> String {
    const MAX_SUMMARY_BYTES: usize = 512;
    if content.len() <= MAX_SUMMARY_BYTES {
        return content.to_string();
    }
    let mut boundary = MAX_SUMMARY_BYTES;
    while !content.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    format!("{}…", &content[..boundary])
}

#[cfg(test)]
mod tests {
    use nostr::EventBuilder;

    use crate::issue31_nostr::{
        Issue31HostConfiguration, Issue31HostController, Issue31NostrError, Issue31PairingEvent,
        Issue31PairingScope,
    };
    use crate::sarah_conversation::SARAH_TURN_RECORD_KIND;

    use super::*;

    // Live proofs against the deployed OpenAgents relay (OMEGA-MOB-31-01).
    //
    // These four ran green against `wss://relay.openagents.com` on 2026-07-25,
    // driving the real `WebSocketRelayAdapter` through connect, NIP-42,
    // publish, and read-back. The relay then degraded mid-session: it now
    // answers any `REQ` carrying a `#p` filter with
    // `["NOTICE","error: Handler error: StorageError"]`, sends no EOSE, and —
    // because one bad filter poisons the whole `REQ` — returns nothing for the
    // other filters in the same subscription either. Every Omega query includes
    // a gift-wrap `#p` filter, so every read is currently blind and the three
    // read-dependent proofs below no longer reproduce. The publish-side proof
    // still passes. Tracked in OpenAgentsInc/nostr-effect#170.
    //
    // Worth considering once that lands: issuing one subscription per filter,
    // so a filter the relay cannot serve degrades that record family alone
    // instead of blinding the client to everything.

    /// Live proof against the deployed OpenAgents relay (OMEGA-MOB-31-01).
    ///
    /// Opt-in, because it needs the network and a real relay:
    ///
    /// ```sh
    /// OMEGA_LIVE_RELAY_URL=wss://openagents-nostr-relay-ezxz4mgdsq-uc.a.run.app \
    /// OMEGA_LIVE_RELAY_AUTH_URL=wss://relay.openagents.com \
    ///   cargo test -p omega_effectd --lib live_relay -- --ignored --nocapture
    /// ```
    ///
    /// `OMEGA_LIVE_RELAY_AUTH_URL` exists because the relay binds
    /// `RELAY_PUBLIC_URL` and refuses a NIP-42 auth event whose `relay` tag
    /// names any other host with `invalid: relay URL mismatch`. A client may
    /// therefore connect through one hostname and still has to tag the
    /// canonical one.
    ///
    /// This exercises the real `WebSocketRelayAdapter` rather than
    /// `MockRelayAdapter`: connect, NIP-42 challenge, sign, authenticate,
    /// publish a durable turn record, and read it back by conversation ref.
    #[test]
    #[ignore = "requires a live relay; set OMEGA_LIVE_RELAY_URL"]
    fn live_relay_round_trip() {
        let Ok(url) = std::env::var("OMEGA_LIVE_RELAY_URL") else {
            eprintln!("OMEGA_LIVE_RELAY_URL unset; skipping");
            return;
        };
        let auth_url = std::env::var("OMEGA_LIVE_RELAY_AUTH_URL").unwrap_or_else(|_| url.clone());

        let keys = Keys::generate();
        let mut relay = WebSocketRelayAdapter::new_for_keys(vec![url.clone()], keys.clone())
            .expect("adapter for live relay");

        relay.connect().expect("connect to live relay");
        // A relay is not reported healthy on socket open alone. `healthy_relays`
        // is populated by a successful publish or query, so the state is
        // deliberately Degraded until an operation proves the relay works.
        assert_ne!(relay.connection_state(), ConnectionState::Disconnected);

        let conversation_ref = format!("sarah.live.{}", &keys.public_key().to_hex()[..16]);
        let record = EventBuilder::new(Kind::Custom(SARAH_TURN_RECORD_KIND), "live round trip")
            .tag(
                nostr::Tag::parse(["conversation", conversation_ref.as_str()])
                    .expect("conversation tag"),
            )
            .sign_with_keys(&keys)
            .expect("signed turn record");

        // Mirror `SarahConversationClient::publish_with_auth`. The adapter does
        // not read the relay's proactive AUTH frame during connect, so the
        // challenge only becomes visible once an operation meets it.
        match relay.publish(&record) {
            Ok(()) => {}
            Err(SarahConversationError::IdentityRequired) => {
                let challenge = relay
                    .auth_challenge()
                    .expect("relay must expose a challenge after refusing the publish");
                let auth_event = EventBuilder::new(Kind::Custom(22242), "")
                    .tag(nostr::Tag::parse(["relay", auth_url.as_str()]).expect("relay tag"))
                    .tag(
                        nostr::Tag::parse(["challenge", challenge.challenge.as_str()])
                            .expect("challenge tag"),
                    )
                    .sign_with_keys(&keys)
                    .expect("signed auth event");
                relay
                    .authenticate(&auth_event)
                    .expect("NIP-42 authenticate");
                assert!(relay.is_authenticated(), "relay must accept our auth");
                relay
                    .publish(&record)
                    .expect("publish after authenticating");
            }
            Err(error) => panic!("unexpected publish error: {error}"),
        }
        assert!(
            relay.publication_complete(&record.id.to_hex()),
            "relay must acknowledge the publication"
        );

        let page = relay
            .query(&conversation_ref, None, 10)
            .expect("query the live relay");
        assert!(
            page.events
                .iter()
                .any(|event| event.event_id == record.id.to_hex()),
            "published event must read back from the live relay"
        );
        assert_eq!(
            page.gap_state,
            GapState::None,
            "no gap on a fresh conversation"
        );

        // Only now, with a publish and a query both acknowledged, is the relay
        // proven healthy rather than merely reachable.
        assert_eq!(relay.connection_state(), ConnectionState::Connected);
        assert_eq!(relay.connected_relays(), vec![url.clone()]);
        eprintln!(
            "live relay OK: published {} and read it back from {}",
            &record.id.to_hex()[..16],
            url
        );
    }

    /// Live proof for the Forge community seam added for omega#108.
    #[test]
    #[ignore = "requires a live relay; set OMEGA_LIVE_RELAY_URL"]
    fn live_community_round_trip() {
        let Ok(url) = std::env::var("OMEGA_LIVE_RELAY_URL") else {
            eprintln!("OMEGA_LIVE_RELAY_URL unset; skipping");
            return;
        };
        let auth_url = std::env::var("OMEGA_LIVE_RELAY_AUTH_URL").unwrap_or_else(|_| url.clone());
        let keys = Keys::generate();
        let coordinate = format!(
            "30617:{}:omega-live-{}",
            keys.public_key().to_hex(),
            unix_now()
        );
        let record = EventBuilder::new(Kind::Custom(1111), "omega community live round trip")
            .tag(nostr::Tag::parse(["A", coordinate.as_str()]).expect("repository root tag"))
            .sign_with_keys(&keys)
            .expect("signed community record");
        let mut relay = live_authenticated_adapter(vec![url.clone()], &keys);

        live_publish(&mut relay, &auth_url, &keys, &record)
            .expect("publish community record through NIP-42");
        let events = relay
            .query_community_once(&coordinate)
            .expect("query community records by repository binding");
        assert!(
            events.iter().any(|event| event.id == record.id),
            "the exact signed community event must read back"
        );
        eprintln!(
            "live community relay OK: published {} and read it back from {}",
            &record.id.to_hex()[..16],
            url
        );
    }

    /// Connect, authenticate if the relay asks, and hand back a ready adapter.
    ///
    /// Shared by the live proofs so each one exercises the identical NIP-42
    /// path the production client uses, rather than a per-test approximation.
    #[cfg(test)]
    fn live_authenticated_adapter(relay_urls: Vec<String>, keys: &Keys) -> WebSocketRelayAdapter {
        let mut relay = WebSocketRelayAdapter::new_for_keys(relay_urls, keys.clone())
            .expect("adapter for live relay");
        relay.connect().expect("connect to live relay");
        relay
    }

    /// Publish through the real adapter, meeting the NIP-42 challenge lazily
    /// exactly as `SarahConversationClient::publish_with_auth` does.
    #[cfg(test)]
    fn live_publish(
        relay: &mut WebSocketRelayAdapter,
        auth_url: &str,
        keys: &Keys,
        record: &Event,
    ) -> Result<(), SarahConversationError> {
        match relay.publish(record) {
            Err(SarahConversationError::IdentityRequired) => {
                let challenge = relay
                    .auth_challenge()
                    .expect("relay must expose a challenge after refusing the publish");
                let auth_event = EventBuilder::new(Kind::Custom(22242), "")
                    .tag(nostr::Tag::parse(["relay", auth_url]).expect("relay tag"))
                    .tag(
                        nostr::Tag::parse(["challenge", challenge.challenge.as_str()])
                            .expect("challenge tag"),
                    )
                    .sign_with_keys(keys)
                    .expect("signed auth event");
                relay
                    .authenticate(&auth_event)
                    .expect("NIP-42 authenticate");
                relay.publish(record)
            }
            other => other,
        }
    }

    #[cfg(test)]
    fn live_turn_record(keys: &Keys, conversation_ref: &str, body: &str, created_at: u64) -> Event {
        EventBuilder::new(Kind::Custom(SARAH_TURN_RECORD_KIND), body)
            .tag(nostr::Tag::parse(["conversation", conversation_ref]).expect("conversation tag"))
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(keys)
            .expect("signed turn record")
    }

    #[cfg(test)]
    fn live_relay_env() -> Option<(String, String)> {
        let url = std::env::var("OMEGA_LIVE_RELAY_URL").ok()?;
        let auth_url = std::env::var("OMEGA_LIVE_RELAY_AUTH_URL").unwrap_or_else(|_| url.clone());
        Some((url, auth_url))
    }

    /// Exit: "Confirmed records remain readable when the application service is
    /// unavailable and the relays are reachable."
    ///
    /// The publishing adapter is dropped outright — the strongest available
    /// stand-in for the application service being gone — and a second adapter
    /// with no cursors, no cache and no shared state reads the record back from
    /// relay storage alone.
    #[test]
    #[ignore = "requires a live relay; set OMEGA_LIVE_RELAY_URL"]
    fn live_relay_confirmed_records_outlive_the_application_service() {
        let Some((url, auth_url)) = live_relay_env() else {
            eprintln!("OMEGA_LIVE_RELAY_URL unset; skipping");
            return;
        };
        let keys = Keys::generate();
        let conversation_ref = format!("sarah.live.survive.{}", &keys.public_key().to_hex()[..16]);
        let record = live_turn_record(
            &keys,
            &conversation_ref,
            "readable without the application service",
            nostr::Timestamp::now().as_secs(),
        );
        {
            let mut publisher = live_authenticated_adapter(vec![url.clone()], &keys);
            live_publish(&mut publisher, &auth_url, &keys, &record).expect("publish");
            assert!(
                publisher.publication_complete(&record.id.to_hex()),
                "the relay must acknowledge before we call anything confirmed"
            );
        } // The application service disappears here, taking all of its state.

        let mut reader = live_authenticated_adapter(vec![url.clone()], &keys);
        let page = match reader.query(&conversation_ref, None, 10) {
            Err(SarahConversationError::IdentityRequired) => {
                let challenge = reader.auth_challenge().expect("challenge");
                let auth_event = EventBuilder::new(Kind::Custom(22242), "")
                    .tag(nostr::Tag::parse(["relay", auth_url.as_str()]).expect("relay tag"))
                    .tag(
                        nostr::Tag::parse(["challenge", challenge.challenge.as_str()])
                            .expect("challenge tag"),
                    )
                    .sign_with_keys(&keys)
                    .expect("signed auth event");
                reader.authenticate(&auth_event).expect("authenticate");
                reader.query(&conversation_ref, None, 10).expect("query")
            }
            other => other.expect("query"),
        };
        assert!(
            page.events
                .iter()
                .any(|event| event.event_id == record.id.to_hex()),
            "a confirmed record must be readable from a relay with no application service"
        );
        eprintln!(
            "live relay OK: {} survived the application service and read back from {url}",
            &record.id.to_hex()[..16]
        );
    }

    /// Exit: "Relay outage and failover do not create false completion."
    ///
    /// Two halves. With one dead relay and one live relay the publish must
    /// succeed and be credited *only* to the relay that actually acknowledged.
    /// With every relay dead the publish must fail and completion must stay
    /// false — an unreachable relay can never be read as a completed write.
    #[test]
    #[ignore = "requires a live relay; set OMEGA_LIVE_RELAY_URL"]
    fn live_relay_outage_and_failover_never_report_false_completion() {
        let Some((url, auth_url)) = live_relay_env() else {
            eprintln!("OMEGA_LIVE_RELAY_URL unset; skipping");
            return;
        };
        // Port 9 is discard: the connection is refused rather than hanging.
        let dead_url = "ws://127.0.0.1:9".to_string();
        let keys = Keys::generate();
        let conversation_ref = format!("sarah.live.failover.{}", &keys.public_key().to_hex()[..16]);

        let record = live_turn_record(
            &keys,
            &conversation_ref,
            "failover",
            nostr::Timestamp::now().as_secs(),
        );
        let mut failover = live_authenticated_adapter(vec![dead_url.clone(), url.clone()], &keys);
        live_publish(&mut failover, &auth_url, &keys, &record).expect("publish must fail over");
        let event_id = record.id.to_hex();
        let acknowledged = failover.acknowledged_relays(&event_id);
        assert!(
            acknowledged.contains(&url),
            "the live relay acknowledged and must be credited"
        );
        assert!(
            !acknowledged.contains(&dead_url),
            "an unreachable relay must never be credited with an acknowledgement"
        );
        // Completion means every configured relay acknowledged. One relay being
        // down is exactly the case where a weaker rule would manufacture a false
        // completion, so partial success must not read as complete, and the gap
        // must be visible.
        assert!(
            !failover.publication_complete(&event_id),
            "a publish that reached only some relays must not report completion"
        );
        assert_eq!(failover.gap_state(), GapState::Possible);

        // Total outage: no relay can acknowledge, so nothing may be complete.
        let outage_record = live_turn_record(
            &keys,
            &conversation_ref,
            "total outage",
            nostr::Timestamp::now().as_secs(),
        );
        let mut outage = WebSocketRelayAdapter::new_for_keys(vec![dead_url], keys.clone())
            .expect("adapter for a dead relay");
        let _ = outage.connect();
        assert!(
            outage.publish(&outage_record).is_err(),
            "a publish to an unreachable relay must fail rather than succeed silently"
        );
        assert!(
            !outage.publication_complete(&outage_record.id.to_hex()),
            "a total relay outage must never report a completed publication"
        );
        assert!(
            outage
                .acknowledged_relays(&outage_record.id.to_hex())
                .is_empty()
        );
        eprintln!("live relay OK: failover credited only {url}; total outage stayed incomplete");
    }

    /// Exit: "Duplicate, reordered, missing, and stale events converge or show
    /// an exact gap."
    ///
    /// Against the real relay: the same signed event published twice converges
    /// to one record; events written newest-first still read back in a stable
    /// order; and a cursor the relay has never seen is reported as a confirmed
    /// gap rather than silently treated as an empty tail.
    #[test]
    #[ignore = "requires a live relay; set OMEGA_LIVE_RELAY_URL"]
    fn live_relay_duplicate_reorder_and_gap_converge_or_report_exactly() {
        let Some((url, auth_url)) = live_relay_env() else {
            eprintln!("OMEGA_LIVE_RELAY_URL unset; skipping");
            return;
        };
        let keys = Keys::generate();
        let conversation_ref = format!("sarah.live.converge.{}", &keys.public_key().to_hex()[..16]);
        let base = nostr::Timestamp::now().as_secs();
        let mut relay = live_authenticated_adapter(vec![url.clone()], &keys);

        // Written newest-first, so a naive reader would surface them reversed.
        let newest = live_turn_record(&keys, &conversation_ref, "third", base + 2);
        let middle = live_turn_record(&keys, &conversation_ref, "second", base + 1);
        let oldest = live_turn_record(&keys, &conversation_ref, "first", base);
        for record in [&newest, &middle, &oldest] {
            live_publish(&mut relay, &auth_url, &keys, record).expect("publish");
        }
        // The duplicate: the identical signed event, published a second time.
        // A relay may answer OK or reject it as a duplicate; neither may create
        // a second record.
        let _ = live_publish(&mut relay, &auth_url, &keys, &newest);

        let page = relay
            .query(&conversation_ref, None, 32)
            .expect("query the live relay");
        let ids = page
            .events
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<Vec<_>>();
        let unique = ids.iter().collect::<HashSet<_>>();
        assert_eq!(
            ids.len(),
            unique.len(),
            "a republished event must converge to a single record"
        );
        let ordering = page
            .events
            .iter()
            .map(|event| event.created_at)
            .collect::<Vec<_>>();
        let mut sorted = ordering.clone();
        sorted.sort_unstable();
        assert_eq!(
            ordering, sorted,
            "out-of-order writes must read back in a deterministic order"
        );
        assert_eq!(page.gap_state, GapState::None);

        // A cursor the relay has never issued is an exact, reported gap.
        let missing = relay
            .query(&conversation_ref, Some("cursor.999999.deadbeef"), 32)
            .expect("query with an unknown cursor");
        assert_eq!(
            missing.gap_state,
            GapState::Confirmed,
            "an unknown cursor must be reported as an exact gap, not an empty tail"
        );
        eprintln!(
            "live relay OK: {} records converged, ordered, and the unknown cursor reported an exact gap on {url}",
            page.events.len()
        );
    }

    /// Exit: "Duplicate, reordered, missing, and STALE events converge or show
    /// an exact gap." The sibling test above covers duplicate, reordered and
    /// missing. This covers stale.
    ///
    /// A pairing request that had already expired when it was written is
    /// published to the live relay, stored by it, and served back to the host.
    /// The relay is transport and storage, so it neither knows nor cares that
    /// the record is dead. The host must refuse it for being expired — not for
    /// being malformed and not for having been seen before — and must issue
    /// neither a challenge nor a grant.
    #[test]
    #[ignore = "requires a live relay; set OMEGA_LIVE_RELAY_URL"]
    fn live_relay_stale_records_are_refused_rather_than_acted_on() {
        let Some((url, auth_url)) = live_relay_env() else {
            eprintln!("OMEGA_LIVE_RELAY_URL unset; skipping");
            return;
        };
        let host_keys = Keys::generate();
        let device_keys = Keys::generate();
        let host_public_key_hex = host_keys.public_key().to_hex();
        let device_public_key_hex = device_keys.public_key().to_hex();
        let configuration = Issue31HostConfiguration {
            host_ref: "omega.host.stale".into(),
            host_public_key_hex: host_public_key_hex.clone(),
            sarah_public_key_hex: Keys::generate().public_key().to_hex(),
            conversation: format!("sarah.{}", &host_public_key_hex[..24]),
            display_name: "Live Omega".into(),
            relay_urls: vec![url.clone()],
            generation: 1,
        };
        let mut controller = Issue31HostController::new(configuration.clone()).expect("controller");
        controller
            .set_admitted_device_policy(
                vec![device_public_key_hex.clone()],
                vec![Issue31PairingScope::ControlFullAuto],
            )
            .expect("admit device");

        // Issued and expired well before now. The device is admitted and the
        // record is otherwise perfectly well formed, so staleness is the only
        // thing that can refuse it.
        let now = nostr::Timestamp::now().as_secs();
        let stale = Issue31PairingRecord::PairingRequest {
            schema: ISSUE31_PAIRING_SCHEMA.into(),
            host_ref: configuration.host_ref.clone(),
            host_public_key_hex: host_public_key_hex.clone(),
            device_public_key_hex,
            issued_at: now - 7_200,
            pairing_request_ref: "pairing_request.live.stale".into(),
            requested_scopes: vec![Issue31PairingScope::ControlFullAuto],
            expires_at: now - 3_600,
        };

        let mut device_relay = live_authenticated_adapter(vec![url.clone()], &device_keys);
        let host_public_key = PublicKey::from_hex(&host_public_key_hex).expect("host key");
        let rumor = EventBuilder::new(
            Kind::PrivateDirectMessage,
            serde_json::to_string(&stale).expect("record json"),
        )
        .tag(nostr::Tag::parse(["p", host_public_key_hex.as_str()]).expect("p tag"))
        .build(device_keys.public_key());
        let gift_wrap = smol::block_on(EventBuilder::gift_wrap(
            &device_keys,
            &host_public_key,
            rumor,
            [],
        ))
        .expect("gift wrap");
        live_publish(&mut device_relay, &auth_url, &device_keys, &gift_wrap)
            .expect("the relay must store a stale record; staleness is not its business");

        let mut host_relay = live_authenticated_adapter(vec![url.clone()], &host_keys);
        live_publish(
            &mut host_relay,
            &auth_url,
            &host_keys,
            &live_turn_record(&host_keys, &configuration.conversation, "host online", now),
        )
        .expect("host authenticates");
        let page = host_relay
            .query(&configuration.conversation, None, 32)
            .expect("query");
        let served = page
            .events
            .iter()
            .filter_map(|event| {
                Issue31PairingRecord::decode(event.content_summary.as_bytes())
                    .ok()
                    .map(|record| (event.event_id.clone(), record))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            served.len(),
            1,
            "the relay must actually serve the stale record back, or this proves nothing"
        );

        for (event_id, record) in served {
            assert!(
                !controller.pairing_event_was_processed(&event_id),
                "the stale record must arrive under an id the host has never seen, so that \
                 de-duplication is not what refuses it"
            );
            let refusal = controller
                .handle_pairing_event(Issue31PairingEvent { event_id, record }, now)
                .expect_err("a stale pairing record must be refused");
            assert!(
                matches!(&refusal, Issue31NostrError::Invalid(message)
                    if message.contains("expired")),
                "the refusal must be staleness, not dedup or shape: {refusal:?}"
            );
        }
        assert!(
            controller
                .grant_projections(now)
                .expect("grants")
                .is_empty(),
            "a stale request must not produce a grant"
        );
        eprintln!(
            "live relay OK: a stale pairing record stored by {url} was refused and granted nothing"
        );
    }

    /// Exit: "A revoked device cannot restore its grant from old relay events."
    ///
    /// Proven end to end through relay storage rather than in memory. The device
    /// re-publishes the two records it authors — the pairing request and the
    /// pairing response — to the live relay, the host reads them back out of
    /// relay storage, and the host must still refuse.
    ///
    /// Two things make this sharp. The replayed records arrive under the rumor
    /// ids the relay returns, so the host's processed-event de-duplication is
    /// not what refuses them. And they carry fresh, unexpired timestamps, so the
    /// liveness check is not what refuses them either. The only thing left
    /// standing between a revoked device and a brand-new `control_full_auto`
    /// grant is the revocation itself.
    ///
    /// This was previously blocked: `relay.openagents.com` answered any filter
    /// carrying a `#p` tag with `["NOTICE","error: Handler error: StorageError"]`
    /// and never terminated the subscription, and every NIP-17/44/59 private
    /// record is `#p`-addressed. That was a double-JSON-encoding defect on both
    /// the query and the INSERT paths, fixed in nostr-effect#170 and deployed as
    /// `openagents-nostr-relay-00006-7wf`. This test now passes against
    /// `wss://relay.openagents.com`. The revocation law itself is additionally
    /// proven without the network in `issue31_nostr`.
    #[test]
    #[ignore = "requires a live relay; set OMEGA_LIVE_RELAY_URL"]
    fn live_relay_replay_cannot_restore_a_revoked_device_grant() {
        let Some((url, auth_url)) = live_relay_env() else {
            eprintln!("OMEGA_LIVE_RELAY_URL unset; skipping");
            return;
        };
        let host_keys = Keys::generate();
        let device_keys = Keys::generate();
        let host_public_key_hex = host_keys.public_key().to_hex();
        let device_public_key_hex = device_keys.public_key().to_hex();
        let configuration = Issue31HostConfiguration {
            host_ref: "omega.host.live".into(),
            host_public_key_hex: host_public_key_hex.clone(),
            sarah_public_key_hex: Keys::generate().public_key().to_hex(),
            conversation: format!("sarah.{}", &host_public_key_hex[..24]),
            display_name: "Live Omega".into(),
            relay_urls: vec![url.clone()],
            generation: 1,
        };
        let mut controller = Issue31HostController::new(configuration.clone()).expect("controller");
        controller
            .set_admitted_device_policy(
                vec![device_public_key_hex.clone()],
                vec![Issue31PairingScope::ControlFullAuto],
            )
            .expect("admit device");

        let now = nostr::Timestamp::now().as_secs();
        let request = Issue31PairingRecord::PairingRequest {
            schema: ISSUE31_PAIRING_SCHEMA.into(),
            host_ref: configuration.host_ref.clone(),
            host_public_key_hex: host_public_key_hex.clone(),
            device_public_key_hex: device_public_key_hex.clone(),
            issued_at: now,
            pairing_request_ref: "pairing_request.live.replay".into(),
            requested_scopes: vec![Issue31PairingScope::ControlFullAuto],
            expires_at: now + 600,
        };

        // Pair for real, then have the owner revoke the grant.
        let challenge = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: "a".repeat(64),
                    record: request.clone(),
                },
                now,
            )
            .expect("request")
            .expect("challenge");
        let challenge_value = match &challenge {
            Issue31PairingRecord::PairingChallenge { challenge, .. } => challenge.clone(),
            _ => panic!("challenge"),
        };
        controller
            .record_emitted_pairing("b".repeat(64), challenge)
            .expect("record challenge");
        let response = Issue31PairingRecord::PairingResponse {
            schema: ISSUE31_PAIRING_SCHEMA.into(),
            host_ref: configuration.host_ref.clone(),
            host_public_key_hex: host_public_key_hex.clone(),
            device_public_key_hex,
            issued_at: now,
            pairing_response_ref: "pairing_response.live.replay".into(),
            pairing_challenge_event_id: "b".repeat(64),
            challenge: challenge_value,
            expires_at: now + 600,
        };
        let grant = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: "c".repeat(64),
                    record: response.clone(),
                },
                now,
            )
            .expect("response")
            .expect("grant");
        let grant_ref = match &grant {
            Issue31PairingRecord::ScopedGrant { grant_ref, .. } => grant_ref.clone(),
            _ => panic!("grant"),
        };
        controller
            .record_emitted_pairing("d".repeat(64), grant)
            .expect("record grant");
        let revocation = controller
            .revoke_grant(
                &grant_ref,
                now + 1,
                Some("reason.omega.owner_revoked".into()),
            )
            .expect("revoke");
        controller
            .record_emitted_pairing("e".repeat(64), revocation)
            .expect("record revocation");

        // The device replays what it authored, through the real relay.
        let mut device_relay = live_authenticated_adapter(vec![url.clone()], &device_keys);
        let host_public_key = PublicKey::from_hex(&host_public_key_hex).expect("host key");
        for record in [&request, &response] {
            let rumor = EventBuilder::new(
                Kind::PrivateDirectMessage,
                serde_json::to_string(record).expect("record json"),
            )
            .tag(nostr::Tag::parse(["p", host_public_key_hex.as_str()]).expect("p tag"))
            .build(device_keys.public_key());
            let gift_wrap = smol::block_on(EventBuilder::gift_wrap(
                &device_keys,
                &host_public_key,
                rumor,
                [],
            ))
            .expect("gift wrap");
            live_publish(&mut device_relay, &auth_url, &device_keys, &gift_wrap)
                .expect("publish the replayed pairing record");
        }

        // The host reads them back out of relay storage. A publish first, so the
        // NIP-42 handshake is settled before the query's own deadline starts.
        let mut host_relay = live_authenticated_adapter(vec![url.clone()], &host_keys);
        live_publish(
            &mut host_relay,
            &auth_url,
            &host_keys,
            &live_turn_record(&host_keys, &configuration.conversation, "host online", now),
        )
        .expect("host authenticates");
        let page = host_relay
            .query(&configuration.conversation, None, 32)
            .expect("query");
        let replayed = page
            .events
            .iter()
            .filter_map(|event| {
                Issue31PairingRecord::decode(event.content_summary.as_bytes())
                    .ok()
                    .map(|record| (event.event_id.clone(), record))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            replayed.len(),
            2,
            "the relay must actually return both replayed records, or this proves nothing"
        );

        for (event_id, record) in replayed {
            assert!(
                !controller.pairing_event_was_processed(&event_id),
                "the replay must arrive under an id the host has never seen, so that \
                 de-duplication is not what refuses it"
            );
            let refusal = controller
                .handle_pairing_event(Issue31PairingEvent { event_id, record }, now + 2)
                .expect_err("a revoked device must be refused, however its records arrive");
            assert!(
                matches!(&refusal, Issue31NostrError::Invalid(message)
                    if message.contains("device admission was revoked")),
                "the refusal must be the revocation, not liveness or dedup: {refusal:?}"
            );
        }
        let projections = controller.grant_projections(now + 2).expect("grants");
        assert_eq!(projections.len(), 1, "no new grant may exist");
        assert_eq!(projections[0].status, "revoked");
        eprintln!(
            "live relay OK: replayed pairing records from {url} could not restore the revoked grant"
        );
    }

    /// A publish retry must resume on the relay it just authenticated.
    ///
    /// Restarting at relay 0 after answering a NIP-42 challenge on relay 1 drops
    /// the authenticated session to re-attempt a relay that is down. The
    /// reconnect that follows earns a fresh challenge, so the caller's single
    /// retry raises `IdentityRequired` again and the publish never completes.
    /// This is the deterministic form of a failure first caught against the live
    /// relay with a dead relay ordered first.
    #[test]
    fn an_authenticated_publish_retry_resumes_on_the_relay_it_authenticated() {
        // No authenticated session: plain left to right.
        assert_eq!(publish_relay_order(3, None), vec![0, 1, 2]);
        // Authenticated on relay 1 — the case where relay 0 is down and relay 1
        // issued the NIP-42 challenge. Relay 1 is retried first, and relay 0 and
        // relay 2 are still attempted.
        assert_eq!(publish_relay_order(3, Some(1)), vec![1, 0, 2]);
        // Every relay appears exactly once, whatever the resume point.
        for resume_at in 0..3 {
            let order = publish_relay_order(3, Some(resume_at));
            assert_eq!(order.len(), 3);
            assert_eq!(order.iter().collect::<HashSet<_>>().len(), 3);
            assert_eq!(order[0], resume_at);
        }
        // An out-of-range resume point cannot drop or duplicate a relay.
        assert_eq!(publish_relay_order(2, Some(7)), vec![0, 1]);
        assert!(publish_relay_order(0, Some(0)).is_empty());
    }

    /// The adapter's own accessor must agree with the pure ordering rule: an
    /// authenticated flag with no open socket is not an authenticated session.
    #[test]
    fn a_closed_socket_is_not_an_authenticated_session() {
        let keys = Keys::generate();
        let mut relay = WebSocketRelayAdapter::new_for_keys(
            vec![
                "wss://down.example.com".to_string(),
                "wss://live.example.com".to_string(),
            ],
            keys,
        )
        .expect("relay");
        relay.active_relay_index = 1;
        relay.authenticated = true;
        assert!(relay.socket.is_none());
        assert_eq!(relay.publish_relay_order(), vec![0, 1]);
    }

    #[test]
    fn relay_urls_are_bounded_deduplicated_and_secret_free() {
        let keys = Keys::generate();
        let relay = WebSocketRelayAdapter::new_for_keys(
            vec![
                "wss://relay.example.com".to_string(),
                "wss://relay.example.com".to_string(),
                "ws://127.0.0.1:7777".to_string(),
            ],
            keys.clone(),
        )
        .expect("valid relay list");
        assert_eq!(relay.relay_urls().len(), 2);
        assert!(
            WebSocketRelayAdapter::new_for_keys(
                vec!["wss://owner:secret@relay.example.com".to_string()],
                keys,
            )
            .is_err()
        );
        assert!(normalize_relay_url("wss://relay.example.com/path").is_ok());
        assert_eq!(
            normalize_relay_url("wss://relay.example.com/").expect("normalized root relay"),
            "wss://relay.example.com"
        );
        assert!(normalize_relay_url("wss://relay.example.com/?token=secret").is_err());
        assert!(normalize_relay_url("wss://relay.example.com/#fragment").is_err());
        assert_eq!(
            normalize_relay_url("wss://relay.example.com/path/sidecar/").expect("normalized path"),
            "wss://relay.example.com/path/sidecar"
        );
    }

    #[test]
    fn each_relay_authentication_retry_is_single_and_bounded() {
        let mut first_relay_attempted = false;
        begin_authentication_retry(&mut first_relay_attempted).expect("first relay auth");
        assert!(begin_authentication_retry(&mut first_relay_attempted).is_err());

        let mut second_relay_attempted = false;
        begin_authentication_retry(&mut second_relay_attempted).expect("second relay auth");

        let mut control_frames = 0;
        for _ in 0..MAX_CONTROL_FRAMES_PER_READ {
            record_control_frame(&mut control_frames).expect("bounded control frame");
        }
        assert!(record_control_frame(&mut control_frames).is_err());
    }

    #[test]
    fn nip42_validation_binds_account_tags_signature_and_freshness() {
        let account_keys = Keys::generate();
        let other_keys = Keys::generate();
        let relay_url = RelayUrl::parse("wss://relay.example.com").expect("relay URL");
        let challenge = "challenge-123";
        let now = unix_now();
        let valid = EventBuilder::auth(challenge, relay_url.clone())
            .custom_created_at(nostr::Timestamp::from_secs(now))
            .sign_with_keys(&account_keys)
            .expect("valid auth event");
        assert_eq!(
            validate_nip42_auth_event(
                &valid,
                &relay_url,
                challenge,
                &account_keys.public_key().to_hex(),
                now,
            ),
            Ok(())
        );
        assert_eq!(
            validate_nip42_auth_event(
                &valid,
                &relay_url,
                challenge,
                &other_keys.public_key().to_hex(),
                now,
            ),
            Err(RelayAuthenticationRefusal::WrongAccount)
        );

        let stale = EventBuilder::auth(challenge, relay_url.clone())
            .custom_created_at(nostr::Timestamp::from_secs(
                now.saturating_sub(NIP42_FRESHNESS_WINDOW + 1),
            ))
            .sign_with_keys(&account_keys)
            .expect("stale auth event");
        assert_eq!(
            validate_nip42_auth_event(
                &stale,
                &relay_url,
                challenge,
                &account_keys.public_key().to_hex(),
                now,
            ),
            Err(RelayAuthenticationRefusal::StaleEvent)
        );

        let duplicate_challenge = EventBuilder::new(Kind::Authentication, "")
            .tags(vec![
                nostr::Tag::parse(["relay", relay_url.as_str()]).expect("relay tag"),
                nostr::Tag::parse(["challenge", challenge]).expect("challenge tag"),
                nostr::Tag::parse(["challenge", challenge]).expect("duplicate challenge tag"),
            ])
            .custom_created_at(nostr::Timestamp::from_secs(now))
            .sign_with_keys(&account_keys)
            .expect("duplicate-tag auth event");
        assert_eq!(
            validate_nip42_auth_event(
                &duplicate_challenge,
                &relay_url,
                challenge,
                &account_keys.public_key().to_hex(),
                now,
            ),
            Err(RelayAuthenticationRefusal::InvalidEvent)
        );
    }

    #[test]
    fn nip42_challenge_projection_never_exposes_the_raw_challenge() {
        let keys = Keys::generate();
        let mut relay =
            WebSocketRelayAdapter::new_for_keys(vec!["wss://relay.example.com/".to_string()], keys)
                .expect("relay");
        relay.connection_generation = 3;
        relay
            .record_auth_challenge(&json!(["AUTH", "raw-secret-shaped-challenge"]))
            .expect("valid challenge");
        let projection = relay.relay_authentication_projection();
        projection.validate().expect("valid public projection");
        let encoded = projection.public_json().expect("public projection JSON");
        assert!(!encoded.contains("raw-secret-shaped-challenge"));
        assert_eq!(
            projection.relays[0].state,
            RelayConnectionAuthenticationState::ChallengePending
        );
        assert_eq!(projection.relays[0].connection_generation, 3);
    }

    #[test]
    fn nip42_auth_proofs_are_one_use_per_adapter() {
        let keys = Keys::generate();
        let mut relay =
            WebSocketRelayAdapter::new_for_keys(vec!["wss://relay.example.com".to_string()], keys)
                .expect("relay");
        let event_id = "a".repeat(64);
        assert!(relay.remember_auth_event(event_id.clone()));
        assert!(!relay.remember_auth_event(event_id));
    }

    #[test]
    fn relay_authentication_snapshot_keeps_accounts_separate_on_the_same_relay() {
        let first_keys = Keys::generate();
        let second_keys = Keys::generate();
        let relay_url = "wss://account-switch-relay.example.com";
        let mut first =
            WebSocketRelayAdapter::new_for_keys(vec![relay_url.to_string()], first_keys.clone())
                .expect("first account relay");
        let mut second =
            WebSocketRelayAdapter::new_for_keys(vec![relay_url.to_string()], second_keys.clone())
                .expect("second account relay");
        first.connection_generation = 71;
        second.connection_generation = 72;
        first
            .record_auth_challenge(&json!(["AUTH", "first-account-challenge"]))
            .expect("first account challenge");
        second
            .record_auth_challenge(&json!(["AUTH", "second-account-challenge"]))
            .expect("second account challenge");

        let snapshot = relay_authentication_projection();
        let first_selected = snapshot.for_account_public_key_hex(&first_keys.public_key().to_hex());
        let second_selected =
            snapshot.for_account_public_key_hex(&second_keys.public_key().to_hex());
        assert_eq!(first_selected.relays.len(), 1);
        assert_eq!(second_selected.relays.len(), 1);
        assert_eq!(first_selected.relays[0].connection_generation, 71);
        assert_eq!(second_selected.relays[0].connection_generation, 72);
        assert_ne!(
            first_selected.relays[0].account_public_key_hex,
            second_selected.relays[0].account_public_key_hex
        );
    }

    #[test]
    fn missing_or_unknown_cursor_is_an_exact_gap() {
        let keys = Keys::generate();
        let mut relay =
            WebSocketRelayAdapter::new_for_keys(vec!["wss://relay.example.com".to_string()], keys)
                .expect("relay");
        relay.events.insert(
            "a".repeat(64),
            StoredConversationEvent {
                event_id: "a".repeat(64),
                kind: Kind::PrivateDirectMessage.as_u16(),
                pubkey: "b".repeat(64),
                created_at: 10,
                conversation_ref: "sarah.test".to_string(),
                content_summary: "hello".to_string(),
                tags: Vec::new(),
                record_kind: "message".to_string(),
                store_index: 0,
            },
        );
        let first = relay.page("sarah.test", None, 10, GapState::None);
        assert_eq!(first.gap_state, GapState::None);
        let missing = relay.page("sarah.test", Some("cursor.9.missing"), 10, GapState::None);
        assert_eq!(missing.gap_state, GapState::Confirmed);
        assert_eq!(missing.events.len(), 1);
    }

    #[test]
    fn duplicate_events_are_idempotent_and_reordered_deterministically() {
        let keys = Keys::generate();
        let mut relay =
            WebSocketRelayAdapter::new_for_keys(vec!["wss://relay.example.com".to_string()], keys)
                .expect("relay");
        for (id, created_at) in [("b", 20), ("a", 10), ("b", 20)] {
            let event_id = id.repeat(64);
            relay
                .events
                .entry(event_id.clone())
                .or_insert(StoredConversationEvent {
                    event_id,
                    kind: Kind::PrivateDirectMessage.as_u16(),
                    pubkey: "c".repeat(64),
                    created_at,
                    conversation_ref: "sarah.test".to_string(),
                    content_summary: id.to_string(),
                    tags: Vec::new(),
                    record_kind: "message".to_string(),
                    store_index: 0,
                });
        }
        relay.reindex_events();
        let page = relay.page("sarah.test", None, 10, GapState::None);
        assert_eq!(page.events.len(), 2);
        assert_eq!(page.events[0].content_summary, "a");
        assert_eq!(page.events[1].content_summary, "b");
    }

    #[test]
    fn relay_event_and_acknowledgement_caches_are_bounded() {
        let keys = Keys::generate();
        let mut relay =
            WebSocketRelayAdapter::new_for_keys(vec!["wss://relay.example.com".to_string()], keys)
                .expect("relay");
        for index in 0..=MAX_CACHED_EVENTS {
            let event_id = format!("{index:064x}");
            relay.events.insert(
                event_id.clone(),
                StoredConversationEvent {
                    event_id,
                    kind: Kind::PrivateDirectMessage.as_u16(),
                    pubkey: "c".repeat(64),
                    created_at: index as u64,
                    conversation_ref: "sarah.test".into(),
                    content_summary: "bounded".into(),
                    tags: Vec::new(),
                    record_kind: "message".into(),
                    store_index: index,
                },
            );
        }
        relay.reindex_events();
        assert_eq!(relay.events.len(), MAX_CACHED_EVENTS);
        assert_eq!(relay.gap_state, GapState::Possible);

        for index in 0..=MAX_PENDING_PUBLICATIONS {
            relay.restore_publication_acknowledgements(
                &format!("{index:064x}"),
                &["wss://relay.example.com".into()],
            );
        }
        assert_eq!(
            relay.publish_acknowledgements.len(),
            MAX_PENDING_PUBLICATIONS
        );
    }

    #[test]
    fn locally_rejects_filter_bypass_and_wrong_private_authors() {
        let owner = Keys::generate();
        let owner_public_key_hex = owner.public_key().to_hex();
        let sarah = Keys::generate();
        let community_author = Keys::generate();
        let attacker = Keys::generate();
        let relay = WebSocketRelayAdapter::new_for_keys_with_policy(
            vec!["wss://relay.example.com".to_string()],
            owner.clone(),
            sarah.public_key().to_hex(),
            vec!["community.openagents".into()],
            vec![community_author.public_key().to_hex()],
        )
        .expect("relay");
        let public_event = EventBuilder::new(Kind::from(SARAH_TURN_RECORD_KIND), "injected")
            .tags(vec![
                nostr::Tag::parse(["conversation", "sarah.test"]).expect("conversation tag"),
            ])
            .sign_with_keys(&attacker)
            .expect("signed attacker event");
        assert!(
            relay
                .admit_event(&public_event, "sarah.test")
                .expect("local admission")
                .is_none()
        );
        let group_event = EventBuilder::new(Kind::from(NIP_29_GROUP_CHAT_KIND), "group message")
            .tags(vec![
                nostr::Tag::parse(["h", "community.openagents"]).expect("group tag"),
            ])
            .sign_with_keys(&owner)
            .expect("signed group event");
        assert_eq!(
            relay
                .admit_event(&group_event, "sarah.test")
                .expect("group admission")
                .expect("admitted group")
                .record_kind,
            "community"
        );
        let missing_group = EventBuilder::new(Kind::from(NIP_29_GROUP_CHAT_KIND), "unbound")
            .sign_with_keys(&owner)
            .expect("signed unbound group event");
        assert!(
            relay
                .admit_event(&missing_group, "sarah.test")
                .expect("missing group admission")
                .is_none()
        );
        let wrong_recipient_rumor =
            EventBuilder::new(Kind::PrivateDirectMessage, "wrong recipient")
                .tags(vec![
                    nostr::Tag::parse(["p", attacker.public_key().to_hex().as_str()])
                        .expect("recipient tag"),
                    nostr::Tag::parse(["conversation", "sarah.test"]).expect("conversation tag"),
                ])
                .build(sarah.public_key());
        let wrong_recipient_gift_wrap = smol::block_on(EventBuilder::gift_wrap(
            &sarah,
            &owner.public_key(),
            wrong_recipient_rumor,
            [],
        ))
        .expect("gift wrap");
        assert!(
            relay
                .admit_event(&wrong_recipient_gift_wrap, "sarah.test")
                .is_err()
        );
        for kind in [
            LBR_AGENTIC_CODING_REQUEST_KIND,
            LBR_AGENTIC_CODING_RESULT_KIND,
            LBR_FEEDBACK_KIND,
        ] {
            let event = EventBuilder::new(Kind::from(kind), "bounded LBR record")
                .sign_with_keys(&community_author)
                .expect("signed LBR event");
            assert_eq!(
                relay
                    .admit_event(&event, "sarah.test")
                    .expect("LBR admission")
                    .expect("admitted LBR")
                    .record_kind,
                "community"
            );
            let owner_event = EventBuilder::new(Kind::from(kind), "unconfigured author")
                .sign_with_keys(&owner)
                .expect("signed owner LBR event");
            assert!(
                relay
                    .admit_event(&owner_event, "sarah.test")
                    .expect("owner LBR admission")
                    .is_none()
            );
        }

        let device = Keys::generate();
        let pairing = Issue31PairingRecord::PairingRequest {
            schema: ISSUE31_PAIRING_SCHEMA.into(),
            host_ref: "omega.host.local".into(),
            host_public_key_hex: owner_public_key_hex.clone(),
            device_public_key_hex: device.public_key().to_hex(),
            issued_at: 100,
            pairing_request_ref: "pairing_request.device.test".into(),
            requested_scopes: vec![crate::Issue31PairingScope::ObserveIssue31],
            expires_at: 200,
        };
        let content = serde_json::to_string(&pairing).expect("pairing json");
        let tags = vec![vec!["p".into(), owner_public_key_hex.clone()]];
        assert!(
            private_record_kind(
                &content,
                &attacker.public_key().to_hex(),
                &owner_public_key_hex,
                &owner_public_key_hex,
                &sarah.public_key().to_hex(),
                &tags,
            )
            .is_err()
        );
        let malformed = json!({
            "schema": ISSUE31_PAIRING_SCHEMA,
            "recordType": "pairing_request",
        })
        .to_string();
        assert!(
            private_record_kind(
                &malformed,
                &device.public_key().to_hex(),
                &owner_public_key_hex,
                &owner_public_key_hex,
                &sarah.public_key().to_hex(),
                &tags,
            )
            .is_err()
        );
        let command_v2 = json!({
            "schema": ISSUE31_COMMAND_SCHEMA_V2,
            "recordType": "command_intent",
            "hostRef": "omega.host.local",
            "hostPublicKeyHex": owner_public_key_hex,
            "devicePublicKeyHex": device.public_key().to_hex(),
            "grantRef": "grant.omega.device_1",
            "idempotencyRef": "idempotency.issue31.read_1",
            "expectedGeneration": 1,
            "arguments": {
                "kind": "read_state_patch",
                "actionRef": "action.issue31.read_state.advance",
                "slotId": "mobile",
                "clientId": "iphone",
                "contextRef": "sarah-conversation:sarah.0123456789abcdef01234567",
                "readAt": 150,
            },
            "issuedAt": 100,
            "expiresAt": 200,
        })
        .to_string();
        assert_eq!(
            private_record_kind(
                &command_v2,
                &device.public_key().to_hex(),
                &owner_public_key_hex,
                &owner_public_key_hex,
                &sarah.public_key().to_hex(),
                &tags,
            )
            .expect("command v2 admission"),
            "control"
        );
        let projection = json!({
            "schema": ISSUE31_OWNER_PROJECTION_SCHEMA,
            "recordType": "owner_projection",
            "hostRef": "omega.host.local",
            "hostPublicKeyHex": owner_public_key_hex,
            "devicePublicKeyHex": device.public_key().to_hex(),
            "grantRef": "grant.omega.device_1",
            "expectedGeneration": 1,
            "sourceEventId": "b".repeat(64),
            "sourceAuthorPublicKeyHex": owner_public_key_hex,
            "sourceRole": "owner",
            "sourceKind": 14,
            "sourceCreatedAt": 150,
            "projectedAt": 151,
            "projection": {
                "kind": "message",
                "role": "owner",
                "conversation": "sarah.0123456789abcdef01234567",
                "text": "Ready",
            },
        })
        .to_string();
        let projection_tags = vec![vec!["p".into(), device.public_key().to_hex()]];
        assert_eq!(
            private_record_kind(
                &projection,
                &owner_public_key_hex,
                &device.public_key().to_hex(),
                &owner_public_key_hex,
                &sarah.public_key().to_hex(),
                &projection_tags,
            )
            .expect("owner projection admission"),
            "owner_projection"
        );
        assert_eq!(query_gap_after_eose(12, false), GapState::None);
        assert_eq!(query_gap_after_eose(256, false), GapState::Possible);
    }

    /// The owner's own send has to be readable back off the relay, because
    /// `load_issue31_source_event` will not confirm a source it cannot see and
    /// the command result degrades to `reason.omega.projection_failed` with no
    /// owner projection ever published — which is exactly what a phone saw on
    /// omega#49: the send published, the host admitted the command, and no
    /// reply ever arrived.
    ///
    /// The rumor is built through the production tag builder and the production
    /// event builder rather than hand-written. That is the whole point: every
    /// prior admission test wrote its own single-`p` rumor and so never saw
    /// that `EventBuilder::build` strips the author's own `p` tag, leaving the
    /// owner's message addressed only to Sarah and unreadable by its own author.
    #[test]
    fn owner_authored_conversation_message_survives_its_own_round_trip() {
        let owner = Keys::generate();
        let sarah = Keys::generate();
        let conversation_ref = "sarah.0123456789abcdef01234567";
        let tags = crate::sarah_conversation::conversation_tags(
            conversation_ref,
            &owner.public_key().to_hex(),
            &sarah.public_key().to_hex(),
        )
        .expect("production conversation tags");
        let mut rumor = EventBuilder::new(Kind::PrivateDirectMessage, "durable hello")
            .tags(tags)
            .build(owner.public_key());
        rumor.ensure_id();
        let rumor_event_id = rumor.id.expect("rumor id").to_hex();

        let relay = WebSocketRelayAdapter::new_for_keys_with_policy(
            vec!["wss://relay.example.com".to_string()],
            owner.clone(),
            sarah.public_key().to_hex(),
            Vec::new(),
            Vec::new(),
        )
        .expect("relay");

        // The host's own copy: sender and recipient are both the owner key,
        // which is the copy `load_issue31_source_event` has to find.
        let own_copy = smol::block_on(EventBuilder::gift_wrap(
            &owner,
            &owner.public_key(),
            rumor.clone(),
            [],
        ))
        .expect("gift wrap to the owner");
        let stored = relay
            .admit_event(&own_copy, conversation_ref)
            .expect("the owner's own message must be admissible")
            .expect("the owner's own message must be stored");
        assert_eq!(stored.record_kind, "message");
        assert_eq!(stored.event_id, rumor_event_id);
        assert_eq!(stored.pubkey, owner.public_key().to_hex());
        assert_eq!(stored.kind, crate::ISSUE31_PRIVATE_RUMOR_KIND);
        // The wire shape this is defending: the author is not in its own `p`
        // set, so a reader that demands to see itself there can never read its
        // own send back.
        assert_eq!(
            stored
                .tags
                .iter()
                .filter(|tag| tag.first().map(String::as_str) == Some("p"))
                .filter_map(|tag| tag.get(1).cloned())
                .collect::<Vec<_>>(),
            vec![sarah.public_key().to_hex()]
        );

        // The other direction has to keep working: Sarah authors, the owner
        // reads, and now it is the owner's `p` tag that survives.
        let sarah_tags = crate::sarah_conversation::conversation_tags(
            conversation_ref,
            &owner.public_key().to_hex(),
            &sarah.public_key().to_hex(),
        )
        .expect("production conversation tags");
        let mut sarah_rumor = EventBuilder::new(Kind::PrivateDirectMessage, "Sarah here")
            .tags(sarah_tags)
            .build(sarah.public_key());
        sarah_rumor.ensure_id();
        let sarah_wrap = smol::block_on(EventBuilder::gift_wrap(
            &sarah,
            &owner.public_key(),
            sarah_rumor,
            [],
        ))
        .expect("gift wrap to the owner");
        let stored_reply = relay
            .admit_event(&sarah_wrap, conversation_ref)
            .expect("Sarah's message must be admissible")
            .expect("Sarah's message must be stored");
        assert_eq!(stored_reply.record_kind, "message");
        assert_eq!(stored_reply.pubkey, sarah.public_key().to_hex());

        // The leak guard the single-recipient rule was buying is unchanged: a
        // third recipient on the rumor is still refused outright.
        let mut leaky_tags = crate::sarah_conversation::conversation_tags(
            conversation_ref,
            &owner.public_key().to_hex(),
            &sarah.public_key().to_hex(),
        )
        .expect("production conversation tags");
        let stranger = Keys::generate();
        leaky_tags.push(
            nostr::Tag::parse(["p", stranger.public_key().to_hex().as_str()]).expect("p tag"),
        );
        let mut leaky = EventBuilder::new(Kind::PrivateDirectMessage, "leaky")
            .tags(leaky_tags)
            .build(owner.public_key());
        leaky.ensure_id();
        let leaky_wrap = smol::block_on(EventBuilder::gift_wrap(
            &owner,
            &owner.public_key(),
            leaky,
            [],
        ))
        .expect("gift wrap");
        assert!(
            relay.admit_event(&leaky_wrap, conversation_ref).is_err(),
            "a conversation rumor naming a third recipient must still be refused"
        );

        // And a rumor addressed away from this reader entirely is still refused.
        let elsewhere_tags = crate::sarah_conversation::conversation_tags(
            conversation_ref,
            &stranger.public_key().to_hex(),
            &sarah.public_key().to_hex(),
        )
        .expect("production conversation tags");
        let mut elsewhere = EventBuilder::new(Kind::PrivateDirectMessage, "elsewhere")
            .tags(elsewhere_tags)
            .build(owner.public_key());
        elsewhere.ensure_id();
        let elsewhere_wrap = smol::block_on(EventBuilder::gift_wrap(
            &owner,
            &owner.public_key(),
            elsewhere,
            [],
        ))
        .expect("gift wrap");
        assert!(
            relay
                .admit_event(&elsewhere_wrap, conversation_ref)
                .is_err(),
            "a conversation rumor addressed to another pair must still be refused"
        );
    }

    #[test]
    fn memory_record_families_use_their_wire_contracts_without_conversation_tags() {
        let owner = Keys::generate();
        let sarah = Keys::generate();
        let relay = WebSocketRelayAdapter::new_for_keys_with_policy(
            vec!["wss://relay.example.com".to_string()],
            owner.clone(),
            sarah.public_key().to_hex(),
            Vec::new(),
            Vec::new(),
        )
        .expect("relay");

        let engram = EventBuilder::new(Kind::from(NIP_AE_KIND), "nip44:ciphertext")
            .tags(vec![
                nostr::Tag::parse(["d", &"a".repeat(64)]).expect("d tag"),
                nostr::Tag::parse(["p", &owner.public_key().to_hex()]).expect("p tag"),
                nostr::Tag::parse(["alt", "encrypted agent memory record"]).expect("alt tag"),
            ])
            .sign_with_keys(&sarah)
            .expect("engram");
        assert_eq!(
            relay
                .admit_event(&engram, "sarah.test")
                .expect("engram admission")
                .expect("engram admitted")
                .record_kind,
            "memory"
        );

        let read_state = EventBuilder::new(Kind::from(NIP_RS_KIND), "nip44:ciphertext")
            .tags(vec![
                nostr::Tag::parse(["d", "read-state:omega-desktop"]).expect("d tag"),
                nostr::Tag::parse(["t", "read-state"]).expect("t tag"),
                nostr::Tag::parse(["alt", "encrypted read state"]).expect("alt tag"),
            ])
            .sign_with_keys(&owner)
            .expect("read state");
        assert!(
            relay
                .admit_event(&read_state, "sarah.test")
                .expect("read state admission")
                .is_some()
        );

        let reminder = EventBuilder::new(Kind::from(NIP_ER_KIND), "nip44:ciphertext")
            .tags(vec![
                nostr::Tag::parse(["d", "reminder-1"]).expect("d tag"),
                nostr::Tag::parse(["alt", "Encrypted reminder"]).expect("alt tag"),
                nostr::Tag::parse(["not_before", "100"]).expect("not-before tag"),
                nostr::Tag::parse(["expiration", "200"]).expect("expiration tag"),
            ])
            .sign_with_keys(&sarah)
            .expect("reminder");
        assert!(
            relay
                .admit_event(&reminder, "sarah.test")
                .expect("reminder admission")
                .is_some()
        );

        let wrong_author = EventBuilder::new(Kind::from(NIP_AE_KIND), "nip44:ciphertext")
            .tags(engram.tags)
            .sign_with_keys(&owner)
            .expect("owner-authored engram");
        assert!(
            relay
                .admit_event(&wrong_author, "sarah.test")
                .expect("wrong author admission")
                .is_none()
        );
    }
}
