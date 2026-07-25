use anyhow::{Context as _, Result, anyhow};
use credentials_provider::CredentialsProvider;
use fs::Fs;
use futures::{
    AsyncBufReadExt, AsyncReadExt, FutureExt, StreamExt, future::BoxFuture, io::BufReader,
    stream::BoxStream,
};
use gpui::{App, AsyncApp, Context, Entity, Subscription, Task};
use http_client::{
    AsyncBody, CustomHeaders, HttpClient, Method, Request as HttpRequest, RequestBuilderExt,
    StatusCode, http,
};
use language_model::InlineDescription;
use language_model::{
    ApiKeyState, AuthenticateError, EnvVar, IconOrSvg, LanguageModel, LanguageModelCompletionError,
    LanguageModelCompletionEvent, LanguageModelId, LanguageModelName, LanguageModelProvider,
    LanguageModelProviderId, LanguageModelProviderName, LanguageModelProviderState,
    LanguageModelRequest, LanguageModelToolChoice, LanguageModelToolSchemaFormat,
    ProviderSettingsView, RateLimiter, SubPageProviderSettings, env_var,
};
use open_ai::ResponseStreamEvent;
use serde::Deserialize;
pub use settings::ExoAvailableModel as AvailableModel;
use settings::{Settings, SettingsStore, update_settings_file};
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};
use ui::{ButtonLike, ConfiguredApiCard, Divider, List, ListBulletItem, Tooltip, prelude::*};
use ui_input::InputField;

use crate::AllLanguageModelSettings;
use crate::provider::open_ai::{ChatCompletionMaxTokensParameter, OpenAiEventMapper, into_open_ai};

const PROVIDER_ID: LanguageModelProviderId = LanguageModelProviderId::new("exo");
const PROVIDER_NAME: LanguageModelProviderName = LanguageModelProviderName::new("exo");

pub const EXO_API_URL: &str = "http://127.0.0.1:52415/v1";

const API_KEY_ENV_VAR_NAME: &str = "EXO_API_KEY";
static API_KEY_ENV_VAR: LazyLock<EnvVar> = env_var!(API_KEY_ENV_VAR_NAME);

#[derive(Default, Debug, Clone, PartialEq)]
pub struct ExoSettings {
    pub api_url: String,
    pub available_models: Vec<AvailableModel>,
    pub custom_headers: CustomHeaders,
}

/// A model entry as reported by exo's `GET /models` endpoint. Unknown fields
/// (storage sizes, tags, descriptions, ...) are deliberately ignored so newer
/// exo releases keep deserializing.
#[derive(Clone, Debug, Deserialize)]
struct ExoModelEntry {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    context_length: u64,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    tasks: Vec<String>,
}

#[derive(Deserialize)]
struct ListModelsResponse {
    data: Vec<ExoModelEntry>,
}

#[derive(Clone, Debug, PartialEq)]
struct ExoModel {
    name: String,
    display_name: Option<String>,
    max_tokens: u64,
    supports_tools: bool,
    supports_images: bool,
    supports_thinking_toggle: bool,
}

const DEFAULT_CONTEXT_LENGTH: u64 = 32_768;

/// exo lists every model in its catalog, including audio/image-generation
/// models that cannot serve chat completions. Keep text-capable entries and,
/// for older servers that report neither capabilities nor tasks, keep the
/// entry rather than hiding it.
fn is_text_model(entry: &ExoModelEntry) -> bool {
    entry
        .capabilities
        .iter()
        .any(|capability| capability == "text")
        || entry.tasks.iter().any(|task| task.contains("text"))
        || (entry.capabilities.is_empty() && entry.tasks.is_empty())
}

fn exo_model_from_entry(entry: &ExoModelEntry) -> ExoModel {
    ExoModel {
        name: entry.id.clone(),
        display_name: (!entry.name.is_empty()).then(|| entry.name.clone()),
        max_tokens: if entry.context_length == 0 {
            DEFAULT_CONTEXT_LENGTH
        } else {
            entry.context_length
        },
        // exo's OpenAI-compatible API supports tool calls for chat models.
        supports_tools: true,
        supports_images: entry
            .capabilities
            .iter()
            .any(|capability| capability == "vision"),
        supports_thinking_toggle: entry
            .capabilities
            .iter()
            .any(|capability| capability == "thinking_toggle"),
    }
}

async fn get_models(
    client: &dyn HttpClient,
    api_url: &str,
    api_key: Option<&str>,
    extra_headers: &CustomHeaders,
) -> Result<Vec<ExoModelEntry>> {
    let uri = format!("{api_url}/models");
    let mut request_builder = HttpRequest::builder()
        .method(Method::GET)
        .uri(uri)
        .header("Accept", "application/json");

    if let Some(api_key) = api_key {
        request_builder = request_builder.header("Authorization", format!("Bearer {}", api_key));
    }

    let request = request_builder
        .extra_headers(extra_headers)
        .body(AsyncBody::default())?;

    let mut response = client.send(request).await?;

    let mut body = String::new();
    response.body_mut().read_to_string(&mut body).await?;

    anyhow::ensure!(
        response.status().is_success(),
        "Failed to connect to exo API: {} {}",
        response.status(),
        body,
    );
    let response: ListModelsResponse =
        serde_json::from_str(&body).context("Unable to parse exo models response")?;
    Ok(response.data)
}

pub struct ExoLanguageModelProvider {
    http_client: Arc<dyn HttpClient>,
    state: Entity<State>,
}

pub struct State {
    api_key_state: ApiKeyState,
    credentials_provider: Arc<dyn CredentialsProvider>,
    http_client: Arc<dyn HttpClient>,
    available_models: Vec<ExoModel>,
    fetch_model_task: Option<Task<Result<()>>>,
    _subscription: Subscription,
}

impl State {
    fn is_authenticated(&self) -> bool {
        !self.available_models.is_empty()
    }

    fn set_api_key(&mut self, api_key: Option<String>, cx: &mut Context<Self>) -> Task<Result<()>> {
        let credentials_provider = self.credentials_provider.clone();
        let api_url = ExoLanguageModelProvider::api_url(cx).into();
        let task = self.api_key_state.store(
            api_url,
            api_key,
            |this| &mut this.api_key_state,
            credentials_provider,
            cx,
        );
        self.restart_fetch_models_task(cx);
        task
    }

    fn fetch_models(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let settings = &AllLanguageModelSettings::get_global(cx).exo;
        let http_client = self.http_client.clone();
        let api_url = settings.api_url.clone();
        let api_key = self.api_key_state.key(&api_url);
        let extra_headers = settings.custom_headers.clone();

        // As a proxy for the server being "authenticated", we'll check if it's up by fetching the models
        cx.spawn(async move |this, cx| {
            let entries = get_models(
                http_client.as_ref(),
                &api_url,
                api_key.as_deref(),
                &extra_headers,
            )
            .await?;

            let mut models: Vec<ExoModel> = entries
                .iter()
                .filter(|entry| is_text_model(entry))
                .map(exo_model_from_entry)
                .collect();

            models.sort_by(|a, b| a.name.cmp(&b.name));

            this.update(cx, |this, cx| {
                this.available_models = models;
                cx.notify();
            })
        })
    }

    fn restart_fetch_models_task(&mut self, cx: &mut Context<Self>) {
        let task = self.fetch_models(cx);
        self.fetch_model_task.replace(task);
    }

    fn authenticate(&mut self, cx: &mut Context<Self>) -> Task<Result<(), AuthenticateError>> {
        let credentials_provider = self.credentials_provider.clone();
        let api_url = ExoLanguageModelProvider::api_url(cx).into();
        let _task = self.api_key_state.load_if_needed(
            api_url,
            |this| &mut this.api_key_state,
            credentials_provider,
            cx,
        );

        if self.is_authenticated() {
            return Task::ready(Ok(()));
        }

        let fetch_models_task = self.fetch_models(cx);
        cx.spawn(async move |_this, _cx| {
            match fetch_models_task.await {
                Ok(()) => Ok(()),
                Err(err) => {
                    // If any cause in the error chain is an std::io::Error with
                    // ErrorKind::ConnectionRefused, treat this as "credentials not found"
                    // (i.e. exo not running).
                    let mut connection_refused = false;
                    for cause in err.chain() {
                        if let Some(io_err) = cause.downcast_ref::<std::io::Error>() {
                            if io_err.kind() == std::io::ErrorKind::ConnectionRefused {
                                connection_refused = true;
                                break;
                            }
                        }
                    }
                    if connection_refused {
                        Err(AuthenticateError::ConnectionRefused)
                    } else {
                        Err(AuthenticateError::Other(err))
                    }
                }
            }
        })
    }
}

impl ExoLanguageModelProvider {
    pub fn new(
        http_client: Arc<dyn HttpClient>,
        credentials_provider: Arc<dyn CredentialsProvider>,
        cx: &mut App,
    ) -> Self {
        let this = Self {
            http_client: http_client.clone(),
            state: cx.new(|cx| {
                let subscription = cx.observe_global::<SettingsStore>({
                    let mut settings = AllLanguageModelSettings::get_global(cx).exo.clone();
                    move |this: &mut State, cx| {
                        let new_settings = AllLanguageModelSettings::get_global(cx).exo.clone();
                        if settings != new_settings {
                            let credentials_provider = this.credentials_provider.clone();
                            let api_url = Self::api_url(cx).into();
                            this.api_key_state.handle_url_change(
                                api_url,
                                |this| &mut this.api_key_state,
                                credentials_provider,
                                cx,
                            );
                            settings = new_settings;
                            this.restart_fetch_models_task(cx);
                            cx.notify();
                        }
                    }
                });

                State {
                    api_key_state: ApiKeyState::new(
                        Self::api_url(cx).into(),
                        (*API_KEY_ENV_VAR).clone(),
                    ),
                    credentials_provider,
                    http_client,
                    available_models: Default::default(),
                    fetch_model_task: None,
                    _subscription: subscription,
                }
            }),
        };
        this.state
            .update(cx, |state, cx| state.restart_fetch_models_task(cx));
        this
    }

    fn api_url(cx: &App) -> String {
        AllLanguageModelSettings::get_global(cx).exo.api_url.clone()
    }

    fn has_custom_url(cx: &App) -> bool {
        Self::api_url(cx) != EXO_API_URL
    }
}

impl LanguageModelProviderState for ExoLanguageModelProvider {
    type ObservableEntity = State;

    fn observable_entity(&self) -> Option<Entity<Self::ObservableEntity>> {
        Some(self.state.clone())
    }
}

impl LanguageModelProvider for ExoLanguageModelProvider {
    fn id(&self) -> LanguageModelProviderId {
        PROVIDER_ID
    }

    fn name(&self) -> LanguageModelProviderName {
        PROVIDER_NAME
    }

    fn icon(&self) -> IconOrSvg {
        IconOrSvg::Icon(IconName::AiOpenAiCompat)
    }

    fn default_model(&self, _: &App) -> Option<Arc<dyn LanguageModel>> {
        // exo lists its whole catalog; a listed model does not imply a loaded
        // instance, so selecting one by default would produce a guaranteed
        // "no running instance" error.
        None
    }

    fn default_fast_model(&self, _: &App) -> Option<Arc<dyn LanguageModel>> {
        // See explanation for default_model.
        None
    }

    fn provided_models(&self, cx: &App) -> Vec<Arc<dyn LanguageModel>> {
        let mut models: BTreeMap<String, ExoModel> = BTreeMap::default();

        // Add models from the exo API
        for model in self.state.read(cx).available_models.iter() {
            models.insert(model.name.clone(), model.clone());
        }

        // Override with available models from settings
        for model in AllLanguageModelSettings::get_global(cx)
            .exo
            .available_models
            .iter()
        {
            models.insert(
                model.name.clone(),
                ExoModel {
                    name: model.name.clone(),
                    display_name: model.display_name.clone(),
                    max_tokens: model.max_tokens,
                    supports_tools: model.supports_tools.unwrap_or(true),
                    supports_images: model.supports_images.unwrap_or(false),
                    supports_thinking_toggle: model.supports_thinking_toggle.unwrap_or(false),
                },
            );
        }

        models
            .into_values()
            .map(|model| {
                Arc::new(ExoLanguageModel {
                    id: LanguageModelId::from(model.name.clone()),
                    model,
                    http_client: self.http_client.clone(),
                    request_limiter: RateLimiter::new(4),
                    state: self.state.clone(),
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

    fn settings_view(&self, _cx: &mut App) -> Option<ProviderSettingsView> {
        let state = self.state.clone();
        Some(ProviderSettingsView::SubPage(
            SubPageProviderSettings::new(move |window, cx| {
                cx.new(|cx| ConfigurationView::new(state.clone(), window, cx))
                    .into()
            })
            .description(InlineDescription::Text(
                "Run models on your local exo cluster.".into(),
            )),
        ))
    }
}

enum ExoRequestError {
    HttpResponseError {
        status_code: StatusCode,
        body: String,
        headers: http::HeaderMap,
    },
    Other(anyhow::Error),
}

fn map_exo_error(error: ExoRequestError) -> LanguageModelCompletionError {
    match error {
        ExoRequestError::HttpResponseError {
            status_code,
            body,
            headers,
        } => {
            // exo returns 404 with this body when the model is listed in the
            // catalog but has no running instance. The generic 404 mapping
            // (ApiEndpointNotFound) would discard the body, so intercept it
            // here with an actionable message.
            if status_code == StatusCode::NOT_FOUND && body.contains("No instance found for model")
            {
                return LanguageModelCompletionError::Other(anyhow!(
                    "exo has no running instance for this model. Load it from the exo dashboard (http://127.0.0.1:52415) or with the exo CLI, then retry."
                ));
            }

            let retry_after = headers
                .get(http::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()?.parse::<u64>().ok())
                .map(std::time::Duration::from_secs);

            LanguageModelCompletionError::from_http_status(
                PROVIDER_NAME,
                status_code,
                body,
                retry_after,
            )
        }
        ExoRequestError::Other(error) => LanguageModelCompletionError::Other(error),
    }
}

/// Serializes the OpenAI-shaped request and injects exo's top-level
/// `enable_thinking` field. The key is omitted entirely for models without
/// the thinking toggle.
fn build_request_body(
    request: open_ai::Request,
    enable_thinking: Option<bool>,
) -> Result<serde_json::Value> {
    let mut body = serde_json::to_value(request)?;
    if let Some(enable_thinking) = enable_thinking {
        body.as_object_mut()
            .context("serialized exo request is not a JSON object")?
            .insert(
                "enable_thinking".into(),
                serde_json::Value::Bool(enable_thinking),
            );
    }
    Ok(body)
}

#[derive(Deserialize)]
struct StreamError {
    message: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StreamResult {
    Ok(ResponseStreamEvent),
    Err { error: StreamError },
}

/// Parses one SSE line from exo's chat-completions stream. Returns `None` for
/// non-data lines (exo emits `: prefill_progress ...` and `: generation_stats
/// ...` comments) and for the `[DONE]` sentinel.
fn parse_sse_line(line: &str) -> Option<Result<ResponseStreamEvent>> {
    let line = line
        .strip_prefix("data: ")
        .or_else(|| line.strip_prefix("data:"))?;
    if line == "[DONE]" {
        return None;
    }
    match serde_json::from_str(line) {
        Ok(StreamResult::Ok(response)) => Some(Ok(response)),
        Ok(StreamResult::Err { error }) => Some(Err(anyhow!(error.message))),
        Err(error) => {
            log::error!(
                "Failed to parse exo response into ResponseStreamEvent: `{}`\n\
                Response: `{}`",
                error,
                line,
            );
            Some(Err(anyhow!(error)))
        }
    }
}

async fn stream_completion(
    client: &dyn HttpClient,
    api_url: &str,
    api_key: Option<&str>,
    body: serde_json::Value,
    extra_headers: &CustomHeaders,
) -> Result<BoxStream<'static, Result<ResponseStreamEvent>>, ExoRequestError> {
    let uri = format!("{api_url}/chat/completions");
    let mut request_builder = HttpRequest::builder()
        .method(Method::POST)
        .uri(uri)
        .header("Content-Type", "application/json");

    if let Some(api_key) = api_key {
        request_builder =
            request_builder.header("Authorization", format!("Bearer {}", api_key.trim()));
    }

    let request = request_builder
        .extra_headers(extra_headers)
        .body(AsyncBody::from(
            serde_json::to_string(&body).map_err(|e| ExoRequestError::Other(e.into()))?,
        ))
        .map_err(|e| ExoRequestError::Other(e.into()))?;

    let mut response = client.send(request).await.map_err(ExoRequestError::Other)?;
    if response.status().is_success() {
        let reader = BufReader::new(response.into_body());
        Ok(reader
            .lines()
            .filter_map(|line| async move {
                match line {
                    Ok(line) => parse_sse_line(&line),
                    Err(error) => Some(Err(anyhow!(error))),
                }
            })
            .boxed())
    } else {
        let mut body = String::new();
        response
            .body_mut()
            .read_to_string(&mut body)
            .await
            .map_err(|e| ExoRequestError::Other(e.into()))?;

        Err(ExoRequestError::HttpResponseError {
            status_code: response.status(),
            body,
            headers: response.headers().clone(),
        })
    }
}

pub struct ExoLanguageModel {
    id: LanguageModelId,
    model: ExoModel,
    http_client: Arc<dyn HttpClient>,
    request_limiter: RateLimiter,
    state: Entity<State>,
}

impl ExoLanguageModel {
    fn stream_completion(
        &self,
        body: serde_json::Value,
        cx: &AsyncApp,
    ) -> BoxFuture<
        'static,
        Result<BoxStream<'static, Result<ResponseStreamEvent>>, LanguageModelCompletionError>,
    > {
        let http_client = self.http_client.clone();
        let (api_key, api_url, extra_headers) = self.state.read_with(cx, |state, cx| {
            let api_url = ExoLanguageModelProvider::api_url(cx);
            let extra_headers = AllLanguageModelSettings::get_global(cx)
                .exo
                .custom_headers
                .clone();
            (state.api_key_state.key(&api_url), api_url, extra_headers)
        });

        let future = self.request_limiter.stream(async move {
            let stream = stream_completion(
                http_client.as_ref(),
                &api_url,
                api_key.as_deref(),
                body,
                &extra_headers,
            )
            .await
            .map_err(map_exo_error)?;
            Ok(stream)
        });

        async move { Ok(future.await?.boxed()) }.boxed()
    }
}

impl LanguageModel for ExoLanguageModel {
    fn id(&self) -> LanguageModelId {
        self.id.clone()
    }

    fn name(&self) -> LanguageModelName {
        LanguageModelName::from(
            self.model
                .display_name
                .clone()
                .unwrap_or_else(|| self.model.name.clone()),
        )
    }

    fn provider_id(&self) -> LanguageModelProviderId {
        PROVIDER_ID
    }

    fn provider_name(&self) -> LanguageModelProviderName {
        PROVIDER_NAME
    }

    fn supports_tools(&self) -> bool {
        self.model.supports_tools
    }

    fn tool_input_format(&self) -> LanguageModelToolSchemaFormat {
        LanguageModelToolSchemaFormat::JsonSchemaSubset
    }

    fn supports_images(&self) -> bool {
        self.model.supports_images
    }

    fn supports_tool_choice(&self, choice: LanguageModelToolChoice) -> bool {
        match choice {
            LanguageModelToolChoice::Auto => self.model.supports_tools,
            LanguageModelToolChoice::Any => self.model.supports_tools,
            LanguageModelToolChoice::None => true,
        }
    }

    fn supports_streaming_tools(&self) -> bool {
        true
    }

    fn supports_thinking(&self) -> bool {
        self.model.supports_thinking_toggle
    }

    fn telemetry_id(&self) -> String {
        format!("exo/{}", self.model.name)
    }

    fn max_token_count(&self) -> u64 {
        self.model.max_tokens
    }

    fn max_output_tokens(&self) -> Option<u64> {
        None
    }

    fn stream_completion(
        &self,
        mut request: LanguageModelRequest,
        cx: &AsyncApp,
    ) -> BoxFuture<
        'static,
        Result<
            BoxStream<'static, Result<LanguageModelCompletionEvent, LanguageModelCompletionError>>,
            LanguageModelCompletionError,
        >,
    > {
        // `speed` can leak in from a parent thread's model; exo never supports
        // fast mode and rejects OpenAI's `service_tier` field.
        request.speed = None;

        let enable_thinking = self
            .model
            .supports_thinking_toggle
            .then_some(request.thinking_allowed);

        let request = match into_open_ai(
            request,
            &self.model.name,
            false,
            false,
            self.max_output_tokens(),
            ChatCompletionMaxTokensParameter::MaxTokens,
            None,
            false,
        ) {
            Ok(request) => request,
            Err(error) => return async move { Err(error.into()) }.boxed(),
        };
        let body = match build_request_body(request, enable_thinking) {
            Ok(body) => body,
            Err(error) => return async move { Err(error.into()) }.boxed(),
        };
        let completions = self.stream_completion(body, cx);
        async move {
            let mapper = OpenAiEventMapper::new();
            Ok(mapper.map_stream(completions.await?).boxed())
        }
        .boxed()
    }
}

struct ConfigurationView {
    state: Entity<State>,
    api_key_editor: Entity<InputField>,
    api_url_editor: Entity<InputField>,
}

impl ConfigurationView {
    pub fn new(state: Entity<State>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let api_key_editor = cx.new(|cx| InputField::new(_window, cx, "sk-...").label("API key"));

        let api_url_editor = cx.new(|cx| {
            let input = InputField::new(_window, cx, EXO_API_URL).label("API URL");
            input.set_text(&ExoLanguageModelProvider::api_url(cx), _window, cx);
            input
        });

        cx.observe(&state, |_, _, cx| {
            cx.notify();
        })
        .detach();

        Self {
            state,
            api_key_editor,
            api_url_editor,
        }
    }

    fn retry_connection(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let has_api_url = ExoLanguageModelProvider::has_custom_url(cx);
        let has_api_key = self
            .state
            .read_with(cx, |state, _| state.api_key_state.has_key());
        if !has_api_url {
            self.save_api_url(cx);
        }
        if !has_api_key {
            self.save_api_key(&Default::default(), _window, cx);
        }

        self.state.update(cx, |state, cx| {
            state.restart_fetch_models_task(cx);
        });
    }

    fn save_api_key(&mut self, _: &menu::Confirm, _window: &mut Window, cx: &mut Context<Self>) {
        let api_key = self.api_key_editor.read(cx).text(cx).trim().to_string();
        if api_key.is_empty() {
            return;
        }

        self.api_key_editor
            .update(cx, |input, cx| input.set_text("", _window, cx));

        let state = self.state.clone();
        cx.spawn_in(_window, async move |_, cx| {
            state
                .update(cx, |state, cx| state.set_api_key(Some(api_key), cx))
                .await
        })
        .detach_and_log_err(cx);
    }

    fn reset_api_key(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.api_key_editor
            .update(cx, |input, cx| input.set_text("", _window, cx));

        let state = self.state.clone();
        cx.spawn_in(_window, async move |_, cx| {
            state
                .update(cx, |state, cx| state.set_api_key(None, cx))
                .await
        })
        .detach_and_log_err(cx);

        cx.notify();
    }

    fn save_api_url(&self, cx: &mut Context<Self>) {
        let api_url = self.api_url_editor.read(cx).text(cx).trim().to_string();
        let current_url = ExoLanguageModelProvider::api_url(cx);
        if !api_url.is_empty() && &api_url != &current_url {
            self.state
                .update(cx, |state, cx| state.set_api_key(None, cx))
                .detach_and_log_err(cx);

            let fs = <dyn Fs>::global(cx);
            update_settings_file(fs, cx, move |settings, _| {
                settings
                    .language_models
                    .get_or_insert_default()
                    .exo
                    .get_or_insert_default()
                    .api_url = Some(api_url);
            });
        }
    }

    fn reset_api_url(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.api_url_editor
            .update(cx, |input, cx| input.set_text("", _window, cx));

        // Clear API key when URL changes since keys are URL-specific
        self.state
            .update(cx, |state, cx| state.set_api_key(None, cx))
            .detach_and_log_err(cx);

        let fs = <dyn Fs>::global(cx);
        update_settings_file(fs, cx, |settings, _cx| {
            if let Some(settings) = settings
                .language_models
                .as_mut()
                .and_then(|models| models.exo.as_mut())
            {
                settings.api_url = Some(EXO_API_URL.into());
            }
        });
        cx.notify();
    }

    fn render_api_url_editor(&self, cx: &Context<Self>) -> impl IntoElement {
        let api_url = ExoLanguageModelProvider::api_url(cx);
        let custom_api_url_set = api_url != EXO_API_URL;

        if custom_api_url_set {
            ConfiguredApiCard::new("reset-api-url", api_url)
                .on_click(cx.listener(|this, _, _window, cx| this.reset_api_url(_window, cx)))
                .into_any_element()
        } else {
            v_flex()
                .on_action(cx.listener(|this, _: &menu::Confirm, _window, cx| {
                    this.save_api_url(cx);
                    cx.notify();
                }))
                .child(self.api_url_editor.clone())
                .into_any_element()
        }
    }

    fn render_api_key_editor(&self, cx: &Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let env_var_set = state.api_key_state.is_from_env_var();
        let configured_card_label = if env_var_set {
            format!("API key set in {API_KEY_ENV_VAR_NAME} environment variable.")
        } else {
            "API key configured".to_string()
        };

        let api_key_control = if !state.api_key_state.has_key() {
            self.api_key_editor.clone().into_any_element()
        } else {
            ConfiguredApiCard::new("exo-reset-key", configured_card_label)
                .disabled(env_var_set)
                .on_click(cx.listener(|this, _, _window, cx| this.reset_api_key(_window, cx)))
                .when(env_var_set, |this| {
                    this.tooltip_label(format!(
                        "To reset your API key, unset the {API_KEY_ENV_VAR_NAME} environment variable."
                    ))
                })
                .into_any_element()
        };

        v_flex()
            .on_action(cx.listener(Self::save_api_key))
            .child(api_key_control)
            .gap_1p5()
            .mb_2()
            .child(
                Label::new(format!(
                    "You can also set the {API_KEY_ENV_VAR_NAME} environment variable and restart Zed."
                ))
                .size(LabelSize::Small)
                .color(Color::Muted),
            )
    }
}

impl Render for ConfigurationView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_authenticated = self.state.read(cx).is_authenticated();

        v_flex()
            .gap_2()
            .child(
                v_flex()
                    .gap_1()
                    .child(Headline::new("exo").size(HeadlineSize::Small))
                    .child(Label::new("Run models on your local exo cluster.").color(Color::Muted))
                    .child(
                        List::new()
                            .child(
                                ListBulletItem::new("")
                                    .child(
                                        Label::new(
                                            "exo must be running on this machine. Start it with",
                                        )
                                        .color(Color::Muted),
                                    )
                                    .child(
                                        Label::new("exo")
                                            .inline_code(cx)
                                            .color(Color::Muted)
                                            .ml_1(),
                                    ),
                            )
                            .child(
                                ListBulletItem::new(
                                    "The exo API has no authentication and listens on your local \
                                    network. Keep the URL on 127.0.0.1 unless you trust the \
                                    network.",
                                )
                                .label_color(Color::Muted),
                            ),
                    )
                    .child(
                        Label::new(
                            "Alternatively, you can connect to a remote exo cluster by specifying \
                            its URL and API key (may not be required):",
                        )
                        .color(Color::Muted),
                    ),
            )
            .child(self.render_api_url_editor(cx))
            .child(self.render_api_key_editor(cx))
            .child(Divider::horizontal())
            .child(h_flex().pt_2().w_full().justify_end().gap_1().map(|this| {
                if is_authenticated {
                    this.child(
                        ButtonLike::new("connected")
                            .size(ButtonSize::Medium)
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(Icon::new(IconName::Check).color(Color::Success))
                                    .child(Label::new("Connected")),
                            )
                            .child(
                                IconButton::new("refresh-models", IconName::RotateCcw)
                                    .tooltip(Tooltip::text("Refresh Models"))
                                    .icon_size(IconSize::Small)
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.state.update(cx, |state, _| {
                                            state.available_models.clear();
                                        });
                                        this.retry_connection(_window, cx);
                                    })),
                            ),
                    )
                } else {
                    this.child(
                        Button::new("retry_exo_models", "Connect")
                            .style(ButtonStyle::Outlined)
                            .size(ButtonSize::Medium)
                            .start_icon(Icon::new(IconName::PlayFilled).size(IconSize::XSmall))
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.retry_connection(_window, cx)
                            })),
                    )
                }
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_list_models_response_deserialization_and_capability_mapping() {
        let payload = json!({
            "object": "list",
            "data": [
                {
                    "id": "qwen3-30b",
                    "object": "model",
                    "created": 1700000000,
                    "owned_by": "exo",
                    "name": "Qwen3 30B",
                    "context_length": 40960,
                    "capabilities": ["text", "thinking_toggle"],
                    "reasoning_dialect": "qwen3",
                    "tasks": ["text-generation"],
                    "family": "qwen",
                    "quantization": "4bit",
                    "storage_size_megabytes": 17408,
                    "supports_tensor": true,
                    "is_custom": false,
                    "base_model": null,
                    "hugging_face_id": "Qwen/Qwen3-30B",
                    "description": "A reasoning model.",
                    "tags": ["reasoning"]
                },
                {
                    "id": "llava-13b",
                    "object": "model",
                    "name": "LLaVA 13B",
                    "context_length": 0,
                    "capabilities": ["text", "vision"],
                    "tasks": ["text-generation"]
                },
                {
                    "id": "sdxl-turbo",
                    "object": "model",
                    "name": "SDXL Turbo",
                    "capabilities": ["image"],
                    "tasks": ["image-generation"]
                },
                {
                    "id": "bare-model",
                    "object": "model"
                }
            ]
        })
        .to_string();

        let response: ListModelsResponse = serde_json::from_str(&payload).unwrap();
        assert_eq!(response.data.len(), 4);

        let text_models: Vec<&ExoModelEntry> = response
            .data
            .iter()
            .filter(|entry| is_text_model(entry))
            .collect();
        assert_eq!(
            text_models
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["qwen3-30b", "llava-13b", "bare-model"],
        );

        let qwen = exo_model_from_entry(text_models[0]);
        assert_eq!(qwen.name, "qwen3-30b");
        assert_eq!(qwen.display_name.as_deref(), Some("Qwen3 30B"));
        assert_eq!(qwen.max_tokens, 40960);
        assert!(qwen.supports_tools);
        assert!(!qwen.supports_images);
        assert!(qwen.supports_thinking_toggle);

        let llava = exo_model_from_entry(text_models[1]);
        assert!(llava.supports_images);
        assert!(!llava.supports_thinking_toggle);
        // Zero context length falls back to the default.
        assert_eq!(llava.max_tokens, DEFAULT_CONTEXT_LENGTH);

        let bare = exo_model_from_entry(text_models[2]);
        assert_eq!(bare.display_name, None);
        assert_eq!(bare.max_tokens, DEFAULT_CONTEXT_LENGTH);
    }

    #[test]
    fn test_request_body_shape() {
        let request = LanguageModelRequest {
            thinking_allowed: true,
            ..Default::default()
        };
        let request = into_open_ai(
            request,
            "qwen3-30b",
            false,
            false,
            Some(1024),
            ChatCompletionMaxTokensParameter::MaxTokens,
            None,
            false,
        )
        .unwrap();

        // Thinking-toggle model with thinking allowed.
        let body = build_request_body(request, Some(true)).unwrap();
        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["max_tokens"], json!(1024));
        assert_eq!(body.get("max_completion_tokens"), None);
        assert_eq!(body.get("service_tier"), None);
        assert_eq!(body.get("reasoning_effort"), None);
        assert_eq!(body["enable_thinking"], json!(true));

        // Non-toggle model: the key must be absent entirely.
        let request = into_open_ai(
            LanguageModelRequest::default(),
            "llama-3.2-1b",
            false,
            false,
            None,
            ChatCompletionMaxTokensParameter::MaxTokens,
            None,
            false,
        )
        .unwrap();
        let body = build_request_body(request, None).unwrap();
        assert_eq!(body.get("enable_thinking"), None);
    }

    #[test]
    fn test_sse_line_parsing() {
        let lines = [
            r#": prefill_progress {"p":0.5}"#,
            "",
            r#"data: {"choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
            r#": generation_stats {"tokens_per_second":42.0}"#,
            "data: [DONE]",
        ];

        let events: Vec<Result<ResponseStreamEvent>> = lines
            .iter()
            .filter_map(|line| parse_sse_line(line))
            .collect();

        assert_eq!(events.len(), 1);
        let event = events.into_iter().next().unwrap().unwrap();
        assert_eq!(
            event.choices[0].delta.as_ref().unwrap().content.as_deref(),
            Some("Hello")
        );
    }

    #[test]
    fn test_map_exo_error_404_no_instance() {
        let error = map_exo_error(ExoRequestError::HttpResponseError {
            status_code: StatusCode::NOT_FOUND,
            body: "No instance found for model qwen3-30b".to_string(),
            headers: http::HeaderMap::new(),
        });
        assert!(
            !matches!(
                error,
                LanguageModelCompletionError::ApiEndpointNotFound { .. }
            ),
            "the 404 body must not be swallowed by ApiEndpointNotFound"
        );
        assert!(error.to_string().contains("no running instance"));

        let error = map_exo_error(ExoRequestError::HttpResponseError {
            status_code: StatusCode::NOT_FOUND,
            body: "not found".to_string(),
            headers: http::HeaderMap::new(),
        });
        assert!(matches!(
            error,
            LanguageModelCompletionError::ApiEndpointNotFound { .. }
        ));
    }
}
