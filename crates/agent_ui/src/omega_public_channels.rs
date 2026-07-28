use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use url::Url;

pub const DESCRIPTOR_SCHEMA: &str = "openagents.public_channel_descriptor.v1";
pub const REGISTRY_SCHEMA: &str = "openagents.public_channel_registry.v1";
pub const AGENT_CHAT_MANIFEST_SCHEMA: &str = "openagents.public_nostr_chat_manifest.v1";
pub const OPENAGENTS_FIXTURE_REVISION: &str = "7b2b43b5b5b71288e39123509bb3d4b98276713a";
pub const OPENAGENTS_FIXTURE_SHA256: &str =
    "86f18dc884eb38b35404b8698b350b64aae4c36100965ff9415eef82d72a0195";

pub fn is_channel_activation_key(key: &str, modified: bool) -> bool {
    !modified && matches!(key, "enter" | "space")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentChatManifest {
    accepted_kinds: Vec<u16>,
    group: ManifestGroup,
    limits: ChannelLimits,
    profile_version: String,
    relay: ManifestRelay,
    rich_content_profile_version: String,
    schema_version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestGroup {
    id: String,
    state_kinds: Vec<u16>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestRelay {
    self_pubkey: Option<String>,
    websocket_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelLimits {
    pub attachment_count: usize,
    pub attachment_bytes: usize,
    pub content_bytes: usize,
    pub event_bytes: usize,
    pub future_skew_seconds: u64,
    pub history_page_size: usize,
    pub max_age_seconds: u64,
    pub tags: usize,
}

impl ChannelLimits {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.attachment_count > 0
                && self.attachment_bytes > 0
                && self.content_bytes > 0
                && self.event_bytes > 0
                && self.future_skew_seconds > 0
                && self.history_page_size > 0
                && self.max_age_seconds > 0
                && self.tags > 0,
            "channel limits must be positive"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelDescriptor {
    pub schema_version: String,
    pub channel_id: String,
    pub display_name: String,
    pub relay_url: String,
    pub group_id: String,
    pub accepted_kinds: Vec<u16>,
    pub group_state_kinds: Vec<u16>,
    pub moderation_kinds: Vec<u16>,
    pub expected_relay_self_pubkey: Option<String>,
    pub relay_trust: RelayTrust,
    pub limits: ChannelLimits,
    pub profile_version: String,
    pub rich_content_profile_version: String,
}

impl ChannelDescriptor {
    pub fn coordinate(&self) -> (&str, &str) {
        (&self.relay_url, &self.group_id)
    }

    pub fn destination_label(&self) -> String {
        format!("#{}", self.channel_id)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == DESCRIPTOR_SCHEMA,
            "unsupported channel descriptor schema"
        );
        ensure!(is_safe_id(&self.channel_id), "invalid channel id");
        ensure!(is_safe_id(&self.group_id), "invalid group id");
        ensure!(
            (1..=80).contains(&self.display_name.chars().count()),
            "invalid display name"
        );
        for value in [&self.profile_version, &self.rich_content_profile_version] {
            ensure!(
                (1..=120).contains(&value.chars().count()),
                "invalid profile version"
            );
        }
        self.limits.validate()?;
        let relay =
            Url::parse(&self.relay_url).map_err(|error| anyhow!("invalid relay URL: {error}"))?;
        ensure!(
            relay.scheme() == "wss"
                && relay.username().is_empty()
                && relay.password().is_none()
                && relay.query().is_none()
                && relay.fragment().is_none(),
            "unsafe relay URL"
        );
        let canonical_path = relay.path().trim_end_matches('/');
        let canonical_relay = if canonical_path.is_empty() {
            relay.origin().ascii_serialization()
        } else {
            format!("{}{}", relay.origin().ascii_serialization(), canonical_path)
        };
        ensure!(
            canonical_relay == self.relay_url,
            "relay URL is not canonical"
        );
        for kinds in [
            &self.accepted_kinds,
            &self.group_state_kinds,
            &self.moderation_kinds,
        ] {
            ensure!(
                !kinds.is_empty() && kinds.windows(2).all(|pair| pair[0] < pair[1]),
                "kind lists must be sorted and unique"
            );
        }
        ensure!(
            self.accepted_kinds
                .iter()
                .all(|kind| [5, 7, 9, 1337, 1984].contains(kind)),
            "unsupported accepted event kind"
        );
        ensure!(
            self.group_state_kinds
                .iter()
                .all(|kind| [39000, 39001, 39003, 39005].contains(kind)),
            "unsupported group-state event kind"
        );
        ensure!(
            self.moderation_kinds
                .iter()
                .all(|kind| [9002, 9005, 9010].contains(kind)),
            "unsupported moderation event kind"
        );
        if let Some(pubkey) = &self.expected_relay_self_pubkey {
            ensure!(
                pubkey.len() == 64
                    && pubkey
                        .chars()
                        .all(|character| character.is_ascii_hexdigit() && !character.is_uppercase()),
                "invalid relay self public key"
            );
        }
        ensure!(
            matches!(
                (&self.expected_relay_self_pubkey, &self.relay_trust),
                (Some(_), RelayTrust::Pinned) | (None, RelayTrust::MetadataUntrusted)
            ),
            "relay identity and trust state disagree"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelayTrust {
    Pinned,
    MetadataUntrusted,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelRegistry {
    pub schema_version: String,
    pub channels: Vec<ChannelDescriptor>,
}

impl ChannelRegistry {
    pub fn decode(json: &str) -> Result<Self> {
        let registry: Self = serde_json::from_str(json)?;
        ensure!(
            registry.schema_version == REGISTRY_SCHEMA,
            "unsupported channel registry schema"
        );
        ensure!(
            !registry.channels.is_empty() && registry.channels.len() <= 64,
            "a registry must contain between 1 and 64 channels"
        );
        let mut channel_ids = BTreeSet::new();
        let mut coordinates = BTreeSet::new();
        for channel in &registry.channels {
            channel.validate()?;
            ensure!(
                channel_ids.insert(channel.channel_id.clone()),
                "duplicate channel id"
            );
            ensure!(
                coordinates.insert((channel.relay_url.clone(), channel.group_id.clone())),
                "duplicate relay-qualified group coordinate"
            );
        }
        Ok(registry)
    }

    pub fn from_agent_chat_manifest(json: &str) -> Result<Self> {
        let mut manifest: AgentChatManifest = serde_json::from_str(json)?;
        ensure!(
            manifest.schema_version == AGENT_CHAT_MANIFEST_SCHEMA,
            "unsupported Agent Chat manifest schema"
        );
        manifest.accepted_kinds.sort_unstable();
        manifest.group.state_kinds.sort_unstable();
        let relay_trust = if manifest.relay.self_pubkey.is_some() {
            RelayTrust::Pinned
        } else {
            RelayTrust::MetadataUntrusted
        };
        let registry = Self {
            schema_version: REGISTRY_SCHEMA.to_string(),
            channels: vec![ChannelDescriptor {
                schema_version: DESCRIPTOR_SCHEMA.to_string(),
                channel_id: "agent-chat".to_string(),
                display_name: "Agent Chat".to_string(),
                relay_url: manifest.relay.websocket_url,
                group_id: manifest.group.id,
                accepted_kinds: manifest.accepted_kinds,
                group_state_kinds: manifest.group.state_kinds,
                moderation_kinds: vec![9002, 9005, 9010],
                expected_relay_self_pubkey: manifest.relay.self_pubkey,
                relay_trust,
                limits: manifest.limits,
                profile_version: manifest.profile_version,
                rich_content_profile_version: manifest.rich_content_profile_version,
            }],
        };
        Self::decode(&serde_json::to_string(&registry)?)
    }
}

fn is_safe_id(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    value.len() <= 64
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChannelLifecycle {
    #[default]
    Disconnected,
    Connecting,
    Replaying,
    Current,
    Reconnecting,
    Stale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicChannelSubscriptionPolicy {
    SelectedOnly,
}

impl ChannelLifecycle {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disconnected => "Not connected",
            Self::Connecting => "Connecting",
            Self::Replaying => "Loading history",
            Self::Current => "Live",
            Self::Reconnecting => "Reconnecting",
            Self::Stale => "Stale",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelCursor {
    pub created_at: i64,
    pub event_ids_at_created_at: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelSnapshot {
    pub relay_url: String,
    pub group_id: String,
    pub lifecycle: ChannelLifecycle,
    pub cursor: Option<ChannelCursor>,
    pub event_ids: BTreeSet<String>,
    pub cached: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelDestination {
    pub channel_id: String,
    pub label: String,
    pub relay_url: String,
    pub group_id: String,
    pub lifecycle: ChannelLifecycle,
    pub cached: bool,
    pub selected: bool,
    pub unread: usize,
}

impl ChannelDestination {
    pub fn accessible_label(&self) -> String {
        format!(
            "Open {} channel, {}{}{}",
            self.label,
            self.lifecycle.label(),
            if self.cached { ", cached" } else { "" },
            if self.unread == 0 {
                String::new()
            } else {
                format!(", {} unread", self.unread)
            }
        )
    }
}

#[derive(Clone, Debug)]
pub struct PublicChannelController {
    registry: ChannelRegistry,
    snapshots: BTreeMap<String, ChannelSnapshot>,
    unread: BTreeMap<String, usize>,
    selected: Option<String>,
}

impl PublicChannelController {
    pub const SUBSCRIPTION_POLICY: PublicChannelSubscriptionPolicy =
        PublicChannelSubscriptionPolicy::SelectedOnly;

    pub fn new(registry: ChannelRegistry) -> Self {
        let snapshots = registry
            .channels
            .iter()
            .map(|channel| {
                (
                    channel.channel_id.clone(),
                    ChannelSnapshot {
                        relay_url: channel.relay_url.clone(),
                        group_id: channel.group_id.clone(),
                        ..Default::default()
                    },
                )
            })
            .collect();
        Self {
            registry,
            snapshots,
            unread: BTreeMap::new(),
            selected: None,
        }
    }

    pub fn empty() -> Self {
        Self {
            registry: ChannelRegistry {
                schema_version: REGISTRY_SCHEMA.to_string(),
                channels: Vec::new(),
            },
            snapshots: BTreeMap::new(),
            unread: BTreeMap::new(),
            selected: None,
        }
    }

    pub fn selected_channel(&self) -> Option<&ChannelDescriptor> {
        let selected = self.selected.as_deref()?;
        self.registry
            .channels
            .iter()
            .find(|channel| channel.channel_id == selected)
    }

    pub fn selected_snapshot(&self) -> Option<&ChannelSnapshot> {
        self.selected
            .as_deref()
            .and_then(|selected| self.snapshots.get(selected))
    }

    pub fn select(&mut self, channel_id: &str) -> bool {
        if !self
            .registry
            .channels
            .iter()
            .any(|channel| channel.channel_id == channel_id)
        {
            return false;
        }
        if let Some(previous) = self.selected.as_deref()
            && previous != channel_id
            && let Some(snapshot) = self.snapshots.get_mut(previous)
        {
            snapshot.cached = snapshot.has_observed_state();
        }
        self.selected = Some(channel_id.to_string());
        self.unread.remove(channel_id);
        if let Some(snapshot) = self.snapshots.get_mut(channel_id) {
            snapshot.cached = false;
        }
        true
    }

    pub fn clear_selection(&mut self) {
        if let Some(selected) = self.selected.take()
            && let Some(snapshot) = self.snapshots.get_mut(&selected)
        {
            snapshot.cached = snapshot.has_observed_state();
        }
    }

    pub fn apply_snapshot(&mut self, channel_id: &str, mut snapshot: ChannelSnapshot) -> bool {
        let Some(channel) = self
            .registry
            .channels
            .iter()
            .find(|channel| channel.channel_id == channel_id)
        else {
            return false;
        };
        if snapshot.relay_url != channel.relay_url || snapshot.group_id != channel.group_id {
            return false;
        }
        let Some(previous) = self.snapshots.get(channel_id) else {
            return false;
        };
        let new_events = snapshot.event_ids.difference(&previous.event_ids).count();
        let selected = self.selected.as_deref() == Some(channel_id);
        snapshot.cached = !selected;
        self.snapshots.insert(channel_id.to_string(), snapshot);
        if !selected && new_events > 0 {
            *self.unread.entry(channel_id.to_string()).or_default() += new_events;
        }
        true
    }

    pub fn destinations(&self) -> Vec<ChannelDestination> {
        self.registry
            .channels
            .iter()
            .map(|channel| {
                let snapshot = self
                    .snapshots
                    .get(&channel.channel_id)
                    .cloned()
                    .unwrap_or_default();
                ChannelDestination {
                    channel_id: channel.channel_id.clone(),
                    label: channel.destination_label(),
                    relay_url: channel.relay_url.clone(),
                    group_id: channel.group_id.clone(),
                    lifecycle: snapshot.lifecycle,
                    cached: snapshot.cached,
                    selected: self.selected.as_deref() == Some(channel.channel_id.as_str()),
                    unread: self.unread.get(&channel.channel_id).copied().unwrap_or(0),
                }
            })
            .collect()
    }
}

impl ChannelSnapshot {
    fn has_observed_state(&self) -> bool {
        self.lifecycle != ChannelLifecycle::Disconnected
            || self.cursor.is_some()
            || !self.event_ids.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_channels() -> ChannelRegistry {
        ChannelRegistry::decode(include_str!("../fixtures/public-channel-registry.v1.json"))
            .expect("the exact OpenAgents two-channel fixture")
    }

    fn snapshot(
        relay_url: &str,
        lifecycle: ChannelLifecycle,
        ids: &[&str],
        created_at: i64,
    ) -> ChannelSnapshot {
        ChannelSnapshot {
            relay_url: relay_url.to_string(),
            group_id: "openagents-public".to_string(),
            lifecycle,
            cursor: Some(ChannelCursor {
                created_at,
                event_ids_at_created_at: ids.iter().map(|id| (*id).to_string()).collect(),
            }),
            event_ids: ids.iter().map(|id| (*id).to_string()).collect(),
            cached: false,
        }
    }

    #[test]
    fn current_manifest_adapts_to_one_production_agent_chat_channel() {
        let manifest = r#"{
          "acceptedKinds": [5, 7, 9, 1337, 1984],
          "group": {
            "id": "openagents-public",
            "stateKinds": [39000, 39001, 39003, 39005]
          },
          "limits": {
            "attachmentCount": 4,
            "attachmentBytes": 26214400,
            "contentBytes": 8192,
            "eventBytes": 32768,
            "futureSkewSeconds": 60,
            "historyPageSize": 50,
            "maxAgeSeconds": 604800,
            "tags": 64
          },
          "profileVersion": "openagents.public_chat.v1",
          "relay": {
            "selfPubkey": null,
            "websocketUrl": "wss://relay.openagents.com"
          },
          "richContentProfileVersion": "openagents.public_chat.rich_content.v1",
          "schemaVersion": "openagents.public_nostr_chat_manifest.v1"
        }"#;
        let registry =
            ChannelRegistry::from_agent_chat_manifest(manifest).expect("current manifest");
        assert_eq!(registry.channels.len(), 1);
        let channel = &registry.channels[0];
        assert_eq!(channel.channel_id, "agent-chat");
        assert_eq!(channel.relay_url, "wss://relay.openagents.com");
        assert_eq!(channel.group_id, "openagents-public");
        assert_eq!(channel.limits.history_page_size, 50);
        assert_eq!(channel.profile_version, "openagents.public_chat.v1");
        assert_eq!(
            OPENAGENTS_FIXTURE_REVISION,
            "7b2b43b5b5b71288e39123509bb3d4b98276713a"
        );
        assert_eq!(
            OPENAGENTS_FIXTURE_SHA256,
            "86f18dc884eb38b35404b8698b350b64aae4c36100965ff9415eef82d72a0195"
        );
    }

    #[test]
    fn equal_group_ids_on_different_relays_are_distinct_destinations() {
        let controller = PublicChannelController::new(two_channels());
        let destinations = controller.destinations();
        assert_eq!(destinations.len(), 2);
        assert_eq!(destinations[0].group_id, destinations[1].group_id);
        assert_ne!(destinations[0].relay_url, destinations[1].relay_url);
    }

    #[test]
    fn messages_never_create_destination_rows() {
        let mut controller = PublicChannelController::new(two_channels());
        let count = controller.destinations().len();
        assert!(controller.select("agent-chat"));
        assert!(controller.apply_snapshot(
            "agent-chat",
            snapshot(
                "wss://relay.openagents.com",
                ChannelLifecycle::Current,
                &["a", "b"],
                10
            )
        ));
        assert_eq!(controller.destinations().len(), count);
    }

    #[test]
    fn switching_keeps_independent_snapshots_cursors_lifecycle_and_unread() {
        let mut controller = PublicChannelController::new(two_channels());
        assert!(controller.select("agent-chat"));
        assert!(controller.apply_snapshot(
            "agent-chat",
            snapshot(
                "wss://relay.openagents.com",
                ChannelLifecycle::Current,
                &["a"],
                10
            )
        ));
        assert!(controller.apply_snapshot(
            "agent-lab",
            snapshot(
                "wss://relay.openagents.com/lab",
                ChannelLifecycle::Stale,
                &["b", "c"],
                20
            )
        ));
        let rows = controller.destinations();
        assert_eq!(rows[1].unread, 2);
        assert!(rows[1].cached);

        assert!(controller.select("agent-lab"));
        let rows = controller.destinations();
        assert!(rows[0].cached);
        assert!(!rows[1].cached);
        assert_eq!(rows[1].unread, 0);
        assert_eq!(
            controller
                .selected_snapshot()
                .and_then(|snapshot| snapshot.cursor.as_ref()),
            Some(&ChannelCursor {
                created_at: 20,
                event_ids_at_created_at: ["b".to_string(), "c".to_string()].into_iter().collect(),
            })
        );
        assert_eq!(
            controller
                .selected_snapshot()
                .map(|snapshot| snapshot.lifecycle),
            Some(ChannelLifecycle::Stale)
        );
        assert_eq!(
            PublicChannelController::SUBSCRIPTION_POLICY,
            PublicChannelSubscriptionPolicy::SelectedOnly
        );
    }

    #[test]
    fn duplicate_coordinates_and_empty_registries_fail_closed() {
        let mut duplicate = two_channels();
        duplicate.channels[1].relay_url = duplicate.channels[0].relay_url.clone();
        assert!(
            ChannelRegistry::decode(
                &serde_json::to_string(&duplicate).expect("serialize duplicate")
            )
            .is_err()
        );
        assert!(
            ChannelRegistry::decode(
                r#"{"schemaVersion":"openagents.public_channel_registry.v1","channels":[]}"#
            )
            .is_err()
        );

        let mut noncanonical = two_channels();
        noncanonical.channels[0].relay_url = "wss://RELAY.OPENAGENTS.COM:443/".to_string();
        assert!(
            ChannelRegistry::decode(
                &serde_json::to_string(&noncanonical).expect("serialize noncanonical")
            )
            .is_err()
        );

        let mut trust_mismatch = two_channels();
        trust_mismatch.channels[0].relay_trust = RelayTrust::Pinned;
        assert!(
            ChannelRegistry::decode(
                &serde_json::to_string(&trust_mismatch).expect("serialize trust mismatch")
            )
            .is_err()
        );

        let with_secret_field = include_str!("../fixtures/public-channel-registry.v1.json")
            .replacen(
                r#""schemaVersion": "openagents.public_channel_registry.v1","#,
                r#""schemaVersion": "openagents.public_channel_registry.v1", "privateKey": "no","#,
                1,
            );
        assert!(ChannelRegistry::decode(&with_secret_field).is_err());

        let mut unsafe_group = two_channels();
        unsafe_group.channels[0].group_id = "-unsafe".to_string();
        assert!(
            ChannelRegistry::decode(
                &serde_json::to_string(&unsafe_group).expect("serialize unsafe group")
            )
            .is_err()
        );

        let mut zero_limit = two_channels();
        zero_limit.channels[0].limits.history_page_size = 0;
        assert!(
            ChannelRegistry::decode(
                &serde_json::to_string(&zero_limit).expect("serialize zero limit")
            )
            .is_err()
        );
    }

    #[test]
    fn manifest_adapter_requires_the_versioned_source_schema() {
        let json = r#"{
          "acceptedKinds": [9],
          "group": {"id": "openagents-public", "stateKinds": [39005]},
          "limits": {
            "attachmentCount": 1,
            "attachmentBytes": 1,
            "contentBytes": 1,
            "eventBytes": 1,
            "futureSkewSeconds": 1,
            "historyPageSize": 1,
            "maxAgeSeconds": 1,
            "tags": 1
          },
          "profileVersion": "profile",
          "relay": {"selfPubkey": null, "websocketUrl": "wss://relay.example.com"},
          "richContentProfileVersion": "rich",
          "schemaVersion": "openagents.public_nostr_chat_manifest.v0"
        }"#;
        assert!(ChannelRegistry::from_agent_chat_manifest(json).is_err());
    }

    #[test]
    fn switching_does_not_claim_an_unobserved_snapshot_is_cached() {
        let mut controller = PublicChannelController::new(two_channels());
        assert!(controller.select("agent-chat"));
        assert!(controller.select("agent-lab"));
        assert!(!controller.destinations()[0].cached);
        controller.clear_selection();
        assert!(!controller.destinations()[1].cached);
    }

    #[test]
    fn coordinate_mismatch_is_rejected_and_duplicate_events_do_not_add_unread() {
        let mut controller = PublicChannelController::new(two_channels());
        let mismatched = snapshot(
            "wss://relay.openagents.com",
            ChannelLifecycle::Current,
            &["a"],
            10,
        );
        assert!(!controller.apply_snapshot("agent-lab", mismatched));

        let first = snapshot(
            "wss://relay.openagents.com/lab",
            ChannelLifecycle::Current,
            &["a"],
            10,
        );
        assert!(controller.apply_snapshot("agent-lab", first.clone()));
        assert!(controller.apply_snapshot("agent-lab", first));
        assert_eq!(controller.destinations()[1].unread, 1);
    }

    #[test]
    fn unknown_channel_selection_does_not_change_the_selected_destination() {
        let mut controller = PublicChannelController::new(two_channels());
        assert!(controller.select("agent-chat"));
        assert!(!controller.select("missing"));
        assert_eq!(
            controller
                .selected_channel()
                .map(|channel| channel.channel_id.as_str()),
            Some("agent-chat")
        );
    }

    #[test]
    fn destination_accessibility_label_reports_state_and_unread() {
        let mut controller = PublicChannelController::new(two_channels());
        let snapshot = snapshot(
            "wss://relay.openagents.com/lab",
            ChannelLifecycle::Stale,
            &["a", "b"],
            10,
        );
        assert!(controller.apply_snapshot("agent-lab", snapshot));
        assert_eq!(
            controller.destinations()[1].accessible_label(),
            "Open #agent-lab channel, Stale, cached, 2 unread"
        );
    }

    #[test]
    fn enter_and_space_activate_a_focused_channel_without_modifiers() {
        assert!(is_channel_activation_key("enter", false));
        assert!(is_channel_activation_key("space", false));
        assert!(!is_channel_activation_key("enter", true));
        assert!(!is_channel_activation_key("down", false));
    }
}
