use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow};
use base64::Engine as _;
use credentials_provider::CredentialsProvider;
use gpui::{App, AsyncApp, Global};
use http_client::{AsyncBody, HttpClient, Method, Request, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use smol::io::AsyncReadExt as _;

use super::openagents_nostr_auth::HostedSessionBlocker;

pub const OPENAGENTS_BASE_URL: &str = "https://openagents.com";
pub const OPENAGENTS_AUTH_SESSION_URL: &str = "https://openagents.com/api/mobile/auth/session";
pub const OPENAGENTS_CONTROLLER_TOKEN_URL: &str =
    "https://pro.openagents.com/api/mobile/controller/token";
pub const OPENAGENTS_SESSION_KEY: &str = "omega://openagents/native-session/v1";
pub const OPENAGENTS_DEVICE_KEY: &str = "omega://openagents/device-binding/v1";

const OPENAGENTS_REFRESH_HEADER: &str = "x-openagents-refresh-token";
const OPENAGENTS_CREDENTIAL_USERNAME: &str = "omega";
const MAX_HTTP_BODY_BYTES: u64 = 64 * 1024;
const MAX_ACCESS_TOKEN_BYTES: usize = 16 * 1024;
const COMMUNITY_SARAH_PATH_PREFIX: &str = "/api/sarah/livekit/room/";

/// How many times one `connect` may attempt the hosted sign-in.
///
/// omega#160, and every constant below is chosen against that measurement, not
/// against a round number. Each new Cloud Run revision of the hosted service
/// takes traffic before its Cloud SQL Auth Proxy sidecar has connected. The
/// observed window is 10-70 seconds, and it flaps: the same identity gets 200,
/// then 503, then 200 within seconds. One retry three seconds later lands
/// inside that window far too often, and the owner sees a red banner for a
/// dependency that was about to come back on its own.
///
/// Five attempts with the delays below spend about 15 seconds of waiting, which
/// covers the common short end of the window. It deliberately does not try to
/// cover 70 seconds: past this point the honest answer is the blocker, not a
/// longer silent spinner.
const HOSTED_RETRY_MAX_ATTEMPTS: u8 = 5;

/// The first backoff delay. Doubles per attempt: 1s, 2s, 4s, 8s.
const HOSTED_RETRY_INITIAL_DELAY: Duration = Duration::from_secs(1);

/// The ceiling one delay may reach, so the last waits stay legible rather than
/// growing past the window they are covering.
const HOSTED_RETRY_MAX_DELAY: Duration = Duration::from_secs(8);

/// The wall-clock deadline for *starting* another attempt, measured from the
/// first one.
///
/// This is not a sleep budget. A failing request against a cold instance burns
/// about 30 seconds server-side before its 503 arrives, so counting only the
/// delays would let five attempts run for two and a half minutes. The budget is
/// checked against the elapsed time of the whole sign-in, including the slow
/// requests, and no new attempt starts once it is spent. The worst case is
/// therefore this budget plus one in-flight request, not an unbounded chain:
/// with fast typed 503s the full five attempts fit inside it, and with 30-second
/// failures the loop stops after the second attempt.
const HOSTED_RETRY_BUDGET: Duration = Duration::from_secs(30);

/// Extra fraction of a delay added at random, to keep several Omega instances
/// that saw the same deploy from re-attempting in lockstep. Additive only: a
/// jittered delay is never shorter than the scheduled one.
const HOSTED_RETRY_JITTER_FRACTION: f64 = 0.25;

/// When Omega re-attempts a hosted sign-in that failed inside the hosted
/// service's own start-up window.
///
/// It is a value rather than a set of constants read at the call site so a test
/// can inject a schedule that does not sleep. A suite that waited out the
/// production backoff would be slow and, worse, would make a hermetic proof
/// harness depend on a real clock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostedRetrySchedule {
    pub max_attempts: u8,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub budget: Duration,
    pub jitter: bool,
}

impl Default for HostedRetrySchedule {
    fn default() -> Self {
        Self::production()
    }
}

impl HostedRetrySchedule {
    /// The schedule the shipped binary uses.
    pub const fn production() -> Self {
        Self {
            max_attempts: HOSTED_RETRY_MAX_ATTEMPTS,
            initial_delay: HOSTED_RETRY_INITIAL_DELAY,
            max_delay: HOSTED_RETRY_MAX_DELAY,
            budget: HOSTED_RETRY_BUDGET,
            jitter: true,
        }
    }

    /// The same attempt count with no waiting and no randomness, for tests.
    pub const fn instant() -> Self {
        Self {
            max_attempts: HOSTED_RETRY_MAX_ATTEMPTS,
            initial_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
            budget: Duration::from_secs(u64::MAX / 2),
            jitter: false,
        }
    }

    /// One attempt and no re-attempt, for deterministic harnesses that must
    /// observe the first answer exactly as the service gave it.
    pub const fn single_attempt() -> Self {
        Self {
            max_attempts: 1,
            ..Self::instant()
        }
    }

    /// The scheduled delay after `attempt` (1-based) failed, or `None` when the
    /// schedule is exhausted by attempt count or by the elapsed budget.
    pub fn delay_after(&self, attempt: u8, elapsed: Duration) -> Option<Duration> {
        if attempt >= self.max_attempts {
            return None;
        }
        let mut delay = self.initial_delay;
        for _ in 1..attempt {
            delay = delay.saturating_mul(2).min(self.max_delay);
        }
        let delay = delay.min(self.max_delay);
        (elapsed.saturating_add(delay) < self.budget).then_some(delay)
    }

    fn with_jitter(&self, delay: Duration) -> Duration {
        if !self.jitter || delay.is_zero() {
            return delay;
        }
        delay.mul_f64(1.0 + HOSTED_RETRY_JITTER_FRACTION * rand::random::<f64>())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAgentsSessionPhase {
    #[default]
    SignedOut,
    Connecting,
    Ready,
    Denied,
    Unavailable,
    Disconnecting,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostedSessionState {
    #[default]
    Disconnected,
    Connecting,
    Verified,
    Rotating,
    Expired,
    Revoked,
    OwnerScopeRefused,
    AccountMismatch,
    ServiceUnavailable,
    StorageFailed,
    RevocationFailed,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedSessionProjection {
    pub state: HostedSessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omega_public_key_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    pub retryable: bool,
}

impl OpenAgentsSessionPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::SignedOut => "Not connected",
            Self::Connecting => "Connecting securely…",
            Self::Ready => "Connected",
            Self::Denied => "Session expired — reconnect required",
            Self::Unavailable => "OpenAgents account unavailable",
            Self::Disconnecting => "Disconnecting…",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedOpenAgentsSession {
    pub base_url: String,
    pub owner_user_id: String,
    pub access_token: String,
}

/// Public, non-secret metadata returned with a short-lived Convex read token.
///
/// The token itself is intentionally not serializable or debuggable. The
/// official Convex client consumes it inside its authentication callback and
/// requests a fresh one after every WebSocket reconnect.
#[derive(Clone)]
pub struct OpenAgentsControllerBootstrap {
    pub convex_url: String,
    pub token: String,
    pub owner_user_id: String,
    pub workspace_id: String,
}

/// Opaque bearer-backed source for short-lived native controller tokens.
///
/// Omega's GPUI surface receives this source, not the stored OpenAgents
/// credential. Fetches are bounded to the one fixed Pro endpoint and never log
/// either bearer. The source is deliberately process-local; the canonical
/// credential remains owned and rotated by [`OpenAgentsSession`].
#[derive(Clone)]
pub struct OpenAgentsControllerTokenSource {
    http_client: Arc<dyn HttpClient>,
    access_token: Arc<Mutex<String>>,
    owner_user_id: String,
    endpoint: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ControllerTokenResponse {
    version: String,
    token: String,
    convex_url: String,
    actor: ControllerActor,
    workspace: ControllerWorkspace,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ControllerActor {
    user_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ControllerWorkspace {
    workspace_id: String,
}

#[derive(Clone)]
pub struct OpenAgentsSession {
    credentials: Arc<dyn CredentialsProvider>,
    http_client: Arc<dyn HttpClient>,
    phase: Arc<Mutex<OpenAgentsSessionPhase>>,
    /// The last reason a connection attempt did not produce a session.
    ///
    /// `OpenAgentsSessionPhase` is deliberately coarse — it drives a status
    /// label. Callers that have to tell a person what to do next need the
    /// specific reason, and reconstructing it from `Unavailable` is impossible.
    blocker: Arc<Mutex<Option<HostedSessionBlocker>>>,
    projection: Arc<Mutex<HostedSessionProjection>>,
    retry: HostedRetrySchedule,
}

struct OpenAgentsSessionGlobal(OpenAgentsSession);
impl Global for OpenAgentsSessionGlobal {}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredCredential {
    schema_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_user_id: Option<String>,
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    issued_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    omega_public_key_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account_generation: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredDeviceBinding {
    schema_version: u8,
    device_ref: String,
}

#[derive(Deserialize)]
struct VerifiedSessionResponse {
    authenticated: bool,
    user: VerifiedSessionUser,
    tokens: Option<RotatedTokens>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerifiedSessionUser {
    user_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RotatedTokens {
    access: String,
    refresh: String,
    expires_in: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RevokedSessionResponse {
    signed_out: bool,
    access_revoked: bool,
    refresh_revoked: bool,
}

#[derive(Deserialize)]
struct CommunitySarahErrorResponse {
    error: String,
}

enum VerificationResult {
    Verified {
        credential: StoredCredential,
        owner_user_id: String,
    },
    Denied,
    Unavailable,
}

pub fn init_openagents_session(cx: &mut App) {
    if cx.try_global::<OpenAgentsSessionGlobal>().is_some() {
        return;
    }
    cx.set_global(OpenAgentsSessionGlobal(OpenAgentsSession {
        credentials: zed_credentials_provider::local_credentials(cx),
        http_client: cx.http_client(),
        phase: Arc::new(Mutex::new(OpenAgentsSessionPhase::SignedOut)),
        blocker: Arc::default(),
        projection: Arc::default(),
        retry: HostedRetrySchedule::production(),
    }));
}

pub fn openagents_session(cx: &App) -> OpenAgentsSession {
    cx.global::<OpenAgentsSessionGlobal>().0.clone()
}

/// The hosted session, or `None` in a process that never initialized one.
///
/// omega#161. The shipped binary always calls [`init_openagents_session`] at
/// startup, so production reads `Some`. Proof harnesses and tests deliberately
/// do not: for them hosted OpenAgents compute is structurally unavailable, and
/// a provider that read the panicking accessor would turn that absence into a
/// crash or, worse, a live network sign-in from inside a deterministic proof.
///
/// It is also why the backoff added for omega#160 cannot make a proof harness
/// slow: a harness that never initializes the global never reaches `connect`,
/// and one that does inject a session uses
/// [`OpenAgentsSession::with_retry_schedule`] with a schedule that does not
/// sleep.
pub fn openagents_session_if_initialized(cx: &App) -> Option<OpenAgentsSession> {
    cx.try_global::<OpenAgentsSessionGlobal>()
        .map(|global| global.0.clone())
}

impl OpenAgentsSession {
    /// The same session with a different re-attempt schedule.
    ///
    /// Tests and deterministic harnesses pass [`HostedRetrySchedule::instant`]
    /// or [`HostedRetrySchedule::single_attempt`], so no suite ever waits out a
    /// production backoff.
    pub fn with_retry_schedule(&self, retry: HostedRetrySchedule) -> Self {
        Self {
            retry,
            ..self.clone()
        }
    }

    pub fn retry_schedule(&self) -> HostedRetrySchedule {
        self.retry
    }

    pub fn phase(&self) -> OpenAgentsSessionPhase {
        self.phase.lock().map(|phase| *phase).unwrap_or_default()
    }

    /// The reason the most recent connection attempt did not produce a session.
    pub fn blocker(&self) -> Option<HostedSessionBlocker> {
        self.blocker.lock().ok().and_then(|blocker| blocker.clone())
    }

    pub fn projection(&self) -> HostedSessionProjection {
        self.projection
            .lock()
            .map(|projection| projection.clone())
            .unwrap_or_default()
    }

    fn set_projection(&self, projection: HostedSessionProjection) {
        if let Ok(mut current) = self.projection.lock() {
            *current = projection;
        }
    }

    fn set_phase(&self, phase: OpenAgentsSessionPhase) {
        if let Ok(mut current) = self.phase.lock() {
            *current = phase;
        }
    }

    fn record_blocker(&self, blocker: Option<HostedSessionBlocker>) {
        if let Some(blocker) = &blocker {
            log::error!(
                "hosted OpenAgents session unavailable: {}",
                blocker.summary()
            );
        }
        if let Ok(mut current) = self.blocker.lock() {
            *current = blocker;
        }
    }

    pub async fn connect(&self, cx: &mut AsyncApp) -> OpenAgentsSessionPhase {
        self.set_phase(OpenAgentsSessionPhase::Connecting);
        self.set_projection(HostedSessionProjection {
            state: HostedSessionState::Connecting,
            retryable: true,
            ..HostedSessionProjection::default()
        });
        let (phase, blocker) = match self.connect_across_service_start_up(cx).await {
            Ok(credential) => {
                if let Err(error) = self.save_credential(&credential, cx).await {
                    log::error!("hosted OpenAgents session could not be stored: {error:#}");
                    (
                        OpenAgentsSessionPhase::Unavailable,
                        Some(HostedSessionBlocker::CredentialStorageFailed),
                    )
                } else {
                    self.set_projection(projection_for_credential(
                        HostedSessionState::Verified,
                        &credential,
                    ));
                    (OpenAgentsSessionPhase::Ready, None)
                }
            }
            Err(blocker) => (OpenAgentsSessionPhase::Unavailable, Some(blocker)),
        };
        self.record_blocker(blocker);
        if phase == OpenAgentsSessionPhase::Unavailable {
            let blocker = self.blocker();
            self.set_projection(HostedSessionProjection {
                state: if blocker.as_ref().is_some_and(|value| {
                    matches!(value, HostedSessionBlocker::CredentialStorageFailed)
                }) {
                    HostedSessionState::StorageFailed
                } else if blocker.as_ref().is_some_and(|value| {
                    matches!(value, HostedSessionBlocker::ProofRejected { .. })
                }) {
                    HostedSessionState::OwnerScopeRefused
                } else {
                    HostedSessionState::ServiceUnavailable
                },
                retryable: blocker
                    .as_ref()
                    .is_none_or(HostedSessionBlocker::is_retryable),
                ..HostedSessionProjection::default()
            });
        }
        self.set_phase(phase);
        phase
    }

    pub async fn disconnect(&self, cx: &mut AsyncApp) -> OpenAgentsSessionPhase {
        self.set_phase(OpenAgentsSessionPhase::Disconnecting);
        let (phase, state) = match self.load_credential(cx).await {
            Ok(None) => (
                OpenAgentsSessionPhase::SignedOut,
                HostedSessionState::Disconnected,
            ),
            Ok(Some(credential)) => {
                let active =
                    omega_identity::AccountRegistryService::for_channel(*app_identity::CHANNEL)
                        .selection_token();
                if !active.as_ref().is_ok_and(|selection| {
                    credential_matches_account(
                        &credential,
                        selection.identity.public_key_hex().as_str(),
                        selection.generation,
                    )
                }) {
                    (
                        OpenAgentsSessionPhase::Denied,
                        HostedSessionState::AccountMismatch,
                    )
                } else {
                    match self.revoke_credential(&credential).await {
                        Ok(true) if self.delete_credential(cx).await.is_ok() => (
                            OpenAgentsSessionPhase::SignedOut,
                            HostedSessionState::Revoked,
                        ),
                        Ok(true) => (
                            OpenAgentsSessionPhase::Unavailable,
                            HostedSessionState::StorageFailed,
                        ),
                        Ok(false) | Err(_) => (
                            OpenAgentsSessionPhase::Unavailable,
                            HostedSessionState::RevocationFailed,
                        ),
                    }
                }
            }
            Err(_) => (
                OpenAgentsSessionPhase::Unavailable,
                HostedSessionState::StorageFailed,
            ),
        };
        self.set_projection(HostedSessionProjection {
            state,
            retryable: matches!(
                state,
                HostedSessionState::StorageFailed | HostedSessionState::RevocationFailed
            ),
            ..HostedSessionProjection::default()
        });
        self.set_phase(phase);
        phase
    }

    pub async fn verify_public(&self, cx: &mut AsyncApp) -> HostedSessionProjection {
        self.resolve_verified(cx).await;
        self.projection()
    }

    pub async fn resolve_verified(&self, cx: &mut AsyncApp) -> Option<VerifiedOpenAgentsSession> {
        let credential = match self.load_credential(cx).await {
            Ok(Some(credential)) => credential,
            Ok(None) => {
                self.set_phase(OpenAgentsSessionPhase::SignedOut);
                self.set_projection(HostedSessionProjection::default());
                return None;
            }
            Err(error) => {
                log::error!("hosted OpenAgents credential could not be read: {error:#}");
                self.record_blocker(Some(HostedSessionBlocker::CredentialStorageFailed));
                self.set_phase(OpenAgentsSessionPhase::Unavailable);
                self.set_projection(HostedSessionProjection {
                    state: HostedSessionState::StorageFailed,
                    retryable: true,
                    ..HostedSessionProjection::default()
                });
                return None;
            }
        };
        let active_selection =
            omega_identity::AccountRegistryService::for_channel(*app_identity::CHANNEL)
                .selection_token();
        let matches_active = active_selection.as_ref().is_ok_and(|selection| {
            credential_matches_account(
                &credential,
                selection.identity.public_key_hex().as_str(),
                selection.generation,
            )
        });
        if !matches_active {
            self.set_phase(OpenAgentsSessionPhase::Denied);
            self.set_projection(projection_for_credential(
                HostedSessionState::AccountMismatch,
                &credential,
            ));
            return None;
        }
        if credential
            .expires_at
            .is_some_and(|expires_at| expires_at <= unix_time_now())
            && credential.refresh_token.is_none()
        {
            self.set_phase(OpenAgentsSessionPhase::Denied);
            self.set_projection(projection_for_credential(
                HostedSessionState::Expired,
                &credential,
            ));
            return None;
        }
        if credential.refresh_token.is_some() {
            self.set_projection(projection_for_credential(
                HostedSessionState::Rotating,
                &credential,
            ));
        }
        match self.verify_credential(&credential).await {
            VerificationResult::Verified {
                credential,
                owner_user_id,
            } => {
                if credential.access_token.len() > MAX_ACCESS_TOKEN_BYTES
                    || self.save_credential(&credential, cx).await.is_err()
                {
                    self.record_blocker(Some(HostedSessionBlocker::CredentialStorageFailed));
                    self.set_phase(OpenAgentsSessionPhase::Unavailable);
                    return None;
                }
                self.record_blocker(None);
                self.set_phase(OpenAgentsSessionPhase::Ready);
                self.set_projection(projection_for_credential(
                    HostedSessionState::Verified,
                    &credential,
                ));
                Some(VerifiedOpenAgentsSession {
                    base_url: OPENAGENTS_BASE_URL.to_string(),
                    owner_user_id,
                    access_token: credential.access_token,
                })
            }
            VerificationResult::Denied => {
                let phase = if self.delete_credential(cx).await.is_ok() {
                    OpenAgentsSessionPhase::Denied
                } else {
                    OpenAgentsSessionPhase::Unavailable
                };
                self.set_phase(phase);
                self.set_projection(HostedSessionProjection {
                    state: HostedSessionState::Revoked,
                    retryable: false,
                    ..HostedSessionProjection::default()
                });
                None
            }
            VerificationResult::Unavailable => {
                self.record_blocker(Some(HostedSessionBlocker::SessionNotVerified));
                self.set_phase(OpenAgentsSessionPhase::Unavailable);
                self.set_projection(HostedSessionProjection {
                    state: HostedSessionState::ServiceUnavailable,
                    retryable: true,
                    ..HostedSessionProjection::default()
                });
                None
            }
        }
    }

    /// Resolve the canonical hosted session and narrow it to the one Pro
    /// controller-token operation needed by the native Convex client.
    pub async fn controller_token_source(
        &self,
        cx: &mut AsyncApp,
    ) -> Result<OpenAgentsControllerTokenSource> {
        let verified = self
            .resolve_verified(cx)
            .await
            .ok_or_else(|| anyhow!("a verified OpenAgents session is required"))?;
        Ok(OpenAgentsControllerTokenSource {
            http_client: self.http_client.clone(),
            access_token: Arc::new(Mutex::new(verified.access_token)),
            owner_user_id: verified.owner_user_id,
            endpoint: OPENAGENTS_CONTROLLER_TOKEN_URL.to_string(),
        })
    }

    pub async fn community_sarah_request<Response>(
        &self,
        path: &str,
        body: &Value,
        cx: &mut AsyncApp,
    ) -> Result<Response>
    where
        Response: DeserializeOwned,
    {
        if !matches!(
            path,
            "/api/sarah/livekit/room/join"
                | "/api/sarah/livekit/room/summon"
                | "/api/sarah/livekit/room/remove"
                | "/api/sarah/livekit/room/floor/member"
                | "/api/sarah/livekit/room/floor/moderator"
        ) || !path.starts_with(COMMUNITY_SARAH_PATH_PREFIX)
        {
            return Err(anyhow!("invalid community Sarah API path"));
        }
        let verified = self
            .resolve_verified(cx)
            .await
            .ok_or_else(|| anyhow!("a verified OpenAgents session is required"))?;
        let endpoint = format!("{}{}", verified.base_url, path);
        let encoded = serde_json::to_vec(body).context("encoding community Sarah request")?;
        if encoded.len() > 4_096 {
            return Err(anyhow!("community Sarah request was too large"));
        }
        let request = Request::builder()
            .method(Method::POST)
            .uri(endpoint)
            .header("authorization", format!("Bearer {}", verified.access_token))
            .header("content-type", "application/json")
            .body(AsyncBody::from(encoded))?;
        let (status, response_body) = send_json(&self.http_client, request).await?;
        if !status.is_success() {
            let error = serde_json::from_slice::<CommunitySarahErrorResponse>(&response_body)
                .ok()
                .map(|response| response.error)
                .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
            return Err(anyhow!("community Sarah request refused: {error}"));
        }
        serde_json::from_slice(&response_body).context("decoding community Sarah response")
    }

    pub async fn create_sarah_voice_session(
        &self,
        thread_ref: &str,
        cx: &mut AsyncApp,
    ) -> std::result::Result<
        super::openagents_sarah_voice::ManagedSarahVoiceSession,
        HostedSessionBlocker,
    > {
        let admission = self.prepare_sarah_voice_admission(thread_ref, cx).await?;
        self.create_sarah_voice_session_from_admission(&admission, cx)
            .await
    }

    pub async fn prepare_sarah_voice_admission(
        &self,
        thread_ref: &str,
        cx: &mut AsyncApp,
    ) -> std::result::Result<
        super::openagents_sarah_voice::PreparedSarahVoiceAdmission,
        HostedSessionBlocker,
    > {
        let session_ref = format!("omega-voice-{}", random_device_component());
        let device_ref = self.load_or_create_device_ref(cx).await.map_err(|error| {
            log::error!("OpenAgents device binding could not be stored: {error:#}");
            HostedSessionBlocker::CredentialStorageFailed
        })?;
        let admission = if let Some(verified) = self.resolve_verified(cx).await {
            super::openagents_sarah_voice::prepare_bearer_sarah_voice_admission(
                &self.http_client,
                &verified.access_token,
                &verified.owner_user_id,
                &device_ref,
                thread_ref,
                &session_ref,
            )
            .await
        } else {
            let issued = super::openagents_sarah_voice::prepare_nostr_sarah_voice_admission(
                &self.http_client,
                &device_ref,
                thread_ref,
                &session_ref,
            )
            .await;
            match issued {
                Ok(issued) => {
                    let selection =
                        omega_identity::AccountRegistryService::for_channel(*app_identity::CHANNEL)
                            .selection_token()
                            .ok();
                    let credential = StoredCredential {
                        schema_version: 2,
                        owner_user_id: None,
                        access_token: issued.access_token,
                        refresh_token: None,
                        issued_at: None,
                        expires_at: None,
                        omega_public_key_hex: selection.as_ref().map(|selection| {
                            selection.identity.public_key_hex().as_str().to_string()
                        }),
                        account_generation: selection.map(|selection| selection.generation),
                    };
                    match self.save_credential(&credential, cx).await {
                        Ok(()) => {
                            self.set_phase(OpenAgentsSessionPhase::Ready);
                            Ok(issued.admission)
                        }
                        Err(error) => {
                            log::error!(
                                "Nostr-issued Sarah voice credential could not be stored: {error:#}"
                            );
                            Err(HostedSessionBlocker::CredentialStorageFailed)
                        }
                    }
                }
                Err(blocker) => Err(blocker),
            }
        };
        match admission {
            Ok(admission) => {
                self.record_blocker(None);
                Ok(admission)
            }
            Err(blocker) => {
                self.record_blocker(Some(blocker.clone()));
                Err(blocker)
            }
        }
    }

    pub async fn create_sarah_voice_session_from_admission(
        &self,
        admission: &super::openagents_sarah_voice::PreparedSarahVoiceAdmission,
        cx: &mut AsyncApp,
    ) -> std::result::Result<
        super::openagents_sarah_voice::ManagedSarahVoiceSession,
        HostedSessionBlocker,
    > {
        let verified = self
            .resolve_verified(cx)
            .await
            .ok_or(HostedSessionBlocker::SessionNotVerified)?;
        if verified.owner_user_id != admission.owner_ref {
            return Err(HostedSessionBlocker::ResponseInvalid);
        }
        let issued = match super::openagents_sarah_voice::issue_bearer_sarah_voice_session(
            &self.http_client,
            &verified.access_token,
            admission,
        )
        .await
        {
            Ok(issued) => issued,
            Err(blocker) => {
                self.record_blocker(Some(blocker.clone()));
                return Err(blocker);
            }
        };
        let voice = issued.voice;
        if voice.session_ref != admission.session_ref
            || voice.thread_ref != admission.thread_ref
            || voice.reserved_credit_msat != admission.projection.reserved_credit_msat
            || voice.max_duration_seconds != admission.projection.max_duration_seconds
        {
            return Err(HostedSessionBlocker::ResponseInvalid);
        }
        let Some(echoed_admission) = voice.admission.as_ref() else {
            return Err(HostedSessionBlocker::ResponseInvalid);
        };
        if echoed_admission.admission_ref != admission.projection.admission_ref
            || echoed_admission.admission_expires_at_ms
                != admission.projection.admission_expires_at_ms
            || !echoed_admission.has_same_reviewed_terms(&admission.projection)
        {
            return Err(HostedSessionBlocker::ResponseInvalid);
        }
        self.record_blocker(None);
        self.set_phase(OpenAgentsSessionPhase::Ready);
        Ok(voice)
    }

    pub async fn read_sarah_voice_settlement(
        &self,
        thread_ref: &str,
        session_ref: &str,
        cx: &mut AsyncApp,
    ) -> std::result::Result<
        super::openagents_sarah_voice::SarahVoiceSettlementProjection,
        HostedSessionBlocker,
    > {
        let verified = self
            .resolve_verified(cx)
            .await
            .ok_or(HostedSessionBlocker::SessionNotVerified)?;
        super::openagents_sarah_voice::read_bearer_sarah_voice_settlement(
            &self.http_client,
            &verified.access_token,
            thread_ref,
            session_ref,
        )
        .await
    }

    /// Sign in, re-attempting only while the hosted service is inside its own
    /// start-up window.
    ///
    /// omega#160. A fresh Cloud Run revision routes traffic before its Cloud
    /// SQL Auth Proxy sidecar has connected, so a sign-in that is about to work
    /// fails for 10-70 seconds after each deploy. Everything else — a refused
    /// proof, a framing bug, a missing identity, a local storage failure —
    /// returns on the first answer, so no owner waits through a backoff for a
    /// verdict that cannot change.
    ///
    /// Each attempt signs a fresh NIP-98 proof, because
    /// `mint_openagents_nostr_session_for_identity` re-signs per call and the
    /// proofs are one-use inside a 60-second freshness window. Re-sending a
    /// consumed proof would turn a transient outage into `ProofAlreadyUsed`.
    ///
    /// When every attempt fails, the caller gets the real blocker from the last
    /// attempt. Nothing is swallowed: a persistent outage still reaches the
    /// owner, it just reaches them after the flap has had a chance to clear.
    async fn connect_across_service_start_up(
        &self,
        cx: &mut AsyncApp,
    ) -> Result<StoredCredential, HostedSessionBlocker> {
        let started_at = Instant::now();
        let mut attempt: u8 = 1;
        loop {
            let blocker = match self.connect_inner().await {
                Ok(credential) => {
                    if attempt > 1 {
                        log::info!(
                            "hosted OpenAgents sign-in: succeeded on attempt {attempt} after \
                             {}ms in the service's start-up window",
                            started_at.elapsed().as_millis()
                        );
                    }
                    return Ok(credential);
                }
                Err(blocker) => blocker,
            };
            if !blocker.is_transient_service_window() {
                return Err(blocker);
            }
            let Some(delay) = self.retry.delay_after(attempt, started_at.elapsed()) else {
                log::error!(
                    "hosted OpenAgents sign-in: still failing after attempt {attempt} of at \
                     most {} and {}ms; reporting the blocker: {}",
                    self.retry.max_attempts,
                    started_at.elapsed().as_millis(),
                    blocker.summary()
                );
                return Err(blocker);
            };
            let delay = self.retry.with_jitter(delay);
            // A flapping dependency is only diagnosable if each re-attempt is
            // in the log with its number, its wait, and the answer that caused
            // it. The summary is a blocker line: a status and a public error
            // code, never a token, key, or signature.
            log::warn!(
                "hosted OpenAgents sign-in: attempt {attempt} of at most {} hit the hosted \
                 service's start-up window; re-attempting in {}ms: {}",
                self.retry.max_attempts,
                delay.as_millis(),
                blocker.summary()
            );
            let timer = cx.background_executor().timer(delay);
            timer.await;
            attempt = attempt.saturating_add(1);
        }
    }

    async fn connect_inner(&self) -> Result<StoredCredential, HostedSessionBlocker> {
        let selection = omega_identity::AccountRegistryService::for_channel(*app_identity::CHANNEL)
            .selection_token()
            .map_err(|error| HostedSessionBlocker::IdentityUnavailable {
                custody_state: error.to_string(),
            })?;
        let session = super::openagents_nostr_auth::mint_openagents_nostr_session_for_identity(
            &self.http_client,
            Some(selection.identity.public_key_hex().as_str()),
        )
        .await?;
        let issued_at = unix_time_now();
        let expires_at = issued_at
            .checked_add(session.expires_in)
            .ok_or(HostedSessionBlocker::ResponseInvalid)?;
        let credential = StoredCredential {
            schema_version: 2,
            owner_user_id: Some(session.user.user_id),
            access_token: session.access_token.trim().to_string(),
            refresh_token: None,
            issued_at: Some(issued_at),
            expires_at: Some(expires_at),
            omega_public_key_hex: Some(selection.identity.public_key_hex().as_str().to_string()),
            account_generation: Some(selection.generation),
        };
        match self.verify_credential(&credential).await {
            VerificationResult::Verified { credential, .. } => Ok(credential),
            VerificationResult::Denied | VerificationResult::Unavailable => {
                Err(HostedSessionBlocker::SessionNotVerified)
            }
        }
    }

    async fn verify_credential(&self, credential: &StoredCredential) -> VerificationResult {
        let request = match authenticated_session_request(Method::GET, credential) {
            Ok(request) => request,
            Err(error) => {
                log::error!("hosted OpenAgents session check could not be built: {error:#}");
                return VerificationResult::Unavailable;
            }
        };
        let (status, body) = match send_json(&self.http_client, request).await {
            Ok(response) => response,
            Err(error) => {
                log::error!(
                    "hosted OpenAgents session check could not reach \
                     {OPENAGENTS_AUTH_SESSION_URL}: {error:#}"
                );
                return VerificationResult::Unavailable;
            }
        };
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            log::error!("hosted OpenAgents session check was denied (HTTP {status})");
            return VerificationResult::Denied;
        }
        if !status.is_success() {
            log::error!("hosted OpenAgents session check failed (HTTP {status})");
            return VerificationResult::Unavailable;
        }
        let Ok(session) = serde_json::from_slice::<VerifiedSessionResponse>(&body) else {
            log::error!("hosted OpenAgents session check returned an undecodable body");
            return VerificationResult::Unavailable;
        };
        let owner_user_id = session.user.user_id.trim();
        if !session.authenticated
            || owner_user_id.is_empty()
            || credential
                .owner_user_id
                .as_deref()
                .is_some_and(|expected| expected != owner_user_id)
        {
            return VerificationResult::Denied;
        }
        let mut verified = credential.clone();
        if let Some(tokens) = session.tokens {
            if tokens.access.trim().is_empty()
                || tokens.refresh.trim().is_empty()
                || tokens.access.len() > MAX_ACCESS_TOKEN_BYTES
                || !tokens.expires_in.is_finite()
                || tokens.expires_in <= 0.0
                || tokens.expires_in > 31_536_000.0
            {
                return VerificationResult::Unavailable;
            }
            verified.access_token = tokens.access.trim().to_string();
            verified.refresh_token = Some(tokens.refresh.trim().to_string());
            let issued_at = unix_time_now();
            verified.issued_at = Some(issued_at);
            verified.expires_at = issued_at.checked_add(tokens.expires_in as u64);
        }
        VerificationResult::Verified {
            credential: verified,
            owner_user_id: owner_user_id.to_string(),
        }
    }

    async fn revoke_credential(&self, credential: &StoredCredential) -> Result<bool> {
        let request = authenticated_session_request(Method::DELETE, credential)?;
        let (status, body) = send_json(&self.http_client, request).await?;
        if !status.is_success() {
            return Ok(false);
        }
        let proof: RevokedSessionResponse =
            serde_json::from_slice(&body).context("OpenAgents revoke response was invalid")?;
        Ok(proof.signed_out
            && proof.access_revoked
            && (credential.refresh_token.is_none() || proof.refresh_revoked))
    }

    async fn load_credential(&self, cx: &AsyncApp) -> Result<Option<StoredCredential>> {
        let Some((username, secret)) = self
            .credentials
            .read_credentials(OPENAGENTS_SESSION_KEY, cx)
            .await?
        else {
            return Ok(None);
        };
        let mut credential: StoredCredential =
            serde_json::from_slice(&secret).context("OpenAgents stored credential was invalid")?;
        if !matches!(credential.schema_version, 1 | 2)
            || !credential_owner_matches_username(&credential, &username)
            || credential.access_token.trim().is_empty()
            || credential
                .refresh_token
                .as_ref()
                .is_some_and(|token| token.trim().is_empty())
            || credential.access_token.len() > MAX_ACCESS_TOKEN_BYTES
        {
            return Err(anyhow!("OpenAgents stored credential was invalid"));
        }
        credential.schema_version = 2;
        Ok(Some(credential))
    }

    async fn save_credential(&self, credential: &StoredCredential, cx: &AsyncApp) -> Result<()> {
        if credential.schema_version != 2
            || credential
                .omega_public_key_hex
                .as_ref()
                .is_none_or(|public_key| public_key.trim().is_empty())
            || credential.account_generation.is_none()
            || credential
                .owner_user_id
                .as_ref()
                .is_some_and(|owner_user_id| owner_user_id.trim().is_empty())
            || credential.access_token.trim().is_empty()
            || credential
                .refresh_token
                .as_ref()
                .is_some_and(|token| token.trim().is_empty())
        {
            return Err(anyhow!("OpenAgents credential was incomplete"));
        }
        let secret = serde_json::to_vec(credential)?;
        self.credentials
            .write_credentials(
                OPENAGENTS_SESSION_KEY,
                credential_username(credential),
                &secret,
                cx,
            )
            .await
    }

    async fn delete_credential(&self, cx: &AsyncApp) -> Result<()> {
        self.credentials
            .delete_credentials(OPENAGENTS_SESSION_KEY, cx)
            .await
    }

    async fn load_or_create_device_ref(&self, cx: &AsyncApp) -> Result<String> {
        if let Some((username, secret)) = self
            .credentials
            .read_credentials(OPENAGENTS_DEVICE_KEY, cx)
            .await?
        {
            let stored: StoredDeviceBinding =
                serde_json::from_slice(&secret).context("OpenAgents device binding was invalid")?;
            if username != "omega"
                || stored.schema_version != 1
                || !valid_device_ref(&stored.device_ref)
            {
                return Err(anyhow!("OpenAgents device binding was invalid"));
            }
            return Ok(stored.device_ref);
        }
        let device_ref = format!("omega-device-{}", random_device_component());
        let secret = serde_json::to_vec(&StoredDeviceBinding {
            schema_version: 2,
            device_ref: device_ref.clone(),
        })?;
        self.credentials
            .write_credentials(OPENAGENTS_DEVICE_KEY, "omega", &secret, cx)
            .await?;
        Ok(device_ref)
    }
}

impl OpenAgentsControllerTokenSource {
    /// Mint one five-minute read token for the official Convex client.
    ///
    /// This method is safe to call from the client's Tokio authentication
    /// callback, including during a reconnect. It has no GPUI or keychain
    /// dependency and cannot target an arbitrary URL.
    pub async fn fetch(&self) -> Result<OpenAgentsControllerBootstrap> {
        let access_token = self
            .access_token
            .lock()
            .map_err(|_| anyhow!("OpenAgents controller credential lock was poisoned"))?
            .clone();
        let request = Request::builder()
            .method(Method::GET)
            .uri(&self.endpoint)
            .header("authorization", format!("Bearer {access_token}"))
            .body(AsyncBody::empty())?;
        let (status, body) = send_json(&self.http_client, request).await?;
        if !status.is_success() {
            return Err(anyhow!(
                "OpenAgents controller token request was refused (HTTP {})",
                status.as_u16()
            ));
        }
        let response: ControllerTokenResponse = serde_json::from_slice(&body)
            .context("OpenAgents controller token response was invalid")?;
        if response.version != "openagents.mobile_controller.v1"
            || response.token.trim().is_empty()
            || response.token.len() > MAX_ACCESS_TOKEN_BYTES
            || response.actor.user_id != self.owner_user_id
            || response.workspace.workspace_id != self.owner_user_id
        {
            return Err(anyhow!(
                "OpenAgents controller token response was inconsistent"
            ));
        }
        let convex_url = url::Url::parse(&response.convex_url)
            .context("OpenAgents controller Convex URL was invalid")?;
        if convex_url.scheme() != "https" || convex_url.host_str().is_none() {
            return Err(anyhow!("OpenAgents controller Convex URL was not HTTPS"));
        }
        Ok(OpenAgentsControllerBootstrap {
            convex_url: response.convex_url,
            token: response.token,
            owner_user_id: response.actor.user_id,
            workspace_id: response.workspace.workspace_id,
        })
    }
}

fn credential_username(credential: &StoredCredential) -> &str {
    credential
        .owner_user_id
        .as_deref()
        .unwrap_or(OPENAGENTS_CREDENTIAL_USERNAME)
}

fn projection_for_credential(
    state: HostedSessionState,
    credential: &StoredCredential,
) -> HostedSessionProjection {
    HostedSessionProjection {
        state,
        owner_user_id: credential.owner_user_id.clone(),
        issued_at: credential.issued_at,
        expires_at: credential.expires_at,
        omega_public_key_hex: credential.omega_public_key_hex.clone(),
        account_generation: credential.account_generation,
        retryable: false,
    }
}

fn credential_matches_account(
    credential: &StoredCredential,
    omega_public_key_hex: &str,
    account_generation: u64,
) -> bool {
    credential.omega_public_key_hex.as_deref() == Some(omega_public_key_hex)
        && credential.account_generation == Some(account_generation)
}

fn unix_time_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn credential_owner_matches_username(credential: &StoredCredential, username: &str) -> bool {
    match credential.owner_user_id.as_deref() {
        Some(owner_user_id) => !owner_user_id.is_empty() && owner_user_id == username,
        None => username == OPENAGENTS_CREDENTIAL_USERNAME,
    }
}

fn random_device_component() -> String {
    let bytes = rand::random::<[u8; 16]>();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn valid_device_ref(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

fn authenticated_session_request(
    method: Method,
    credential: &StoredCredential,
) -> Result<http_client::Request<AsyncBody>> {
    let mut builder = Request::builder()
        .method(method)
        .uri(OPENAGENTS_AUTH_SESSION_URL)
        .header(
            "authorization",
            format!("Bearer {}", credential.access_token),
        );
    if let Some(refresh_token) = credential.refresh_token.as_deref() {
        builder = builder.header(OPENAGENTS_REFRESH_HEADER, refresh_token);
    }
    Ok(builder.body(AsyncBody::empty())?)
}

async fn send_json(
    client: &Arc<dyn HttpClient>,
    request: http_client::Request<AsyncBody>,
) -> Result<(StatusCode, Vec<u8>)> {
    let mut response = client.send(request).await?;
    let status = response.status();
    let mut body = Vec::new();
    response
        .body_mut()
        .take(MAX_HTTP_BODY_BYTES)
        .read_to_end(&mut body)
        .await?;
    Ok((status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connecting_copy_does_not_claim_a_browser_flow() {
        assert_eq!(
            OpenAgentsSessionPhase::Connecting.label(),
            "Connecting securely…"
        );
        assert!(
            !OpenAgentsSessionPhase::Connecting
                .label()
                .contains("browser")
        );
    }

    #[test]
    fn credential_namespace_and_public_phase_never_contain_tokens() {
        assert_eq!(
            OPENAGENTS_SESSION_KEY,
            "omega://openagents/native-session/v1"
        );
        assert_eq!(
            OPENAGENTS_DEVICE_KEY,
            "omega://openagents/device-binding/v1"
        );
        assert!(!OPENAGENTS_SESSION_KEY.contains("token"));
        let projection =
            serde_json::to_string(&OpenAgentsSessionPhase::Ready).expect("serialize phase");
        assert_eq!(projection, "\"ready\"");
        assert!(!projection.contains("access-fixture"));
    }

    #[test]
    fn legacy_bearer_credential_and_device_binding_remain_separate() {
        let credential: StoredCredential = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "ownerUserId": "owner.fixture",
            "accessToken": "short-lived-session"
        }))
        .expect("legacy bearer credential");
        assert_eq!(credential.owner_user_id.as_deref(), Some("owner.fixture"));
        assert_eq!(credential_username(&credential), "owner.fixture");
        assert_eq!(credential.refresh_token, None);

        let binding = StoredDeviceBinding {
            schema_version: 1,
            device_ref: "omega-device-fixture".into(),
        };
        assert!(valid_device_ref(&binding.device_ref));
        assert!(!valid_device_ref("contains a space"));
    }

    #[test]
    fn nostr_issued_bearer_does_not_persist_the_server_owner_mapping() {
        let credential = StoredCredential {
            schema_version: 1,
            owner_user_id: None,
            access_token: "short-lived-session".into(),
            refresh_token: None,
            issued_at: None,
            expires_at: None,
            omega_public_key_hex: None,
            account_generation: None,
        };
        let stored = serde_json::to_value(&credential).expect("serialize credential");
        assert!(stored.get("ownerUserId").is_none());
        assert_eq!(
            credential_username(&credential),
            OPENAGENTS_CREDENTIAL_USERNAME
        );
        assert!(credential_owner_matches_username(
            &credential,
            OPENAGENTS_CREDENTIAL_USERNAME
        ));
        assert!(!credential_owner_matches_username(
            &credential,
            "mapped-owner"
        ));
    }

    #[test]
    fn rotation_and_revoke_decoders_fail_closed() {
        let session: VerifiedSessionResponse = serde_json::from_value(serde_json::json!({
            "authenticated": true,
            "user": { "userId": "owner.fixture" },
            "tokens": { "access": "rotated-access", "refresh": "rotated-refresh", "expiresIn": 3600 }
        })).expect("verified session");
        assert_eq!(session.user.user_id, "owner.fixture");
        assert_eq!(session.tokens.expect("rotation").access, "rotated-access");

        let incomplete: RevokedSessionResponse = serde_json::from_value(serde_json::json!({
            "signedOut": true, "accessRevoked": true, "refreshRevoked": false
        }))
        .expect("revoke proof");
        assert!(
            !(incomplete.signed_out && incomplete.access_revoked && incomplete.refresh_revoked)
        );
    }

    /// omega#160. The schedule is the whole fix, so its shape is asserted
    /// exactly. The defect it replaces was a single re-attempt a few seconds
    /// later, which lands inside a 10-70 second Cloud SQL Auth Proxy warm-up
    /// far too often and shows the owner a red banner for a dependency that
    /// was about to answer.
    #[test]
    fn the_hosted_backoff_doubles_and_stops_on_both_attempts_and_wall_clock() {
        let schedule = HostedRetrySchedule::production();
        assert_eq!(schedule.max_attempts, 5);
        assert!(schedule.jitter);

        // Four re-attempts after the first failure: 1s, 2s, 4s, 8s.
        let delays: Vec<Duration> = (1..=5)
            .map(|attempt| schedule.delay_after(attempt, Duration::ZERO))
            .take_while(Option::is_some)
            .flatten()
            .collect();
        assert_eq!(
            delays,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
            ]
        );
        assert_eq!(delays.iter().sum::<Duration>(), Duration::from_secs(15));

        // The attempt count is a hard stop even with the whole budget unspent.
        assert_eq!(schedule.delay_after(5, Duration::ZERO), None);

        // The budget counts the failed requests, not just the sleeping. A
        // 30-second server-side connect timeout on the first two attempts
        // spends it, and no third attempt is scheduled — which is the
        // difference between a bounded wait and a two-and-a-half-minute one.
        assert_eq!(
            schedule.delay_after(1, Duration::from_secs(29)),
            None,
            "a 1s delay must not start past the budget"
        );
        assert_eq!(
            schedule.delay_after(2, Duration::from_secs(31)),
            None,
            "an attempt that already overran the budget schedules nothing"
        );
        assert_eq!(
            schedule.delay_after(2, Duration::from_secs(3)),
            Some(Duration::from_secs(2)),
            "a fast typed 503 leaves the budget intact"
        );
    }

    /// Requirement from the same issue: a test must never sleep through the
    /// production backoff, and a deterministic harness must be able to observe
    /// the first answer exactly as the service gave it.
    #[test]
    fn injected_schedules_never_sleep() {
        let instant = HostedRetrySchedule::instant();
        assert_eq!(instant.max_attempts, HOSTED_RETRY_MAX_ATTEMPTS);
        assert!(!instant.jitter);
        for attempt in 1..instant.max_attempts {
            assert_eq!(
                instant.delay_after(attempt, Duration::from_secs(600)),
                Some(Duration::ZERO)
            );
            assert_eq!(instant.with_jitter(Duration::ZERO), Duration::ZERO);
        }

        let single = HostedRetrySchedule::single_attempt();
        assert_eq!(single.max_attempts, 1);
        assert_eq!(single.delay_after(1, Duration::ZERO), None);
    }

    /// Jitter spreads instances that all saw the same deploy, and must only
    /// ever lengthen a wait — a shortened one would re-attempt into the same
    /// unready instance sooner than the schedule says.
    #[test]
    fn jitter_only_lengthens_a_delay_and_stays_bounded() {
        let schedule = HostedRetrySchedule::production();
        for _ in 0..64 {
            let jittered = schedule.with_jitter(Duration::from_secs(4));
            assert!(jittered >= Duration::from_secs(4));
            assert!(jittered <= Duration::from_secs(5));
        }
    }

    #[test]
    fn hosted_credential_is_bound_to_omega_account_key_and_generation() {
        let credential = StoredCredential {
            schema_version: 2,
            owner_user_id: Some("owner.fixture".to_string()),
            access_token: "session".to_string(),
            refresh_token: None,
            issued_at: Some(100),
            expires_at: Some(200),
            omega_public_key_hex: Some("aa".repeat(32)),
            account_generation: Some(7),
        };
        assert!(credential_matches_account(&credential, &"aa".repeat(32), 7));
        assert!(!credential_matches_account(
            &credential,
            &"bb".repeat(32),
            7
        ));
        assert!(!credential_matches_account(
            &credential,
            &"aa".repeat(32),
            8
        ));
        let projection =
            projection_for_credential(HostedSessionState::AccountMismatch, &credential);
        assert_eq!(projection.state, HostedSessionState::AccountMismatch);
        assert_eq!(projection.account_generation, Some(7));
        assert_eq!(projection.omega_public_key_hex, Some("aa".repeat(32)));
    }
}
