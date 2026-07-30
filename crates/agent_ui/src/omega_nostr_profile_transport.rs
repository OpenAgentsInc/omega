use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::Path,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context as _, Result, anyhow, ensure};
use async_tungstenite::{async_std::connect_async, tungstenite::Message};
use futures::{FutureExt as _, StreamExt as _, pin_mut, select};
use nostr::{Event, JsonUtil as _, RelayUrl};
use omega_effectd::{BindingProjection, HostedSessionProjection};
use omega_identity::{
    AccountProfileSummary, AccountRegistryService, AccountSelectionToken, AdmittedSigningRequest,
    IdentityCandidateOrigin, Nip46OperationRequest, NostrPublicKeyHex, ReceiptRef, SigningPurpose,
    SigningResult, UnsignedEventTemplate,
};
use omega_identity_sync::{
    BulkDecryptConsentState, CacheFallbackReason, HydrationAccountFence, HydrationCache,
    HydrationCacheArea, HydrationError, HydrationPlan, HydrationReceipt, HydrationScheduler,
    HydrationSource, HydrationSourceOutcome, HydrationSourcePlan, HydrationSourceRequest,
    HydrationSourceRunner, HydrationTrigger,
};
use omega_signer_broker::{SignerBroker, SignerBrokerError, SignerRoute};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

pub const PROFILE_KINDS: &[u16] = &[0, 10_002, 10_009];
pub const RELAY_GROUP_STATE_KINDS: &[u16] = &[39_000, 39_001, 39_002, 39_003, 39_005];
pub const MEMBERSHIP_KINDS: &[u16] = &[9_000, 9_001];
pub const MAX_BOOTSTRAP_RELAYS: usize = 8;
pub const MAX_PROFILE_AUTHORS: usize = 256;
pub const MAX_GROUPS: usize = 64;
pub const MAX_EVENT_BYTES: usize = 64 * 1024;
pub const MAX_PROFILE_CONTENT_BYTES: usize = 16 * 1024;
pub const MAX_TAGS: usize = 256;
pub const MAX_RECENT_ROOM_EVENTS: usize = 200;
pub const MAX_BULK_DECRYPT_ITEMS: usize = 64;
const MAX_ACK_FRAMES: usize = 64;
const RELAY_TIMEOUT: Duration = Duration::from_secs(8);
const HYDRATION_SOURCE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RELAY_REASON_BYTES: usize = 256;
const SYSTEM_BOOTSTRAP_RELAYS: &[&str] = &[
    "wss://relay.openagents.com",
    "wss://relay.damus.io",
    "wss://nos.lol",
];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GroupCoordinate {
    pub relay_url: String,
    pub group_id: String,
}

impl GroupCoordinate {
    pub fn new(relay_url: &str, group_id: &str) -> Result<Self> {
        let relay_url = canonical_wss_relay(relay_url)?;
        let group_id = group_id.trim();
        ensure!(
            !group_id.is_empty()
                && group_id.len() <= 256
                && !group_id.chars().any(char::is_control),
            "the NIP-29 group id is invalid"
        );
        Ok(Self {
            relay_url,
            group_id: group_id.to_string(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayUse {
    Read,
    Write,
    ReadWrite,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayPreference {
    pub relay_url: String,
    pub usage: RelayUse,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpaqueVerifiedEvent {
    pub event_id: String,
    pub author_public_key_hex: String,
    pub created_at: u64,
    pub kind: u16,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub event_json: String,
}

impl OpaqueVerifiedEvent {
    fn from_event(event: &Event) -> Result<Self> {
        ensure!(
            event.as_json().len() <= MAX_EVENT_BYTES
                && event.content.len() <= MAX_PROFILE_CONTENT_BYTES
                && event.tags.len() <= MAX_TAGS,
            "the Nostr event exceeds the bounded projection limits"
        );
        event.verify().context("verifying the Nostr event")?;
        Ok(Self {
            event_id: event.id.to_hex(),
            author_public_key_hex: event.pubkey.to_hex(),
            created_at: event.created_at.as_secs(),
            kind: event.kind.as_u16(),
            tags: event
                .tags
                .iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect(),
            content: event.content.clone(),
            event_json: event.as_json(),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AuthorNostrProjection {
    pub profile: Option<Value>,
    pub relay_preferences: Vec<RelayPreference>,
    pub groups: Vec<GroupCoordinate>,
    pub has_opaque_private_groups: bool,
    pub latest_events: BTreeMap<u16, OpaqueVerifiedEvent>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NostrTransportCache {
    pub authors: BTreeMap<String, AuthorNostrProjection>,
    pub groups: BTreeMap<GroupCoordinate, GroupNostrProjection>,
}

impl NostrTransportCache {
    pub fn ingest_author(
        &mut self,
        expected_author: &str,
        events: impl IntoIterator<Item = Event>,
    ) -> Result<()> {
        if !self.authors.contains_key(expected_author) {
            ensure!(
                self.authors.len() < MAX_PROFILE_AUTHORS,
                "the profile-author cache exceeds its cap"
            );
        }
        self.authors
            .entry(expected_author.to_string())
            .or_default()
            .ingest(expected_author, events)
    }

    pub fn group_mut(&mut self, coordinate: GroupCoordinate) -> Result<&mut GroupNostrProjection> {
        if !self.groups.contains_key(&coordinate) {
            ensure!(
                self.groups.len() < MAX_GROUPS,
                "the NIP-29 coordinate cache exceeds its cap"
            );
        }
        Ok(self.groups.entry(coordinate).or_default())
    }
}

impl AuthorNostrProjection {
    pub fn ingest(
        &mut self,
        expected_author: &str,
        events: impl IntoIterator<Item = Event>,
    ) -> Result<()> {
        let mut admitted = 0_usize;
        for event in events {
            admitted = admitted.saturating_add(1);
            ensure!(admitted <= 1_024, "too many profile events in one batch");
            ensure!(
                event.pubkey.to_hex() == expected_author,
                "a profile event was authored by a different identity"
            );
            let kind = event.kind.as_u16();
            ensure!(
                PROFILE_KINDS.contains(&kind),
                "unsupported profile event kind"
            );
            let event = OpaqueVerifiedEvent::from_event(&event)?;
            let replace = self
                .latest_events
                .get(&kind)
                .is_none_or(|current| replaceable_order(&event) > replaceable_order(current));
            if replace {
                self.latest_events.insert(kind, event);
            }
        }
        self.reproject()
    }

    fn reproject(&mut self) -> Result<()> {
        self.profile = self
            .latest_events
            .get(&0)
            .map(|event| {
                serde_json::from_str::<Value>(&event.content)
                    .context("decoding kind-0 profile metadata")
                    .and_then(|value| {
                        ensure!(value.is_object(), "kind-0 metadata must be a JSON object");
                        Ok(value)
                    })
            })
            .transpose()?;

        self.relay_preferences = self
            .latest_events
            .get(&10_002)
            .map(project_relay_preferences)
            .transpose()?
            .unwrap_or_default();

        let (groups, has_private) = self
            .latest_events
            .get(&10_009)
            .map(project_groups)
            .transpose()?
            .unwrap_or_default();
        self.groups = groups;
        self.has_opaque_private_groups = has_private;
        Ok(())
    }
}

fn replaceable_order(event: &OpaqueVerifiedEvent) -> (u64, &str) {
    (event.created_at, event.event_id.as_str())
}

fn project_relay_preferences(event: &OpaqueVerifiedEvent) -> Result<Vec<RelayPreference>> {
    let mut relays = BTreeMap::new();
    for tag in &event.tags {
        if tag.first().map(String::as_str) != Some("r") {
            continue;
        }
        let Some(relay_url) = tag.get(1) else {
            continue;
        };
        let relay_url = canonical_wss_relay(relay_url)?;
        let usage = match tag.get(2).map(String::as_str) {
            None => RelayUse::ReadWrite,
            Some("read") => RelayUse::Read,
            Some("write") => RelayUse::Write,
            Some(_) => continue,
        };
        relays.insert(relay_url.clone(), RelayPreference { relay_url, usage });
        ensure!(
            relays.len() <= MAX_BOOTSTRAP_RELAYS,
            "the relay preference list exceeds its cap"
        );
    }
    Ok(relays.into_values().collect())
}

fn project_groups(event: &OpaqueVerifiedEvent) -> Result<(Vec<GroupCoordinate>, bool)> {
    let mut groups = BTreeSet::new();
    for tag in &event.tags {
        if tag.first().map(String::as_str) != Some("group") {
            continue;
        }
        let (Some(group_id), Some(relay_url)) = (tag.get(1), tag.get(2)) else {
            continue;
        };
        groups.insert(GroupCoordinate::new(relay_url, group_id)?);
        ensure!(groups.len() <= MAX_GROUPS, "the group list exceeds its cap");
    }
    Ok((groups.into_iter().collect(), !event.content.is_empty()))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MembershipState {
    #[default]
    Unknown,
    Member,
    Removed,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupNostrProjection {
    pub relay_state: BTreeMap<u16, OpaqueVerifiedEvent>,
    pub advisory_members: Option<OpaqueVerifiedEvent>,
    pub membership: MembershipState,
    pub membership_event: Option<OpaqueVerifiedEvent>,
    pub recent_room_events: Vec<OpaqueVerifiedEvent>,
}

impl GroupNostrProjection {
    pub fn ingest_relay_state(
        &mut self,
        coordinate: &GroupCoordinate,
        pinned_relay_public_key: &str,
        event: Event,
    ) -> Result<()> {
        let event = OpaqueVerifiedEvent::from_event(&event)?;
        ensure!(
            RELAY_GROUP_STATE_KINDS.contains(&event.kind),
            "unsupported relay group-state kind"
        );
        ensure!(
            event.author_public_key_hex == pinned_relay_public_key,
            "group state was not signed by the pinned relay self key"
        );
        ensure!(
            exact_tag(&event.tags, "d") == Some(coordinate.group_id.as_str()),
            "group state is not bound to the exact NIP-29 coordinate"
        );
        if event.kind == 39_002 {
            let replace = self
                .advisory_members
                .as_ref()
                .is_none_or(|current| replaceable_order(&event) > replaceable_order(current));
            if replace {
                self.advisory_members = Some(event);
            }
            return Ok(());
        }
        let replace = self
            .relay_state
            .get(&event.kind)
            .is_none_or(|current| replaceable_order(&event) > replaceable_order(current));
        if replace {
            self.relay_state.insert(event.kind, event);
        }
        Ok(())
    }

    pub fn ingest_membership(
        &mut self,
        coordinate: &GroupCoordinate,
        account_public_key_hex: &str,
        event: Event,
    ) -> Result<()> {
        let event = OpaqueVerifiedEvent::from_event(&event)?;
        ensure!(
            MEMBERSHIP_KINDS.contains(&event.kind),
            "unsupported membership event kind"
        );
        ensure!(
            exact_tag(&event.tags, "h") == Some(coordinate.group_id.as_str()),
            "membership event is for another group"
        );
        ensure!(
            event.tags.iter().any(|tag| {
                tag.first().map(String::as_str) == Some("p")
                    && tag.get(1).map(String::as_str) == Some(account_public_key_hex)
            }),
            "membership event is not relevant to the selected account"
        );
        let replace = self
            .membership_event
            .as_ref()
            .is_none_or(|current| replaceable_order(&event) > replaceable_order(current));
        if replace {
            self.membership = if event.kind == 9_000 {
                MembershipState::Member
            } else {
                MembershipState::Removed
            };
            self.membership_event = Some(event);
        }
        Ok(())
    }

    pub fn ingest_room_event(&mut self, coordinate: &GroupCoordinate, event: Event) -> Result<()> {
        let event = OpaqueVerifiedEvent::from_event(&event)?;
        ensure!(
            exact_tag(&event.tags, "h") == Some(coordinate.group_id.as_str()),
            "room event is for another NIP-29 group"
        );
        self.recent_room_events
            .retain(|current| current.event_id != event.event_id);
        self.recent_room_events.push(event);
        self.recent_room_events
            .sort_by(|left, right| replaceable_order(right).cmp(&replaceable_order(left)));
        self.recent_room_events.truncate(MAX_RECENT_ROOM_EVENTS);
        Ok(())
    }
}

fn exact_tag<'a>(tags: &'a [Vec<String>], name: &str) -> Option<&'a str> {
    let mut values = tags
        .iter()
        .filter(|tag| tag.first().map(String::as_str) == Some(name));
    let first = values.next()?;
    if values.next().is_some() || first.len() != 2 {
        return None;
    }
    first.get(1).map(String::as_str)
}

pub fn canonical_wss_relay(value: &str) -> Result<String> {
    ensure!(
        value.len() <= 2_048
            && value.starts_with("wss://")
            && !value.contains('?')
            && !value.contains('#')
            && !value.chars().any(char::is_control),
        "relay URL must be a credential-free wss endpoint"
    );
    let parsed = RelayUrl::parse(value).context("parsing relay URL")?;
    let mut canonical = parsed.as_str().to_string();
    ensure!(
        !canonical.contains('@'),
        "relay URL must not contain credentials"
    );
    if canonical.ends_with('/') {
        canonical.pop();
    }
    ensure!(
        canonical == value.trim_end_matches('/'),
        "relay URL is not canonical"
    );
    Ok(canonical)
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProfileChoice {
    Skip,
    SaveLocal(Value),
    Publish(Value),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileChoiceOutcome {
    Skipped,
    SavedLocally,
    Published {
        event_id: String,
        relays: Vec<ProfileRelayReceipt>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileRelayReceipt {
    pub relay_url: String,
    pub outcome: ProfileRelayOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileRelayOutcome {
    Accepted,
    Rejected { reason: String },
    TimedOut,
    AuthenticationRequired,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BulkDecryptPlan {
    PromptOnce,
    Declined,
    Requests(Vec<Nip46OperationRequest>),
}

pub fn plan_bulk_decrypt(
    cache: &HydrationCache,
    signer_capability_ref: &str,
    peer_public_key: NostrPublicKeyHex,
    ciphertexts: &[String],
) -> Result<BulkDecryptPlan> {
    ensure!(
        !ciphertexts.is_empty() && ciphertexts.len() <= MAX_BULK_DECRYPT_ITEMS,
        "bulk decrypt must contain one to sixty-four items"
    );
    ensure!(
        ciphertexts
            .iter()
            .all(|ciphertext| !ciphertext.is_empty() && ciphertext.len() <= MAX_EVENT_BYTES),
        "bulk decrypt ciphertext exceeds its bound"
    );
    let consent = cache
        .read_bulk_decrypt_consent(signer_capability_ref)
        .context("reading durable bulk-decrypt consent")?
        .map_or(BulkDecryptConsentState::Unknown, |consent| consent.state);
    match consent {
        BulkDecryptConsentState::Unknown => Ok(BulkDecryptPlan::PromptOnce),
        BulkDecryptConsentState::Declined => Ok(BulkDecryptPlan::Declined),
        BulkDecryptConsentState::Allowed => Ok(BulkDecryptPlan::Requests(
            ciphertexts
                .iter()
                .map(|ciphertext| Nip46OperationRequest::Nip44Decrypt {
                    public_key: peer_public_key.clone(),
                    ciphertext: ciphertext.clone(),
                })
                .collect(),
        )),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HydratedSourceDocument {
    pub outcome: HydrationSourceOutcome,
    pub cache_value: Option<Value>,
}

pub trait NostrHydrationSource: Send + Sync {
    fn fetch(
        &self,
        request: HydrationSourceRequest,
    ) -> Pin<Box<dyn Future<Output = HydratedSourceDocument> + Send + 'static>>;

    fn is_enabled(&self, source: HydrationSource) -> bool {
        !matches!(
            source,
            HydrationSource::BuzzProfile | HydrationSource::ArmadaProfile
        )
    }
}

#[derive(Clone)]
pub struct SystemNostrHydrationSource {
    cache: HydrationCache,
    bootstrap_relays: Arc<[String]>,
    context: SystemHydrationContext,
}

#[derive(Clone, Debug, Default)]
pub struct SystemHydrationContext {
    pub generation: Option<u64>,
    pub hosted_account: Option<HostedSessionProjection>,
    pub hosted_device: Option<BindingProjection>,
}

impl SystemNostrHydrationSource {
    pub fn new(cache: HydrationCache) -> Result<Self> {
        Self::with_context(cache, SystemHydrationContext::default())
    }

    pub fn with_context(cache: HydrationCache, context: SystemHydrationContext) -> Result<Self> {
        let bootstrap_relays = SYSTEM_BOOTSTRAP_RELAYS
            .iter()
            .map(|relay| canonical_wss_relay(relay))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            cache,
            bootstrap_relays: bootstrap_relays.into(),
            context,
        })
    }

    async fn group_coordinates(&self, author: &str) -> Result<Vec<GroupCoordinate>> {
        if let Some(groups) = self
            .cache
            .read(HydrationCacheArea::Groups, "nip29-group-list")?
            .and_then(|value| value.as_array().cloned())
        {
            return groups
                .into_iter()
                .take(MAX_GROUPS)
                .filter_map(|group| {
                    let relay_url = group.get("relay_url")?.as_str()?;
                    let group_id = group.get("group_id")?.as_str()?;
                    Some(GroupCoordinate::new(relay_url, group_id))
                })
                .collect();
        }
        let events = fetch_from_relays(
            &self.bootstrap_relays,
            json!({
                "authors": [author],
                "kinds": [10_009],
                "limit": MAX_GROUPS,
            }),
            MAX_GROUPS as u32,
        )
        .await?;
        let mut projection = AuthorNostrProjection::default();
        projection.ingest(author, events)?;
        Ok(projection.groups)
    }

    async fn fetch_document(&self, request: &HydrationSourceRequest) -> Result<Value> {
        let author = request.fence.public_key_hex.as_str();
        match request.source {
            HydrationSource::Profile
            | HydrationSource::RelayPreferences
            | HydrationSource::Nip29GroupList => {
                let kind = match request.source {
                    HydrationSource::Profile => 0,
                    HydrationSource::RelayPreferences => 10_002,
                    HydrationSource::Nip29GroupList => 10_009,
                    _ => return Err(anyhow!("invalid author hydration source")),
                };
                let filter = json!({
                    "authors": [author],
                    "kinds": [kind],
                    "limit": request.item_limit.min(64),
                });
                let events =
                    fetch_from_relays(&self.bootstrap_relays, filter, request.item_limit).await?;
                let mut projection = AuthorNostrProjection::default();
                projection.ingest(author, events)?;
                match request.source {
                    HydrationSource::Profile => Ok(projection.profile.unwrap_or(Value::Null)),
                    HydrationSource::RelayPreferences => Ok(Value::Array(
                        projection
                            .relay_preferences
                            .into_iter()
                            .map(|relay| {
                                json!({
                                    "relay_url": relay.relay_url,
                                    "usage": match relay.usage {
                                        RelayUse::Read => "read",
                                        RelayUse::Write => "write",
                                        RelayUse::ReadWrite => "read_write",
                                    },
                                })
                            })
                            .collect(),
                    )),
                    HydrationSource::Nip29GroupList => Ok(Value::Array(
                        projection
                            .groups
                            .into_iter()
                            .map(|group| {
                                json!({
                                    "relay_url": group.relay_url,
                                    "group_id": group.group_id,
                                })
                            })
                            .collect(),
                    )),
                    _ => Err(anyhow!("invalid author hydration projection")),
                }
            }
            HydrationSource::MembershipMetadata | HydrationSource::RecentRooms => {
                let groups = self.group_coordinates(author).await?;
                let mut documents = Vec::new();
                for coordinate in groups.into_iter().take(MAX_GROUPS) {
                    let filter = match request.source {
                        HydrationSource::MembershipMetadata => json!({
                            "kinds": MEMBERSHIP_KINDS,
                            "#h": [coordinate.group_id],
                            "#p": [author],
                            "limit": request.item_limit.min(64),
                        }),
                        HydrationSource::RecentRooms => json!({
                            "#h": [coordinate.group_id],
                            "limit": request.item_limit.min(MAX_RECENT_ROOM_EVENTS as u32),
                        }),
                        _ => return Err(anyhow!("invalid group hydration source")),
                    };
                    let events = fetch_from_relays(
                        std::slice::from_ref(&coordinate.relay_url),
                        filter,
                        request.item_limit,
                    )
                    .await?;
                    let mut projection = GroupNostrProjection::default();
                    for event in events {
                        match request.source {
                            HydrationSource::MembershipMetadata => {
                                projection.ingest_membership(&coordinate, author, event)?;
                            }
                            HydrationSource::RecentRooms => {
                                projection.ingest_room_event(&coordinate, event)?;
                            }
                            _ => return Err(anyhow!("invalid group event projection")),
                        }
                    }
                    match request.source {
                        HydrationSource::MembershipMetadata => {
                            if let Some(event) = projection.membership_event {
                                documents.push(json!({
                                    "relay_url": coordinate.relay_url,
                                    "group_id": coordinate.group_id,
                                    "membership": match projection.membership {
                                        MembershipState::Unknown => "unknown",
                                        MembershipState::Member => "member",
                                        MembershipState::Removed => "removed",
                                    },
                                    "event": serde_json::from_str::<Value>(&event.event_json)?,
                                }));
                            }
                        }
                        HydrationSource::RecentRooms => {
                            for event in projection.recent_room_events {
                                documents.push(json!({
                                    "relay_url": coordinate.relay_url,
                                    "group_id": coordinate.group_id,
                                    "event": serde_json::from_str::<Value>(&event.event_json)?,
                                }));
                            }
                        }
                        _ => return Err(anyhow!("invalid group document projection")),
                    }
                    if documents.len() >= request.item_limit as usize {
                        break;
                    }
                }
                documents.truncate(request.item_limit as usize);
                Ok(Value::Array(documents))
            }
            HydrationSource::HostedAccount => {
                let Some(projection) = self.context.hosted_account.as_ref() else {
                    return Err(anyhow!("the hosted account adapter is disabled"));
                };
                ensure!(
                    self.context.generation == Some(request.fence.generation)
                        && projection.account_generation == Some(request.fence.generation)
                        && projection.omega_public_key_hex.as_deref() == Some(author),
                    "hosted account projection is fenced to another account generation"
                );
                Ok(serde_json::to_value(projection)?)
            }
            HydrationSource::HostedDevice => {
                let Some(projection) = self.context.hosted_device.as_ref() else {
                    return Err(anyhow!("the hosted device adapter is disabled"));
                };
                ensure!(
                    self.context.generation == Some(request.fence.generation)
                        && projection.omega_public_key_hex.as_deref() == Some(author),
                    "hosted device projection is fenced to another account generation"
                );
                Ok(serde_json::to_value(projection)?)
            }
            HydrationSource::BuzzProfile | HydrationSource::ArmadaProfile => {
                Err(anyhow!("the optional hydration adapter is disabled"))
            }
        }
    }
}

impl NostrHydrationSource for SystemNostrHydrationSource {
    fn fetch(
        &self,
        request: HydrationSourceRequest,
    ) -> Pin<Box<dyn Future<Output = HydratedSourceDocument> + Send + 'static>> {
        let source = self.clone();
        Box::pin(async move {
            let result = async_std::future::timeout(
                HYDRATION_SOURCE_TIMEOUT,
                source.fetch_document(&request),
            )
            .await;
            match result {
                Ok(Ok(cache_value)) => {
                    let items = cache_value.as_array().map_or_else(
                        || u32::from(!cache_value.is_null()),
                        |items| u32::try_from(items.len()).unwrap_or(u32::MAX),
                    );
                    HydratedSourceDocument {
                        outcome: HydrationSourceOutcome::Complete { items },
                        cache_value: Some(cache_value),
                    }
                }
                Err(_) => HydratedSourceDocument {
                    outcome: HydrationSourceOutcome::TimedOut {
                        scope: omega_identity_sync::TimeoutScope::Source,
                    },
                    cache_value: None,
                },
                Ok(Err(_)) => HydratedSourceDocument {
                    outcome: HydrationSourceOutcome::Offline,
                    cache_value: None,
                },
            }
        })
    }

    fn is_enabled(&self, source: HydrationSource) -> bool {
        match source {
            HydrationSource::HostedAccount => self.context.hosted_account.is_some(),
            HydrationSource::HostedDevice => self.context.hosted_device.is_some(),
            HydrationSource::BuzzProfile | HydrationSource::ArmadaProfile => false,
            _ => true,
        }
    }
}

async fn fetch_from_relays(
    relay_urls: &[String],
    filter: Value,
    item_limit: u32,
) -> Result<Vec<Event>> {
    ensure!(
        !relay_urls.is_empty() && relay_urls.len() <= MAX_BOOTSTRAP_RELAYS,
        "hydration relay set is outside its bound"
    );
    let mut events = BTreeMap::new();
    let mut completed_relays = 0_usize;
    for (relay_index, relay_url) in relay_urls.iter().enumerate() {
        let relay_url = canonical_wss_relay(relay_url)?;
        let subscription_id = format!("omega-hydration-{relay_index}");
        let operation = async {
            let (mut socket, _) = connect_async(&relay_url)
                .await
                .context("connecting to hydration relay")?;
            socket
                .send(Message::Text(
                    json!(["REQ", subscription_id, filter]).to_string().into(),
                ))
                .await
                .context("sending bounded hydration request")?;
            let mut relay_events = Vec::new();
            for _ in 0..=item_limit.min(256) {
                let Some(message) = socket.next().await else {
                    break;
                };
                let message = message.context("reading hydration relay response")?;
                let Message::Text(text) = message else {
                    continue;
                };
                ensure!(
                    text.len() <= MAX_EVENT_BYTES,
                    "hydration relay frame exceeds its cap"
                );
                let frame: Value =
                    serde_json::from_str(&text).context("decoding hydration relay frame")?;
                let Some(frame) = frame.as_array() else {
                    continue;
                };
                if frame.first().and_then(Value::as_str) == Some("EOSE")
                    && frame.get(1).and_then(Value::as_str) == Some(subscription_id.as_str())
                {
                    break;
                }
                if frame.first().and_then(Value::as_str) != Some("EVENT")
                    || frame.get(1).and_then(Value::as_str) != Some(subscription_id.as_str())
                {
                    continue;
                }
                let Some(event) = frame.get(2) else {
                    continue;
                };
                relay_events.push(Event::from_json(event.to_string())?);
            }
            socket
                .send(Message::Text(
                    json!(["CLOSE", subscription_id]).to_string().into(),
                ))
                .await
                .context("closing hydration subscription")?;
            Ok::<_, anyhow::Error>(relay_events)
        };
        if let Ok(Ok(relay_events)) = async_std::future::timeout(RELAY_TIMEOUT, operation).await {
            completed_relays = completed_relays.saturating_add(1);
            for event in relay_events {
                event.verify().context("verifying hydration event")?;
                events.insert(event.id.to_hex(), event);
                if events.len() >= item_limit as usize {
                    break;
                }
            }
        }
        if events.len() >= item_limit as usize {
            break;
        }
    }
    ensure!(
        completed_relays > 0,
        "no hydration relay completed its bounded request"
    );
    Ok(events.into_values().collect())
}

struct AgentUiHydrationRunner {
    cache: HydrationCache,
    source: Arc<dyn NostrHydrationSource>,
}

impl HydrationSourceRunner for AgentUiHydrationRunner {
    fn hydrate(
        &self,
        request: HydrationSourceRequest,
    ) -> Pin<Box<dyn Future<Output = HydrationSourceOutcome> + Send + 'static>> {
        let cache = self.cache.clone();
        let source = self.source.clone();
        Box::pin(async move {
            if request.fence != *cache.fence() {
                return HydrationSourceOutcome::Failed;
            }
            if !source.is_enabled(request.source) {
                return HydrationSourceOutcome::Disabled;
            }
            let fetched = source.fetch(request.clone()).await;
            let (area, key) = hydration_cache_location(request.source);
            match fetched.outcome {
                HydrationSourceOutcome::Complete { items } => {
                    let Some(value) = fetched.cache_value else {
                        return HydrationSourceOutcome::Failed;
                    };
                    if cache.write(area, key, value).is_err() {
                        return HydrationSourceOutcome::Failed;
                    }
                    HydrationSourceOutcome::Complete { items }
                }
                HydrationSourceOutcome::Offline => {
                    cached_fallback(&cache, area, key, CacheFallbackReason::Offline)
                        .unwrap_or(HydrationSourceOutcome::Offline)
                }
                HydrationSourceOutcome::Failed => {
                    cached_fallback(&cache, area, key, CacheFallbackReason::Failure)
                        .unwrap_or(HydrationSourceOutcome::Failed)
                }
                outcome => outcome,
            }
        })
    }
}

fn cached_fallback(
    cache: &HydrationCache,
    area: HydrationCacheArea,
    key: &str,
    reason: CacheFallbackReason,
) -> Option<HydrationSourceOutcome> {
    let value = cache.read(area, key).ok().flatten()?;
    let items = value
        .as_array()
        .map_or(1, |items| u32::try_from(items.len()).unwrap_or(u32::MAX));
    Some(HydrationSourceOutcome::Cached { items, reason })
}

fn hydration_cache_location(source: HydrationSource) -> (HydrationCacheArea, &'static str) {
    match source {
        HydrationSource::Profile => (HydrationCacheArea::Profiles, "profile"),
        HydrationSource::RelayPreferences => (HydrationCacheArea::Relays, "relay-preferences"),
        HydrationSource::Nip29GroupList => (HydrationCacheArea::Groups, "nip29-group-list"),
        HydrationSource::MembershipMetadata => (HydrationCacheArea::Groups, "membership"),
        HydrationSource::RecentRooms => (HydrationCacheArea::Groups, "recent-rooms"),
        HydrationSource::HostedAccount => (HydrationCacheArea::Profiles, "hosted-account"),
        HydrationSource::HostedDevice => (HydrationCacheArea::Profiles, "hosted-device"),
        HydrationSource::BuzzProfile => (HydrationCacheArea::Profiles, "buzz-profile"),
        HydrationSource::ArmadaProfile => (HydrationCacheArea::Profiles, "armada-profile"),
    }
}

fn default_hydration_plan(
    selection: &AccountSelectionToken,
    trigger: HydrationTrigger,
    fresh_unpublished_candidate: bool,
) -> std::result::Result<HydrationPlan, HydrationError> {
    let fence = HydrationAccountFence::new(
        selection.account_ref.clone(),
        selection.identity.public_key_hex().clone(),
        selection.generation,
    )?;
    let sources = [
        HydrationSource::Profile,
        HydrationSource::RelayPreferences,
        HydrationSource::Nip29GroupList,
        HydrationSource::MembershipMetadata,
        HydrationSource::RecentRooms,
        HydrationSource::HostedAccount,
        HydrationSource::HostedDevice,
        HydrationSource::BuzzProfile,
        HydrationSource::ArmadaProfile,
    ]
    .into_iter()
    .map(|source| HydrationSourcePlan::new(source, 15_000))
    .collect::<std::result::Result<Vec<_>, _>>()?;
    HydrationPlan::new(fence, trigger, 45_000, fresh_unpublished_candidate, sources)
}

pub async fn start_nostr_identity_hydration(
    cache_root: &Path,
    selection: AccountSelectionToken,
    trigger: HydrationTrigger,
    fresh_unpublished_candidate: bool,
    source: Arc<dyn NostrHydrationSource>,
) -> Result<HydrationReceipt> {
    let plan = default_hydration_plan(&selection, trigger, fresh_unpublished_candidate)
        .context("building bounded Nostr hydration plan")?;
    let cache = HydrationCache::open(cache_root, plan.fence.clone())
        .context("opening account-fenced Nostr hydration cache")?;
    let scheduler = HydrationScheduler::new(Arc::new(AgentUiHydrationRunner {
        cache: cache.clone(),
        source,
    }));
    let receipt = scheduler
        .run(plan)
        .await
        .context("running bounded Nostr identity hydration")?;
    cache
        .write_hydration_receipt(&receipt)
        .context("persisting Nostr hydration receipt")?;
    Ok(receipt)
}

pub fn start_system_identity_hydration(
    selection: AccountSelectionToken,
    trigger: HydrationTrigger,
    fresh_unpublished_candidate: bool,
) -> Result<async_std::task::JoinHandle<Result<HydrationReceipt>>> {
    start_system_identity_hydration_with_context(
        selection,
        trigger,
        fresh_unpublished_candidate,
        SystemHydrationContext::default(),
    )
}

pub fn start_system_identity_hydration_with_context(
    selection: AccountSelectionToken,
    trigger: HydrationTrigger,
    fresh_unpublished_candidate: bool,
    context: SystemHydrationContext,
) -> Result<async_std::task::JoinHandle<Result<HydrationReceipt>>> {
    let plan = default_hydration_plan(&selection, trigger, fresh_unpublished_candidate)
        .context("building bounded system Nostr hydration plan")?;
    let cache = HydrationCache::system(plan.fence.clone())
        .context("opening system account-fenced Nostr hydration cache")?;
    let source = Arc::new(SystemNostrHydrationSource::with_context(
        cache.clone(),
        context,
    )?);
    Ok(async_std::task::spawn(async move {
        let scheduler = HydrationScheduler::new(Arc::new(AgentUiHydrationRunner {
            cache: cache.clone(),
            source: source.clone(),
        }));
        let mut receipt = scheduler
            .run(plan)
            .await
            .context("running bounded system Nostr identity hydration")?;
        cache
            .write_hydration_receipt(&receipt)
            .context("persisting system Nostr hydration receipt")?;
        if receipt.background_continuation_available {
            let original =
                default_hydration_plan(&selection, HydrationTrigger::BackgroundContinuation, false)
                    .context("building one bounded system hydration continuation")?;
            if let Some(continuation) = original
                .continuation(&receipt, 45_000)
                .context("selecting retryable system hydration sources")?
            {
                receipt = scheduler
                    .run(continuation)
                    .await
                    .context("running one bounded system hydration continuation")?;
                cache
                    .write_hydration_receipt(&receipt)
                    .context("persisting system hydration continuation")?;
            }
        }
        if let Some(profile) = cache
            .read(HydrationCacheArea::Profiles, "profile")
            .context("reading verified profile hydration projection")?
        {
            let summary = profile_summary_from_value(&profile);
            AccountRegistryService::system(*app_identity::CHANNEL)
                .record_hydrated_profile(&selection, summary)
                .context("recording generation-fenced hydrated profile")?;
        }
        Ok(receipt)
    }))
}

pub fn start_system_identity_hydration_for_trigger(
    trigger: HydrationTrigger,
) -> Result<async_std::task::JoinHandle<Result<HydrationReceipt>>> {
    let registry = AccountRegistryService::system(*app_identity::CHANNEL);
    let selection = registry
        .selection_token()
        .context("selecting the active account for system hydration")?;
    let account = registry
        .partition_identity_service(&selection.account_ref)
        .inspect_account()
        .context("inspecting the active identity hydration candidate")?;
    let fence = HydrationAccountFence::new(
        selection.account_ref.clone(),
        selection.identity.public_key_hex().clone(),
        selection.generation,
    )
    .context("fencing the active identity hydration candidate")?;
    let has_hydration_receipt = HydrationCache::system(fence)
        .context("opening the active identity hydration cache")?
        .read_hydration_receipt()
        .context("checking the active identity hydration receipt")?
        .is_some();
    let fresh_unpublished_candidate =
        account.candidate_origin == IdentityCandidateOrigin::Local && !has_hydration_receipt;
    start_system_identity_hydration(selection, trigger, fresh_unpublished_candidate)
}

pub fn start_system_identity_hydration_on_startup()
-> Result<async_std::task::JoinHandle<Result<HydrationReceipt>>> {
    start_system_identity_hydration_for_trigger(HydrationTrigger::Startup)
}

pub async fn continue_nostr_identity_hydration(
    cache_root: &Path,
    selection: AccountSelectionToken,
    source: Arc<dyn NostrHydrationSource>,
) -> Result<Option<HydrationReceipt>> {
    let original =
        default_hydration_plan(&selection, HydrationTrigger::BackgroundContinuation, false)
            .context("building bounded continuation plan")?;
    let cache = HydrationCache::open(cache_root, original.fence.clone())
        .context("opening account-fenced Nostr hydration cache")?;
    let Some(previous) = cache
        .read_hydration_receipt()
        .context("reading previous Nostr hydration receipt")?
    else {
        return Ok(None);
    };
    let Some(plan) = original
        .continuation(&previous, 45_000)
        .context("building Nostr hydration continuation")?
    else {
        return Ok(None);
    };
    let scheduler = HydrationScheduler::new(Arc::new(AgentUiHydrationRunner {
        cache: cache.clone(),
        source,
    }));
    let receipt = scheduler
        .run(plan)
        .await
        .context("running bounded Nostr identity continuation")?;
    cache
        .write_hydration_receipt(&receipt)
        .context("persisting continued Nostr hydration receipt")?;
    Ok(Some(receipt))
}

#[derive(Debug)]
pub enum ProfilePublishError {
    RemotePermissionRequired,
    RelayAuthenticationRequired,
    RelayRejected(String),
    MissingAcknowledgement,
    Other(anyhow::Error),
}

impl std::fmt::Display for ProfilePublishError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RemotePermissionRequired => {
                formatter.write_str("kind-0 signing permission is required from the remote signer")
            }
            Self::RelayAuthenticationRequired => {
                formatter.write_str("relay authentication is required before profile publication")
            }
            Self::RelayRejected(reason) => {
                write!(
                    formatter,
                    "the relay rejected the kind-0 profile event: {reason}"
                )
            }
            Self::MissingAcknowledgement => {
                formatter.write_str("the relay did not acknowledge the exact kind-0 profile event")
            }
            Self::Other(error) => std::fmt::Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for ProfilePublishError {}

impl From<anyhow::Error> for ProfilePublishError {
    fn from(error: anyhow::Error) -> Self {
        Self::Other(error)
    }
}

pub async fn apply_profile_choice(
    broker: &SignerBroker,
    route: &SignerRoute,
    selection: AccountSelectionToken,
    choice: ProfileChoice,
    relay_urls: &[String],
    created_at: u64,
    timer: gpui::BackgroundExecutor,
    save_local: impl FnOnce(&Value) -> Result<()>,
) -> std::result::Result<ProfileChoiceOutcome, ProfilePublishError> {
    match choice {
        ProfileChoice::Skip => Ok(ProfileChoiceOutcome::Skipped),
        ProfileChoice::SaveLocal(profile) => {
            validate_profile_value(&profile)?;
            save_local(&profile)?;
            Ok(ProfileChoiceOutcome::SavedLocally)
        }
        ProfileChoice::Publish(profile) => {
            validate_profile_value(&profile)?;
            let content = serde_json::to_string(&profile).context("encoding profile metadata")?;
            let digest = format!("{:x}", Sha256::digest(content.as_bytes()));
            let event_template = UnsignedEventTemplate {
                created_at,
                kind: 0,
                tags: Vec::new(),
                content,
            };
            let request = AdmittedSigningRequest {
                request_ref: ReceiptRef::new(format!("omega.profile.{}", &digest[..32]))
                    .context("building profile signing receipt")?,
                identity_ref: selection.identity.identity_ref().clone(),
                purpose: SigningPurpose::NostrEvent,
                event: event_template.clone(),
            };
            let signed = broker
                .sign(route, selection.clone(), request)
                .await
                .map_err(|error| match error {
                    SignerBrokerError::ProfilePermissionRequired => {
                        ProfilePublishError::RemotePermissionRequired
                    }
                    error => ProfilePublishError::Other(anyhow!(error)),
                })?;
            let event = validate_signed_profile(&selection, &event_template, &signed)?;
            let relays = publish_exact_profile_event(&event, relay_urls, timer).await?;
            if relays
                .iter()
                .any(|receipt| receipt.outcome == ProfileRelayOutcome::Accepted)
            {
                AccountRegistryService::system(*app_identity::CHANNEL)
                    .record_hydrated_profile(&selection, profile_summary_from_value(&profile))
                    .context("recording generation-fenced published profile")?;
            }
            Ok(ProfileChoiceOutcome::Published {
                event_id: event.id.to_hex(),
                relays,
            })
        }
    }
}

fn validate_profile_value(profile: &Value) -> Result<()> {
    ensure!(
        profile.is_object(),
        "profile metadata must be a JSON object"
    );
    ensure!(
        serde_json::to_vec(profile)?.len() <= MAX_PROFILE_CONTENT_BYTES,
        "profile metadata exceeds its byte cap"
    );
    Ok(())
}

fn profile_summary_from_value(profile: &Value) -> Option<AccountProfileSummary> {
    profile.as_object().map(|profile| AccountProfileSummary {
        display_name: profile
            .get("display_name")
            .or_else(|| profile.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        avatar_ref: profile
            .get("picture")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn validate_signed_profile(
    selection: &AccountSelectionToken,
    expected: &UnsignedEventTemplate,
    signed: &SigningResult,
) -> std::result::Result<Event, ProfilePublishError> {
    let event =
        Event::from_json(&signed.signed_event_json).context("decoding signed profile metadata")?;
    event
        .verify()
        .context("verifying signed profile metadata")?;
    if expected.kind != 0
        || !expected.tags.is_empty()
        || event.kind.as_u16() != expected.kind
        || event.created_at.as_secs() != expected.created_at
        || event.content.as_bytes() != expected.content.as_bytes()
        || event.pubkey.to_hex() != selection.identity.public_key_hex().as_str()
        || event.id.to_hex() != signed.event_id
        || event.sig.to_string() != signed.signature
        || !event.tags.is_empty()
    {
        return Err(anyhow!("the signer returned a different kind-0 profile event").into());
    }
    Ok(event)
}

async fn publish_exact_profile_event(
    event: &Event,
    relay_urls: &[String],
    timer: gpui::BackgroundExecutor,
) -> std::result::Result<Vec<ProfileRelayReceipt>, ProfilePublishError> {
    if relay_urls.is_empty() || relay_urls.len() > MAX_BOOTSTRAP_RELAYS {
        return Err(anyhow!("profile publication requires one to eight relays").into());
    }
    let relays = relay_urls
        .iter()
        .map(|relay| canonical_wss_relay(relay))
        .collect::<Result<BTreeSet<_>>>()?;
    if relays.len() != relay_urls.len() {
        return Err(anyhow!("profile relays must be unique").into());
    }
    let mut receipts = Vec::new();
    for relay in relays {
        let outcome =
            relay_outcome(publish_exact_profile_event_to_relay(event, &relay, timer.clone()).await);
        receipts.push(ProfileRelayReceipt {
            relay_url: relay,
            outcome,
        });
    }
    Ok(receipts)
}

fn relay_outcome(result: std::result::Result<(), ProfilePublishError>) -> ProfileRelayOutcome {
    match result {
        Ok(()) => ProfileRelayOutcome::Accepted,
        Err(ProfilePublishError::RelayAuthenticationRequired) => {
            ProfileRelayOutcome::AuthenticationRequired
        }
        Err(ProfilePublishError::RelayRejected(reason)) => ProfileRelayOutcome::Rejected {
            reason: bounded_relay_reason(&reason),
        },
        Err(ProfilePublishError::MissingAcknowledgement) => ProfileRelayOutcome::TimedOut,
        Err(ProfilePublishError::RemotePermissionRequired | ProfilePublishError::Other(_)) => {
            ProfileRelayOutcome::Failed
        }
    }
}

fn bounded_relay_reason(reason: &str) -> String {
    reason
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

async fn publish_exact_profile_event_to_relay(
    event: &Event,
    relay_url: &str,
    timer: gpui::BackgroundExecutor,
) -> std::result::Result<(), ProfilePublishError> {
    let connect = connect_async(relay_url).fuse();
    let timeout = timer.timer(RELAY_TIMEOUT).fuse();
    pin_mut!(connect, timeout);
    let (mut socket, _) = select! {
        result = connect => result.context("connecting to profile relay")?,
        _ = timeout => return Err(ProfilePublishError::MissingAcknowledgement),
    };
    socket
        .send(Message::Text(json!(["EVENT", event]).to_string().into()))
        .await
        .context("publishing kind-0 profile event")?;
    for _ in 0..MAX_ACK_FRAMES {
        let incoming = socket.next().fuse();
        let timeout = timer.timer(RELAY_TIMEOUT).fuse();
        pin_mut!(incoming, timeout);
        let message = select! {
            message = incoming => message,
            _ = timeout => return Err(ProfilePublishError::MissingAcknowledgement),
        };
        let Some(message) = message else {
            return Err(ProfilePublishError::MissingAcknowledgement);
        };
        let message = message.context("reading profile relay acknowledgement")?;
        let text = match message {
            Message::Text(text) => text.to_string(),
            Message::Binary(bytes) => String::from_utf8(bytes.to_vec())
                .context("decoding profile relay acknowledgement")?,
            Message::Ping(payload) => {
                socket
                    .send(Message::Pong(payload))
                    .await
                    .context("answering profile relay ping")?;
                continue;
            }
            Message::Close(_) => return Err(ProfilePublishError::MissingAcknowledgement),
            Message::Pong(_) | Message::Frame(_) => continue,
        };
        if text.len() > MAX_EVENT_BYTES {
            return Err(anyhow!("relay acknowledgement exceeds its cap").into());
        }
        match parse_profile_ack(&event.id.to_hex(), &text)? {
            ProfileAck::Unrelated => continue,
            ProfileAck::Accepted => return Ok(()),
            ProfileAck::AuthenticationRequired => {
                return Err(ProfilePublishError::RelayAuthenticationRequired);
            }
            ProfileAck::Rejected(reason) => {
                return Err(ProfilePublishError::RelayRejected(reason));
            }
        }
    }
    Err(ProfilePublishError::MissingAcknowledgement)
}

#[derive(Debug, PartialEq, Eq)]
enum ProfileAck {
    Unrelated,
    Accepted,
    AuthenticationRequired,
    Rejected(String),
}

fn parse_profile_ack(
    expected_event_id: &str,
    text: &str,
) -> std::result::Result<ProfileAck, ProfilePublishError> {
    let frame: Value = serde_json::from_str(text).context("decoding relay acknowledgement")?;
    let Some(frame) = frame.as_array() else {
        return Ok(ProfileAck::Unrelated);
    };
    if frame.first().and_then(Value::as_str) == Some("AUTH") {
        return Ok(ProfileAck::AuthenticationRequired);
    }
    if frame.first().and_then(Value::as_str) != Some("OK")
        || frame.get(1).and_then(Value::as_str) != Some(expected_event_id)
    {
        return Ok(ProfileAck::Unrelated);
    }
    if frame.len() != 4
        || frame.get(2).and_then(Value::as_bool).is_none()
        || frame.get(3).and_then(Value::as_str).is_none()
    {
        return Err(anyhow!("relay returned a malformed acknowledgement").into());
    }
    if frame.get(2).and_then(Value::as_bool) == Some(true) {
        return Ok(ProfileAck::Accepted);
    }
    Ok(ProfileAck::Rejected(bounded_relay_reason(
        frame
            .get(3)
            .and_then(Value::as_str)
            .unwrap_or("profile event rejected"),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use omega_identity::{AccountRef, IdentityRef, PublicIdentity};
    use std::sync::Mutex;

    fn selection(keys: &Keys, generation: u64) -> AccountSelectionToken {
        AccountSelectionToken {
            account_ref: AccountRef::new("account-fixture").expect("account ref"),
            identity: PublicIdentity::from_public_key_hex(
                IdentityRef::new("identity-fixture").expect("identity ref"),
                keys.public_key().to_hex(),
            )
            .expect("public identity"),
            generation,
        }
    }

    fn signed_event(
        keys: &Keys,
        kind: u16,
        created_at: u64,
        tags: Vec<Tag>,
        content: &str,
    ) -> Event {
        EventBuilder::new(Kind::from(kind), content)
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(created_at))
            .sign_with_keys(keys)
            .expect("signed event")
    }

    #[test]
    fn author_projection_uses_latest_then_event_id_tie_break() {
        let keys = Keys::generate();
        let older = signed_event(&keys, 0, 10, Vec::new(), r#"{"name":"older"}"#);
        let left = signed_event(&keys, 0, 11, Vec::new(), r#"{"name":"left"}"#);
        let right = signed_event(&keys, 0, 11, Vec::new(), r#"{"name":"right"}"#);
        let expected = if left.id > right.id { "left" } else { "right" };
        let mut projection = AuthorNostrProjection::default();
        projection
            .ingest(&keys.public_key().to_hex(), [right, older, left])
            .expect("project profile events");
        assert_eq!(
            projection
                .profile
                .as_ref()
                .and_then(|value| value["name"].as_str()),
            Some(expected)
        );
        assert_eq!(projection.latest_events.len(), 1);
    }

    #[test]
    fn relay_and_group_lists_are_canonical_bounded_and_preserve_private_content() {
        let keys = Keys::generate();
        let relay_list = signed_event(
            &keys,
            10_002,
            10,
            vec![
                Tag::parse(["r", "wss://read.example.com/", "read"]).expect("read relay"),
                Tag::parse(["r", "wss://write.example.com", "write"]).expect("write relay"),
            ],
            "",
        );
        let groups = signed_event(
            &keys,
            10_009,
            11,
            vec![
                Tag::parse(["group", "same-id", "wss://one.example.com", "One"])
                    .expect("first group"),
                Tag::parse(["group", "same-id", "wss://two.example.com", "Two"])
                    .expect("second group"),
                Tag::parse(["unsupported", "opaque"]).expect("opaque tag"),
            ],
            "opaque-nip44-content",
        );
        let mut projection = AuthorNostrProjection::default();
        projection
            .ingest(&keys.public_key().to_hex(), [relay_list, groups])
            .expect("project lists");
        assert_eq!(projection.relay_preferences.len(), 2);
        assert_eq!(projection.groups.len(), 2);
        assert_ne!(
            projection.groups[0].relay_url,
            projection.groups[1].relay_url
        );
        assert!(projection.has_opaque_private_groups);
        assert!(
            projection.latest_events[&10_009]
                .event_json
                .contains("opaque-nip44-content")
        );
    }

    #[test]
    fn pinned_group_state_and_membership_have_separate_authorities() {
        let relay_keys = Keys::generate();
        let admin_keys = Keys::generate();
        let member_keys = Keys::generate();
        let member_public_key = member_keys.public_key().to_hex();
        let coordinate =
            GroupCoordinate::new("wss://group.example.com", "room").expect("coordinate");
        let state = signed_event(
            &relay_keys,
            39_000,
            10,
            vec![Tag::parse(["d", "room"]).expect("d tag")],
            "",
        );
        let advisory = signed_event(
            &relay_keys,
            39_002,
            11,
            vec![
                Tag::parse(["d", "room"]).expect("d tag"),
                Tag::parse(["p", member_public_key.as_str()]).expect("advisory member"),
            ],
            "",
        );
        let added = signed_event(
            &admin_keys,
            9_000,
            12,
            vec![
                Tag::parse(["h", "room"]).expect("h tag"),
                Tag::parse(["p", member_public_key.as_str()]).expect("member"),
            ],
            "",
        );
        let removed = signed_event(
            &admin_keys,
            9_001,
            13,
            vec![
                Tag::parse(["h", "room"]).expect("h tag"),
                Tag::parse(["p", member_public_key.as_str()]).expect("member"),
            ],
            "",
        );
        let mut projection = GroupNostrProjection::default();
        projection
            .ingest_relay_state(&coordinate, &relay_keys.public_key().to_hex(), state)
            .expect("relay state");
        projection
            .ingest_relay_state(&coordinate, &relay_keys.public_key().to_hex(), advisory)
            .expect("advisory membership");
        assert_eq!(projection.membership, MembershipState::Unknown);
        projection
            .ingest_membership(&coordinate, &member_public_key, added)
            .expect("added member");
        projection
            .ingest_membership(&coordinate, &member_public_key, removed)
            .expect("removed member");
        assert_eq!(projection.membership, MembershipState::Removed);
        assert!(projection.advisory_members.is_some());
    }

    #[test]
    fn recent_room_page_is_verified_deduplicated_and_bounded() {
        let keys = Keys::generate();
        let coordinate =
            GroupCoordinate::new("wss://group.example.com", "room").expect("coordinate");
        let mut projection = GroupNostrProjection::default();
        for created_at in 0..=MAX_RECENT_ROOM_EVENTS {
            projection
                .ingest_room_event(
                    &coordinate,
                    signed_event(
                        &keys,
                        9,
                        u64::try_from(created_at).expect("timestamp"),
                        vec![Tag::parse(["h", "room"]).expect("h tag")],
                        &format!("message-{created_at}"),
                    ),
                )
                .expect("room event");
        }
        assert_eq!(projection.recent_room_events.len(), MAX_RECENT_ROOM_EVENTS);
        assert_eq!(
            projection.recent_room_events[0].created_at,
            u64::try_from(MAX_RECENT_ROOM_EVENTS).expect("timestamp")
        );
    }

    #[test]
    fn exact_profile_ack_rejects_wrong_or_malformed_outcomes() {
        let event_id = "a".repeat(64);
        assert_eq!(
            parse_profile_ack(
                &event_id,
                &json!(["OK", event_id, true, "saved"]).to_string()
            )
            .expect("accepted ack"),
            ProfileAck::Accepted
        );
        assert_eq!(
            parse_profile_ack(
                &"a".repeat(64),
                &json!(["OK", "b".repeat(64), true, "saved"]).to_string()
            )
            .expect("unrelated ack"),
            ProfileAck::Unrelated
        );
        assert!(
            parse_profile_ack(
                &"a".repeat(64),
                &json!(["OK", "a".repeat(64), true]).to_string()
            )
            .is_err()
        );
        assert_eq!(
            parse_profile_ack(&"a".repeat(64), &json!(["AUTH", "challenge"]).to_string())
                .expect("auth challenge"),
            ProfileAck::AuthenticationRequired
        );
    }

    #[test]
    fn relay_receipts_preserve_success_and_timeout_without_leaking_rejection_text() {
        assert_eq!(relay_outcome(Ok(())), ProfileRelayOutcome::Accepted);
        assert_eq!(
            relay_outcome(Err(ProfilePublishError::MissingAcknowledgement)),
            ProfileRelayOutcome::TimedOut
        );
        let reason = format!("{}\nsecret", "x".repeat(300));
        let ProfileRelayOutcome::Rejected { reason } =
            relay_outcome(Err(ProfilePublishError::RelayRejected(reason)))
        else {
            panic!("rejection receipt");
        };
        assert!(reason.len() <= MAX_RELAY_REASON_BYTES);
        assert!(!reason.chars().any(char::is_control));
    }

    #[test]
    fn signed_profile_must_match_content_and_timestamp_exactly() {
        let keys = Keys::generate();
        let selection = selection(&keys, 1);
        let event = signed_event(&keys, 0, 41, Vec::new(), r#"{"name":"Omega"}"#);
        let signed = SigningResult {
            request_ref: ReceiptRef::new("profile-fixture").expect("receipt ref"),
            identity: selection.identity.clone(),
            event_id: event.id.to_hex(),
            signature: event.sig.to_string(),
            signed_event_json: event.as_json(),
        };
        let exact = UnsignedEventTemplate {
            created_at: 41,
            kind: 0,
            tags: Vec::new(),
            content: r#"{"name":"Omega"}"#.to_string(),
        };
        validate_signed_profile(&selection, &exact, &signed).expect("exact signed event");
        let different_content = UnsignedEventTemplate {
            content: r#"{"name":"omega"}"#.to_string(),
            ..exact.clone()
        };
        assert!(validate_signed_profile(&selection, &different_content, &signed).is_err());
        let different_timestamp = UnsignedEventTemplate {
            created_at: 42,
            ..exact
        };
        assert!(validate_signed_profile(&selection, &different_timestamp, &signed).is_err());
    }

    #[test]
    fn bulk_decrypt_requires_durable_consent_and_expands_to_ordinary_requests() {
        let keys = Keys::generate();
        let peer = Keys::generate();
        let selected = selection(&keys, 1);
        let fence = HydrationAccountFence::new(
            selected.account_ref,
            selected.identity.public_key_hex().clone(),
            selected.generation,
        )
        .expect("fence");
        let directory = tempfile::tempdir().expect("temporary cache");
        let cache = HydrationCache::open(directory.path(), fence).expect("cache");
        let ciphertexts = vec!["ciphertext-one".to_string(), "ciphertext-two".to_string()];

        assert_eq!(
            plan_bulk_decrypt(
                &cache,
                "capability-fixture",
                selection(&peer, 1).identity.public_key_hex().clone(),
                &ciphertexts,
            )
            .expect("unknown plan"),
            BulkDecryptPlan::PromptOnce
        );
        cache
            .set_bulk_decrypt_consent("capability-fixture", BulkDecryptConsentState::Declined)
            .expect("declined consent");
        assert_eq!(
            plan_bulk_decrypt(
                &cache,
                "capability-fixture",
                selection(&peer, 1).identity.public_key_hex().clone(),
                &ciphertexts,
            )
            .expect("declined plan"),
            BulkDecryptPlan::Declined
        );
        cache
            .set_bulk_decrypt_consent("capability-fixture", BulkDecryptConsentState::Allowed)
            .expect("allowed consent");
        let BulkDecryptPlan::Requests(requests) = plan_bulk_decrypt(
            &cache,
            "capability-fixture",
            selection(&peer, 1).identity.public_key_hex().clone(),
            &ciphertexts,
        )
        .expect("allowed plan") else {
            panic!("ordinary decrypt requests");
        };
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| matches!(request, Nip46OperationRequest::Nip44Decrypt { .. }))
        );
    }

    #[derive(Default)]
    struct RecordingHydrationSource {
        calls: Mutex<Vec<HydrationSourceRequest>>,
    }

    impl NostrHydrationSource for RecordingHydrationSource {
        fn fetch(
            &self,
            request: HydrationSourceRequest,
        ) -> Pin<Box<dyn Future<Output = HydratedSourceDocument> + Send + 'static>> {
            self.calls.lock().expect("calls lock").push(request);
            Box::pin(async {
                HydratedSourceDocument {
                    outcome: HydrationSourceOutcome::Complete { items: 1 },
                    cache_value: Some(json!({"verified": true})),
                }
            })
        }

        fn is_enabled(&self, source: HydrationSource) -> bool {
            !matches!(
                source,
                HydrationSource::HostedAccount
                    | HydrationSource::HostedDevice
                    | HydrationSource::BuzzProfile
                    | HydrationSource::ArmadaProfile
            )
        }
    }

    #[test]
    fn hydration_is_generation_fenced_bounded_and_disables_unavailable_adapters() {
        smol::block_on(async {
            let keys = Keys::generate();
            let selection = selection(&keys, 7);
            let directory = tempfile::tempdir().expect("temporary cache");
            let source = Arc::new(RecordingHydrationSource::default());
            let receipt = start_nostr_identity_hydration(
                directory.path(),
                selection.clone(),
                HydrationTrigger::Startup,
                false,
                source.clone(),
            )
            .await
            .expect("hydration receipt");
            assert_eq!(receipt.sources.len(), 9);
            assert_eq!(source.calls.lock().expect("calls lock").len(), 5);
            assert!(
                source
                    .calls
                    .lock()
                    .expect("calls lock")
                    .iter()
                    .all(|request| request.fence.generation == selection.generation)
            );
            for disabled in [
                HydrationSource::HostedAccount,
                HydrationSource::HostedDevice,
                HydrationSource::BuzzProfile,
                HydrationSource::ArmadaProfile,
            ] {
                assert!(receipt.sources.iter().any(|source| {
                    source.source == disabled && source.outcome == HydrationSourceOutcome::Disabled
                }));
            }

            let fresh = start_nostr_identity_hydration(
                directory.path(),
                selection,
                HydrationTrigger::Imported,
                true,
                Arc::new(RecordingHydrationSource::default()),
            )
            .await
            .expect("fresh receipt");
            assert!(fresh.sources.is_empty());
        });
    }

    #[test]
    fn hosted_projection_hydration_requires_the_exact_public_key_and_generation() {
        smol::block_on(async {
            let keys = Keys::generate();
            let selection = selection(&keys, 9);
            let fence = HydrationAccountFence::new(
                selection.account_ref.clone(),
                selection.identity.public_key_hex().clone(),
                selection.generation,
            )
            .expect("fence");
            let directory = tempfile::tempdir().expect("temporary cache");
            let cache = HydrationCache::open(directory.path(), fence.clone()).expect("cache");
            let source = SystemNostrHydrationSource::with_context(
                cache,
                SystemHydrationContext {
                    generation: Some(selection.generation),
                    hosted_account: Some(HostedSessionProjection {
                        omega_public_key_hex: Some(
                            selection.identity.public_key_hex().as_str().to_string(),
                        ),
                        account_generation: Some(selection.generation),
                        ..HostedSessionProjection::default()
                    }),
                    hosted_device: None,
                },
            )
            .expect("system source");
            let document = source
                .fetch(HydrationSourceRequest {
                    fence,
                    trigger: HydrationTrigger::Startup,
                    source: HydrationSource::HostedAccount,
                    item_limit: 1,
                })
                .await;
            assert_eq!(
                document.outcome,
                HydrationSourceOutcome::Complete { items: 1 }
            );
            let value = document.cache_value.expect("public-safe projection");
            assert!(value.get("omegaPublicKeyHex").is_some());
            assert!(value.get("accessToken").is_none());
        });
    }

    #[test]
    fn startup_hydration_entrypoint_requires_no_identity_or_sync_arguments() {
        let _entrypoint: fn() -> Result<async_std::task::JoinHandle<Result<HydrationReceipt>>> =
            start_system_identity_hydration_on_startup;
    }
}
