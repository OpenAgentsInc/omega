//! Transport-neutral NIP-MKT discovery state (omega#244).
//!
//! Omega owns the WebSocket and the NIP-11 fetch. Every event admitted into
//! the corpus is validated by the Immortal domain module
//! (`immortal_client::domain`): canonical ID, BIP-340 signature, and the
//! complete NIP-MKT public-record contract. This module never reimplements
//! event, signature, or MKT validation.

use std::collections::{BTreeMap, VecDeque};
use std::mem;

use immortal_client::domain::{
    Event, MKT_OFFERING_KIND, MKT_PROVIDER_PROFILE_KIND, MKT_SWP_KEY_ROTATION_KIND,
    MKT_SWP_RELAY_SET_KIND, ReplacementDecision, compare_replacement_order,
    validate_mkt_public_event,
};
use serde_json::{Value, json};

pub const DEFAULT_DEV_RELAY_URL: &str = "ws://127.0.0.1:18080";
pub const RELAY_URL_ENVIRONMENT_VARIABLE: &str = "OMEGA_MARKET_RELAY_URL";
pub const SUBSCRIPTION_ID: &str = "omega-market-discovery";
pub const NIP11_ACCEPT_MEDIA_TYPE: &str = "application/nostr+json";
/// The NIP-11 `supported_extensions` entry that gates the whole panel.
pub const NIP_MKT_EXTENSION: &str = "nip-mkt";
/// The relay-observable MKT-SWP surface advertisement.
pub const MKT_SWP_EXTENSION: &str = "mkt-swp:1";

const DISCOVERY_EVENT_LIMIT: usize = 256;
pub const MAX_RELAY_INFORMATION_BYTES: usize = 65_536;
const MAX_DIAGNOSTICS: usize = 32;
const MAX_TRACKED_HEADS: usize = 1_024;
const MAX_DISPLAY_TEXT_CHARS: usize = 160;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketDiscoveryConfig {
    pub relay_websocket_url: String,
}

impl MarketDiscoveryConfig {
    pub fn from_environment() -> Self {
        let relay_websocket_url = std::env::var(RELAY_URL_ENVIRONMENT_VARIABLE)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_DEV_RELAY_URL.to_owned());
        Self {
            relay_websocket_url,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.relay_websocket_url.starts_with("ws://")
            || self.relay_websocket_url.starts_with("wss://")
        {
            Ok(())
        } else {
            Err("relay URL must use the ws or wss scheme".to_owned())
        }
    }

    /// The NIP-11 relay information URL derived from the WebSocket URL.
    pub fn relay_information_url(&self) -> Result<String, String> {
        if let Some(rest) = self.relay_websocket_url.strip_prefix("ws://") {
            Ok(format!("http://{rest}"))
        } else if let Some(rest) = self.relay_websocket_url.strip_prefix("wss://") {
            Ok(format!("https://{rest}"))
        } else {
            Err("relay URL must use the ws or wss scheme".to_owned())
        }
    }
}

/// The result of the NIP-11 gate check: the relay must advertise NIP-01,
/// NIP-11, and the `nip-mkt` extension before the panel subscribes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketRelayGate {
    pub relay_name: String,
    pub advertises_mkt_swp: bool,
    pub max_limit: usize,
}

pub fn validate_market_relay_information(text: &str) -> Result<MarketRelayGate, String> {
    if text.len() > MAX_RELAY_INFORMATION_BYTES {
        return Err("relay information exceeds 65536 bytes".to_owned());
    }
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("relay information is not JSON: {error}"))?;
    let document = value
        .as_object()
        .ok_or_else(|| "relay information must be a JSON object".to_owned())?;
    let relay_name = document
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "relay information requires a name".to_owned())?;
    if relay_name.is_empty() || relay_name.len() > 160 {
        return Err("relay name must be 1-160 bytes".to_owned());
    }
    let supported_nips: Vec<u64> = document
        .get("supported_nips")
        .and_then(Value::as_array)
        .ok_or_else(|| "relay information requires supported_nips".to_owned())?
        .iter()
        .filter_map(Value::as_u64)
        .collect();
    if !supported_nips.contains(&1) || !supported_nips.contains(&11) {
        return Err("relay must advertise NIP-01 and NIP-11".to_owned());
    }
    let supported_extensions: Vec<&str> = document
        .get("supported_extensions")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !supported_extensions.contains(&NIP_MKT_EXTENSION) {
        return Err("relay does not advertise the nip-mkt extension".to_owned());
    }
    let max_limit = document
        .get("limitation")
        .and_then(Value::as_object)
        .and_then(|limitation| limitation.get("max_limit"))
        .and_then(Value::as_u64)
        .ok_or_else(|| "relay information requires limitation.max_limit".to_owned())?;
    if max_limit == 0 {
        return Err("relay limitation.max_limit must be positive".to_owned());
    }
    Ok(MarketRelayGate {
        relay_name: relay_name.to_owned(),
        advertises_mkt_swp: supported_extensions.contains(&MKT_SWP_EXTENSION),
        max_limit: usize::try_from(max_limit).unwrap_or(usize::MAX),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Idle,
    CheckingGate,
    GateFailed(String),
    Connecting,
    AwaitingSnapshot,
    Live,
    Disconnected(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOutcome {
    AcceptedEvent,
    RejectedEvent,
    SnapshotComplete,
    Ignored,
    Closed,
}

type HeadKey = (u16, String, String);

/// Bounded discovery corpus with the Immortal project-client snapshot rule:
/// events received before `EOSE` are provisional, and a partial reconnect
/// never mixes into the last complete snapshot.
pub struct MarketDiscovery {
    connection: ConnectionState,
    effective_limit: usize,
    pending: BTreeMap<HeadKey, Event>,
    heads: BTreeMap<HeadKey, Event>,
    snapshot_complete: bool,
    diagnostics: VecDeque<String>,
}

impl Default for MarketDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

impl MarketDiscovery {
    pub fn new() -> Self {
        Self {
            connection: ConnectionState::Idle,
            effective_limit: DISCOVERY_EVENT_LIMIT,
            pending: BTreeMap::new(),
            heads: BTreeMap::new(),
            snapshot_complete: false,
            diagnostics: VecDeque::new(),
        }
    }

    pub fn connection(&self) -> &ConnectionState {
        &self.connection
    }

    pub fn diagnostics(&self) -> impl Iterator<Item = &str> {
        self.diagnostics.iter().map(String::as_str)
    }

    pub fn begin_gate_check(&mut self) {
        self.connection = ConnectionState::CheckingGate;
    }

    pub fn gate_failed(&mut self, reason: String) {
        self.connection = ConnectionState::GateFailed(reason);
    }

    pub fn begin_connect(&mut self, gate: &MarketRelayGate) {
        self.effective_limit = DISCOVERY_EVENT_LIMIT.min(gate.max_limit);
        self.pending.clear();
        self.snapshot_complete = false;
        self.connection = ConnectionState::Connecting;
    }

    pub fn opened(&mut self) {
        self.connection = ConnectionState::AwaitingSnapshot;
    }

    pub fn disconnected(&mut self, reason: String) {
        self.pending.clear();
        self.snapshot_complete = false;
        self.connection = ConnectionState::Disconnected(reason);
    }

    /// The single bounded NIP-01 REQ includes market discovery heads and the
    /// immutable provider-network histories needed to verify sessions.
    pub fn subscription_request(&self) -> String {
        json!([
            "REQ",
            SUBSCRIPTION_ID,
            {
                "kinds": [
                    MKT_PROVIDER_PROFILE_KIND,
                    MKT_OFFERING_KIND,
                    MKT_SWP_KEY_ROTATION_KIND,
                    MKT_SWP_RELAY_SET_KIND
                ],
                "limit": self.effective_limit,
            }
        ])
        .to_string()
    }

    pub fn ingest_text(&mut self, text: &str, now: u64) -> Result<IngestOutcome, String> {
        let value: Value = serde_json::from_str(text)
            .map_err(|error| format!("relay frame is not JSON: {error}"))?;
        let frame = value
            .as_array()
            .ok_or_else(|| "relay frame must be a JSON array".to_owned())?;
        let label = frame
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| "relay frame requires a type label".to_owned())?;
        match label {
            "EVENT" => {
                if frame.get(1).and_then(Value::as_str) != Some(SUBSCRIPTION_ID) {
                    return Ok(IngestOutcome::Ignored);
                }
                let event = frame
                    .get(2)
                    .cloned()
                    .ok_or_else(|| "EVENT frame requires an event object".to_owned())?;
                Ok(self.ingest_event(event, now))
            }
            "EOSE" => {
                if frame.get(1).and_then(Value::as_str) != Some(SUBSCRIPTION_ID) {
                    return Ok(IngestOutcome::Ignored);
                }
                self.heads = mem::take(&mut self.pending);
                self.snapshot_complete = true;
                self.connection = ConnectionState::Live;
                Ok(IngestOutcome::SnapshotComplete)
            }
            "CLOSED" => {
                let reason = frame
                    .get(2)
                    .and_then(Value::as_str)
                    .unwrap_or("subscription closed by the relay");
                self.disconnected(truncate_display(reason));
                Ok(IngestOutcome::Closed)
            }
            "NOTICE" => {
                let notice = frame.get(1).and_then(Value::as_str).unwrap_or("notice");
                self.push_diagnostic(format!("relay notice: {}", truncate_display(notice)));
                Ok(IngestOutcome::Ignored)
            }
            _ => Ok(IngestOutcome::Ignored),
        }
    }

    fn ingest_event(&mut self, value: Value, now: u64) -> IngestOutcome {
        let event: Event = match serde_json::from_value(value) {
            Ok(event) => event,
            Err(error) => {
                self.push_diagnostic(format!("invalid event shape: {error}"));
                return IngestOutcome::RejectedEvent;
            }
        };
        if let Err(error) = event
            .validate_structure()
            .and_then(|()| event.validate_id())
            .and_then(|()| event.validate_crypto())
        {
            self.push_diagnostic(format!("rejected event: {error:?}"));
            return IngestOutcome::RejectedEvent;
        }
        if !matches!(
            event.kind,
            MKT_PROVIDER_PROFILE_KIND
                | MKT_OFFERING_KIND
                | MKT_SWP_KEY_ROTATION_KIND
                | MKT_SWP_RELAY_SET_KIND
        ) {
            self.push_diagnostic(format!("unexpected kind {}", event.kind));
            return IngestOutcome::RejectedEvent;
        }
        if let Err(error) = validate_mkt_public_event(&event) {
            self.push_diagnostic(format!("rejected MKT record: {error}"));
            return IngestOutcome::RejectedEvent;
        }
        if event.is_expired(now) {
            self.push_diagnostic("rejected expired record".to_owned());
            return IngestOutcome::RejectedEvent;
        }
        let Some(distinct) = event.distinct_parameter().map(str::to_owned) else {
            self.push_diagnostic("rejected record without a d identifier".to_owned());
            return IngestOutcome::RejectedEvent;
        };
        let key = (event.kind, event.pubkey.clone(), distinct);
        let target = if self.snapshot_complete {
            &mut self.heads
        } else {
            &mut self.pending
        };
        match target.get(&key) {
            Some(current) => {
                match compare_replacement_order(
                    current.created_at,
                    &current.id,
                    event.created_at,
                    &event.id,
                ) {
                    ReplacementDecision::ReplaceCurrent => {
                        target.insert(key, event);
                        IngestOutcome::AcceptedEvent
                    }
                    ReplacementDecision::KeepCurrent | ReplacementDecision::Duplicate => {
                        IngestOutcome::Ignored
                    }
                }
            }
            None => {
                if target.len() >= MAX_TRACKED_HEADS {
                    self.push_diagnostic("discovery corpus is full".to_owned());
                    return IngestOutcome::RejectedEvent;
                }
                target.insert(key, event);
                IngestOutcome::AcceptedEvent
            }
        }
    }

    pub fn providers(&self) -> Vec<ProviderListing> {
        self.heads
            .values()
            .filter(|event| event.kind == MKT_PROVIDER_PROFILE_KIND)
            .map(ProviderListing::from_validated_event)
            .collect()
    }

    pub fn offerings(&self) -> Vec<OfferingListing> {
        self.heads
            .values()
            .filter(|event| event.kind == MKT_OFFERING_KIND)
            .map(OfferingListing::from_validated_event)
            .collect()
    }

    pub fn provider_network_events(&self, provider_id: &str) -> Vec<Event> {
        self.heads
            .values()
            .filter(|event| {
                matches!(
                    event.kind,
                    MKT_SWP_KEY_ROTATION_KIND | MKT_SWP_RELAY_SET_KIND
                ) && event.tag_values("provider").next() == Some(provider_id)
            })
            .cloned()
            .collect()
    }

    pub fn network_events(&self) -> Vec<Event> {
        self.heads
            .values()
            .filter(|event| {
                matches!(
                    event.kind,
                    MKT_SWP_KEY_ROTATION_KIND | MKT_SWP_RELAY_SET_KIND
                )
            })
            .cloned()
            .collect()
    }

    fn push_diagnostic(&mut self, diagnostic: String) {
        if self.diagnostics.len() >= MAX_DIAGNOSTICS {
            self.diagnostics.pop_front();
        }
        self.diagnostics.push_back(diagnostic);
    }
}

/// Display projection of a validated `kind:39600` Provider Profile. The
/// required tags were already enforced by `validate_mkt_public_event`, so
/// extraction here is presentation only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderListing {
    pub provider_id: String,
    pub pubkey: String,
    pub status: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub profiles: Vec<String>,
}

impl ProviderListing {
    fn from_validated_event(event: &Event) -> Self {
        let content: Option<Value> = serde_json::from_str(&event.content).ok();
        let content_text = |field: &str| {
            content
                .as_ref()
                .and_then(|value| value.get(field))
                .and_then(Value::as_str)
                .map(truncate_display)
        };
        Self {
            provider_id: event.distinct_parameter().unwrap_or_default().to_owned(),
            pubkey: event.pubkey.clone(),
            status: event
                .tag_values("status")
                .next()
                .unwrap_or_default()
                .to_owned(),
            name: content_text("name"),
            summary: content_text("summary"),
            profiles: profile_labels(event),
        }
    }
}

/// Display projection of a validated `kind:39601` Offering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferingListing {
    pub offering_id: String,
    pub pubkey: String,
    pub status: String,
    pub profile: String,
    pub provider_address: String,
    pub published_at: u64,
    pub sides: Vec<OfferingSideListing>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferingSideListing {
    pub input_asset_id: String,
    pub output_asset_id: String,
    pub minimum_amount: String,
    pub maximum_amount: String,
}

impl OfferingListing {
    fn from_validated_event(event: &Event) -> Self {
        let content: Option<Value> = serde_json::from_str(&event.content).ok();
        let sides = content
            .as_ref()
            .and_then(|content| content.pointer("/mkt_swp/sides"))
            .and_then(Value::as_array)
            .map(|sides| {
                sides
                    .iter()
                    .filter_map(|side| {
                        Some(OfferingSideListing {
                            input_asset_id: side.get("input_asset_id")?.as_str()?.to_owned(),
                            output_asset_id: side.get("output_asset_id")?.as_str()?.to_owned(),
                            minimum_amount: side.get("min")?.as_str()?.to_owned(),
                            maximum_amount: side.get("max")?.as_str()?.to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            offering_id: event.distinct_parameter().unwrap_or_default().to_owned(),
            pubkey: event.pubkey.clone(),
            status: event
                .tag_values("status")
                .next()
                .unwrap_or_default()
                .to_owned(),
            profile: profile_labels(event).into_iter().next().unwrap_or_default(),
            provider_address: event
                .tag_values("provider")
                .next()
                .unwrap_or_default()
                .to_owned(),
            published_at: event
                .tag_values("published_at")
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(event.created_at),
            sides,
        }
    }
}

fn profile_labels(event: &Event) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let slice = tag.as_slice();
            if slice.first().map(String::as_str) == Some("profile") {
                let id = slice.get(1)?;
                let version = slice.get(2)?;
                Some(format!("{id}:{version}"))
            } else {
                None
            }
        })
        .collect()
}

fn truncate_display(text: &str) -> String {
    if text.chars().count() <= MAX_DISPLAY_TEXT_CHARS {
        text.to_owned()
    } else {
        let truncated: String = text.chars().take(MAX_DISPLAY_TEXT_CHARS).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use immortal_client::domain::Tag;
    use immortal_client::market::MarketSigner;

    fn signer() -> MarketSigner {
        MarketSigner::from_secret_bytes([7u8; 32]).expect("test signer key is valid")
    }

    fn provider_event(signer: &MarketSigner, created_at: u64, name: &str) -> Event {
        signer.sign(
            created_at,
            MKT_PROVIDER_PROFILE_KIND,
            vec![
                Tag::new(vec!["d".into(), "prov1".into()]),
                Tag::new(vec!["status".into(), "active".into()]),
                Tag::new(vec!["published_at".into(), created_at.to_string()]),
                Tag::new(vec!["profile".into(), "demo.compute".into(), "1".into()]),
            ],
            json!({ "name": name, "summary": "test provider" }).to_string(),
        )
    }

    fn offering_event(signer: &MarketSigner, created_at: u64) -> Event {
        let provider_address = format!("{}:{}:prov1", MKT_PROVIDER_PROFILE_KIND, signer.pubkey());
        signer.sign(
            created_at,
            MKT_OFFERING_KIND,
            vec![
                Tag::new(vec!["d".into(), "off1".into()]),
                Tag::new(vec!["status".into(), "active".into()]),
                Tag::new(vec!["published_at".into(), created_at.to_string()]),
                Tag::new(vec!["profile".into(), "demo.compute".into(), "1".into()]),
                Tag::new(vec!["provider".into(), provider_address]),
            ],
            json!({ "description": "demo offering" }).to_string(),
        )
    }

    fn event_frame(event: &Event) -> String {
        json!(["EVENT", SUBSCRIPTION_ID, event]).to_string()
    }

    fn eose_frame() -> String {
        json!(["EOSE", SUBSCRIPTION_ID]).to_string()
    }

    fn passing_gate() -> MarketRelayGate {
        MarketRelayGate {
            relay_name: "test".to_owned(),
            advertises_mkt_swp: true,
            max_limit: 500,
        }
    }

    fn relay_information(extensions: &[&str]) -> String {
        json!({
            "name": "immortal-dev",
            "supported_nips": [1, 9, 11, 40],
            "supported_extensions": extensions,
            "limitation": { "max_limit": 500 },
        })
        .to_string()
    }

    #[test]
    fn gate_requires_the_nip_mkt_extension() {
        let passed =
            validate_market_relay_information(&relay_information(&["nip-mkt", "mkt-swp:1"]))
                .expect("gate passes with nip-mkt advertised");
        assert_eq!(passed.relay_name, "immortal-dev");
        assert!(passed.advertises_mkt_swp);
        assert_eq!(passed.max_limit, 500);

        let failed = validate_market_relay_information(&relay_information(&["nip-oa"]));
        assert_eq!(
            failed,
            Err("relay does not advertise the nip-mkt extension".to_owned())
        );
    }

    #[test]
    fn gate_requires_core_nips_and_bounded_input() {
        let missing = json!({
            "name": "immortal-dev",
            "supported_nips": [9, 40],
            "supported_extensions": ["nip-mkt"],
            "limitation": { "max_limit": 500 },
        })
        .to_string();
        assert_eq!(
            validate_market_relay_information(&missing),
            Err("relay must advertise NIP-01 and NIP-11".to_owned())
        );

        let oversized = " ".repeat(MAX_RELAY_INFORMATION_BYTES + 1);
        assert_eq!(
            validate_market_relay_information(&oversized),
            Err("relay information exceeds 65536 bytes".to_owned())
        );
    }

    #[test]
    fn relay_information_url_follows_the_websocket_scheme() {
        let plain = MarketDiscoveryConfig {
            relay_websocket_url: "ws://127.0.0.1:18080".to_owned(),
        };
        assert_eq!(
            plain.relay_information_url(),
            Ok("http://127.0.0.1:18080".to_owned())
        );
        let secure = MarketDiscoveryConfig {
            relay_websocket_url: "wss://relay.openagents.com".to_owned(),
        };
        assert_eq!(
            secure.relay_information_url(),
            Ok("https://relay.openagents.com".to_owned())
        );
        let invalid = MarketDiscoveryConfig {
            relay_websocket_url: "https://relay.openagents.com".to_owned(),
        };
        assert!(invalid.relay_information_url().is_err());
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn subscription_request_is_bounded_to_discovery_kinds() {
        let mut discovery = MarketDiscovery::new();
        discovery.begin_connect(&MarketRelayGate {
            max_limit: 100,
            ..passing_gate()
        });
        let request: Value = serde_json::from_str(&discovery.subscription_request())
            .expect("subscription request is JSON");
        assert_eq!(request[0], "REQ");
        assert_eq!(request[1], SUBSCRIPTION_ID);
        assert_eq!(
            request[2]["kinds"],
            json!([
                MKT_PROVIDER_PROFILE_KIND,
                MKT_OFFERING_KIND,
                MKT_SWP_KEY_ROTATION_KIND,
                MKT_SWP_RELAY_SET_KIND,
            ])
        );
        assert_eq!(request[2]["limit"], json!(100));
    }

    #[test]
    fn snapshot_becomes_visible_only_at_eose() {
        let signer = signer();
        let mut discovery = MarketDiscovery::new();
        discovery.begin_connect(&passing_gate());
        discovery.opened();

        let outcome = discovery
            .ingest_text(
                &event_frame(&provider_event(&signer, 1_700_000_000, "P")),
                1_700_000_100,
            )
            .expect("provider frame ingests");
        assert_eq!(outcome, IngestOutcome::AcceptedEvent);
        assert!(discovery.providers().is_empty());

        discovery
            .ingest_text(
                &event_frame(&offering_event(&signer, 1_700_000_000)),
                1_700_000_100,
            )
            .expect("offering frame ingests");
        discovery
            .ingest_text(&eose_frame(), 1_700_000_100)
            .expect("EOSE ingests");

        assert_eq!(discovery.connection(), &ConnectionState::Live);
        let providers = discovery.providers();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].provider_id, "prov1");
        assert_eq!(providers[0].status, "active");
        assert_eq!(providers[0].name.as_deref(), Some("P"));
        assert_eq!(providers[0].profiles, vec!["demo.compute:1".to_owned()]);
        let offerings = discovery.offerings();
        assert_eq!(offerings.len(), 1);
        assert_eq!(offerings[0].offering_id, "off1");
        assert_eq!(offerings[0].profile, "demo.compute:1");
        assert!(offerings[0].provider_address.ends_with(":prov1"));
    }

    #[test]
    fn tampered_signature_is_rejected_by_the_immortal_domain() {
        let signer = signer();
        let mut discovery = MarketDiscovery::new();
        discovery.begin_connect(&passing_gate());
        discovery.opened();

        let mut tampered = provider_event(&signer, 1_700_000_000, "P");
        tampered.content = json!({ "name": "forged" }).to_string();
        let outcome = discovery
            .ingest_text(&event_frame(&tampered), 1_700_000_100)
            .expect("frame parses");
        assert_eq!(outcome, IngestOutcome::RejectedEvent);
        discovery
            .ingest_text(&eose_frame(), 1_700_000_100)
            .expect("EOSE ingests");
        assert!(discovery.providers().is_empty());
        assert!(discovery.diagnostics().next().is_some());
    }

    #[test]
    fn newer_head_replaces_and_older_head_is_kept() {
        let signer = signer();
        let mut discovery = MarketDiscovery::new();
        discovery.begin_connect(&passing_gate());
        discovery.opened();
        discovery
            .ingest_text(
                &event_frame(&provider_event(&signer, 1_700_000_000, "old")),
                1_700_000_100,
            )
            .expect("first head ingests");
        discovery
            .ingest_text(&eose_frame(), 1_700_000_100)
            .expect("EOSE ingests");

        discovery
            .ingest_text(
                &event_frame(&provider_event(&signer, 1_700_000_050, "new")),
                1_700_000_100,
            )
            .expect("newer head ingests");
        assert_eq!(discovery.providers()[0].name.as_deref(), Some("new"));

        let stale = discovery
            .ingest_text(
                &event_frame(&provider_event(&signer, 1_699_999_000, "stale")),
                1_700_000_100,
            )
            .expect("older head ingests");
        assert_eq!(stale, IngestOutcome::Ignored);
        assert_eq!(discovery.providers()[0].name.as_deref(), Some("new"));
    }

    #[test]
    fn foreign_subscription_and_expired_records_are_excluded() {
        let signer = signer();
        let mut discovery = MarketDiscovery::new();
        discovery.begin_connect(&passing_gate());
        discovery.opened();

        let foreign = json!([
            "EVENT",
            "other",
            provider_event(&signer, 1_700_000_000, "P")
        ]);
        let outcome = discovery
            .ingest_text(&foreign.to_string(), 1_700_000_100)
            .expect("foreign frame parses");
        assert_eq!(outcome, IngestOutcome::Ignored);

        let expired = signer.sign(
            1_700_000_000,
            MKT_PROVIDER_PROFILE_KIND,
            vec![
                Tag::new(vec!["d".into(), "prov2".into()]),
                Tag::new(vec!["status".into(), "active".into()]),
                Tag::new(vec!["published_at".into(), "1700000000".into()]),
                Tag::new(vec!["profile".into(), "demo.compute".into(), "1".into()]),
                Tag::new(vec!["expiration".into(), "1700000010".into()]),
            ],
            json!({ "name": "expired" }).to_string(),
        );
        let outcome = discovery
            .ingest_text(&event_frame(&expired), 1_700_000_100)
            .expect("expired frame parses");
        assert_eq!(outcome, IngestOutcome::RejectedEvent);

        discovery
            .ingest_text(&eose_frame(), 1_700_000_100)
            .expect("EOSE ingests");
        assert!(discovery.providers().is_empty());
    }
}
