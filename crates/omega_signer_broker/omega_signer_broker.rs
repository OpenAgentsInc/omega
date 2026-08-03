use std::{
    collections::BTreeSet,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use async_io::Timer;
use async_tungstenite::{
    WebSocketStream,
    async_std::{ConnectStream, connect_async},
    tungstenite::Message,
};
use futures::{FutureExt, StreamExt, pin_mut, select};
use nostr::{Event, JsonUtil as _, Kind, PublicKey, Tag, Timestamp, UnsignedEvent};
use omega_identity::{
    AccountRegistryService, AccountSelectionToken, AdmittedSigningRequest, IdentityService,
    Nip46Capability, Nip46Error, Nip46InboundEvent, Nip46Operation, Nip46OperationRequest,
    Nip46OperationResult, Nip46RequestEnvelope, Nip46Service, NostrPublicKeyHex,
    RemoteSignerRuntimeOutcome, SigningResult,
};
use serde_json::{Value, json};
use thiserror::Error;

pub const FIRST_WAVE_SIGN_EVENT_KINDS: &[u16] = &[9, 1_111, 1_984, 22_242, 27_235];
pub const SIGNED_WORKROOM_EVENT_KINDS: &[u16] = &[
    32_150, 32_151, 32_152, 32_153, 32_154, 32_155, 32_156, 32_157, 32_158, 32_159, 32_160, 32_161,
    32_162, 32_163,
];
pub const PROFILE_METADATA_KIND: u16 = 0;
pub const GROUP_JOIN_REQUEST_KIND: u16 = 9_021;
pub const GROUP_LIST_KIND: u16 = 10_009;
const NIP46_EVENT_KIND: u16 = 24_133;
const MAX_INBOUND_FRAME_BYTES: usize = 2 * 1024 * 1024;
const MAX_CONTROL_FRAMES: usize = 64;
const DEFAULT_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSignerMetadata {
    pub capability: Nip46Capability,
}

#[cfg(test)]
mod protocol_tests {
    use nostr::{Keys, Tag};
    use omega_identity::{
        AccountRef, IdentityRef, PublicIdentity, ReceiptRef, SigningPurpose, UnsignedEventTemplate,
    };

    use super::*;

    fn selection(keys: &Keys) -> AccountSelectionToken {
        AccountSelectionToken {
            account_ref: AccountRef::new("remote-account").expect("account ref"),
            identity: PublicIdentity::from_public_key_hex(
                IdentityRef::new("remote-identity").expect("identity ref"),
                keys.public_key().to_hex(),
            )
            .expect("identity"),
            generation: 2,
        }
    }

    fn request(selection: &AccountSelectionToken) -> AdmittedSigningRequest {
        AdmittedSigningRequest {
            request_ref: ReceiptRef::new("remote-request").expect("request ref"),
            identity_ref: selection.identity.identity_ref().clone(),
            purpose: SigningPurpose::NostrEvent,
            event: UnsignedEventTemplate {
                created_at: 1_700_000_000,
                kind: 9,
                tags: vec![Tag::hashtag("omega").as_slice().to_vec()],
                content: "exact payload".to_string(),
            },
        }
    }

    #[test]
    fn first_wave_allowlist_is_exact() {
        assert_eq!(
            FIRST_WAVE_SIGN_EVENT_KINDS,
            &[9, 1_111, 1_984, 22_242, 27_235]
        );
        assert_eq!(
            SIGNED_WORKROOM_EVENT_KINDS,
            &[
                32_150, 32_151, 32_152, 32_153, 32_154, 32_155, 32_156, 32_157, 32_158, 32_159,
                32_160, 32_161, 32_162, 32_163,
            ]
        );
    }

    #[test]
    fn relay_filter_binds_kind_recipient_and_optional_author() {
        let client = NostrPublicKeyHex::new(Keys::generate().public_key().to_hex())
            .expect("client public key");
        let signer = NostrPublicKeyHex::new(Keys::generate().public_key().to_hex())
            .expect("signer public key");
        let filter = relay_request("sub", Some(&signer), &client);

        assert_eq!(filter[2]["kinds"], json!([24_133]));
        assert_eq!(filter[2]["#p"], json!([client.as_str()]));
        assert_eq!(filter[2]["authors"], json!([signer.as_str()]));
    }

    #[test]
    fn exact_signed_event_is_accepted_and_wrong_author_is_terminal() {
        let user_keys = Keys::generate();
        let selection = selection(&user_keys);
        let request = request(&selection);
        let signed = nostr::EventBuilder::new(
            Kind::from(request.event.kind),
            request.event.content.clone(),
        )
        .tags(vec![Tag::hashtag("omega")])
        .custom_created_at(Timestamp::from_secs(request.event.created_at))
        .sign_with_keys(&user_keys)
        .expect("signed event");
        let result = signing_result(&selection, &request, signed.as_json()).expect("result");
        verify_signing_result(&selection, &request, &result).expect("exact event");

        let wrong = nostr::EventBuilder::new(
            Kind::from(request.event.kind),
            request.event.content.clone(),
        )
        .tags(vec![Tag::hashtag("omega")])
        .custom_created_at(Timestamp::from_secs(request.event.created_at))
        .sign_with_keys(&Keys::generate())
        .expect("wrong signed event");
        let wrong_result =
            signing_result(&selection, &request, wrong.as_json()).expect("shape result");
        assert_eq!(
            verify_signing_result(&selection, &request, &wrong_result),
            Err(SignerBrokerError::InvalidSignedEvent)
        );
    }

    #[test]
    fn explicit_rejection_and_protocol_mismatch_are_not_retryable() {
        assert!(!SignerBrokerError::ExplicitRejection.is_retryable());
        assert!(!SignerBrokerError::WrongRequestId.is_retryable());
        assert!(!SignerBrokerError::WrongRelay.is_retryable());
        assert!(SignerBrokerError::Offline.is_retryable());
        assert!(SignerBrokerError::Timeout.is_retryable());
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSignEventRequest {
    pub request_id: String,
    pub capability_ref: String,
    pub selection: AccountSelectionToken,
    pub signer_public_key_hex: String,
    pub relays: BTreeSet<String>,
    pub signing_request: AdmittedSigningRequest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteSignEventOutcome {
    Signed {
        request_id: String,
        relay_url: String,
        signed_event_json: String,
    },
    Rejected {
        request_id: String,
        reason: String,
    },
    Revoked {
        request_id: String,
    },
}

pub trait RemoteSignerTransport: Send + Sync {
    fn sign_event(
        &self,
        request: RemoteSignEventRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Vec<RemoteSignEventOutcome>, SignerBrokerError>>
                + Send
                + 'static,
        >,
    >;
}

#[derive(Clone, Debug)]
pub struct Nip46RelayCoordinator {
    exchange_timeout: Duration,
}

impl Nip46RelayCoordinator {
    pub fn new(exchange_timeout: Duration) -> Result<Self, Nip46RelayError> {
        if exchange_timeout.is_zero() || exchange_timeout > Duration::from_secs(120) {
            return Err(Nip46RelayError::InvalidTimeout);
        }
        Ok(Self { exchange_timeout })
    }

    pub async fn exchange<T, F>(
        &self,
        envelope: &Nip46RequestEnvelope,
        expected_author: Option<&NostrPublicKeyHex>,
        client_public_key: &NostrPublicKeyHex,
        accept: F,
    ) -> Result<T, Nip46RelayError>
    where
        F: FnMut(&str, &str, u64) -> Result<Option<T>, Nip46Error>,
    {
        let publication = relay_publication(&envelope.event_json)?;
        self.coordinate(
            &envelope.relay_urls,
            &envelope.request_id,
            expected_author,
            client_public_key,
            Some(publication),
            accept,
        )
        .await
    }

    pub async fn listen<T, F>(
        &self,
        relay_urls: &[String],
        correlation_id: &str,
        expected_author: Option<&NostrPublicKeyHex>,
        client_public_key: &NostrPublicKeyHex,
        accept: F,
    ) -> Result<T, Nip46RelayError>
    where
        F: FnMut(&str, &str, u64) -> Result<Option<T>, Nip46Error>,
    {
        self.coordinate(
            relay_urls,
            correlation_id,
            expected_author,
            client_public_key,
            None,
            accept,
        )
        .await
    }

    async fn coordinate<T, F>(
        &self,
        relay_urls: &[String],
        correlation_id: &str,
        expected_author: Option<&NostrPublicKeyHex>,
        client_public_key: &NostrPublicKeyHex,
        publication: Option<Value>,
        mut accept: F,
    ) -> Result<T, Nip46RelayError>
    where
        F: FnMut(&str, &str, u64) -> Result<Option<T>, Nip46Error>,
    {
        if relay_urls.is_empty() {
            return Err(Nip46RelayError::NoRelay);
        }
        let deadline = Instant::now() + self.exchange_timeout;
        let subscription_id = format!("omega-nip46-{correlation_id}");
        let request = relay_request(&subscription_id, expected_author, client_public_key);
        let mut saw_relay_activity = false;
        let mut connected = false;

        for (relay_index, relay_url) in relay_urls.iter().enumerate() {
            if Instant::now() >= deadline {
                break;
            }
            let remaining_relays = relay_urls.len().saturating_sub(relay_index).max(1) as u32;
            let relay_deadline = Instant::now()
                + deadline.saturating_duration_since(Instant::now()) / remaining_relays;
            let connect_deadline = relay_deadline.min(Instant::now() + CONNECT_TIMEOUT);
            let connection = await_until(connect_async(relay_url.as_str()), connect_deadline).await;
            let Ok((mut socket, _)) = connection else {
                continue;
            };
            connected = true;
            if send_until(&mut socket, request.clone(), relay_deadline)
                .await
                .is_err()
            {
                continue;
            }
            if let Some(publication) = publication.clone()
                && send_until(&mut socket, publication, relay_deadline)
                    .await
                    .is_err()
            {
                continue;
            }

            let mut control_frames = 0_usize;
            while Instant::now() < relay_deadline {
                let frame = match next_until(&mut socket, relay_deadline).await {
                    Ok(Some(frame)) => frame,
                    Ok(None) | Err(Nip46RelayError::Offline | Nip46RelayError::Timeout) => break,
                    Err(error) => return Err(error),
                };
                saw_relay_activity = true;
                if let Message::Ping(payload) = frame {
                    if send_message_until(&mut socket, Message::Pong(payload), relay_deadline)
                        .await
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }
                let Some(event_json) =
                    response_event_json(frame, &subscription_id, &mut control_frames)?
                else {
                    continue;
                };
                let received_at = unix_time();
                match accept(relay_url, &event_json, received_at) {
                    Ok(Some(result)) => {
                        close_subscription(&mut socket, &subscription_id, relay_deadline).await;
                        return Ok(result);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        close_subscription(&mut socket, &subscription_id, relay_deadline).await;
                        return Err(Nip46RelayError::Protocol(error));
                    }
                }
            }
            close_subscription(&mut socket, &subscription_id, relay_deadline).await;
        }

        if Instant::now() < deadline {
            Timer::at(deadline).await;
        }
        if saw_relay_activity {
            Err(Nip46RelayError::Timeout)
        } else if connected {
            Err(Nip46RelayError::Silence)
        } else {
            Err(Nip46RelayError::Offline)
        }
    }
}

impl Default for Nip46RelayCoordinator {
    fn default() -> Self {
        Self {
            exchange_timeout: DEFAULT_EXCHANGE_TIMEOUT,
        }
    }
}

fn relay_request(
    subscription_id: &str,
    expected_author: Option<&NostrPublicKeyHex>,
    client_public_key: &NostrPublicKeyHex,
) -> Value {
    let mut filter = serde_json::Map::new();
    filter.insert("kinds".into(), json!([NIP46_EVENT_KIND]));
    filter.insert("#p".into(), json!([client_public_key.as_str()]));
    if let Some(expected_author) = expected_author {
        filter.insert("authors".into(), json!([expected_author.as_str()]));
    }
    json!(["REQ", subscription_id, Value::Object(filter)])
}

fn relay_publication(event_json: &str) -> Result<Value, Nip46RelayError> {
    let event =
        serde_json::from_str::<Value>(event_json).map_err(|_| Nip46RelayError::MalformedFrame)?;
    if !event.is_object() {
        return Err(Nip46RelayError::MalformedFrame);
    }
    Ok(json!(["EVENT", event]))
}

async fn await_until<F, T, E>(future: F, deadline: Instant) -> Result<T, ()>
where
    F: Future<Output = Result<T, E>>,
{
    let future = future.fuse();
    let timeout = FutureExt::fuse(Timer::at(deadline));
    pin_mut!(future, timeout);
    select! {
        result = future => result.map_err(|_| ()),
        _ = timeout => Err(()),
    }
}

async fn send_until(
    socket: &mut WebSocketStream<ConnectStream>,
    value: Value,
    deadline: Instant,
) -> Result<(), Nip46RelayError> {
    let text = serde_json::to_string(&value).map_err(|_| Nip46RelayError::MalformedFrame)?;
    await_until(socket.send(Message::Text(text.into())), deadline)
        .await
        .map_err(|_| Nip46RelayError::Offline)
}

async fn send_message_until(
    socket: &mut WebSocketStream<ConnectStream>,
    message: Message,
    deadline: Instant,
) -> Result<(), Nip46RelayError> {
    await_until(socket.send(message), deadline)
        .await
        .map_err(|_| Nip46RelayError::Offline)
}

async fn next_until(
    socket: &mut WebSocketStream<ConnectStream>,
    deadline: Instant,
) -> Result<Option<Message>, Nip46RelayError> {
    let read = FutureExt::fuse(socket.next());
    let timeout = FutureExt::fuse(Timer::at(deadline));
    pin_mut!(read, timeout);
    let message = select! {
        message = read => message,
        _ = timeout => return Err(Nip46RelayError::Timeout),
    };
    match message {
        Some(Ok(message)) => Ok(Some(message)),
        Some(Err(_)) => Err(Nip46RelayError::Offline),
        None => Ok(None),
    }
}

fn response_event_json(
    frame: Message,
    subscription_id: &str,
    control_frames: &mut usize,
) -> Result<Option<String>, Nip46RelayError> {
    let text = match frame {
        Message::Text(text) => text.to_string(),
        Message::Binary(bytes) => {
            String::from_utf8(bytes.to_vec()).map_err(|_| Nip46RelayError::MalformedFrame)?
        }
        Message::Close(_) => return Ok(None),
        _ => {
            *control_frames = control_frames.saturating_add(1);
            if *control_frames > MAX_CONTROL_FRAMES {
                return Err(Nip46RelayError::MalformedFrame);
            }
            return Ok(None);
        }
    };
    if text.len() > MAX_INBOUND_FRAME_BYTES {
        return Err(Nip46RelayError::MalformedFrame);
    }
    let value =
        serde_json::from_str::<Value>(&text).map_err(|_| Nip46RelayError::MalformedFrame)?;
    let Some(values) = value.as_array() else {
        return Err(Nip46RelayError::MalformedFrame);
    };
    if values.first().and_then(Value::as_str) != Some("EVENT") {
        *control_frames = control_frames.saturating_add(1);
        if *control_frames > MAX_CONTROL_FRAMES {
            return Err(Nip46RelayError::MalformedFrame);
        }
        return Ok(None);
    }
    if values.len() != 3 || values.get(1).and_then(Value::as_str) != Some(subscription_id) {
        return Err(Nip46RelayError::MalformedFrame);
    }
    let event = values
        .get(2)
        .filter(|event| event.is_object())
        .ok_or(Nip46RelayError::MalformedFrame)?;
    serde_json::to_string(event)
        .map(Some)
        .map_err(|_| Nip46RelayError::MalformedFrame)
}

async fn close_subscription(
    socket: &mut WebSocketStream<ConnectStream>,
    subscription_id: &str,
    deadline: Instant,
) {
    if let Err(error) = send_until(socket, json!(["CLOSE", subscription_id]), deadline).await {
        log::debug!("NIP-46 subscription close failed: {error}");
    }
    if let Err(error) = send_message_until(socket, Message::Close(None), deadline).await {
        log::debug!("NIP-46 websocket close failed: {error}");
    }
}

#[derive(Debug, Error)]
pub enum Nip46RelayError {
    #[error("the NIP-46 exchange timeout must be between one and 120 seconds")]
    InvalidTimeout,
    #[error("the NIP-46 request declares no relay")]
    NoRelay,
    #[error("no declared NIP-46 relay could be reached")]
    Offline,
    #[error("a declared NIP-46 relay was silent")]
    Silence,
    #[error("a declared NIP-46 relay was active but did not return a terminal response")]
    Timeout,
    #[error("the relay returned a malformed frame")]
    MalformedFrame,
    #[error("the NIP-46 protocol rejected the response: {0}")]
    Protocol(#[source] Nip46Error),
}

pub struct Nip46WebSocketTransport {
    service: Arc<Nip46Service>,
    coordinator: Nip46RelayCoordinator,
    timeout_seconds: u64,
}

impl Nip46WebSocketTransport {
    pub fn system() -> Self {
        Self {
            service: Arc::new(Nip46Service::system(*app_identity::CHANNEL)),
            coordinator: Nip46RelayCoordinator::default(),
            timeout_seconds: DEFAULT_EXCHANGE_TIMEOUT.as_secs(),
        }
    }

    pub fn new(service: Arc<Nip46Service>, timeout: Duration) -> Result<Self, Nip46RelayError> {
        let coordinator = Nip46RelayCoordinator::new(timeout)?;
        Ok(Self {
            service,
            coordinator,
            timeout_seconds: timeout.as_secs(),
        })
    }
}

impl RemoteSignerTransport for Nip46WebSocketTransport {
    fn sign_event(
        &self,
        request: RemoteSignEventRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Vec<RemoteSignEventOutcome>, SignerBrokerError>>
                + Send
                + 'static,
        >,
    > {
        let service = self.service.clone();
        let coordinator = self.coordinator.clone();
        let timeout_seconds = self.timeout_seconds;
        Box::pin(async move {
            let unsigned_event_json =
                unsigned_event_json(&request.selection, &request.signing_request)?;
            let capability_ref = request.capability_ref.clone();
            let envelope = service
                .begin_operation(
                    &capability_ref,
                    &request.selection,
                    Nip46OperationRequest::SignEvent {
                        unsigned_event_json,
                    },
                    unix_time(),
                    timeout_seconds,
                )
                .map_err(map_nip46_error)?;
            let signer_public_key = NostrPublicKeyHex::new(request.signer_public_key_hex.clone())
                .map_err(|_| SignerBrokerError::InvalidSignedEvent)?;
            let client_public_key = service
                .load_capability(&capability_ref)
                .map_err(map_nip46_error)?
                .client_public_key;
            let result = coordinator
                .exchange(
                    &envelope,
                    Some(&signer_public_key),
                    &client_public_key,
                    |relay_url, event_json, received_at| {
                        service
                            .receive_operation_response(
                                &capability_ref,
                                &request.selection,
                                Nip46InboundEvent {
                                    relay_url,
                                    event_json,
                                    received_at,
                                },
                            )
                            .map(|result| Some((result, relay_url.to_string())))
                    },
                )
                .await
                .map_err(map_relay_error);
            if result.is_err() {
                let deadline_result =
                    service.operation_deadline_outcome(&capability_ref, unix_time());
                if let Err(error) = deadline_result
                    && !matches!(error, Nip46Error::Silence | Nip46Error::Timeout)
                {
                    return Err(map_nip46_error(error));
                }
            }
            match result? {
                (Nip46OperationResult::SignedEvent(event), relay_url) => {
                    Ok(vec![RemoteSignEventOutcome::Signed {
                        request_id: request.request_id,
                        relay_url,
                        signed_event_json: event.as_json(),
                    }])
                }
                _ => Err(SignerBrokerError::InvalidSignedEvent),
            }
        })
    }
}

fn unsigned_event_json(
    selection: &AccountSelectionToken,
    request: &AdmittedSigningRequest,
) -> Result<String, SignerBrokerError> {
    request
        .validate()
        .map_err(|_| SignerBrokerError::InvalidSignedEvent)?;
    let public_key = PublicKey::from_hex(selection.identity.public_key_hex().as_str())
        .map_err(|_| SignerBrokerError::InvalidSignedEvent)?;
    let tags = request
        .event
        .tags
        .iter()
        .cloned()
        .map(Tag::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SignerBrokerError::InvalidSignedEvent)?;
    UnsignedEvent::new(
        public_key,
        Timestamp::from_secs(request.event.created_at),
        Kind::from(request.event.kind),
        tags,
        request.event.content.clone(),
    )
    .try_as_json()
    .map_err(|_| SignerBrokerError::InvalidSignedEvent)
}

pub trait SelectionValidator: Send + Sync {
    fn validate(&self, token: &AccountSelectionToken) -> Result<(), SignerBrokerError>;
}

pub struct RegistrySelectionValidator;

impl SelectionValidator for RegistrySelectionValidator {
    fn validate(&self, token: &AccountSelectionToken) -> Result<(), SignerBrokerError> {
        AccountRegistryService::for_channel(*app_identity::CHANNEL)
            .validate_signing_selection(token)
            .map_err(map_selection_error)
    }
}

/// Keep the registry's refusal reason instead of collapsing every failure to
/// "the active account selection changed". That collapse sent the 2026-07-30
/// owner outage hunting for a selection race when the actual refusal was an
/// account-state gate that never mentions selection at all.
fn map_selection_error(error: omega_identity::AccountRegistryError) -> SignerBrokerError {
    match error {
        omega_identity::AccountRegistryError::StaleSelection => {
            SignerBrokerError::StaleAccountSelection
        }
        other => SignerBrokerError::AccountNotSignable {
            reason: other.to_string(),
        },
    }
}

pub enum SignerRoute {
    Local {
        identity_service: Arc<IdentityService>,
    },
    RemoteNip46 {
        metadata: RemoteSignerMetadata,
        transport: Arc<dyn RemoteSignerTransport>,
    },
}

pub struct SignerBroker {
    validator: Arc<dyn SelectionValidator>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    record_system_outcomes: bool,
}

impl SignerBroker {
    pub fn system() -> Self {
        Self {
            validator: Arc::new(RegistrySelectionValidator),
            clock: Arc::new(unix_time),
            record_system_outcomes: true,
        }
    }

    pub fn with_validator(validator: Arc<dyn SelectionValidator>) -> Self {
        Self {
            validator,
            clock: Arc::new(unix_time),
            record_system_outcomes: false,
        }
    }

    pub fn with_validator_and_clock(
        validator: Arc<dyn SelectionValidator>,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Self {
        Self {
            validator,
            clock,
            record_system_outcomes: false,
        }
    }

    pub async fn sign(
        &self,
        route: &SignerRoute,
        selection: AccountSelectionToken,
        request: AdmittedSigningRequest,
    ) -> Result<SigningResult, SignerBrokerError> {
        self.validator.validate(&selection)?;
        ensure_request_identity(&selection, &request)?;
        let result = match route {
            SignerRoute::Local { identity_service } => identity_service
                .sign(&request)
                .map_err(|error| SignerBrokerError::LocalSigner(error.to_string()))?,
            SignerRoute::RemoteNip46 {
                metadata,
                transport,
            } => {
                authorize_remote_sign_event(
                    metadata,
                    &selection,
                    request.event.kind,
                    (self.clock)(),
                )?;
                validate_remote_metadata(metadata)?;
                let request_id = request.request_ref.as_str().to_string();
                let outcomes = match transport
                    .sign_event(RemoteSignEventRequest {
                        request_id: request_id.clone(),
                        capability_ref: metadata.capability.capability_ref.clone(),
                        selection: selection.clone(),
                        signer_public_key_hex: metadata
                            .capability
                            .remote_signer_public_key
                            .as_str()
                            .to_string(),
                        relays: metadata.capability.relays.iter().cloned().collect(),
                        signing_request: request.clone(),
                    })
                    .await
                {
                    Ok(outcomes) => outcomes,
                    Err(error) => {
                        self.record_remote_error(&selection, &error)?;
                        return Err(error);
                    }
                };
                match resolve_remote_outcomes(&selection, &request, metadata, &request_id, outcomes)
                {
                    Ok(result) => result,
                    Err(error) => {
                        self.record_remote_error(&selection, &error)?;
                        return Err(error);
                    }
                }
            }
        };
        self.validator.validate(&selection)?;
        verify_signing_result(&selection, &request, &result)?;
        if matches!(route, SignerRoute::RemoteNip46 { .. }) {
            self.record_remote_outcome(
                &selection,
                RemoteSignerRuntimeOutcome::Ready {
                    used_at: (self.clock)(),
                },
            )?;
        }
        Ok(result)
    }

    fn record_remote_error(
        &self,
        selection: &AccountSelectionToken,
        error: &SignerBrokerError,
    ) -> Result<(), SignerBrokerError> {
        let outcome = match error {
            SignerBrokerError::Offline | SignerBrokerError::Timeout => {
                Some(RemoteSignerRuntimeOutcome::Offline)
            }
            SignerBrokerError::ExplicitRejection => Some(RemoteSignerRuntimeOutcome::Rejected),
            SignerBrokerError::Revoked => Some(RemoteSignerRuntimeOutcome::Revoked),
            _ => None,
        };
        if let Some(outcome) = outcome {
            self.record_remote_outcome(selection, outcome)?;
        }
        Ok(())
    }

    fn record_remote_outcome(
        &self,
        selection: &AccountSelectionToken,
        outcome: RemoteSignerRuntimeOutcome,
    ) -> Result<(), SignerBrokerError> {
        if !self.record_system_outcomes {
            return Ok(());
        }
        AccountRegistryService::for_channel(*app_identity::CHANNEL)
            .record_remote_signer_outcome(selection, outcome, (self.clock)())
            .map(|_| ())
            .map_err(|_| SignerBrokerError::AccountState)
    }
}

impl Default for SignerBroker {
    fn default() -> Self {
        Self::system()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum SignerBrokerError {
    #[error("the active account selection changed")]
    StaleAccountSelection,
    #[error("the active account cannot sign right now ({reason})")]
    AccountNotSignable { reason: String },
    #[error("the signing request targets a different identity")]
    WrongIdentity,
    #[error("NIP-46 signing was not declared for event kind {kind}")]
    EventKindNotDeclared { kind: u16 },
    #[error("kind-0 profile signing requires explicit remote signer permission")]
    ProfilePermissionRequired,
    #[error("kind-9021 group join signing requires explicit remote signer permission")]
    GroupJoinPermissionRequired,
    #[error("kind-10009 group-list signing requires explicit remote signer permission")]
    GroupListPermissionRequired,
    #[error("the NIP-46 capability has no bounded relay")]
    NoRelay,
    #[error("the remote signer explicitly rejected the request")]
    ExplicitRejection,
    #[error("the remote signer capability was revoked")]
    Revoked,
    #[error("the remote signer returned the wrong request id")]
    WrongRequestId,
    #[error("the remote signer answered through an undeclared relay")]
    WrongRelay,
    #[error("the remote signer returned more than one response")]
    DuplicateResponse,
    #[error("the remote signer returned no response before the bounded exchange ended")]
    Timeout,
    #[error("the remote signer returned a malformed or mismatched event")]
    InvalidSignedEvent,
    #[error("the local signer failed: {0}")]
    LocalSigner(String),
    #[error("the remote signer is offline")]
    Offline,
    #[error("the remote signer account state could not be updated")]
    AccountState,
}

impl SignerBrokerError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Offline | Self::Timeout)
    }
}

fn ensure_request_identity(
    selection: &AccountSelectionToken,
    request: &AdmittedSigningRequest,
) -> Result<(), SignerBrokerError> {
    if request.identity_ref != *selection.identity.identity_ref() {
        return Err(SignerBrokerError::WrongIdentity);
    }
    Ok(())
}

fn validate_remote_metadata(metadata: &RemoteSignerMetadata) -> Result<(), SignerBrokerError> {
    PublicKey::from_hex(metadata.capability.remote_signer_public_key.as_str())
        .map_err(|_| SignerBrokerError::InvalidSignedEvent)?;
    if metadata.capability.relays.is_empty() {
        return Err(SignerBrokerError::NoRelay);
    }
    Ok(())
}

fn authorize_remote_sign_event(
    metadata: &RemoteSignerMetadata,
    selection: &AccountSelectionToken,
    kind: u16,
    now: u64,
) -> Result<(), SignerBrokerError> {
    if kind != PROFILE_METADATA_KIND
        && kind != GROUP_JOIN_REQUEST_KIND
        && kind != GROUP_LIST_KIND
        && !FIRST_WAVE_SIGN_EVENT_KINDS.contains(&kind)
        && !SIGNED_WORKROOM_EVENT_KINDS.contains(&kind)
    {
        return Err(SignerBrokerError::EventKindNotDeclared { kind });
    }
    metadata
        .capability
        .authorize(selection, Nip46Operation::SignEvent { kind }, now)
        .map_err(|error| match error {
            Nip46Error::UndeclaredCapability if kind == PROFILE_METADATA_KIND => {
                SignerBrokerError::ProfilePermissionRequired
            }
            Nip46Error::UndeclaredCapability if kind == GROUP_JOIN_REQUEST_KIND => {
                SignerBrokerError::GroupJoinPermissionRequired
            }
            Nip46Error::UndeclaredCapability if kind == GROUP_LIST_KIND => {
                SignerBrokerError::GroupListPermissionRequired
            }
            Nip46Error::UndeclaredCapability => SignerBrokerError::EventKindNotDeclared { kind },
            error => map_nip46_error(error),
        })
}

fn resolve_remote_outcomes(
    selection: &AccountSelectionToken,
    request: &AdmittedSigningRequest,
    metadata: &RemoteSignerMetadata,
    request_id: &str,
    outcomes: Vec<RemoteSignEventOutcome>,
) -> Result<SigningResult, SignerBrokerError> {
    if outcomes.is_empty() {
        return Err(SignerBrokerError::Timeout);
    }
    if outcomes.len() != 1 {
        return Err(SignerBrokerError::DuplicateResponse);
    }
    match outcomes.into_iter().next() {
        Some(RemoteSignEventOutcome::Signed {
            request_id: response_id,
            relay_url,
            signed_event_json,
        }) => {
            if response_id != request_id {
                return Err(SignerBrokerError::WrongRequestId);
            }
            if !metadata.capability.relays.contains(&relay_url) {
                return Err(SignerBrokerError::WrongRelay);
            }
            signing_result(selection, request, signed_event_json)
        }
        Some(RemoteSignEventOutcome::Rejected {
            request_id: response_id,
            ..
        }) => {
            if response_id != request_id {
                return Err(SignerBrokerError::WrongRequestId);
            }
            Err(SignerBrokerError::ExplicitRejection)
        }
        Some(RemoteSignEventOutcome::Revoked {
            request_id: response_id,
        }) => {
            if response_id != request_id {
                return Err(SignerBrokerError::WrongRequestId);
            }
            Err(SignerBrokerError::Revoked)
        }
        None => Err(SignerBrokerError::Timeout),
    }
}

fn map_nip46_error(error: Nip46Error) -> SignerBrokerError {
    match error {
        Nip46Error::StaleGeneration => SignerBrokerError::StaleAccountSelection,
        Nip46Error::Rejected => SignerBrokerError::ExplicitRejection,
        Nip46Error::Revoked => SignerBrokerError::Revoked,
        Nip46Error::Timeout => SignerBrokerError::Timeout,
        Nip46Error::Silence => SignerBrokerError::Offline,
        Nip46Error::WrongAuthor => SignerBrokerError::InvalidSignedEvent,
        Nip46Error::WrongRequestId => SignerBrokerError::WrongRequestId,
        Nip46Error::WrongRelay => SignerBrokerError::WrongRelay,
        Nip46Error::DuplicateResponse => SignerBrokerError::DuplicateResponse,
        Nip46Error::InvalidEvent
        | Nip46Error::MalformedCiphertext
        | Nip46Error::MalformedResponse
        | Nip46Error::InvalidChallenge => SignerBrokerError::InvalidSignedEvent,
        Nip46Error::UndeclaredCapability => SignerBrokerError::InvalidSignedEvent,
        _ => SignerBrokerError::InvalidSignedEvent,
    }
}

fn map_relay_error(error: Nip46RelayError) -> SignerBrokerError {
    match error {
        Nip46RelayError::NoRelay => SignerBrokerError::NoRelay,
        Nip46RelayError::Offline | Nip46RelayError::Silence => SignerBrokerError::Offline,
        Nip46RelayError::Timeout => SignerBrokerError::Timeout,
        Nip46RelayError::Protocol(error) => map_nip46_error(error),
        Nip46RelayError::InvalidTimeout | Nip46RelayError::MalformedFrame => {
            SignerBrokerError::InvalidSignedEvent
        }
    }
}

fn unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn signing_result(
    selection: &AccountSelectionToken,
    request: &AdmittedSigningRequest,
    signed_event_json: String,
) -> Result<SigningResult, SignerBrokerError> {
    let event =
        Event::from_json(&signed_event_json).map_err(|_| SignerBrokerError::InvalidSignedEvent)?;
    event
        .verify()
        .map_err(|_| SignerBrokerError::InvalidSignedEvent)?;
    Ok(SigningResult {
        request_ref: request.request_ref.clone(),
        identity: selection.identity.clone(),
        event_id: event.id.to_hex(),
        signature: event.sig.to_string(),
        signed_event_json,
    })
}

fn verify_signing_result(
    selection: &AccountSelectionToken,
    request: &AdmittedSigningRequest,
    result: &SigningResult,
) -> Result<(), SignerBrokerError> {
    if result.request_ref != request.request_ref || result.identity != selection.identity {
        return Err(SignerBrokerError::InvalidSignedEvent);
    }
    let event = Event::from_json(&result.signed_event_json)
        .map_err(|_| SignerBrokerError::InvalidSignedEvent)?;
    event
        .verify()
        .map_err(|_| SignerBrokerError::InvalidSignedEvent)?;
    let expected_id = request
        .event_id(&selection.identity)
        .map_err(|_| SignerBrokerError::InvalidSignedEvent)?;
    if event.id != expected_id
        || event.id.to_hex() != result.event_id
        || event.sig.to_string() != result.signature
        || event.pubkey.to_hex() != selection.identity.public_key_hex().as_str()
        || event.kind.as_u16() != request.event.kind
        || event.created_at.as_secs() != request.event.created_at
        || event.content != request.event.content
        || event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect::<Vec<_>>()
            != request.event.tags
    {
        return Err(SignerBrokerError::InvalidSignedEvent);
    }
    Ok(())
}
