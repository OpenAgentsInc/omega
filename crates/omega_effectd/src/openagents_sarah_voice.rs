use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use http_client::{AsyncBody, HttpClient, Method, Request, StatusCode};
use serde::{Deserialize, Serialize};
use smol::io::AsyncReadExt as _;
use url::Url;

use super::openagents_nostr_auth::{HostedSessionBlocker, ready_local_identity, sign_nip98_post};

pub const SARAH_VOICE_NOSTR_CHALLENGE_URL: &str =
    "https://openagents.com/api/omega/sarah/voice/auth/challenge";
pub const SARAH_VOICE_SESSION_URL: &str = "https://openagents.com/api/omega/sarah/voice/session";
pub const SARAH_VOICE_PROTOCOL: &str = "openagents.sarah.voice.v1";
pub const SARAH_VOICE_CHALLENGE_PROTOCOL: &str = "openagents.sarah.voice.auth-challenge.v1";
pub const SARAH_VOICE_DISCLOSURE_REF: &str = "omega.voice.disclosure.v1";
pub const SARAH_VOICE_DEVICE_HEADER: &str = "x-openagents-omega-device-ref";
pub const SARAH_VOICE_SESSION_HEADER: &str = "x-openagents-sarah-voice-session";
pub const SARAH_VOICE_TICKET_HEADER: &str = "x-openagents-sarah-voice-ticket";

const SARAH_VOICE_MODEL: &str = "gpt-realtime-2.1";
const MAX_HTTP_BODY_BYTES: u64 = 64 * 1024;
const MAX_REF_BYTES: usize = 256;
const MAX_ACCESS_TOKEN_BYTES: usize = 16 * 1024;
const MAX_GATEWAY_URL_BYTES: usize = 2 * 1024;
const MAX_CHALLENGE_LIFETIME_MS: u64 = 120_000;

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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IssuedSarahVoiceSession {
    pub voice: ManagedSarahVoiceSession,
    pub access_token: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    auth: Option<NostrAuthentication<'a>>,
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

#[derive(Serialize)]
struct NostrAuthentication<'a> {
    method: &'static str,
    challenge: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VoiceSessionResponse {
    schema: String,
    session_ref: String,
    model: String,
    client_profile: String,
    gateway_url: String,
    ticket: String,
    ticket_expires_at_ms: u64,
    session_expires_at_ms: u64,
    #[serde(rename = "reservedCreditMsat")]
    _reserved_credit_msat: u64,
    max_duration_seconds: u64,
    input_audio: VoiceAudioFormat,
    output_audio: VoiceAudioFormat,
    #[serde(default)]
    auth: Option<IssuedAuthentication>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VoiceAudioFormat {
    codec: String,
    sample_rate_hz: u32,
    channels: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IssuedAuthentication {
    method: String,
    access_token: String,
    expires_in: u64,
}

pub(crate) async fn issue_nostr_sarah_voice_session(
    http_client: &Arc<dyn HttpClient>,
    device_ref: &str,
    thread_ref: &str,
    session_ref: &str,
) -> Result<IssuedSarahVoiceSession, HostedSessionBlocker> {
    validate_ref(device_ref)?;
    validate_ref(thread_ref)?;
    validate_ref(session_ref)?;
    let identity = ready_local_identity()?;
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
        if let Some(blocker) = insufficient_credit_blocker(status, &response_body) {
            return Err(blocker);
        }
        if status.is_success() {
            return Err(HostedSessionBlocker::ResponseInvalid);
        }
        return Err(authentication_status_blocker(status));
    }
    let challenge: ChallengeResponse = serde_json::from_slice(&response_body)
        .map_err(|_| HostedSessionBlocker::ResponseInvalid)?;
    validate_challenge(&challenge)?;

    let request_body = serde_json::to_vec(&VoiceSessionRequest {
        schema: SARAH_VOICE_PROTOCOL,
        identity: VoiceIdentity {
            owner_ref: &challenge.owner_ref,
            device_ref,
            thread_ref,
            session_ref,
            generation: 1,
        },
        disclosure_ref: SARAH_VOICE_DISCLOSURE_REF,
        auth: Some(NostrAuthentication {
            method: "nostr_nip98",
            challenge: &challenge.challenge,
        }),
    })
    .map_err(|_| HostedSessionBlocker::ResponseInvalid)?;
    let authorization = sign_nip98_post(SARAH_VOICE_SESSION_URL, &request_body, &identity)?;
    let (status, response_body) = send_json_post(
        http_client,
        SARAH_VOICE_SESSION_URL,
        Some(&authorization),
        Some(device_ref),
        request_body,
    )
    .await?;
    if status != StatusCode::CREATED {
        if let Some(blocker) = insufficient_credit_blocker(status, &response_body) {
            return Err(blocker);
        }
        if status.is_success() {
            return Err(HostedSessionBlocker::ResponseInvalid);
        }
        return Err(signed_voice_status_blocker(status));
    }
    parse_voice_session_response(
        &response_body,
        &challenge.owner_ref,
        device_ref,
        thread_ref,
        session_ref,
        true,
    )
}

pub(crate) async fn issue_bearer_sarah_voice_session(
    http_client: &Arc<dyn HttpClient>,
    access_token: &str,
    owner_ref: &str,
    device_ref: &str,
    thread_ref: &str,
    session_ref: &str,
) -> Result<IssuedSarahVoiceSession, HostedSessionBlocker> {
    for value in [owner_ref, device_ref, thread_ref, session_ref] {
        validate_ref(value)?;
    }
    let request_body = serde_json::to_vec(&VoiceSessionRequest {
        schema: SARAH_VOICE_PROTOCOL,
        identity: VoiceIdentity {
            owner_ref,
            device_ref,
            thread_ref,
            session_ref,
            generation: 1,
        },
        disclosure_ref: SARAH_VOICE_DISCLOSURE_REF,
        auth: None,
    })
    .map_err(|_| HostedSessionBlocker::ResponseInvalid)?;
    let authorization = format!("Bearer {access_token}");
    let (status, response_body) = send_json_post(
        http_client,
        SARAH_VOICE_SESSION_URL,
        Some(&authorization),
        Some(device_ref),
        request_body,
    )
    .await?;
    if status != StatusCode::CREATED {
        if let Some(blocker) = insufficient_credit_blocker(status, &response_body) {
            return Err(blocker);
        }
        if status.is_success() {
            return Err(HostedSessionBlocker::ResponseInvalid);
        }
        return Err(voice_status_blocker(status));
    }
    parse_voice_session_response(
        &response_body,
        owner_ref,
        device_ref,
        thread_ref,
        session_ref,
        false,
    )
}

fn parse_voice_session_response(
    body: &[u8],
    owner_ref: &str,
    device_ref: &str,
    thread_ref: &str,
    session_ref: &str,
    expect_auth: bool,
) -> Result<IssuedSarahVoiceSession, HostedSessionBlocker> {
    let response: VoiceSessionResponse =
        serde_json::from_slice(body).map_err(|_| HostedSessionBlocker::ResponseInvalid)?;
    let now_ms = now_ms()?;
    let valid_audio = |audio: &VoiceAudioFormat| {
        audio.codec == "pcm_s16le" && audio.sample_rate_hz == 24_000 && audio.channels == 1
    };
    let gateway_url =
        Url::parse(&response.gateway_url).map_err(|_| HostedSessionBlocker::ResponseInvalid)?;
    if response.schema != SARAH_VOICE_PROTOCOL
        || response.model != SARAH_VOICE_MODEL
        || response.client_profile != "omega_editor"
        || response.session_ref != session_ref
        || !valid_ref(&response.session_ref)
        || response.ticket_expires_at_ms <= now_ms
        || response.ticket_expires_at_ms - now_ms > 60_000
        || response.session_expires_at_ms < response.ticket_expires_at_ms
        || response.session_expires_at_ms - now_ms > 900_000
        || response.max_duration_seconds < 60
        || response.max_duration_seconds > 900
        || !valid_base64url(&response.ticket)
        || response.gateway_url.len() > MAX_GATEWAY_URL_BYTES
        || gateway_url.scheme() != "wss"
        || gateway_url.host_str() != Some("openagents.com")
        || !gateway_url.username().is_empty()
        || gateway_url.password().is_some()
        || gateway_url.query().is_some()
        || gateway_url.fragment().is_some()
        || !valid_audio(&response.input_audio)
        || !valid_audio(&response.output_audio)
        || expect_auth != response.auth.is_some()
    {
        return Err(HostedSessionBlocker::ResponseInvalid);
    }
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
    Ok(IssuedSarahVoiceSession {
        voice: ManagedSarahVoiceSession {
            owner_ref: owner_ref.to_string(),
            device_ref: device_ref.to_string(),
            thread_ref: thread_ref.to_string(),
            session_ref: session_ref.to_string(),
            generation: 1,
            disclosure_ref: SARAH_VOICE_DISCLOSURE_REF.to_string(),
            gateway_url: response.gateway_url,
            ticket: response.ticket,
            ticket_expires_at_ms: response.ticket_expires_at_ms,
            session_expires_at_ms: response.session_expires_at_ms,
        },
        access_token,
    })
}

fn validate_challenge(challenge: &ChallengeResponse) -> Result<(), HostedSessionBlocker> {
    let now_ms = now_ms()?;
    if challenge.schema != SARAH_VOICE_CHALLENGE_PROTOCOL
        || !valid_ref(&challenge.owner_ref)
        || !valid_base64url(&challenge.challenge)
        || challenge.expires_at_ms <= now_ms
        || challenge.expires_at_ms - now_ms > MAX_CHALLENGE_LIFETIME_MS
    {
        return Err(HostedSessionBlocker::ResponseInvalid);
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_and_voice_endpoints_are_exact_https_contracts() {
        assert_eq!(
            SARAH_VOICE_NOSTR_CHALLENGE_URL,
            "https://openagents.com/api/omega/sarah/voice/auth/challenge"
        );
        assert_eq!(
            SARAH_VOICE_SESSION_URL,
            "https://openagents.com/api/omega/sarah/voice/session"
        );
        assert!(valid_base64url(&"a".repeat(32)));
        assert!(!valid_base64url("not padded base64="));
        assert!(valid_access_token(&format!("oa_omega_{}", "a".repeat(32))));
        assert!(!valid_access_token("oa_omega_too-short"));

        let request = serde_json::to_value(ChallengeRequest {
            schema: SARAH_VOICE_CHALLENGE_PROTOCOL,
            device_ref: "omega-device",
            pubkey: &"a".repeat(64),
        })
        .expect("serialize challenge request");
        assert_eq!(
            request,
            serde_json::json!({
                "schema": SARAH_VOICE_CHALLENGE_PROTOCOL,
                "deviceRef": "omega-device",
                "pubkey": "a".repeat(64),
            })
        );
    }

    #[test]
    fn signed_body_is_bounded_and_carries_the_server_challenge() {
        let body = serde_json::to_vec(&VoiceSessionRequest {
            schema: SARAH_VOICE_PROTOCOL,
            identity: VoiceIdentity {
                owner_ref: "nostr:owner",
                device_ref: "omega-device",
                thread_ref: "sarah-owner-private",
                session_ref: "voice-session",
                generation: 1,
            },
            disclosure_ref: SARAH_VOICE_DISCLOSURE_REF,
            auth: Some(NostrAuthentication {
                method: "nostr_nip98",
                challenge: "abcdefghijklmnopqrstuvwxyzABCDEF",
            }),
        })
        .expect("serialize signed voice request");
        let value: serde_json::Value =
            serde_json::from_slice(&body).expect("decode signed voice request");
        assert_eq!(value["auth"]["method"], "nostr_nip98");
        assert_eq!(
            value["auth"]["challenge"],
            "abcdefghijklmnopqrstuvwxyzABCDEF"
        );
        assert_eq!(value["identity"]["generation"], 1);
        assert!(body.len() < 8 * 1024);
    }

    #[test]
    fn signed_response_requires_the_managed_format_and_short_lived_bearer() {
        let now = now_ms().expect("system time");
        let response = serde_json::to_vec(&serde_json::json!({
            "schema": SARAH_VOICE_PROTOCOL,
            "sessionRef": "voice-session",
            "model": SARAH_VOICE_MODEL,
            "clientProfile": "omega_editor",
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
            },
            "auth": {
                "method": "nostr_nip98",
                "accessToken": format!("oa_omega_{}", "a".repeat(32)),
                "expiresIn": 900
            }
        }))
        .expect("serialize response");
        let issued = parse_voice_session_response(
            &response,
            "owner",
            "device",
            "thread",
            "voice-session",
            true,
        )
        .expect("parse managed response");
        assert!(issued.access_token.is_some());

        let mut invalid: serde_json::Value =
            serde_json::from_slice(&response).expect("decode response");
        invalid["auth"]["expiresIn"] = serde_json::json!(901);
        let invalid = serde_json::to_vec(&invalid).expect("serialize invalid response");
        assert!(
            parse_voice_session_response(
                &invalid,
                "owner",
                "device",
                "thread",
                "voice-session",
                true,
            )
            .is_err()
        );

        let mut invalid: serde_json::Value =
            serde_json::from_slice(&response).expect("decode response");
        invalid["clientProfile"] = serde_json::json!("openagents_mobile");
        let invalid = serde_json::to_vec(&invalid).expect("serialize invalid response");
        assert!(
            parse_voice_session_response(
                &invalid,
                "owner",
                "device",
                "thread",
                "voice-session",
                true,
            )
            .is_err()
        );
    }
}
