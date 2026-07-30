//! Scoped writes for Omega's alpha tester channel.
//!
//! Secret key material remains inside `omega_identity`. This module prepares
//! bounded public events, asks the identity service to sign them, verifies the
//! returned event, and hands those exact signed bytes to the existing NIP-42
//! relay transport.

use std::{
    collections::BTreeSet,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, anyhow, ensure};
use nostr::{Event, JsonUtil as _};
use omega_identity::{
    AdmittedSigningRequest, IdentityService, ReceiptRef, SigningPurpose, UnsignedEventTemplate,
};
use sha2::{Digest as _, Sha256};

use crate::{
    omega_public_channel_timeline::NostrEventRecord, omega_public_channels::ChannelDescriptor,
};

pub const CHAT_MESSAGE_KIND: u16 = 9;
pub const REPORT_KIND: u16 = 1984;
const MAX_PUBLISH_ATTEMPTS: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublicChannelWrite {
    Message {
        content: String,
    },
    Report {
        event_id: String,
        author_public_key: String,
    },
}

impl PublicChannelWrite {
    fn kind(&self) -> u16 {
        match self {
            Self::Message { .. } => CHAT_MESSAGE_KIND,
            Self::Report { .. } => REPORT_KIND,
        }
    }

    fn content(&self) -> &str {
        match self {
            Self::Message { content } => content,
            Self::Report { .. } => "",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SignedPublicChannelWrite {
    event: Event,
    record: NostrEventRecord,
    author_public_key: String,
    is_report: bool,
}

impl SignedPublicChannelWrite {
    pub fn record(&self) -> &NostrEventRecord {
        &self.record
    }

    pub fn is_report(&self) -> bool {
        self.is_report
    }
}

pub fn previous_references(events: &[NostrEventRecord], own_public_key: &str) -> Vec<String> {
    let mut candidates = events
        .iter()
        .filter(|event| {
            event.public_key != own_public_key
                && event.is_verified()
                && matches!(event.kind, CHAT_MESSAGE_KIND | 1337)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .take(50)
        .filter_map(|event| event.id.get(..8).map(str::to_owned))
        .filter(|prefix| seen.insert(prefix.clone()))
        .take(3)
        .collect()
}

pub fn event_template(
    descriptor: &ChannelDescriptor,
    write: &PublicChannelWrite,
    events: &[NostrEventRecord],
    own_public_key: &str,
    created_at: u64,
) -> Result<UnsignedEventTemplate> {
    let kind = write.kind();
    ensure!(
        descriptor.accepted_kinds.contains(&kind),
        "the tester channel does not accept this event kind"
    );
    let content = write.content().trim();
    if matches!(write, PublicChannelWrite::Message { .. }) {
        ensure!(!content.is_empty(), "write a message before sending");
        ensure!(
            content.len() <= descriptor.limits.content_bytes,
            "the message exceeds the channel's {} byte limit",
            descriptor.limits.content_bytes
        );
    }

    let mut tags = vec![vec!["h".to_string(), descriptor.group_id.clone()]];
    match write {
        PublicChannelWrite::Message { .. } => {
            let previous = previous_references(events, own_public_key);
            if !previous.is_empty() {
                let mut tag = vec!["previous".to_string()];
                tag.extend(previous);
                tags.push(tag);
            }
        }
        PublicChannelWrite::Report {
            event_id,
            author_public_key,
        } => {
            ensure!(is_hex_64(event_id), "the report target event id is invalid");
            ensure!(
                is_hex_64(author_public_key),
                "the report target public key is invalid"
            );
            tags.push(vec![
                "e".to_string(),
                event_id.clone(),
                descriptor.relay_url.clone(),
                "other".to_string(),
            ]);
            tags.push(vec![
                "p".to_string(),
                author_public_key.clone(),
                String::new(),
                "other".to_string(),
            ]);
        }
    }
    ensure!(
        tags.len() <= descriptor.limits.tags,
        "the event exceeds the channel's tag limit"
    );

    Ok(UnsignedEventTemplate {
        created_at,
        kind,
        tags,
        content: content.to_string(),
    })
}

pub fn sign_write(
    identity_service: &IdentityService,
    descriptor: &ChannelDescriptor,
    write: PublicChannelWrite,
    events: &[NostrEventRecord],
) -> Result<SignedPublicChannelWrite> {
    let provision_receipt = ReceiptRef::new("omega.tester-channel.identity.v1")?;
    let custody = identity_service
        .provision_unattended(provision_receipt)
        .context("preparing the Omega identity")?;
    let identity = custody
        .identity
        .ok_or_else(|| anyhow!("the Omega identity is not ready"))?;
    let created_at = unix_time_seconds()?;
    if let PublicChannelWrite::Report {
        event_id,
        author_public_key,
    } = &write
    {
        ensure!(
            events.iter().any(|event| {
                event.id == *event_id
                    && event.public_key == *author_public_key
                    && event.is_verified()
                    && matches!(event.kind, CHAT_MESSAGE_KIND | 1337)
                    && event.has_tag("h", &descriptor.group_id)
            }),
            "the report target is not a verified message in this tester channel"
        );
    }
    let event = event_template(
        descriptor,
        &write,
        events,
        identity.public_key_hex().as_str(),
        created_at,
    )?;
    let request_ref = signing_receipt(&identity, &event)?;
    let signed = identity_service
        .sign(&AdmittedSigningRequest {
            request_ref,
            identity_ref: identity.identity_ref().clone(),
            purpose: SigningPurpose::NostrEvent,
            event,
        })
        .context("signing the public tester-channel event")?;
    let event = Event::from_json(&signed.signed_event_json)
        .context("decoding the signed tester-channel event")?;
    event
        .verify()
        .context("verifying the signed tester-channel event")?;
    ensure!(
        event.id.to_hex() == signed.event_id,
        "the identity service returned a different event"
    );
    let record = serde_json::from_str::<NostrEventRecord>(&signed.signed_event_json)
        .context("projecting the signed tester-channel event")?;
    ensure!(
        record.is_verified(),
        "the projected tester-channel event is invalid"
    );

    Ok(SignedPublicChannelWrite {
        event,
        record,
        author_public_key: identity.public_key_hex().as_str().to_string(),
        is_report: matches!(write, PublicChannelWrite::Report { .. }),
    })
}

pub fn publish_signed_write(
    descriptor: &ChannelDescriptor,
    signed: &SignedPublicChannelWrite,
) -> Result<()> {
    publish_signed_write_with(descriptor, signed, |relay_url, event| {
        Ok(omega_effectd::publish_community_event(
            vec![relay_url.to_string()],
            event,
        )?)
    })
}

fn publish_signed_write_with(
    descriptor: &ChannelDescriptor,
    signed: &SignedPublicChannelWrite,
    mut publish: impl FnMut(&str, &Event) -> Result<()>,
) -> Result<()> {
    descriptor.validate()?;
    ensure!(
        matches!(
            descriptor.relay_trust,
            crate::omega_public_channels::RelayTrust::Pinned
        ) && descriptor.expected_relay_self_pubkey.is_some(),
        "the tester relay identity is not pinned"
    );
    ensure!(
        signed.record.public_key == signed.author_public_key,
        "the signed event author does not match the Omega custody identity"
    );
    ensure!(
        matches!(signed.event.kind.as_u16(), CHAT_MESSAGE_KIND | REPORT_KIND),
        "the signed event kind is outside the tester-channel writer"
    );
    ensure!(
        descriptor
            .accepted_kinds
            .contains(&signed.event.kind.as_u16()),
        "the signed event kind is not accepted by this channel"
    );
    match signed.event.kind.as_u16() {
        CHAT_MESSAGE_KIND => ensure!(
            !signed.record.content.trim().is_empty(),
            "the signed tester-channel message is empty"
        ),
        REPORT_KIND => {
            let event_targets = signed
                .record
                .tags
                .iter()
                .filter(|tag| tag.first().map(String::as_str) == Some("e"))
                .collect::<Vec<_>>();
            let author_targets = signed
                .record
                .tags
                .iter()
                .filter(|tag| tag.first().map(String::as_str) == Some("p"))
                .collect::<Vec<_>>();
            let ([event_target], [author_target]) =
                (event_targets.as_slice(), author_targets.as_slice())
            else {
                return Err(anyhow!(
                    "the signed tester-channel report target is invalid"
                ));
            };
            ensure!(
                signed.record.content.is_empty()
                    && event_target.get(1).is_some_and(|value| is_hex_64(value))
                    && event_target.get(2).map(String::as_str)
                        == Some(descriptor.relay_url.as_str())
                    && event_target.get(3).map(String::as_str) == Some("other")
                    && author_target.get(1).is_some_and(|value| is_hex_64(value))
                    && author_target.get(2).map(String::as_str) == Some("")
                    && author_target.get(3).map(String::as_str) == Some("other"),
                "the signed tester-channel report target is invalid"
            );
        }
        _ => {
            return Err(anyhow!(
                "the signed event kind is outside the tester-channel writer"
            ));
        }
    }
    ensure!(
        signed
            .record
            .tags
            .iter()
            .filter(|tag| tag.first().map(String::as_str) == Some("h"))
            .count()
            == 1
            && signed.record.has_tag("h", &descriptor.group_id),
        "the signed event is not bound to the exact tester group"
    );
    ensure!(
        signed.record.content.len() <= descriptor.limits.content_bytes
            && signed.record.tags.len() <= descriptor.limits.tags
            && signed.event.as_json().len() <= descriptor.limits.event_bytes,
        "the signed event exceeds the tester-channel limits"
    );
    signed
        .event
        .verify()
        .context("verifying the event immediately before publication")?;
    let mut last_error = None;
    for _ in 0..MAX_PUBLISH_ATTEMPTS {
        match publish(&descriptor.relay_url, &signed.event) {
            Ok(()) => return Ok(()),
            Err(error) if error.to_string().to_ascii_lowercase().contains("duplicate") => {
                return Err(anyhow!(
                    "the relay reported a duplicate event; the identical signed event may already be present, so check the timeline before retrying"
                ));
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(anyhow!(
        "{}",
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "the relay did not acknowledge the event".to_string())
    ))
}

fn signing_receipt(
    identity: &omega_identity::PublicIdentity,
    event: &UnsignedEventTemplate,
) -> Result<ReceiptRef> {
    let canonical = serde_json::to_vec(&(
        identity.identity_ref().as_str(),
        event.created_at,
        event.kind,
        &event.tags,
        &event.content,
    ))?;
    let digest = format!("{:x}", Sha256::digest(canonical));
    ReceiptRef::new(format!("omega.tester-channel.{}", &digest[..32])).map_err(Into::into)
}

fn is_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
}

fn unix_time_seconds() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .context("reading the system clock for the public event")
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_identity::AppChannel;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    fn descriptor() -> ChannelDescriptor {
        crate::omega_public_channels::bundled_tester_registry()
            .expect("bundled registry")
            .channels
            .into_iter()
            .next()
            .expect("alpha feedback channel")
    }

    fn event(keys: &Keys, id_seed: u64, created_at: u64) -> NostrEventRecord {
        let event = EventBuilder::new(Kind::Custom(CHAT_MESSAGE_KIND), id_seed.to_string())
            .custom_created_at(Timestamp::from_secs(created_at))
            .tags(vec![
                Tag::parse(["h", "openagents-public"]).expect("group tag"),
            ])
            .sign_with_keys(keys)
            .expect("signed fixture event");
        serde_json::from_str(&event.as_json()).expect("event record")
    }

    #[test]
    fn message_template_binds_group_limits_and_three_foreign_predecessors() {
        let own = Keys::generate();
        let other = Keys::generate();
        let events = (0..6)
            .map(|index| {
                let keys = if index == 5 { &own } else { &other };
                event(keys, index, 100 + index)
            })
            .collect::<Vec<_>>();
        let template = event_template(
            &descriptor(),
            &PublicChannelWrite::Message {
                content: "alpha feedback".to_string(),
            },
            &events,
            &own.public_key().to_hex(),
            200,
        )
        .expect("message template");
        assert_eq!(template.kind, CHAT_MESSAGE_KIND);
        assert_eq!(template.content, "alpha feedback");
        assert_eq!(template.tags[0], ["h", "openagents-public"]);
        assert_eq!(
            template.tags[1].first().map(String::as_str),
            Some("previous")
        );
        assert_eq!(template.tags[1].len(), 4);
        assert!(
            event_template(
                &descriptor(),
                &PublicChannelWrite::Message {
                    content: "x".repeat(8_193),
                },
                &[],
                &own.public_key().to_hex(),
                200,
            )
            .is_err()
        );
    }

    #[test]
    fn report_template_names_only_target_coordinate_and_never_copies_content() {
        let template = event_template(
            &descriptor(),
            &PublicChannelWrite::Report {
                event_id: "a".repeat(64),
                author_public_key: "b".repeat(64),
            },
            &[],
            &"c".repeat(64),
            200,
        )
        .expect("report template");
        assert_eq!(template.kind, REPORT_KIND);
        assert!(template.content.is_empty());
        assert_eq!(template.tags[0], ["h", "openagents-public"]);
        assert_eq!(template.tags[1][0], "e");
        assert_eq!(template.tags[2][0], "p");
    }

    #[test]
    fn identity_service_signs_a_verified_exact_channel_event() {
        let directory = tempfile::tempdir().expect("identity directory");
        let service =
            IdentityService::for_channel_data_root(AppChannel::Dev, directory.path().to_path_buf());
        let signed = sign_write(
            &service,
            &descriptor(),
            PublicChannelWrite::Message {
                content: "signed alpha feedback".to_string(),
            },
            &[],
        )
        .expect("signed write");
        assert!(signed.event.verify().is_ok());
        assert_eq!(signed.record.kind, CHAT_MESSAGE_KIND);
        assert!(signed.record.has_tag("h", "openagents-public"));
        assert!(!signed.is_report);
    }

    #[test]
    fn signing_a_report_requires_the_exact_verified_channel_message() {
        let directory = tempfile::tempdir().expect("identity directory");
        let service =
            IdentityService::for_channel_data_root(AppChannel::Dev, directory.path().to_path_buf());
        let author = Keys::generate();
        let target = event(&author, 7, 100);
        let write = PublicChannelWrite::Report {
            event_id: target.id.clone(),
            author_public_key: target.public_key.clone(),
        };
        assert!(
            sign_write(&service, &descriptor(), write.clone(), &[]).is_err(),
            "an arbitrary coordinate must not become a signed public report"
        );
        let signed =
            sign_write(&service, &descriptor(), write, &[target]).expect("verified target report");
        assert_eq!(signed.record.kind, REPORT_KIND);
        assert!(signed.record.content.is_empty());
        assert!(signed.record.has_tag("h", "openagents-public"));
        assert!(signed.is_report);
    }

    #[test]
    fn transport_retry_reuses_the_exact_signed_event() {
        let directory = tempfile::tempdir().expect("identity directory");
        let service =
            IdentityService::for_channel_data_root(AppChannel::Dev, directory.path().to_path_buf());
        let descriptor = descriptor();
        let signed = sign_write(
            &service,
            &descriptor,
            PublicChannelWrite::Message {
                content: "immutable retry".to_string(),
            },
            &[],
        )
        .expect("signed write");
        let mut attempts = Vec::new();
        let result = publish_signed_write_with(&descriptor, &signed, |relay_url, event| {
            attempts.push((relay_url.to_string(), event.as_json()));
            Err(anyhow!("relay unavailable"))
        });
        assert!(result.is_err());
        assert_eq!(attempts.len(), MAX_PUBLISH_ATTEMPTS);
        assert_eq!(attempts[0], attempts[1]);
    }
}
