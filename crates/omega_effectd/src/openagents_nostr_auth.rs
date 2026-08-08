use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::Engine as _;
use http_client::{AsyncBody, HttpClient, Method, Request};
use nostr::{Event, JsonUtil as _};
use omega_identity::{
    AccountRegistryService, AdmittedSigningRequest, CustodyState, IdentityService, PublicIdentity,
    ReceiptRef, SignerKind, SigningPurpose, UnsignedEventTemplate,
};
use omega_signer_broker::{
    Nip46WebSocketTransport, RemoteSignerMetadata, SignerBroker, SignerRoute,
};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use smol::io::AsyncReadExt as _;
use thiserror::Error;

pub const OPENAGENTS_NOSTR_SESSION_URL: &str = "https://openagents.com/api/omega/auth/session";

const MAX_HTTP_BODY_BYTES: u64 = 64 * 1024;
const MAX_ACCESS_TOKEN_BYTES: usize = 16 * 1024;
const NIP98_KIND: u16 = 27_235;
const NIP98_MAX_CLOCK_SKEW_SECONDS: u64 = 60;
const MAX_NIP98_ISSUERS: usize = 128;

/// The prefix every public error code the hosted auth route emits for a failure
/// of its own session-storage path shares.
///
/// omega#160. The confirmed production cause is a Cloud Run cold start: a fresh
/// revision takes traffic before its Cloud SQL Auth Proxy sidecar has
/// connected, so the first database-touching requests fail on a connect
/// timeout. The route reported that as `omega_nostr_auth_storage_unavailable`,
/// and now also as `..._timeout` and `..._unreachable` for the same class, so
/// the prefix — not any one code — is what Omega recognizes. The owner's binary
/// talks to whichever revision is live, which may still be the older build.
const STORAGE_UNAVAILABLE_CODE_PREFIX: &str = "omega_nostr_auth_storage";

static NIP98_ISSUANCE_LEDGER: OnceLock<Mutex<BTreeMap<String, u64>>> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedNip98Proof {
    pub event_id: String,
    pub public_key_hex: String,
    pub created_at: u64,
}

#[derive(Debug, Default)]
pub struct Nip98ProofReplayGuard {
    consumed_event_ids: BTreeSet<String>,
}

impl Nip98ProofReplayGuard {
    pub fn verify_and_consume(
        &mut self,
        authorization: &str,
        expected_url: &str,
        expected_method: &str,
        expected_payload: &[u8],
        expected_public_key_hex: &str,
        now: u64,
    ) -> Result<VerifiedNip98Proof, HostedSessionBlocker> {
        let proof = verify_nip98_authorization(
            authorization,
            expected_url,
            expected_method,
            expected_payload,
            expected_public_key_hex,
            now,
        )?;
        if !self.consumed_event_ids.insert(proof.event_id.clone()) {
            return Err(HostedSessionBlocker::ProofAlreadyUsed);
        }
        Ok(proof)
    }
}

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
        "OpenAgents needs Omega's local identity. Finish identity setup in Omega, then try again."
    )]
    IdentityMissing,
    #[error(
        "OpenAgents found a protected local identity that this Omega profile has not adopted. \
         Review and adopt it in Omega's identity setup, then try again."
    )]
    IdentityAdoptionRequired,
    #[error(
        "Omega's local identity is unavailable ({custody_state}). \
         Open identity setup to unlock, recover, or resolve it."
    )]
    IdentityUnavailable { custody_state: String },
    #[error("Omega could not sign the hosted sign-in proof ({reason}).")]
    ProofSigningFailed { reason: String },
    #[error("The hosted sign-in proof did not pass exact-request validation.")]
    ProofInvalid,
    #[error("The hosted sign-in proof was outside the 60-second freshness window.")]
    ProofExpired,
    #[error("The hosted sign-in proof was already consumed. Create a fresh proof and retry.")]
    ProofAlreadyUsed,
    #[error("OpenAgents could not be reached to sign in.")]
    ServiceUnreachable,
    #[error(
        "OpenAgents is temporarily limiting Sarah sign-in challenges. Wait briefly, then retry."
    )]
    ChallengeRateLimited,
    #[error(
        "OpenAgents rejected this installation's sign-in proof (HTTP {status}). \
         Retrying will not change the answer: this identity is not admitted for hosted Omega."
    )]
    ProofRejected { status: u16 },
    #[error(
        "The one-use Sarah sign-in proof was already used or expired. Retry to obtain a fresh challenge."
    )]
    VoiceProofExpired,
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
    /// The hosted service answered, but its own session storage was not ready.
    ///
    /// omega#160. This is a start-up window, not a refusal: a new Cloud Run
    /// revision routes traffic before its Cloud SQL Auth Proxy sidecar has
    /// connected, and the same identity that fails here succeeds seconds
    /// later. It is named apart from [`Self::ServiceUnavailable`] so the retry
    /// decision is made on the server's own typed code rather than on a guess
    /// about a status, and the code is carried so the log says which of the
    /// storage codes was returned.
    #[error(
        "OpenAgents is still starting its session storage (HTTP {status}, {code}). \
         Omega re-attempted across the start-up window and it did not clear. Try again shortly."
    )]
    ServiceStorageUnavailable { status: u16, code: String },
    #[error(
        "OpenAgents rejected the Sarah voice session request (HTTP {status}). \
         Check voice access and credits in your OpenAgents account."
    )]
    VoiceSessionRejected { status: u16 },
    #[error(
        "Insufficient OpenAgents voice credits (HTTP {status}). \
         Add credits or enable voice access in your OpenAgents account, then try again."
    )]
    InsufficientVoiceCredits { status: u16 },
    #[error(
        "This OpenAgents account is not active in Sarah voice cohort {cohort_ref}. Voice credit alone does not grant admission."
    )]
    VoiceCohortInactive { cohort_ref: String },
    #[error(
        "Sarah voice cohort {cohort_ref} admitted this account, but its spendable credit is below the required hold."
    )]
    VoiceAdmissionInsufficientCredit { cohort_ref: String },
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
                | Self::ChallengeRateLimited
                | Self::ProofExpired
                | Self::ProofAlreadyUsed
                | Self::VoiceProofExpired
                | Self::ServiceUnavailable { .. }
                | Self::ServiceStorageUnavailable { .. }
                | Self::VoiceSessionRejected { status: 409 | 429 }
                | Self::InsufficientVoiceCredits { .. }
                | Self::SessionNotVerified
                | Self::CredentialStorageFailed
        )
    }

    /// Whether this failure is the hosted service's own start-up window: the
    /// deployment is reachable but its storage path is not ready yet.
    ///
    /// omega#160. This is the only class Omega re-attempts on a backoff inside
    /// a single sign-in. It is deliberately narrower than [`Self::is_retryable`]
    /// — a rate limit, a consumed proof, or a local storage failure are all
    /// retryable by a person and none of them clear by waiting a few seconds
    /// inside one call. Everything outside this set fails on the first answer.
    pub fn is_transient_service_window(&self) -> bool {
        matches!(
            self,
            Self::ServiceStorageUnavailable { .. }
                | Self::ServiceUnreachable
                | Self::ServiceUnavailable { status: 502 | 504 }
        )
    }

    pub fn requires_voice_access(&self) -> bool {
        matches!(
            self,
            Self::InsufficientVoiceCredits { .. } | Self::VoiceAdmissionInsufficientCredit { .. }
        )
    }
}

fn identity_blocker(state: CustodyState) -> Option<HostedSessionBlocker> {
    match state {
        CustodyState::Ready => None,
        CustodyState::Absent => Some(HostedSessionBlocker::IdentityMissing),
        CustodyState::Unadopted => Some(HostedSessionBlocker::IdentityAdoptionRequired),
        state => Some(HostedSessionBlocker::IdentityUnavailable {
            custody_state: format!("{state:?}"),
        }),
    }
}

pub(crate) fn ready_local_identity() -> Result<PublicIdentity, HostedSessionBlocker> {
    let identity_service = IdentityService::system(*app_identity::CHANNEL);
    let channel_name = app_identity::CHANNEL.display_name();
    let custody = identity_service.inspect().map_err(|error| {
        log::error!(
            "hosted OpenAgents sign-in: {} identity is not usable: {error}",
            channel_name
        );
        HostedSessionBlocker::IdentityUnavailable {
            custody_state: error.to_string(),
        }
    })?;
    if let Some(blocker) = identity_blocker(custody.state) {
        return Err(blocker);
    }
    custody
        .identity
        .ok_or_else(|| HostedSessionBlocker::IdentityUnavailable {
            custody_state: "no public identity was resolved".to_string(),
        })
}

pub(crate) fn ready_account_identity() -> Result<PublicIdentity, HostedSessionBlocker> {
    let registry = AccountRegistryService::for_channel(*app_identity::CHANNEL);
    let selection =
        registry
            .selection_token()
            .map_err(|error| HostedSessionBlocker::IdentityUnavailable {
                custody_state: error.to_string(),
            })?;
    let dashboard =
        registry
            .inspect()
            .map_err(|error| HostedSessionBlocker::IdentityUnavailable {
                custody_state: error.to_string(),
            })?;
    let active = dashboard
        .accounts
        .iter()
        .find(|account| account.account_ref == selection.account_ref)
        .ok_or_else(|| HostedSessionBlocker::IdentityUnavailable {
            custody_state: "the active account is unavailable".to_string(),
        })?;
    match active.signer.kind {
        SignerKind::LocalNative => {
            let identity = ready_local_identity()?;
            if identity != selection.identity {
                return Err(HostedSessionBlocker::IdentityUnavailable {
                    custody_state: "local identity does not match the active account".to_string(),
                });
            }
            Ok(identity)
        }
        SignerKind::RemoteNip46 => {
            registry
                .remote_signer_capability(&selection)
                .map_err(|error| HostedSessionBlocker::IdentityUnavailable {
                    custody_state: error.to_string(),
                })?;
            Ok(selection.identity)
        }
        SignerKind::BrowserNip07
        | SignerKind::AndroidNip55
        | SignerKind::DeviceGrant
        | SignerKind::AgentGrant => Err(HostedSessionBlocker::IdentityUnavailable {
            custody_state: "the active signer has no hosted-auth route".to_string(),
        }),
    }
}

pub(crate) async fn sign_nip98_post(
    url: &str,
    payload: &[u8],
    identity: &PublicIdentity,
) -> Result<String, HostedSessionBlocker> {
    sign_nip98_request(
        url,
        "POST",
        Some(payload),
        Some(identity.public_key_hex().as_str()),
    )
    .await
}

pub async fn sign_nip98_request(
    url: &str,
    method: &str,
    payload: Option<&[u8]>,
    expected_public_key_hex: Option<&str>,
) -> Result<String, HostedSessionBlocker> {
    if !valid_nip98_url(url) || !valid_http_method(method) {
        return Err(HostedSessionBlocker::ProofSigningFailed {
            reason: "the request URL or uppercase HTTP method is invalid".to_string(),
        });
    }
    // A generation bump between resolving the selection token and the broker's
    // validation (an account refresh, a profile hydration commit) is a benign
    // race, not a refusal: re-resolve and retry exactly once. The identity
    // binding below still pins the retry to the account the sign-in started
    // with, so a genuine account switch fails instead of silently signing as
    // someone else. Without this, one mid-proof bump dead-ended the whole
    // hosted lane (owner outage 2026-07-30).
    match sign_nip98_request_once(url, method, payload, expected_public_key_hex).await {
        Err(HostedSessionBlocker::ProofSigningFailed { reason })
            if reason == STALE_SELECTION_REASON =>
        {
            log::info!(
                "hosted OpenAgents sign-in: the account selection advanced mid-proof; \
                 re-resolving and retrying once"
            );
            sign_nip98_request_once(url, method, payload, expected_public_key_hex).await
        }
        result => result,
    }
}

const STALE_SELECTION_REASON: &str = "the active account selection changed";

async fn sign_nip98_request_once(
    url: &str,
    method: &str,
    payload: Option<&[u8]>,
    expected_public_key_hex: Option<&str>,
) -> Result<String, HostedSessionBlocker> {
    let registry = AccountRegistryService::for_channel(*app_identity::CHANNEL);
    let selection =
        registry
            .selection_token()
            .map_err(|error| HostedSessionBlocker::IdentityUnavailable {
                custody_state: error.to_string(),
            })?;
    if expected_public_key_hex
        .is_some_and(|expected| expected != selection.identity.public_key_hex().as_str())
    {
        return Err(HostedSessionBlocker::ProofSigningFailed {
            reason: "the active Omega identity changed before hosted sign-in".to_string(),
        });
    }
    let wall_clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HostedSessionBlocker::ProofSigningFailed {
            reason: "the system clock is before the Unix epoch".to_string(),
        })?
        .as_secs();
    let issuer_ref = format!(
        "{}:{}:{}",
        selection.account_ref.as_str(),
        selection.generation,
        selection.identity.public_key_hex().as_str()
    );
    let created_at = next_nip98_created_at(&issuer_ref, wall_clock)?;
    let payload_hash = payload.map(|payload| format!("{:x}", Sha256::digest(payload)));
    let event = nip98_event(url, method, payload_hash.clone(), created_at);
    let semantic_binding = serde_json::to_vec(&serde_json::json!({
        "url": url,
        "method": method.to_uppercase(),
        "payload": payload_hash,
        "createdAt": created_at,
        "publicKey": selection.identity.public_key_hex().as_str(),
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
    let request = AdmittedSigningRequest {
        request_ref,
        identity_ref: selection.identity.identity_ref().clone(),
        purpose: SigningPurpose::NostrEvent,
        event,
    };
    let dashboard =
        registry
            .inspect()
            .map_err(|error| HostedSessionBlocker::IdentityUnavailable {
                custody_state: error.to_string(),
            })?;
    let active = dashboard
        .accounts
        .iter()
        .find(|account| account.account_ref == selection.account_ref)
        .ok_or_else(|| HostedSessionBlocker::IdentityUnavailable {
            custody_state: "the active account is unavailable".to_string(),
        })?;
    let route = match active.signer.kind {
        SignerKind::LocalNative => SignerRoute::Local {
            identity_service: Arc::new(IdentityService::system(*app_identity::CHANNEL)),
        },
        SignerKind::RemoteNip46 => {
            let capability = registry
                .remote_signer_capability(&selection)
                .map_err(|error| HostedSessionBlocker::IdentityUnavailable {
                    custody_state: error.to_string(),
                })?;
            SignerRoute::RemoteNip46 {
                metadata: RemoteSignerMetadata { capability },
                transport: Arc::new(Nip46WebSocketTransport::system()),
            }
        }
        SignerKind::BrowserNip07
        | SignerKind::AndroidNip55
        | SignerKind::DeviceGrant
        | SignerKind::AgentGrant => {
            return Err(HostedSessionBlocker::IdentityUnavailable {
                custody_state: "the active external signer has no broker route".to_string(),
            });
        }
    };
    let signed = SignerBroker::system()
        .sign(&route, selection, request)
        .await
        .map_err(|error| {
            log::error!("hosted OpenAgents sign-in: signing the proof failed: {error}");
            HostedSessionBlocker::ProofSigningFailed {
                reason: error.to_string(),
            }
        })?;
    Ok(format!(
        "Nostr {}",
        base64::engine::general_purpose::STANDARD.encode(signed.signed_event_json.as_bytes())
    ))
}

fn next_nip98_created_at(issuer_ref: &str, wall_clock: u64) -> Result<u64, HostedSessionBlocker> {
    let ledger = NIP98_ISSUANCE_LEDGER.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut ledger = ledger
        .lock()
        .map_err(|_| HostedSessionBlocker::ProofSigningFailed {
            reason: "the NIP-98 issuance ledger is unavailable".to_string(),
        })?;
    let created_at = match ledger.get(issuer_ref).copied() {
        Some(previous) => previous.saturating_add(1).max(wall_clock),
        None => wall_clock,
    };
    if created_at > wall_clock.saturating_add(NIP98_MAX_CLOCK_SKEW_SECONDS) {
        return Err(HostedSessionBlocker::ChallengeRateLimited);
    }
    if ledger.len() >= MAX_NIP98_ISSUERS && !ledger.contains_key(issuer_ref) {
        let Some(oldest) = ledger
            .iter()
            .min_by_key(|(_, created_at)| **created_at)
            .map(|(issuer, _)| issuer.clone())
        else {
            return Err(HostedSessionBlocker::ProofSigningFailed {
                reason: "the NIP-98 issuance ledger could not evict an issuer".to_string(),
            });
        };
        ledger.remove(&oldest);
    }
    ledger.insert(issuer_ref.to_string(), created_at);
    Ok(created_at)
}

fn nip98_event(
    url: &str,
    method: &str,
    payload_hash: Option<String>,
    created_at: u64,
) -> UnsignedEventTemplate {
    let mut tags = vec![
        vec!["u".to_string(), url.to_string()],
        vec!["method".to_string(), method.to_uppercase()],
    ];
    if let Some(payload_hash) = payload_hash {
        tags.push(vec!["payload".to_string(), payload_hash]);
    }
    UnsignedEventTemplate {
        created_at,
        kind: NIP98_KIND,
        tags,
        content: String::new(),
    }
}

pub fn verify_nip98_authorization(
    authorization: &str,
    expected_url: &str,
    expected_method: &str,
    expected_payload: &[u8],
    expected_public_key_hex: &str,
    now: u64,
) -> Result<VerifiedNip98Proof, HostedSessionBlocker> {
    if !valid_nip98_url(expected_url) || !valid_http_method(expected_method) {
        return Err(HostedSessionBlocker::ProofInvalid);
    }
    let encoded = authorization
        .strip_prefix("Nostr ")
        .ok_or(HostedSessionBlocker::ProofInvalid)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| HostedSessionBlocker::ProofInvalid)?;
    let event_json = String::from_utf8(bytes).map_err(|_| HostedSessionBlocker::ProofInvalid)?;
    let event = Event::from_json(event_json).map_err(|_| HostedSessionBlocker::ProofInvalid)?;
    event
        .verify()
        .map_err(|_| HostedSessionBlocker::ProofInvalid)?;
    if event.kind.as_u16() != NIP98_KIND
        || !event.content.is_empty()
        || event.pubkey.to_hex() != expected_public_key_hex
    {
        return Err(HostedSessionBlocker::ProofInvalid);
    }
    let created_at = event.created_at.as_secs();
    if created_at.abs_diff(now) > NIP98_MAX_CLOCK_SKEW_SECONDS {
        return Err(HostedSessionBlocker::ProofExpired);
    }
    let expected = [
        ("u", expected_url.to_string()),
        ("method", expected_method.to_uppercase()),
        ("payload", format!("{:x}", Sha256::digest(expected_payload))),
    ];
    if event.tags.len() != expected.len() {
        return Err(HostedSessionBlocker::ProofInvalid);
    }
    for (name, value) in expected {
        let matches = event
            .tags
            .iter()
            .filter(|tag| {
                let values = tag.as_slice();
                values.len() == 2 && values[0] == name && values[1] == value
            })
            .count();
        if matches != 1 {
            return Err(HostedSessionBlocker::ProofInvalid);
        }
    }
    Ok(VerifiedNip98Proof {
        event_id: event.id.to_hex(),
        public_key_hex: event.pubkey.to_hex(),
        created_at,
    })
}

fn valid_nip98_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.as_str() == value
}

fn valid_http_method(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_uppercase()
                || byte.is_ascii_digit()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
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

pub(crate) async fn mint_openagents_nostr_session_for_identity(
    http_client: &Arc<dyn HttpClient>,
    expected_public_key_hex: Option<&str>,
) -> std::result::Result<MintedOpenAgentsSession, HostedSessionBlocker> {
    let channel_name = app_identity::CHANNEL.display_name();
    let identity = ready_account_identity()?;
    if expected_public_key_hex
        .is_some_and(|expected| expected != identity.public_key_hex().as_str())
    {
        return Err(HostedSessionBlocker::ProofSigningFailed {
            reason: "the active Omega identity changed before hosted sign-in".to_string(),
        });
    }
    log::info!(
        "hosted OpenAgents sign-in: signing as {} identity {}",
        channel_name,
        identity.public_key_hex().as_str()
    );

    let authorization = sign_nip98_post(OPENAGENTS_NOSTR_SESSION_URL, &[], &identity).await?;
    // Empty body on purpose: the NIP-98 `payload` tag is the SHA-256 of the
    // empty byte string. Google Frontend (HTTP/1.1) answers 411 Length Required
    // when a POST has a Content-Type and no Content-Length — which is how
    // `AsyncBody::empty()` was being framed — so the length must be stated
    // explicitly as zero. Without it the owner saw "identity is not admitted"
    // for a request that never reached the auth handler.
    let request = Request::builder()
        .method(Method::POST)
        .uri(OPENAGENTS_NOSTR_SESSION_URL)
        .header("authorization", authorization)
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
        let public_code = public_error_code(&body);
        log::error!(
            "hosted OpenAgents sign-in: {OPENAGENTS_NOSTR_SESSION_URL} refused {} identity {} with HTTP {status}: {public_code}",
            channel_name,
            identity.public_key_hex().as_str(),
        );
        let code = status.as_u16();
        return Err(if code == 411 {
            HostedSessionBlocker::RequestFramingRejected
        } else if status.is_client_error() {
            // 401/403 and every other client answer stays terminal. Waiting
            // cannot turn a refused identity into an admitted one, and the
            // backoff below must never be reached from here.
            HostedSessionBlocker::ProofRejected { status: code }
        } else if is_transient_storage_refusal(code, &public_code) {
            HostedSessionBlocker::ServiceStorageUnavailable {
                status: code,
                code: public_code,
            }
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

/// Whether a server-error answer is the hosted storage start-up class.
///
/// Two facts admit it, because the client cannot assume which build is live.
/// The typed code prefix is the honest signal and is preferred. A bare 503 is
/// accepted as well: the deployment that produced omega#160 answers 503 with
/// `omega_nostr_auth_storage_unavailable`, and an older or truncated body must
/// not cost the owner the re-attempt. Every other 5xx (500, 501, …) keeps the
/// generic [`HostedSessionBlocker::ServiceUnavailable`] naming and is not
/// re-attempted inside one sign-in, because it is not evidence of a start-up
/// window.
fn is_transient_storage_refusal(status: u16, public_code: &str) -> bool {
    public_code.starts_with(STORAGE_UNAVAILABLE_CODE_PREFIX) || status == 503
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

    /// The retry-once wrapper recognizes a mid-proof selection bump by the
    /// broker's exact refusal line. If that line drifts, the retry silently
    /// stops firing, which is how one benign generation bump dead-ended the
    /// hosted lane on 2026-07-30.
    #[test]
    fn the_stale_selection_retry_reason_matches_the_broker_refusal() {
        assert_eq!(
            omega_signer_broker::SignerBrokerError::StaleAccountSelection.to_string(),
            STALE_SELECTION_REASON
        );
        // The registry's other refusals must NOT be narrated as a selection
        // change: they carry their own reason and are not retried as a race.
        let not_signable = omega_signer_broker::SignerBrokerError::AccountNotSignable {
            reason: "the account is not available for switching".to_string(),
        };
        assert_ne!(not_signable.to_string(), STALE_SELECTION_REASON);
        assert!(not_signable.to_string().contains("cannot sign right now"));
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
            HostedSessionBlocker::IdentityMissing,
            HostedSessionBlocker::IdentityAdoptionRequired,
            HostedSessionBlocker::IdentityUnavailable {
                custody_state: "state".to_string(),
            },
            HostedSessionBlocker::ProofSigningFailed {
                reason: "reason".to_string(),
            },
            HostedSessionBlocker::ProofInvalid,
            HostedSessionBlocker::ProofExpired,
            HostedSessionBlocker::ProofAlreadyUsed,
            HostedSessionBlocker::ServiceUnreachable,
            HostedSessionBlocker::ChallengeRateLimited,
            HostedSessionBlocker::ProofRejected { status: 401 },
            HostedSessionBlocker::VoiceProofExpired,
            HostedSessionBlocker::RequestFramingRejected,
            HostedSessionBlocker::ServiceUnavailable { status: 503 },
            HostedSessionBlocker::ServiceStorageUnavailable {
                status: 503,
                code: "omega_nostr_auth_storage_timeout".to_string(),
            },
            HostedSessionBlocker::VoiceSessionRejected { status: 402 },
            HostedSessionBlocker::InsufficientVoiceCredits { status: 402 },
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

    #[test]
    fn automatic_sign_in_never_creates_or_adopts_an_identity() {
        assert_eq!(
            identity_blocker(CustodyState::Absent),
            Some(HostedSessionBlocker::IdentityMissing)
        );
        assert_eq!(
            identity_blocker(CustodyState::Unadopted),
            Some(HostedSessionBlocker::IdentityAdoptionRequired)
        );
        assert_eq!(identity_blocker(CustodyState::Ready), None);
        assert!(matches!(
            identity_blocker(CustodyState::Conflict),
            Some(HostedSessionBlocker::IdentityUnavailable { custody_state })
                if custody_state == "Conflict"
        ));
    }

    #[test]
    fn nip98_post_has_only_the_exact_url_method_and_payload_tags() {
        let payload_hash = format!("{:x}", Sha256::digest(b"exact request bytes"));
        let event = nip98_event(
            "https://openagents.com/api/omega/sarah/voice/session",
            "POST",
            Some(payload_hash.clone()),
            1_700_000_000,
        );
        assert_eq!(event.kind, 27_235);
        assert_eq!(event.created_at, 1_700_000_000);
        assert!(event.content.is_empty());
        assert_eq!(
            event.tags,
            vec![
                vec![
                    "u".to_string(),
                    "https://openagents.com/api/omega/sarah/voice/session".to_string()
                ],
                vec!["method".to_string(), "POST".to_string()],
                vec!["payload".to_string(), payload_hash],
            ]
        );
    }

    #[test]
    fn nip98_get_without_a_body_omits_the_payload_tag() {
        let event = nip98_event(
            "https://api.openagents.com/v1/responses",
            "GET",
            None,
            1_700_000_000,
        );
        assert_eq!(
            event.tags,
            vec![
                vec![
                    "u".to_string(),
                    "https://api.openagents.com/v1/responses".to_string()
                ],
                vec!["method".to_string(), "GET".to_string()],
            ]
        );
    }

    #[test]
    fn nip98_verifier_binds_exact_request_freshness_signature_and_one_use() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let keys = Keys::generate();
        let url = "https://openagents.com/api/omega/auth/session";
        let payload = b"exact bytes";
        let payload_hash = format!("{:x}", Sha256::digest(payload));
        let created_at = 1_700_000_000;
        let event = EventBuilder::new(Kind::from(NIP98_KIND), "")
            .tags(vec![
                Tag::parse(["u", url]).expect("url tag"),
                Tag::parse(["method", "POST"]).expect("method tag"),
                Tag::parse(["payload", &payload_hash]).expect("payload tag"),
            ])
            .custom_created_at(Timestamp::from_secs(created_at))
            .sign_with_keys(&keys)
            .expect("signed NIP-98 event");
        let authorization = format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(event.as_json())
        );
        let mut guard = Nip98ProofReplayGuard::default();
        let proof = guard
            .verify_and_consume(
                &authorization,
                url,
                "POST",
                payload,
                &keys.public_key().to_hex(),
                created_at + 10,
            )
            .expect("valid proof");
        assert_eq!(proof.event_id, event.id.to_hex());
        assert_eq!(
            guard.verify_and_consume(
                &authorization,
                url,
                "POST",
                payload,
                &keys.public_key().to_hex(),
                created_at + 10,
            ),
            Err(HostedSessionBlocker::ProofAlreadyUsed)
        );
        assert_eq!(
            verify_nip98_authorization(
                &authorization,
                url,
                "POST",
                b"wrong bytes",
                &keys.public_key().to_hex(),
                created_at + 10,
            ),
            Err(HostedSessionBlocker::ProofInvalid)
        );
        assert_eq!(
            verify_nip98_authorization(
                &authorization,
                url,
                "POST",
                payload,
                &keys.public_key().to_hex(),
                created_at + 61,
            ),
            Err(HostedSessionBlocker::ProofExpired)
        );
    }

    #[test]
    fn nip98_issuance_is_monotonic_for_same_account_generation_and_key() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let issuer = format!("fixture-{}", rand::random::<u64>());
        let first = next_nip98_created_at(&issuer, 1_700_000_000).expect("first issuance");
        let second = next_nip98_created_at(&issuer, 1_700_000_000).expect("second issuance");
        assert_eq!(first, 1_700_000_000);
        assert_eq!(second, first + 1);
        let keys = Keys::generate();
        let make_event = |created_at| {
            EventBuilder::new(Kind::from(NIP98_KIND), "")
                .tags(vec![
                    Tag::parse(["u", OPENAGENTS_NOSTR_SESSION_URL]).expect("url tag"),
                    Tag::parse(["method", "POST"]).expect("method tag"),
                    Tag::parse([
                        "payload",
                        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                    ])
                    .expect("payload tag"),
                ])
                .custom_created_at(Timestamp::from_secs(created_at))
                .sign_with_keys(&keys)
                .expect("signed event")
        };
        assert_ne!(make_event(first).id, make_event(second).id);
    }

    #[test]
    fn nip98_request_target_and_method_are_strict() {
        assert!(valid_nip98_url(
            "https://openagents.com/api/omega/auth/session"
        ));
        assert!(!valid_nip98_url("not a URL"));
        assert!(!valid_nip98_url("wss://relay.example"));
        assert!(!valid_nip98_url(
            "https://user:secret@openagents.com/session"
        ));
        assert!(!valid_nip98_url("https://openagents.com/session#fragment"));
        assert!(valid_http_method("POST"));
        assert!(valid_http_method("CUSTOM-METHOD"));
        assert!(!valid_http_method("post"));
        assert!(!valid_http_method("POST request"));
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

    /// omega#160. The hosted route's storage window must be recognized from
    /// the server's own code, across the three sibling codes it can send and
    /// the older build that only sends the first one.
    #[test]
    fn the_hosted_storage_start_up_window_is_recognized_from_the_server_code() {
        for code in [
            "omega_nostr_auth_storage_unavailable",
            "omega_nostr_auth_storage_timeout",
            "omega_nostr_auth_storage_unreachable",
        ] {
            assert!(
                is_transient_storage_refusal(503, code),
                "`{code}` is the hosted storage start-up class"
            );
            // The typed code is enough on its own: a future revision that
            // answers 500 or 429-shaped 5xx for the same cause is still the
            // same window.
            assert!(is_transient_storage_refusal(500, code));
        }
        // A build that answers a bare 503 with no decodable body still gets
        // the re-attempt: the owner's binary talks to whichever revision is
        // live, and 503 is what the failing deployment returns.
        assert!(is_transient_storage_refusal(503, "no public error code"));
        // Every other server error keeps its generic naming.
        assert!(!is_transient_storage_refusal(500, "internal_error"));
        assert!(!is_transient_storage_refusal(502, "no public error code"));
    }

    /// The whole point of the new variant: it is retryable, it is the transient
    /// start-up class, and the terminal answers are neither. A regression that
    /// widened `is_transient_service_window` to a refused proof would put the
    /// owner into a silent 30-second wait for an answer that cannot change.
    #[test]
    fn only_the_service_start_up_class_is_re_attempted_inside_one_sign_in() {
        let window = HostedSessionBlocker::ServiceStorageUnavailable {
            status: 503,
            code: "omega_nostr_auth_storage_timeout".to_string(),
        };
        assert!(window.is_retryable());
        assert!(window.is_transient_service_window());
        assert!(window.summary().contains("503"));
        assert!(
            window
                .summary()
                .contains("omega_nostr_auth_storage_timeout")
        );
        assert!(HostedSessionBlocker::ServiceUnreachable.is_transient_service_window());
        assert!(
            HostedSessionBlocker::ServiceUnavailable { status: 504 }.is_transient_service_window()
        );

        for terminal in [
            HostedSessionBlocker::ProofRejected { status: 401 },
            HostedSessionBlocker::ProofRejected { status: 403 },
            HostedSessionBlocker::RequestFramingRejected,
            HostedSessionBlocker::IdentityMissing,
            HostedSessionBlocker::IdentityAdoptionRequired,
            HostedSessionBlocker::IdentityUnavailable {
                custody_state: "Lost".to_string(),
            },
            HostedSessionBlocker::ProofInvalid,
            HostedSessionBlocker::ResponseInvalid,
            // Retryable by a person, but never by waiting a few seconds inside
            // one call.
            HostedSessionBlocker::ChallengeRateLimited,
            HostedSessionBlocker::ProofAlreadyUsed,
            HostedSessionBlocker::CredentialStorageFailed,
            HostedSessionBlocker::SessionNotVerified,
            HostedSessionBlocker::ServiceUnavailable { status: 500 },
        ] {
            assert!(
                !terminal.is_transient_service_window(),
                "{terminal:?} must not be re-attempted on a backoff"
            );
        }
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
