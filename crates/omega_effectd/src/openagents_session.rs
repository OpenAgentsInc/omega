use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow};
use base64::Engine as _;
use credentials_provider::CredentialsProvider;
use gpui::{App, AsyncApp, Global};
use http_client::{AsyncBody, HttpClient, Method, Request, StatusCode};
use serde::{Deserialize, Serialize};
use smol::io::AsyncReadExt as _;

use super::openagents_nostr_auth::HostedSessionBlocker;

pub const OPENAGENTS_BASE_URL: &str = "https://openagents.com";
pub const OPENAGENTS_AUTH_SESSION_URL: &str = "https://openagents.com/api/mobile/auth/session";
pub const OPENAGENTS_SESSION_KEY: &str = "omega://openagents/native-session/v1";
pub const OPENAGENTS_DEVICE_KEY: &str = "omega://openagents/device-binding/v1";

const OPENAGENTS_REFRESH_HEADER: &str = "x-openagents-refresh-token";
const MAX_HTTP_BODY_BYTES: u64 = 64 * 1024;
const MAX_ACCESS_TOKEN_BYTES: usize = 16 * 1024;

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
}

struct OpenAgentsSessionGlobal(OpenAgentsSession);
impl Global for OpenAgentsSessionGlobal {}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredCredential {
    schema_version: u8,
    owner_user_id: String,
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
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

enum VerificationResult {
    Verified(StoredCredential),
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
    }));
}

pub fn openagents_session(cx: &App) -> OpenAgentsSession {
    cx.global::<OpenAgentsSessionGlobal>().0.clone()
}

impl OpenAgentsSession {
    pub fn phase(&self) -> OpenAgentsSessionPhase {
        self.phase.lock().map(|phase| *phase).unwrap_or_default()
    }

    /// The reason the most recent connection attempt did not produce a session.
    pub fn blocker(&self) -> Option<HostedSessionBlocker> {
        self.blocker.lock().ok().and_then(|blocker| blocker.clone())
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
        let (phase, blocker) = match self.connect_inner().await {
            Ok(credential) => {
                if let Err(error) = self.save_credential(&credential, cx).await {
                    log::error!("hosted OpenAgents session could not be stored: {error:#}");
                    (
                        OpenAgentsSessionPhase::Unavailable,
                        Some(HostedSessionBlocker::CredentialStorageFailed),
                    )
                } else {
                    (OpenAgentsSessionPhase::Ready, None)
                }
            }
            Err(blocker) => (OpenAgentsSessionPhase::Unavailable, Some(blocker)),
        };
        self.record_blocker(blocker);
        self.set_phase(phase);
        phase
    }

    pub async fn disconnect(&self, cx: &mut AsyncApp) -> OpenAgentsSessionPhase {
        self.set_phase(OpenAgentsSessionPhase::Disconnecting);
        let phase = match self.load_credential(cx).await {
            Ok(None) => OpenAgentsSessionPhase::SignedOut,
            Ok(Some(credential)) => match self.revoke_credential(&credential).await {
                Ok(true) if self.delete_credential(cx).await.is_ok() => {
                    OpenAgentsSessionPhase::SignedOut
                }
                Ok(true) | Ok(false) | Err(_) => OpenAgentsSessionPhase::Unavailable,
            },
            Err(_) => OpenAgentsSessionPhase::Unavailable,
        };
        self.set_phase(phase);
        phase
    }

    pub async fn resolve_verified(&self, cx: &mut AsyncApp) -> Option<VerifiedOpenAgentsSession> {
        let credential = match self.load_credential(cx).await {
            Ok(Some(credential)) => credential,
            Ok(None) => {
                self.set_phase(OpenAgentsSessionPhase::SignedOut);
                return None;
            }
            Err(error) => {
                log::error!("hosted OpenAgents credential could not be read: {error:#}");
                self.record_blocker(Some(HostedSessionBlocker::CredentialStorageFailed));
                self.set_phase(OpenAgentsSessionPhase::Unavailable);
                return None;
            }
        };
        match self.verify_credential(&credential).await {
            VerificationResult::Verified(verified) => {
                if verified.access_token.len() > MAX_ACCESS_TOKEN_BYTES
                    || self.save_credential(&verified, cx).await.is_err()
                {
                    self.record_blocker(Some(HostedSessionBlocker::CredentialStorageFailed));
                    self.set_phase(OpenAgentsSessionPhase::Unavailable);
                    return None;
                }
                self.record_blocker(None);
                self.set_phase(OpenAgentsSessionPhase::Ready);
                Some(VerifiedOpenAgentsSession {
                    base_url: OPENAGENTS_BASE_URL.to_string(),
                    owner_user_id: verified.owner_user_id,
                    access_token: verified.access_token,
                })
            }
            VerificationResult::Denied => {
                let phase = if self.delete_credential(cx).await.is_ok() {
                    OpenAgentsSessionPhase::Denied
                } else {
                    OpenAgentsSessionPhase::Unavailable
                };
                self.set_phase(phase);
                None
            }
            VerificationResult::Unavailable => {
                self.record_blocker(Some(HostedSessionBlocker::SessionNotVerified));
                self.set_phase(OpenAgentsSessionPhase::Unavailable);
                None
            }
        }
    }

    pub async fn create_sarah_voice_session(
        &self,
        thread_ref: &str,
        cx: &mut AsyncApp,
    ) -> std::result::Result<
        super::openagents_sarah_voice::ManagedSarahVoiceSession,
        HostedSessionBlocker,
    > {
        let session_ref = format!("omega-voice-{}", random_device_component());
        let device_ref = self.load_or_create_device_ref(cx).await.map_err(|error| {
            log::error!("OpenAgents device binding could not be stored: {error:#}");
            HostedSessionBlocker::CredentialStorageFailed
        })?;
        if let Some(verified) = self.resolve_verified(cx).await {
            match super::openagents_sarah_voice::issue_bearer_sarah_voice_session(
                &self.http_client,
                &verified.access_token,
                &verified.owner_user_id,
                &device_ref,
                thread_ref,
                &session_ref,
            )
            .await
            {
                Ok(issued) => {
                    self.record_blocker(None);
                    return Ok(issued.voice);
                }
                Err(HostedSessionBlocker::VoiceSessionRejected { status: 401 }) => {}
                Err(blocker) => {
                    self.record_blocker(Some(blocker.clone()));
                    return Err(blocker);
                }
            }
        }

        let issued = match super::openagents_sarah_voice::issue_nostr_sarah_voice_session(
            &self.http_client,
            &device_ref,
            thread_ref,
            &session_ref,
        )
        .await
        {
            Ok(issued) => issued,
            Err(blocker) => {
                self.record_blocker(Some(blocker.clone()));
                return Err(blocker);
            }
        };
        let access_token = issued
            .access_token
            .ok_or(HostedSessionBlocker::ResponseInvalid)?;
        let credential = StoredCredential {
            schema_version: 1,
            owner_user_id: issued.voice.owner_ref.clone(),
            access_token,
            refresh_token: None,
        };
        self.save_credential(&credential, cx)
            .await
            .map_err(|error| {
                log::error!("Nostr-issued OpenAgents session could not be stored: {error:#}");
                HostedSessionBlocker::CredentialStorageFailed
            })?;
        self.record_blocker(None);
        self.set_phase(OpenAgentsSessionPhase::Ready);
        Ok(issued.voice)
    }

    async fn connect_inner(&self) -> Result<StoredCredential, HostedSessionBlocker> {
        let session =
            super::openagents_nostr_auth::mint_openagents_nostr_session(&self.http_client).await?;
        let credential = StoredCredential {
            schema_version: 1,
            owner_user_id: session.user.user_id,
            access_token: session.access_token.trim().to_string(),
            refresh_token: None,
        };
        match self.verify_credential(&credential).await {
            VerificationResult::Verified(credential) => Ok(credential),
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
            || (!credential.owner_user_id.is_empty() && credential.owner_user_id != owner_user_id)
        {
            return VerificationResult::Denied;
        }
        let mut verified = StoredCredential {
            schema_version: 1,
            owner_user_id: owner_user_id.to_string(),
            access_token: credential.access_token.clone(),
            refresh_token: credential.refresh_token.clone(),
        };
        if let Some(tokens) = session.tokens {
            if tokens.access.trim().is_empty()
                || tokens.refresh.trim().is_empty()
                || tokens.access.len() > MAX_ACCESS_TOKEN_BYTES
                || !tokens.expires_in.is_finite()
                || tokens.expires_in <= 0.0
            {
                return VerificationResult::Unavailable;
            }
            verified.access_token = tokens.access.trim().to_string();
            verified.refresh_token = Some(tokens.refresh.trim().to_string());
        }
        VerificationResult::Verified(verified)
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
        let credential: StoredCredential =
            serde_json::from_slice(&secret).context("OpenAgents stored credential was invalid")?;
        if credential.schema_version != 1
            || credential.owner_user_id != username
            || credential.owner_user_id.trim().is_empty()
            || credential.access_token.trim().is_empty()
            || credential
                .refresh_token
                .as_ref()
                .is_some_and(|token| token.trim().is_empty())
            || credential.access_token.len() > MAX_ACCESS_TOKEN_BYTES
        {
            return Err(anyhow!("OpenAgents stored credential was invalid"));
        }
        Ok(Some(credential))
    }

    async fn save_credential(&self, credential: &StoredCredential, cx: &AsyncApp) -> Result<()> {
        if credential.owner_user_id.trim().is_empty()
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
                &credential.owner_user_id,
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
            schema_version: 1,
            device_ref: device_ref.clone(),
        })?;
        self.credentials
            .write_credentials(OPENAGENTS_DEVICE_KEY, "omega", &secret, cx)
            .await?;
        Ok(device_ref)
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
        assert_eq!(credential.owner_user_id, "owner.fixture");
        assert_eq!(credential.refresh_token, None);

        let binding = StoredDeviceBinding {
            schema_version: 1,
            device_ref: "omega-device-fixture".into(),
        };
        assert!(valid_device_ref(&binding.device_ref));
        assert!(!valid_device_ref("contains a space"));
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
}
