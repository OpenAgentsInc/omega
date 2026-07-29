//! OpenAgents hosted inference provider.
//!
//! Serves the hosted lanes from `openagents.com` that Omega selects through the
//! Flash / Pro tier control. Pro maps to Kimi K3 on Fireworks; Flash stays on
//! the Google provider's hosted Gemini path. Authentication reuses the verified
//! OpenAgents session from zero base — no local Fireworks or Gemini key is
//! required when that session is ready.

use anyhow::{Context as _, Result};
use futures::{FutureExt, StreamExt, future::BoxFuture};
use gpui::{App, AppContext, AsyncApp, Entity, SharedString, Task};
use http_client::{CustomHeaders, HttpClient};
use language_model::{
    AuthenticateError, IconOrSvg, LanguageModel, LanguageModelCompletionError,
    LanguageModelCompletionEvent, LanguageModelId, LanguageModelName, LanguageModelProvider,
    LanguageModelProviderId, LanguageModelProviderName, LanguageModelProviderState,
    LanguageModelRequest, LanguageModelToolChoice, ProviderSettingsView, RateLimiter,
};
use open_ai::{
    ResponseStreamEvent,
    completion::{ChatCompletionMaxTokensParameter, OpenAiEventMapper, into_open_ai},
    stream_completion,
};
use std::sync::Arc;
use ui::IconName;

const PROVIDER_ID: LanguageModelProviderId = LanguageModelProviderId::new("openagents");
const PROVIDER_NAME: LanguageModelProviderName = LanguageModelProviderName::new("OpenAgents");

/// Wire model id for the Pro tier (Fireworks Kimi K3).
pub const KIMI_K3_MODEL_ID: &str = "kimi-k3";

/// OpenAI-compatible chat completions path under the OpenAgents base URL.
const CHAT_COMPLETIONS_PREFIX: &str = "/api/v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostedModel {
    KimiK3,
}

impl HostedModel {
    fn id(self) -> &'static str {
        match self {
            Self::KimiK3 => KIMI_K3_MODEL_ID,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::KimiK3 => "Kimi K3",
        }
    }

    fn max_tokens(self) -> u64 {
        match self {
            Self::KimiK3 => 131_072,
        }
    }

    fn max_output_tokens(self) -> Option<u64> {
        match self {
            Self::KimiK3 => Some(16_384),
        }
    }

    fn all() -> &'static [Self] {
        &[Self::KimiK3]
    }
}

pub struct OpenAgentsLanguageModelProvider {
    http_client: Arc<dyn HttpClient>,
    state: Entity<State>,
}

pub struct State {
    /// True when zero base can authenticate via the OpenAgents session.
    ready: bool,
}

impl OpenAgentsLanguageModelProvider {
    pub fn new(http_client: Arc<dyn HttpClient>, cx: &mut App) -> Self {
        let state = cx.new(|_cx| State {
            ready: omega_zero_base::is_active(),
        });
        Self { http_client, state }
    }

    fn create_language_model(&self, model: HostedModel) -> Arc<dyn LanguageModel> {
        Arc::new(OpenAgentsLanguageModel {
            id: LanguageModelId::from(model.id().to_string()),
            model,
            state: self.state.clone(),
            http_client: self.http_client.clone(),
            request_limiter: RateLimiter::new(4),
        })
    }
}

impl LanguageModelProviderState for OpenAgentsLanguageModelProvider {
    type ObservableEntity = State;

    fn observable_entity(&self) -> Option<Entity<Self::ObservableEntity>> {
        Some(self.state.clone())
    }
}

impl LanguageModelProvider for OpenAgentsLanguageModelProvider {
    fn id(&self) -> LanguageModelProviderId {
        PROVIDER_ID
    }

    fn name(&self) -> LanguageModelProviderName {
        PROVIDER_NAME
    }

    fn icon(&self) -> IconOrSvg {
        IconOrSvg::Icon(IconName::OmegaAssistant)
    }

    fn default_model(&self, _cx: &App) -> Option<Arc<dyn LanguageModel>> {
        Some(self.create_language_model(HostedModel::KimiK3))
    }

    fn default_fast_model(&self, _cx: &App) -> Option<Arc<dyn LanguageModel>> {
        None
    }

    fn provided_models(&self, _cx: &App) -> Vec<Arc<dyn LanguageModel>> {
        HostedModel::all()
            .iter()
            .copied()
            .map(|model| self.create_language_model(model))
            .collect()
    }

    fn is_authenticated(&self, cx: &App) -> bool {
        // Zero base authenticates through the OpenAgents session at request
        // time. Outside zero base this provider is still listed so Pro can be
        // selected, and the stream path will try the session then fail closed.
        omega_zero_base::is_active() || self.state.read(cx).ready
    }

    fn authenticate(&self, cx: &mut App) -> Task<Result<(), AuthenticateError>> {
        self.state.update(cx, |state, cx| {
            state.ready = true;
            cx.notify();
        });
        // Real session resolution happens on the first stream; a hard connect
        // here would block provider registration on network.
        Task::ready(Ok(()))
    }

    fn settings_view(&self, _cx: &mut App) -> Option<ProviderSettingsView> {
        // Hosted compute; no local API key form.
        None
    }

    fn authentication_error_message(&self) -> SharedString {
        "OpenAgents hosted Pro needs a signed-in OpenAgents account. \
         Sign in from the agent panel, then try again."
            .into()
    }

    fn missing_credentials_error_message(&self) -> SharedString {
        "OpenAgents hosted Pro needs a signed-in OpenAgents account. \
         Sign in from the agent panel, then try again."
            .into()
    }
}

struct OpenAgentsLanguageModel {
    id: LanguageModelId,
    model: HostedModel,
    #[allow(dead_code)]
    state: Entity<State>,
    http_client: Arc<dyn HttpClient>,
    request_limiter: RateLimiter,
}

impl OpenAgentsLanguageModel {
    fn stream_open_ai(
        &self,
        request: open_ai::Request,
        cx: &AsyncApp,
    ) -> BoxFuture<
        'static,
        Result<
            futures::stream::BoxStream<'static, Result<ResponseStreamEvent>>,
            LanguageModelCompletionError,
        >,
    > {
        let http_client = self.http_client.clone();
        let hosted_mode = omega_zero_base::is_active();
        let session_task = if hosted_mode {
            let session = cx.update(|cx| omega_effectd::openagents_session(cx));
            Some(cx.spawn(async move |cx| {
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
            }))
        } else {
            None
        };

        let future = self.request_limiter.stream(async move {
            let mut hosted_blocker = None;
            if let Some(session_task) = session_task {
                let (session, blocker) = session_task.await;
                hosted_blocker = blocker;
                if let Some(session) = session {
                    let api_url = format!(
                        "{}{}",
                        session.base_url.trim_end_matches('/'),
                        CHAT_COMPLETIONS_PREFIX
                    );
                    let response = stream_completion(
                        http_client.as_ref(),
                        PROVIDER_NAME.0.as_str(),
                        &api_url,
                        &session.access_token,
                        request,
                        &CustomHeaders::default(),
                    )
                    .await
                    .context("failed to stream OpenAgents hosted completion")
                    .map_err(LanguageModelCompletionError::Other)?;
                    return Ok(response);
                }
            }

            if !hosted_mode {
                return Err(LanguageModelCompletionError::NoApiKey {
                    provider: PROVIDER_NAME,
                });
            }

            Err(LanguageModelCompletionError::Other(anyhow::anyhow!(
                "{}",
                hosted_sign_in_failure_message(hosted_blocker.as_ref())
            )))
        });

        async move { Ok(future.await?.boxed()) }.boxed()
    }
}

fn hosted_sign_in_failure_message(blocker: Option<&omega_effectd::HostedSessionBlocker>) -> String {
    match blocker {
        Some(blocker) => format!(
            "OpenAgents hosted Pro (Kimi K3) needs a signed-in account. {}",
            blocker.summary()
        ),
        None => "OpenAgents hosted Pro (Kimi K3) needs a signed-in OpenAgents account.".to_string(),
    }
}

impl LanguageModel for OpenAgentsLanguageModel {
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
        true
    }

    fn supports_images(&self) -> bool {
        true
    }

    fn supports_tool_choice(&self, choice: LanguageModelToolChoice) -> bool {
        match choice {
            LanguageModelToolChoice::Auto
            | LanguageModelToolChoice::Any
            | LanguageModelToolChoice::None => true,
        }
    }

    fn supports_streaming_tools(&self) -> bool {
        true
    }

    fn telemetry_id(&self) -> String {
        format!("openagents/{}", self.model.id())
    }

    fn max_token_count(&self) -> u64 {
        self.model.max_tokens()
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
        let request = match into_open_ai(
            request,
            self.model.id(),
            true,
            false,
            self.max_output_tokens(),
            ChatCompletionMaxTokensParameter::MaxCompletionTokens,
            None,
            false,
        ) {
            Ok(request) => request,
            Err(error) => return async move { Err(error.into()) }.boxed(),
        };
        let completions = self.stream_open_ai(request, cx);
        async move {
            let mapper = OpenAiEventMapper::new();
            Ok(mapper.map_stream(completions.await?).boxed())
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pro_model_id_is_kimi_k3() {
        assert_eq!(HostedModel::KimiK3.id(), "kimi-k3");
        assert_eq!(KIMI_K3_MODEL_ID, "kimi-k3");
    }

    #[test]
    fn provider_id_is_openagents() {
        assert_eq!(PROVIDER_ID.0.as_ref(), "openagents");
    }
}
