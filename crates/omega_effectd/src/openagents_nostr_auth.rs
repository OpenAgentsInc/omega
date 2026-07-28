use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::Engine as _;
use http_client::{AsyncBody, HttpClient, Method, Request};
use omega_identity::{
    AdmittedSigningRequest, IdentityService, ReceiptRef, SigningPurpose, UnsignedEventTemplate,
};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use smol::io::AsyncReadExt as _;
use thiserror::Error;

pub const OPENAGENTS_NOSTR_SESSION_URL: &str = "https://openagents.com/api/omega/auth/session";

/// Stable so a retried provision resumes the same durable create transaction
/// instead of opening a second one and landing custody in `Conflict`.
const PROVISION_RECEIPT_REF: &str = "omega-hosted-session-provision-v1";

const MAX_HTTP_BODY_BYTES: u64 = 64 * 1024;
const MAX_ACCESS_TOKEN_BYTES: usize = 16 * 1024;
const NIP98_KIND: u16 = 27_235;

/// Why a hosted OpenAgents session could not be obtained, in terms a person
/// can act on.
///
/// Every variant is safe to log and to render: it carries a custody state name
/// or an HTTP status, never a token, key, or signature. It exists because the
/// previous code folded every one of these into a single `Unavailable` phase,
/// so the only thing the owner ever saw was "sign-in was not completed" — true
/// of an unprovisioned install, a rejected proof, and a dead network alike.
#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum HostedSessionBlocker {
    #[error(
        "this installation has no signing identity Omega can use ({custody_state}). \
         Open Omega's onboarding to create or recover one."
    )]
    IdentityUnavailable { custody_state: String },
    #[error("Omega could not sign the hosted sign-in proof ({reason}).")]
    ProofSigningFailed { reason: String },
    #[error("OpenAgents could not be reached to sign in.")]
    ServiceUnreachable,
    #[error(
        "OpenAgents rejected this installation's sign-in proof (HTTP {status}). \
         Retrying will not change the answer: this identity is not admitted for hosted Omega."
    )]
    ProofRejected { status: u16 },
    /// Google Frontend returns 411 when a POST is missing `Content-Length`.
    /// That is a client framing bug, not an identity refusal — naming it as
    /// "not admitted" sent the owner looking for an allowlist that does not
    /// exist.
    #[error(
        "Omega could not complete hosted sign-in: the request was missing its \
         Content-Length (HTTP 411). This is a client bug, not an identity refusal."
    )]
    RequestFramingRejected,
    #[error("OpenAgents could not issue a session right now (HTTP {status}).")]
    ServiceUnavailable { status: u16 },
    #[error("OpenAgents returned a sign-in response Omega could not use.")]
    ResponseInvalid,
    #[error("OpenAgents did not accept the session it had just issued.")]
    SessionNotVerified,
    #[error("Omega could not store the hosted session on this device.")]
    CredentialStorageFailed,
}

impl HostedSessionBlocker {
    /// A single-line, public-safe summary for logs and UI text.
    pub fn summary(&self) -> String {
        self.to_string()
    }

    /// Whether sending the same request again could plausibly succeed.
    ///
    /// The old copy told the owner to "send the message again" for every
    /// failure, including a 401 that no number of retries can turn into a
    /// session.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::ServiceUnreachable
                | Self::ServiceUnavailable { .. }
                | Self::SessionNotVerified
                | Self::CredentialStorageFailed
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MintedOpenAgentsSession {
    pub access_token: String,
    pub expires_in: u64,
    pub user: MintedOpenAgentsUser,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MintedOpenAgentsUser {
    pub user_id: String,
}

pub async fn mint_openagents_nostr_session(
    http_client: &Arc<dyn HttpClient>,
) -> std::result::Result<MintedOpenAgentsSession, HostedSessionBlocker> {
    let identity_service = IdentityService::system(*app_identity::CHANNEL);
    let channel_name = app_identity::CHANNEL.display_name();
    let receipt_ref = ReceiptRef::new(PROVISION_RECEIPT_REF).map_err(|error| {
        HostedSessionBlocker::ProofSigningFailed {
            reason: error.to_string(),
        }
    })?;
    // A fresh install has no identity and nothing else creates one: the
    // startup onboarding gate is dormant, so without this the hosted lane is
    // unreachable on every new machine.
    let custody = identity_service
        .provision_unattended(receipt_ref)
        .map_err(|error| {
            log::error!(
                "hosted OpenAgents sign-in: {} identity is not usable: {error}",
                channel_name
            );
            HostedSessionBlocker::IdentityUnavailable {
                custody_state: error.to_string(),
            }
        })?;
    let identity = custody
        .identity
        .ok_or_else(|| HostedSessionBlocker::IdentityUnavailable {
            custody_state: "no public identity was resolved".to_string(),
        })?;
    log::info!(
        "hosted OpenAgents sign-in: signing as {} identity {}",
        channel_name,
        identity.public_key_hex().as_str()
    );

    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HostedSessionBlocker::ProofSigningFailed {
            reason: "the system clock is before the Unix epoch".to_string(),
        })?
        .as_secs();
    let empty_payload_hash = format!("{:x}", Sha256::digest([]));
    let semantic_binding = serde_json::to_vec(&serde_json::json!({
        "url": OPENAGENTS_NOSTR_SESSION_URL,
        "method": "POST",
        "payload": empty_payload_hash,
        "createdAt": created_at,
        "publicKey": identity.public_key_hex().as_str(),
    }))
    .map_err(|error| HostedSessionBlocker::ProofSigningFailed {
        reason: error.to_string(),
    })?;
    let digest = format!("{:x}", Sha256::digest(semantic_binding));
    let request_ref = ReceiptRef::new(format!("nip98.{}", &digest[..32])).map_err(|error| {
        HostedSessionBlocker::ProofSigningFailed {
            reason: error.to_string(),
        }
    })?;
    let signed = identity_service
        .sign(&AdmittedSigningRequest {
            request_ref,
            identity_ref: identity.identity_ref().clone(),
            purpose: SigningPurpose::NostrEvent,
            event: UnsignedEventTemplate {
                created_at,
                kind: NIP98_KIND,
                tags: vec![
                    vec!["u".to_string(), OPENAGENTS_NOSTR_SESSION_URL.to_string()],
                    vec!["method".to_string(), "POST".to_string()],
                    vec!["payload".to_string(), empty_payload_hash],
                ],
                content: String::new(),
            },
        })
        .map_err(|error| {
            log::error!("hosted OpenAgents sign-in: signing the proof failed: {error}");
            HostedSessionBlocker::ProofSigningFailed {
                reason: error.to_string(),
            }
        })?;
    let authorization =
        base64::engine::general_purpose::STANDARD.encode(signed.signed_event_json.as_bytes());
    // Empty body on purpose: the NIP-98 `payload` tag is the SHA-256 of the
    // empty byte string. Google Frontend (HTTP/1.1) answers 411 Length Required
    // when a POST has a Content-Type and no Content-Length — which is how
    // `AsyncBody::empty()` was being framed — so the length must be stated
    // explicitly as zero. Without it the owner saw "identity is not admitted"
    // for a request that never reached the auth handler.
    let request = Request::builder()
        .method(Method::POST)
        .uri(OPENAGENTS_NOSTR_SESSION_URL)
        .header("authorization", format!("Nostr {authorization}"))
        .header("content-type", "application/octet-stream")
        .header("content-length", "0")
        .body(AsyncBody::empty())
        .map_err(|error| HostedSessionBlocker::ProofSigningFailed {
            reason: error.to_string(),
        })?;
    let mut response = http_client.send(request).await.map_err(|error| {
        log::error!(
            "hosted OpenAgents sign-in: {OPENAGENTS_NOSTR_SESSION_URL} was unreachable: {error}"
        );
        HostedSessionBlocker::ServiceUnreachable
    })?;
    let status = response.status();
    let mut body = Vec::new();
    response
        .body_mut()
        .take(MAX_HTTP_BODY_BYTES)
        .read_to_end(&mut body)
        .await
        .map_err(|error| {
            log::error!("hosted OpenAgents sign-in: reading the response failed: {error}");
            HostedSessionBlocker::ServiceUnreachable
        })?;
    if !status.is_success() {
        // The body of a refusal is a short public error code
        // (`unauthorized`, `omega_nostr_owner_unavailable`, …). It is the one
        // fact that tells an operator whether the identity was refused or the
        // service was not configured, so keep it.
        log::error!(
            "hosted OpenAgents sign-in: {OPENAGENTS_NOSTR_SESSION_URL} refused {} identity {} with HTTP {status}: {}",
            channel_name,
            identity.public_key_hex().as_str(),
            public_error_code(&body)
        );
        let code = status.as_u16();
        return Err(if code == 411 {
            HostedSessionBlocker::RequestFramingRejected
        } else if status.is_client_error() {
            HostedSessionBlocker::ProofRejected { status: code }
        } else {
            HostedSessionBlocker::ServiceUnavailable { status: code }
        });
    }
    let session: MintedOpenAgentsSession = serde_json::from_slice(&body).map_err(|error| {
        log::error!(
            "hosted OpenAgents sign-in: the session response could not be decoded: {error}"
        );
        HostedSessionBlocker::ResponseInvalid
    })?;
    if session.access_token.trim().is_empty()
        || session.access_token.len() > MAX_ACCESS_TOKEN_BYTES
        || session.expires_in == 0
        || session.user.user_id.trim().is_empty()
    {
        log::error!("hosted OpenAgents sign-in: the session response was incomplete");
        return Err(HostedSessionBlocker::ResponseInvalid);
    }
    log::info!("hosted OpenAgents sign-in: session issued (HTTP {status})");
    Ok(session)
}

/// Extract the server's short `error` code, and nothing else.
///
/// Refusal bodies are public-safe by contract, but bounding what is logged to
/// a known field keeps an unexpected body — one that grew a token or an email
/// — out of a log file the owner is asked to paste into an issue.
fn public_error_code(body: &[u8]) -> String {
    #[derive(Deserialize)]
    struct PublicError {
        error: String,
    }

    serde_json::from_slice::<PublicError>(body)
        .ok()
        .filter(|parsed| {
            parsed.error.len() <= 64
                && parsed
                    .error
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
        .map(|parsed| parsed.error)
        .unwrap_or_else(|| "no public error code".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_is_https_and_owner_scoped() {
        assert_eq!(
            OPENAGENTS_NOSTR_SESSION_URL,
            "https://openagents.com/api/omega/auth/session"
        );
        assert_eq!(
            format!("{:x}", Sha256::digest([])),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// The defect this replaces: a 401 was reported as "send the message again
    /// to connect hosted Omega", which is advice that can never work.
    #[test]
    fn a_refused_proof_is_named_and_not_offered_as_retryable() {
        let refused = HostedSessionBlocker::ProofRejected { status: 401 };
        assert!(!refused.is_retryable());
        assert!(refused.summary().contains("401"));
        assert!(refused.summary().contains("not admitted"));

        let unreachable = HostedSessionBlocker::ServiceUnreachable;
        assert!(unreachable.is_retryable());

        let unprovisioned = HostedSessionBlocker::IdentityUnavailable {
            custody_state: "identity custody is unavailable in state Lost".to_string(),
        };
        assert!(!unprovisioned.is_retryable());
        assert!(unprovisioned.summary().contains("Lost"));
    }

    #[test]
    fn no_blocker_summary_can_carry_credential_material() {
        for blocker in [
            HostedSessionBlocker::IdentityUnavailable {
                custody_state: "state".to_string(),
            },
            HostedSessionBlocker::ProofSigningFailed {
                reason: "reason".to_string(),
            },
            HostedSessionBlocker::ServiceUnreachable,
            HostedSessionBlocker::ProofRejected { status: 401 },
            HostedSessionBlocker::RequestFramingRejected,
            HostedSessionBlocker::ServiceUnavailable { status: 503 },
            HostedSessionBlocker::ResponseInvalid,
            HostedSessionBlocker::SessionNotVerified,
            HostedSessionBlocker::CredentialStorageFailed,
        ] {
            let summary = blocker.summary().to_lowercase();
            for forbidden in ["bearer", "oa_omega_", "nsec", "authorization", "signature"] {
                assert!(
                    !summary.contains(forbidden),
                    "{summary} leaks `{forbidden}`"
                );
            }
        }
    }

    /// HTTP 411 is Google Frontend rejecting a POST with no Content-Length.
    /// It must never be narrated as an identity allowlist refusal.
    #[test]
    fn length_required_is_named_as_framing_not_admission() {
        let framing = HostedSessionBlocker::RequestFramingRejected;
        assert!(!framing.is_retryable());
        assert!(framing.summary().contains("411") || framing.summary().contains("Content-Length"));
        assert!(!framing.summary().to_lowercase().contains("not admitted"));
        assert!(!framing.summary().to_lowercase().contains("gemini_api_key"));
    }

    #[test]
    fn only_a_bounded_public_error_code_is_logged() {
        assert_eq!(
            public_error_code(br#"{"error":"unauthorized"}"#),
            "unauthorized"
        );
        assert_eq!(
            public_error_code(br#"{"error":"omega_nostr_owner_unavailable"}"#),
            "omega_nostr_owner_unavailable"
        );
        assert_eq!(
            public_error_code(br#"{"error":"Bearer oa_omega_secret value"}"#),
            "no public error code"
        );
        assert_eq!(
            public_error_code(b"<html>gateway error</html>"),
            "no public error code"
        );
    }
}
