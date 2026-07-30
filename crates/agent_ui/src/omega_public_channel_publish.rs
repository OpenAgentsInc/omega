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
    AdmittedSigningRequest, DurableIdentityActionDecision, DurableIdentityActionDescriptor,
    DurableIdentityActionKind, IdentityActionAuthorization, IdentityActivationRequired,
    IdentityRef, IdentityService, ProofRef, ReceiptRef, ResourceRef, SigningPurpose,
    UnsignedEventTemplate,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedPublicChannelWrite {
    identity_ref: IdentityRef,
    request_ref: ReceiptRef,
    activation: DurableIdentityActionDescriptor,
    event: UnsignedEventTemplate,
    is_report: bool,
}

impl PreparedPublicChannelWrite {
    pub fn is_report(&self) -> bool {
        self.is_report
    }

    #[cfg(test)]
    fn event(&self) -> &UnsignedEventTemplate {
        &self.event
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
    let prepared = prepare_write(identity_service, descriptor, write, events)?;
    match authorize_prepared_write(identity_service, &prepared)? {
        DurableIdentityActionDecision::Authorized(authorization) => {
            sign_prepared_write(identity_service, descriptor, prepared, &authorization)
        }
        DurableIdentityActionDecision::ActivationRequired { account, intent } => {
            Err(IdentityActivationRequired::new(account, intent).into())
        }
    }
}

pub fn prepare_write(
    identity_service: &IdentityService,
    descriptor: &ChannelDescriptor,
    write: PublicChannelWrite,
    events: &[NostrEventRecord],
) -> Result<PreparedPublicChannelWrite> {
    prepare_write_at(
        identity_service,
        descriptor,
        write,
        events,
        unix_time_seconds()?,
    )
}

fn prepare_write_at(
    identity_service: &IdentityService,
    descriptor: &ChannelDescriptor,
    write: PublicChannelWrite,
    events: &[NostrEventRecord],
    created_at: u64,
) -> Result<PreparedPublicChannelWrite> {
    let custody = identity_service
        .inspect()
        .context("inspecting the Omega identity")?;
    let identity = custody
        .identity
        .ok_or_else(|| anyhow!("the Omega identity is not ready"))?;
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
    let payload_digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&event)?));
    let destination_digest = format!(
        "{:x}",
        Sha256::digest(format!("{}\0{}", descriptor.relay_url, descriptor.group_id))
    );
    Ok(PreparedPublicChannelWrite {
        identity_ref: identity.identity_ref().clone(),
        request_ref: request_ref.clone(),
        activation: DurableIdentityActionDescriptor {
            intent_ref: request_ref.clone(),
            kind: DurableIdentityActionKind::PublicPost,
            destination_ref: ResourceRef::new(format!("nip29-{destination_digest}"))?,
            authorization_ref: ProofRef::new(format!("activation-{}", request_ref.as_str()))?,
            payload_digest,
            expires_at: created_at
                .checked_add(300)
                .ok_or_else(|| anyhow!("the activation window overflowed"))?,
        },
        event,
        is_report: matches!(write, PublicChannelWrite::Report { .. }),
    })
}

pub fn authorize_prepared_write(
    identity_service: &IdentityService,
    prepared: &PreparedPublicChannelWrite,
) -> Result<DurableIdentityActionDecision> {
    let custody = identity_service
        .inspect()
        .context("rechecking the Omega identity before public-write authorization")?;
    let identity = custody
        .identity
        .ok_or_else(|| anyhow!("the Omega identity is not ready"))?;
    ensure!(
        identity.identity_ref() == &prepared.identity_ref,
        "the Omega identity changed after the public write was prepared"
    );
    identity_service
        .authorize_or_hold_identity_action(prepared.activation.clone())
        .context("authorizing the prepared public tester-channel event")
}

pub fn sign_prepared_write(
    identity_service: &IdentityService,
    live_descriptor: &ChannelDescriptor,
    prepared: PreparedPublicChannelWrite,
    authorization: &IdentityActionAuthorization,
) -> Result<SignedPublicChannelWrite> {
    live_descriptor.validate()?;
    let prepared_payload_digest =
        format!("{:x}", Sha256::digest(serde_json::to_vec(&prepared.event)?));
    ensure!(
        prepared_payload_digest == prepared.activation.payload_digest,
        "the prepared public-write payload changed after identity activation"
    );
    let destination_digest = format!(
        "{:x}",
        Sha256::digest(format!(
            "{}\0{}",
            live_descriptor.relay_url, live_descriptor.group_id
        ))
    );
    let live_destination = ResourceRef::new(format!("nip29-{destination_digest}"))?;
    let intent = authorization.intent();
    ensure!(
        intent.intent_ref == prepared.activation.intent_ref
            && intent.identity_ref == prepared.identity_ref
            && intent.kind == prepared.activation.kind
            && intent.destination_ref == prepared.activation.destination_ref
            && intent.authorization_ref == prepared.activation.authorization_ref
            && intent.payload_digest == prepared.activation.payload_digest
            && intent.expires_at == prepared.activation.expires_at,
        "the activation authorization does not match the prepared public write"
    );
    ensure!(
        live_destination == intent.destination_ref,
        "the public channel destination changed after identity activation"
    );
    ensure!(
        live_descriptor
            .accepted_kinds
            .contains(&prepared.event.kind),
        "the public channel no longer accepts the prepared event kind"
    );
    identity_service
        .validate_identity_action_authorization(authorization)
        .context("revalidating the prepared public-write authorization")?;
    let custody = identity_service
        .inspect()
        .context("rechecking the Omega identity before signing")?;
    let identity = custody
        .identity
        .ok_or_else(|| anyhow!("the Omega identity is not ready"))?;
    ensure!(
        identity.identity_ref() == &prepared.identity_ref,
        "the Omega identity changed before the prepared public write was signed"
    );
    ensure!(
        signing_receipt(&identity, &prepared.event)? == prepared.request_ref,
        "the prepared public-write receipt no longer matches its exact event"
    );
    let signed = identity_service
        .sign(&AdmittedSigningRequest {
            request_ref: prepared.request_ref,
            identity_ref: prepared.identity_ref,
            purpose: SigningPurpose::NostrEvent,
            event: prepared.event,
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

    // omega#164. A signed tester-channel write is the first kind of value a
    // background-created identity accrues, so it arms the quiet backup nudge
    // (OMEGA-DELTA-0183). Recorded through the same service that signed —
    // tests pass a temporary data root — and fail-soft: never a publish
    // blocker.
    if let Err(error) =
        identity_service.record_backup_value_accrued(omega_identity::BackupValueKind::ChannelPost)
    {
        log::warn!("could not record identity backup value accrual: {error}");
    }

    Ok(SignedPublicChannelWrite {
        event,
        record,
        author_public_key: identity.public_key_hex().as_str().to_string(),
        is_report: prepared.is_report,
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
    use omega_identity::RecoveryPassword;
    use serde_json::{Value, json};

    use crate::omega_public_channel_relay::{
        RelayAdmissionLimits, RelayCommand, RelayGapReason, RelayInput, RelayLifecycle,
        RelaySession, RelaySessionConfig,
    };

    fn descriptor() -> ChannelDescriptor {
        crate::omega_public_channels::bundled_tester_registry()
            .expect("bundled registry")
            .channels
            .into_iter()
            .next()
            .expect("alpha feedback channel")
    }

    fn activate_for_write(
        service: &IdentityService,
        channel: &ChannelDescriptor,
        write: PublicChannelWrite,
        events: &[NostrEventRecord],
    ) {
        let error = sign_write(service, channel, write, events)
            .expect_err("candidate write must require activation");
        let activation = error
            .downcast_ref::<IdentityActivationRequired>()
            .expect("typed activation requirement");
        let recovery_directory = tempfile::tempdir().expect("recovery directory");
        service
            .export_recovery_artifact(
                &activation.intent().identity_ref,
                &recovery_directory.path().join("identity.ncryptsec"),
                RecoveryPassword::new("test public write recovery".to_string())
                    .expect("valid recovery password"),
            )
            .expect("protect candidate recovery");
        service
            .complete_activation(activation.intent())
            .expect("complete activation");
        service
            .take_activated_identity_action(activation.intent())
            .expect("take held write once");
    }

    fn event(keys: &Keys, id_seed: u64, created_at: u64, group_id: &str) -> NostrEventRecord {
        let event = EventBuilder::new(Kind::Custom(CHAT_MESSAGE_KIND), id_seed.to_string())
            .custom_created_at(Timestamp::from_secs(created_at))
            .tags(vec![Tag::parse(["h", group_id]).expect("group tag")])
            .sign_with_keys(keys)
            .expect("signed fixture event");
        serde_json::from_str(&event.as_json()).expect("event record")
    }

    fn relay_config(
        descriptor: &ChannelDescriptor,
        relay_self_public_key: String,
    ) -> RelaySessionConfig {
        RelaySessionConfig {
            relay_url: descriptor.relay_url.clone(),
            group_id: descriptor.group_id.clone(),
            accepted_kinds: descriptor.accepted_kinds.clone(),
            group_state_kinds: descriptor.group_state_kinds.clone(),
            moderation_kinds: descriptor.moderation_kinds.clone(),
            expected_relay_self_pubkey: Some(relay_self_public_key),
            history_page_size: descriptor.limits.history_page_size,
            limits: RelayAdmissionLimits {
                content_bytes: descriptor.limits.content_bytes,
                event_bytes: descriptor.limits.event_bytes,
                future_skew_seconds: descriptor.limits.future_skew_seconds,
                max_age_seconds: descriptor.limits.max_age_seconds,
                tags: descriptor.limits.tags,
            },
        }
    }

    fn sent_frames(commands: &[RelayCommand]) -> Vec<Value> {
        commands
            .iter()
            .filter_map(|command| match command {
                RelayCommand::SendText(text) => serde_json::from_str(text).ok(),
                RelayCommand::Connect { .. } | RelayCommand::ScheduleReconnect { .. } => None,
            })
            .collect()
    }

    fn subscription_for(frames: &[Value], filter_name: &str) -> String {
        frames
            .iter()
            .find(|frame| {
                frame
                    .get(2)
                    .and_then(|filter| filter.get(filter_name))
                    .is_some()
            })
            .and_then(|frame| frame.get(1))
            .and_then(Value::as_str)
            .expect("subscription")
            .to_string()
    }

    fn start_reader(
        descriptor: &ChannelDescriptor,
        relay_self_public_key: String,
        message: &NostrEventRecord,
        relay_state: &NostrEventRecord,
        now_ms: u64,
    ) -> (RelaySession, String) {
        let mut session = RelaySession::new(relay_config(descriptor, relay_self_public_key));
        assert!(matches!(
            session
                .apply(RelayInput::ConnectRequested { now_ms })
                .as_slice(),
            [RelayCommand::Connect { .. }]
        ));
        let frames = sent_frames(&session.apply(RelayInput::Connected { now_ms }));
        let history_subscription = subscription_for(&frames, "#h");
        let state_subscription = subscription_for(&frames, "#d");
        session.apply(RelayInput::TextFrame {
            text: json!(["EVENT", history_subscription, message]).to_string(),
            now_ms,
        });
        session.apply(RelayInput::TextFrame {
            text: json!(["EVENT", state_subscription, relay_state]).to_string(),
            now_ms,
        });
        session.apply(RelayInput::TextFrame {
            text: json!(["EOSE", history_subscription]).to_string(),
            now_ms,
        });
        session.apply(RelayInput::TextFrame {
            text: json!(["EOSE", state_subscription]).to_string(),
            now_ms,
        });
        assert_eq!(session.snapshot().lifecycle, RelayLifecycle::Current);
        assert!(session.snapshot().metadata_trusted);
        (session, history_subscription)
    }

    #[test]
    fn message_template_binds_group_limits_and_three_foreign_predecessors() {
        let own = Keys::generate();
        let other = Keys::generate();
        let channel = descriptor();
        let group_id = channel.group_id.clone();
        let events = (0..6)
            .map(|index| {
                let keys = if index == 5 { &own } else { &other };
                event(keys, index, 100 + index, &group_id)
            })
            .collect::<Vec<_>>();
        let template = event_template(
            &channel,
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
        assert_eq!(template.tags[0], ["h", group_id.as_str()]);
        assert_eq!(
            template.tags[1].first().map(String::as_str),
            Some("previous")
        );
        assert_eq!(template.tags[1].len(), 4);
        assert!(
            event_template(
                &channel,
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
        let channel = descriptor();
        let template = event_template(
            &channel,
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
        assert_eq!(template.tags[0], ["h", channel.group_id.as_str()]);
        assert_eq!(template.tags[1][0], "e");
        assert_eq!(template.tags[2][0], "p");
    }

    #[test]
    fn identity_service_signs_a_verified_exact_channel_event() {
        let directory = tempfile::tempdir().expect("identity directory");
        let service =
            IdentityService::for_channel_data_root(AppChannel::Dev, directory.path().to_path_buf());
        service
            .create(ReceiptRef::new("test-public-write-create").expect("valid receipt"))
            .expect("create candidate identity");
        let channel = descriptor();
        activate_for_write(
            &service,
            &channel,
            PublicChannelWrite::Message {
                content: "activate identity".to_string(),
            },
            &[],
        );
        let signed = sign_write(
            &service,
            &channel,
            PublicChannelWrite::Message {
                content: "signed alpha feedback".to_string(),
            },
            &[],
        )
        .expect("signed write");
        assert!(signed.event.verify().is_ok());
        assert_eq!(signed.record.kind, CHAT_MESSAGE_KIND);
        assert!(signed.record.has_tag("h", &channel.group_id));
        assert!(!signed.is_report);
    }

    #[test]
    fn activation_resume_signs_the_exact_prepared_event() {
        let directory = tempfile::tempdir().expect("identity directory");
        let service =
            IdentityService::for_channel_data_root(AppChannel::Dev, directory.path().to_path_buf());
        service
            .create(ReceiptRef::new("test-prepared-resume-create").expect("valid receipt"))
            .expect("create candidate identity");
        let channel = descriptor();
        let prepared = prepare_write(
            &service,
            &channel,
            PublicChannelWrite::Message {
                content: "retain this exact event".to_string(),
            },
            &[],
        )
        .expect("prepare candidate write");
        let original_event = prepared.event().clone();
        let original_digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&original_event).expect("serialize prepared event"))
        );
        let activation = match authorize_prepared_write(&service, &prepared)
            .expect("hold prepared candidate write")
        {
            DurableIdentityActionDecision::ActivationRequired { intent, .. } => intent,
            DurableIdentityActionDecision::Authorized(_) => {
                panic!("candidate write unexpectedly authorized")
            }
        };
        assert_eq!(activation.payload_digest, original_digest);
        let recovery_directory = tempfile::tempdir().expect("recovery directory");
        service
            .export_recovery_artifact(
                &activation.identity_ref,
                &recovery_directory.path().join("identity.ncryptsec"),
                RecoveryPassword::new("test exact resume recovery".to_string())
                    .expect("valid recovery password"),
            )
            .expect("protect candidate recovery");
        service
            .complete_activation(&activation)
            .expect("complete exact activation");
        let authorization = service
            .take_activated_identity_action(&activation)
            .expect("consume exact held write");
        assert_eq!(authorization.intent().payload_digest, original_digest);
        let signed = sign_prepared_write(&service, &channel, prepared, &authorization)
            .expect("sign exact prepared event");

        assert_eq!(
            u64::try_from(signed.record.created_at).expect("non-negative event time"),
            original_event.created_at
        );
        assert_eq!(signed.record.content, original_event.content);
        assert_eq!(signed.record.tags, original_event.tags);
        assert_eq!(signed.record.kind, original_event.kind);
        assert!(
            service.take_activated_identity_action(&activation).is_err(),
            "the held write must remain one-shot"
        );
    }

    #[test]
    fn prepared_write_refuses_a_changed_live_destination() {
        let directory = tempfile::tempdir().expect("identity directory");
        let service =
            IdentityService::for_channel_data_root(AppChannel::Dev, directory.path().to_path_buf());
        service
            .create(ReceiptRef::new("test-live-destination-create").expect("valid receipt"))
            .expect("create candidate identity");
        let channel = descriptor();
        activate_for_write(
            &service,
            &channel,
            PublicChannelWrite::Message {
                content: "activate destination test".to_string(),
            },
            &[],
        );
        let prepared = prepare_write(
            &service,
            &channel,
            PublicChannelWrite::Message {
                content: "must stay in the original channel".to_string(),
            },
            &[],
        )
        .expect("prepare active write");
        let authorization =
            match authorize_prepared_write(&service, &prepared).expect("authorize active write") {
                DurableIdentityActionDecision::Authorized(authorization) => authorization,
                DurableIdentityActionDecision::ActivationRequired { .. } => {
                    panic!("active write unexpectedly requires activation")
                }
            };
        let mut changed_channel = channel;
        changed_channel.group_id.push_str("-different");

        let error = sign_prepared_write(&service, &changed_channel, prepared, &authorization)
            .expect_err("changed destination must be refused");
        assert!(
            error
                .to_string()
                .contains("destination changed after identity activation")
        );
    }

    #[test]
    fn signing_a_report_requires_the_exact_verified_channel_message() {
        let directory = tempfile::tempdir().expect("identity directory");
        let service =
            IdentityService::for_channel_data_root(AppChannel::Dev, directory.path().to_path_buf());
        service
            .create(ReceiptRef::new("test-public-report-create").expect("valid receipt"))
            .expect("create candidate identity");
        let channel = descriptor();
        let author = Keys::generate();
        let target = event(&author, 7, 100, &channel.group_id);
        let write = PublicChannelWrite::Report {
            event_id: target.id.clone(),
            author_public_key: target.public_key.clone(),
        };
        assert!(
            sign_write(&service, &channel, write.clone(), &[]).is_err(),
            "an arbitrary coordinate must not become a signed public report"
        );
        activate_for_write(
            &service,
            &channel,
            write.clone(),
            std::slice::from_ref(&target),
        );
        let signed =
            sign_write(&service, &channel, write, &[target]).expect("verified target report");
        assert_eq!(signed.record.kind, REPORT_KIND);
        assert!(signed.record.content.is_empty());
        assert!(signed.record.has_tag("h", &channel.group_id));
        assert!(signed.is_report);
    }

    #[test]
    fn transport_retry_reuses_the_exact_signed_event() {
        let directory = tempfile::tempdir().expect("identity directory");
        let service =
            IdentityService::for_channel_data_root(AppChannel::Dev, directory.path().to_path_buf());
        service
            .create(ReceiptRef::new("test-public-retry-create").expect("valid receipt"))
            .expect("create candidate identity");
        let descriptor = descriptor();
        activate_for_write(
            &service,
            &descriptor,
            PublicChannelWrite::Message {
                content: "activate identity".to_string(),
            },
            &[],
        );
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

    #[test]
    fn two_account_send_receive_report_moderation_and_outage_are_hermetic() {
        let directory = tempfile::tempdir().expect("acceptance identities");
        let sender_service = IdentityService::for_channel_data_root(
            AppChannel::Dev,
            directory.path().join("sender"),
        );
        let receiver_service = IdentityService::for_channel_data_root(
            AppChannel::Dev,
            directory.path().join("receiver"),
        );
        let mut descriptor = descriptor();
        let relay_self = Keys::generate();
        descriptor.expected_relay_self_pubkey = Some(relay_self.public_key().to_hex());
        sender_service
            .create(ReceiptRef::new("test-hermetic-sender-create").expect("valid receipt"))
            .expect("create sender identity");
        receiver_service
            .create(ReceiptRef::new("test-hermetic-receiver-create").expect("valid receipt"))
            .expect("create receiver identity");
        activate_for_write(
            &sender_service,
            &descriptor,
            PublicChannelWrite::Message {
                content: "activate hermetic sender".to_string(),
            },
            &[],
        );

        let signed_message = sign_write(
            &sender_service,
            &descriptor,
            PublicChannelWrite::Message {
                content: "simulated installed-candidate feedback".to_string(),
            },
            &[],
        )
        .expect("first account signs its message");
        let mut published = Vec::new();
        publish_signed_write_with(&descriptor, &signed_message, |relay_url, event| {
            assert_eq!(relay_url, descriptor.relay_url);
            published.push(event.as_json());
            Ok(())
        })
        .expect("simulated relay accepts the message");
        let message: NostrEventRecord =
            serde_json::from_str(&published[0]).expect("published message record");

        let created_at =
            u64::try_from(message.created_at).expect("message timestamp is non-negative");
        let now_ms = created_at.saturating_mul(1_000);
        let relay_state_event = EventBuilder::new(Kind::Custom(39001), "")
            .custom_created_at(Timestamp::from_secs(created_at))
            .tag(Tag::parse(["d", descriptor.group_id.as_str()]).expect("group-state tag"))
            .sign_with_keys(&relay_self)
            .expect("signed relay state");
        let relay_state: NostrEventRecord =
            serde_json::from_str(&relay_state_event.as_json()).expect("relay state record");

        let (mut sender_reader, sender_history) = start_reader(
            &descriptor,
            relay_self.public_key().to_hex(),
            &message,
            &relay_state,
            now_ms,
        );
        let (mut receiver_reader, receiver_history) = start_reader(
            &descriptor,
            relay_self.public_key().to_hex(),
            &message,
            &relay_state,
            now_ms,
        );
        let received_message = receiver_reader
            .snapshot()
            .events
            .into_iter()
            .find(|event| event.id == message.id)
            .expect("second account receives the first account's message");
        assert_eq!(
            received_message.public_key,
            signed_message.author_public_key
        );
        activate_for_write(
            &receiver_service,
            &descriptor,
            PublicChannelWrite::Report {
                event_id: message.id.clone(),
                author_public_key: message.public_key.clone(),
            },
            &receiver_reader.snapshot().events,
        );

        let signed_report = sign_write(
            &receiver_service,
            &descriptor,
            PublicChannelWrite::Report {
                event_id: message.id.clone(),
                author_public_key: message.public_key.clone(),
            },
            &receiver_reader.snapshot().events,
        )
        .expect("second account signs a report of the verified message");
        assert_ne!(
            signed_report.author_public_key, signed_message.author_public_key,
            "the acceptance proof must use two isolated identities"
        );
        publish_signed_write_with(&descriptor, &signed_report, |relay_url, event| {
            assert_eq!(relay_url, descriptor.relay_url);
            published.push(event.as_json());
            Ok(())
        })
        .expect("simulated relay accepts the report");
        let report: NostrEventRecord =
            serde_json::from_str(&published[1]).expect("published report record");
        assert!(report.content.is_empty());
        assert_eq!(report.tag_values("e").next(), Some(message.id.as_str()));
        assert_eq!(
            report.tag_values("p").next(),
            Some(message.public_key.as_str())
        );

        let moderation_event = EventBuilder::new(Kind::Custom(9005), "")
            .custom_created_at(Timestamp::from_secs(created_at))
            .tags(vec![
                Tag::parse(["h", descriptor.group_id.as_str()]).expect("moderation group tag"),
                Tag::parse(["e", message.id.as_str()]).expect("moderation event tag"),
            ])
            .sign_with_keys(&relay_self)
            .expect("signed moderation event");
        let moderation: NostrEventRecord =
            serde_json::from_str(&moderation_event.as_json()).expect("moderation record");
        for (session, history_subscription) in [
            (&mut sender_reader, sender_history.as_str()),
            (&mut receiver_reader, receiver_history.as_str()),
        ] {
            for event in [&report, &moderation] {
                session.apply(RelayInput::TextFrame {
                    text: json!(["EVENT", history_subscription, event]).to_string(),
                    now_ms,
                });
            }
        }
        let current = receiver_reader.snapshot();
        assert!(current.events.iter().any(|event| event.id == report.id));
        assert!(current.events.iter().any(|event| event.id == moderation.id));

        let reconnect = receiver_reader.apply(RelayInput::Disconnected {
            now_ms: now_ms.saturating_add(1),
        });
        let stale = receiver_reader.snapshot();
        assert_eq!(stale.lifecycle, RelayLifecycle::Stale);
        assert_eq!(stale.gap_reason, Some(RelayGapReason::RelayUnavailable));
        assert_eq!(stale.events, current.events);
        assert!(matches!(
            reconnect.as_slice(),
            [RelayCommand::ScheduleReconnect { .. }]
        ));
    }
}
