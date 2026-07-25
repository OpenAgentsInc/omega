use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const ISSUE31_HOST_DISCOVERY_SCHEMA: &str = "openagents.omega.issue31.host_discovery.v1";
pub const ISSUE31_PAIRING_SCHEMA: &str = "openagents.omega.issue31.pairing.v1";
pub const ISSUE31_COMMAND_SCHEMA: &str = "openagents.omega.issue31.command.v1";
pub const ISSUE31_HOST_DISCOVERY_KIND: u16 = 31_990;
pub const ISSUE31_PRIVATE_RUMOR_KIND: u16 = 14;
pub const ISSUE31_PRIVATE_SEAL_KIND: u16 = 13;
pub const ISSUE31_PRIVATE_GIFT_WRAP_KIND: u16 = 1_059;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Issue31NostrError {
    #[error("invalid Issue 31 record: {0}")]
    Invalid(String),
    #[error("Issue 31 record decoding failed: {0}")]
    Decode(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Issue31HostDiscovery {
    pub schema: String,
    pub host_ref: String,
    pub host_public_key_hex: String,
    pub sarah_public_key_hex: String,
    pub display_name: String,
    pub protocols: Vec<String>,
    pub relay_urls: Vec<String>,
    pub generation: u64,
    pub issued_at: u64,
    pub expires_at: u64,
}

impl Issue31HostDiscovery {
    pub fn decode(bytes: &[u8]) -> Result<Self, Issue31NostrError> {
        if bytes.len() > 64 * 1024 {
            return Err(Issue31NostrError::Invalid(
                "host discovery exceeds the record budget".into(),
            ));
        }
        let record: Self = serde_json::from_slice(bytes)
            .map_err(|error| Issue31NostrError::Decode(error.to_string()))?;
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), Issue31NostrError> {
        if self.schema != ISSUE31_HOST_DISCOVERY_SCHEMA
            || !valid_ref(&self.host_ref)
            || !valid_hex64(&self.host_public_key_hex)
            || !valid_hex64(&self.sarah_public_key_hex)
            || self.sarah_public_key_hex == self.host_public_key_hex
            || self.display_name.is_empty()
            || self.display_name.len() > 80
            || self.generation == 0
            || self.expires_at <= self.issued_at
            || self.protocols.len() != 2
            || !all_unique(&self.protocols)
            || !self
                .protocols
                .iter()
                .any(|protocol| protocol == ISSUE31_PAIRING_SCHEMA)
            || !self
                .protocols
                .iter()
                .any(|protocol| protocol == ISSUE31_COMMAND_SCHEMA)
            || self.relay_urls.is_empty()
            || self.relay_urls.len() > 8
            || !all_unique(&self.relay_urls)
            || self
                .relay_urls
                .iter()
                .any(|relay_url| !valid_relay_url(relay_url))
        {
            return Err(Issue31NostrError::Invalid(
                "host discovery failed its schema, identity, protocol, relay, or lifetime law"
                    .into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Issue31PairingScope {
    ObserveIssue31,
    SendMessage,
    InterruptTurn,
    ControlFullAuto,
    RequestProviderHandoff,
    ActInCommunity,
}

impl Issue31PairingScope {
    pub fn parse(value: &str) -> Result<Self, Issue31NostrError> {
        match value {
            "observe_issue31" => Ok(Self::ObserveIssue31),
            "send_message" => Ok(Self::SendMessage),
            "interrupt_turn" => Ok(Self::InterruptTurn),
            "control_full_auto" => Ok(Self::ControlFullAuto),
            "request_provider_handoff" => Ok(Self::RequestProviderHandoff),
            "act_in_community" => Ok(Self::ActInCommunity),
            _ => Err(Issue31NostrError::Invalid(
                "unknown Issue 31 pairing scope".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "recordType", rename_all = "snake_case", deny_unknown_fields)]
pub enum Issue31PairingRecord {
    PairingRequest {
        schema: String,
        #[serde(rename = "hostRef")]
        host_ref: String,
        #[serde(rename = "hostPublicKeyHex")]
        host_public_key_hex: String,
        #[serde(rename = "devicePublicKeyHex")]
        device_public_key_hex: String,
        #[serde(rename = "issuedAt")]
        issued_at: u64,
        #[serde(rename = "pairingRequestRef")]
        pairing_request_ref: String,
        #[serde(rename = "requestedScopes")]
        requested_scopes: Vec<Issue31PairingScope>,
        #[serde(rename = "expiresAt")]
        expires_at: u64,
    },
    PairingChallenge {
        schema: String,
        #[serde(rename = "hostRef")]
        host_ref: String,
        #[serde(rename = "hostPublicKeyHex")]
        host_public_key_hex: String,
        #[serde(rename = "devicePublicKeyHex")]
        device_public_key_hex: String,
        #[serde(rename = "issuedAt")]
        issued_at: u64,
        #[serde(rename = "pairingChallengeRef")]
        pairing_challenge_ref: String,
        #[serde(rename = "pairingRequestEventId")]
        pairing_request_event_id: String,
        challenge: String,
        #[serde(rename = "expiresAt")]
        expires_at: u64,
    },
    PairingResponse {
        schema: String,
        #[serde(rename = "hostRef")]
        host_ref: String,
        #[serde(rename = "hostPublicKeyHex")]
        host_public_key_hex: String,
        #[serde(rename = "devicePublicKeyHex")]
        device_public_key_hex: String,
        #[serde(rename = "issuedAt")]
        issued_at: u64,
        #[serde(rename = "pairingResponseRef")]
        pairing_response_ref: String,
        #[serde(rename = "pairingChallengeEventId")]
        pairing_challenge_event_id: String,
        challenge: String,
        #[serde(rename = "expiresAt")]
        expires_at: u64,
    },
    ScopedGrant {
        schema: String,
        #[serde(rename = "hostRef")]
        host_ref: String,
        #[serde(rename = "hostPublicKeyHex")]
        host_public_key_hex: String,
        #[serde(rename = "sarahPublicKeyHex")]
        sarah_public_key_hex: String,
        #[serde(rename = "devicePublicKeyHex")]
        device_public_key_hex: String,
        #[serde(rename = "issuedAt")]
        issued_at: u64,
        #[serde(rename = "pairingResponseEventId")]
        pairing_response_event_id: String,
        #[serde(rename = "grantRef")]
        grant_ref: String,
        generation: u64,
        scopes: Vec<Issue31PairingScope>,
        #[serde(rename = "expiresAt")]
        expires_at: u64,
    },
    GrantRenewal {
        schema: String,
        #[serde(rename = "hostRef")]
        host_ref: String,
        #[serde(rename = "hostPublicKeyHex")]
        host_public_key_hex: String,
        #[serde(rename = "sarahPublicKeyHex")]
        sarah_public_key_hex: String,
        #[serde(rename = "devicePublicKeyHex")]
        device_public_key_hex: String,
        #[serde(rename = "issuedAt")]
        issued_at: u64,
        #[serde(rename = "grantRef")]
        grant_ref: String,
        #[serde(rename = "previousGrantEventId")]
        previous_grant_event_id: String,
        #[serde(rename = "priorGeneration")]
        prior_generation: u64,
        generation: u64,
        scopes: Vec<Issue31PairingScope>,
        #[serde(rename = "expiresAt")]
        expires_at: u64,
    },
    GrantRevocation {
        schema: String,
        #[serde(rename = "hostRef")]
        host_ref: String,
        #[serde(rename = "hostPublicKeyHex")]
        host_public_key_hex: String,
        #[serde(rename = "sarahPublicKeyHex")]
        sarah_public_key_hex: String,
        #[serde(rename = "devicePublicKeyHex")]
        device_public_key_hex: String,
        #[serde(rename = "issuedAt")]
        issued_at: u64,
        #[serde(rename = "grantRef")]
        grant_ref: String,
        generation: u64,
        #[serde(rename = "reasonRef", default, skip_serializing_if = "Option::is_none")]
        reason_ref: Option<String>,
    },
}

impl Issue31PairingRecord {
    pub fn decode(bytes: &[u8]) -> Result<Self, Issue31NostrError> {
        if bytes.len() > 64 * 1024 {
            return Err(Issue31NostrError::Invalid(
                "pairing record exceeds the record budget".into(),
            ));
        }
        let record: Self = serde_json::from_slice(bytes)
            .map_err(|error| Issue31NostrError::Decode(error.to_string()))?;
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), Issue31NostrError> {
        let (schema, host_ref, host_key, device_key, issued_at) = self.base();
        if schema != ISSUE31_PAIRING_SCHEMA
            || !valid_ref(host_ref)
            || !valid_hex64(host_key)
            || !valid_hex64(device_key)
        {
            return Err(Issue31NostrError::Invalid(
                "invalid pairing identity".into(),
            ));
        }
        match self {
            Self::PairingRequest {
                pairing_request_ref,
                requested_scopes,
                expires_at,
                ..
            } => validate_scopes(requested_scopes, "requested scopes")
                .and_then(|()| validate_ref_value(pairing_request_ref, "pairing request"))
                .and_then(|()| validate_lifetime(issued_at, *expires_at)),
            Self::PairingChallenge {
                pairing_challenge_ref,
                pairing_request_event_id,
                challenge,
                expires_at,
                ..
            } => validate_ref_value(pairing_challenge_ref, "pairing challenge")
                .and_then(|()| {
                    validate_hex_value(pairing_request_event_id, "pairing request event")
                })
                .and_then(|()| validate_hex_value(challenge, "pairing challenge nonce"))
                .and_then(|()| validate_lifetime(issued_at, *expires_at)),
            Self::PairingResponse {
                pairing_response_ref,
                pairing_challenge_event_id,
                challenge,
                expires_at,
                ..
            } => validate_ref_value(pairing_response_ref, "pairing response")
                .and_then(|()| validate_hex_value(pairing_challenge_event_id, "challenge event"))
                .and_then(|()| validate_hex_value(challenge, "pairing challenge nonce"))
                .and_then(|()| validate_lifetime(issued_at, *expires_at)),
            Self::ScopedGrant {
                sarah_public_key_hex,
                pairing_response_event_id,
                grant_ref,
                generation,
                scopes,
                expires_at,
                ..
            } => validate_sarah_binding(host_key, sarah_public_key_hex)
                .and_then(|()| {
                    validate_hex_value(pairing_response_event_id, "pairing response event")
                })
                .and_then(|()| validate_ref_value(grant_ref, "grant"))
                .and_then(|()| validate_generation(*generation))
                .and_then(|()| validate_scopes(scopes, "grant scopes"))
                .and_then(|()| validate_lifetime(issued_at, *expires_at)),
            Self::GrantRenewal {
                sarah_public_key_hex,
                grant_ref,
                previous_grant_event_id,
                prior_generation,
                generation,
                scopes,
                expires_at,
                ..
            } => validate_sarah_binding(host_key, sarah_public_key_hex)
                .and_then(|()| validate_ref_value(grant_ref, "grant"))
                .and_then(|()| validate_hex_value(previous_grant_event_id, "previous grant event"))
                .and_then(|()| validate_generation(*prior_generation))
                .and_then(|()| {
                    if *generation == prior_generation.saturating_add(1) {
                        Ok(())
                    } else {
                        Err(Issue31NostrError::Invalid(
                            "grant renewal must advance exactly one generation".into(),
                        ))
                    }
                })
                .and_then(|()| validate_scopes(scopes, "grant scopes"))
                .and_then(|()| validate_lifetime(issued_at, *expires_at)),
            Self::GrantRevocation {
                sarah_public_key_hex,
                grant_ref,
                generation,
                reason_ref,
                ..
            } => validate_sarah_binding(host_key, sarah_public_key_hex)
                .and_then(|()| validate_ref_value(grant_ref, "grant"))
                .and_then(|()| validate_generation(*generation))
                .and_then(|()| match reason_ref {
                    Some(reason_ref) => validate_ref_value(reason_ref, "revocation reason"),
                    None => Ok(()),
                }),
        }
    }

    fn base(&self) -> (&str, &str, &str, &str, u64) {
        match self {
            Self::PairingRequest {
                schema,
                host_ref,
                host_public_key_hex,
                device_public_key_hex,
                issued_at,
                ..
            }
            | Self::PairingChallenge {
                schema,
                host_ref,
                host_public_key_hex,
                device_public_key_hex,
                issued_at,
                ..
            }
            | Self::PairingResponse {
                schema,
                host_ref,
                host_public_key_hex,
                device_public_key_hex,
                issued_at,
                ..
            }
            | Self::ScopedGrant {
                schema,
                host_ref,
                host_public_key_hex,
                device_public_key_hex,
                issued_at,
                ..
            }
            | Self::GrantRenewal {
                schema,
                host_ref,
                host_public_key_hex,
                device_public_key_hex,
                issued_at,
                ..
            }
            | Self::GrantRevocation {
                schema,
                host_ref,
                host_public_key_hex,
                device_public_key_hex,
                issued_at,
                ..
            } => (
                schema,
                host_ref,
                host_public_key_hex,
                device_public_key_hex,
                *issued_at,
            ),
        }
    }

    fn lifecycle_binding(&self) -> Option<(&str, &str, &str, &str, &str, u64, u64)> {
        match self {
            Self::ScopedGrant {
                host_ref,
                host_public_key_hex,
                sarah_public_key_hex,
                device_public_key_hex,
                issued_at,
                grant_ref,
                generation,
                ..
            }
            | Self::GrantRenewal {
                host_ref,
                host_public_key_hex,
                sarah_public_key_hex,
                device_public_key_hex,
                issued_at,
                grant_ref,
                generation,
                ..
            }
            | Self::GrantRevocation {
                host_ref,
                host_public_key_hex,
                sarah_public_key_hex,
                device_public_key_hex,
                issued_at,
                grant_ref,
                generation,
                ..
            } => Some((
                host_ref,
                host_public_key_hex,
                sarah_public_key_hex,
                device_public_key_hex,
                grant_ref,
                *generation,
                *issued_at,
            )),
            Self::PairingRequest { .. }
            | Self::PairingChallenge { .. }
            | Self::PairingResponse { .. } => None,
        }
    }

    pub fn validate_private_binding(
        &self,
        sender_public_key_hex: &str,
        recipient_public_key_hex: &str,
    ) -> Result<(), Issue31NostrError> {
        self.validate()?;
        let (_, _, host_key, device_key, _) = self.base();
        let device_authored = matches!(
            self,
            Self::PairingRequest { .. } | Self::PairingResponse { .. }
        );
        let expected_sender = if device_authored {
            device_key
        } else {
            host_key
        };
        let expected_recipient = if device_authored {
            host_key
        } else {
            device_key
        };
        if sender_public_key_hex != expected_sender
            || recipient_public_key_hex != expected_recipient
        {
            return Err(Issue31NostrError::Invalid(
                "pairing record signer or recipient does not match its binding".into(),
            ));
        }
        Ok(())
    }

    pub fn device_public_key_hex(&self) -> &str {
        let (_, _, _, device_public_key_hex, _) = self.base();
        device_public_key_hex
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Issue31PairingEvent {
    pub event_id: String,
    pub record: Issue31PairingRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Issue31GrantStatus {
    Active,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue31GrantState {
    pub grant_ref: String,
    pub host_ref: String,
    pub host_public_key_hex: String,
    pub sarah_public_key_hex: String,
    pub device_public_key_hex: String,
    pub generation: u64,
    pub status: Issue31GrantStatus,
    pub scopes: Vec<Issue31PairingScope>,
    pub expires_at: Option<u64>,
    pub issued_at: u64,
    pub source_event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Issue31GrantProjection {
    pub grant_ref: String,
    pub device_fingerprint: String,
    pub generation: u64,
    pub status: String,
    pub scopes: Vec<Issue31PairingScope>,
    pub expires_at: Option<u64>,
    pub source_event_id: String,
}

pub fn fold_issue31_grant(
    events: &[Issue31PairingEvent],
    grant_ref: &str,
) -> Result<Option<Issue31GrantState>, Issue31NostrError> {
    validate_ref_value(grant_ref, "grant")?;
    let mut unique_events: BTreeMap<&str, &Issue31PairingRecord> = BTreeMap::new();
    for event in events {
        if !valid_hex64(&event.event_id) {
            return Err(Issue31NostrError::Invalid(
                "invalid pairing event id".into(),
            ));
        }
        event.record.validate()?;
        if let Some(prior) = unique_events.get(event.event_id.as_str()) {
            if **prior != event.record {
                return Err(Issue31NostrError::Invalid(format!(
                    "Issue 31 event {} has conflicting records",
                    event.event_id
                )));
            }
        } else {
            unique_events.insert(&event.event_id, &event.record);
        }
    }

    let mut candidates: Vec<(&str, &Issue31PairingRecord)> = unique_events
        .iter()
        .map(|(event_id, record)| (*event_id, *record))
        .filter(|(_, record)| {
            record
                .lifecycle_binding()
                .is_some_and(|(_, _, _, _, candidate_grant_ref, _, _)| {
                    candidate_grant_ref == grant_ref
                })
        })
        .collect();
    candidates.sort_by(|(left_id, left), (right_id, right)| {
        let left_generation = left
            .lifecycle_binding()
            .map(|(_, _, _, _, _, generation, _)| generation)
            .unwrap_or_default();
        let right_generation = right
            .lifecycle_binding()
            .map(|(_, _, _, _, _, generation, _)| generation)
            .unwrap_or_default();
        (left_generation, *left_id).cmp(&(right_generation, *right_id))
    });

    let Some((_, identity_record)) = candidates.first() else {
        return Ok(None);
    };
    let (identity_host_ref, identity_host_key, identity_sarah_key, identity_device_key, _, _, _) =
        identity_record
            .lifecycle_binding()
            .ok_or_else(|| Issue31NostrError::Invalid("invalid grant lifecycle record".into()))?;
    if candidates.iter().any(|(_, record)| {
        record.lifecycle_binding().is_some_and(
            |(host_ref, host_key, sarah_key, device_key, _, _, _)| {
                host_ref != identity_host_ref
                    || host_key != identity_host_key
                    || sarah_key != identity_sarah_key
                    || device_key != identity_device_key
            },
        )
    }) {
        return Err(Issue31NostrError::Invalid(format!(
            "Issue 31 grant {grant_ref} has an identity fork"
        )));
    }

    if let Some((
        event_id,
        Issue31PairingRecord::GrantRevocation {
            host_ref,
            host_public_key_hex,
            sarah_public_key_hex,
            device_public_key_hex,
            issued_at,
            generation,
            ..
        },
    )) = candidates
        .iter()
        .rev()
        .find(|(_, record)| matches!(record, Issue31PairingRecord::GrantRevocation { .. }))
        .copied()
    {
        return Ok(Some(Issue31GrantState {
            grant_ref: grant_ref.to_string(),
            host_ref: host_ref.clone(),
            host_public_key_hex: host_public_key_hex.clone(),
            sarah_public_key_hex: sarah_public_key_hex.clone(),
            device_public_key_hex: device_public_key_hex.clone(),
            generation: *generation,
            status: Issue31GrantStatus::Revoked,
            scopes: Vec::new(),
            expires_at: None,
            issued_at: *issued_at,
            source_event_id: event_id.to_string(),
        }));
    }

    let mut records_by_generation: BTreeMap<u64, Vec<&Issue31PairingRecord>> = BTreeMap::new();
    for (_, record) in &candidates {
        let generation = record
            .lifecycle_binding()
            .map(|(_, _, _, _, _, generation, _)| generation)
            .ok_or_else(|| Issue31NostrError::Invalid("invalid grant lifecycle record".into()))?;
        records_by_generation
            .entry(generation)
            .or_default()
            .push(record);
    }
    for (generation, records) in records_by_generation {
        if records
            .first()
            .is_some_and(|first| records.iter().skip(1).any(|record| *record != *first))
        {
            return Err(Issue31NostrError::Invalid(format!(
                "Issue 31 grant {grant_ref} forks at generation {generation}"
            )));
        }
    }

    let mut state: Option<Issue31GrantState> = None;
    for (event_id, record) in candidates {
        match (state.as_ref(), record) {
            (
                None,
                Issue31PairingRecord::ScopedGrant {
                    host_ref,
                    host_public_key_hex,
                    sarah_public_key_hex,
                    device_public_key_hex,
                    issued_at,
                    generation,
                    scopes,
                    expires_at,
                    ..
                },
            ) => {
                validate_scoped_grant_pairing_chain(record, &unique_events)?;
                state = Some(Issue31GrantState {
                    grant_ref: grant_ref.to_string(),
                    host_ref: host_ref.clone(),
                    host_public_key_hex: host_public_key_hex.clone(),
                    sarah_public_key_hex: sarah_public_key_hex.clone(),
                    device_public_key_hex: device_public_key_hex.clone(),
                    generation: *generation,
                    status: Issue31GrantStatus::Active,
                    scopes: scopes.clone(),
                    expires_at: Some(*expires_at),
                    issued_at: *issued_at,
                    source_event_id: event_id.to_string(),
                });
            }
            (
                Some(prior),
                Issue31PairingRecord::GrantRenewal {
                    host_ref,
                    host_public_key_hex,
                    sarah_public_key_hex,
                    device_public_key_hex,
                    issued_at,
                    previous_grant_event_id,
                    prior_generation,
                    generation,
                    scopes,
                    expires_at,
                    ..
                },
            ) if previous_grant_event_id == &prior.source_event_id
                && *prior_generation == prior.generation
                && *generation == prior.generation.saturating_add(1)
                && host_ref == &prior.host_ref
                && host_public_key_hex == &prior.host_public_key_hex
                && sarah_public_key_hex == &prior.sarah_public_key_hex
                && device_public_key_hex == &prior.device_public_key_hex =>
            {
                state = Some(Issue31GrantState {
                    grant_ref: grant_ref.to_string(),
                    host_ref: host_ref.clone(),
                    host_public_key_hex: host_public_key_hex.clone(),
                    sarah_public_key_hex: sarah_public_key_hex.clone(),
                    device_public_key_hex: device_public_key_hex.clone(),
                    generation: *generation,
                    status: Issue31GrantStatus::Active,
                    scopes: scopes.clone(),
                    expires_at: Some(*expires_at),
                    issued_at: *issued_at,
                    source_event_id: event_id.to_string(),
                });
            }
            (None, Issue31PairingRecord::GrantRenewal { .. }) => {
                return Err(Issue31NostrError::Invalid(format!(
                    "Issue 31 grant {grant_ref} renewal has no initial grant"
                )));
            }
            (Some(_), Issue31PairingRecord::GrantRenewal { .. }) => {
                return Err(Issue31NostrError::Invalid(format!(
                    "Issue 31 grant {grant_ref} renewal lineage is invalid"
                )));
            }
            (Some(_), Issue31PairingRecord::ScopedGrant { .. }) => {
                return Err(Issue31NostrError::Invalid(format!(
                    "Issue 31 grant {grant_ref} has more than one initial grant"
                )));
            }
            (_, Issue31PairingRecord::GrantRevocation { .. }) => {}
            (
                _,
                Issue31PairingRecord::PairingRequest { .. }
                | Issue31PairingRecord::PairingChallenge { .. }
                | Issue31PairingRecord::PairingResponse { .. },
            ) => {
                return Err(Issue31NostrError::Invalid(
                    "non-lifecycle pairing record entered grant projection".into(),
                ));
            }
        }
    }
    Ok(state)
}

fn validate_scoped_grant_pairing_chain(
    grant: &Issue31PairingRecord,
    records_by_event_id: &BTreeMap<&str, &Issue31PairingRecord>,
) -> Result<(), Issue31NostrError> {
    let Issue31PairingRecord::ScopedGrant {
        host_ref,
        host_public_key_hex,
        device_public_key_hex,
        issued_at: grant_issued_at,
        pairing_response_event_id,
        scopes,
        ..
    } = grant
    else {
        return Err(Issue31NostrError::Invalid(
            "pairing-chain validation requires a scoped grant".into(),
        ));
    };
    let Some(Issue31PairingRecord::PairingResponse {
        host_ref: response_host_ref,
        host_public_key_hex: response_host_key,
        device_public_key_hex: response_device_key,
        issued_at: response_issued_at,
        pairing_challenge_event_id,
        challenge: response_challenge,
        ..
    }) = records_by_event_id
        .get(pairing_response_event_id.as_str())
        .copied()
    else {
        return Err(Issue31NostrError::Invalid(
            "Issue 31 scoped grant has no pairing response".into(),
        ));
    };
    let Some(Issue31PairingRecord::PairingChallenge {
        host_ref: challenge_host_ref,
        host_public_key_hex: challenge_host_key,
        device_public_key_hex: challenge_device_key,
        issued_at: challenge_issued_at,
        pairing_request_event_id,
        challenge,
        ..
    }) = records_by_event_id
        .get(pairing_challenge_event_id.as_str())
        .copied()
    else {
        return Err(Issue31NostrError::Invalid(
            "Issue 31 pairing response has no challenge".into(),
        ));
    };
    let Some(Issue31PairingRecord::PairingRequest {
        host_ref: request_host_ref,
        host_public_key_hex: request_host_key,
        device_public_key_hex: request_device_key,
        issued_at: request_issued_at,
        requested_scopes,
        ..
    }) = records_by_event_id
        .get(pairing_request_event_id.as_str())
        .copied()
    else {
        return Err(Issue31NostrError::Invalid(
            "Issue 31 pairing challenge has no request".into(),
        ));
    };
    for (candidate_host_ref, candidate_host_key, candidate_device_key) in [
        (response_host_ref, response_host_key, response_device_key),
        (challenge_host_ref, challenge_host_key, challenge_device_key),
        (request_host_ref, request_host_key, request_device_key),
    ] {
        if candidate_host_ref != host_ref
            || candidate_host_key != host_public_key_hex
            || candidate_device_key != device_public_key_hex
        {
            return Err(Issue31NostrError::Invalid(
                "Issue 31 pairing chain changes host or device identity".into(),
            ));
        }
    }
    if response_challenge != challenge {
        return Err(Issue31NostrError::Invalid(
            "Issue 31 pairing response does not answer its challenge".into(),
        ));
    }
    if scopes.iter().any(|scope| !requested_scopes.contains(scope)) {
        return Err(Issue31NostrError::Invalid(
            "Issue 31 scoped grant exceeds requested scopes".into(),
        ));
    }
    if request_issued_at > challenge_issued_at
        || challenge_issued_at > response_issued_at
        || response_issued_at > grant_issued_at
    {
        return Err(Issue31NostrError::Invalid(
            "Issue 31 pairing chain time order is invalid".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Issue31CommandStatus {
    Completed,
    Failed,
    Refused,
    Stopped,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "recordType", rename_all = "snake_case", deny_unknown_fields)]
pub enum Issue31CommandRecord {
    CommandIntent {
        schema: String,
        #[serde(rename = "hostRef")]
        host_ref: String,
        #[serde(rename = "hostPublicKeyHex")]
        host_public_key_hex: String,
        #[serde(rename = "devicePublicKeyHex")]
        device_public_key_hex: String,
        #[serde(rename = "grantRef")]
        grant_ref: String,
        #[serde(rename = "actionRef")]
        action_ref: String,
        #[serde(rename = "idempotencyRef")]
        idempotency_ref: String,
        #[serde(rename = "expectedGeneration")]
        expected_generation: u64,
        #[serde(rename = "argumentsRef")]
        arguments_ref: String,
        #[serde(rename = "issuedAt")]
        issued_at: u64,
        #[serde(rename = "expiresAt")]
        expires_at: u64,
    },
    CommandResult {
        schema: String,
        #[serde(rename = "hostRef")]
        host_ref: String,
        #[serde(rename = "hostPublicKeyHex")]
        host_public_key_hex: String,
        #[serde(rename = "devicePublicKeyHex")]
        device_public_key_hex: String,
        #[serde(rename = "grantRef")]
        grant_ref: String,
        #[serde(rename = "intentEventId")]
        intent_event_id: String,
        #[serde(rename = "actionRef")]
        action_ref: String,
        #[serde(rename = "idempotencyRef")]
        idempotency_ref: String,
        #[serde(rename = "expectedGeneration")]
        expected_generation: u64,
        status: Issue31CommandStatus,
        #[serde(rename = "outcomeRef")]
        outcome_ref: String,
        #[serde(rename = "reasonRef", default, skip_serializing_if = "Option::is_none")]
        reason_ref: Option<String>,
        #[serde(rename = "completedAt")]
        completed_at: u64,
    },
}

impl Issue31CommandRecord {
    pub fn decode(bytes: &[u8]) -> Result<Self, Issue31NostrError> {
        if bytes.len() > 64 * 1024 {
            return Err(Issue31NostrError::Invalid(
                "command record exceeds the record budget".into(),
            ));
        }
        let record: Self = serde_json::from_slice(bytes)
            .map_err(|error| Issue31NostrError::Decode(error.to_string()))?;
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), Issue31NostrError> {
        let (
            schema,
            host_ref,
            host_key,
            device_key,
            grant_ref,
            action_ref,
            idempotency_ref,
            generation,
        ) = self.binding();
        if schema != ISSUE31_COMMAND_SCHEMA
            || !valid_ref(host_ref)
            || !valid_hex64(host_key)
            || !valid_hex64(device_key)
            || !valid_ref(grant_ref)
            || !valid_ref(action_ref)
            || !valid_ref(idempotency_ref)
            || generation == 0
        {
            return Err(Issue31NostrError::Invalid("invalid command binding".into()));
        }
        match self {
            Self::CommandIntent {
                arguments_ref,
                issued_at,
                expires_at,
                ..
            } => {
                validate_ref_value(arguments_ref, "arguments")?;
                validate_lifetime(*issued_at, *expires_at)
            }
            Self::CommandResult {
                intent_event_id,
                outcome_ref,
                reason_ref,
                ..
            } => {
                validate_hex_value(intent_event_id, "intent event")?;
                validate_ref_value(outcome_ref, "outcome")?;
                if let Some(reason_ref) = reason_ref {
                    validate_ref_value(reason_ref, "reason")?;
                }
                Ok(())
            }
        }
    }

    fn binding(&self) -> (&str, &str, &str, &str, &str, &str, &str, u64) {
        match self {
            Self::CommandIntent {
                schema,
                host_ref,
                host_public_key_hex,
                device_public_key_hex,
                grant_ref,
                action_ref,
                idempotency_ref,
                expected_generation,
                ..
            }
            | Self::CommandResult {
                schema,
                host_ref,
                host_public_key_hex,
                device_public_key_hex,
                grant_ref,
                action_ref,
                idempotency_ref,
                expected_generation,
                ..
            } => (
                schema,
                host_ref,
                host_public_key_hex,
                device_public_key_hex,
                grant_ref,
                action_ref,
                idempotency_ref,
                *expected_generation,
            ),
        }
    }

    pub fn validate_private_binding(
        &self,
        sender_public_key_hex: &str,
        recipient_public_key_hex: &str,
    ) -> Result<(), Issue31NostrError> {
        self.validate()?;
        let (_, _, host_key, device_key, _, _, _, _) = self.binding();
        let device_authored = matches!(self, Self::CommandIntent { .. });
        let expected_sender = if device_authored {
            device_key
        } else {
            host_key
        };
        let expected_recipient = if device_authored {
            host_key
        } else {
            device_key
        };
        if sender_public_key_hex != expected_sender
            || recipient_public_key_hex != expected_recipient
        {
            return Err(Issue31NostrError::Invalid(
                "command record signer or recipient does not match its binding".into(),
            ));
        }
        Ok(())
    }

    pub fn device_public_key_hex(&self) -> &str {
        let (_, _, _, device_public_key_hex, _, _, _, _) = self.binding();
        device_public_key_hex
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue31CommandEvent {
    pub event_id: String,
    pub record: Issue31CommandRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue31CommandState {
    pub intent_event_id: String,
    pub intent: Issue31CommandRecord,
    pub result_event_id: Option<String>,
    pub result: Option<Issue31CommandRecord>,
}

pub fn reconcile_issue31_commands(
    events: &[Issue31CommandEvent],
) -> Result<Vec<Issue31CommandState>, Issue31NostrError> {
    let mut intents: BTreeMap<String, (&String, &Issue31CommandRecord)> = BTreeMap::new();
    let mut results: BTreeMap<String, (&String, &Issue31CommandRecord)> = BTreeMap::new();
    for event in events {
        event.record.validate()?;
        if !valid_hex64(&event.event_id) {
            return Err(Issue31NostrError::Invalid(
                "invalid command event id".into(),
            ));
        }
        let (_, _, _, _, _, _, idempotency_ref, _) = event.record.binding();
        let target = match &event.record {
            Issue31CommandRecord::CommandIntent { .. } => &mut intents,
            Issue31CommandRecord::CommandResult { .. } => &mut results,
        };
        if let Some((prior_event_id, prior)) = target.get_mut(idempotency_ref) {
            if *prior != &event.record {
                return Err(Issue31NostrError::Invalid(format!(
                    "conflicting records for {idempotency_ref}"
                )));
            }
            if event.event_id.as_str() < prior_event_id.as_str() {
                *prior_event_id = &event.event_id;
            }
        } else {
            target.insert(
                idempotency_ref.to_string(),
                (&event.event_id, &event.record),
            );
        }
    }

    let mut states = Vec::new();
    for (idempotency_ref, (intent_event_id, intent)) in intents {
        let result = results.get(&idempotency_ref).copied();
        if let Some((_, result_record)) = result {
            let Issue31CommandRecord::CommandResult {
                intent_event_id: result_intent_id,
                ..
            } = result_record
            else {
                return Err(Issue31NostrError::Invalid(
                    "result map contained intent".into(),
                ));
            };
            if result_intent_id != intent_event_id || result_record.binding() != intent.binding() {
                return Err(Issue31NostrError::Invalid(format!(
                    "command result does not match {idempotency_ref}"
                )));
            }
        }
        states.push(Issue31CommandState {
            intent_event_id: intent_event_id.clone(),
            intent: intent.clone(),
            result_event_id: result.map(|(event_id, _)| event_id.clone()),
            result: result.map(|(_, record)| record.clone()),
        });
    }
    Ok(states)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Issue31HostConfiguration {
    pub host_ref: String,
    pub host_public_key_hex: String,
    pub sarah_public_key_hex: String,
    pub display_name: String,
    pub relay_urls: Vec<String>,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue31CommandExecution {
    pub status: Issue31CommandStatus,
    pub outcome_ref: String,
    pub reason_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Issue31HostController {
    configuration: Issue31HostConfiguration,
    #[serde(skip)]
    admitted_device_scopes: BTreeMap<String, BTreeSet<Issue31PairingScope>>,
    pairing_events: Vec<Issue31PairingEvent>,
    processed_pairing_event_ids: BTreeSet<String>,
    processed_command_event_ids: BTreeSet<String>,
    command_results: BTreeMap<String, (Issue31CommandRecord, Issue31CommandRecord)>,
}

const MAX_ISSUE31_PAIRING_EVENTS: usize = 4_096;
const MAX_ISSUE31_PROCESSED_EVENTS: usize = 4_096;
const MAX_ISSUE31_COMMAND_RESULTS: usize = 1_024;

impl Issue31HostController {
    pub fn new(configuration: Issue31HostConfiguration) -> Result<Self, Issue31NostrError> {
        Issue31HostDiscovery {
            schema: ISSUE31_HOST_DISCOVERY_SCHEMA.into(),
            host_ref: configuration.host_ref.clone(),
            host_public_key_hex: configuration.host_public_key_hex.clone(),
            sarah_public_key_hex: configuration.sarah_public_key_hex.clone(),
            display_name: configuration.display_name.clone(),
            protocols: vec![ISSUE31_PAIRING_SCHEMA.into(), ISSUE31_COMMAND_SCHEMA.into()],
            relay_urls: configuration.relay_urls.clone(),
            generation: configuration.generation,
            issued_at: 1,
            expires_at: 2,
        }
        .validate()?;
        Ok(Self {
            configuration,
            admitted_device_scopes: BTreeMap::new(),
            pairing_events: Vec::new(),
            processed_pairing_event_ids: BTreeSet::new(),
            processed_command_event_ids: BTreeSet::new(),
            command_results: BTreeMap::new(),
        })
    }

    pub fn discovery(
        &self,
        issued_at: u64,
        expires_at: u64,
    ) -> Result<Issue31HostDiscovery, Issue31NostrError> {
        let discovery = Issue31HostDiscovery {
            schema: ISSUE31_HOST_DISCOVERY_SCHEMA.into(),
            host_ref: self.configuration.host_ref.clone(),
            host_public_key_hex: self.configuration.host_public_key_hex.clone(),
            sarah_public_key_hex: self.configuration.sarah_public_key_hex.clone(),
            display_name: self.configuration.display_name.clone(),
            protocols: vec![ISSUE31_PAIRING_SCHEMA.into(), ISSUE31_COMMAND_SCHEMA.into()],
            relay_urls: self.configuration.relay_urls.clone(),
            generation: self.configuration.generation,
            issued_at,
            expires_at,
        };
        discovery.validate()?;
        Ok(discovery)
    }

    pub fn matches_configuration(&self, configuration: &Issue31HostConfiguration) -> bool {
        &self.configuration == configuration
    }

    pub fn set_admitted_device_policy(
        &mut self,
        public_keys: Vec<String>,
        approved_scopes: Vec<Issue31PairingScope>,
    ) -> Result<(), Issue31NostrError> {
        if public_keys.is_empty()
            || public_keys.len() > 32
            || public_keys
                .iter()
                .any(|public_key| !valid_hex64(public_key))
        {
            return Err(Issue31NostrError::Invalid(
                "Issue 31 device allowlist must contain one to 32 lowercase public keys".into(),
            ));
        }
        let public_key_count = public_keys.len();
        let admitted = public_keys.into_iter().collect::<BTreeSet<_>>();
        if admitted.len() != public_key_count {
            return Err(Issue31NostrError::Invalid(
                "Issue 31 device allowlist contains duplicates".into(),
            ));
        }
        validate_scopes(&approved_scopes, "owner-approved scopes")?;
        let approved_scopes = approved_scopes.into_iter().collect::<BTreeSet<_>>();
        self.admitted_device_scopes = admitted
            .into_iter()
            .map(|public_key| (public_key, approved_scopes.clone()))
            .collect();
        Ok(())
    }

    pub fn admitted_device_fingerprints(&self) -> Vec<String> {
        self.admitted_device_scopes
            .keys()
            .map(|public_key| {
                let digest = format!("{:x}", Sha256::digest(public_key.as_bytes()));
                digest[..16].to_ascii_uppercase()
            })
            .collect()
    }

    pub fn grant_projections(
        &self,
        now: u64,
    ) -> Result<Vec<Issue31GrantProjection>, Issue31NostrError> {
        let grant_refs = self
            .pairing_events
            .iter()
            .filter_map(|event| match &event.record {
                Issue31PairingRecord::ScopedGrant { grant_ref, .. }
                | Issue31PairingRecord::GrantRenewal { grant_ref, .. }
                | Issue31PairingRecord::GrantRevocation { grant_ref, .. } => {
                    Some(grant_ref.clone())
                }
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        grant_refs
            .into_iter()
            .map(|grant_ref| {
                let state =
                    fold_issue31_grant(&self.pairing_events, &grant_ref)?.ok_or_else(|| {
                        Issue31NostrError::Invalid(
                            "durable grant projection lost its lifecycle state".into(),
                        )
                    })?;
                let device_digest = format!(
                    "{:x}",
                    Sha256::digest(state.device_public_key_hex.as_bytes())
                );
                let status = match state.status {
                    Issue31GrantStatus::Revoked => "revoked",
                    Issue31GrantStatus::Active
                        if state.expires_at.is_some_and(|expires_at| now >= expires_at) =>
                    {
                        "expired"
                    }
                    Issue31GrantStatus::Active => "active",
                };
                Ok(Issue31GrantProjection {
                    grant_ref: state.grant_ref,
                    device_fingerprint: device_digest[..16].to_ascii_uppercase(),
                    generation: state.generation,
                    status: status.into(),
                    scopes: state.scopes,
                    expires_at: state.expires_at,
                    source_event_id: state.source_event_id,
                })
            })
            .collect()
    }

    pub fn pairing_event_was_processed(&self, event_id: &str) -> bool {
        self.processed_pairing_event_ids.contains(event_id)
    }

    pub fn validate_persisted_state(&self) -> Result<(), Issue31NostrError> {
        Self::new(self.configuration.clone())?;
        if self.pairing_events.len() > MAX_ISSUE31_PAIRING_EVENTS
            || self.processed_pairing_event_ids.len() > MAX_ISSUE31_PROCESSED_EVENTS
            || self.processed_command_event_ids.len() > MAX_ISSUE31_PROCESSED_EVENTS
            || self.command_results.len() > MAX_ISSUE31_COMMAND_RESULTS
        {
            return Err(Issue31NostrError::Invalid(
                "persisted Issue 31 host state exceeds its bounds".into(),
            ));
        }
        for event in &self.pairing_events {
            if !valid_hex64(&event.event_id) {
                return Err(Issue31NostrError::Invalid(
                    "persisted pairing event id is invalid".into(),
                ));
            }
            event.record.validate()?;
            ensure_pairing_targets_host(&event.record, &self.configuration)?;
        }
        if self
            .processed_pairing_event_ids
            .iter()
            .chain(&self.processed_command_event_ids)
            .any(|event_id| !valid_hex64(event_id))
        {
            return Err(Issue31NostrError::Invalid(
                "persisted processed event id is invalid".into(),
            ));
        }
        for (idempotency_ref, (intent, result)) in &self.command_results {
            validate_ref_value(idempotency_ref, "persisted idempotency")?;
            intent.validate()?;
            result.validate()?;
            let Issue31CommandRecord::CommandIntent {
                idempotency_ref: intent_idempotency_ref,
                issued_at,
                ..
            } = intent
            else {
                return Err(Issue31NostrError::Invalid(
                    "persisted command result omitted its intent".into(),
                ));
            };
            let Issue31CommandRecord::CommandResult {
                intent_event_id,
                idempotency_ref: result_idempotency_ref,
                completed_at,
                ..
            } = result
            else {
                return Err(Issue31NostrError::Invalid(
                    "persisted command result stored a second intent".into(),
                ));
            };
            if intent.binding() != result.binding()
                || intent_idempotency_ref != idempotency_ref
                || result_idempotency_ref != idempotency_ref
                || completed_at < issued_at
                || !self.processed_command_event_ids.contains(intent_event_id)
            {
                return Err(Issue31NostrError::Invalid(
                    "persisted command result changes its intent binding".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn record_emitted_pairing(
        &mut self,
        event_id: String,
        record: Issue31PairingRecord,
    ) -> Result<(), Issue31NostrError> {
        ensure_pairing_targets_host(&record, &self.configuration)?;
        record.validate_private_binding(
            &self.configuration.host_public_key_hex,
            record_device_public_key(&record),
        )?;
        self.insert_pairing_event(Issue31PairingEvent { event_id, record })
    }

    pub fn handle_pairing_event(
        &mut self,
        event: Issue31PairingEvent,
        now: u64,
    ) -> Result<Option<Issue31PairingRecord>, Issue31NostrError> {
        if self.processed_pairing_event_ids.contains(&event.event_id) {
            return Ok(None);
        }
        event.record.validate()?;
        ensure_pairing_targets_host(&event.record, &self.configuration)?;
        if matches!(
            &event.record,
            Issue31PairingRecord::PairingRequest {
                device_public_key_hex,
                requested_scopes,
                ..
            } if self
                .admitted_device_scopes
                .get(device_public_key_hex)
                .is_none_or(|approved| requested_scopes.iter().all(|scope| !approved.contains(scope)))
        ) {
            return Ok(None);
        }
        let outbound = match &event.record {
            Issue31PairingRecord::PairingRequest {
                device_public_key_hex,
                issued_at,
                expires_at,
                ..
            } => {
                require_live_record(*issued_at, *expires_at, now, "pairing request")?;
                let challenge_seed = rand::random::<[u8; 32]>();
                let challenge = format!("{:x}", Sha256::digest(challenge_seed));
                let event_digest = digest_ref_suffix(event.event_id.as_bytes());
                Some(Issue31PairingRecord::PairingChallenge {
                    schema: ISSUE31_PAIRING_SCHEMA.into(),
                    host_ref: self.configuration.host_ref.clone(),
                    host_public_key_hex: self.configuration.host_public_key_hex.clone(),
                    device_public_key_hex: device_public_key_hex.clone(),
                    issued_at: now,
                    pairing_challenge_ref: format!("pairing_challenge.omega.{event_digest}"),
                    pairing_request_event_id: event.event_id.clone(),
                    challenge,
                    expires_at: (*expires_at).min(now.saturating_add(600)),
                })
            }
            Issue31PairingRecord::PairingResponse {
                device_public_key_hex,
                issued_at,
                pairing_challenge_event_id,
                challenge: response_challenge,
                expires_at,
                ..
            } => {
                require_live_record(*issued_at, *expires_at, now, "pairing response")?;
                let challenge = self
                    .pairing_events
                    .iter()
                    .find(|candidate| candidate.event_id == *pairing_challenge_event_id);
                let Some(Issue31PairingEvent {
                    record:
                        Issue31PairingRecord::PairingChallenge {
                            pairing_request_event_id,
                            challenge,
                            expires_at: challenge_expires_at,
                            ..
                        },
                    ..
                }) = challenge
                else {
                    return Err(Issue31NostrError::Invalid(
                        "pairing response references an unknown challenge".into(),
                    ));
                };
                if response_challenge != challenge || now >= *challenge_expires_at {
                    return Err(Issue31NostrError::Invalid(
                        "pairing response challenge is wrong or expired".into(),
                    ));
                }
                let request = self
                    .pairing_events
                    .iter()
                    .find(|candidate| candidate.event_id == *pairing_request_event_id);
                let Some(Issue31PairingEvent {
                    record:
                        Issue31PairingRecord::PairingRequest {
                            requested_scopes,
                            device_public_key_hex: request_device_key,
                            ..
                        },
                    ..
                }) = request
                else {
                    return Err(Issue31NostrError::Invalid(
                        "pairing challenge references an unknown request".into(),
                    ));
                };
                if request_device_key != device_public_key_hex {
                    return Err(Issue31NostrError::Invalid(
                        "pairing response changes device identity".into(),
                    ));
                }
                let approved_scopes = self
                    .admitted_device_scopes
                    .get(device_public_key_hex)
                    .ok_or_else(|| {
                        Issue31NostrError::Invalid("device no longer has owner admission".into())
                    })?;
                let scopes = requested_scopes
                    .iter()
                    .copied()
                    .filter(|scope| approved_scopes.contains(scope))
                    .collect::<Vec<_>>();
                validate_scopes(&scopes, "owner-approved grant scopes")?;
                let event_digest = digest_ref_suffix(event.event_id.as_bytes());
                Some(Issue31PairingRecord::ScopedGrant {
                    schema: ISSUE31_PAIRING_SCHEMA.into(),
                    host_ref: self.configuration.host_ref.clone(),
                    host_public_key_hex: self.configuration.host_public_key_hex.clone(),
                    sarah_public_key_hex: self.configuration.sarah_public_key_hex.clone(),
                    device_public_key_hex: device_public_key_hex.clone(),
                    issued_at: now,
                    pairing_response_event_id: event.event_id.clone(),
                    grant_ref: format!("grant.omega.{event_digest}"),
                    generation: 1,
                    scopes,
                    expires_at: now.saturating_add(24 * 60 * 60),
                })
            }
            Issue31PairingRecord::PairingChallenge { .. }
            | Issue31PairingRecord::ScopedGrant { .. }
            | Issue31PairingRecord::GrantRenewal { .. }
            | Issue31PairingRecord::GrantRevocation { .. } => None,
        };
        self.insert_pairing_event(event)?;
        Ok(outbound)
    }

    pub fn renew_grant(
        &self,
        grant_ref: &str,
        scopes: Vec<Issue31PairingScope>,
        now: u64,
        expires_at: u64,
    ) -> Result<Issue31PairingRecord, Issue31NostrError> {
        validate_scopes(&scopes, "renewal scopes")?;
        validate_lifetime(now, expires_at)?;
        let prior = fold_issue31_grant(&self.pairing_events, grant_ref)?
            .ok_or_else(|| Issue31NostrError::Invalid("cannot renew an unknown grant".into()))?;
        if prior.status != Issue31GrantStatus::Active
            || scopes.iter().any(|scope| !prior.scopes.contains(scope))
        {
            return Err(Issue31NostrError::Invalid(
                "renewal requires an active grant and cannot widen scope".into(),
            ));
        }
        Ok(Issue31PairingRecord::GrantRenewal {
            schema: ISSUE31_PAIRING_SCHEMA.into(),
            host_ref: prior.host_ref,
            host_public_key_hex: prior.host_public_key_hex,
            sarah_public_key_hex: prior.sarah_public_key_hex,
            device_public_key_hex: prior.device_public_key_hex,
            issued_at: now,
            grant_ref: grant_ref.to_string(),
            previous_grant_event_id: prior.source_event_id,
            prior_generation: prior.generation,
            generation: prior.generation.saturating_add(1),
            scopes,
            expires_at,
        })
    }

    pub fn revoke_grant(
        &self,
        grant_ref: &str,
        now: u64,
        reason_ref: Option<String>,
    ) -> Result<Issue31PairingRecord, Issue31NostrError> {
        let prior = fold_issue31_grant(&self.pairing_events, grant_ref)?
            .ok_or_else(|| Issue31NostrError::Invalid("cannot revoke an unknown grant".into()))?;
        if prior.status == Issue31GrantStatus::Revoked {
            return Err(Issue31NostrError::Invalid(
                "grant is already revoked".into(),
            ));
        }
        let record = Issue31PairingRecord::GrantRevocation {
            schema: ISSUE31_PAIRING_SCHEMA.into(),
            host_ref: prior.host_ref,
            host_public_key_hex: prior.host_public_key_hex,
            sarah_public_key_hex: prior.sarah_public_key_hex,
            device_public_key_hex: prior.device_public_key_hex,
            issued_at: now,
            grant_ref: grant_ref.to_string(),
            generation: prior.generation,
            reason_ref,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn handle_command_event<F>(
        &mut self,
        event: Issue31CommandEvent,
        now: u64,
        execute: F,
    ) -> Result<Option<Issue31CommandRecord>, Issue31NostrError>
    where
        F: FnOnce(&str, &str) -> Issue31CommandExecution,
    {
        if !valid_hex64(&event.event_id) {
            return Err(Issue31NostrError::Invalid(
                "Issue 31 command event id is invalid".into(),
            ));
        }
        if self.processed_command_event_ids.contains(&event.event_id) {
            return Ok(None);
        }
        event.record.validate()?;
        if self.processed_command_event_ids.len() >= MAX_ISSUE31_PROCESSED_EVENTS {
            return Err(Issue31NostrError::Invalid(
                "Issue 31 processed command event bound is exhausted".into(),
            ));
        }
        let Issue31CommandRecord::CommandIntent {
            host_ref,
            host_public_key_hex,
            device_public_key_hex,
            grant_ref,
            action_ref,
            idempotency_ref,
            expected_generation,
            arguments_ref,
            issued_at,
            expires_at,
            ..
        } = &event.record
        else {
            self.processed_command_event_ids.insert(event.event_id);
            return Ok(None);
        };
        if let Some((prior_intent, prior_result)) = self.command_results.get(idempotency_ref) {
            if prior_intent == &event.record {
                self.processed_command_event_ids.insert(event.event_id);
                return Ok(Some(prior_result.clone()));
            }
            return Err(Issue31NostrError::Invalid(format!(
                "idempotency ref {idempotency_ref} conflicts with an earlier command"
            )));
        }
        if self.command_results.len() >= MAX_ISSUE31_COMMAND_RESULTS {
            return Err(Issue31NostrError::Invalid(
                "Issue 31 command result bound is exhausted".into(),
            ));
        }

        let refusal = |reason: &str| Issue31CommandExecution {
            status: Issue31CommandStatus::Refused,
            outcome_ref: "outcome.omega.refused".into(),
            reason_ref: Some(reason.into()),
        };
        let execution = if host_ref != &self.configuration.host_ref
            || host_public_key_hex != &self.configuration.host_public_key_hex
        {
            refusal("reason.omega.host_binding_mismatch")
        } else if require_live_record(*issued_at, *expires_at, now, "command intent").is_err() {
            refusal("reason.omega.command_expired")
        } else {
            let grant = fold_issue31_grant(&self.pairing_events, grant_ref)?;
            match grant {
                Some(grant)
                    if grant.status == Issue31GrantStatus::Active
                        && grant.device_public_key_hex == *device_public_key_hex
                        && grant.generation == *expected_generation
                        && grant.expires_at.is_some_and(|expires_at| now < expires_at) =>
                {
                    match required_scope(action_ref) {
                        Some(scope) if grant.scopes.contains(&scope) => {
                            execute(action_ref, arguments_ref)
                        }
                        Some(_) => refusal("reason.omega.scope_denied"),
                        None => Issue31CommandExecution {
                            status: Issue31CommandStatus::Unavailable,
                            outcome_ref: "outcome.omega.unavailable".into(),
                            reason_ref: Some("reason.omega.action_unsupported".into()),
                        },
                    }
                }
                _ => refusal("reason.omega.grant_invalid"),
            }
        };
        let result = Issue31CommandRecord::CommandResult {
            schema: ISSUE31_COMMAND_SCHEMA.into(),
            host_ref: host_ref.clone(),
            host_public_key_hex: host_public_key_hex.clone(),
            device_public_key_hex: device_public_key_hex.clone(),
            grant_ref: grant_ref.clone(),
            intent_event_id: event.event_id.clone(),
            action_ref: action_ref.clone(),
            idempotency_ref: idempotency_ref.clone(),
            expected_generation: *expected_generation,
            status: execution.status,
            outcome_ref: execution.outcome_ref,
            reason_ref: execution.reason_ref,
            completed_at: now,
        };
        result.validate()?;
        self.processed_command_event_ids.insert(event.event_id);
        self.command_results
            .insert(idempotency_ref.clone(), (event.record, result.clone()));
        Ok(Some(result))
    }

    fn insert_pairing_event(
        &mut self,
        event: Issue31PairingEvent,
    ) -> Result<(), Issue31NostrError> {
        if !valid_hex64(&event.event_id) {
            return Err(Issue31NostrError::Invalid(
                "Issue 31 pairing event id is invalid".into(),
            ));
        }
        if let Some(prior) = self
            .pairing_events
            .iter()
            .find(|prior| prior.event_id == event.event_id)
        {
            if prior.record != event.record {
                return Err(Issue31NostrError::Invalid(format!(
                    "Issue 31 event {} has conflicting records",
                    event.event_id
                )));
            }
            return Ok(());
        }
        if self.pairing_events.len() >= MAX_ISSUE31_PAIRING_EVENTS
            || self.processed_pairing_event_ids.len() >= MAX_ISSUE31_PROCESSED_EVENTS
        {
            return Err(Issue31NostrError::Invalid(
                "Issue 31 pairing event bound is exhausted".into(),
            ));
        }
        self.processed_pairing_event_ids
            .insert(event.event_id.clone());
        self.pairing_events.push(event);
        Ok(())
    }
}

fn ensure_pairing_targets_host(
    record: &Issue31PairingRecord,
    configuration: &Issue31HostConfiguration,
) -> Result<(), Issue31NostrError> {
    let (_, host_ref, host_public_key_hex, _, _) = record.base();
    if host_ref != configuration.host_ref
        || host_public_key_hex != configuration.host_public_key_hex
    {
        return Err(Issue31NostrError::Invalid(
            "pairing record targets another host".into(),
        ));
    }
    let sarah_public_key_hex = match record {
        Issue31PairingRecord::ScopedGrant {
            sarah_public_key_hex,
            ..
        }
        | Issue31PairingRecord::GrantRenewal {
            sarah_public_key_hex,
            ..
        }
        | Issue31PairingRecord::GrantRevocation {
            sarah_public_key_hex,
            ..
        } => Some(sarah_public_key_hex),
        Issue31PairingRecord::PairingRequest { .. }
        | Issue31PairingRecord::PairingChallenge { .. }
        | Issue31PairingRecord::PairingResponse { .. } => None,
    };
    if sarah_public_key_hex.is_some_and(|key| key != &configuration.sarah_public_key_hex) {
        return Err(Issue31NostrError::Invalid(
            "pairing grant binds another Sarah identity".into(),
        ));
    }
    Ok(())
}

fn record_device_public_key(record: &Issue31PairingRecord) -> &str {
    let (_, _, _, device_public_key_hex, _) = record.base();
    device_public_key_hex
}

fn require_live_record(
    issued_at: u64,
    expires_at: u64,
    now: u64,
    label: &str,
) -> Result<(), Issue31NostrError> {
    validate_lifetime(issued_at, expires_at)?;
    if issued_at > now.saturating_add(300) || now >= expires_at {
        return Err(Issue31NostrError::Invalid(format!(
            "{label} is future-dated or expired"
        )));
    }
    Ok(())
}

fn required_scope(action_ref: &str) -> Option<Issue31PairingScope> {
    match action_ref {
        "action.omega.send_message" => Some(Issue31PairingScope::SendMessage),
        "action.omega.interrupt_turn" => Some(Issue31PairingScope::InterruptTurn),
        "action.omega.full_auto.stop"
        | "action.omega.full_auto.pause"
        | "action.omega.full_auto.resume" => Some(Issue31PairingScope::ControlFullAuto),
        "action.omega.provider_handoff" => Some(Issue31PairingScope::RequestProviderHandoff),
        "action.omega.community.act" => Some(Issue31PairingScope::ActInCommunity),
        _ => None,
    }
}

fn digest_ref_suffix(bytes: &[u8]) -> String {
    let digest = format!("{:x}", Sha256::digest(bytes));
    digest[..24].to_string()
}

#[cfg(test)]
pub(crate) fn restart_fixture() -> (
    Issue31HostConfiguration,
    Issue31HostController,
    String,
    String,
) {
    let host_public_key_hex = "1".repeat(64);
    let device_public_key_hex = "2".repeat(64);
    let configuration = Issue31HostConfiguration {
        host_ref: "omega.host.local".into(),
        host_public_key_hex: host_public_key_hex.clone(),
        sarah_public_key_hex: "3".repeat(64),
        display_name: "Local Omega".into(),
        relay_urls: vec!["wss://relay.example.com".into()],
        generation: 1,
    };
    let mut controller = Issue31HostController::new(configuration.clone()).expect("controller");
    controller
        .set_admitted_device_policy(
            vec![device_public_key_hex.clone()],
            vec![Issue31PairingScope::ControlFullAuto],
        )
        .expect("admit device");
    let challenge = controller
        .handle_pairing_event(
            Issue31PairingEvent {
                event_id: "a".repeat(64),
                record: Issue31PairingRecord::PairingRequest {
                    schema: ISSUE31_PAIRING_SCHEMA.into(),
                    host_ref: configuration.host_ref.clone(),
                    host_public_key_hex: host_public_key_hex.clone(),
                    device_public_key_hex: device_public_key_hex.clone(),
                    issued_at: 100,
                    pairing_request_ref: "pairing_request.restart.device".into(),
                    requested_scopes: vec![Issue31PairingScope::ControlFullAuto],
                    expires_at: 1_000,
                },
            },
            101,
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
    let grant = controller
        .handle_pairing_event(
            Issue31PairingEvent {
                event_id: "c".repeat(64),
                record: Issue31PairingRecord::PairingResponse {
                    schema: ISSUE31_PAIRING_SCHEMA.into(),
                    host_ref: configuration.host_ref.clone(),
                    host_public_key_hex: host_public_key_hex.clone(),
                    device_public_key_hex: device_public_key_hex.clone(),
                    issued_at: 102,
                    pairing_response_ref: "pairing_response.restart.device".into(),
                    pairing_challenge_event_id: "b".repeat(64),
                    challenge: challenge_value,
                    expires_at: 1_000,
                },
            },
            102,
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
    controller
        .handle_command_event(
            Issue31CommandEvent {
                event_id: "e".repeat(64),
                record: Issue31CommandRecord::CommandIntent {
                    schema: ISSUE31_COMMAND_SCHEMA.into(),
                    host_ref: configuration.host_ref.clone(),
                    host_public_key_hex,
                    device_public_key_hex: device_public_key_hex.clone(),
                    grant_ref: grant_ref.clone(),
                    action_ref: "action.omega.full_auto.stop".into(),
                    idempotency_ref: "idempotency.restart.first_stop".into(),
                    expected_generation: 1,
                    arguments_ref: "arguments.omega.none".into(),
                    issued_at: 103,
                    expires_at: 200,
                },
            },
            104,
            |_, _| Issue31CommandExecution {
                status: Issue31CommandStatus::Stopped,
                outcome_ref: "outcome.omega.stopped".into(),
                reason_ref: None,
            },
        )
        .expect("command")
        .expect("result");
    let revocation = controller
        .revoke_grant(&grant_ref, 105, Some("reason.omega.owner_revoked".into()))
        .expect("revoke");
    controller
        .record_emitted_pairing("f".repeat(64), revocation)
        .expect("record revoke");
    (configuration, controller, device_public_key_hex, grant_ref)
}

fn validate_sarah_binding(
    host_public_key_hex: &str,
    sarah_public_key_hex: &str,
) -> Result<(), Issue31NostrError> {
    if !valid_hex64(sarah_public_key_hex) || sarah_public_key_hex == host_public_key_hex {
        return Err(Issue31NostrError::Invalid(
            "grant Sarah identity is invalid or aliases the host".into(),
        ));
    }
    Ok(())
}

fn validate_generation(generation: u64) -> Result<(), Issue31NostrError> {
    if generation == 0 {
        Err(Issue31NostrError::Invalid(
            "generation must be positive".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_lifetime(issued_at: u64, expires_at: u64) -> Result<(), Issue31NostrError> {
    if expires_at <= issued_at {
        Err(Issue31NostrError::Invalid(
            "expiration must follow issue time".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_ref_value(value: &str, field: &str) -> Result<(), Issue31NostrError> {
    if valid_ref(value) {
        Ok(())
    } else {
        Err(Issue31NostrError::Invalid(format!("invalid {field} ref")))
    }
}

fn validate_hex_value(value: &str, field: &str) -> Result<(), Issue31NostrError> {
    if valid_hex64(value) {
        Ok(())
    } else {
        Err(Issue31NostrError::Invalid(format!("invalid {field} id")))
    }
}

fn validate_scopes(scopes: &[Issue31PairingScope], field: &str) -> Result<(), Issue31NostrError> {
    let unique: BTreeSet<Issue31PairingScope> = scopes.iter().copied().collect();
    if scopes.is_empty() || scopes.len() > 6 || unique.len() != scopes.len() {
        Err(Issue31NostrError::Invalid(format!("invalid {field}")))
    } else {
        Ok(())
    }
}

fn valid_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn valid_ref(value: &str) -> bool {
    if value.len() < 3 || value.len() > 256 {
        return false;
    }
    let mut colon_parts = value.split(':');
    let Some(path) = colon_parts.next() else {
        return false;
    };
    if let Some(suffix) = colon_parts.next()
        && (suffix.is_empty()
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
    {
        return false;
    }
    if colon_parts.next().is_some() {
        return false;
    }
    let segments: Vec<&str> = path.split('.').collect();
    if segments.len() < 2 {
        return false;
    }
    let Some(first) = segments.first() else {
        return false;
    };
    if !first
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase())
        || !first.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    {
        return false;
    }
    segments.iter().skip(1).all(|segment| {
        segment
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    })
}

pub fn is_issue31_public_ref(value: &str) -> bool {
    valid_ref(value)
}

fn valid_relay_url(value: &str) -> bool {
    if value.len() < 6 || value.len() > 512 {
        return false;
    }
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    matches!(url.scheme(), "ws" | "wss")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}

fn all_unique(values: &[String]) -> bool {
    values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn shared_fixtures_decode_and_reconcile() {
        let discovery = Issue31HostDiscovery::decode(include_bytes!(
            "../fixtures/openagents.omega.issue31.host_discovery.v1.canonical.json"
        ))
        .expect("host fixture");
        let pairing = Issue31PairingRecord::decode(include_bytes!(
            "../fixtures/openagents.omega.issue31.pairing.v1.canonical.json"
        ))
        .expect("pairing fixture");
        let intent = Issue31CommandRecord::decode(include_bytes!(
            "../fixtures/openagents.omega.issue31.command.v1.canonical-intent.json"
        ))
        .expect("intent fixture");
        let result = Issue31CommandRecord::decode(include_bytes!(
            "../fixtures/openagents.omega.issue31.command.v1.canonical-result.json"
        ))
        .expect("result fixture");
        assert_eq!(discovery.generation, 4);
        assert_eq!(discovery.sarah_public_key_hex, "3".repeat(64));
        assert!(matches!(
            pairing,
            Issue31PairingRecord::ScopedGrant {
                sarah_public_key_hex,
                ..
            } if sarah_public_key_hex == "3".repeat(64)
        ));
        let states = reconcile_issue31_commands(&[
            Issue31CommandEvent {
                event_id: "4".repeat(64),
                record: intent,
            },
            Issue31CommandEvent {
                event_id: "5".repeat(64),
                record: result,
            },
        ])
        .expect("reconcile");
        assert_eq!(states.len(), 1);
        assert!(states[0].result.is_some());
    }

    #[test]
    fn excess_fields_and_unsafe_relays_fail_closed() {
        let bytes =
            include_bytes!("../fixtures/openagents.omega.issue31.host_discovery.v1.canonical.json");
        let mut value: serde_json::Value = serde_json::from_slice(bytes).expect("fixture json");
        value["secret"] = serde_json::Value::String("nsec1forbidden".into());
        assert!(Issue31HostDiscovery::decode(value.to_string().as_bytes()).is_err());
        value.as_object_mut().expect("object").remove("secret");
        value["relayUrls"] = serde_json::json!(["wss://owner:secret@relay.example.com"]);
        assert!(Issue31HostDiscovery::decode(value.to_string().as_bytes()).is_err());
        value["relayUrls"] = serde_json::json!(["wss://relay.example.com/?token=forbidden"]);
        assert!(Issue31HostDiscovery::decode(value.to_string().as_bytes()).is_err());

        let pairing_bytes =
            include_bytes!("../fixtures/openagents.omega.issue31.pairing.v1.canonical.json");
        let mut pairing: serde_json::Value =
            serde_json::from_slice(pairing_bytes).expect("pairing fixture json");
        pairing["sarahPublicKeyHex"] = pairing["hostPublicKeyHex"].clone();
        assert!(Issue31PairingRecord::decode(pairing.to_string().as_bytes()).is_err());
        pairing
            .as_object_mut()
            .expect("pairing object")
            .remove("sarahPublicKeyHex");
        assert!(Issue31PairingRecord::decode(pairing.to_string().as_bytes()).is_err());
    }

    #[test]
    fn production_host_controller_pairs_executes_renews_and_revokes() {
        let host_public_key_hex = "1".repeat(64);
        let device_public_key_hex = "2".repeat(64);
        let mut controller = Issue31HostController::new(Issue31HostConfiguration {
            host_ref: "omega.host.local".into(),
            host_public_key_hex: host_public_key_hex.clone(),
            sarah_public_key_hex: "3".repeat(64),
            display_name: "Local Omega".into(),
            relay_urls: vec!["wss://relay.example.com".into()],
            generation: 1,
        })
        .expect("controller");
        assert!(
            controller
                .set_admitted_device_policy(Vec::new(), vec![Issue31PairingScope::ObserveIssue31],)
                .is_err()
        );
        assert!(
            controller
                .set_admitted_device_policy(
                    vec!["A".repeat(64)],
                    vec![Issue31PairingScope::ObserveIssue31],
                )
                .is_err()
        );
        controller
            .set_admitted_device_policy(
                vec![device_public_key_hex.clone()],
                vec![Issue31PairingScope::ControlFullAuto],
            )
            .expect("admit device");
        let expected_fingerprint =
            format!("{:x}", Sha256::digest(device_public_key_hex.as_bytes()))[..16]
                .to_ascii_uppercase();
        assert_eq!(
            controller.admitted_device_fingerprints(),
            vec![expected_fingerprint]
        );
        assert!(
            !serde_json::to_string(&controller)
                .expect("serialize runtime policy boundary")
                .contains(&device_public_key_hex)
        );
        let attacker_request = Issue31PairingRecord::PairingRequest {
            schema: ISSUE31_PAIRING_SCHEMA.into(),
            host_ref: "omega.host.local".into(),
            host_public_key_hex: host_public_key_hex.clone(),
            device_public_key_hex: "3".repeat(64),
            issued_at: 100,
            pairing_request_ref: "pairing_request.attacker.one".into(),
            requested_scopes: vec![
                Issue31PairingScope::ObserveIssue31,
                Issue31PairingScope::SendMessage,
                Issue31PairingScope::InterruptTurn,
                Issue31PairingScope::ControlFullAuto,
                Issue31PairingScope::RequestProviderHandoff,
                Issue31PairingScope::ActInCommunity,
            ],
            expires_at: 1_000,
        };
        assert!(
            controller
                .handle_pairing_event(
                    Issue31PairingEvent {
                        event_id: "9".repeat(64),
                        record: attacker_request,
                    },
                    101,
                )
                .expect("ignore attacker")
                .is_none()
        );
        assert!(controller.pairing_events.is_empty());
        let request_event_id = "a".repeat(64);
        let request = Issue31PairingRecord::PairingRequest {
            schema: ISSUE31_PAIRING_SCHEMA.into(),
            host_ref: "omega.host.local".into(),
            host_public_key_hex: host_public_key_hex.clone(),
            device_public_key_hex: device_public_key_hex.clone(),
            issued_at: 100,
            pairing_request_ref: "pairing_request.device.one".into(),
            requested_scopes: vec![
                Issue31PairingScope::ObserveIssue31,
                Issue31PairingScope::ControlFullAuto,
            ],
            expires_at: 1_000,
        };
        let challenge = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: request_event_id,
                    record: request,
                },
                101,
            )
            .expect("request")
            .expect("challenge");
        let challenge_value = match &challenge {
            Issue31PairingRecord::PairingChallenge { challenge, .. } => challenge.clone(),
            _ => panic!("expected challenge"),
        };
        let challenge_event_id = "b".repeat(64);
        controller
            .record_emitted_pairing(challenge_event_id.clone(), challenge)
            .expect("record challenge");
        let response_event_id = "c".repeat(64);
        let grant = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: response_event_id,
                    record: Issue31PairingRecord::PairingResponse {
                        schema: ISSUE31_PAIRING_SCHEMA.into(),
                        host_ref: "omega.host.local".into(),
                        host_public_key_hex: host_public_key_hex.clone(),
                        device_public_key_hex: device_public_key_hex.clone(),
                        issued_at: 102,
                        pairing_response_ref: "pairing_response.device.one".into(),
                        pairing_challenge_event_id: challenge_event_id,
                        challenge: challenge_value,
                        expires_at: 1_000,
                    },
                },
                102,
            )
            .expect("response")
            .expect("grant");
        let grant_ref = match &grant {
            Issue31PairingRecord::ScopedGrant {
                grant_ref,
                sarah_public_key_hex,
                scopes,
                ..
            } => {
                assert_eq!(sarah_public_key_hex, &"3".repeat(64));
                assert_eq!(scopes, &vec![Issue31PairingScope::ControlFullAuto]);
                grant_ref.clone()
            }
            _ => panic!("expected scoped grant"),
        };
        controller
            .record_emitted_pairing("d".repeat(64), grant)
            .expect("record grant");

        let executions = Cell::new(0_u32);
        let result = controller
            .handle_command_event(
                Issue31CommandEvent {
                    event_id: "e".repeat(64),
                    record: Issue31CommandRecord::CommandIntent {
                        schema: ISSUE31_COMMAND_SCHEMA.into(),
                        host_ref: "omega.host.local".into(),
                        host_public_key_hex,
                        device_public_key_hex,
                        grant_ref: grant_ref.clone(),
                        action_ref: "action.omega.full_auto.stop".into(),
                        idempotency_ref: "idempotency.device.stop_one".into(),
                        expected_generation: 1,
                        arguments_ref: "arguments.omega.none".into(),
                        issued_at: 103,
                        expires_at: 200,
                    },
                },
                104,
                |_, _| {
                    executions.set(executions.get().saturating_add(1));
                    Issue31CommandExecution {
                        status: Issue31CommandStatus::Stopped,
                        outcome_ref: "outcome.omega.stopped".into(),
                        reason_ref: None,
                    }
                },
            )
            .expect("command")
            .expect("terminal result");
        assert_eq!(executions.get(), 1);
        assert!(matches!(
            result,
            Issue31CommandRecord::CommandResult {
                status: Issue31CommandStatus::Stopped,
                ..
            }
        ));

        let renewal = controller
            .renew_grant(
                &grant_ref,
                vec![Issue31PairingScope::ControlFullAuto],
                105,
                2_000,
            )
            .expect("renewal");
        controller
            .record_emitted_pairing("f".repeat(64), renewal)
            .expect("record renewal");
        let revocation = controller
            .revoke_grant(&grant_ref, 106, Some("reason.omega.owner_revoked".into()))
            .expect("revocation");
        controller
            .record_emitted_pairing("0".repeat(64), revocation)
            .expect("record revocation");
        let state = fold_issue31_grant(&controller.pairing_events, &grant_ref)
            .expect("fold")
            .expect("state");
        assert_eq!(state.status, Issue31GrantStatus::Revoked);
    }

    #[test]
    fn scoped_grant_requires_the_complete_pairing_chain() {
        let grant = Issue31PairingRecord::decode(include_bytes!(
            "../fixtures/openagents.omega.issue31.pairing.v1.canonical.json"
        ))
        .expect("grant fixture");
        let error = fold_issue31_grant(
            &[Issue31PairingEvent {
                event_id: "1".repeat(64),
                record: grant,
            }],
            "grant.omega.device_1",
        )
        .expect_err("unsolicited grant must fail");
        assert!(error.to_string().contains("pairing response"));
    }

    #[test]
    fn runtime_admission_policy_rebind_does_not_change_durable_grants() {
        let (_, controller, device_public_key_hex, grant_ref) = restart_fixture();
        let projections = controller.grant_projections(107).expect("grant list");
        assert_eq!(projections.len(), 1);
        assert_eq!(projections[0].grant_ref, grant_ref);
        assert_eq!(projections[0].status, "revoked");
        let public_projection = serde_json::to_string(&projections).expect("projection json");
        assert!(!public_projection.contains(&device_public_key_hex));
        let encoded = serde_json::to_vec(&controller).expect("serialize controller");
        let mut reloaded: Issue31HostController =
            serde_json::from_slice(&encoded).expect("deserialize controller");
        assert!(reloaded.admitted_device_fingerprints().is_empty());
        let before = fold_issue31_grant(&reloaded.pairing_events, &grant_ref)
            .expect("fold before policy rebind")
            .expect("durable grant");
        reloaded
            .set_admitted_device_policy(
                vec!["4".repeat(64)],
                vec![Issue31PairingScope::ObserveIssue31],
            )
            .expect("rebind runtime policy");
        let after = fold_issue31_grant(&reloaded.pairing_events, &grant_ref)
            .expect("fold after policy rebind")
            .expect("durable grant");
        assert_eq!(before, after);
    }

    #[test]
    fn controller_bounds_refuse_before_execution_or_mutation() {
        let host_public_key_hex = "1".repeat(64);
        let mut controller = Issue31HostController::new(Issue31HostConfiguration {
            host_ref: "omega.host.local".into(),
            host_public_key_hex: host_public_key_hex.clone(),
            sarah_public_key_hex: "3".repeat(64),
            display_name: "Local Omega".into(),
            relay_urls: vec!["wss://relay.example.com".into()],
            generation: 1,
        })
        .expect("controller");
        controller.processed_command_event_ids = (0..MAX_ISSUE31_PROCESSED_EVENTS)
            .map(|index| format!("{index:064x}"))
            .collect();
        let executed = Cell::new(false);
        let error = controller
            .handle_command_event(
                Issue31CommandEvent {
                    event_id: "f".repeat(64),
                    record: Issue31CommandRecord::CommandIntent {
                        schema: ISSUE31_COMMAND_SCHEMA.into(),
                        host_ref: "omega.host.local".into(),
                        host_public_key_hex,
                        device_public_key_hex: "2".repeat(64),
                        grant_ref: "grant.omega.bound_test".into(),
                        action_ref: "action.omega.full_auto.stop".into(),
                        idempotency_ref: "idempotency.omega.bound_test".into(),
                        expected_generation: 1,
                        arguments_ref: "arguments.omega.none".into(),
                        issued_at: 100,
                        expires_at: 200,
                    },
                },
                101,
                |_, _| {
                    executed.set(true);
                    Issue31CommandExecution {
                        status: Issue31CommandStatus::Stopped,
                        outcome_ref: "outcome.omega.stopped".into(),
                        reason_ref: None,
                    }
                },
            )
            .expect_err("processed event bound");
        assert!(error.to_string().contains("bound"));
        assert!(!executed.get());
        assert!(controller.command_results.is_empty());

        controller.processed_command_event_ids.clear();
        controller.command_results = (0..MAX_ISSUE31_COMMAND_RESULTS)
            .map(|index| {
                let idempotency_ref = format!("idempotency.omega.bound_{index}");
                let intent = Issue31CommandRecord::CommandIntent {
                    schema: ISSUE31_COMMAND_SCHEMA.into(),
                    host_ref: "omega.host.local".into(),
                    host_public_key_hex: "1".repeat(64),
                    device_public_key_hex: "2".repeat(64),
                    grant_ref: "grant.omega.bound_test".into(),
                    action_ref: "action.omega.full_auto.stop".into(),
                    idempotency_ref: idempotency_ref.clone(),
                    expected_generation: 1,
                    arguments_ref: "arguments.omega.none".into(),
                    issued_at: 100,
                    expires_at: 200,
                };
                let result = Issue31CommandRecord::CommandResult {
                    schema: ISSUE31_COMMAND_SCHEMA.into(),
                    host_ref: "omega.host.local".into(),
                    host_public_key_hex: "1".repeat(64),
                    device_public_key_hex: "2".repeat(64),
                    grant_ref: "grant.omega.bound_test".into(),
                    intent_event_id: format!("{index:064x}"),
                    action_ref: "action.omega.full_auto.stop".into(),
                    idempotency_ref: idempotency_ref.clone(),
                    expected_generation: 1,
                    status: Issue31CommandStatus::Unavailable,
                    outcome_ref: "outcome.omega.unavailable".into(),
                    reason_ref: Some("reason.omega.controller_not_bound".into()),
                    completed_at: 101,
                };
                (idempotency_ref, (intent, result))
            })
            .collect();
        controller.processed_command_event_ids = (0..MAX_ISSUE31_COMMAND_RESULTS)
            .map(|index| format!("{index:064x}"))
            .collect();
        let command_result_count = controller.command_results.len();
        let executed = Cell::new(false);
        let error = controller
            .handle_command_event(
                Issue31CommandEvent {
                    event_id: "f".repeat(64),
                    record: Issue31CommandRecord::CommandIntent {
                        schema: ISSUE31_COMMAND_SCHEMA.into(),
                        host_ref: "omega.host.local".into(),
                        host_public_key_hex: "1".repeat(64),
                        device_public_key_hex: "2".repeat(64),
                        grant_ref: "grant.omega.bound_test".into(),
                        action_ref: "action.omega.full_auto.stop".into(),
                        idempotency_ref: "idempotency.omega.bound_overflow".into(),
                        expected_generation: 1,
                        arguments_ref: "arguments.omega.none".into(),
                        issued_at: 100,
                        expires_at: 200,
                    },
                },
                101,
                |_, _| {
                    executed.set(true);
                    Issue31CommandExecution {
                        status: Issue31CommandStatus::Stopped,
                        outcome_ref: "outcome.omega.stopped".into(),
                        reason_ref: None,
                    }
                },
            )
            .expect_err("command result bound");
        assert!(error.to_string().contains("bound"));
        assert!(!executed.get());
        assert_eq!(controller.command_results.len(), command_result_count);
        assert!(
            !controller
                .processed_command_event_ids
                .contains(&"f".repeat(64))
        );
        let encoded = serde_json::to_vec(&controller).expect("serialize bounded controller");
        let reloaded: Issue31HostController =
            serde_json::from_slice(&encoded).expect("reload bounded controller");
        reloaded
            .validate_persisted_state()
            .expect("bounded controller remains persistable");

        controller.processed_pairing_event_ids = (0..MAX_ISSUE31_PROCESSED_EVENTS)
            .map(|index| format!("{index:064x}"))
            .collect();
        let error = controller
            .record_emitted_pairing(
                "e".repeat(64),
                Issue31PairingRecord::PairingChallenge {
                    schema: ISSUE31_PAIRING_SCHEMA.into(),
                    host_ref: "omega.host.local".into(),
                    host_public_key_hex: "1".repeat(64),
                    device_public_key_hex: "2".repeat(64),
                    issued_at: 100,
                    pairing_challenge_ref: "pairing_challenge.omega.bound_test".into(),
                    pairing_request_event_id: "d".repeat(64),
                    challenge: "3".repeat(64),
                    expires_at: 200,
                },
            )
            .expect_err("pairing event bound");
        assert!(error.to_string().contains("bound"));
        assert!(controller.pairing_events.is_empty());
    }
}
