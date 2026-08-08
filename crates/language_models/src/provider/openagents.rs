//! Omega Agent cloud inference provider.
//!
//! Omega exposes one logical agent to the client. The OpenAgents API owns the
//! provider model and routing decisions, while Omega keeps its native thread
//! and tool loop. Each request uses the active Omega Nostr identity.

use anyhow::Result;
use async_tungstenite::tungstenite::{Message as WebSocketMessage, client::IntoClientRequest};
use futures::{FutureExt, Stream, StreamExt, future::BoxFuture, stream};
use gpui::{App, AppContext, AsyncApp, Entity, SharedString, Task};
use http_client::{CustomHeaders, HttpClient};
use language_model::{
    AuthenticateError, IconOrSvg, LanguageModel, LanguageModelCompletionError,
    LanguageModelCompletionEvent, LanguageModelId, LanguageModelName, LanguageModelProvider,
    LanguageModelProviderId, LanguageModelProviderName, LanguageModelProviderState,
    LanguageModelRequest, LanguageModelToolChoice, ProviderSettingsView, RateLimiter,
};
use open_ai::{
    completion::{OpenAiResponseEventMapper, into_open_ai_response},
    responses::{
        Request as ResponseRequest, StreamEvent as ResponsesStreamEvent,
        serialize_response_request, stream_response_with_authorization,
    },
};
use settings::Settings as _;
use std::sync::Arc;
use ui::IconName;

const PROVIDER_ID: LanguageModelProviderId = LanguageModelProviderId::new("openagents");
const PROVIDER_NAME: LanguageModelProviderName = LanguageModelProviderName::new("OpenAgents");

pub const OMEGA_AGENT_MODEL_ID: &str = "omega-agent";
pub const DEVELOPMENT_API_URL: &str = "http://127.0.0.1:8080/v1";
pub const DEVELOPMENT_WEBSOCKET_URL: &str = "ws://127.0.0.1:8080/v1/responses";
pub const PRODUCTION_API_URL: &str = "https://api.openagents.com/v1";

const MAX_TOKENS: u64 = 1_050_000;
const MAX_OUTPUT_TOKENS: u64 = 128_000;

#[derive(Default, Clone, Debug, PartialEq)]
pub struct OpenAgentsSettings {
    pub use_development_api: bool,
}

impl OpenAgentsSettings {
    pub fn api_url(&self) -> &'static str {
        if self.use_development_api {
            DEVELOPMENT_API_URL
        } else {
            PRODUCTION_API_URL
        }
    }

    pub fn authentication_url(&self) -> &'static str {
        PRODUCTION_API_URL
    }

    pub fn websocket_url(&self) -> Option<&'static str> {
        self.use_development_api
            .then_some(DEVELOPMENT_WEBSOCKET_URL)
    }
}

pub struct OpenAgentsLanguageModelProvider {
    http_client: Arc<dyn HttpClient>,
    state: Entity<State>,
}

pub struct State {
    ready: bool,
}

impl OpenAgentsLanguageModelProvider {
    pub fn new(http_client: Arc<dyn HttpClient>, cx: &mut App) -> Self {
        let state = cx.new(|_cx| State {
            ready: omega_zero_base::is_active(),
        });
        Self { http_client, state }
    }

    fn create_language_model(&self) -> Arc<dyn LanguageModel> {
        Arc::new(OpenAgentsLanguageModel {
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
        Some(self.create_language_model())
    }

    fn default_fast_model(&self, _cx: &App) -> Option<Arc<dyn LanguageModel>> {
        None
    }

    fn provided_models(&self, _cx: &App) -> Vec<Arc<dyn LanguageModel>> {
        vec![self.create_language_model()]
    }

    fn is_authenticated(&self, cx: &App) -> bool {
        omega_zero_base::is_active() || self.state.read(cx).ready
    }

    fn authenticate(&self, cx: &mut App) -> Task<Result<(), AuthenticateError>> {
        self.state.update(cx, |state, cx| {
            state.ready = true;
            cx.notify();
        });
        Task::ready(Ok(()))
    }

    fn settings_view(&self, _cx: &mut App) -> Option<ProviderSettingsView> {
        None
    }

    fn authentication_error_message(&self) -> SharedString {
        "Omega could not sign this request with your Nostr identity.".into()
    }

    fn missing_credentials_error_message(&self) -> SharedString {
        self.authentication_error_message()
    }
}

struct OpenAgentsLanguageModel {
    #[allow(dead_code)]
    state: Entity<State>,
    http_client: Arc<dyn HttpClient>,
    request_limiter: RateLimiter,
}

impl OpenAgentsLanguageModel {
    fn stream_responses(
        &self,
        request: ResponseRequest,
        cx: &AsyncApp,
    ) -> BoxFuture<
        'static,
        Result<
            futures::stream::BoxStream<'static, Result<ResponsesStreamEvent>>,
            LanguageModelCompletionError,
        >,
    > {
        let http_client = self.http_client.clone();
        let (api_url, authentication_url) = cx.update(|cx| {
            let settings = &crate::settings::AllLanguageModelSettings::get_global(cx).openagents;
            (
                settings.api_url().to_owned(),
                settings.authentication_url().to_owned(),
            )
        });

        let future = self.request_limiter.stream(async move {
            let is_streaming = request.stream;
            let body =
                serialize_response_request(&request).map_err(LanguageModelCompletionError::from)?;
            let signed_url = format!("{authentication_url}/responses");
            let authorization =
                omega_effectd::sign_nip98_request(&signed_url, "POST", Some(body.as_bytes()), None)
                    .await
                    .map_err(nostr_signing_error)?;
            stream_response_with_authorization(
                http_client.as_ref(),
                PROVIDER_NAME.0.as_str(),
                &api_url,
                &authorization,
                body,
                is_streaming,
                &CustomHeaders::default(),
            )
            .await
            .map_err(LanguageModelCompletionError::from)
        });

        async move { Ok(future.await?.boxed()) }.boxed()
    }

    fn stream_websocket_responses(
        &self,
        request: ResponseRequest,
        cx: &AsyncApp,
    ) -> BoxFuture<
        'static,
        Result<
            futures::stream::BoxStream<'static, Result<ResponsesStreamEvent>>,
            LanguageModelCompletionError,
        >,
    > {
        let request_limiter = self.request_limiter.clone();
        let (websocket_url, authentication_url) = cx.update(|cx| {
            let settings = &crate::settings::AllLanguageModelSettings::get_global(cx).openagents;
            (
                settings.websocket_url().map(str::to_owned),
                settings.authentication_url().to_owned(),
            )
        });

        async move {
            let websocket_url = websocket_url.ok_or_else(|| {
                LanguageModelCompletionError::Other(anyhow::anyhow!(
                    "The OpenAgents development WebSocket is not configured."
                ))
            })?;
            let future = request_limiter.stream(async move {
                let signed_url = format!("{authentication_url}/responses");
                let authorization =
                    omega_effectd::sign_nip98_request(&signed_url, "GET", None, None)
                        .await
                        .map_err(nostr_signing_error)?;
                let mut websocket_request = websocket_url
                    .into_client_request()
                    .map_err(|error| LanguageModelCompletionError::Other(error.into()))?;
                websocket_request.headers_mut().insert(
                    "Authorization",
                    authorization.parse().map_err(|error| {
                        LanguageModelCompletionError::Other(anyhow::anyhow!(
                            "The OpenAgents authorization header is invalid: {error}"
                        ))
                    })?,
                );

                let (mut socket, _) =
                    async_tungstenite::async_std::connect_async(websocket_request)
                        .await
                        .map_err(|error| LanguageModelCompletionError::Other(error.into()))?;
                socket
                    .send(WebSocketMessage::Text(
                        websocket_session_configuration().to_string().into(),
                    ))
                    .await
                    .map_err(|error| LanguageModelCompletionError::Other(error.into()))?;
                socket
                    .send(WebSocketMessage::Text(
                        websocket_response_request(request)?.into(),
                    ))
                    .await
                    .map_err(|error| LanguageModelCompletionError::Other(error.into()))?;

                Ok(websocket_response_stream(socket))
            });
            Ok(future.await?.boxed())
        }
        .boxed()
    }
}

fn websocket_response_stream<S, E>(
    socket: S,
) -> futures::stream::BoxStream<'static, Result<ResponsesStreamEvent>>
where
    S: Stream<Item = std::result::Result<WebSocketMessage, E>> + Send + Unpin + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    stream::unfold(Some(socket), |socket| async move {
        let mut socket = socket?;
        loop {
            let message = socket.next().await?;
            match message {
                Ok(WebSocketMessage::Text(text)) => match websocket_response_event(text.as_str()) {
                    Ok(ResponsesStreamEvent::Unknown) => continue,
                    Ok(event) => {
                        let next_socket = if websocket_response_is_terminal(&event) {
                            None
                        } else {
                            Some(socket)
                        };
                        return Some((Ok(event), next_socket));
                    }
                    Err(error) => return Some((Err(error), None)),
                },
                Ok(WebSocketMessage::Close(_)) => return None,
                Ok(WebSocketMessage::Ping(_))
                | Ok(WebSocketMessage::Pong(_))
                | Ok(WebSocketMessage::Frame(_)) => continue,
                Ok(WebSocketMessage::Binary(_)) => {
                    return Some((
                        Err(anyhow::anyhow!(
                            "The OpenAgents WebSocket returned a binary message."
                        )),
                        None,
                    ));
                }
                Err(error) => return Some((Err(anyhow::Error::new(error)), None)),
            }
        }
    })
    .boxed()
}

fn websocket_response_is_terminal(event: &ResponsesStreamEvent) -> bool {
    matches!(
        event,
        ResponsesStreamEvent::Completed { .. }
            | ResponsesStreamEvent::Incomplete { .. }
            | ResponsesStreamEvent::Failed { .. }
            | ResponsesStreamEvent::Error { .. }
            | ResponsesStreamEvent::GenericError { .. }
    )
}

fn websocket_session_configuration() -> serde_json::Value {
    let detected_agents = omega_agent_detect::detected()
        .iter()
        .map(|agent| {
            serde_json::json!({
                "id": agent.id,
                "name": agent.name
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "type": "openagents:session.configure",
        "system": {
            "os": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
            "detected_agents": detected_agents
        }
    })
}

fn websocket_response_request(request: ResponseRequest) -> Result<String> {
    let mut value = serde_json::to_value(request)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("The OpenAgents response request must be an object."))?;
    object.insert(
        "type".to_owned(),
        serde_json::Value::String("response.create".to_owned()),
    );
    object.remove("stream");
    Ok(serde_json::to_string(&value)?)
}

fn websocket_response_event(text: &str) -> Result<ResponsesStreamEvent> {
    let value = serde_json::from_str::<serde_json::Value>(text)?;
    if value.get("type").and_then(serde_json::Value::as_str) == Some("openagents:response.status") {
        let message = value
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let classification = value
            .get("classification")
            .and_then(serde_json::Value::as_str);
        let delta = if classification == Some("greeting") {
            message.to_owned()
        } else {
            format!("{message}\n\n")
        };
        return Ok(ResponsesStreamEvent::OutputTextDelta {
            item_id: "openagents-status".to_owned(),
            output_index: 0,
            content_index: Some(0),
            delta,
        });
    }
    Ok(serde_json::from_value(value)?)
}

fn nostr_signing_error(
    blocker: omega_effectd::HostedSessionBlocker,
) -> LanguageModelCompletionError {
    LanguageModelCompletionError::AuthenticationError {
        provider: PROVIDER_NAME,
        message: format!(
            "Omega could not sign this request with your Nostr identity. {}",
            blocker.summary()
        ),
    }
}

fn response_request(mut request: LanguageModelRequest) -> Result<ResponseRequest> {
    request.thinking_allowed = false;
    request.thinking_effort = None;
    request.speed = None;
    into_open_ai_response(
        request,
        OMEGA_AGENT_MODEL_ID,
        true,
        true,
        Some(MAX_OUTPUT_TOKENS),
        None,
        false,
        &PROVIDER_ID,
    )
}

impl LanguageModel for OpenAgentsLanguageModel {
    fn id(&self) -> LanguageModelId {
        LanguageModelId::from(OMEGA_AGENT_MODEL_ID.to_string())
    }

    fn name(&self) -> LanguageModelName {
        LanguageModelName::from("Omega Agent".to_string())
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

    fn supports_split_token_display(&self) -> bool {
        true
    }

    fn telemetry_id(&self) -> String {
        "openagents/omega-agent".to_string()
    }

    fn max_token_count(&self) -> u64 {
        MAX_TOKENS
    }

    fn max_output_tokens(&self) -> Option<u64> {
        Some(MAX_OUTPUT_TOKENS)
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
        let request = match response_request(request) {
            Ok(request) => request,
            Err(error) => return async move { Err(error.into()) }.boxed(),
        };
        let use_development_websocket = cx.update(|cx| {
            crate::settings::AllLanguageModelSettings::get_global(cx)
                .openagents
                .use_development_api
        });
        let responses = if use_development_websocket {
            self.stream_websocket_responses(request, cx)
        } else {
            self.stream_responses(request, cx)
        };
        async move {
            let mapper = OpenAiResponseEventMapper::new(PROVIDER_ID);
            Ok(mapper.map_stream(responses.await?).boxed())
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_environment_selects_the_expected_endpoint() {
        assert_eq!(OpenAgentsSettings::default().api_url(), PRODUCTION_API_URL);
        assert_eq!(
            OpenAgentsSettings {
                use_development_api: true,
            }
            .api_url(),
            DEVELOPMENT_API_URL
        );
        assert_eq!(
            OpenAgentsSettings {
                use_development_api: true,
            }
            .authentication_url(),
            PRODUCTION_API_URL
        );
        assert_eq!(
            OpenAgentsSettings {
                use_development_api: true,
            }
            .websocket_url(),
            Some(DEVELOPMENT_WEBSOCKET_URL)
        );
        assert_eq!(OpenAgentsSettings::default().websocket_url(), None);
    }

    #[test]
    fn omega_agent_request_hides_provider_model_and_client_reasoning() {
        let request = LanguageModelRequest {
            thinking_allowed: true,
            thinking_effort: Some("high".to_string()),
            ..Default::default()
        };
        let request = response_request(request).expect("Omega Agent request should convert");

        assert_eq!(request.model, OMEGA_AGENT_MODEL_ID);
        assert!(request.stream);
        assert!(request.reasoning.is_none());
        assert!(request.service_tier.is_none());
        assert_eq!(request.max_output_tokens, Some(MAX_OUTPUT_TOKENS));
    }

    #[test]
    fn provider_id_is_openagents() {
        assert_eq!(PROVIDER_ID.0.as_ref(), "openagents");
    }

    #[test]
    fn websocket_request_uses_the_response_create_event() {
        let request = response_request(LanguageModelRequest::default())
            .expect("Omega Agent request should convert");
        let value = serde_json::from_str::<serde_json::Value>(
            &websocket_response_request(request).expect("request should serialize"),
        )
        .expect("request should be JSON");

        assert_eq!(value["type"], "response.create");
        assert_eq!(value["model"], OMEGA_AGENT_MODEL_ID);
        assert!(value.get("stream").is_none());
    }

    #[test]
    fn websocket_status_becomes_response_text() {
        let event = websocket_response_event(
            &serde_json::json!({
                "type": "openagents:response.status",
                "classification": "request",
                "message": "Checking on that."
            })
            .to_string(),
        )
        .expect("status should parse");

        assert!(matches!(
            event,
            ResponsesStreamEvent::OutputTextDelta { delta, .. }
                if delta == "Checking on that.\n\n"
        ));
    }

    #[test]
    fn websocket_stream_ends_after_the_response_terminal_event() {
        async_std::task::block_on(async {
            let completed = serde_json::json!({
                "type": "response.completed",
                "response": {
                    "id": "response-1",
                    "status": "completed",
                    "output": []
                }
            })
            .to_string();
            let late_delta = serde_json::json!({
                "type": "response.output_text.delta",
                "item_id": "message-1",
                "output_index": 0,
                "content_index": 0,
                "delta": "late"
            })
            .to_string();
            let socket = stream::iter([
                Ok::<_, async_tungstenite::tungstenite::Error>(WebSocketMessage::Text(
                    completed.into(),
                )),
                Ok(WebSocketMessage::Text(late_delta.into())),
            ]);
            let mut responses = websocket_response_stream(socket);

            assert!(matches!(
                responses.next().await,
                Some(Ok(ResponsesStreamEvent::Completed { .. }))
            ));
            assert!(responses.next().await.is_none());
        });
    }

    #[test]
    fn development_websocket_connects_without_a_tokio_runtime() {
        async_std::task::block_on(async {
            let listener = async_std::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("test WebSocket listener should bind");
            let address = listener
                .local_addr()
                .expect("test WebSocket listener should have an address");
            let server = async_std::task::spawn(async move {
                let (stream, _) = listener
                    .accept()
                    .await
                    .expect("test WebSocket listener should accept a connection");
                async_tungstenite::accept_async(stream)
                    .await
                    .expect("test WebSocket server should complete the handshake");
            });

            async_tungstenite::async_std::connect_async(format!("ws://{address}"))
                .await
                .expect("Omega WebSocket client should connect without Tokio");
            server.await;
        });
    }
}
