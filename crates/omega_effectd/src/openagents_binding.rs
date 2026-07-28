//! OMEGA-SW-01: one-time binding of an Omega Nostr public key to an OpenAgents
//! account.
//!
//! Spec: docs/omega/2026-07-24-sarah-workroom-mvp-spec.md §4, §9.2.
//!
//! This is a recorded relation, not a per-request session and not a merge of
//! the two identities. Conversation transport uses NIP-42; metering and ledger
//! rows need the OpenAgents account id that this binding supplies.
//!
//! Client id is `openagents-omega` (never `openagents-desktop`). Public binding
//! facts live under the Omega data root. Token material stays in Omega isolated
//! private local credential custody and never enters a log, crash record, or UI projection.

use std::{
    fmt, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, anyhow};
use credentials_provider::CredentialsProvider;
use gpui::{App, AsyncApp, Global};
use http_client::{AsyncBody, HttpClient, Method, Request, StatusCode};
use serde::{Deserialize, Serialize};
use smol::io::AsyncReadExt as _;

/// Distinct Omega OpenAuth client identity. Never reuse the Electron client.
pub const OPENAGENTS_OMEGA_CLIENT_ID: &str = "openagents-omega";
pub const OPENAGENTS_AUTH_SESSION_URL: &str = "https://openagents.com/api/mobile/auth/session";
pub const OPENAGENTS_SARAH_OWNER_URL: &str = "https://openagents.com/api/mobile/sarah";
/// Storage key for binding credentials. Omega-namespaced by the provider.
pub const OPENAGENTS_BINDING_CREDENTIAL_KEY: &str = "omega://openagents/account-binding/v1";
/// On-disk public relation schema (no secrets).
pub const BINDING_RECORD_SCHEMA: &str = "openagents.omega.account-binding.v1";
pub const BINDING_RECORD_FILE: &str = "account-binding.json";
pub const BINDING_RECORD_SUBDIR: &str = "openagents";

/// Honest owner-scope gate copy. Must not look like a network fault.
pub const OWNER_SCOPE_REFUSED_MESSAGE: &str = "The Sarah workroom is owner-scoped today. This OpenAgents account is not admitted for the MVP owner gate. This is not a network fault.";

const OPENAGENTS_REFRESH_HEADER: &str = "x-openagents-refresh-token";
const MAX_HTTP_BODY_BYTES: u64 = 64 * 1024;
const MAX_ACCESS_TOKEN_BYTES: usize = 16 * 1024;

/// Visible binding states for UI and session_status projection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingState {
    #[default]
    Unbound,
    Bound,
    Refused,
}

impl BindingState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unbound => "unbound",
            Self::Bound => "bound",
            Self::Refused => "refused",
        }
    }

    /// UI-facing status line. Refused carries the owner-scope message.
    pub fn status_line(self) -> &'static str {
        match self {
            Self::Unbound => "OpenAgents account unbound",
            Self::Bound => "OpenAgents account bound",
            Self::Refused => OWNER_SCOPE_REFUSED_MESSAGE,
        }
    }
}

/// Public-safe projection. Never carries tokens, refresh material, or secrets.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingProjection {
    pub state: BindingState,
    pub omega_public_key_hex: Option<String>,
    pub openagents_account_id: Option<String>,
    pub account_label: Option<String>,
    /// Set only when `state == Refused`. Honest owner-scope copy.
    pub gate_message: Option<String>,
    pub bound_at: Option<String>,
    pub client_id: String,
}

impl BindingProjection {
    pub fn unbound() -> Self {
        Self {
            state: BindingState::Unbound,
            omega_public_key_hex: None,
            openagents_account_id: None,
            account_label: None,
            gate_message: None,
            bound_at: None,
            client_id: OPENAGENTS_OMEGA_CLIENT_ID.to_string(),
        }
    }

    /// Serialize for UI / logs. Asserted free of secret-shaped keys.
    pub fn public_json(&self) -> Result<String> {
        serde_json::to_string(self).context("serialize binding projection")
    }
}

/// On-disk public relation. Identities stay distinct fields (never merged).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BindingRecord {
    schema: String,
    schema_version: u8,
    state: BindingState,
    omega_public_key_hex: String,
    openagents_account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gate_message: Option<String>,
    bound_at: String,
    client_id: String,
}

impl BindingRecord {
    fn validate(&self) -> Result<()> {
        if self.schema != BINDING_RECORD_SCHEMA {
            return Err(anyhow!("binding record schema mismatch"));
        }
        if self.schema_version != 1 {
            return Err(anyhow!("binding record schema version unsupported"));
        }
        if self.client_id != OPENAGENTS_OMEGA_CLIENT_ID {
            return Err(anyhow!("binding record client id is not openagents-omega"));
        }
        if self.omega_public_key_hex.trim().is_empty()
            || self.openagents_account_id.trim().is_empty()
        {
            return Err(anyhow!("binding record identities incomplete"));
        }
        // Falsifier: identities must remain related, not merged into one field.
        if self.omega_public_key_hex == self.openagents_account_id {
            return Err(anyhow!("binding record merges distinct identities"));
        }
        match self.state {
            BindingState::Bound => {
                if self.gate_message.is_some() {
                    return Err(anyhow!("bound record must not carry a gate message"));
                }
            }
            BindingState::Refused => {
                if self.gate_message.as_deref() != Some(OWNER_SCOPE_REFUSED_MESSAGE) {
                    return Err(anyhow!("refused record missing owner-scope gate message"));
                }
            }
            BindingState::Unbound => {
                return Err(anyhow!("binding record must not persist unbound"));
            }
        }
        Ok(())
    }

    fn projection(&self) -> BindingProjection {
        BindingProjection {
            state: self.state,
            omega_public_key_hex: Some(self.omega_public_key_hex.clone()),
            openagents_account_id: Some(self.openagents_account_id.clone()),
            account_label: self.account_label.clone(),
            gate_message: self.gate_message.clone(),
            bound_at: Some(self.bound_at.clone()),
            client_id: self.client_id.clone(),
        }
    }
}

/// Credential custody payload. Never serialized into UI structs or logs.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BindingCredential {
    schema_version: u8,
    openagents_account_id: String,
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
}

impl fmt::Debug for BindingCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BindingCredential")
            .field("schema_version", &self.schema_version)
            .field("openagents_account_id", &self.openagents_account_id)
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .finish()
    }
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

/// Result of the owner-scope gate check after a verified OpenAgents account.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OwnerScopeDecision {
    Admitted,
    Refused,
    Unavailable,
}

#[derive(Clone)]
pub struct OpenAgentsBinding {
    data_root: PathBuf,
    credentials: Arc<dyn CredentialsProvider>,
    http_client: Arc<dyn HttpClient>,
    phase: Arc<Mutex<BindingState>>,
}

struct OpenAgentsBindingGlobal(OpenAgentsBinding);
impl Global for OpenAgentsBindingGlobal {}

pub fn binding_record_path(data_root: &Path) -> PathBuf {
    data_root
        .join(BINDING_RECORD_SUBDIR)
        .join(BINDING_RECORD_FILE)
}

/// Default Omega data root for binding records (channel-local, never Zed).
pub fn default_binding_data_root() -> PathBuf {
    paths::data_dir().clone()
}

pub fn init_openagents_binding(cx: &mut App) {
    if cx.try_global::<OpenAgentsBindingGlobal>().is_some() {
        return;
    }
    let binding = OpenAgentsBinding::new(
        default_binding_data_root(),
        zed_credentials_provider::local_credentials(cx),
        cx.http_client(),
    );
    // Load any existing public record so the visible state is honest at boot.
    let _ = binding.refresh_phase_from_disk();
    cx.set_global(OpenAgentsBindingGlobal(binding));
}

pub fn openagents_binding(cx: &App) -> OpenAgentsBinding {
    cx.global::<OpenAgentsBindingGlobal>().0.clone()
}

pub fn try_openagents_binding(cx: &App) -> Option<OpenAgentsBinding> {
    cx.try_global::<OpenAgentsBindingGlobal>()
        .map(|global| global.0.clone())
}

impl OpenAgentsBinding {
    pub fn new(
        data_root: PathBuf,
        credentials: Arc<dyn CredentialsProvider>,
        http_client: Arc<dyn HttpClient>,
    ) -> Self {
        Self {
            data_root,
            credentials,
            http_client,
            phase: Arc::new(Mutex::new(BindingState::Unbound)),
        }
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn record_path(&self) -> PathBuf {
        binding_record_path(&self.data_root)
    }

    pub fn state(&self) -> BindingState {
        self.phase.lock().map(|state| *state).unwrap_or_default()
    }

    fn set_state(&self, state: BindingState) {
        if let Ok(mut current) = self.phase.lock() {
            *current = state;
        }
    }

    pub fn refresh_phase_from_disk(&self) -> BindingProjection {
        let projection = self.load_projection();
        self.set_state(projection.state);
        projection
    }

    pub fn load_projection(&self) -> BindingProjection {
        match self.read_record() {
            Ok(Some(record)) => record.projection(),
            Ok(None) | Err(_) => BindingProjection::unbound(),
        }
    }

    /// Resolve the OpenAgents account id for metering given an Omega pubkey.
    /// Returns None when unbound, refused, or pubkey does not match the record.
    pub fn resolve_account_id(&self, omega_public_key_hex: &str) -> Option<String> {
        let record = self.read_record().ok().flatten()?;
        if record.state != BindingState::Bound {
            return None;
        }
        if record.omega_public_key_hex != omega_public_key_hex {
            return None;
        }
        Some(record.openagents_account_id)
    }

    /// Prove the built-in Omega identity in the background and record the relation.
    pub async fn bind(&self, omega_public_key_hex: &str, cx: &mut AsyncApp) -> BindingProjection {
        let pubkey = omega_public_key_hex.trim();
        if pubkey.is_empty() {
            let projection = BindingProjection::unbound();
            self.set_state(BindingState::Unbound);
            return projection;
        }

        match self.bind_inner(pubkey, cx).await {
            Ok(projection) => {
                self.set_state(projection.state);
                projection
            }
            Err(_) => {
                // Network / exchange failures are not owner-scope refusal.
                // Leave any prior durable record alone; surface unbound only
                // when nothing was written in this attempt.
                let projection = self.load_projection();
                self.set_state(projection.state);
                projection
            }
        }
    }

    /// Clear the public relation and isolated binding credentials.
    pub async fn clear(&self, cx: &mut AsyncApp) -> BindingProjection {
        let _ = self.delete_credential(cx).await;
        let _ = self.remove_record();
        self.set_state(BindingState::Unbound);
        BindingProjection::unbound()
    }

    async fn bind_inner(
        &self,
        omega_public_key_hex: &str,
        cx: &mut AsyncApp,
    ) -> Result<BindingProjection> {
        let session =
            super::openagents_nostr_auth::mint_openagents_nostr_session(&self.http_client)
                .await
                .map_err(|blocker| {
                    anyhow::anyhow!("OpenAgents binding sign-in failed: {}", blocker.summary())
                })?;
        let mut credential = BindingCredential {
            schema_version: 1,
            openagents_account_id: session.user.user_id,
            access_token: session.access_token.trim().to_string(),
            refresh_token: None,
        };

        let verified = self
            .verify_credential(&credential)
            .await
            .context("OpenAgents binding session verification failed")?;
        credential = verified;

        let scope = self.check_owner_scope(&credential).await;
        let bound_at = now_iso8601();
        let projection = match scope {
            OwnerScopeDecision::Admitted => {
                let record = BindingRecord {
                    schema: BINDING_RECORD_SCHEMA.to_string(),
                    schema_version: 1,
                    state: BindingState::Bound,
                    omega_public_key_hex: omega_public_key_hex.to_string(),
                    openagents_account_id: credential.openagents_account_id.clone(),
                    account_label: None,
                    gate_message: None,
                    bound_at,
                    client_id: OPENAGENTS_OMEGA_CLIENT_ID.to_string(),
                };
                record.validate()?;
                self.write_record(&record)?;
                self.save_credential(&credential, cx).await?;
                record.projection()
            }
            OwnerScopeDecision::Refused => {
                // Durable refused relation so the UI stays honest across restarts.
                // Do not keep tokens for a refused owner-scope account.
                let _ = self.delete_credential(cx).await;
                let record = BindingRecord {
                    schema: BINDING_RECORD_SCHEMA.to_string(),
                    schema_version: 1,
                    state: BindingState::Refused,
                    omega_public_key_hex: omega_public_key_hex.to_string(),
                    openagents_account_id: credential.openagents_account_id.clone(),
                    account_label: None,
                    gate_message: Some(OWNER_SCOPE_REFUSED_MESSAGE.to_string()),
                    bound_at,
                    client_id: OPENAGENTS_OMEGA_CLIENT_ID.to_string(),
                };
                record.validate()?;
                self.write_record(&record)?;
                record.projection()
            }
            OwnerScopeDecision::Unavailable => {
                // Not a refusal. Do not write a refused record for a network fault.
                return Err(anyhow!("OpenAgents owner-scope gate unavailable"));
            }
        };
        Ok(projection)
    }

    async fn verify_credential(&self, credential: &BindingCredential) -> Result<BindingCredential> {
        let request = authenticated_request(Method::GET, OPENAGENTS_AUTH_SESSION_URL, credential)?;
        let (status, body) = send_json(&self.http_client, request).await?;
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(anyhow!("OpenAgents binding session denied"));
        }
        if !status.is_success() {
            return Err(anyhow!("OpenAgents binding session unavailable ({status})"));
        }
        let session: VerifiedSessionResponse = serde_json::from_slice(&body)
            .context("OpenAgents binding session response was invalid")?;
        let account_id = session.user.user_id.trim();
        if !session.authenticated || account_id.is_empty() {
            return Err(anyhow!("OpenAgents binding session was not authenticated"));
        }
        let mut verified = BindingCredential {
            schema_version: 1,
            openagents_account_id: account_id.to_string(),
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
                return Err(anyhow!("OpenAgents binding token rotation was invalid"));
            }
            verified.access_token = tokens.access.trim().to_string();
            verified.refresh_token = Some(tokens.refresh.trim().to_string());
        }
        Ok(verified)
    }

    async fn check_owner_scope(&self, credential: &BindingCredential) -> OwnerScopeDecision {
        let request =
            match authenticated_request(Method::GET, OPENAGENTS_SARAH_OWNER_URL, credential) {
                Ok(request) => request,
                Err(_) => return OwnerScopeDecision::Unavailable,
            };
        let (status, _body) = match send_json(&self.http_client, request).await {
            Ok(response) => response,
            Err(_) => return OwnerScopeDecision::Unavailable,
        };
        if status.is_success() {
            return OwnerScopeDecision::Admitted;
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return OwnerScopeDecision::Refused;
        }
        OwnerScopeDecision::Unavailable
    }

    fn read_record(&self) -> Result<Option<BindingRecord>> {
        let path = self.record_path();
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        // Public record must never contain token material.
        if bytes_look_secret_shaped(&bytes) {
            return Err(anyhow!("binding record contains secret-shaped material"));
        }
        let record: BindingRecord =
            serde_json::from_slice(&bytes).context("binding record decode failed")?;
        record.validate()?;
        Ok(Some(record))
    }

    fn write_record(&self, record: &BindingRecord) -> Result<()> {
        record.validate()?;
        let path = self.record_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let bytes = serde_json::to_vec_pretty(record)?;
        if bytes_look_secret_shaped(&bytes) {
            return Err(anyhow!("refusing to write secret-shaped binding record"));
        }
        // Path hygiene: never Zed or ~/.codex.
        let rendered = path.to_string_lossy();
        if rendered.contains("/.codex")
            || rendered.contains("\\.codex")
            || rendered.contains("/Zed/")
            || rendered.contains("\\Zed\\")
            || rendered.contains("/zed/")
        {
            return Err(anyhow!(
                "binding record path escapes Omega data root: {rendered}"
            ));
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &bytes).with_context(|| format!("write {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("publish {}", path.display()))?;
        Ok(())
    }

    fn remove_record(&self) -> Result<()> {
        let path = self.record_path();
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    async fn save_credential(&self, credential: &BindingCredential, cx: &AsyncApp) -> Result<()> {
        if credential.openagents_account_id.trim().is_empty()
            || credential.access_token.trim().is_empty()
            || credential
                .refresh_token
                .as_ref()
                .is_some_and(|token| token.trim().is_empty())
        {
            return Err(anyhow!("binding credential incomplete"));
        }
        let secret = serde_json::to_vec(credential)?;
        self.credentials
            .write_credentials(
                OPENAGENTS_BINDING_CREDENTIAL_KEY,
                &credential.openagents_account_id,
                &secret,
                cx,
            )
            .await
    }

    async fn delete_credential(&self, cx: &AsyncApp) -> Result<()> {
        self.credentials
            .delete_credentials(OPENAGENTS_BINDING_CREDENTIAL_KEY, cx)
            .await
    }
}

/// Apply a pure state-machine transition used by tests and offline recovery.
pub fn apply_binding_transition(
    current: BindingState,
    event: BindingEvent,
) -> (BindingState, Option<&'static str>) {
    match (current, event) {
        (_, BindingEvent::Clear) => (BindingState::Unbound, None),
        (
            BindingState::Unbound | BindingState::Bound | BindingState::Refused,
            BindingEvent::BindAdmitted,
        ) => (BindingState::Bound, None),
        (
            BindingState::Unbound | BindingState::Bound | BindingState::Refused,
            BindingEvent::BindRefused,
        ) => (BindingState::Refused, Some(OWNER_SCOPE_REFUSED_MESSAGE)),
        (_, BindingEvent::BindCancelled | BindingEvent::BindUnavailable) => (current, None),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingEvent {
    BindAdmitted,
    BindRefused,
    BindCancelled,
    BindUnavailable,
    Clear,
}

fn authenticated_request(
    method: Method,
    uri: &str,
    credential: &BindingCredential,
) -> Result<http_client::Request<AsyncBody>> {
    let mut builder = Request::builder().method(method).uri(uri).header(
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

fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Compact UTC stamp; sufficient for public relation metadata.
    format!("{secs}")
}

fn bytes_look_secret_shaped(bytes: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    lower.contains("access_token")
        || lower.contains("accesstoken")
        || lower.contains("refresh_token")
        || lower.contains("refreshtoken")
        || lower.contains("\"bearer ")
        || lower.contains("authorization")
}

/// Pure helpers used by the state machine and offline store tests without GPUI.
#[cfg(test)]
mod store {
    use super::*;

    pub fn write_public_record(data_root: &Path, record: &BindingRecord) -> Result<()> {
        let binding = OpenAgentsBinding {
            data_root: data_root.to_path_buf(),
            credentials: Arc::new(NoopCredentials),
            http_client: Arc::new(NoopHttp),
            phase: Arc::new(Mutex::new(BindingState::Unbound)),
        };
        binding.write_record(record)
    }

    pub fn read_public_record(data_root: &Path) -> Result<Option<BindingRecord>> {
        let binding = OpenAgentsBinding {
            data_root: data_root.to_path_buf(),
            credentials: Arc::new(NoopCredentials),
            http_client: Arc::new(NoopHttp),
            phase: Arc::new(Mutex::new(BindingState::Unbound)),
        };
        binding.read_record()
    }

    pub fn make_bound_record(
        omega_public_key_hex: &str,
        openagents_account_id: &str,
    ) -> BindingRecord {
        BindingRecord {
            schema: BINDING_RECORD_SCHEMA.to_string(),
            schema_version: 1,
            state: BindingState::Bound,
            omega_public_key_hex: omega_public_key_hex.to_string(),
            openagents_account_id: openagents_account_id.to_string(),
            account_label: None,
            gate_message: None,
            bound_at: "0".to_string(),
            client_id: OPENAGENTS_OMEGA_CLIENT_ID.to_string(),
        }
    }

    pub fn make_refused_record(
        omega_public_key_hex: &str,
        openagents_account_id: &str,
    ) -> BindingRecord {
        BindingRecord {
            schema: BINDING_RECORD_SCHEMA.to_string(),
            schema_version: 1,
            state: BindingState::Refused,
            omega_public_key_hex: omega_public_key_hex.to_string(),
            openagents_account_id: openagents_account_id.to_string(),
            account_label: None,
            gate_message: Some(OWNER_SCOPE_REFUSED_MESSAGE.to_string()),
            bound_at: "0".to_string(),
            client_id: OPENAGENTS_OMEGA_CLIENT_ID.to_string(),
        }
    }

    struct NoopCredentials;
    impl CredentialsProvider for NoopCredentials {
        fn read_credentials<'a>(
            &'a self,
            _url: &'a str,
            _cx: &'a AsyncApp,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>,
        > {
            Box::pin(async { Ok(None) })
        }

        fn write_credentials<'a>(
            &'a self,
            _url: &'a str,
            _username: &'a str,
            _password: &'a [u8],
            _cx: &'a AsyncApp,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
            Box::pin(async { Ok(()) })
        }

        fn delete_credentials<'a>(
            &'a self,
            _url: &'a str,
            _cx: &'a AsyncApp,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct NoopHttp;
    impl HttpClient for NoopHttp {
        fn user_agent(&self) -> Option<&http_client::http::HeaderValue> {
            None
        }

        fn proxy(&self) -> Option<&http_client::Url> {
            None
        }

        fn send(
            &self,
            _req: http_client::Request<AsyncBody>,
        ) -> futures::future::BoxFuture<'static, Result<http_client::Response<AsyncBody>>> {
            use futures::FutureExt as _;
            async { Err(anyhow!("noop http client")) }.boxed()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn binding_record_retains_the_omega_client_identity() {
        assert_eq!(OPENAGENTS_OMEGA_CLIENT_ID, "openagents-omega");
        assert_ne!(OPENAGENTS_OMEGA_CLIENT_ID, "openagents-desktop");
    }

    #[test]
    fn state_machine_covers_unbound_bound_refused() {
        assert_eq!(
            apply_binding_transition(BindingState::Unbound, BindingEvent::BindAdmitted),
            (BindingState::Bound, None)
        );
        let (state, message) =
            apply_binding_transition(BindingState::Unbound, BindingEvent::BindRefused);
        assert_eq!(state, BindingState::Refused);
        assert_eq!(message, Some(OWNER_SCOPE_REFUSED_MESSAGE));
        let message = message.unwrap();
        assert!(message.contains("owner-scoped today"));
        // Must explicitly deny a network-fault reading, and must not claim timeout/unreachable.
        assert!(message.contains("not a network fault"));
        assert!(!message.to_ascii_lowercase().contains("timeout"));
        assert!(!message.to_ascii_lowercase().contains("unreachable"));
        assert!(!message.to_ascii_lowercase().contains("connection failed"));

        // Cancel / unavailable must not look like refusal.
        assert_eq!(
            apply_binding_transition(BindingState::Unbound, BindingEvent::BindCancelled),
            (BindingState::Unbound, None)
        );
        assert_eq!(
            apply_binding_transition(BindingState::Unbound, BindingEvent::BindUnavailable),
            (BindingState::Unbound, None)
        );
        assert_eq!(
            apply_binding_transition(BindingState::Bound, BindingEvent::Clear),
            (BindingState::Unbound, None)
        );
        assert_eq!(
            apply_binding_transition(BindingState::Refused, BindingEvent::Clear),
            (BindingState::Unbound, None)
        );
    }

    #[test]
    fn public_record_stores_distinct_identities_without_tokens() {
        let root = tempdir().expect("tempdir");
        // Simulate Omega RC data root shape.
        let data_root = root.path().join("Omega RC");
        let record = store::make_bound_record(
            "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899",
            "user.openagents.fixture",
        );
        store::write_public_record(&data_root, &record).expect("write");
        let path = binding_record_path(&data_root);
        let rendered = path.to_string_lossy();
        assert!(rendered.contains("Omega RC"));
        assert!(rendered.contains("openagents/account-binding.json"));
        assert!(!rendered.contains(".codex"));
        assert!(!rendered.contains("/Zed/"));
        assert!(!rendered.contains("/zed/"));

        let disk = fs::read_to_string(&path).expect("read disk");
        assert!(!disk.to_ascii_lowercase().contains("access_token"));
        assert!(!disk.to_ascii_lowercase().contains("refresh_token"));
        assert!(!disk.contains("Bearer"));
        assert!(disk.contains("openagents-omega"));
        assert!(disk.contains("user.openagents.fixture"));
        assert!(disk.contains("aabbccddeeff00112233445566778899"));

        let loaded = store::read_public_record(&data_root)
            .expect("read")
            .expect("present");
        assert_eq!(loaded.state, BindingState::Bound);
        assert_eq!(
            loaded.omega_public_key_hex,
            "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899"
        );
        assert_eq!(loaded.openagents_account_id, "user.openagents.fixture");
        // Distinct fields: relation, not merge.
        assert_ne!(loaded.omega_public_key_hex, loaded.openagents_account_id);
    }

    #[test]
    fn refused_record_carries_owner_scope_message_not_network_fault() {
        let root = tempdir().expect("tempdir");
        let record = store::make_refused_record(&"pk".repeat(32), "user.not-owner");
        store::write_public_record(root.path(), &record).expect("write");
        let loaded = store::read_public_record(root.path())
            .expect("read")
            .expect("present");
        assert_eq!(loaded.state, BindingState::Refused);
        assert_eq!(
            loaded.gate_message.as_deref(),
            Some(OWNER_SCOPE_REFUSED_MESSAGE)
        );
        let projection = loaded.projection();
        let json = projection.public_json().expect("json");
        assert!(json.contains("refused"));
        assert!(json.contains("owner-scoped today"));
        assert!(!json.to_ascii_lowercase().contains("access_token"));
        assert!(!json.to_ascii_lowercase().contains("refresh"));
        assert_eq!(projection.state.status_line(), OWNER_SCOPE_REFUSED_MESSAGE);
    }

    #[test]
    fn resolve_account_id_only_when_bound_and_pubkey_matches() {
        let root = tempdir().expect("tempdir");
        let pubkey = "11".repeat(32);
        let other = "22".repeat(32);
        store::write_public_record(
            root.path(),
            &store::make_bound_record(&pubkey, "account.metering"),
        )
        .expect("write");
        let binding = OpenAgentsBinding::new(
            root.path().to_path_buf(),
            Arc::new(TestNoopCredentials),
            Arc::new(TestNoopHttp),
        );
        assert_eq!(
            binding.resolve_account_id(&pubkey).as_deref(),
            Some("account.metering")
        );
        assert_eq!(binding.resolve_account_id(&other), None);

        store::write_public_record(
            root.path(),
            &store::make_refused_record(&pubkey, "account.metering"),
        )
        .expect("write refused");
        assert_eq!(
            binding.resolve_account_id(&pubkey),
            None,
            "refused must not resolve for metering"
        );
    }

    #[test]
    fn credential_debug_and_ui_projection_never_contain_tokens() {
        let credential = BindingCredential {
            schema_version: 1,
            openagents_account_id: "owner.fixture".to_string(),
            access_token: "secret-access-token-value".to_string(),
            refresh_token: Some("secret-refresh-token-value".to_string()),
        };
        let debug = format!("{credential:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret-access-token-value"));
        assert!(!debug.contains("secret-refresh-token-value"));

        let projection = BindingProjection {
            state: BindingState::Bound,
            omega_public_key_hex: Some("pk".into()),
            openagents_account_id: Some("owner.fixture".into()),
            account_label: Some("owner@example.com".into()),
            gate_message: None,
            bound_at: Some("1".into()),
            client_id: OPENAGENTS_OMEGA_CLIENT_ID.to_string(),
        };
        let json = projection.public_json().expect("json");
        assert!(!json.contains("access"));
        assert!(!json.contains("refresh"));
        assert!(!json.contains("token"));
        assert!(!json.contains("secret"));
        assert!(json.contains("\"state\":\"bound\""));
    }

    #[test]
    fn merged_identities_are_rejected() {
        let mut record = store::make_bound_record("same-id", "same-id");
        assert!(record.validate().is_err());
        record.openagents_account_id = "different".into();
        record.client_id = "openagents-desktop".into();
        assert!(record.validate().is_err());
    }

    struct TestNoopCredentials;
    impl CredentialsProvider for TestNoopCredentials {
        fn read_credentials<'a>(
            &'a self,
            _url: &'a str,
            _cx: &'a AsyncApp,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>,
        > {
            Box::pin(async { Ok(None) })
        }

        fn write_credentials<'a>(
            &'a self,
            _url: &'a str,
            _username: &'a str,
            _password: &'a [u8],
            _cx: &'a AsyncApp,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
            Box::pin(async { Ok(()) })
        }

        fn delete_credentials<'a>(
            &'a self,
            _url: &'a str,
            _cx: &'a AsyncApp,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct TestNoopHttp;
    impl HttpClient for TestNoopHttp {
        fn user_agent(&self) -> Option<&http_client::http::HeaderValue> {
            None
        }

        fn proxy(&self) -> Option<&http_client::Url> {
            None
        }

        fn send(
            &self,
            _req: http_client::Request<AsyncBody>,
        ) -> futures::future::BoxFuture<'static, Result<http_client::Response<AsyncBody>>> {
            use futures::FutureExt as _;
            async { Err(anyhow!("test noop http")) }.boxed()
        }
    }
}
