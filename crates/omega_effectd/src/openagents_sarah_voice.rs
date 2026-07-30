use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use http_client::{AsyncBody, HttpClient, Method, Request, StatusCode};
use serde::{Deserialize, Serialize};
use smol::io::AsyncReadExt as _;
use url::Url;

use super::openagents_nostr_auth::{HostedSessionBlocker, ready_account_identity, sign_nip98_post};

pub const SARAH_VOICE_NOSTR_CHALLENGE_URL: &str =
    "https://openagents.com/api/omega/sarah/voice/auth/challenge";
pub const SARAH_VOICE_SESSION_URL: &str = "https://openagents.com/api/omega/sarah/voice/session";
pub const SARAH_VOICE_ADMISSION_URL: &str =
    "https://openagents.com/api/omega/sarah/voice/admission";
pub const SARAH_VOICE_SETTLEMENT_URL: &str =
    "https://openagents.com/api/omega/sarah/voice/settlement";
pub const SARAH_VOICE_PROTOCOL: &str = "openagents.sarah.voice.v1";
pub const SARAH_VOICE_CHALLENGE_PROTOCOL: &str = "openagents.sarah.voice.auth-challenge.v1";
pub const SARAH_VOICE_DISCLOSURE_REF: &str = "omega.voice.disclosure.v1";
pub const SARAH_VOICE_DEVICE_HEADER: &str = "x-openagents-omega-device-ref";
pub const SARAH_VOICE_SESSION_HEADER: &str = "x-openagents-sarah-voice-session";
pub const SARAH_VOICE_TICKET_HEADER: &str = "x-openagents-sarah-voice-ticket";
pub const SARAH_VOICE_ADMISSION_SCHEMA: &str = "openagents.sarah.voice.admission.v1";
pub const SARAH_VOICE_SETTLEMENT_SCHEMA: &str = "openagents.sarah.voice.settlement.v1";

const SARAH_VOICE_MODEL: &str = "gpt-realtime-2.1";
const MAX_HTTP_BODY_BYTES: u64 = 64 * 1024;
const MAX_REF_BYTES: usize = 256;
const MAX_ACCESS_TOKEN_BYTES: usize = 16 * 1024;
const MAX_GATEWAY_URL_BYTES: usize = 2 * 1024;
const MAX_CHALLENGE_LIFETIME_MS: u64 = 120_000;
const MAX_ADMISSION_LIFETIME_MS: u64 = 120_000;
const MAX_TICKET_LIFETIME_MS: u64 = 60_000;
const MAX_SESSION_LIFETIME_MS: u64 = 900_000;
const MAX_SERVER_CLOCK_SKEW_MS: u64 = 60_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedSarahVoiceSession {
    pub owner_ref: String,
    pub device_ref: String,
    pub thread_ref: String,
    pub session_ref: String,
    pub generation: u32,
    pub disclosure_ref: String,
    pub gateway_url: String,
    pub ticket: String,
    pub ticket_expires_at_ms: u64,
    pub session_expires_at_ms: u64,
    pub reserved_credit_msat: u64,
    pub max_duration_seconds: u64,
    pub admission: Option<SarahVoiceAdmissionProjection>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SarahVoiceProjectionGap {
    Complete,
    ServiceFieldsMissing,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SarahVoiceAdmissionProjection {
    pub schema: String,
    pub admission_ref: String,
    pub admission_expires_at_ms: u64,
    pub client_profile: String,
    pub thread_ref: String,
    pub session_ref: String,
    pub reserved_credit_msat: u64,
    pub max_duration_seconds: u64,
    pub credit_msat_per_million_tokens: Option<u64>,
    pub remaining_credit_msat: Option<u64>,
    pub admission_cohort_ref: String,
    pub credit_mode: SarahVoiceCreditMode,
    pub commands: Vec<SarahVoiceCapabilityId>,
    pub confirmation_required: Vec<SarahVoiceCapabilityId>,
    pub excluded_authorities: SarahVoiceExcludedAuthorities,
    pub gap: SarahVoiceProjectionGap,
    pub detail: Option<String>,
}

impl SarahVoiceAdmissionProjection {
    pub fn has_same_reviewed_terms(&self, other: &Self) -> bool {
        self.client_profile == other.client_profile
            && self.admission_cohort_ref == other.admission_cohort_ref
            && self.credit_mode == other.credit_mode
            && self.credit_msat_per_million_tokens == other.credit_msat_per_million_tokens
            && self.reserved_credit_msat == other.reserved_credit_msat
            && self.remaining_credit_msat == other.remaining_credit_msat
            && self.max_duration_seconds == other.max_duration_seconds
            && self.commands == other.commands
            && self.confirmation_required == other.confirmation_required
            && self.excluded_authorities == other.excluded_authorities
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SarahVoiceCreditMode {
    Metered,
    StagingOwnerEntitlement,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SarahVoiceCapabilityId {
    ContextRead,
    OpenPath,
    RevealRange,
    ReplaceSelection,
    SaveDocument,
    StartAgentThread,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SarahVoiceExcludedAuthorities {
    pub direct_shell: bool,
    pub direct_git: bool,
    pub payment: bool,
    pub credential_access: bool,
    pub device_control: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedSarahVoiceAdmission {
    pub owner_ref: String,
    pub device_ref: String,
    pub thread_ref: String,
    pub session_ref: String,
    pub generation: u32,
    pub projection: SarahVoiceAdmissionProjection,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SarahVoiceSettlementState {
    Pending,
    Settled,
    Released,
    Unavailable,
    Malformed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SarahVoiceSettlementProjection {
    pub schema: String,
    pub thread_ref: String,
    pub session_ref: String,
    pub state: SarahVoiceSettlementState,
    pub credit_mode: Option<SarahVoiceCreditMode>,
    pub final_charge_msat: Option<u64>,
    pub remaining_credit_msat: Option<u64>,
    pub receipt_ref: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IssuedSarahVoiceSession {
    pub voice: ManagedSarahVoiceSession,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NostrSarahVoiceAdmission {
    pub admission: PreparedSarahVoiceAdmission,
    pub access_token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChallengeRequest<'a> {
    schema: &'static str,
    device_ref: &'a str,
    pubkey: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChallengeResponse {
    schema: String,
    challenge: String,
    expires_at_ms: u64,
    owner_ref: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceSessionRequest<'a> {
    schema: &'static str,
    identity: VoiceIdentity<'a>,
    disclosure_ref: &'static str,
    client_profile: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    admission_ref: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth: Option<NostrAuthentication<'a>>,
}

#[derive(Serialize)]
struct NostrAuthentication<'a> {
    method: &'static str,
    challenge: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VoiceAdmissionResponse {
    schema: String,
    admitted: bool,
    #[serde(default)]
    admission_ref: Option<String>,
    #[serde(default)]
    admission_expires_at_ms: Option<u64>,
    client_profile: String,
    admission_cohort_ref: String,
    credit_mode: SarahVoiceCreditMode,
    credit_rate_msat_per_million_tokens: u64,
    required_hold_msat: u64,
    #[serde(deserialize_with = "deserialize_required_nullable_u64")]
    spendable_remaining_credit_msat: Option<u64>,
    max_duration_seconds: u64,
    capability_boundary: VoiceCapabilityBoundary,
    #[serde(default)]
    refusal_reason: Option<VoiceAdmissionRefusal>,
    #[serde(default)]
    auth: Option<IssuedAuthentication>,
}

fn deserialize_required_nullable_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<u64>::deserialize(deserializer)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IssuedAuthentication {
    method: String,
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VoiceCapabilityBoundary {
    commands: Vec<SarahVoiceCapabilityId>,
    confirmation_required: Vec<SarahVoiceCapabilityId>,
    direct_shell: bool,
    direct_git: bool,
    payment: bool,
    credential_access: bool,
    device_control: bool,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum VoiceAdmissionRefusal {
    InsufficientCredit,
    CohortInactive,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VoiceSettlementResponse {
    schema: String,
    session_ref: String,
    state: SarahVoiceSettlementState,
    credit_mode: SarahVoiceCreditMode,
    final_charge_msat: u64,
    spendable_remaining_credit_msat: Option<u64>,
    receipt_ref: String,
}

fn valid_admitted_response(response: &VoiceAdmissionResponse) -> bool {
    let Ok(now_ms) = now_ms() else {
        return false;
    };
    let commands = response
        .capability_boundary
        .commands
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let confirmations = response
        .capability_boundary
        .confirmation_required
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    response.admitted
        && response.refusal_reason.is_none()
        && response.schema == SARAH_VOICE_ADMISSION_SCHEMA
        && response.client_profile == "omega_editor"
        && response
            .admission_ref
            .as_deref()
            .is_some_and(valid_admission_ref)
        && response
            .admission_expires_at_ms
            .is_some_and(|expires_at_ms| {
                expires_at_ms > now_ms
                    && expires_at_ms - now_ms
                        <= MAX_ADMISSION_LIFETIME_MS + MAX_SERVER_CLOCK_SKEW_MS
            })
        && matches!(
            response.admission_cohort_ref.as_str(),
            "sarah_voice_cohort:alpha_v1" | "sarah_voice_cohort:staging_owner_v1"
        )
        && response.credit_rate_msat_per_million_tokens > 0
        && (60..=900).contains(&response.max_duration_seconds)
        && !response.capability_boundary.commands.is_empty()
        && commands.len() == response.capability_boundary.commands.len()
        && confirmations.len() == response.capability_boundary.confirmation_required.len()
        && confirmations.is_subset(&commands)
        && !response.capability_boundary.direct_shell
        && !response.capability_boundary.direct_git
        && !response.capability_boundary.payment
        && !response.capability_boundary.credential_access
        && !response.capability_boundary.device_control
        && match response.credit_mode {
            SarahVoiceCreditMode::Metered => {
                response.required_hold_msat > 0
                    && response.spendable_remaining_credit_msat.is_some()
            }
            SarahVoiceCreditMode::StagingOwnerEntitlement => {
                response.required_hold_msat == 0
                    && response.spendable_remaining_credit_msat.is_none()
            }
        }
}

fn valid_settlement_response(response: &VoiceSettlementResponse, session_ref: &str) -> bool {
    response.schema == SARAH_VOICE_SETTLEMENT_SCHEMA
        && response.session_ref == session_ref
        && matches!(
            response.state,
            SarahVoiceSettlementState::Settled | SarahVoiceSettlementState::Released
        )
        && valid_ref(&response.receipt_ref)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceIdentity<'a> {
    owner_ref: &'a str,
    device_ref: &'a str,
    thread_ref: &'a str,
    session_ref: &'a str,
    generation: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VoiceSessionResponse {
    schema: String,
    session_ref: String,
    model: String,
    client_profile: String,
    admission_ref: String,
    admission_expires_at_ms: u64,
    admission_cohort_ref: String,
    credit_mode: SarahVoiceCreditMode,
    credit_rate_msat_per_million_tokens: u64,
    #[serde(deserialize_with = "deserialize_required_nullable_u64")]
    spendable_remaining_credit_msat: Option<u64>,
    capability_boundary: VoiceCapabilityBoundary,
    gateway_url: String,
    ticket: String,
    ticket_expires_at_ms: u64,
    session_expires_at_ms: u64,
    reserved_credit_msat: u64,
    max_duration_seconds: u64,
    input_audio: VoiceAudioFormat,
    output_audio: VoiceAudioFormat,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VoiceAudioFormat {
    codec: String,
    sample_rate_hz: u32,
    channels: u8,
}

pub(crate) async fn prepare_bearer_sarah_voice_admission(
    http_client: &Arc<dyn HttpClient>,
    access_token: &str,
    owner_ref: &str,
    device_ref: &str,
    thread_ref: &str,
    session_ref: &str,
) -> Result<PreparedSarahVoiceAdmission, HostedSessionBlocker> {
    for value in [owner_ref, device_ref, thread_ref, session_ref] {
        validate_ref(value)?;
    }
    let request_body = serde_json::to_vec(&VoiceSessionRequest {
        schema: SARAH_VOICE_ADMISSION_SCHEMA,
        identity: VoiceIdentity {
            owner_ref,
            device_ref,
            thread_ref,
            session_ref,
            generation: 1,
        },
        disclosure_ref: SARAH_VOICE_DISCLOSURE_REF,
        client_profile: "omega_editor",
        admission_ref: None,
        auth: None,
    })
    .map_err(|_| HostedSessionBlocker::ResponseInvalid)?;
    let authorization = format!("Bearer {access_token}");
    let (status, response_body) = send_json_post(
        http_client,
        SARAH_VOICE_ADMISSION_URL,
        Some(&authorization),
        Some(device_ref),
        request_body,
    )
    .await?;
    if status != StatusCode::OK {
        if let Some(blocker) = insufficient_credit_blocker(status, &response_body) {
            return Err(blocker);
        }
        return Err(voice_status_blocker(status));
    }
    let (admission, access_token) = parse_voice_admission_response(
        &response_body,
        owner_ref,
        device_ref,
        thread_ref,
        session_ref,
        false,
    )?;
    if access_token.is_some() {
        return Err(HostedSessionBlocker::ResponseInvalid);
    }
    Ok(admission)
}

pub(crate) async fn prepare_nostr_sarah_voice_admission(
    http_client: &Arc<dyn HttpClient>,
    device_ref: &str,
    thread_ref: &str,
    session_ref: &str,
) -> Result<NostrSarahVoiceAdmission, HostedSessionBlocker> {
    for value in [device_ref, thread_ref, session_ref] {
        validate_ref(value)?;
    }
    let identity = ready_account_identity()?;
    let public_key = identity.public_key_hex().as_str();
    let challenge_body = serde_json::to_vec(&ChallengeRequest {
        schema: SARAH_VOICE_CHALLENGE_PROTOCOL,
        device_ref,
        pubkey: public_key,
    })
    .map_err(|_| HostedSessionBlocker::ResponseInvalid)?;
    let (status, response_body) = send_json_post(
        http_client,
        SARAH_VOICE_NOSTR_CHALLENGE_URL,
        None,
        None,
        challenge_body,
    )
    .await?;
    if status != StatusCode::CREATED {
        if status.is_success() {
            return Err(HostedSessionBlocker::ResponseInvalid);
        }
        return Err(authentication_status_blocker(status));
    }
    let challenge: ChallengeResponse = serde_json::from_slice(&response_body).map_err(|error| {
        log::error!("Sarah voice authentication challenge could not be decoded: {error}");
        HostedSessionBlocker::ResponseInvalid
    })?;
    validate_challenge(&challenge)?;

    let request_body = serde_json::to_vec(&VoiceSessionRequest {
        schema: SARAH_VOICE_ADMISSION_SCHEMA,
        identity: VoiceIdentity {
            owner_ref: &challenge.owner_ref,
            device_ref,
            thread_ref,
            session_ref,
            generation: 1,
        },
        disclosure_ref: SARAH_VOICE_DISCLOSURE_REF,
        client_profile: "omega_editor",
        admission_ref: None,
        auth: Some(NostrAuthentication {
            method: "nostr_nip98",
            challenge: &challenge.challenge,
        }),
    })
    .map_err(|_| HostedSessionBlocker::ResponseInvalid)?;
    let authorization =
        sign_nip98_post(SARAH_VOICE_ADMISSION_URL, &request_body, &identity).await?;
    let (status, response_body) = send_json_post(
        http_client,
        SARAH_VOICE_ADMISSION_URL,
        Some(&authorization),
        Some(device_ref),
        request_body,
    )
    .await?;
    if status != StatusCode::OK {
        if let Some(blocker) = insufficient_credit_blocker(status, &response_body) {
            return Err(blocker);
        }
        return Err(signed_voice_status_blocker(status));
    }
    let (admission, access_token) = parse_voice_admission_response(
        &response_body,
        &challenge.owner_ref,
        device_ref,
        thread_ref,
        session_ref,
        true,
    )?;
    Ok(NostrSarahVoiceAdmission {
        admission,
        access_token: access_token.ok_or(HostedSessionBlocker::ResponseInvalid)?,
    })
}

fn parse_voice_admission_response(
    response_body: &[u8],
    owner_ref: &str,
    device_ref: &str,
    thread_ref: &str,
    session_ref: &str,
    expect_auth: bool,
) -> Result<(PreparedSarahVoiceAdmission, Option<String>), HostedSessionBlocker> {
    let response: VoiceAdmissionResponse =
        serde_json::from_slice(response_body).map_err(|error| {
            log::error!("Sarah voice admission response could not be decoded: {error}");
            HostedSessionBlocker::ResponseInvalid
        })?;
    let valid_cohort = matches!(
        response.admission_cohort_ref.as_str(),
        "sarah_voice_cohort:alpha_v1" | "sarah_voice_cohort:staging_owner_v1"
    );
    if !valid_cohort {
        return Err(HostedSessionBlocker::ResponseInvalid);
    }
    if !response.admitted {
        return match response.refusal_reason {
            Some(VoiceAdmissionRefusal::InsufficientCredit) => {
                Err(HostedSessionBlocker::VoiceAdmissionInsufficientCredit {
                    cohort_ref: response.admission_cohort_ref,
                })
            }
            Some(VoiceAdmissionRefusal::CohortInactive) => {
                Err(HostedSessionBlocker::VoiceCohortInactive {
                    cohort_ref: response.admission_cohort_ref,
                })
            }
            None => Err(HostedSessionBlocker::ResponseInvalid),
        };
    }
    if !valid_admitted_response(&response) {
        return Err(HostedSessionBlocker::ResponseInvalid);
    }
    if expect_auth != response.auth.is_some() {
        return Err(HostedSessionBlocker::ResponseInvalid);
    }
    let admission_ref = response
        .admission_ref
        .ok_or(HostedSessionBlocker::ResponseInvalid)?;
    let admission_expires_at_ms = response
        .admission_expires_at_ms
        .ok_or(HostedSessionBlocker::ResponseInvalid)?;
    let access_token = if let Some(auth) = response.auth {
        if auth.method != "nostr_nip98"
            || auth.expires_in != 900
            || !valid_access_token(&auth.access_token)
        {
            return Err(HostedSessionBlocker::ResponseInvalid);
        }
        Some(auth.access_token)
    } else {
        None
    };
    let admission = PreparedSarahVoiceAdmission {
        owner_ref: owner_ref.to_string(),
        device_ref: device_ref.to_string(),
        thread_ref: thread_ref.to_string(),
        session_ref: session_ref.to_string(),
        generation: 1,
        projection: SarahVoiceAdmissionProjection {
            schema: SARAH_VOICE_ADMISSION_SCHEMA.to_string(),
            admission_ref,
            admission_expires_at_ms,
            client_profile: response.client_profile,
            thread_ref: thread_ref.to_string(),
            session_ref: session_ref.to_string(),
            reserved_credit_msat: response.required_hold_msat,
            max_duration_seconds: response.max_duration_seconds,
            credit_msat_per_million_tokens: Some(response.credit_rate_msat_per_million_tokens),
            remaining_credit_msat: response.spendable_remaining_credit_msat,
            admission_cohort_ref: response.admission_cohort_ref,
            credit_mode: response.credit_mode,
            commands: response.capability_boundary.commands,
            confirmation_required: response.capability_boundary.confirmation_required,
            excluded_authorities: SarahVoiceExcludedAuthorities {
                direct_shell: false,
                direct_git: false,
                payment: false,
                credential_access: false,
                device_control: false,
            },
            gap: SarahVoiceProjectionGap::Complete,
            detail: None,
        },
    };
    Ok((admission, access_token))
}

pub(crate) async fn read_bearer_sarah_voice_settlement(
    http_client: &Arc<dyn HttpClient>,
    access_token: &str,
    thread_ref: &str,
    session_ref: &str,
) -> Result<SarahVoiceSettlementProjection, HostedSessionBlocker> {
    validate_ref(thread_ref)?;
    validate_ref(session_ref)?;
    let authorization = format!("Bearer {access_token}");
    let (status, response_body) = send_json_get(
        http_client,
        SARAH_VOICE_SETTLEMENT_URL,
        &authorization,
        session_ref,
    )
    .await?;
    if status == StatusCode::NOT_FOUND {
        return Ok(SarahVoiceSettlementProjection {
            schema: SARAH_VOICE_SETTLEMENT_SCHEMA.to_string(),
            thread_ref: thread_ref.to_string(),
            session_ref: session_ref.to_string(),
            state: SarahVoiceSettlementState::Pending,
            credit_mode: None,
            final_charge_msat: None,
            remaining_credit_msat: None,
            receipt_ref: None,
            detail: Some(
                "Settlement is not readable yet; the session may still be active or settling."
                    .into(),
            ),
        });
    }
    if status != StatusCode::OK {
        return Err(voice_status_blocker(status));
    }
    let response: VoiceSettlementResponse =
        serde_json::from_slice(&response_body).map_err(|error| {
            log::error!("Sarah voice settlement response could not be decoded: {error}");
            HostedSessionBlocker::ResponseInvalid
        })?;
    if !valid_settlement_response(&response, session_ref) {
        return Err(HostedSessionBlocker::ResponseInvalid);
    }
    Ok(SarahVoiceSettlementProjection {
        schema: response.schema,
        thread_ref: thread_ref.to_string(),
        session_ref: response.session_ref,
        state: response.state,
        credit_mode: Some(response.credit_mode),
        final_charge_msat: Some(response.final_charge_msat),
        remaining_credit_msat: response.spendable_remaining_credit_msat,
        receipt_ref: Some(response.receipt_ref),
        detail: None,
    })
}

pub(crate) async fn issue_bearer_sarah_voice_session(
    http_client: &Arc<dyn HttpClient>,
    access_token: &str,
    admission: &PreparedSarahVoiceAdmission,
) -> Result<IssuedSarahVoiceSession, HostedSessionBlocker> {
    for value in [
        admission.owner_ref.as_str(),
        admission.device_ref.as_str(),
        admission.thread_ref.as_str(),
        admission.session_ref.as_str(),
    ] {
        validate_ref(value)?;
    }
    if now_ms()? >= admission.projection.admission_expires_at_ms
        || !valid_admission_ref(&admission.projection.admission_ref)
    {
        return Err(HostedSessionBlocker::ResponseInvalid);
    }
    let request_body = serde_json::to_vec(&VoiceSessionRequest {
        schema: SARAH_VOICE_PROTOCOL,
        identity: VoiceIdentity {
            owner_ref: &admission.owner_ref,
            device_ref: &admission.device_ref,
            thread_ref: &admission.thread_ref,
            session_ref: &admission.session_ref,
            generation: admission.generation,
        },
        disclosure_ref: SARAH_VOICE_DISCLOSURE_REF,
        client_profile: "omega_editor",
        admission_ref: Some(&admission.projection.admission_ref),
        auth: None,
    })
    .map_err(|_| HostedSessionBlocker::ResponseInvalid)?;
    let authorization = format!("Bearer {access_token}");
    let (status, response_body) = send_json_post(
        http_client,
        SARAH_VOICE_SESSION_URL,
        Some(&authorization),
        Some(&admission.device_ref),
        request_body,
    )
    .await?;
    if status != StatusCode::CREATED {
        if let Some(blocker) = insufficient_credit_blocker(status, &response_body) {
            return Err(blocker);
        }
        if status.is_success() {
            log::error!("Sarah voice session request returned unexpected success status {status}");
            return Err(HostedSessionBlocker::ResponseInvalid);
        }
        return Err(voice_status_blocker(status));
    }
    parse_voice_session_response(&response_body, admission)
}

fn parse_voice_session_response(
    body: &[u8],
    admission: &PreparedSarahVoiceAdmission,
) -> Result<IssuedSarahVoiceSession, HostedSessionBlocker> {
    let response: VoiceSessionResponse = serde_json::from_slice(body).map_err(|error| {
        log::error!("Sarah voice session response could not be decoded: {error}");
        HostedSessionBlocker::ResponseInvalid
    })?;
    let now_ms = now_ms()?;
    let valid_audio = |audio: &VoiceAudioFormat| {
        audio.codec == "pcm_s16le" && audio.sample_rate_hz == 24_000 && audio.channels == 1
    };
    let gateway_url =
        Url::parse(&response.gateway_url).map_err(|_| HostedSessionBlocker::ResponseInvalid)?;
    let echoed_admission = SarahVoiceAdmissionProjection {
        schema: SARAH_VOICE_ADMISSION_SCHEMA.to_string(),
        admission_ref: response.admission_ref.clone(),
        admission_expires_at_ms: response.admission_expires_at_ms,
        client_profile: response.client_profile.clone(),
        thread_ref: admission.thread_ref.clone(),
        session_ref: admission.session_ref.clone(),
        reserved_credit_msat: response.reserved_credit_msat,
        max_duration_seconds: response.max_duration_seconds,
        credit_msat_per_million_tokens: Some(response.credit_rate_msat_per_million_tokens),
        remaining_credit_msat: response.spendable_remaining_credit_msat,
        admission_cohort_ref: response.admission_cohort_ref.clone(),
        credit_mode: response.credit_mode,
        commands: response.capability_boundary.commands.clone(),
        confirmation_required: response.capability_boundary.confirmation_required.clone(),
        excluded_authorities: SarahVoiceExcludedAuthorities {
            direct_shell: response.capability_boundary.direct_shell,
            direct_git: response.capability_boundary.direct_git,
            payment: response.capability_boundary.payment,
            credential_access: response.capability_boundary.credential_access,
            device_control: response.capability_boundary.device_control,
        },
        gap: SarahVoiceProjectionGap::Complete,
        detail: None,
    };
    let invalid_reason = if response.schema != SARAH_VOICE_PROTOCOL {
        Some("schema")
    } else if response.model != SARAH_VOICE_MODEL {
        Some("model")
    } else if response.client_profile != "omega_editor" {
        Some("client profile")
    } else if response.session_ref != admission.session_ref || !valid_ref(&response.session_ref) {
        Some("session reference")
    } else if echoed_admission.admission_ref != admission.projection.admission_ref
        || echoed_admission.admission_expires_at_ms != admission.projection.admission_expires_at_ms
        || !echoed_admission.has_same_reviewed_terms(&admission.projection)
    {
        Some("admission terms")
    } else if response.ticket_expires_at_ms <= now_ms
        || response.ticket_expires_at_ms - now_ms
            > MAX_TICKET_LIFETIME_MS + MAX_SERVER_CLOCK_SKEW_MS
    {
        Some("ticket lifetime")
    } else if response.session_expires_at_ms < response.ticket_expires_at_ms
        || response.session_expires_at_ms - now_ms
            > MAX_SESSION_LIFETIME_MS + MAX_SERVER_CLOCK_SKEW_MS
    {
        Some("session lifetime")
    } else if response.max_duration_seconds < 60 || response.max_duration_seconds > 900 {
        Some("maximum duration")
    } else if !valid_base64url(&response.ticket) {
        Some("ticket format")
    } else if response.gateway_url.len() > MAX_GATEWAY_URL_BYTES
        || gateway_url.scheme() != "wss"
        || gateway_url.host_str() != Some("openagents.com")
        || !gateway_url.username().is_empty()
        || gateway_url.password().is_some()
        || gateway_url.query().is_some()
        || gateway_url.fragment().is_some()
    {
        Some("gateway URL")
    } else if !valid_audio(&response.input_audio) || !valid_audio(&response.output_audio) {
        Some("audio format")
    } else {
        None
    };
    if let Some(reason) = invalid_reason {
        log::error!("Sarah voice session response failed validation: {reason}");
        return Err(HostedSessionBlocker::ResponseInvalid);
    }
    Ok(IssuedSarahVoiceSession {
        voice: ManagedSarahVoiceSession {
            owner_ref: admission.owner_ref.clone(),
            device_ref: admission.device_ref.clone(),
            thread_ref: admission.thread_ref.clone(),
            session_ref: admission.session_ref.clone(),
            generation: admission.generation,
            disclosure_ref: SARAH_VOICE_DISCLOSURE_REF.to_string(),
            gateway_url: response.gateway_url,
            ticket: response.ticket,
            ticket_expires_at_ms: response.ticket_expires_at_ms,
            session_expires_at_ms: response.session_expires_at_ms,
            reserved_credit_msat: response.reserved_credit_msat,
            max_duration_seconds: response.max_duration_seconds,
            admission: Some(echoed_admission),
        },
    })
}

fn insufficient_credit_blocker(
    status: StatusCode,
    response_body: &[u8],
) -> Option<HostedSessionBlocker> {
    let body = String::from_utf8_lossy(response_body).to_ascii_lowercase();
    let names_insufficient_credits = (body.contains("insufficient")
        && (body.contains("credit") || body.contains("fund")))
        || body.contains("payment_required")
        || body.contains("voice_access_required");
    if status == StatusCode::PAYMENT_REQUIRED || names_insufficient_credits {
        Some(HostedSessionBlocker::InsufficientVoiceCredits {
            status: status.as_u16(),
        })
    } else {
        None
    }
}

fn validate_challenge(challenge: &ChallengeResponse) -> Result<(), HostedSessionBlocker> {
    let now_ms = now_ms()?;
    if challenge.schema != SARAH_VOICE_CHALLENGE_PROTOCOL
        || !valid_ref(&challenge.owner_ref)
        || !valid_base64url(&challenge.challenge)
        || challenge.expires_at_ms <= now_ms
        || challenge.expires_at_ms - now_ms > MAX_CHALLENGE_LIFETIME_MS + MAX_SERVER_CLOCK_SKEW_MS
    {
        return Err(HostedSessionBlocker::ResponseInvalid);
    }
    Ok(())
}

fn validate_ref(value: &str) -> Result<(), HostedSessionBlocker> {
    if !valid_ref(value) {
        return Err(HostedSessionBlocker::ResponseInvalid);
    }
    Ok(())
}

fn valid_ref(value: &str) -> bool {
    !value.is_empty() && value == value.trim() && value.len() <= MAX_REF_BYTES
}

fn valid_base64url(value: &str) -> bool {
    (32..=MAX_REF_BYTES).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_admission_ref(value: &str) -> bool {
    let Some(random) = value.strip_prefix("sarah_voice_admission:") else {
        return false;
    };
    value.len() <= MAX_REF_BYTES && valid_base64url(random)
}

fn valid_access_token(value: &str) -> bool {
    let Some(random) = value.strip_prefix("oa_omega_") else {
        return false;
    };
    value.len() <= MAX_ACCESS_TOKEN_BYTES && valid_base64url(random)
}

fn now_ms() -> Result<u64, HostedSessionBlocker> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HostedSessionBlocker::ResponseInvalid)?
        .as_millis();
    u64::try_from(millis).map_err(|_| HostedSessionBlocker::ResponseInvalid)
}

fn voice_status_blocker(status: StatusCode) -> HostedSessionBlocker {
    if status.is_client_error() {
        HostedSessionBlocker::VoiceSessionRejected {
            status: status.as_u16(),
        }
    } else {
        HostedSessionBlocker::ServiceUnavailable {
            status: status.as_u16(),
        }
    }
}

fn authentication_status_blocker(status: StatusCode) -> HostedSessionBlocker {
    if status == StatusCode::TOO_MANY_REQUESTS {
        HostedSessionBlocker::ChallengeRateLimited
    } else if status.is_client_error() {
        HostedSessionBlocker::ProofRejected {
            status: status.as_u16(),
        }
    } else {
        HostedSessionBlocker::ServiceUnavailable {
            status: status.as_u16(),
        }
    }
}

fn signed_voice_status_blocker(status: StatusCode) -> HostedSessionBlocker {
    if status == StatusCode::CONFLICT {
        HostedSessionBlocker::VoiceProofExpired
    } else if status == StatusCode::UNAUTHORIZED {
        HostedSessionBlocker::ProofRejected {
            status: status.as_u16(),
        }
    } else {
        voice_status_blocker(status)
    }
}

async fn send_json_post(
    http_client: &Arc<dyn HttpClient>,
    url: &str,
    authorization: Option<&str>,
    device_ref: Option<&str>,
    body: Vec<u8>,
) -> Result<(StatusCode, Vec<u8>), HostedSessionBlocker> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(url)
        .header("content-type", "application/json")
        .header("content-length", body.len().to_string());
    if let Some(authorization) = authorization {
        builder = builder.header("authorization", authorization);
    }
    if let Some(device_ref) = device_ref {
        builder = builder.header(SARAH_VOICE_DEVICE_HEADER, device_ref);
    }
    let request = builder
        .body(AsyncBody::from(body))
        .map_err(|_| HostedSessionBlocker::ResponseInvalid)?;
    let mut response = http_client
        .send(request)
        .await
        .map_err(|_| HostedSessionBlocker::ServiceUnreachable)?;
    let status = response.status();
    let mut response_body = Vec::new();
    response
        .body_mut()
        .take(MAX_HTTP_BODY_BYTES)
        .read_to_end(&mut response_body)
        .await
        .map_err(|_| HostedSessionBlocker::ServiceUnreachable)?;
    Ok((status, response_body))
}

async fn send_json_get(
    http_client: &Arc<dyn HttpClient>,
    url: &str,
    authorization: &str,
    session_ref: &str,
) -> Result<(StatusCode, Vec<u8>), HostedSessionBlocker> {
    let request = Request::builder()
        .method(Method::GET)
        .uri(url)
        .header("authorization", authorization)
        .header(SARAH_VOICE_SESSION_HEADER, session_ref)
        .body(AsyncBody::empty())
        .map_err(|_| HostedSessionBlocker::ResponseInvalid)?;
    let mut response = http_client
        .send(request)
        .await
        .map_err(|_| HostedSessionBlocker::ServiceUnreachable)?;
    let status = response.status();
    let mut response_body = Vec::new();
    response
        .body_mut()
        .take(MAX_HTTP_BODY_BYTES)
        .read_to_end(&mut response_body)
        .await
        .map_err(|_| HostedSessionBlocker::ServiceUnreachable)?;
    Ok((status, response_body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reviewed_admission_projection() -> SarahVoiceAdmissionProjection {
        SarahVoiceAdmissionProjection {
            schema: SARAH_VOICE_ADMISSION_SCHEMA.into(),
            admission_ref: format!("sarah_voice_admission:{}", "a".repeat(43)),
            admission_expires_at_ms: now_ms().expect("system time") + MAX_ADMISSION_LIFETIME_MS,
            client_profile: "omega_editor".into(),
            thread_ref: "thread-1".into(),
            session_ref: "session-1".into(),
            reserved_credit_msat: 256_000,
            max_duration_seconds: 300,
            credit_msat_per_million_tokens: Some(64_000_000),
            remaining_credit_msat: Some(8_000_000),
            admission_cohort_ref: "sarah_voice_cohort:alpha_v1".into(),
            credit_mode: SarahVoiceCreditMode::Metered,
            commands: vec![
                SarahVoiceCapabilityId::ContextRead,
                SarahVoiceCapabilityId::ReplaceSelection,
            ],
            confirmation_required: vec![SarahVoiceCapabilityId::ReplaceSelection],
            excluded_authorities: SarahVoiceExcludedAuthorities {
                direct_shell: false,
                direct_git: false,
                payment: false,
                credential_access: false,
                device_control: false,
            },
            gap: SarahVoiceProjectionGap::Complete,
            detail: None,
        }
    }

    fn prepared_admission() -> PreparedSarahVoiceAdmission {
        PreparedSarahVoiceAdmission {
            owner_ref: "owner".into(),
            device_ref: "device".into(),
            thread_ref: "thread".into(),
            session_ref: "voice-session".into(),
            generation: 1,
            projection: reviewed_admission_projection(),
        }
    }

    #[test]
    fn reconnect_requires_every_reviewed_server_term_to_be_identical() {
        let reviewed = reviewed_admission_projection();
        let mut refreshed = reviewed.clone();
        refreshed.thread_ref = "thread-2".into();
        refreshed.session_ref = "session-2".into();
        assert!(reviewed.has_same_reviewed_terms(&refreshed));

        macro_rules! assert_term_change_refused {
            ($field:ident, $value:expr) => {{
                let mut changed = reviewed.clone();
                changed.$field = $value;
                assert!(!reviewed.has_same_reviewed_terms(&changed));
            }};
        }
        assert_term_change_refused!(client_profile, "another_profile".into());
        assert_term_change_refused!(
            admission_cohort_ref,
            "sarah_voice_cohort:staging_owner_v1".into()
        );
        assert_term_change_refused!(credit_mode, SarahVoiceCreditMode::StagingOwnerEntitlement);
        assert_term_change_refused!(credit_msat_per_million_tokens, Some(1));
        assert_term_change_refused!(reserved_credit_msat, 1);
        assert_term_change_refused!(remaining_credit_msat, Some(1));
        assert_term_change_refused!(max_duration_seconds, 60);
        assert_term_change_refused!(commands, vec![SarahVoiceCapabilityId::ContextRead]);
        assert_term_change_refused!(confirmation_required, Vec::new());
        let mut changed = reviewed.clone();
        changed.excluded_authorities.direct_shell = true;
        assert!(!reviewed.has_same_reviewed_terms(&changed));
    }

    #[test]
    fn voice_endpoints_are_exact_https_contracts() {
        assert_eq!(
            SARAH_VOICE_NOSTR_CHALLENGE_URL,
            "https://openagents.com/api/omega/sarah/voice/auth/challenge"
        );
        assert_eq!(
            SARAH_VOICE_ADMISSION_URL,
            "https://openagents.com/api/omega/sarah/voice/admission"
        );
        assert_eq!(
            SARAH_VOICE_SESSION_URL,
            "https://openagents.com/api/omega/sarah/voice/session"
        );
        assert_eq!(
            SARAH_VOICE_SETTLEMENT_URL,
            "https://openagents.com/api/omega/sarah/voice/settlement"
        );
        assert!(valid_base64url(&"a".repeat(32)));
        assert!(!valid_base64url("not padded base64="));
        assert!(valid_access_token(&format!("oa_omega_{}", "a".repeat(32))));
        assert!(!valid_access_token("oa_omega_too-short"));
    }

    #[test]
    fn direct_nip98_admission_binds_the_exact_challenge_and_request_body() {
        let challenge = ChallengeRequest {
            schema: SARAH_VOICE_CHALLENGE_PROTOCOL,
            device_ref: "omega-device",
            pubkey: &"a".repeat(64),
        };
        assert_eq!(
            serde_json::to_value(challenge).expect("serialize challenge request"),
            serde_json::json!({
                "schema": SARAH_VOICE_CHALLENGE_PROTOCOL,
                "deviceRef": "omega-device",
                "pubkey": "a".repeat(64),
            })
        );

        let body = serde_json::to_vec(&VoiceSessionRequest {
            schema: SARAH_VOICE_ADMISSION_SCHEMA,
            identity: VoiceIdentity {
                owner_ref: "owner-from-challenge",
                device_ref: "omega-device",
                thread_ref: "sarah-owner-private",
                session_ref: "voice-session",
                generation: 1,
            },
            disclosure_ref: SARAH_VOICE_DISCLOSURE_REF,
            client_profile: "omega_editor",
            admission_ref: None,
            auth: Some(NostrAuthentication {
                method: "nostr_nip98",
                challenge: "abcdefghijklmnopqrstuvwxyzABCDEF",
            }),
        })
        .expect("serialize signed admission request once");
        let value: serde_json::Value =
            serde_json::from_slice(&body).expect("decode signed admission request");
        assert_eq!(value["schema"], SARAH_VOICE_ADMISSION_SCHEMA);
        assert_eq!(value["identity"]["ownerRef"], "owner-from-challenge");
        assert_eq!(value["auth"]["method"], "nostr_nip98");
        assert_eq!(
            value["auth"]["challenge"],
            "abcdefghijklmnopqrstuvwxyzABCDEF"
        );
        assert!(value.get("admissionRef").is_none());
        assert!(body.len() < 8 * 1024);
    }

    #[test]
    fn session_request_carries_the_reviewed_one_use_admission_reference() {
        let admission = prepared_admission();
        let body = serde_json::to_value(VoiceSessionRequest {
            schema: SARAH_VOICE_PROTOCOL,
            identity: VoiceIdentity {
                owner_ref: &admission.owner_ref,
                device_ref: &admission.device_ref,
                thread_ref: &admission.thread_ref,
                session_ref: &admission.session_ref,
                generation: admission.generation,
            },
            disclosure_ref: SARAH_VOICE_DISCLOSURE_REF,
            client_profile: "omega_editor",
            admission_ref: Some(&admission.projection.admission_ref),
            auth: None,
        })
        .expect("serialize session request");
        assert_eq!(
            body["admissionRef"],
            admission.projection.admission_ref.as_str()
        );
    }

    #[test]
    fn direct_nip98_challenge_is_short_lived_and_owner_bound() {
        let now = now_ms().expect("system time");
        let challenge = ChallengeResponse {
            schema: SARAH_VOICE_CHALLENGE_PROTOCOL.into(),
            challenge: "a".repeat(32),
            expires_at_ms: now + MAX_CHALLENGE_LIFETIME_MS + MAX_SERVER_CLOCK_SKEW_MS,
            owner_ref: "owner-from-challenge".into(),
        };
        assert!(validate_challenge(&challenge).is_ok());

        let invalid = ChallengeResponse {
            expires_at_ms: now + MAX_CHALLENGE_LIFETIME_MS + MAX_SERVER_CLOCK_SKEW_MS + 10_000,
            ..challenge
        };
        assert!(validate_challenge(&invalid).is_err());
    }

    #[test]
    fn direct_admission_returns_exact_terms_and_a_bearer_without_issuing_a_ticket() {
        let response = serde_json::to_vec(&serde_json::json!({
            "schema": SARAH_VOICE_ADMISSION_SCHEMA,
            "admitted": true,
            "admissionRef": format!("sarah_voice_admission:{}", "a".repeat(43)),
            "admissionExpiresAtMs": now_ms().expect("system time") + MAX_ADMISSION_LIFETIME_MS,
            "clientProfile": "omega_editor",
            "admissionCohortRef": "sarah_voice_cohort:alpha_v1",
            "creditMode": "metered",
            "creditRateMsatPerMillionTokens": 64_000_000,
            "requiredHoldMsat": 256_000,
            "spendableRemainingCreditMsat": 8_000_000,
            "maxDurationSeconds": 300,
            "capabilityBoundary": {
                "commands": ["context_read", "replace_selection"],
                "confirmationRequired": ["replace_selection"],
                "directShell": false,
                "directGit": false,
                "payment": false,
                "credentialAccess": false,
                "deviceControl": false
            },
            "auth": {
                "method": "nostr_nip98",
                "accessToken": format!("oa_omega_{}", "a".repeat(32)),
                "expiresIn": 900
            }
        }))
        .expect("serialize direct admission response");
        let (admission, access_token) = parse_voice_admission_response(
            &response,
            "owner-from-challenge",
            "omega-device",
            "sarah-owner-private",
            "voice-session",
            true,
        )
        .expect("parse direct admission response");

        assert_eq!(admission.projection.reserved_credit_msat, 256_000);
        assert_eq!(admission.projection.max_duration_seconds, 300);
        assert_eq!(admission.projection.remaining_credit_msat, Some(8_000_000));
        assert_eq!(
            access_token.as_deref(),
            Some(format!("oa_omega_{}", "a".repeat(32)).as_str())
        );
        assert!(!String::from_utf8_lossy(&response).contains("ticket"));
        assert!(!String::from_utf8_lossy(&response).contains("gatewayUrl"));
    }

    #[test]
    fn session_response_requires_the_managed_format() {
        let now = now_ms().expect("system time");
        let mut admission = prepared_admission();
        admission.projection.reserved_credit_msat = 1;
        admission.projection.max_duration_seconds = 60;
        let response = serde_json::to_vec(&serde_json::json!({
            "schema": SARAH_VOICE_PROTOCOL,
            "sessionRef": "voice-session",
            "model": SARAH_VOICE_MODEL,
            "clientProfile": "omega_editor",
            "admissionRef": admission.projection.admission_ref,
            "admissionExpiresAtMs": admission.projection.admission_expires_at_ms,
            "admissionCohortRef": "sarah_voice_cohort:alpha_v1",
            "creditMode": "metered",
            "creditRateMsatPerMillionTokens": 64_000_000,
            "spendableRemainingCreditMsat": 8_000_000,
            "capabilityBoundary": {
                "commands": ["context_read", "replace_selection"],
                "confirmationRequired": ["replace_selection"],
                "directShell": false,
                "directGit": false,
                "payment": false,
                "credentialAccess": false,
                "deviceControl": false
            },
            "gatewayUrl": "wss://openagents.com/api/omega/sarah/voice/connect",
            "ticket": "t".repeat(32),
            "ticketExpiresAtMs": now + 60_000,
            "sessionExpiresAtMs": now + 60_000,
            "reservedCreditMsat": 1,
            "maxDurationSeconds": 60,
            "inputAudio": {
                "codec": "pcm_s16le",
                "sampleRateHz": 24_000,
                "channels": 1
            },
            "outputAudio": {
                "codec": "pcm_s16le",
                "sampleRateHz": 24_000,
                "channels": 1
            }
        }))
        .expect("serialize response");
        let issued =
            parse_voice_session_response(&response, &admission).expect("parse managed response");
        assert_eq!(issued.voice.session_ref, "voice-session");

        let mut invalid: serde_json::Value =
            serde_json::from_slice(&response).expect("decode response");
        invalid["clientProfile"] = serde_json::json!("openagents_mobile");
        let invalid = serde_json::to_vec(&invalid).expect("serialize invalid response");
        assert!(parse_voice_session_response(&invalid, &admission).is_err());

        let mut invalid: serde_json::Value =
            serde_json::from_slice(&response).expect("decode response");
        invalid["admissionRef"] =
            serde_json::json!(format!("sarah_voice_admission:{}", "b".repeat(43)));
        let invalid = serde_json::to_vec(&invalid).expect("serialize changed admission");
        assert!(parse_voice_session_response(&invalid, &admission).is_err());

        let mut invalid: serde_json::Value =
            serde_json::from_slice(&response).expect("decode response");
        invalid["creditRateMsatPerMillionTokens"] = serde_json::json!(1);
        let invalid = serde_json::to_vec(&invalid).expect("serialize changed terms");
        assert!(parse_voice_session_response(&invalid, &admission).is_err());

        let mut invalid: serde_json::Value =
            serde_json::from_slice(&response).expect("decode response");
        invalid
            .as_object_mut()
            .expect("response object")
            .remove("spendableRemainingCreditMsat");
        let invalid = serde_json::to_vec(&invalid).expect("serialize missing terms");
        assert!(parse_voice_session_response(&invalid, &admission).is_err());
    }

    #[test]
    fn session_response_allows_bounded_server_clock_skew() {
        let now = now_ms().expect("system time");
        let mut admission = prepared_admission();
        admission.projection.reserved_credit_msat = 1;
        admission.projection.max_duration_seconds = 900;
        let response_value = serde_json::json!({
            "schema": SARAH_VOICE_PROTOCOL,
            "sessionRef": "voice-session",
            "model": SARAH_VOICE_MODEL,
            "clientProfile": "omega_editor",
            "admissionRef": admission.projection.admission_ref,
            "admissionExpiresAtMs": admission.projection.admission_expires_at_ms,
            "admissionCohortRef": "sarah_voice_cohort:alpha_v1",
            "creditMode": "metered",
            "creditRateMsatPerMillionTokens": 64_000_000,
            "spendableRemainingCreditMsat": 8_000_000,
            "capabilityBoundary": {
                "commands": ["context_read", "replace_selection"],
                "confirmationRequired": ["replace_selection"],
                "directShell": false,
                "directGit": false,
                "payment": false,
                "credentialAccess": false,
                "deviceControl": false
            },
            "gatewayUrl": "wss://openagents.com/api/omega/sarah/voice/connect",
            "ticket": "t".repeat(32),
            "ticketExpiresAtMs": now + MAX_TICKET_LIFETIME_MS + MAX_SERVER_CLOCK_SKEW_MS,
            "sessionExpiresAtMs": now + MAX_SESSION_LIFETIME_MS + MAX_SERVER_CLOCK_SKEW_MS,
            "reservedCreditMsat": 1,
            "maxDurationSeconds": 900,
            "inputAudio": {
                "codec": "pcm_s16le",
                "sampleRateHz": 24_000,
                "channels": 1
            },
            "outputAudio": {
                "codec": "pcm_s16le",
                "sampleRateHz": 24_000,
                "channels": 1
            }
        });
        let body = serde_json::to_vec(&response_value).expect("serialize response");
        assert!(parse_voice_session_response(&body, &admission).is_ok());

        let mut invalid = response_value;
        invalid["ticketExpiresAtMs"] =
            serde_json::json!(now + MAX_TICKET_LIFETIME_MS + MAX_SERVER_CLOCK_SKEW_MS + 10_000);
        let invalid = serde_json::to_vec(&invalid).expect("serialize invalid response");
        assert!(parse_voice_session_response(&invalid, &admission).is_err());
    }

    #[test]
    fn admission_contract_preserves_cohort_price_credit_and_exact_authority() {
        let response_value = serde_json::json!({
            "schema": SARAH_VOICE_ADMISSION_SCHEMA,
            "admitted": true,
            "admissionRef": format!("sarah_voice_admission:{}", "a".repeat(43)),
            "admissionExpiresAtMs": now_ms().expect("system time") + MAX_ADMISSION_LIFETIME_MS,
            "clientProfile": "omega_editor",
            "admissionCohortRef": "sarah_voice_cohort:alpha_v1",
            "creditMode": "metered",
            "creditRateMsatPerMillionTokens": 64_000_000,
            "requiredHoldMsat": 256_000,
            "spendableRemainingCreditMsat": 8_000_000,
            "maxDurationSeconds": 300,
            "capabilityBoundary": {
                "commands": ["context_read", "replace_selection", "start_agent_thread"],
                "confirmationRequired": ["replace_selection", "start_agent_thread"],
                "directShell": false,
                "directGit": false,
                "payment": false,
                "credentialAccess": false,
                "deviceControl": false
            }
        });
        let response: VoiceAdmissionResponse =
            serde_json::from_value(response_value.clone()).expect("decode admission response");
        assert!(valid_admitted_response(&response));

        let mut widened = response_value.clone();
        widened["capabilityBoundary"]["directShell"] = serde_json::json!(true);
        let widened: VoiceAdmissionResponse =
            serde_json::from_value(widened).expect("decode widened response");
        assert!(!valid_admitted_response(&widened));

        let mut missing_ref = response_value.clone();
        missing_ref
            .as_object_mut()
            .expect("admission object")
            .remove("admissionRef");
        let missing_ref: VoiceAdmissionResponse =
            serde_json::from_value(missing_ref).expect("decode missing reference");
        assert!(!valid_admitted_response(&missing_ref));

        let mut expired = response_value;
        expired["admissionExpiresAtMs"] =
            serde_json::json!(now_ms().expect("system time").saturating_sub(1));
        let expired: VoiceAdmissionResponse =
            serde_json::from_value(expired).expect("decode expired admission");
        assert!(!valid_admitted_response(&expired));
    }

    #[test]
    fn entitlement_admission_keeps_nullable_spendable_credit() {
        let response: VoiceAdmissionResponse = serde_json::from_value(serde_json::json!({
            "schema": SARAH_VOICE_ADMISSION_SCHEMA,
            "admitted": true,
            "admissionRef": format!("sarah_voice_admission:{}", "a".repeat(43)),
            "admissionExpiresAtMs": now_ms().expect("system time") + MAX_ADMISSION_LIFETIME_MS,
            "clientProfile": "omega_editor",
            "admissionCohortRef": "sarah_voice_cohort:staging_owner_v1",
            "creditMode": "staging_owner_entitlement",
            "creditRateMsatPerMillionTokens": 64_000_000,
            "requiredHoldMsat": 0,
            "spendableRemainingCreditMsat": null,
            "maxDurationSeconds": 300,
            "capabilityBoundary": {
                "commands": ["context_read"],
                "confirmationRequired": [],
                "directShell": false,
                "directGit": false,
                "payment": false,
                "credentialAccess": false,
                "deviceControl": false
            }
        }))
        .expect("decode entitlement admission");
        assert!(valid_admitted_response(&response));
        assert_eq!(response.spendable_remaining_credit_msat, None);
    }

    #[test]
    fn settlement_contract_binds_final_charge_and_receipt_to_the_session() {
        let response: VoiceSettlementResponse = serde_json::from_value(serde_json::json!({
            "schema": SARAH_VOICE_SETTLEMENT_SCHEMA,
            "sessionRef": "voice-session",
            "state": "settled",
            "creditMode": "metered",
            "finalChargeMsat": 12_345,
            "spendableRemainingCreditMsat": 7_987_655,
            "receiptRef": "sarah_voice_settlement:voice-session"
        }))
        .expect("decode settlement response");
        assert!(valid_settlement_response(&response, "voice-session"));
        assert!(!valid_settlement_response(&response, "another-session"));
    }
}
