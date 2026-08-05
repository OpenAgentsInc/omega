//! OpenAgents hosted inference provider.
//!
//! Serves the hosted lanes from `openagents.com` that Omega selects through the
//! model tier control. The GPT-5.6 family is served through the hosted
//! OpenAgents passthrough lane. Pro maps to Kimi K3 on Fireworks; Flash stays
//! on the Google provider's hosted Gemini path.
//! Authentication reuses the verified OpenAgents session from zero base — no
//! local OpenAI, Fireworks, or Gemini key is required when that session is
//! ready.

use anyhow::{Context as _, Result};
use futures::{FutureExt, StreamExt, future::BoxFuture};
use gpui::{App, AppContext, AsyncApp, Entity, SharedString, Task};
use http_client::{CustomHeaders, HttpClient};
use language_model::{
    AuthenticateError, IconOrSvg, LanguageModel, LanguageModelCompletionError,
    LanguageModelCompletionEvent, LanguageModelEffortLevel, LanguageModelId, LanguageModelName,
    LanguageModelProvider, LanguageModelProviderId, LanguageModelProviderName,
    LanguageModelProviderState, LanguageModelRequest, LanguageModelToolChoice,
    ProviderSettingsView, RateLimiter,
};
use open_ai::{
    ReasoningEffort, ResponseStreamEvent,
    completion::{ChatCompletionMaxTokensParameter, OpenAiEventMapper, into_open_ai},
    stream_completion,
};
use std::sync::Arc;
use ui::IconName;

const PROVIDER_ID: LanguageModelProviderId = LanguageModelProviderId::new("openagents");
const PROVIDER_NAME: LanguageModelProviderName = LanguageModelProviderName::new("OpenAgents");

/// Wire model id for the Pro tier (Fireworks Kimi K3).
pub const KIMI_K3_MODEL_ID: &str = "kimi-k3";

/// Wire model id for the primary tier (GPT-5.6 Luna over the hosted OpenAI
/// passthrough lane).
pub const GPT_56_LUNA_MODEL_ID: &str = "gpt-5.6-luna";

/// Wire model id for GPT-5.6 Terra over the hosted OpenAI passthrough lane.
pub const GPT_56_TERRA_MODEL_ID: &str = "gpt-5.6-terra";

/// Wire model id for GPT-5.6 Sol over the hosted OpenAI passthrough lane.
pub const GPT_56_SOL_MODEL_ID: &str = "gpt-5.6-sol";

/// Wire model id for the Flash tier over the hosted Gemini lane.
///
/// `OMEGA-DELTA-0202`. The Flash tier used to point at the direct Google
/// provider, which needs the person's own Google AI credential, so the middle
/// rung of the always-work chain reported "Gemini 3.6 Flash is unavailable" on
/// a machine whose hosted Gemini lane was healthy and serving other lanes on
/// the same session.
pub const GEMINI_36_FLASH_MODEL_ID: &str = "gemini-3.6-flash";

/// OpenAI-compatible chat completions path under the OpenAgents base URL.
const CHAT_COMPLETIONS_PREFIX: &str = "/api/v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostedModel {
    Gpt56Luna,
    Gpt56Terra,
    Gpt56Sol,
    Gemini36Flash,
    KimiK3,
}

impl HostedModel {
    fn id(self) -> &'static str {
        match self {
            Self::Gpt56Luna => GPT_56_LUNA_MODEL_ID,
            Self::Gpt56Terra => GPT_56_TERRA_MODEL_ID,
            Self::Gpt56Sol => GPT_56_SOL_MODEL_ID,
            Self::Gemini36Flash => GEMINI_36_FLASH_MODEL_ID,
            Self::KimiK3 => KIMI_K3_MODEL_ID,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Gpt56Luna => "GPT-5.6 Luna",
            Self::Gpt56Terra => "GPT-5.6 Terra",
            Self::Gpt56Sol => "GPT-5.6 Sol",
            Self::Gemini36Flash => "Gemini 3.6 Flash",
            Self::KimiK3 => "Kimi K3",
        }
    }

    fn max_tokens(self) -> u64 {
        match self {
            // Matches the corresponding open_ai::Model GPT-5.6 variants.
            Self::Gpt56Luna | Self::Gpt56Terra | Self::Gpt56Sol => 1_050_000,
            // Matches google_ai::Model::Gemini36Flash.
            Self::Gemini36Flash => 1_048_576,
            Self::KimiK3 => 131_072,
        }
    }

    fn max_output_tokens(self) -> Option<u64> {
        match self {
            Self::Gpt56Luna | Self::Gpt56Terra | Self::Gpt56Sol => Some(128_000),
            Self::Gemini36Flash => Some(65_536),
            Self::KimiK3 => Some(16_384),
        }
    }

    fn all() -> &'static [Self] {
        &[
            Self::Gpt56Luna,
            Self::Gpt56Terra,
            Self::Gpt56Sol,
            Self::Gemini36Flash,
            Self::KimiK3,
        ]
    }

    fn default_reasoning_effort(self) -> Option<ReasoningEffort> {
        match self {
            Self::Gpt56Sol => Some(ReasoningEffort::Low),
            Self::Gpt56Luna | Self::Gpt56Terra => Some(ReasoningEffort::Medium),
            Self::Gemini36Flash | Self::KimiK3 => None,
        }
    }

    fn supported_reasoning_efforts(self) -> &'static [ReasoningEffort] {
        match self {
            Self::Gpt56Luna | Self::Gpt56Terra | Self::Gpt56Sol => &[
                ReasoningEffort::None,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::XHigh,
                ReasoningEffort::Max,
            ],
            Self::Gemini36Flash | Self::KimiK3 => &[],
        }
    }
}

fn reasoning_effort_for_request(
    request: &LanguageModelRequest,
    model: HostedModel,
) -> Option<ReasoningEffort> {
    let supported_efforts = model.supported_reasoning_efforts();
    if supported_efforts.is_empty() {
        return None;
    }

    if request.thinking_allowed {
        request
            .thinking_effort
            .as_deref()
            .and_then(|effort| effort.parse::<ReasoningEffort>().ok())
            .filter(|effort| supported_efforts.contains(effort))
            .filter(|effort| *effort != ReasoningEffort::None)
            .or_else(|| model.default_reasoning_effort())
    } else if supported_efforts.contains(&ReasoningEffort::None) {
        Some(ReasoningEffort::None)
    } else {
        None
    }
}

fn supported_thinking_effort_levels(model: HostedModel) -> Vec<LanguageModelEffortLevel> {
    let default_effort = model.default_reasoning_effort();
    model
        .supported_reasoning_efforts()
        .iter()
        .copied()
        .filter_map(|effort| {
            let (name, value) = match effort {
                ReasoningEffort::None => return None,
                ReasoningEffort::Minimal => ("Minimal", "minimal"),
                ReasoningEffort::Low => ("Low", "low"),
                ReasoningEffort::Medium => ("Medium", "medium"),
                ReasoningEffort::High => ("High", "high"),
                ReasoningEffort::XHigh => ("Extra High", "xhigh"),
                ReasoningEffort::Max => ("Max", "max"),
            };

            Some(LanguageModelEffortLevel {
                name: name.into(),
                value: value.into(),
                is_default: Some(effort) == default_effort,
            })
        })
        .collect()
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
        // Owner direction 2026-07-30: the core Omega Agent defaults to
        // GPT-5.6 Luna through the hosted OpenAgents lane.
        Some(self.create_language_model(HostedModel::Gpt56Luna))
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

            // omega#170. A non-retryable blocker (a rejected sign-in proof)
            // must not land in `Other`, whose generic retry bucket schedules
            // "Attempt 1 of 2" under a message that says retrying is futile.
            // `is_retryable()` is the single authority; the terminal case
            // becomes a `PermissionError`, which is never retried and keeps
            // the exact reason in the callout.
            Err(match hosted_blocker.as_ref() {
                Some(blocker) if !blocker.is_retryable() => {
                    LanguageModelCompletionError::PermissionError {
                        provider: PROVIDER_NAME,
                        message: hosted_sign_in_failure_message(Some(blocker)),
                    }
                }
                blocker => LanguageModelCompletionError::Other(anyhow::anyhow!(
                    "{}",
                    hosted_sign_in_failure_message(blocker)
                )),
            })
        });

        async move { Ok(future.await?.boxed()) }.boxed()
    }
}

fn hosted_sign_in_failure_message(blocker: Option<&omega_effectd::HostedSessionBlocker>) -> String {
    match blocker {
        Some(blocker) => format!("OpenAgents hosted sign-in failed. {}", blocker.summary()),
        None => "OpenAgents hosted sign-in is not ready.".to_string(),
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

    fn supports_thinking(&self) -> bool {
        !self.model.supported_reasoning_efforts().is_empty()
    }

    fn supported_effort_levels(&self) -> Vec<LanguageModelEffortLevel> {
        supported_thinking_effort_levels(self.model)
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
        let reasoning_effort = reasoning_effort_for_request(&request, self.model);
        let request = match into_open_ai(
            request,
            self.model.id(),
            true,
            false,
            self.max_output_tokens(),
            ChatCompletionMaxTokensParameter::MaxCompletionTokens,
            reasoning_effort,
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

    /// Owner direction 2026-07-30: the core Omega Agent defaults to
    /// GPT-5.6 Luna over the hosted OpenAgents lane; Gemini is the backup.
    #[test]
    fn primary_hosted_model_is_gpt_56_luna() {
        assert_eq!(HostedModel::Gpt56Luna.id(), "gpt-5.6-luna");
        assert_eq!(GPT_56_LUNA_MODEL_ID, "gpt-5.6-luna");
        assert_eq!(HostedModel::all().first(), Some(&HostedModel::Gpt56Luna));
    }

    #[test]
    fn hosted_gpt_56_family_has_wire_ids_and_reasoning_efforts() {
        assert_eq!(HostedModel::Gpt56Terra.id(), "gpt-5.6-terra");
        assert_eq!(HostedModel::Gpt56Sol.id(), "gpt-5.6-sol");
        assert_eq!(GPT_56_TERRA_MODEL_ID, "gpt-5.6-terra");
        assert_eq!(GPT_56_SOL_MODEL_ID, "gpt-5.6-sol");
        assert!(HostedModel::all().contains(&HostedModel::Gpt56Terra));
        assert!(HostedModel::all().contains(&HostedModel::Gpt56Sol));

        for model in [
            HostedModel::Gpt56Luna,
            HostedModel::Gpt56Terra,
            HostedModel::Gpt56Sol,
        ] {
            assert_eq!(
                model.supported_reasoning_efforts(),
                &[
                    ReasoningEffort::None,
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                    ReasoningEffort::Max,
                ]
            );
        }
    }

    #[test]
    fn hosted_gpt_56_reasoning_efforts_are_selectable() {
        let effort_levels = supported_thinking_effort_levels(HostedModel::Gpt56Sol);
        let values = effort_levels
            .iter()
            .map(|level| level.value.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(values, ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(
            effort_levels
                .iter()
                .find(|level| level.is_default)
                .map(|level| level.value.as_ref()),
            Some("low")
        );
    }

    #[test]
    fn hosted_gpt_56_request_uses_selected_or_disabled_reasoning_effort() {
        let selected = LanguageModelRequest {
            thinking_allowed: true,
            thinking_effort: Some("xhigh".to_string()),
            ..Default::default()
        };
        assert_eq!(
            reasoning_effort_for_request(&selected, HostedModel::Gpt56Terra),
            Some(ReasoningEffort::XHigh)
        );

        let disabled = LanguageModelRequest {
            thinking_allowed: false,
            ..Default::default()
        };
        assert_eq!(
            reasoning_effort_for_request(&disabled, HostedModel::Gpt56Luna),
            Some(ReasoningEffort::None)
        );

        let reasoning_effort = reasoning_effort_for_request(&selected, HostedModel::Gpt56Terra);
        let Ok(open_ai_request) = into_open_ai(
            selected,
            HostedModel::Gpt56Terra.id(),
            true,
            false,
            HostedModel::Gpt56Terra.max_output_tokens(),
            ChatCompletionMaxTokensParameter::MaxCompletionTokens,
            reasoning_effort,
            false,
        ) else {
            panic!("the hosted GPT-5.6 request should convert to OpenAI format");
        };
        assert_eq!(
            open_ai_request.reasoning_effort,
            Some(ReasoningEffort::XHigh)
        );
    }

    /// `OMEGA-DELTA-0202`. Flash is served by the hosted lane, so the middle
    /// rung of the always-work chain needs no credential of the person's own.
    #[test]
    fn flash_is_served_by_the_hosted_lane() {
        assert_eq!(HostedModel::Gemini36Flash.id(), "gemini-3.6-flash");
        assert_eq!(GEMINI_36_FLASH_MODEL_ID, "gemini-3.6-flash");
        assert_eq!(
            HostedModel::Gemini36Flash.display_name(),
            "Gemini 3.6 Flash"
        );
        assert!(HostedModel::all().contains(&HostedModel::Gemini36Flash));
    }

    #[test]
    fn provider_id_is_openagents() {
        assert_eq!(PROVIDER_ID.0.as_ref(), "openagents");
    }
}
