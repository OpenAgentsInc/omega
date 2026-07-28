use std::collections::{BTreeMap, BTreeSet};

use nostr::JsonUtil as _;
use serde::{Deserialize, Serialize};
use url::Url;

pub const OPENAGENTS_PARITY_FIXTURE_REVISION: &str = "3d7c49d4fdc3215802707088242e709dbe902932";
pub const OPENAGENTS_PARITY_FIXTURE_SHA256: &str =
    "33a55a30e444ed7f05d65d581ccafeaa7d082314b4a95f4ea5f016040a782595";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct NostrEventRecord {
    pub content: String,
    pub created_at: i64,
    pub id: String,
    pub kind: u16,
    #[serde(rename = "pubkey")]
    pub public_key: String,
    #[serde(rename = "sig")]
    pub signature: String,
    pub tags: Vec<Vec<String>>,
}

impl NostrEventRecord {
    pub fn is_verified(&self) -> bool {
        serde_json::to_string(self)
            .ok()
            .and_then(|json| nostr::Event::from_json(json).ok())
            .is_some_and(|event| event.verify().is_ok())
    }

    pub fn has_tag(&self, name: &str, value: &str) -> bool {
        self.tags.iter().any(|tag| {
            tag.first().map(String::as_str) == Some(name)
                && tag.get(1).map(String::as_str) == Some(value)
        })
    }

    pub fn tag_values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.tags.iter().filter_map(move |tag| {
            (tag.first().map(String::as_str) == Some(name))
                .then(|| tag.get(1).map(String::as_str))
                .flatten()
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
pub enum ContentPart {
    Text(String),
    HttpLink(String),
    NostrReference(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeletionKind {
    Author,
    Moderator,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileFact {
    pub bot: bool,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReactionCount {
    pub count: usize,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaFact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blurhash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<String>,
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    pub url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waveform: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineRow {
    pub event_id: String,
    pub public_key: String,
    pub created_at: i64,
    pub kind: u16,
    pub content_parts: Vec<ContentPart>,
    pub content_warning: bool,
    pub deletion: Option<DeletionKind>,
    pub pinned: bool,
    pub profile: Option<ProfileFact>,
    pub reactions: Vec<ReactionCount>,
    pub media: Vec<MediaFact>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimelineProjection {
    pub rows: Vec<TimelineRow>,
    pub administrators: BTreeSet<String>,
    pub pinned_event_ids: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureState {
    Verified,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventFacts {
    pub public_key: String,
    pub event_id: String,
    pub kind: u16,
    pub relay_url: String,
    pub group_id: String,
    pub signature_state: SignatureState,
    pub created_at: i64,
    pub pinned: bool,
    pub deletion: Option<DeletionKind>,
    pub media: Vec<MediaFact>,
}

pub fn event_facts(row: &TimelineRow, relay_url: &str, group_id: &str) -> EventFacts {
    EventFacts {
        public_key: row.public_key.clone(),
        event_id: row.event_id.clone(),
        kind: row.kind,
        relay_url: relay_url.to_string(),
        group_id: group_id.to_string(),
        signature_state: SignatureState::Verified,
        created_at: row.created_at,
        pinned: row.pinned,
        deletion: row.deletion,
        media: row.media.clone(),
    }
}

pub fn stable_verified_events(events: &[NostrEventRecord]) -> Vec<NostrEventRecord> {
    let mut unique = BTreeMap::new();
    for event in events {
        if event.is_verified() {
            unique
                .entry(event.id.clone())
                .or_insert_with(|| event.clone());
        }
    }
    let mut events: Vec<_> = unique.into_values().collect();
    events.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    events
}

pub fn project_timeline(
    events: &[NostrEventRecord],
    group_id: &str,
    relay_self_public_key: Option<&str>,
) -> TimelineProjection {
    let events = stable_verified_events(events);
    let current_administrator_state =
        current_relay_state(&events, 39001, group_id, relay_self_public_key);
    let administrators = current_administrator_state
        .iter()
        .flat_map(|event| event.tag_values("p"))
        .filter(|public_key| is_lower_hex(public_key, 64))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let current_pin_state = current_relay_state(&events, 39005, group_id, relay_self_public_key);
    let pinned_event_ids = current_pin_state
        .iter()
        .flat_map(|event| event.tag_values("e"))
        .filter(|event_id| is_lower_hex(event_id, 64))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();

    let mut profiles = BTreeMap::<String, ProfileFact>::new();
    for event in events.iter().filter(|event| event.kind == 0) {
        if let Some(profile) = parse_profile(&event.content) {
            profiles.insert(event.public_key.clone(), profile);
        }
    }

    let message_ids = events
        .iter()
        .filter(|event| matches!(event.kind, 9 | 1337) && event.has_tag("h", group_id))
        .map(|event| event.id.clone())
        .collect::<BTreeSet<_>>();
    let mut deletions = BTreeMap::<String, DeletionKind>::new();
    for deletion in events
        .iter()
        .filter(|event| event.kind == 5 && event.has_tag("h", group_id))
    {
        for target_id in deletion.tag_values("e") {
            let Some(target) = events.iter().find(|event| event.id == target_id) else {
                continue;
            };
            if message_ids.contains(target_id) && deletion.public_key == target.public_key {
                deletions.insert(target_id.to_string(), DeletionKind::Author);
            }
        }
    }
    for deletion in events.iter().filter(|event| {
        event.kind == 9005
            && event.has_tag("h", group_id)
            && administrators.contains(&event.public_key)
    }) {
        for target_id in deletion.tag_values("e") {
            if message_ids.contains(target_id) {
                deletions.insert(target_id.to_string(), DeletionKind::Moderator);
            }
        }
    }

    let retained_ids = events
        .iter()
        .map(|event| event.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut reactions = BTreeMap::<String, Vec<ReactionCount>>::new();
    for reaction in events
        .iter()
        .filter(|event| event.kind == 7 && event.has_tag("h", group_id))
    {
        let Some(target_id) = reaction.tag_values("e").next() else {
            continue;
        };
        if !retained_ids.contains(target_id) {
            continue;
        }
        let target_reactions = reactions.entry(target_id.to_string()).or_default();
        if let Some(existing) = target_reactions
            .iter_mut()
            .find(|entry| entry.value == reaction.content)
        {
            existing.count = existing.count.saturating_add(1);
        } else {
            target_reactions.push(ReactionCount {
                count: 1,
                value: reaction.content.clone(),
            });
        }
    }

    let rows = events
        .iter()
        .filter(|event| matches!(event.kind, 9 | 1337) && event.has_tag("h", group_id))
        .map(|event| TimelineRow {
            event_id: event.id.clone(),
            public_key: event.public_key.clone(),
            created_at: event.created_at,
            kind: event.kind,
            content_parts: parse_content_parts(&event.content),
            content_warning: event
                .tags
                .iter()
                .any(|tag| tag.first().map(String::as_str) == Some("content-warning")),
            deletion: deletions.get(&event.id).copied(),
            pinned: pinned_event_ids.contains(&event.id),
            profile: profiles.get(&event.public_key).cloned(),
            reactions: reactions.remove(&event.id).unwrap_or_default(),
            media: parse_inline_media(event),
        })
        .collect();

    TimelineProjection {
        rows,
        administrators,
        pinned_event_ids,
    }
}

fn current_relay_state<'a>(
    events: &'a [NostrEventRecord],
    kind: u16,
    group_id: &str,
    relay_self_public_key: Option<&str>,
) -> Option<&'a NostrEventRecord> {
    let relay_self_public_key = relay_self_public_key?;
    events
        .iter()
        .filter(|event| {
            event.kind == kind
                && event.public_key == relay_self_public_key
                && event.has_tag("d", group_id)
        })
        .max_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        })
}

fn parse_profile(content: &str) -> Option<ProfileFact> {
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let object = value.as_object()?;
    let display_name = object
        .get("display_name")
        .and_then(serde_json::Value::as_str)
        .or_else(|| object.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_string);
    Some(ProfileFact {
        bot: object.get("bot").and_then(serde_json::Value::as_bool) == Some(true),
        display_name,
    })
}

pub fn parse_content_parts(content: &str) -> Vec<ContentPart> {
    let mut parts = Vec::new();
    let mut offset = 0;
    while offset < content.len() {
        let Some((start, reference)) = next_reference(content, offset) else {
            parts.push(ContentPart::Text(content[offset..].to_string()));
            break;
        };
        if start > offset {
            parts.push(ContentPart::Text(content[offset..start].to_string()));
        }
        let end = match reference {
            ReferenceKind::Http => content[start..]
                .char_indices()
                .find_map(|(index, character)| character.is_whitespace().then_some(start + index))
                .unwrap_or(content.len()),
            ReferenceKind::Nostr => {
                let prefix_end = start + "nostr:".len();
                content[prefix_end..]
                    .char_indices()
                    .find_map(|(index, character)| {
                        (!character.is_ascii_alphanumeric()).then_some(prefix_end + index)
                    })
                    .unwrap_or(content.len())
            }
        };
        let value = content[start..end].to_string();
        parts.push(match reference {
            ReferenceKind::Http => ContentPart::HttpLink(value),
            ReferenceKind::Nostr => ContentPart::NostrReference(value),
        });
        offset = end;
    }
    if content.is_empty() {
        Vec::new()
    } else {
        parts
    }
}

#[derive(Clone, Copy)]
enum ReferenceKind {
    Http,
    Nostr,
}

fn next_reference(content: &str, offset: usize) -> Option<(usize, ReferenceKind)> {
    content[offset..].char_indices().find_map(|(relative, _)| {
        let start = offset + relative;
        let tail = &content[start..];
        if starts_with_ascii_case_insensitive(tail, "https://")
            || starts_with_ascii_case_insensitive(tail, "http://")
        {
            Some((start, ReferenceKind::Http))
        } else if starts_with_ascii_case_insensitive(tail, "nostr:")
            && tail["nostr:".len()..]
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric())
        {
            Some((start, ReferenceKind::Nostr))
        } else {
            None
        }
    })
}

fn starts_with_ascii_case_insensitive(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

fn parse_inline_media(event: &NostrEventRecord) -> Vec<MediaFact> {
    let content_urls = event
        .content
        .split_whitespace()
        .filter(|value| is_safe_http_url(value))
        .map(|value| {
            value
                .trim_end_matches([',', ')', '.', ';', '!', '?'])
                .to_string()
        })
        .collect::<BTreeSet<_>>();
    event
        .tags
        .iter()
        .filter(|tag| tag.first().map(String::as_str) == Some("imeta"))
        .filter_map(|tag| {
            let fields = tag
                .iter()
                .skip(1)
                .map(|field| {
                    field
                        .split_once(' ')
                        .map_or((field.as_str(), ""), |(name, value)| (name, value))
                })
                .collect::<BTreeMap<_, _>>();
            let url = fields.get("url").copied()?;
            let mime_type = fields.get("m").copied()?;
            if !content_urls.contains(url) || !is_safe_media_mime(mime_type) {
                return None;
            }
            let size = match fields.get("size") {
                Some(value) => {
                    let value = value.parse::<u64>().ok()?;
                    if value > 25 * 1024 * 1024 {
                        return None;
                    }
                    Some(value)
                }
                None => None,
            };
            Some(MediaFact {
                alt: fields.get("alt").map(|value| (*value).to_string()),
                blurhash: fields.get("blurhash").map(|value| (*value).to_string()),
                dimensions: fields.get("dim").map(|value| (*value).to_string()),
                digest: fields
                    .get("x")
                    .filter(|value| is_lower_hex(value, 64))
                    .map(|value| (*value).to_string()),
                duration_seconds: fields.get("duration").map(|value| (*value).to_string()),
                mime_type: mime_type.to_string(),
                size,
                thumbnail_url: fields.get("thumb").map(|value| (*value).to_string()),
                url: url.to_string(),
                waveform: fields
                    .get("waveform")
                    .map(|value| {
                        value
                            .split_whitespace()
                            .take(256)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .take(4)
        .collect()
}

fn is_safe_http_url(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .is_some_and(|url| matches!(url.scheme(), "http" | "https"))
}

fn is_safe_media_mime(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "image/avif"
            | "image/gif"
            | "image/jpeg"
            | "image/png"
            | "image/webp"
            | "audio/aac"
            | "audio/flac"
            | "audio/mpeg"
            | "audio/mp4"
            | "audio/ogg"
            | "audio/wav"
            | "audio/webm"
            | "video/mp4"
            | "video/ogg"
            | "video/webm"
            | "application/json"
            | "application/pdf"
            | "text/csv"
            | "text/plain"
    )
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fixture {
        projection: FixtureProjection,
        schema_version: String,
        source: FixtureSource,
    }

    #[derive(Deserialize)]
    struct FixtureProjection {
        events: Vec<NostrEventRecord>,
        #[serde(rename = "expectedTimeline")]
        expected_timeline: Vec<ExpectedTimelineRow>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureSource {
        group_id: String,
        relay_url: String,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "camelCase")]
    struct ExpectedTimelineRow {
        attachments: Vec<MediaFact>,
        content_parts: Vec<ContentPart>,
        content_warning: bool,
        deletion: Option<DeletionKind>,
        event_id: String,
        kind: u16,
        pinned: bool,
        profile: Option<ProfileFact>,
        reactions: Vec<ReactionCount>,
    }

    fn fixture() -> Fixture {
        serde_json::from_str(include_str!("../fixtures/agent-chat-parity.v1.json"))
            .expect("the pinned OpenAgents fixture must decode")
    }

    fn expected_row(row: &TimelineRow) -> ExpectedTimelineRow {
        ExpectedTimelineRow {
            attachments: row.media.clone(),
            content_parts: row.content_parts.clone(),
            content_warning: row.content_warning,
            deletion: row.deletion,
            event_id: row.event_id.clone(),
            kind: row.kind,
            pinned: row.pinned,
            profile: row.profile.clone(),
            reactions: row.reactions.clone(),
        }
    }

    fn signed_event(
        keys: &Keys,
        kind: u16,
        content: &str,
        created_at: u64,
        tags: &[&[&str]],
    ) -> NostrEventRecord {
        let tags = tags
            .iter()
            .map(|parts| Tag::parse(parts.iter().copied()).expect("valid test tag"))
            .collect::<Vec<_>>();
        let event = EventBuilder::new(Kind::Custom(kind), content)
            .custom_created_at(Timestamp::from_secs(created_at))
            .tags(tags)
            .sign_with_keys(keys)
            .expect("test event must sign");
        serde_json::from_value(serde_json::to_value(event).expect("event JSON"))
            .expect("event record")
    }

    #[test]
    fn pinned_fixture_projects_exactly_and_is_arrival_order_independent() {
        let fixture = fixture();
        assert_eq!(
            fixture.schema_version,
            "openagents.agent_chat_parity_fixture.v1"
        );
        assert!(
            fixture
                .projection
                .events
                .iter()
                .all(NostrEventRecord::is_verified)
        );
        let relay_self = fixture
            .projection
            .events
            .iter()
            .find(|event| event.kind == 39001)
            .map(|event| event.public_key.as_str());
        let projected = project_timeline(
            &fixture.projection.events,
            &fixture.source.group_id,
            relay_self,
        );
        let actual = projected.rows.iter().map(expected_row).collect::<Vec<_>>();
        assert_eq!(actual, fixture.projection.expected_timeline);

        let mut reversed = fixture.projection.events.clone();
        reversed.reverse();
        let reversed = project_timeline(&reversed, &fixture.source.group_id, relay_self);
        assert_eq!(projected, reversed);
        assert_eq!(
            event_facts(
                projected.rows.last().expect("fixture row"),
                &fixture.source.relay_url,
                &fixture.source.group_id,
            )
            .signature_state,
            SignatureState::Verified
        );
    }

    #[test]
    fn only_current_addressable_admin_and_pin_state_applies() {
        let relay = Keys::generate();
        let old_administrator = Keys::generate();
        let current_administrator = Keys::generate();
        let author = Keys::generate();
        let group = "test-group";
        let first_message = signed_event(&author, 9, "first", 10, &[&["h", group]]);
        let second_message = signed_event(&author, 9, "second", 11, &[&["h", group]]);
        let old_administrator_key = old_administrator.public_key().to_hex();
        let current_administrator_key = current_administrator.public_key().to_hex();
        let first_id = first_message.id.clone();
        let second_id = second_message.id.clone();
        let events = vec![
            first_message,
            second_message,
            signed_event(
                &relay,
                39001,
                "",
                12,
                &[&["d", group], &["p", old_administrator_key.as_str()]],
            ),
            signed_event(
                &relay,
                39005,
                "",
                13,
                &[&["d", group], &["e", first_id.as_str()]],
            ),
            signed_event(
                &relay,
                39001,
                "",
                14,
                &[&["d", group], &["p", current_administrator_key.as_str()]],
            ),
            signed_event(
                &relay,
                39005,
                "",
                15,
                &[&["d", group], &["e", second_id.as_str()]],
            ),
            signed_event(
                &old_administrator,
                9005,
                "",
                16,
                &[&["h", group], &["e", first_id.as_str()]],
            ),
            signed_event(
                &current_administrator,
                9005,
                "",
                17,
                &[&["h", group], &["e", second_id.as_str()]],
            ),
        ];

        let projection =
            project_timeline(&events, group, Some(relay.public_key().to_hex().as_str()));
        assert_eq!(
            projection.administrators,
            BTreeSet::from([current_administrator_key])
        );
        assert_eq!(projection.pinned_event_ids, BTreeSet::from([second_id]));
        assert_eq!(projection.rows[0].deletion, None);
        assert!(!projection.rows[0].pinned);
        assert_eq!(projection.rows[1].deletion, Some(DeletionKind::Moderator));
        assert!(projection.rows[1].pinned);
    }

    #[test]
    fn unsafe_schemes_remain_inert_text() {
        assert_eq!(
            parse_content_parts("safe https://example.test nostr:nevent1test javascript:alert(1)"),
            vec![
                ContentPart::Text("safe ".into()),
                ContentPart::HttpLink("https://example.test".into()),
                ContentPart::Text(" ".into()),
                ContentPart::NostrReference("nostr:nevent1test".into()),
                ContentPart::Text(" javascript:alert(1)".into()),
            ]
        );
    }
}
