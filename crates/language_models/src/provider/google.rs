use anyhow::{Context as _, Result};
use collections::BTreeMap;
use credentials_provider::CredentialsProvider;
use futures::{AsyncReadExt as _, FutureExt, StreamExt, future::BoxFuture};
use google_ai::GenerateContentResponse;
pub use google_ai::completion::{GoogleEventMapper, into_google};
use gpui::{App, AppContext, AsyncApp, Context, Entity, SharedString, Task};
use http_client::{
    AsyncBody, CustomHeaders, HttpClient, Method, Request as HttpRequest, StatusCode,
};
use language_model::{
    ApiKeyConfiguration, AuthenticateError, EnvVar, LanguageModelCompletionError,
    LanguageModelCompletionEvent, LanguageModelToolChoice, LanguageModelToolSchemaFormat,
};
use language_model::{
    GOOGLE_PROVIDER_ID, GOOGLE_PROVIDER_NAME, IconOrSvg, LanguageModel, LanguageModelEffortLevel,
    LanguageModelId, LanguageModelName, LanguageModelProvider, LanguageModelProviderId,
    LanguageModelProviderName, LanguageModelProviderState, LanguageModelRequest,
    ProviderSettingsView, RateLimiter,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub use settings::GoogleAvailableModel as AvailableModel;
use settings::{Settings, SettingsStore};

use std::{
    sync::{Arc, LazyLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use strum::IntoEnumIterator;
use ui::IconName;

use language_model::ApiKeyState;
use omega_effectd::HostedSessionBlocker;

const PROVIDER_ID: LanguageModelProviderId = GOOGLE_PROVIDER_ID;
const PROVIDER_NAME: LanguageModelProviderName = GOOGLE_PROVIDER_NAME;
const HOSTED_GRANT_PATH: &str = "/api/provider-accounts/google-gemini/grants/builtin";

#[derive(Default, Clone, Debug, PartialEq)]
pub struct GoogleSettings {
    pub api_url: String,
    pub available_models: Vec<AvailableModel>,
    pub custom_headers: CustomHeaders,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ModelMode {
    #[default]
    Default,
    Thinking {
        /// The maximum number of tokens to use for reasoning. Must be lower than the model's `max_output_tokens`.
        budget_tokens: Option<u32>,
    },
}

pub struct GoogleLanguageModelProvider {
    http_client: Arc<dyn HttpClient>,
    state: Entity<State>,
    hosted_grant: Arc<async_lock::Mutex<Option<HostedGrant>>>,
}

pub struct State {
    api_key_state: ApiKeyState,
    credentials_provider: Arc<dyn CredentialsProvider>,
}

const GEMINI_API_KEY_VAR_NAME: &str = "GEMINI_API_KEY";
const GOOGLE_AI_API_KEY_VAR_NAME: &str = "GOOGLE_AI_API_KEY";

static API_KEY_ENV_VAR: LazyLock<EnvVar> = LazyLock::new(|| {
    // Try GEMINI_API_KEY first as primary, fallback to GOOGLE_AI_API_KEY
    EnvVar::new(GEMINI_API_KEY_VAR_NAME.into()).or(EnvVar::new(GOOGLE_AI_API_KEY_VAR_NAME.into()))
});

impl State {
    fn is_authenticated(&self) -> bool {
        omega_zero_base::is_active() || self.api_key_state.has_key()
    }

    fn set_api_key(&mut self, api_key: Option<String>, cx: &mut Context<Self>) -> Task<Result<()>> {
        let credentials_provider = self.credentials_provider.clone();
        let api_url = GoogleLanguageModelProvider::api_url(cx);
        self.api_key_state.store(
            api_url,
            api_key,
            |this| &mut this.api_key_state,
            credentials_provider,
            cx,
        )
    }

    fn authenticate(&mut self, cx: &mut Context<Self>) -> Task<Result<(), AuthenticateError>> {
        if omega_zero_base::is_active() {
            return Task::ready(Ok(()));
        }
        let credentials_provider = self.credentials_provider.clone();
        let api_url = GoogleLanguageModelProvider::api_url(cx);
        self.api_key_state.load_if_needed(
            api_url,
            |this| &mut this.api_key_state,
            credentials_provider,
            cx,
        )
    }
}

impl GoogleLanguageModelProvider {
    pub fn new(
        http_client: Arc<dyn HttpClient>,
        credentials_provider: Arc<dyn CredentialsProvider>,
        cx: &mut App,
    ) -> Self {
        let state = cx.new(|cx| {
            cx.observe_global::<SettingsStore>(|this: &mut State, cx| {
                let credentials_provider = this.credentials_provider.clone();
                let api_url = Self::api_url(cx);
                this.api_key_state.handle_url_change(
                    api_url,
                    |this| &mut this.api_key_state,
                    credentials_provider,
                    cx,
                );
                cx.notify();
            })
            .detach();
            State {
                api_key_state: ApiKeyState::new(Self::api_url(cx), (*API_KEY_ENV_VAR).clone()),
                credentials_provider,
            }
        });

        Self {
            http_client,
            state,
            hosted_grant: Arc::default(),
        }
    }

    fn create_language_model(&self, model: google_ai::Model) -> Arc<dyn LanguageModel> {
        Arc::new(GoogleLanguageModel {
            id: LanguageModelId::from(model.id().to_string()),
            model,
            state: self.state.clone(),
            http_client: self.http_client.clone(),
            hosted_grant: self.hosted_grant.clone(),
            request_limiter: RateLimiter::new(4),
        })
    }

    fn settings(cx: &App) -> &GoogleSettings {
        &crate::AllLanguageModelSettings::get_global(cx).google
    }

    fn api_url(cx: &App) -> SharedString {
        let api_url = &Self::settings(cx).api_url;
        if api_url.is_empty() {
            google_ai::API_URL.into()
        } else {
            SharedString::new(api_url.as_str())
        }
    }
}

impl LanguageModelProviderState for GoogleLanguageModelProvider {
    type ObservableEntity = State;

    fn observable_entity(&self) -> Option<Entity<Self::ObservableEntity>> {
        Some(self.state.clone())
    }
}

impl LanguageModelProvider for GoogleLanguageModelProvider {
    fn id(&self) -> LanguageModelProviderId {
        PROVIDER_ID
    }

    fn name(&self) -> LanguageModelProviderName {
        PROVIDER_NAME
    }

    fn icon(&self) -> IconOrSvg {
        IconOrSvg::Icon(IconName::AiGoogle)
    }

    fn default_model(&self, _cx: &App) -> Option<Arc<dyn LanguageModel>> {
        Some(self.create_language_model(google_ai::Model::default()))
    }

    fn default_fast_model(&self, _cx: &App) -> Option<Arc<dyn LanguageModel>> {
        Some(self.create_language_model(google_ai::Model::default_fast()))
    }

    fn provided_models(&self, cx: &App) -> Vec<Arc<dyn LanguageModel>> {
        let mut models = BTreeMap::default();

        // Add base models from google_ai::Model::iter()
        for model in google_ai::Model::iter() {
            if !matches!(model, google_ai::Model::Custom { .. }) {
                models.insert(model.id().to_string(), model);
            }
        }

        // Override with available models from settings
        for model in &GoogleLanguageModelProvider::settings(cx).available_models {
            models.insert(
                model.name.clone(),
                google_ai::Model::Custom {
                    name: model.name.clone(),
                    display_name: model.display_name.clone(),
                    max_tokens: model.max_tokens,
                    mode: model.mode.unwrap_or_default(),
                },
            );
        }

        models
            .into_values()
            .map(|model| {
                Arc::new(GoogleLanguageModel {
                    id: LanguageModelId::from(model.id().to_string()),
                    model,
                    state: self.state.clone(),
                    http_client: self.http_client.clone(),
                    hosted_grant: self.hosted_grant.clone(),
                    request_limiter: RateLimiter::new(4),
                }) as Arc<dyn LanguageModel>
            })
            .collect()
    }

    fn is_authenticated(&self, cx: &App) -> bool {
        self.state.read(cx).is_authenticated()
    }

    fn authenticate(&self, cx: &mut App) -> Task<Result<(), AuthenticateError>> {
        self.state.update(cx, |state, cx| state.authenticate(cx))
    }

    fn settings_view(&self, cx: &mut App) -> Option<ProviderSettingsView> {
        let state = self.state.read(cx);
        Some(ProviderSettingsView::ApiKey(ApiKeyConfiguration::new(
            state.api_key_state.has_key(),
            state.api_key_state.is_from_env_var(),
            state.api_key_state.env_var_name().clone(),
            "https://aistudio.google.com/app/apikey".into(),
        )))
    }

    fn set_api_key(&self, api_key: Option<String>, cx: &mut App) -> Task<Result<()>> {
        self.state
            .update(cx, |state, cx| state.set_api_key(api_key, cx))
    }
}

pub struct GoogleLanguageModel {
    id: LanguageModelId,
    model: google_ai::Model,
    state: Entity<State>,
    http_client: Arc<dyn HttpClient>,
    hosted_grant: Arc<async_lock::Mutex<Option<HostedGrant>>>,
    request_limiter: RateLimiter,
}

impl GoogleLanguageModel {
    fn stream_completion(
        &self,
        request: google_ai::GenerateContentRequest,
        cx: &AsyncApp,
    ) -> BoxFuture<
        'static,
        std::result::Result<
            futures::stream::BoxStream<'static, Result<GenerateContentResponse>>,
            LanguageModelCompletionError,
        >,
    > {
        let http_client = self.http_client.clone();
        let hosted_grant = self.hosted_grant.clone();
        let hosted_mode = omega_zero_base::is_active();
        let hosted_session_task = if hosted_mode {
            // A process without the session global (proof harnesses, tests)
            // has no hosted lane at all; the direct-key fallback below is the
            // only path there. The shipped binary always initializes it.
            let session = cx.update(|cx| omega_effectd::openagents_session_if_initialized(cx));
            session.map(|session| {
                cx.spawn(async move |cx| {
                    if let Some(verified) = session.resolve_verified(cx).await {
                        return (Some(verified), None);
                    }
                    if session.connect(cx).await == omega_effectd::OpenAgentsSessionPhase::Ready {
                        let verified = session.resolve_verified(cx).await;
                        let blocker = verified.is_none().then(|| session.blocker()).flatten();
                        (verified, blocker)
                    } else {
                        (None, session.blocker())
                    }
                })
            })
        } else {
            None
        };

        let (api_key, api_url, extra_headers) = self.state.read_with(cx, |state, cx| {
            let api_url = GoogleLanguageModelProvider::api_url(cx);
            let extra_headers = GoogleLanguageModelProvider::settings(cx)
                .custom_headers
                .clone();
            // Zero base returns early from `authenticate`, because the hosted
            // broker signs the owner in rather than a stored key. That leaves
            // `api_key_state` empty, which made the direct fallback below
            // unreachable: with the broker unavailable the owner could not
            // send a message at all, even with `GEMINI_API_KEY` exported.
            // Read the environment directly so the fallback is real.
            let api_key = state.api_key_state.key(&api_url).or_else(|| {
                API_KEY_ENV_VAR
                    .value
                    .as_deref()
                    .map(std::sync::Arc::<str>::from)
            });
            (api_key, api_url, extra_headers)
        });

        let hosted_lane_exists = hosted_session_task.is_some();
        async move {
            let mut hosted_blocker = None;
            if let Some(hosted_session_task) = hosted_session_task {
                let (session, blocker) = hosted_session_task.await;
                hosted_blocker = blocker;
                if let Some(session) = session {
                    let grant_ref = request_hosted_grant(
                        http_client.as_ref(),
                        &session.base_url,
                        &session.access_token,
                        &hosted_grant,
                    )
                    .await?;
                    return google_ai::stream_generate_content_with_bearer(
                        http_client.as_ref(),
                        &session.base_url,
                        &session.access_token,
                        &grant_ref,
                        request,
                        &extra_headers,
                    )
                    .await
                    .context("failed to stream hosted completion")
                    .map_err(LanguageModelCompletionError::Other);
                }
            }

            if let Some(api_key) = api_key {
                return google_ai::stream_generate_content(
                    http_client.as_ref(),
                    &api_url,
                    &api_key,
                    request,
                    &extra_headers,
                )
                .await
                .context("failed to stream completion")
                .map_err(LanguageModelCompletionError::Other);
            }

            if !hosted_mode || !hosted_lane_exists {
                // No hosted lane in this process (proof harnesses and tests
                // never initialize the session global), and no direct key:
                // the terminal no-credentials answer, never a sign-in retry.
                return Err(LanguageModelCompletionError::NoApiKey {
                    provider: PROVIDER_NAME,
                });
            }

            Err(hosted_sign_in_completion_error(hosted_blocker.as_ref()))
        }
        .boxed()
    }
}

/// The typed completion error for a hosted lane that could not sign in.
///
/// omega#170. A non-retryable blocker used to be flattened into
/// `LanguageModelCompletionError::Other`, and `Other` is in the generic
/// retry bucket — so the callout said "Retrying will not change the answer"
/// and then immediately scheduled "Attempt 1 of 2". `is_retryable()` is the
/// single authority: a terminal refusal becomes a `PermissionError`, which
/// the turn's retry policy never retries and whose exact message still
/// reaches the callout.
fn hosted_sign_in_completion_error(
    blocker: Option<&HostedSessionBlocker>,
) -> LanguageModelCompletionError {
    match blocker {
        Some(blocker) if !blocker.is_retryable() => LanguageModelCompletionError::PermissionError {
            provider: PROVIDER_NAME,
            message: hosted_sign_in_failure_message(Some(blocker)),
        },
        blocker => LanguageModelCompletionError::Other(anyhow::anyhow!(
            "{}",
            hosted_sign_in_failure_message(blocker)
        )),
    }
}

/// Say what actually stopped the hosted lane, and offer only advice that can
/// work.
///
/// The message this replaces claimed sign-in "was not completed" and told the
/// person to send the message again. For the failure the owner actually hit —
/// a signing identity the hosted service refuses, and before that an install
/// with no identity at all — both halves were false, and the log said nothing,
/// so the only diagnosis available was guesswork.
fn hosted_sign_in_failure_message(blocker: Option<&HostedSessionBlocker>) -> String {
    // Owner direction 2026-07-30: a hosted-auth failure is handled by the
    // turn's automatic provider fallback, so this line stays short and never
    // offers "Set GEMINI_API_KEY" as the primary remedy. A direct key remains
    // an optional extra lane, mentioned only as such.
    let Some(blocker) = blocker else {
        return "Hosted Gemini sign-in failed and reported no reason. \
             Omega falls back to another model automatically."
            .to_string();
    };
    format!(
        "{} Omega falls back to another model automatically; \
         a {GEMINI_API_KEY_VAR_NAME} key optionally enables the direct Gemini lane.",
        blocker.summary()
    )
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostedGrant {
    grant_ref: String,
    expires_at: u64,
}

#[derive(Deserialize)]
struct HostedGrantResponse {
    grant: HostedGrant,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostedGrantErrorResponse {
    resets_at: Option<String>,
}

fn hosted_grant_error(status: StatusCode, body: &str) -> LanguageModelCompletionError {
    if status == StatusCode::TOO_MANY_REQUESTS {
        let resets_at = serde_json::from_str::<HostedGrantErrorResponse>(body)
            .ok()
            .and_then(|response| response.resets_at);
        let message = match resets_at {
            Some(resets_at) => {
                format!("Hosted Omega's daily usage limit was reached. It resets at {resets_at}.")
            }
            None => "Hosted Omega's daily usage limit was reached.".to_string(),
        };
        return LanguageModelCompletionError::HostedUsageLimitExceeded { message };
    }

    LanguageModelCompletionError::Other(anyhow::anyhow!(
        "Hosted Omega is unavailable (HTTP {status})."
    ))
}

async fn request_hosted_grant(
    http_client: &dyn HttpClient,
    base_url: &str,
    bearer_token: &str,
    hosted_grant: &async_lock::Mutex<Option<HostedGrant>>,
) -> Result<String, LanguageModelCompletionError> {
    let mut cached_grant = hosted_grant.lock().await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64;
    if let Some(grant) = cached_grant.as_ref()
        && grant.expires_at > now.saturating_add(30_000)
    {
        return Ok(grant.grant_ref.clone());
    }

    let uri = format!("{}{}", base_url.trim_end_matches('/'), HOSTED_GRANT_PATH);
    let request = HttpRequest::builder()
        .method(Method::POST)
        .uri(uri)
        .header("Authorization", format!("Bearer {}", bearer_token.trim()))
        .header("Content-Type", "application/json")
        .body(AsyncBody::from("{}"))
        .context("failed to build hosted-compute grant request")
        .map_err(LanguageModelCompletionError::Other)?;
    let mut response = http_client
        .send(request)
        .await
        .context("failed to request hosted compute")
        .map_err(LanguageModelCompletionError::Other)?;
    let status = response.status();
    let mut body = String::new();
    response
        .body_mut()
        .take(64 * 1024)
        .read_to_string(&mut body)
        .await
        .context("failed to read hosted-compute grant response")
        .map_err(LanguageModelCompletionError::Other)?;

    if !status.is_success() {
        return Err(hosted_grant_error(status, &body));
    }

    let response: HostedGrantResponse = serde_json::from_str(&body)
        .context("hosted-compute grant response was invalid")
        .map_err(LanguageModelCompletionError::Other)?;
    if response.grant.grant_ref.trim().is_empty() {
        return Err(LanguageModelCompletionError::Other(anyhow::anyhow!(
            "Hosted Omega returned an empty grant."
        )));
    }
    let grant_ref = response.grant.grant_ref.clone();
    *cached_grant = Some(response.grant);
    Ok(grant_ref)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_allowance_error_is_not_a_google_rate_limit() {
        let error = hosted_grant_error(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":"builtin_agent_quota_exhausted","resetsAt":"2026-07-28T00:00:00.000Z"}"#,
        );

        assert!(matches!(
            &error,
            LanguageModelCompletionError::HostedUsageLimitExceeded { .. }
        ));
        assert_eq!(
            error.to_string(),
            "Hosted Omega's daily usage limit was reached. It resets at 2026-07-28T00:00:00.000Z."
        );
    }

    /// A refused proof is terminal. Telling the owner to send the message
    /// again is advice that can never work, and is what made this defect look
    /// like a transient network problem for days.
    #[test]
    fn a_refused_hosted_proof_is_not_reported_as_an_incomplete_sign_in() {
        let message = hosted_sign_in_failure_message(Some(&HostedSessionBlocker::ProofRejected {
            status: 401,
        }));

        assert!(message.contains("401"));
        assert!(message.contains("not admitted"));
        assert!(!message.contains("Send the message again"));
        assert!(!message.contains("sign-in was not completed"));
        assert!(message.contains(GEMINI_API_KEY_VAR_NAME));
    }

    /// omega#170. The message above says retrying is futile, so the error
    /// class must agree: a terminal refusal is a `PermissionError` (never
    /// retried), while a retryable blocker stays in the retried bucket.
    #[test]
    fn a_terminal_hosted_refusal_is_not_scheduled_for_retry() {
        let error = hosted_sign_in_completion_error(Some(&HostedSessionBlocker::ProofRejected {
            status: 401,
        }));
        match &error {
            LanguageModelCompletionError::PermissionError { message, .. } => {
                assert!(message.contains("not admitted"));
            }
            other => panic!(
                "a non-retryable blocker must map to PermissionError so the \
                 turn does not retry what its own copy calls futile, got {other:?}"
            ),
        }

        let retryable =
            hosted_sign_in_completion_error(Some(&HostedSessionBlocker::ServiceUnreachable));
        assert!(
            matches!(retryable, LanguageModelCompletionError::Other(_)),
            "a retryable blocker keeps the retried mapping, got {retryable:?}"
        );
    }

    #[test]
    fn an_unprovisioned_install_is_told_which_custody_state_blocked_it() {
        let message =
            hosted_sign_in_failure_message(Some(&HostedSessionBlocker::IdentityUnavailable {
                custody_state: "identity custody is unavailable in state Lost".to_string(),
            }));

        assert!(message.contains("Lost"));
        // omega#164 retired the onboarding ceremony; the blocker now points
        // at identity setup, which is where a Lost identity is resolved.
        assert!(message.contains("identity setup"));
        assert!(!message.contains("Send the message again"));
    }

    /// Owner direction 2026-07-30: the automatic provider fallback owns
    /// recovery, so no hosted failure copy pushes "Set GEMINI_API_KEY" or
    /// "send the message again" as the remedy.
    #[test]
    fn hosted_failure_copy_names_the_automatic_fallback_not_a_key() {
        let message =
            hosted_sign_in_failure_message(Some(&HostedSessionBlocker::ServiceUnreachable));

        assert!(message.contains("falls back to another model automatically"));
        assert!(!message.contains("Send the message again"));
        assert!(!message.starts_with("Set "));
    }
}

impl LanguageModel for GoogleLanguageModel {
    fn id(&self) -> LanguageModelId {
        self.id.clone()
    }

    fn name(&self) -> LanguageModelName {
        LanguageModelName::from(self.model.display_name().to_string())
    }

    fn provider_id(&self) -> LanguageModelProviderId {
        PROVIDER_ID
    }

    fn provider_name(&self) -> LanguageModelProviderName {
        PROVIDER_NAME
    }

    fn supports_tools(&self) -> bool {
        self.model.supports_tools()
    }

    fn supports_images(&self) -> bool {
        self.model.supports_images()
    }

    fn supports_thinking(&self) -> bool {
        self.model.supports_thinking()
    }

    fn supported_effort_levels(&self) -> Vec<LanguageModelEffortLevel> {
        let default_level = self.model.default_thinking_level();
        self.model
            .supported_thinking_levels()
            .iter()
            .map(|level| LanguageModelEffortLevel {
                name: level.name().into(),
                value: level.value().into(),
                is_default: Some(*level) == default_level,
            })
            .collect()
    }

    fn supports_tool_choice(&self, choice: LanguageModelToolChoice) -> bool {
        match choice {
            LanguageModelToolChoice::Auto
            | LanguageModelToolChoice::Any
            | LanguageModelToolChoice::None => true,
        }
    }

    fn tool_input_format(&self) -> LanguageModelToolSchemaFormat {
        LanguageModelToolSchemaFormat::JsonSchemaSubset
    }

    fn telemetry_id(&self) -> String {
        format!("google/{}", self.model.request_id())
    }

    fn max_token_count(&self) -> u64 {
        self.model.max_token_count()
    }

    fn max_output_tokens(&self) -> Option<u64> {
        self.model.max_output_tokens()
    }

    fn stream_completion(
        &self,
        request: LanguageModelRequest,
        cx: &AsyncApp,
    ) -> BoxFuture<
        'static,
        Result<
            futures::stream::BoxStream<
                'static,
                Result<LanguageModelCompletionEvent, LanguageModelCompletionError>,
            >,
            LanguageModelCompletionError,
        >,
    > {
        let request = match into_google(
            request,
            self.model.request_id().to_string(),
            self.model.mode(),
        ) {
            Ok(request) => request,
            Err(error) => return async move { Err(error.into()) }.boxed(),
        };
        let request = self.stream_completion(request, cx);
        let future = self.request_limiter.stream(async move {
            let response = request.await?;
            Ok(GoogleEventMapper::new().map_stream(response))
        });
        async move { Ok(future.await?.boxed()) }.boxed()
    }
}
