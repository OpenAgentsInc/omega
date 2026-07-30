use std::{collections::BTreeSet, future::Future, pin::Pin, time::Duration};

use anyhow::{Context as _, Result, anyhow, ensure};
use async_tungstenite::{async_std::connect_async, tungstenite::Message};
use futures::StreamExt as _;
use nostr::{Event, JsonUtil as _};
use omega_identity::{
    AccountSelectionToken, AdmittedSigningRequest, ReceiptRef, RelayAuthenticationReceipt,
    RelayConnectionAuthenticationState, SigningPurpose, SigningResult, UnsignedEventTemplate,
};
use omega_invites::{
    JoinAccountFence, JoinMutationExecutor, JoinMutationOutcome, JoinPlan, JoinStepKind,
    JoinStepStatus, JoinStoreError, JoinTransactionProjection, JoinTransactionStore,
    Nip29GroupInvite, OpaqueInviteEvidence, PlannedJoinStep, PreparedJoinMutation,
};
use omega_signer_broker::{SignerBroker, SignerRoute};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::omega_nostr_profile_transport::{GroupCoordinate, canonical_wss_relay};

pub const GROUP_JOIN_REQUEST_KIND: u16 = 9_021;
pub const GROUP_MEMBER_ADDED_KIND: u16 = 9_000;
pub const GROUP_MEMBER_REMOVED_KIND: u16 = 9_001;
pub const GROUP_LIST_KIND: u16 = 10_009;
const MAX_JOIN_CODE_BYTES: usize = 512;
const MAX_RELAY_FRAME_BYTES: usize = 64 * 1024;
const MAX_RELAY_REASON_BYTES: usize = 256;
const MAX_MEMBERSHIP_EVENTS: usize = 64;
const MAX_RELAY_FRAMES: usize = 128;
const RELAY_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nip29JoinMutationRequest {
    pub relay_url: String,
    pub relay_public_key_hex: String,
    pub group_id: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invite_code: Option<Vec<u8>>,
}

impl Nip29JoinMutationRequest {
    pub fn from_invite(invite: &Nip29GroupInvite, created_at: u64) -> Result<Self> {
        let coordinate = GroupCoordinate::new(&invite.relay_url, &invite.group_id)?;
        ensure!(
            invite.relay_public_key_hex.len() == 64
                && invite
                    .relay_public_key_hex
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "NIP-29 relay self key is invalid"
        );
        ensure!(created_at > 0, "NIP-29 join timestamp must be nonzero");
        Ok(Self {
            relay_url: coordinate.relay_url,
            relay_public_key_hex: invite.relay_public_key_hex.clone(),
            group_id: coordinate.group_id,
            created_at,
            invite_code: invite.invite_code().map(<[u8]>::to_vec),
        })
    }

    pub fn coordinate(&self) -> Result<GroupCoordinate> {
        GroupCoordinate::new(&self.relay_url, &self.group_id)
    }

    pub fn invite_code(&self) -> Option<&[u8]> {
        self.invite_code.as_deref()
    }
}

impl std::fmt::Debug for Nip29JoinMutationRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Nip29JoinMutationRequest")
            .field("relay_url", &self.relay_url)
            .field("relay_public_key_hex", &self.relay_public_key_hex)
            .field("group_id", &self.group_id)
            .field("created_at", &self.created_at)
            .field(
                "invite_code",
                &self.invite_code.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedRelayGroupMetadata {
    request: Nip29JoinMutationRequest,
    event_id: String,
}

impl VerifiedRelayGroupMetadata {
    pub fn request(&self) -> &Nip29JoinMutationRequest {
        &self.request
    }

    pub fn event_id(&self) -> &str {
        &self.event_id
    }
}

pub fn verify_relay_group_metadata(
    request: &Nip29JoinMutationRequest,
    event: &Event,
) -> Result<VerifiedRelayGroupMetadata> {
    event.verify().context("verifying NIP-29 group metadata")?;
    ensure!(
        event.kind.as_u16() == 39_000
            && event.pubkey.to_hex() == request.relay_public_key_hex
            && exact_single_tag(event, "d") == Some(request.group_id.as_str()),
        "NIP-29 metadata is not signed by the invited relay self key for this group"
    );
    Ok(VerifiedRelayGroupMetadata {
        request: request.clone(),
        event_id: event.id.to_hex(),
    })
}

pub fn nip29_join_plan(
    invite: &Nip29GroupInvite,
    created_at: u64,
) -> Result<JoinPlan, JoinStoreError> {
    let request = Nip29JoinMutationRequest::from_invite(invite, created_at)
        .map_err(|_| JoinStoreError::InvalidPlan)?;
    let exact_request = serde_json::to_vec(&request).map_err(|_| JoinStoreError::InvalidPlan)?;
    let invite_evidence = OpaqueInviteEvidence {
        profile_hint: omega_invites::InviteProfile::Nip29,
        sha256: format!("{:x}", Sha256::digest(&exact_request)),
        byte_length: exact_request.len(),
    };
    JoinPlan::new(
        invite_evidence,
        vec![
            PlannedJoinStep::new(JoinStepKind::AddRelay, false, exact_request.clone())?,
            PlannedJoinStep::new(
                JoinStepKind::Nip42Authenticate,
                false,
                exact_request.clone(),
            )?,
            PlannedJoinStep::new(JoinStepKind::RequestNip29Join, true, exact_request.clone())?,
            PlannedJoinStep::new(
                JoinStepKind::AwaitNip29Admission,
                true,
                exact_request.clone(),
            )?,
            PlannedJoinStep::new(JoinStepKind::UpdateNip29GroupList, true, exact_request)?,
        ],
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupJoinError {
    InvalidRequest(String),
    Signing(String),
    MismatchedSignedEvent,
}

impl std::fmt::Display for GroupJoinError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRequest(message) => formatter.write_str(message),
            Self::Signing(message) => write!(formatter, "signing group join request: {message}"),
            Self::MismatchedSignedEvent => {
                formatter.write_str("the signer returned a different group join request")
            }
        }
    }
}

impl std::error::Error for GroupJoinError {}

pub fn group_join_template(
    coordinate: &GroupCoordinate,
    invite_code: Option<&str>,
    created_at: u64,
) -> std::result::Result<UnsignedEventTemplate, GroupJoinError> {
    if created_at == 0 {
        return Err(GroupJoinError::InvalidRequest(
            "the group join timestamp must be nonzero".to_string(),
        ));
    }
    let mut tags = vec![vec!["h".to_string(), coordinate.group_id.clone()]];
    if let Some(invite_code) = invite_code {
        if invite_code.is_empty()
            || invite_code.len() > MAX_JOIN_CODE_BYTES
            || invite_code.chars().any(char::is_control)
        {
            return Err(GroupJoinError::InvalidRequest(
                "the group invite code is invalid".to_string(),
            ));
        }
        tags.push(vec!["code".to_string(), invite_code.to_string()]);
    }
    Ok(UnsignedEventTemplate {
        created_at,
        kind: GROUP_JOIN_REQUEST_KIND,
        tags,
        content: String::new(),
    })
}

pub async fn sign_group_join_request(
    broker: &SignerBroker,
    route: &SignerRoute,
    selection: AccountSelectionToken,
    coordinate: &GroupCoordinate,
    invite_code: Option<&str>,
    created_at: u64,
    request_ref: ReceiptRef,
) -> std::result::Result<Event, GroupJoinError> {
    let expected = group_join_template(coordinate, invite_code, created_at)?;
    let result = broker
        .sign(
            route,
            selection.clone(),
            AdmittedSigningRequest {
                request_ref,
                identity_ref: selection.identity.identity_ref().clone(),
                purpose: SigningPurpose::NostrEvent,
                event: expected.clone(),
            },
        )
        .await
        .map_err(|error| GroupJoinError::Signing(error.to_string()))?;
    verify_signed_join_request(&selection, &expected, &result)
}

fn verify_signed_join_request(
    selection: &AccountSelectionToken,
    expected: &UnsignedEventTemplate,
    result: &SigningResult,
) -> std::result::Result<Event, GroupJoinError> {
    verify_exact_signed_template(selection, expected, result)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupListMutation {
    Add {
        coordinate: GroupCoordinate,
        label: Option<String>,
    },
    Remove(GroupCoordinate),
}

pub fn merge_relay_qualified_group_list(
    expected_author: &str,
    current: Option<&Event>,
    mutation: GroupListMutation,
    created_at: u64,
) -> Result<UnsignedEventTemplate> {
    ensure!(created_at > 0, "group-list timestamp must be nonzero");
    let (mut tags, content) = if let Some(current) = current {
        current
            .verify()
            .context("verifying current kind-10009 event")?;
        ensure!(
            current.kind.as_u16() == GROUP_LIST_KIND && current.pubkey.to_hex() == expected_author,
            "current group list is not bound to the selected author"
        );
        ensure!(
            current.content.len() <= MAX_RELAY_FRAME_BYTES && current.tags.len() <= 256,
            "current group list exceeds its projection bounds"
        );
        (
            current
                .tags
                .iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect::<Vec<_>>(),
            current.content.clone(),
        )
    } else {
        (Vec::new(), String::new())
    };

    let target = match &mutation {
        GroupListMutation::Add { coordinate, .. } | GroupListMutation::Remove(coordinate) => {
            coordinate
        }
    };
    let removing_target = matches!(&mutation, GroupListMutation::Remove(_));
    let mut seen_coordinates = BTreeSet::new();
    tags.retain(|tag| {
        let Some(coordinate) = group_coordinate_from_tag(tag) else {
            return true;
        };
        if coordinate == *target {
            return !removing_target && seen_coordinates.insert(coordinate);
        }
        seen_coordinates.insert(coordinate)
    });

    if let GroupListMutation::Add { coordinate, label } = mutation
        && !seen_coordinates.contains(&coordinate)
    {
        let mut tag = vec![
            "group".to_string(),
            coordinate.group_id,
            coordinate.relay_url,
        ];
        if let Some(label) = label {
            ensure!(
                !label.is_empty() && label.len() <= 256 && !label.chars().any(char::is_control),
                "group label is invalid"
            );
            tag.push(label);
        }
        tags.push(tag);
    }
    ensure!(tags.len() <= 256, "merged group list exceeds its tag cap");
    Ok(UnsignedEventTemplate {
        created_at,
        kind: GROUP_LIST_KIND,
        tags,
        content,
    })
}

pub async fn sign_group_list_update(
    broker: &SignerBroker,
    route: &SignerRoute,
    selection: AccountSelectionToken,
    template: UnsignedEventTemplate,
    request_ref: ReceiptRef,
) -> std::result::Result<Event, GroupJoinError> {
    if template.kind != GROUP_LIST_KIND {
        return Err(GroupJoinError::InvalidRequest(
            "group-list signing requires an exact kind-10009 template".to_string(),
        ));
    }
    let result = broker
        .sign(
            route,
            selection.clone(),
            AdmittedSigningRequest {
                request_ref,
                identity_ref: selection.identity.identity_ref().clone(),
                purpose: SigningPurpose::NostrEvent,
                event: template.clone(),
            },
        )
        .await
        .map_err(|error| GroupJoinError::Signing(error.to_string()))?;
    verify_exact_signed_template(&selection, &template, &result)
}

fn verify_exact_signed_template(
    selection: &AccountSelectionToken,
    expected: &UnsignedEventTemplate,
    result: &SigningResult,
) -> std::result::Result<Event, GroupJoinError> {
    let event = Event::from_json(&result.signed_event_json)
        .map_err(|_| GroupJoinError::MismatchedSignedEvent)?;
    event
        .verify()
        .map_err(|_| GroupJoinError::MismatchedSignedEvent)?;
    let returned_tags = event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect::<Vec<_>>();
    if event.kind.as_u16() != expected.kind
        || event.created_at.as_secs() != expected.created_at
        || event.content.as_bytes() != expected.content.as_bytes()
        || returned_tags != expected.tags
        || event.pubkey.to_hex() != selection.identity.public_key_hex().as_str()
        || event.id.to_hex() != result.event_id
        || event.sig.to_string() != result.signature
        || result.identity != selection.identity
    {
        return Err(GroupJoinError::MismatchedSignedEvent);
    }
    Ok(event)
}

fn group_coordinate_from_tag(tag: &[String]) -> Option<GroupCoordinate> {
    if tag.first().map(String::as_str) != Some("group") {
        return None;
    }
    let group_id = tag.get(1)?;
    let relay_url = tag.get(2)?;
    GroupCoordinate::new(relay_url, group_id).ok()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nip42AuthenticationReceipt {
    receipt_ref: String,
    relay_url: String,
    account_public_key_hex: String,
    connection_generation: u64,
}

impl Nip42AuthenticationReceipt {
    pub fn from_verified_receipt(
        receipt: &RelayAuthenticationReceipt,
        expected_relay_url: &str,
        expected_account_public_key_hex: &str,
    ) -> Result<Self> {
        receipt
            .validate()
            .context("validating NIP-42 authentication receipt")?;
        let expected_relay_url = canonical_wss_relay(expected_relay_url)?;
        ensure!(
            receipt.state == RelayConnectionAuthenticationState::Authenticated
                && canonical_wss_relay(&receipt.relay_url)? == expected_relay_url
                && receipt.account_public_key_hex.as_str() == expected_account_public_key_hex,
            "NIP-42 receipt is not authenticated for this relay and account"
        );
        let receipt_ref = receipt
            .auth_event_id
            .clone()
            .ok_or_else(|| anyhow!("authenticated NIP-42 receipt has no auth event"))?;
        Ok(Self {
            receipt_ref,
            relay_url: expected_relay_url,
            account_public_key_hex: expected_account_public_key_hex.to_string(),
            connection_generation: receipt.connection_generation,
        })
    }

    pub fn receipt_ref(&self) -> &str {
        &self.receipt_ref
    }

    pub fn connection_generation(&self) -> u64 {
        self.connection_generation
    }

    pub fn relay_url(&self) -> &str {
        &self.relay_url
    }

    pub fn account_public_key_hex(&self) -> &str {
        &self.account_public_key_hex
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nip42Result {
    NotRequested,
    Authenticated(Nip42AuthenticationReceipt),
    RequiredButUnavailable { challenge: String },
    Refused,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayPublicationResult {
    Accepted,
    Rejected { reason: String },
    TimedOut,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayPublicationReceipt {
    pub relay_url: String,
    pub event_id: String,
    pub result: RelayPublicationResult,
    pub nip42: Nip42Result,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MembershipState {
    Unknown,
    Member,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MembershipProjection {
    pub state: MembershipState,
    pub event_id: Option<String>,
    pub created_at: Option<u64>,
}

impl Default for MembershipProjection {
    fn default() -> Self {
        Self {
            state: MembershipState::Unknown,
            event_id: None,
            created_at: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MembershipQueryResult {
    Complete(MembershipProjection),
    TimedOut,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MembershipQueryReceipt {
    pub coordinate: GroupCoordinate,
    pub account_public_key_hex: String,
    pub result: MembershipQueryResult,
    pub nip42: Nip42Result,
}

pub fn fold_membership_events(
    coordinate: &GroupCoordinate,
    relay_public_key_hex: &str,
    account_public_key_hex: &str,
    events: impl IntoIterator<Item = Event>,
) -> Result<MembershipProjection> {
    let mut latest: Option<Event> = None;
    let mut admitted = 0_usize;
    for event in events {
        admitted = admitted.saturating_add(1);
        ensure!(
            admitted <= MAX_MEMBERSHIP_EVENTS,
            "membership response exceeds its event cap"
        );
        event.verify().context("verifying membership event")?;
        ensure!(
            matches!(
                event.kind.as_u16(),
                GROUP_MEMBER_ADDED_KIND | GROUP_MEMBER_REMOVED_KIND
            ),
            "relay returned a non-membership event"
        );
        ensure!(
            event.pubkey.to_hex() == relay_public_key_hex,
            "membership event was not signed by the invited relay self key"
        );
        ensure!(
            exact_single_tag(&event, "h") == Some(coordinate.group_id.as_str())
                && event.tags.iter().any(|tag| {
                    tag.as_slice().first().map(String::as_str) == Some("p")
                        && tag.as_slice().get(1).map(String::as_str) == Some(account_public_key_hex)
                }),
            "membership event is for another coordinate or account"
        );
        let replace = latest.as_ref().is_none_or(|current| {
            (event.created_at.as_secs(), event.id.to_hex())
                > (current.created_at.as_secs(), current.id.to_hex())
        });
        if replace {
            latest = Some(event);
        }
    }
    let Some(event) = latest else {
        return Ok(MembershipProjection::default());
    };
    Ok(MembershipProjection {
        state: if event.kind.as_u16() == GROUP_MEMBER_ADDED_KIND {
            MembershipState::Member
        } else {
            MembershipState::Removed
        },
        event_id: Some(event.id.to_hex()),
        created_at: Some(event.created_at.as_secs()),
    })
}

fn exact_single_tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    let mut tags = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some(name));
    let tag = tags.next()?;
    if tags.next().is_some() || tag.as_slice().len() != 2 {
        return None;
    }
    tag.as_slice().get(1).map(String::as_str)
}

pub trait AuthoritativeGroupRelayTransport: Send + Sync {
    fn publish_join_request(
        &self,
        coordinate: GroupCoordinate,
        event: Event,
    ) -> Pin<Box<dyn Future<Output = RelayPublicationReceipt> + Send + 'static>>;

    fn publish_group_list_update(
        &self,
        coordinate: GroupCoordinate,
        event: Event,
    ) -> Pin<Box<dyn Future<Output = RelayPublicationReceipt> + Send + 'static>>;

    fn query_membership(
        &self,
        coordinate: GroupCoordinate,
        relay_public_key_hex: String,
        account_public_key_hex: String,
    ) -> Pin<Box<dyn Future<Output = MembershipQueryReceipt> + Send + 'static>>;
}

#[derive(Clone, Default)]
pub struct WebSocketGroupRelayTransport;

impl AuthoritativeGroupRelayTransport for WebSocketGroupRelayTransport {
    fn publish_join_request(
        &self,
        coordinate: GroupCoordinate,
        event: Event,
    ) -> Pin<Box<dyn Future<Output = RelayPublicationReceipt> + Send + 'static>> {
        Box::pin(async move {
            match async_std::future::timeout(
                RELAY_OPERATION_TIMEOUT,
                publish_to_authoritative_relay(&coordinate, &event),
            )
            .await
            {
                Ok(Ok(receipt)) => receipt,
                Err(_) => RelayPublicationReceipt {
                    relay_url: coordinate.relay_url,
                    event_id: event.id.to_hex(),
                    result: RelayPublicationResult::TimedOut,
                    nip42: Nip42Result::NotRequested,
                },
                Ok(Err(_)) => RelayPublicationReceipt {
                    relay_url: coordinate.relay_url,
                    event_id: event.id.to_hex(),
                    result: RelayPublicationResult::Failed,
                    nip42: Nip42Result::NotRequested,
                },
            }
        })
    }

    fn publish_group_list_update(
        &self,
        coordinate: GroupCoordinate,
        event: Event,
    ) -> Pin<Box<dyn Future<Output = RelayPublicationReceipt> + Send + 'static>> {
        Box::pin(async move {
            match async_std::future::timeout(
                RELAY_OPERATION_TIMEOUT,
                publish_group_list_to_authoritative_relay(&coordinate, &event),
            )
            .await
            {
                Ok(Ok(receipt)) => receipt,
                Err(_) => RelayPublicationReceipt {
                    relay_url: coordinate.relay_url,
                    event_id: event.id.to_hex(),
                    result: RelayPublicationResult::TimedOut,
                    nip42: Nip42Result::NotRequested,
                },
                Ok(Err(_)) => RelayPublicationReceipt {
                    relay_url: coordinate.relay_url,
                    event_id: event.id.to_hex(),
                    result: RelayPublicationResult::Failed,
                    nip42: Nip42Result::NotRequested,
                },
            }
        })
    }

    fn query_membership(
        &self,
        coordinate: GroupCoordinate,
        relay_public_key_hex: String,
        account_public_key_hex: String,
    ) -> Pin<Box<dyn Future<Output = MembershipQueryReceipt> + Send + 'static>> {
        Box::pin(async move {
            match async_std::future::timeout(
                RELAY_OPERATION_TIMEOUT,
                query_authoritative_membership(
                    &coordinate,
                    &relay_public_key_hex,
                    &account_public_key_hex,
                ),
            )
            .await
            {
                Ok(Ok(receipt)) => receipt,
                Err(_) => MembershipQueryReceipt {
                    coordinate,
                    account_public_key_hex,
                    result: MembershipQueryResult::TimedOut,
                    nip42: Nip42Result::NotRequested,
                },
                Ok(Err(_)) => MembershipQueryReceipt {
                    coordinate,
                    account_public_key_hex,
                    result: MembershipQueryResult::Failed,
                    nip42: Nip42Result::NotRequested,
                },
            }
        })
    }
}

async fn publish_to_authoritative_relay(
    coordinate: &GroupCoordinate,
    event: &Event,
) -> Result<RelayPublicationReceipt> {
    ensure!(
        event.kind.as_u16() == GROUP_JOIN_REQUEST_KIND
            && exact_single_tag(event, "h") == Some(coordinate.group_id.as_str()),
        "join event is not bound to the authoritative coordinate"
    );
    publish_event_to_authoritative_relay(coordinate, event).await
}

async fn publish_event_to_authoritative_relay(
    coordinate: &GroupCoordinate,
    event: &Event,
) -> Result<RelayPublicationReceipt> {
    let relay_url = canonical_wss_relay(&coordinate.relay_url)?;
    let (mut socket, _) = connect_async(&relay_url)
        .await
        .context("connecting to authoritative group relay")?;
    socket
        .send(Message::Text(json!(["EVENT", event]).to_string().into()))
        .await
        .context("publishing group join request")?;
    let nip42 = Nip42Result::NotRequested;
    for _ in 0..MAX_RELAY_FRAMES {
        let Some(message) = socket.next().await else {
            break;
        };
        let Message::Text(text) = message.context("reading join publication response")? else {
            continue;
        };
        ensure!(
            text.len() <= MAX_RELAY_FRAME_BYTES,
            "join publication response exceeds its cap"
        );
        match parse_relay_response(&event.id.to_hex(), &text)? {
            ParsedRelayResponse::Unrelated => {}
            ParsedRelayResponse::Auth(challenge) => {
                return Ok(RelayPublicationReceipt {
                    relay_url,
                    event_id: event.id.to_hex(),
                    result: RelayPublicationResult::Failed,
                    nip42: Nip42Result::RequiredButUnavailable { challenge },
                });
            }
            ParsedRelayResponse::Accepted => {
                return Ok(RelayPublicationReceipt {
                    relay_url,
                    event_id: event.id.to_hex(),
                    result: RelayPublicationResult::Accepted,
                    nip42,
                });
            }
            ParsedRelayResponse::Rejected(reason) => {
                return Ok(RelayPublicationReceipt {
                    relay_url,
                    event_id: event.id.to_hex(),
                    result: RelayPublicationResult::Rejected { reason },
                    nip42,
                });
            }
        }
    }
    Ok(RelayPublicationReceipt {
        relay_url,
        event_id: event.id.to_hex(),
        result: RelayPublicationResult::TimedOut,
        nip42,
    })
}

async fn publish_group_list_to_authoritative_relay(
    coordinate: &GroupCoordinate,
    event: &Event,
) -> Result<RelayPublicationReceipt> {
    ensure!(
        event.kind.as_u16() == GROUP_LIST_KIND,
        "group-list publication requires kind 10009"
    );
    publish_event_to_authoritative_relay(coordinate, event).await
}

async fn query_authoritative_membership(
    coordinate: &GroupCoordinate,
    relay_public_key_hex: &str,
    account_public_key_hex: &str,
) -> Result<MembershipQueryReceipt> {
    let relay_url = canonical_wss_relay(&coordinate.relay_url)?;
    let subscription_id = "omega-group-membership";
    let (mut socket, _) = connect_async(&relay_url)
        .await
        .context("connecting to authoritative membership relay")?;
    socket
        .send(Message::Text(
            json!([
                "REQ",
                subscription_id,
                {
                    "kinds": [GROUP_MEMBER_ADDED_KIND, GROUP_MEMBER_REMOVED_KIND],
                    "#h": [coordinate.group_id],
                    "#p": [account_public_key_hex],
                    "limit": MAX_MEMBERSHIP_EVENTS,
                }
            ])
            .to_string()
            .into(),
        ))
        .await
        .context("querying authoritative membership")?;
    let mut events = Vec::new();
    let nip42 = Nip42Result::NotRequested;
    for _ in 0..MAX_RELAY_FRAMES {
        let Some(message) = socket.next().await else {
            break;
        };
        let Message::Text(text) = message.context("reading membership response")? else {
            continue;
        };
        ensure!(
            text.len() <= MAX_RELAY_FRAME_BYTES,
            "membership response exceeds its cap"
        );
        let frame: Value = serde_json::from_str(&text).context("decoding membership response")?;
        let Some(frame) = frame.as_array() else {
            continue;
        };
        if frame.first().and_then(Value::as_str) == Some("AUTH") {
            if let Some(challenge) = frame.get(1).and_then(Value::as_str) {
                return Ok(MembershipQueryReceipt {
                    coordinate: coordinate.clone(),
                    account_public_key_hex: account_public_key_hex.to_string(),
                    result: MembershipQueryResult::Failed,
                    nip42: Nip42Result::RequiredButUnavailable {
                        challenge: bounded_public_text(challenge),
                    },
                });
            }
            return Err(anyhow!("relay returned malformed NIP-42 challenge"));
        }
        if frame.first().and_then(Value::as_str) == Some("EOSE")
            && frame.get(1).and_then(Value::as_str) == Some(subscription_id)
        {
            let projection = fold_membership_events(
                coordinate,
                relay_public_key_hex,
                account_public_key_hex,
                events,
            )?;
            return Ok(MembershipQueryReceipt {
                coordinate: coordinate.clone(),
                account_public_key_hex: account_public_key_hex.to_string(),
                result: MembershipQueryResult::Complete(projection),
                nip42,
            });
        }
        if frame.first().and_then(Value::as_str) == Some("EVENT")
            && frame.get(1).and_then(Value::as_str) == Some(subscription_id)
            && let Some(event) = frame.get(2)
        {
            ensure!(
                events.len() < MAX_MEMBERSHIP_EVENTS,
                "membership response exceeds its event cap"
            );
            events.push(Event::from_json(event.to_string())?);
        }
    }
    Ok(MembershipQueryReceipt {
        coordinate: coordinate.clone(),
        account_public_key_hex: account_public_key_hex.to_string(),
        result: MembershipQueryResult::TimedOut,
        nip42,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ParsedRelayResponse {
    Unrelated,
    Auth(String),
    Accepted,
    Rejected(String),
}

fn parse_relay_response(expected_event_id: &str, text: &str) -> Result<ParsedRelayResponse> {
    let frame: Value = serde_json::from_str(text).context("decoding relay response")?;
    let Some(frame) = frame.as_array() else {
        return Ok(ParsedRelayResponse::Unrelated);
    };
    if frame.first().and_then(Value::as_str) == Some("AUTH") {
        let challenge = frame
            .get(1)
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("relay returned malformed NIP-42 challenge"))?;
        return Ok(ParsedRelayResponse::Auth(bounded_public_text(challenge)));
    }
    if frame.first().and_then(Value::as_str) != Some("OK")
        || frame.get(1).and_then(Value::as_str) != Some(expected_event_id)
    {
        return Ok(ParsedRelayResponse::Unrelated);
    }
    ensure!(
        frame.len() == 4
            && frame.get(2).and_then(Value::as_bool).is_some()
            && frame.get(3).and_then(Value::as_str).is_some(),
        "relay returned malformed OK response"
    );
    if frame.get(2).and_then(Value::as_bool) == Some(true) {
        Ok(ParsedRelayResponse::Accepted)
    } else {
        Ok(ParsedRelayResponse::Rejected(bounded_public_text(
            frame.get(3).and_then(Value::as_str).unwrap_or_default(),
        )))
    }
}

fn bounded_public_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .scan(0_usize, |used, character| {
            let width = character.len_utf8();
            if used.saturating_add(width) > MAX_RELAY_REASON_BYTES {
                None
            } else {
                *used += width;
                Some(character)
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalClaimLayer {
    Disabled,
    Pending { claim_ref: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArmadaJoinLayer {
    Disabled,
    UnsupportedOpaque { sha256: String, byte_length: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteroperableJoinLayers {
    pub buzz: ExternalClaimLayer,
    pub openagents: ExternalClaimLayer,
    pub armada: ArmadaJoinLayer,
}

impl Default for InteroperableJoinLayers {
    fn default() -> Self {
        Self {
            buzz: ExternalClaimLayer::Disabled,
            openagents: ExternalClaimLayer::Disabled,
            armada: ArmadaJoinLayer::Disabled,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupJoinReceipt {
    pub coordinate: GroupCoordinate,
    pub relay_public_key_hex: String,
    pub join_event_id: String,
    pub publication: RelayPublicationReceipt,
    pub membership: MembershipQueryReceipt,
    pub external: InteroperableJoinLayers,
}

pub struct ReceiptBackedJoinMutationExecutor {
    receipt: GroupJoinReceipt,
}

impl ReceiptBackedJoinMutationExecutor {
    pub fn new(receipt: GroupJoinReceipt) -> Self {
        Self { receipt }
    }

    fn validate_request(&self, mutation: &PreparedJoinMutation) -> Result<(), JoinStoreError> {
        let request: Nip29JoinMutationRequest = serde_json::from_slice(mutation.exact_request())
            .map_err(|_| JoinStoreError::InvalidPlan)?;
        let coordinate = request
            .coordinate()
            .map_err(|_| JoinStoreError::InvalidPlan)?;
        if coordinate != self.receipt.coordinate
            || request.relay_public_key_hex != self.receipt.relay_public_key_hex
            || mutation.account.account_public_key_hex
                != self.receipt.membership.account_public_key_hex
        {
            return Err(JoinStoreError::StaleGeneration);
        }
        Ok(())
    }
}

fn mutation_receipt_ref(event_id: &str, suffix: &str) -> String {
    let bounded_id = event_id.get(..32).unwrap_or(event_id);
    format!("omega.join.{suffix}.{bounded_id}")
}

fn nip42_mutation_outcome(nip42: &Nip42Result, event_id: &str) -> JoinMutationOutcome {
    match nip42 {
        Nip42Result::Authenticated(_) => JoinMutationOutcome::Succeeded {
            receipt_ref: mutation_receipt_ref(event_id, "nip42"),
        },
        Nip42Result::NotRequested => JoinMutationOutcome::Skipped {
            receipt_ref: mutation_receipt_ref(event_id, "nip42-not-requested"),
        },
        Nip42Result::RequiredButUnavailable { .. } | Nip42Result::Refused => {
            JoinMutationOutcome::FailedTerminal {
                failure_code: "nip42-unavailable".to_string(),
            }
        }
        Nip42Result::TimedOut => JoinMutationOutcome::FailedRetryable {
            failure_code: "nip42-timeout".to_string(),
        },
    }
}

fn request_mutation_outcome(
    publication: &RelayPublicationResult,
    event_id: &str,
) -> JoinMutationOutcome {
    match publication {
        RelayPublicationResult::Accepted => JoinMutationOutcome::Succeeded {
            receipt_ref: mutation_receipt_ref(event_id, "request"),
        },
        RelayPublicationResult::Rejected { .. } => JoinMutationOutcome::FailedTerminal {
            failure_code: "join-rejected".to_string(),
        },
        RelayPublicationResult::TimedOut | RelayPublicationResult::Failed => {
            JoinMutationOutcome::FailedRetryable {
                failure_code: "join-publication-unavailable".to_string(),
            }
        }
    }
}

fn admission_mutation_outcome(
    membership: &MembershipQueryResult,
    event_id: &str,
) -> JoinMutationOutcome {
    match membership {
        MembershipQueryResult::Complete(projection) => match projection.state {
            MembershipState::Member => JoinMutationOutcome::Succeeded {
                receipt_ref: mutation_receipt_ref(event_id, "membership"),
            },
            MembershipState::Removed => JoinMutationOutcome::FailedTerminal {
                failure_code: "membership-removed".to_string(),
            },
            MembershipState::Unknown => JoinMutationOutcome::FailedRetryable {
                failure_code: "membership-pending".to_string(),
            },
        },
        MembershipQueryResult::TimedOut | MembershipQueryResult::Failed => {
            JoinMutationOutcome::FailedRetryable {
                failure_code: "membership-unavailable".to_string(),
            }
        }
    }
}

impl JoinMutationExecutor for ReceiptBackedJoinMutationExecutor {
    fn execute(
        &mut self,
        mutation: &PreparedJoinMutation,
    ) -> Result<JoinMutationOutcome, JoinStoreError> {
        self.validate_request(mutation)?;
        let outcome = match mutation.kind {
            JoinStepKind::AddRelay => JoinMutationOutcome::Skipped {
                receipt_ref: "omega.join.relay-not-persisted".to_string(),
            },
            JoinStepKind::Nip42Authenticate => {
                nip42_mutation_outcome(&self.receipt.publication.nip42, &self.receipt.join_event_id)
            }
            JoinStepKind::RequestNip29Join => request_mutation_outcome(
                &self.receipt.publication.result,
                &self.receipt.join_event_id,
            ),
            JoinStepKind::AwaitNip29Admission => admission_mutation_outcome(
                &self.receipt.membership.result,
                &self.receipt.join_event_id,
            ),
            JoinStepKind::UpdateNip29GroupList => JoinMutationOutcome::FailedRetryable {
                failure_code: "group-list-publication-pending".to_string(),
            },
            JoinStepKind::ClaimBuzzInvite
            | JoinStepKind::VerifyOpenAgentsGrant
            | JoinStepKind::PersistLocalClaim => JoinMutationOutcome::FailedTerminal {
                failure_code: "external-authority-not-verified".to_string(),
            },
        };
        Ok(outcome)
    }
}

pub async fn publish_and_query_group_join(
    transport: &dyn AuthoritativeGroupRelayTransport,
    request: Nip29JoinMutationRequest,
    verified_metadata: &VerifiedRelayGroupMetadata,
    account_public_key_hex: String,
    join_event: Event,
    external: InteroperableJoinLayers,
) -> Result<GroupJoinReceipt> {
    ensure!(
        verified_metadata.request == request,
        "verified NIP-29 metadata is for another relay-qualified group"
    );
    let coordinate = request.coordinate()?;
    let join_event_id = join_event.id.to_hex();
    let publication = transport
        .publish_join_request(coordinate.clone(), join_event)
        .await;
    let membership = if matches!(
        publication.nip42,
        Nip42Result::RequiredButUnavailable { .. } | Nip42Result::Refused
    ) {
        MembershipQueryReceipt {
            coordinate: coordinate.clone(),
            account_public_key_hex,
            result: MembershipQueryResult::Failed,
            nip42: publication.nip42.clone(),
        }
    } else {
        transport
            .query_membership(
                coordinate.clone(),
                request.relay_public_key_hex.clone(),
                account_public_key_hex,
            )
            .await
    };
    Ok(GroupJoinReceipt {
        coordinate,
        relay_public_key_hex: request.relay_public_key_hex,
        join_event_id,
        publication,
        membership,
        external,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_nip29_join_transaction(
    store: &JoinTransactionStore,
    transaction_ref: &str,
    account: &JoinAccountFence,
    invite: &Nip29GroupInvite,
    metadata_event: &Event,
    current_group_list: Option<&Event>,
    broker: &SignerBroker,
    route: &SignerRoute,
    selection: AccountSelectionToken,
    transport: &dyn AuthoritativeGroupRelayTransport,
    now: u64,
) -> Result<JoinTransactionProjection> {
    ensure!(
        selection.account_ref.as_str() == account.account_ref
            && selection.identity.public_key_hex().as_str() == account.account_public_key_hex
            && selection.generation == account.generation,
        "join transaction belongs to another account generation"
    );
    let plan_created_at = store
        .inspect_optional(transaction_ref, account)
        .context("inspecting durable NIP-29 join transaction")?
        .map(|transaction| transaction.created_at)
        .unwrap_or(now);
    let plan =
        nip29_join_plan(invite, plan_created_at).context("building durable NIP-29 join plan")?;
    let mut transaction = store
        .create(transaction_ref, account.clone(), plan, now)
        .context("creating or resuming NIP-29 join transaction")?;
    if transaction.complete {
        return Ok(transaction);
    }
    if transaction
        .steps
        .iter()
        .any(|step| step.required && matches!(step.status, JoinStepStatus::FailedTerminal))
    {
        return Ok(transaction);
    }

    if step_can_execute(&transaction, JoinStepKind::AddRelay) {
        let relay_mutation = store
            .prepare_step(transaction_ref, account, JoinStepKind::AddRelay, now)
            .context("preparing relay-list mutation")?;
        store
            .record_outcome(
                &relay_mutation,
                JoinMutationOutcome::Skipped {
                    receipt_ref: "omega.join.relay-not-persisted".to_string(),
                },
                now,
            )
            .context("recording pending relay-list mutation")?;
    }

    let request_material = store
        .read_step_private_material(transaction_ref, account, JoinStepKind::RequestNip29Join)
        .context("reading exact NIP-29 join request")?;
    let stored_request: Nip29JoinMutationRequest =
        serde_json::from_slice(request_material.exact_request())
            .context("decoding exact NIP-29 join request")?;
    let expected_request = Nip29JoinMutationRequest::from_invite(invite, stored_request.created_at)
        .context("reading NIP-29 invite request")?;
    ensure!(
        stored_request == expected_request,
        "durable NIP-29 join request changed"
    );
    let metadata = verify_relay_group_metadata(&stored_request, metadata_event)?;
    let coordinate = stored_request.coordinate()?;
    let invite_code = stored_request
        .invite_code()
        .map(|code| std::str::from_utf8(code).context("decoding NIP-29 invite code"))
        .transpose()?;
    let expected_template =
        group_join_template(&coordinate, invite_code, stored_request.created_at)
            .map_err(|error| anyhow!(error))?;
    let request_mutation = if step_can_execute(&transaction, JoinStepKind::RequestNip29Join) {
        Some(
            store
                .prepare_step(
                    transaction_ref,
                    account,
                    JoinStepKind::RequestNip29Join,
                    now,
                )
                .context("preparing exact NIP-29 join request")?,
        )
    } else {
        ensure!(
            matches!(request_material.status, JoinStepStatus::Succeeded),
            "NIP-29 join request is not resumable"
        );
        None
    };
    let bound_event = match &request_mutation {
        Some(mutation) => store
            .read_bound_signed_event(mutation)
            .context("reading crash-safe signed join event")?,
        None => request_material.signed_event().cloned(),
    };
    let join_event = if let Some(binding) = bound_event {
        let event = Event::from_json(
            String::from_utf8(binding.exact_event_json().to_vec())
                .context("decoding crash-safe signed join event")?,
        )
        .context("parsing crash-safe signed join event")?;
        let result = SigningResult {
            request_ref: ReceiptRef::new(request_material.idempotency_key.clone())
                .context("building resumed NIP-29 signing receipt")?,
            identity: selection.identity.clone(),
            event_id: event.id.to_hex(),
            signature: event.sig.to_string(),
            signed_event_json: event.as_json(),
        };
        verify_signed_join_request(&selection, &expected_template, &result)
            .map_err(|error| anyhow!(error))?
    } else {
        let request_mutation = request_mutation
            .as_ref()
            .ok_or_else(|| anyhow!("completed NIP-29 request has no bound signed event"))?;
        let event = sign_group_join_request(
            broker,
            route,
            selection.clone(),
            &coordinate,
            invite_code,
            stored_request.created_at,
            ReceiptRef::new(request_mutation.idempotency_key.clone())
                .context("building NIP-29 signing receipt")?,
        )
        .await
        .map_err(|error| anyhow!(error))?;
        store
            .bind_signed_event(
                &request_mutation,
                &event.id.to_hex(),
                event.as_json().into_bytes(),
                now,
            )
            .context("persisting exact signed join event before publication")?;
        event
    };
    let join_event_id = join_event.id.to_hex();
    let (publication, membership) = if request_mutation.is_some() {
        let receipt = publish_and_query_group_join(
            transport,
            stored_request.clone(),
            &metadata,
            selection.identity.public_key_hex().as_str().to_string(),
            join_event,
            InteroperableJoinLayers::default(),
        )
        .await?;
        (Some(receipt.publication), receipt.membership)
    } else {
        let membership = transport
            .query_membership(
                coordinate.clone(),
                stored_request.relay_public_key_hex.clone(),
                selection.identity.public_key_hex().as_str().to_string(),
            )
            .await;
        (None, membership)
    };

    if step_can_execute(&transaction, JoinStepKind::Nip42Authenticate) {
        let nip42_mutation = store
            .prepare_step(
                transaction_ref,
                account,
                JoinStepKind::Nip42Authenticate,
                now,
            )
            .context("preparing NIP-42 join step")?;
        let nip42 = publication
            .as_ref()
            .map(|receipt| &receipt.nip42)
            .unwrap_or(&membership.nip42);
        let nip42_outcome = nip42_mutation_outcome(nip42, &join_event_id);
        store
            .record_outcome(&nip42_mutation, nip42_outcome, now)
            .context("recording NIP-42 join outcome")?;
    }

    if let Some(request_mutation) = &request_mutation {
        let publication = publication
            .as_ref()
            .ok_or_else(|| anyhow!("join publication receipt is missing"))?;
        let request_outcome = request_mutation_outcome(&publication.result, &join_event_id);
        transaction = store
            .record_outcome(request_mutation, request_outcome, now)
            .context("recording join publication outcome")?;
        if transaction
            .steps
            .iter()
            .any(|step| step.required && matches!(step.status, JoinStepStatus::FailedTerminal))
        {
            return Ok(transaction);
        }
    }

    if step_can_execute(&transaction, JoinStepKind::AwaitNip29Admission) {
        let admission_mutation = store
            .prepare_step(
                transaction_ref,
                account,
                JoinStepKind::AwaitNip29Admission,
                now,
            )
            .context("preparing authoritative membership check")?;
        let admission_outcome = admission_mutation_outcome(&membership.result, &join_event_id);
        transaction = store
            .record_outcome(&admission_mutation, admission_outcome, now)
            .context("recording authoritative membership outcome")?;
        if transaction
            .steps
            .iter()
            .any(|step| step.required && matches!(step.status, JoinStepStatus::FailedTerminal))
        {
            return Ok(transaction);
        }
    }

    if !step_can_execute(&transaction, JoinStepKind::UpdateNip29GroupList) {
        return store
            .inspect(transaction_ref, account)
            .context("inspecting resumed NIP-29 join transaction");
    }

    let group_list_mutation = store
        .prepare_step(
            transaction_ref,
            account,
            JoinStepKind::UpdateNip29GroupList,
            now,
        )
        .context("preparing kind-10009 update")?;
    let group_list_outcome = if matches!(
        membership.result,
        MembershipQueryResult::Complete(MembershipProjection {
            state: MembershipState::Member,
            ..
        })
    ) {
        let event = if let Some(binding) = store
            .read_bound_signed_event(&group_list_mutation)
            .context("reading crash-safe signed group-list event")?
        {
            let event = Event::from_json(
                String::from_utf8(binding.exact_event_json().to_vec())
                    .context("decoding crash-safe signed group-list event")?,
            )
            .context("parsing crash-safe signed group-list event")?;
            event
                .verify()
                .context("verifying crash-safe signed group-list event")?;
            ensure!(
                event.kind.as_u16() == GROUP_LIST_KIND
                    && event.created_at.as_secs() == stored_request.created_at
                    && event.pubkey.to_hex() == selection.identity.public_key_hex().as_str()
                    && event.tags.iter().any(|tag| {
                        group_coordinate_from_tag(tag.as_slice()).as_ref() == Some(&coordinate)
                    }),
                "crash-safe group-list event is not the persisted coordinate mutation"
            );
            event
        } else {
            let template = merge_relay_qualified_group_list(
                selection.identity.public_key_hex().as_str(),
                current_group_list,
                GroupListMutation::Add {
                    coordinate: coordinate.clone(),
                    label: None,
                },
                stored_request.created_at,
            )?;
            let event = sign_group_list_update(
                broker,
                route,
                selection.clone(),
                template,
                ReceiptRef::new(group_list_mutation.idempotency_key.clone())
                    .context("building kind-10009 signing receipt")?,
            )
            .await
            .map_err(|error| anyhow!(error))?;
            store
                .bind_signed_event(
                    &group_list_mutation,
                    &event.id.to_hex(),
                    event.as_json().into_bytes(),
                    now,
                )
                .context("persisting exact signed group-list event before publication")?;
            event
        };
        let publication = transport.publish_group_list_update(coordinate, event).await;
        match publication.result {
            RelayPublicationResult::Accepted => JoinMutationOutcome::Succeeded {
                receipt_ref: format!(
                    "omega.join.group-list.{}",
                    publication
                        .event_id
                        .get(..32)
                        .unwrap_or(publication.event_id.as_str())
                ),
            },
            RelayPublicationResult::Rejected { .. } => JoinMutationOutcome::FailedTerminal {
                failure_code: "group-list-rejected".to_string(),
            },
            RelayPublicationResult::TimedOut | RelayPublicationResult::Failed => {
                JoinMutationOutcome::FailedRetryable {
                    failure_code: "group-list-publication-unavailable".to_string(),
                }
            }
        }
    } else {
        JoinMutationOutcome::FailedRetryable {
            failure_code: "membership-pending".to_string(),
        }
    };
    store
        .record_outcome(&group_list_mutation, group_list_outcome, now)
        .context("recording kind-10009 update outcome")
}

fn step_can_execute(transaction: &JoinTransactionProjection, kind: JoinStepKind) -> bool {
    transaction.steps.iter().any(|step| {
        step.kind == kind
            && matches!(
                step.status,
                JoinStepStatus::Pending
                    | JoinStepStatus::Prepared
                    | JoinStepStatus::FailedRetryable
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{
        EventBuilder, Keys, Kind, RelayUrl, Tag, Timestamp, ToBech32 as _,
        nips::{nip01::Coordinate, nip19::Nip19Coordinate},
    };
    use omega_identity::{AccountRef, IdentityRef, IdentityService, PublicIdentity};
    use omega_invites::{InviteResolver, ParsedInvite};
    use omega_signer_broker::{SelectionValidator, SignerBrokerError};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    fn selection(keys: &Keys) -> AccountSelectionToken {
        AccountSelectionToken {
            account_ref: AccountRef::new("join-account").expect("account ref"),
            identity: PublicIdentity::from_public_key_hex(
                IdentityRef::new("join-identity").expect("identity ref"),
                keys.public_key().to_hex(),
            )
            .expect("identity"),
            generation: 1,
        }
    }

    fn event(keys: &Keys, kind: u16, created_at: u64, tags: Vec<Tag>) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(created_at))
            .sign_with_keys(keys)
            .expect("event")
    }

    #[test]
    fn join_template_and_signed_result_are_exact() {
        let keys = Keys::generate();
        let selection = selection(&keys);
        let coordinate =
            GroupCoordinate::new("wss://relay.example.com", "group").expect("coordinate");
        let template = group_join_template(&coordinate, Some("invite-code"), 42).expect("template");
        assert_eq!(template.kind, GROUP_JOIN_REQUEST_KIND);
        assert_eq!(
            template.tags,
            vec![
                vec!["h".to_string(), "group".to_string()],
                vec!["code".to_string(), "invite-code".to_string()],
            ]
        );
        let signed = event(
            &keys,
            GROUP_JOIN_REQUEST_KIND,
            42,
            vec![
                Tag::parse(["h", "group"]).expect("h tag"),
                Tag::parse(["code", "invite-code"]).expect("code tag"),
            ],
        );
        let result = SigningResult {
            request_ref: ReceiptRef::new("join-request").expect("receipt"),
            identity: selection.identity.clone(),
            event_id: signed.id.to_hex(),
            signature: signed.sig.to_string(),
            signed_event_json: signed.as_json(),
        };
        verify_signed_join_request(&selection, &template, &result).expect("exact event");
        let wrong = UnsignedEventTemplate {
            content: "different".to_string(),
            ..template
        };
        assert!(verify_signed_join_request(&selection, &wrong, &result).is_err());
    }

    #[test]
    fn group_list_merge_is_relay_qualified_and_preserves_unknown_material() {
        let keys = Keys::generate();
        let author = keys.public_key().to_hex();
        let current = EventBuilder::new(Kind::from(GROUP_LIST_KIND), "opaque-private-content")
            .tags(vec![
                Tag::parse(["group", "same", "wss://one.example.com", "One", "opaque"])
                    .expect("first group"),
                Tag::parse(["group", "same", "wss://two.example.com", "Two"])
                    .expect("second group"),
                Tag::parse(["unknown", "preserve", "all", "fields"]).expect("unknown"),
            ])
            .custom_created_at(Timestamp::from_secs(1))
            .sign_with_keys(&keys)
            .expect("current");
        let merged = merge_relay_qualified_group_list(
            &author,
            Some(&current),
            GroupListMutation::Remove(
                GroupCoordinate::new("wss://one.example.com", "same").expect("coordinate"),
            ),
            2,
        )
        .expect("merged");
        assert_eq!(merged.content, "opaque-private-content");
        assert!(merged.tags.contains(&vec![
            "unknown".into(),
            "preserve".into(),
            "all".into(),
            "fields".into()
        ]));
        assert!(merged.tags.iter().any(|tag| {
            tag.get(1).map(String::as_str) == Some("same")
                && tag.get(2).map(String::as_str) == Some("wss://two.example.com")
        }));
        assert!(
            !merged
                .tags
                .iter()
                .any(|tag| { tag.get(2).map(String::as_str) == Some("wss://one.example.com") })
        );
        let signed_merged = EventBuilder::new(Kind::from(GROUP_LIST_KIND), merged.content.clone())
            .tags(
                merged
                    .tags
                    .iter()
                    .cloned()
                    .map(Tag::parse)
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .expect("merged tags"),
            )
            .custom_created_at(Timestamp::from_secs(merged.created_at))
            .sign_with_keys(&keys)
            .expect("signed merged list");
        let selection = selection(&keys);
        let result = SigningResult {
            request_ref: ReceiptRef::new("group-list-update").expect("receipt"),
            identity: selection.identity.clone(),
            event_id: signed_merged.id.to_hex(),
            signature: signed_merged.sig.to_string(),
            signed_event_json: signed_merged.as_json(),
        };
        verify_exact_signed_template(&selection, &merged, &result)
            .expect("exact signed group-list update");
    }

    #[test]
    fn latest_9000_or_9001_is_membership_truth_not_publication_ok() {
        let relay = Keys::generate();
        let member = Keys::generate();
        let member_public_key = member.public_key().to_hex();
        let coordinate =
            GroupCoordinate::new("wss://relay.example.com", "group").expect("coordinate");
        let added = event(
            &relay,
            GROUP_MEMBER_ADDED_KIND,
            10,
            vec![
                Tag::parse(["h", "group"]).expect("h"),
                Tag::parse(["p", member_public_key.as_str()]).expect("p"),
            ],
        );
        let removed = event(
            &relay,
            GROUP_MEMBER_REMOVED_KIND,
            11,
            vec![
                Tag::parse(["h", "group"]).expect("h"),
                Tag::parse(["p", member_public_key.as_str()]).expect("p"),
            ],
        );
        let projection = fold_membership_events(
            &coordinate,
            &relay.public_key().to_hex(),
            &member_public_key,
            [added, removed],
        )
        .expect("membership");
        assert_eq!(projection.state, MembershipState::Removed);
        assert_eq!(
            parse_relay_response(
                &"a".repeat(64),
                &json!(["OK", "a".repeat(64), true, "accepted"]).to_string(),
            )
            .expect("OK"),
            ParsedRelayResponse::Accepted
        );
        assert_ne!(projection.state, MembershipState::Member);
    }

    #[test]
    fn nip42_and_external_claims_never_fake_membership() {
        assert_eq!(
            parse_relay_response("event", r#"["AUTH","challenge\nsecret"]"#).expect("challenge"),
            ParsedRelayResponse::Auth("challengesecret".to_string())
        );
        let layers = InteroperableJoinLayers {
            buzz: ExternalClaimLayer::Pending {
                claim_ref: "buzz-claim".to_string(),
            },
            openagents: ExternalClaimLayer::Disabled,
            armada: ArmadaJoinLayer::UnsupportedOpaque {
                sha256: "a".repeat(64),
                byte_length: 128,
            },
        };
        assert!(matches!(layers.buzz, ExternalClaimLayer::Pending { .. }));
        assert!(matches!(
            layers.armada,
            ArmadaJoinLayer::UnsupportedOpaque { .. }
        ));
    }

    struct AcceptedButPendingTransport;

    impl AuthoritativeGroupRelayTransport for AcceptedButPendingTransport {
        fn publish_join_request(
            &self,
            coordinate: GroupCoordinate,
            event: Event,
        ) -> Pin<Box<dyn Future<Output = RelayPublicationReceipt> + Send + 'static>> {
            Box::pin(async move {
                RelayPublicationReceipt {
                    relay_url: coordinate.relay_url,
                    event_id: event.id.to_hex(),
                    result: RelayPublicationResult::Accepted,
                    nip42: Nip42Result::NotRequested,
                }
            })
        }

        fn publish_group_list_update(
            &self,
            coordinate: GroupCoordinate,
            event: Event,
        ) -> Pin<Box<dyn Future<Output = RelayPublicationReceipt> + Send + 'static>> {
            Box::pin(async move {
                RelayPublicationReceipt {
                    relay_url: coordinate.relay_url,
                    event_id: event.id.to_hex(),
                    result: RelayPublicationResult::Failed,
                    nip42: Nip42Result::NotRequested,
                }
            })
        }

        fn query_membership(
            &self,
            coordinate: GroupCoordinate,
            _relay_public_key_hex: String,
            account_public_key_hex: String,
        ) -> Pin<Box<dyn Future<Output = MembershipQueryReceipt> + Send + 'static>> {
            Box::pin(async move {
                MembershipQueryReceipt {
                    coordinate,
                    account_public_key_hex,
                    result: MembershipQueryResult::Complete(MembershipProjection::default()),
                    nip42: Nip42Result::NotRequested,
                }
            })
        }
    }

    struct AllowSelection;

    impl SelectionValidator for AllowSelection {
        fn validate(
            &self,
            _token: &AccountSelectionToken,
        ) -> std::result::Result<(), SignerBrokerError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct ResumeCountingTransport {
        join_publications: AtomicUsize,
        group_list_publications: AtomicUsize,
        membership_queries: AtomicUsize,
        member: AtomicBool,
        published_group_list_event_id: Mutex<Option<String>>,
    }

    impl AuthoritativeGroupRelayTransport for ResumeCountingTransport {
        fn publish_join_request(
            &self,
            coordinate: GroupCoordinate,
            event: Event,
        ) -> Pin<Box<dyn Future<Output = RelayPublicationReceipt> + Send + 'static>> {
            self.join_publications.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                RelayPublicationReceipt {
                    relay_url: coordinate.relay_url,
                    event_id: event.id.to_hex(),
                    result: RelayPublicationResult::Accepted,
                    nip42: Nip42Result::NotRequested,
                }
            })
        }

        fn publish_group_list_update(
            &self,
            coordinate: GroupCoordinate,
            event: Event,
        ) -> Pin<Box<dyn Future<Output = RelayPublicationReceipt> + Send + 'static>> {
            self.group_list_publications.fetch_add(1, Ordering::SeqCst);
            *self
                .published_group_list_event_id
                .lock()
                .expect("group-list event lock") = Some(event.id.to_hex());
            Box::pin(async move {
                RelayPublicationReceipt {
                    relay_url: coordinate.relay_url,
                    event_id: event.id.to_hex(),
                    result: RelayPublicationResult::Accepted,
                    nip42: Nip42Result::NotRequested,
                }
            })
        }

        fn query_membership(
            &self,
            coordinate: GroupCoordinate,
            _relay_public_key_hex: String,
            account_public_key_hex: String,
        ) -> Pin<Box<dyn Future<Output = MembershipQueryReceipt> + Send + 'static>> {
            self.membership_queries.fetch_add(1, Ordering::SeqCst);
            let state = if self.member.load(Ordering::SeqCst) {
                MembershipState::Member
            } else {
                MembershipState::Unknown
            };
            Box::pin(async move {
                MembershipQueryReceipt {
                    coordinate,
                    account_public_key_hex,
                    result: MembershipQueryResult::Complete(MembershipProjection {
                        state,
                        event_id: None,
                        created_at: None,
                    }),
                    nip42: Nip42Result::NotRequested,
                }
            })
        }
    }

    #[test]
    fn succeeded_join_request_resumes_without_republishing_or_changing_timestamp() {
        smol::block_on(async {
            let directory = tempfile::tempdir().expect("temporary directory");
            let store = JoinTransactionStore::for_data_root(directory.path());
            let member = Keys::generate();
            let relay = Keys::generate();
            let coordinate = Coordinate {
                kind: Kind::from(39_000),
                public_key: relay.public_key(),
                identifier: "group".to_string(),
            };
            let address = Nip19Coordinate::new(
                coordinate,
                [RelayUrl::parse("wss://relay.example.com").expect("relay URL")],
            )
            .to_bech32()
            .expect("naddr");
            let resolved = InviteResolver.resolve(&address).expect("invite");
            let ParsedInvite::Nip29Group(invite) = resolved.parsed else {
                panic!("expected NIP-29 invite");
            };
            let selection = selection(&member);
            let account = JoinAccountFence::new(
                selection.account_ref.as_str(),
                member.public_key().to_hex(),
                selection.generation,
            )
            .expect("account fence");
            store
                .create(
                    "resume-join",
                    account.clone(),
                    nip29_join_plan(&invite, 10).expect("plan"),
                    10,
                )
                .expect("transaction");
            let request = store
                .prepare_step("resume-join", &account, JoinStepKind::RequestNip29Join, 10)
                .expect("prepared request");
            let join_event = event(
                &member,
                GROUP_JOIN_REQUEST_KIND,
                10,
                vec![Tag::parse(["h", "group"]).expect("h")],
            );
            store
                .bind_signed_event(
                    &request,
                    &join_event.id.to_hex(),
                    join_event.as_json().into_bytes(),
                    10,
                )
                .expect("bound request");
            store
                .record_outcome(
                    &request,
                    JoinMutationOutcome::Succeeded {
                        receipt_ref: "omega.join.request.persisted".to_string(),
                    },
                    10,
                )
                .expect("request succeeded");
            let metadata = event(
                &relay,
                39_000,
                9,
                vec![Tag::parse(["d", "group"]).expect("d")],
            );
            let broker = SignerBroker::with_validator(Arc::new(AllowSelection));
            let route = SignerRoute::Local {
                identity_service: Arc::new(IdentityService::for_channel_data_root(
                    app_identity::AppChannel::Dev,
                    directory.path().join("signer"),
                )),
            };
            let transport = ResumeCountingTransport::default();
            let projection = execute_nip29_join_transaction(
                &store,
                "resume-join",
                &account,
                &invite,
                &metadata,
                None,
                &broker,
                &route,
                selection.clone(),
                &transport,
                99,
            )
            .await
            .expect("resume transaction");
            assert_eq!(transport.join_publications.load(Ordering::SeqCst), 0);
            assert_eq!(transport.membership_queries.load(Ordering::SeqCst), 1);
            assert_eq!(projection.created_at, 10);
            assert_eq!(
                projection
                    .steps
                    .iter()
                    .find(|step| step.kind == JoinStepKind::RequestNip29Join)
                    .expect("request step")
                    .status,
                JoinStepStatus::Succeeded
            );
            assert_eq!(
                projection
                    .steps
                    .iter()
                    .find(|step| step.kind == JoinStepKind::AwaitNip29Admission)
                    .expect("admission step")
                    .status,
                JoinStepStatus::FailedRetryable
            );
            assert_eq!(
                projection
                    .steps
                    .iter()
                    .find(|step| step.kind == JoinStepKind::UpdateNip29GroupList)
                    .expect("group-list step")
                    .status,
                JoinStepStatus::FailedRetryable
            );
            let material = store
                .read_step_private_material("resume-join", &account, JoinStepKind::RequestNip29Join)
                .expect("private request material");
            let exact_request: Nip29JoinMutationRequest =
                serde_json::from_slice(material.exact_request()).expect("exact request");
            assert_eq!(exact_request.created_at, 10);
            assert_eq!(
                material
                    .signed_event()
                    .expect("signed event")
                    .exact_event_json(),
                join_event.as_json().as_bytes()
            );

            let group_list_mutation = store
                .prepare_step(
                    "resume-join",
                    &account,
                    JoinStepKind::UpdateNip29GroupList,
                    99,
                )
                .expect("prepared group-list retry");
            let bound_group_list =
                EventBuilder::new(Kind::from(GROUP_LIST_KIND), "persisted-before-crash")
                    .tags(vec![
                        Tag::parse([
                            "group",
                            "group",
                            invite.relay_url.as_str(),
                            "Persisted label",
                        ])
                        .expect("group tag"),
                        Tag::parse(["unknown", "persisted"]).expect("unknown tag"),
                    ])
                    .custom_created_at(Timestamp::from_secs(10))
                    .sign_with_keys(&member)
                    .expect("bound group-list event");
            store
                .bind_signed_event(
                    &group_list_mutation,
                    &bound_group_list.id.to_hex(),
                    bound_group_list.as_json().into_bytes(),
                    99,
                )
                .expect("bound group-list event");
            store
                .record_outcome(
                    &group_list_mutation,
                    JoinMutationOutcome::FailedRetryable {
                        failure_code: "group-list-publication-unavailable".to_string(),
                    },
                    99,
                )
                .expect("group-list pending");
            let changed_current_group_list =
                EventBuilder::new(Kind::from(GROUP_LIST_KIND), "changed-after-crash")
                    .tags(vec![
                        Tag::parse(["group", "other", "wss://other.example.com"])
                            .expect("changed group tag"),
                    ])
                    .custom_created_at(Timestamp::from_secs(98))
                    .sign_with_keys(&member)
                    .expect("changed current group list");
            transport.member.store(true, Ordering::SeqCst);
            let resumed_projection = execute_nip29_join_transaction(
                &store,
                "resume-join",
                &account,
                &invite,
                &metadata,
                Some(&changed_current_group_list),
                &broker,
                &route,
                selection,
                &transport,
                100,
            )
            .await
            .expect("resume bound group-list transaction");
            assert_eq!(transport.join_publications.load(Ordering::SeqCst), 0);
            assert_eq!(transport.group_list_publications.load(Ordering::SeqCst), 1);
            assert_eq!(
                transport
                    .published_group_list_event_id
                    .lock()
                    .expect("group-list event lock")
                    .as_deref(),
                Some(bound_group_list.id.to_hex().as_str())
            );
            assert!(resumed_projection.complete);
        });
    }

    #[test]
    fn accepted_event_ack_remains_pending_until_membership_event_exists() {
        smol::block_on(async {
            let member = Keys::generate();
            let relay = Keys::generate();
            let coordinate =
                GroupCoordinate::new("wss://relay.example.com", "group").expect("coordinate");
            let request = Nip29JoinMutationRequest {
                relay_url: coordinate.relay_url.clone(),
                relay_public_key_hex: relay.public_key().to_hex(),
                group_id: coordinate.group_id.clone(),
                created_at: 10,
                invite_code: None,
            };
            let metadata_event = event(
                &relay,
                39_000,
                9,
                vec![Tag::parse(["d", "group"]).expect("d")],
            );
            let metadata =
                verify_relay_group_metadata(&request, &metadata_event).expect("metadata");
            let join = event(
                &member,
                GROUP_JOIN_REQUEST_KIND,
                10,
                vec![Tag::parse(["h", "group"]).expect("h")],
            );
            let receipt = publish_and_query_group_join(
                &AcceptedButPendingTransport,
                request,
                &metadata,
                member.public_key().to_hex(),
                join,
                InteroperableJoinLayers::default(),
            )
            .await
            .expect("join receipt");
            assert_eq!(receipt.publication.result, RelayPublicationResult::Accepted);
            assert_eq!(
                receipt.membership.result,
                MembershipQueryResult::Complete(MembershipProjection::default())
            );
        });
    }

    #[test]
    fn relay_metadata_and_membership_reject_the_wrong_relay_author() {
        let expected_relay = Keys::generate();
        let wrong_relay = Keys::generate();
        let member = Keys::generate();
        let coordinate =
            GroupCoordinate::new("wss://relay.example.com", "group").expect("coordinate");
        let request = Nip29JoinMutationRequest {
            relay_url: coordinate.relay_url.clone(),
            relay_public_key_hex: expected_relay.public_key().to_hex(),
            group_id: coordinate.group_id.clone(),
            created_at: 2,
            invite_code: None,
        };
        let wrong_metadata = event(
            &wrong_relay,
            39_000,
            1,
            vec![Tag::parse(["d", "group"]).expect("d")],
        );
        assert!(verify_relay_group_metadata(&request, &wrong_metadata).is_err());
        let wrong_membership = event(
            &wrong_relay,
            GROUP_MEMBER_ADDED_KIND,
            2,
            vec![
                Tag::parse(["h", "group"]).expect("h"),
                Tag::parse(["p", member.public_key().to_hex().as_str()]).expect("p"),
            ],
        );
        assert!(
            fold_membership_events(
                &coordinate,
                &expected_relay.public_key().to_hex(),
                &member.public_key().to_hex(),
                [wrong_membership],
            )
            .is_err()
        );
    }
}
