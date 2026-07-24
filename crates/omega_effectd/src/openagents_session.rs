use std::{
    io::{ErrorKind, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow};
use base64::Engine as _;
use credentials_provider::CredentialsProvider;
use gpui::{App, AsyncApp, Global};
use http_client::{AsyncBody, HttpClient, Method, Request, StatusCode};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use smol::io::AsyncReadExt as _;
use url::{Url, form_urlencoded};

pub const OPENAGENTS_DESKTOP_CLIENT_ID: &str = "openagents-desktop";
pub const OPENAGENTS_AUTHORIZE_URL: &str = "https://auth.openagents.com/authorize";
pub const OPENAGENTS_TOKEN_URL: &str = "https://auth.openagents.com/token";
pub const OPENAGENTS_BASE_URL: &str = "https://openagents.com";
pub const OPENAGENTS_AUTH_SESSION_URL: &str = "https://openagents.com/api/mobile/auth/session";
pub const OPENAGENTS_CALLBACK_PATH: &str = "/auth/callback";
pub const OPENAGENTS_SESSION_KEY: &str = "omega://openagents/native-session/v1";

const OPENAGENTS_REFRESH_HEADER: &str = "x-openagents-refresh-token";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(120);
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
            Self::Connecting => "Connecting in your browser…",
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
    pub access_token: String,
}

#[derive(Clone)]
pub struct OpenAgentsSession {
    credentials: Arc<dyn CredentialsProvider>,
    http_client: Arc<dyn HttpClient>,
    phase: Arc<Mutex<OpenAgentsSessionPhase>>,
}

struct OpenAgentsSessionGlobal(OpenAgentsSession);
impl Global for OpenAgentsSessionGlobal {}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredCredential {
    schema_version: u8,
    owner_user_id: String,
    access_token: String,
    refresh_token: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: Option<f64>,
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

enum CallbackResult {
    Code(String),
    Cancelled,
    Unavailable,
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
        credentials: zed_credentials_provider::system_keychain(cx),
        http_client: cx.http_client(),
        phase: Arc::new(Mutex::new(OpenAgentsSessionPhase::SignedOut)),
    }));
}

pub fn openagents_session(cx: &App) -> OpenAgentsSession {
    cx.global::<OpenAgentsSessionGlobal>().0.clone()
}

impl OpenAgentsSession {
    pub fn phase(&self) -> OpenAgentsSessionPhase {
        self.phase.lock().map(|phase| *phase).unwrap_or_default()
    }

    fn set_phase(&self, phase: OpenAgentsSessionPhase) {
        if let Ok(mut current) = self.phase.lock() {
            *current = phase;
        }
    }

    pub async fn connect(&self, cx: &mut AsyncApp) -> OpenAgentsSessionPhase {
        self.set_phase(OpenAgentsSessionPhase::Connecting);
        let phase = match self.connect_inner(cx).await {
            Ok(Some(credential)) if self.save_credential(&credential, cx).await.is_ok() => {
                OpenAgentsSessionPhase::Ready
            }
            Ok(None) => OpenAgentsSessionPhase::SignedOut,
            Ok(Some(_)) | Err(_) => OpenAgentsSessionPhase::Unavailable,
        };
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
            Err(_) => {
                self.set_phase(OpenAgentsSessionPhase::Unavailable);
                return None;
            }
        };
        match self.verify_credential(&credential).await {
            VerificationResult::Verified(verified) => {
                if verified.access_token.len() > MAX_ACCESS_TOKEN_BYTES
                    || self.save_credential(&verified, cx).await.is_err()
                {
                    self.set_phase(OpenAgentsSessionPhase::Unavailable);
                    return None;
                }
                self.set_phase(OpenAgentsSessionPhase::Ready);
                Some(VerifiedOpenAgentsSession {
                    base_url: OPENAGENTS_BASE_URL.to_string(),
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
                self.set_phase(OpenAgentsSessionPhase::Unavailable);
                None
            }
        }
    }

    async fn connect_inner(&self, cx: &mut AsyncApp) -> Result<Option<StoredCredential>> {
        let state = random_base64_url();
        let verifier = random_base64_url();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .context("OpenAgents callback listener unavailable")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        if port < 1024 {
            return Err(anyhow!("OpenAgents callback listener unavailable"));
        }
        let redirect_uri = format!("http://127.0.0.1:{port}{OPENAGENTS_CALLBACK_PATH}");
        let authorize_url = build_authorize_url(&redirect_uri, &state, &verifier)?;
        cx.update(|cx| cx.open_url(authorize_url.as_str()));
        let background_executor = cx.background_executor().clone();
        let code =
            match wait_for_callback(&listener, &state, CALLBACK_TIMEOUT, &background_executor)
                .await?
            {
                CallbackResult::Code(code) => code,
                CallbackResult::Cancelled => return Ok(None),
                CallbackResult::Unavailable => {
                    return Err(anyhow!("OpenAgents authorization timed out"));
                }
            };
        let tokens = self.exchange_code(&code, &verifier, &redirect_uri).await?;
        let credential = StoredCredential {
            schema_version: 1,
            owner_user_id: String::new(),
            access_token: tokens.access_token.trim().to_string(),
            refresh_token: tokens.refresh_token.trim().to_string(),
        };
        if credential.access_token.is_empty()
            || credential.refresh_token.is_empty()
            || credential.access_token.len() > MAX_ACCESS_TOKEN_BYTES
            || tokens
                .expires_in
                .is_some_and(|expires| !expires.is_finite() || expires <= 0.0)
        {
            return Err(anyhow!("OpenAgents token exchange was invalid"));
        }
        match self.verify_credential(&credential).await {
            VerificationResult::Verified(credential) => Ok(Some(credential)),
            VerificationResult::Denied | VerificationResult::Unavailable => {
                Err(anyhow!("OpenAgents session verification failed"))
            }
        }
    }

    async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<TokenResponse> {
        let body = form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", OPENAGENTS_DESKTOP_CLIENT_ID)
            .append_pair("code", code)
            .append_pair("code_verifier", verifier)
            .append_pair("grant_type", "authorization_code")
            .append_pair("redirect_uri", redirect_uri)
            .finish();
        let request = Request::builder()
            .method(Method::POST)
            .uri(OPENAGENTS_TOKEN_URL)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(AsyncBody::from(body))?;
        let (status, body) = send_json(&self.http_client, request).await?;
        if !status.is_success() {
            return Err(anyhow!("OpenAgents token exchange failed ({status})"));
        }
        serde_json::from_slice(&body).context("OpenAgents token exchange response was invalid")
    }

    async fn verify_credential(&self, credential: &StoredCredential) -> VerificationResult {
        let request = match authenticated_session_request(Method::GET, credential) {
            Ok(request) => request,
            Err(_) => return VerificationResult::Unavailable,
        };
        let (status, body) = match send_json(&self.http_client, request).await {
            Ok(response) => response,
            Err(_) => return VerificationResult::Unavailable,
        };
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return VerificationResult::Denied;
        }
        if !status.is_success() {
            return VerificationResult::Unavailable;
        }
        let Ok(session) = serde_json::from_slice::<VerifiedSessionResponse>(&body) else {
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
            verified.refresh_token = tokens.refresh.trim().to_string();
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
        Ok(proof.signed_out && proof.access_revoked && proof.refresh_revoked)
    }

    async fn load_credential(&self, cx: &AsyncApp) -> Result<Option<StoredCredential>> {
        let Some((username, secret)) = self
            .credentials
            .read_credentials(OPENAGENTS_SESSION_KEY, cx)
            .await?
        else {
            return Ok(None);
        };
        let credential: StoredCredential = serde_json::from_slice(&secret)
            .context("OpenAgents keychain credential was invalid")?;
        if credential.schema_version != 1
            || credential.owner_user_id != username
            || credential.owner_user_id.trim().is_empty()
            || credential.access_token.trim().is_empty()
            || credential.refresh_token.trim().is_empty()
            || credential.access_token.len() > MAX_ACCESS_TOKEN_BYTES
        {
            return Err(anyhow!("OpenAgents keychain credential was invalid"));
        }
        Ok(Some(credential))
    }

    async fn save_credential(&self, credential: &StoredCredential, cx: &AsyncApp) -> Result<()> {
        if credential.owner_user_id.trim().is_empty()
            || credential.access_token.trim().is_empty()
            || credential.refresh_token.trim().is_empty()
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
}

fn authenticated_session_request(
    method: Method,
    credential: &StoredCredential,
) -> Result<http_client::Request<AsyncBody>> {
    Ok(Request::builder()
        .method(method)
        .uri(OPENAGENTS_AUTH_SESSION_URL)
        .header(
            "authorization",
            format!("Bearer {}", credential.access_token),
        )
        .header(OPENAGENTS_REFRESH_HEADER, &credential.refresh_token)
        .body(AsyncBody::empty())?)
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

fn random_base64_url() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn build_authorize_url(redirect_uri: &str, state: &str, verifier: &str) -> Result<Url> {
    let mut authorize = Url::parse(OPENAGENTS_AUTHORIZE_URL)?;
    authorize
        .query_pairs_mut()
        .append_pair("client_id", OPENAGENTS_DESKTOP_CLIENT_ID)
        .append_pair("code_challenge", &pkce_challenge(verifier))
        .append_pair("code_challenge_method", "S256")
        .append_pair("provider", "github")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("state", state);
    Ok(authorize)
}

async fn wait_for_callback(
    listener: &TcpListener,
    expected_state: &str,
    timeout: Duration,
    background_executor: &gpui::BackgroundExecutor,
) -> Result<CallbackResult> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                if let Some(result) = read_callback(&mut stream, expected_state)? {
                    return Ok(result);
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }
        if Instant::now() >= deadline {
            return Ok(CallbackResult::Unavailable);
        }
        background_executor.timer(Duration::from_millis(50)).await;
    }
}

fn read_callback(stream: &mut TcpStream, expected_state: &str) -> Result<Option<CallbackResult>> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut buffer = [0_u8; 8192];
    let size = stream.read(&mut buffer)?;
    let request = std::str::from_utf8(&buffer[..size])?;
    let Some(request_line) = request.lines().next() else {
        write_callback_response(stream, 400)?;
        return Ok(None);
    };
    let mut parts = request_line.split_whitespace();
    if parts.next().unwrap_or_default() != "GET" {
        write_callback_response(stream, 400)?;
        return Ok(None);
    }
    let target = parts.next().unwrap_or_default();
    let url = Url::parse(&format!("http://127.0.0.1{target}"))?;
    if url.path() != OPENAGENTS_CALLBACK_PATH {
        write_callback_response(stream, 404)?;
        return Ok(None);
    }
    let parameter = |name: &str| {
        url.query_pairs()
            .find_map(|(key, value)| (key == name).then(|| value.into_owned()))
    };
    if parameter("state").as_deref() != Some(expected_state) {
        write_callback_response(stream, 400)?;
        return Ok(None);
    }
    if parameter("error").is_some_and(|error| !error.trim().is_empty()) {
        write_callback_response(stream, 200)?;
        return Ok(Some(CallbackResult::Cancelled));
    }
    let code = parameter("code").unwrap_or_default();
    if code.trim().is_empty() {
        write_callback_response(stream, 400)?;
        return Ok(None);
    }
    write_callback_response(stream, 200)?;
    Ok(Some(CallbackResult::Code(code)))
}

fn write_callback_response(stream: &mut TcpStream, status: u16) -> Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body = "<!doctype html><meta charset=utf-8><meta name=referrer content=no-referrer><title>Omega</title><p>You can return to Omega.</p>";
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_uses_admitted_desktop_pkce_contract() {
        let url = build_authorize_url(
            "http://127.0.0.1:49152/auth/callback",
            "state-fixture",
            "verifier-fixture-with-enough-entropy-for-the-test",
        )
        .expect("authorize URL");
        assert_eq!(
            url.as_str().split('?').next(),
            Some(OPENAGENTS_AUTHORIZE_URL)
        );
        let values = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            values.get("client_id").map(|value| value.as_ref()),
            Some("openagents-desktop")
        );
        assert_eq!(
            values.get("provider").map(|value| value.as_ref()),
            Some("github")
        );
        assert_eq!(
            values
                .get("code_challenge_method")
                .map(|value| value.as_ref()),
            Some("S256")
        );
        assert_eq!(
            values.get("state").map(|value| value.as_ref()),
            Some("state-fixture")
        );
        assert_eq!(
            values.get("redirect_uri").map(|value| value.as_ref()),
            Some("http://127.0.0.1:49152/auth/callback")
        );
        assert_eq!(
            values.get("code_challenge").map(|value| value.len()),
            Some(43)
        );
    }

    #[test]
    fn keychain_namespace_and_public_phase_never_contain_tokens() {
        assert_eq!(
            OPENAGENTS_SESSION_KEY,
            "omega://openagents/native-session/v1"
        );
        assert!(!OPENAGENTS_SESSION_KEY.contains("token"));
        let projection =
            serde_json::to_string(&OpenAgentsSessionPhase::Ready).expect("serialize phase");
        assert_eq!(projection, "\"ready\"");
        assert!(!projection.contains("access-fixture"));
    }

    #[test]
    fn callback_requires_exact_path_state_and_hides_code() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("listener");
        let address = listener.local_addr().expect("address");
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).expect("connect");
            write!(stream, "GET /auth/callback?state=exact&code=private-code HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").expect("write");
            let mut response = String::new();
            stream.read_to_string(&mut response).expect("response");
            response
        });
        let (mut stream, _) = listener.accept().expect("accept");
        let result = read_callback(&mut stream, "exact").expect("callback");
        assert!(matches!(result, Some(CallbackResult::Code(code)) if code == "private-code"));
        drop(stream);
        assert!(!client.join().expect("client").contains("private-code"));
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
