use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use anyhow::{Context as _, Result, anyhow, bail};
use async_tungstenite::tungstenite::{
    Message,
    client::IntoClientRequest as _,
    http::{HeaderValue, header::AUTHORIZATION},
    protocol::WebSocketConfig,
};
use cpal::DeviceId;
use futures::{FutureExt as _, StreamExt as _, pin_mut, select_biased};
use rodio::{buffer::SamplesBuffer, nz};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use audio::RodioExt as _;
use omega_effectd::VerifiedOpenAgentsSession;

pub const SARAH_VOICE_PROTOCOL_VERSION: u16 = 1;
pub const SARAH_VOICE_MODEL: &str = "gpt-realtime-2.1";
pub const SARAH_VOICE_GATEWAY_PATH: &str = "/api/sarah/realtime";
pub const SARAH_AUDIO_SAMPLE_RATE: u32 = 24_000;
const MICROPHONE_CHUNK_SAMPLES: usize = 480;
const MAX_EDITOR_TEXT_BYTES: usize = 64 * 1024;
const MAX_CONTEXT_CHARS: u32 = 16 * 1024;
const MAX_GATEWAY_FRAME_BYTES: usize = 256 * 1024;
const MAX_PROTOCOL_ID_BYTES: usize = 256;
const MAX_TRANSCRIPT_TEXT_BYTES: usize = 64 * 1024;
const MAX_ERROR_TEXT_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SarahVoiceState {
    #[default]
    Idle,
    Authenticating,
    RequestingMicrophone,
    Connecting,
    Listening,
    UserSpeaking,
    SarahSpeaking,
    Reconnecting,
    Ending,
    Error,
}

impl SarahVoiceState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Voice is off",
            Self::Authenticating => "Checking OpenAgents account…",
            Self::RequestingMicrophone => "Requesting microphone access…",
            Self::Connecting => "Connecting to Sarah…",
            Self::Listening => "Sarah is listening",
            Self::UserSpeaking => "You are speaking",
            Self::SarahSpeaking => "Sarah is speaking",
            Self::Reconnecting => "Reconnect required",
            Self::Ending => "Ending voice session…",
            Self::Error => "Voice session needs attention",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::RequestingMicrophone
                | Self::Connecting
                | Self::Listening
                | Self::UserSpeaking
                | Self::SarahSpeaking
                | Self::Reconnecting
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceParticipant {
    User,
    Sarah,
}

impl VoiceParticipant {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "You",
            Self::Sarah => "Sarah",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoiceTranscriptItem {
    pub item_id: String,
    pub participant: VoiceParticipant,
    pub text: String,
    pub complete: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "name",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SarahEditorCommand {
    ReadContext {
        #[serde(default)]
        max_chars: Option<u32>,
    },
    Navigate {
        line: u32,
        #[serde(default)]
        column: u32,
    },
    Insert {
        text: String,
    },
    ReplaceSelection {
        text: String,
    },
    Action {
        action: ApprovedEditorAction,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovedEditorAction {
    Undo,
    Redo,
    SaveActiveFile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandConfirmation {
    None,
    Destructive,
    ExternalEffect,
}

impl SarahEditorCommand {
    pub fn confirmation(&self) -> CommandConfirmation {
        match self {
            Self::ReadContext { .. } | Self::Navigate { .. } | Self::Insert { .. } => {
                CommandConfirmation::None
            }
            Self::ReplaceSelection { .. }
            | Self::Action {
                action: ApprovedEditorAction::Undo | ApprovedEditorAction::Redo,
            } => CommandConfirmation::Destructive,
            Self::Action {
                action: ApprovedEditorAction::SaveActiveFile,
            } => CommandConfirmation::ExternalEffect,
        }
    }

    pub fn validate(&self) -> Result<()> {
        match self {
            Self::ReadContext { max_chars } => {
                if max_chars
                    .is_some_and(|max_chars| max_chars == 0 || max_chars > MAX_CONTEXT_CHARS)
                {
                    bail!(
                        "context reads must request between 1 and {MAX_CONTEXT_CHARS} characters"
                    );
                }
            }
            Self::Insert { text } | Self::ReplaceSelection { text } => {
                if text.len() > MAX_EDITOR_TEXT_BYTES {
                    bail!("editor text exceeds the {MAX_EDITOR_TEXT_BYTES}-byte limit");
                }
            }
            Self::Navigate { .. } | Self::Action { .. } => {}
        }
        Ok(())
    }

    pub fn confirmation_copy(&self) -> String {
        match self {
            Self::ReplaceSelection { text } => format!(
                "Replace the current editor selection with {} character{}?",
                text.chars().count(),
                if text.chars().count() == 1 { "" } else { "s" }
            ),
            Self::Action {
                action: ApprovedEditorAction::Undo,
            } => "Undo the last editor change?".into(),
            Self::Action {
                action: ApprovedEditorAction::Redo,
            } => "Redo the last editor change?".into(),
            Self::Action {
                action: ApprovedEditorAction::SaveActiveFile,
            } => "Save the active file to disk?".into(),
            Self::ReadContext { .. } | Self::Navigate { .. } | Self::Insert { .. } => {
                "Run this Sarah editor command?".into()
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoiceCommandRequest {
    pub request_id: String,
    pub command: SarahEditorCommand,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandResultStatus {
    Completed,
    Rejected,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCommandResult {
    pub request_id: String,
    pub status: CommandResultStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl VoiceCommandResult {
    pub fn completed(request_id: String, output: Option<Value>) -> Self {
        Self {
            request_id,
            status: CommandResultStatus::Completed,
            output,
            message: None,
        }
    }

    pub fn rejected(request_id: String, message: impl Into<String>) -> Self {
        Self {
            request_id,
            status: CommandResultStatus::Rejected,
            output: None,
            message: Some(message.into()),
        }
    }

    pub fn failed(request_id: String, message: impl Into<String>) -> Self {
        Self {
            request_id,
            status: CommandResultStatus::Failed,
            output: None,
            message: Some(message.into()),
        }
    }
}

#[derive(Clone, Debug)]
pub enum SarahVoiceEvent {
    State(SarahVoiceState),
    Ready {
        session_id: String,
    },
    TranscriptDelta {
        item_id: String,
        participant: VoiceParticipant,
        delta: String,
    },
    TranscriptCompleted {
        item_id: String,
        participant: VoiceParticipant,
        text: String,
    },
    CommandRequest(VoiceCommandRequest),
    Error {
        message: String,
        retryable: bool,
        action: Option<String>,
    },
    Ended {
        reason: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub enum SarahVoiceControl {
    SetMuted(bool),
    Interrupt,
    CommandResult(VoiceCommandResult),
    Close,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioFormat {
    encoding: &'static str,
    sample_rate: u32,
    channels: u8,
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            encoding: "pcm_s16le",
            sample_rate: SARAH_AUDIO_SAMPLE_RATE,
            channels: 1,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EditorBridgeCapabilities {
    protocol_version: u16,
    commands: [&'static str; 5],
    approved_actions: [&'static str; 3],
    confirmation_required_for: [&'static str; 2],
}

impl Default for EditorBridgeCapabilities {
    fn default() -> Self {
        Self {
            protocol_version: 1,
            commands: [
                "read_context",
                "navigate",
                "insert",
                "replace_selection",
                "action",
            ],
            approved_actions: ["undo", "redo", "save_active_file"],
            confirmation_required_for: ["destructive", "external_effect"],
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartSession {
    protocol_version: u16,
    client_session_id: String,
    model: &'static str,
    input_audio: AudioFormat,
    output_audio: AudioFormat,
    editor_bridge: EditorBridgeCapabilities,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
enum ClientMessage {
    #[serde(rename = "session.start")]
    StartSession {
        #[serde(flatten)]
        session: StartSession,
    },
    #[serde(rename = "response.cancel")]
    CancelResponse,
    #[serde(rename = "command.result")]
    CommandResult {
        #[serde(flatten)]
        result: VoiceCommandResult,
    },
    #[serde(rename = "session.close")]
    CloseSession,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ServerMessage {
    #[serde(rename = "session.ready")]
    SessionReady { session_id: String },
    #[serde(rename = "session.state")]
    SessionState { state: GatewayVoiceState },
    #[serde(rename = "transcript.delta")]
    TranscriptDelta {
        item_id: String,
        participant: VoiceParticipant,
        delta: String,
    },
    #[serde(rename = "transcript.completed")]
    TranscriptCompleted {
        item_id: String,
        participant: VoiceParticipant,
        text: String,
    },
    #[serde(rename = "command.request")]
    CommandRequest { request_id: String, command: Value },
    #[serde(rename = "error")]
    Error {
        message: String,
        #[serde(default)]
        retryable: bool,
        #[serde(default)]
        action: Option<String>,
    },
    #[serde(rename = "session.ended")]
    SessionEnded {
        #[serde(default)]
        reason: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GatewayVoiceState {
    Listening,
    UserSpeaking,
    SarahSpeaking,
}

impl From<GatewayVoiceState> for SarahVoiceState {
    fn from(state: GatewayVoiceState) -> Self {
        match state {
            GatewayVoiceState::Listening => Self::Listening,
            GatewayVoiceState::UserSpeaking => Self::UserSpeaking,
            GatewayVoiceState::SarahSpeaking => Self::SarahSpeaking,
        }
    }
}

pub struct SarahVoiceConnection {
    pub controls: async_channel::Sender<SarahVoiceControl>,
    pub events: async_channel::Receiver<SarahVoiceEvent>,
}

pub struct ManagedSarahVoiceClient {
    endpoint: Url,
    access_token: String,
    input_device_id: Option<DeviceId>,
    output_device_id: Option<DeviceId>,
}

impl ManagedSarahVoiceClient {
    pub fn from_verified_session(
        session: VerifiedOpenAgentsSession,
        input_device_id: Option<DeviceId>,
        output_device_id: Option<DeviceId>,
    ) -> Result<Self> {
        let endpoint = voice_gateway_url(&session.base_url)?;
        if session.access_token.trim().is_empty() {
            bail!("OpenAgents session did not provide an access token");
        }
        Ok(Self {
            endpoint,
            access_token: session.access_token,
            input_device_id,
            output_device_id,
        })
    }

    pub fn connect(self) -> SarahVoiceConnection {
        let (control_sender, control_receiver) = async_channel::bounded(32);
        let (event_sender, event_receiver) = async_channel::bounded(128);
        smol::spawn(async move {
            if let Err(error) = self.run(control_receiver, event_sender.clone()).await {
                let (message, action) = actionable_error(&error);
                if event_sender
                    .send(SarahVoiceEvent::Error {
                        message,
                        retryable: true,
                        action,
                    })
                    .await
                    .is_err()
                {
                    log::debug!("Sarah voice event receiver closed while reporting an error");
                }
            }
        })
        .detach();
        SarahVoiceConnection {
            controls: control_sender,
            events: event_receiver,
        }
    }

    async fn run(
        self,
        controls: async_channel::Receiver<SarahVoiceControl>,
        events: async_channel::Sender<SarahVoiceEvent>,
    ) -> Result<()> {
        events
            .send(SarahVoiceEvent::State(
                SarahVoiceState::RequestingMicrophone,
            ))
            .await
            .context("reporting microphone state")?;
        let microphone = MicrophoneCapture::start(self.input_device_id)
            .context("opening the selected microphone")?;
        let mut playback =
            VoicePlayback::open(self.output_device_id).context("opening the selected speaker")?;

        events
            .send(SarahVoiceEvent::State(SarahVoiceState::Connecting))
            .await
            .context("reporting connecting state")?;
        let mut request = self
            .endpoint
            .as_str()
            .into_client_request()
            .context("building Sarah voice WebSocket request")?;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.access_token))
                .context("encoding OpenAgents authorization")?,
        );
        request
            .headers_mut()
            .insert("x-openagents-sarah-protocol", HeaderValue::from_static("1"));

        let websocket_config = WebSocketConfig::default()
            .max_message_size(Some(MAX_GATEWAY_FRAME_BYTES))
            .max_frame_size(Some(MAX_GATEWAY_FRAME_BYTES));
        let (mut socket, _) = async_tungstenite::async_std::connect_async_with_config(
            request,
            Some(websocket_config),
        )
        .await
        .context("connecting to the managed Sarah voice gateway")?;
        send_client_message(
            &mut socket,
            &ClientMessage::StartSession {
                session: StartSession {
                    protocol_version: SARAH_VOICE_PROTOCOL_VERSION,
                    client_session_id: uuid::Uuid::new_v4().to_string(),
                    model: SARAH_VOICE_MODEL,
                    input_audio: AudioFormat::default(),
                    output_audio: AudioFormat::default(),
                    editor_bridge: EditorBridgeCapabilities::default(),
                },
            },
        )
        .await?;

        loop {
            let incoming = socket.next().fuse();
            let control = controls.recv().fuse();
            let audio = microphone.receiver.recv().fuse();
            pin_mut!(incoming, control, audio);
            select_biased! {
                control = control => {
                    match control {
                        Ok(SarahVoiceControl::SetMuted(muted)) => {
                            microphone.muted.store(muted, Ordering::Release);
                        }
                        Ok(SarahVoiceControl::Interrupt) => {
                            playback = playback.reopen()?;
                            send_client_message(&mut socket, &ClientMessage::CancelResponse).await?;
                        }
                        Ok(SarahVoiceControl::CommandResult(result)) => {
                            send_client_message(
                                &mut socket,
                                &ClientMessage::CommandResult { result },
                            ).await?;
                        }
                        Ok(SarahVoiceControl::Close) | Err(_) => {
                            if let Err(error) =
                                send_client_message(&mut socket, &ClientMessage::CloseSession).await
                            {
                                log::debug!("Sarah voice close message failed: {error:#}");
                            }
                            if let Err(error) = socket.close(None).await {
                                log::debug!("Sarah voice socket close failed: {error}");
                            }
                            events.send(SarahVoiceEvent::Ended {
                                reason: Some("ended_by_user".into()),
                            }).await.context("reporting Sarah voice close")?;
                            return Ok(());
                        }
                    }
                }
                audio = audio => {
                    let audio = audio.context("microphone capture stopped")?;
                    socket
                        .send(Message::Binary(audio.into()))
                        .await
                        .context("sending microphone audio")?;
                }
                incoming = incoming => {
                    let Some(incoming) = incoming else {
                        events.send(SarahVoiceEvent::Ended {
                            reason: Some("gateway_disconnected".into()),
                        }).await.context("reporting Sarah voice disconnect")?;
                        return Ok(());
                    };
                    match incoming.context("reading Sarah voice gateway frame")? {
                        Message::Text(text) => {
                            if handle_server_message(
                                text.as_str(),
                                &mut socket,
                                &events,
                            ).await? {
                                return Ok(());
                            }
                        }
                        Message::Binary(bytes) => playback.play_pcm16(&bytes)?,
                        Message::Ping(payload) => {
                            socket.send(Message::Pong(payload)).await.context("replying to Sarah voice ping")?;
                        }
                        Message::Pong(_) | Message::Frame(_) => {}
                        Message::Close(frame) => {
                            events.send(SarahVoiceEvent::Ended {
                                reason: frame.map(|frame| frame.reason.to_string()),
                            }).await.context("reporting Sarah voice gateway close")?;
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
}

pub fn voice_gateway_url(base_url: &str) -> Result<Url> {
    let mut url = Url::parse(base_url).context("parsing OpenAgents base URL")?;
    match url.scheme() {
        "https" => url
            .set_scheme("wss")
            .map_err(|_| anyhow!("could not select the secure WebSocket scheme"))?,
        "http" => url
            .set_scheme("ws")
            .map_err(|_| anyhow!("could not select the WebSocket scheme"))?,
        scheme => bail!("unsupported OpenAgents URL scheme {scheme}"),
    }
    url.set_path(SARAH_VOICE_GATEWAY_PATH);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

async fn send_client_message(
    socket: &mut async_tungstenite::WebSocketStream<async_tungstenite::async_std::ConnectStream>,
    message: &ClientMessage,
) -> Result<()> {
    let text = serde_json::to_string(message).context("encoding Sarah voice message")?;
    socket
        .send(Message::Text(text.into()))
        .await
        .context("sending Sarah voice message")
}

async fn handle_server_message(
    text: &str,
    socket: &mut async_tungstenite::WebSocketStream<async_tungstenite::async_std::ConnectStream>,
    events: &async_channel::Sender<SarahVoiceEvent>,
) -> Result<bool> {
    let message: ServerMessage =
        serde_json::from_str(text).context("decoding Sarah voice gateway message")?;
    match message {
        ServerMessage::SessionReady { session_id } => {
            validate_gateway_text("session id", &session_id, MAX_PROTOCOL_ID_BYTES)?;
            events
                .send(SarahVoiceEvent::Ready { session_id })
                .await
                .context("reporting Sarah voice readiness")?;
        }
        ServerMessage::SessionState { state } => {
            events
                .send(SarahVoiceEvent::State(state.into()))
                .await
                .context("reporting Sarah voice state")?;
        }
        ServerMessage::TranscriptDelta {
            item_id,
            participant,
            delta,
        } => {
            validate_gateway_text("transcript item id", &item_id, MAX_PROTOCOL_ID_BYTES)?;
            validate_gateway_text("transcript delta", &delta, MAX_TRANSCRIPT_TEXT_BYTES)?;
            events
                .send(SarahVoiceEvent::TranscriptDelta {
                    item_id,
                    participant,
                    delta,
                })
                .await
                .context("reporting Sarah transcript delta")?;
        }
        ServerMessage::TranscriptCompleted {
            item_id,
            participant,
            text,
        } => {
            validate_gateway_text("transcript item id", &item_id, MAX_PROTOCOL_ID_BYTES)?;
            validate_gateway_text("completed transcript", &text, MAX_TRANSCRIPT_TEXT_BYTES)?;
            events
                .send(SarahVoiceEvent::TranscriptCompleted {
                    item_id,
                    participant,
                    text,
                })
                .await
                .context("reporting Sarah transcript completion")?;
        }
        ServerMessage::CommandRequest {
            request_id,
            command,
        } => {
            validate_gateway_text("command request id", &request_id, MAX_PROTOCOL_ID_BYTES)?;
            match serde_json::from_value::<SarahEditorCommand>(command) {
                Ok(command) => match command.validate() {
                    Ok(()) => {
                        events
                            .send(SarahVoiceEvent::CommandRequest(VoiceCommandRequest {
                                request_id,
                                command,
                            }))
                            .await
                            .context("reporting Sarah editor command")?;
                    }
                    Err(error) => {
                        send_client_message(
                            socket,
                            &ClientMessage::CommandResult {
                                result: VoiceCommandResult::rejected(
                                    request_id,
                                    format!("Command validation failed: {error}"),
                                ),
                            },
                        )
                        .await?;
                    }
                },
                Err(error) => {
                    send_client_message(
                        socket,
                        &ClientMessage::CommandResult {
                            result: VoiceCommandResult::rejected(
                                request_id,
                                format!("Command is not on Omega's allowlist: {error}"),
                            ),
                        },
                    )
                    .await?;
                }
            }
        }
        ServerMessage::Error {
            message,
            retryable,
            action,
        } => {
            validate_gateway_text("gateway error", &message, MAX_ERROR_TEXT_BYTES)?;
            if let Some(action) = &action {
                validate_gateway_text("gateway error action", action, MAX_ERROR_TEXT_BYTES)?;
            }
            events
                .send(SarahVoiceEvent::Error {
                    message,
                    retryable,
                    action,
                })
                .await
                .context("reporting Sarah voice gateway error")?;
        }
        ServerMessage::SessionEnded { reason } => {
            if let Some(reason) = &reason {
                validate_gateway_text("session end reason", reason, MAX_ERROR_TEXT_BYTES)?;
            }
            events
                .send(SarahVoiceEvent::Ended { reason })
                .await
                .context("reporting Sarah voice end")?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_gateway_text(label: &str, text: &str, max_bytes: usize) -> Result<()> {
    if text.is_empty() {
        bail!("Sarah voice {label} was empty");
    }
    if text.len() > max_bytes {
        bail!("Sarah voice {label} exceeded the {max_bytes}-byte limit");
    }
    Ok(())
}

struct MicrophoneCapture {
    receiver: async_channel::Receiver<Vec<u8>>,
    muted: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    _thread: Option<thread::JoinHandle<()>>,
}

impl MicrophoneCapture {
    fn start(input_device_id: Option<DeviceId>) -> Result<Self> {
        let microphone = audio::open_input_stream(input_device_id)?;
        let mut microphone = microphone
            .possibly_disconnected_channels_to_mono()
            .constant_samplerate(nz!(24_000));
        let (sender, receiver) = async_channel::bounded(25);
        let muted = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::Builder::new()
            .name("SarahMicrophoneCapture".into())
            .spawn({
                let muted = muted.clone();
                let stop = stop.clone();
                move || {
                    let mut samples = Vec::with_capacity(MICROPHONE_CHUNK_SAMPLES);
                    while !stop.load(Ordering::Acquire) {
                        let Some(sample) = microphone.next() else {
                            break;
                        };
                        samples.push(sample);
                        if samples.len() < MICROPHONE_CHUNK_SAMPLES {
                            continue;
                        }
                        if muted.load(Ordering::Acquire) {
                            samples.clear();
                            continue;
                        }
                        let bytes = encode_pcm16(&samples);
                        samples.clear();
                        if sender.send_blocking(bytes).is_err() {
                            break;
                        }
                    }
                }
            })
            .context("starting Sarah microphone capture")?;
        Ok(Self {
            receiver,
            muted,
            stop,
            _thread: Some(thread),
        })
    }
}

impl Drop for MicrophoneCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

struct VoicePlayback {
    output_device_id: Option<DeviceId>,
    output: rodio::MixerDeviceSink,
}

impl VoicePlayback {
    fn open(output_device_id: Option<DeviceId>) -> Result<Self> {
        let output = audio::open_test_output(output_device_id.clone())?;
        Ok(Self {
            output_device_id,
            output,
        })
    }

    fn reopen(self) -> Result<Self> {
        Self::open(self.output_device_id)
    }

    fn play_pcm16(&self, bytes: &[u8]) -> Result<()> {
        if !bytes.len().is_multiple_of(2) {
            bail!("Sarah audio frame had an odd byte count");
        }
        let samples = decode_pcm16(bytes);
        self.output
            .mixer()
            .add(SamplesBuffer::new(nz!(1), nz!(24_000), samples));
        Ok(())
    }
}

fn encode_pcm16(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let sample = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

fn decode_pcm16(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as f32 / i16::MAX as f32)
        .collect()
}

fn actionable_error(error: &anyhow::Error) -> (String, Option<String>) {
    let message = format!("{error:#}");
    let lowercase = message.to_lowercase();
    if lowercase.contains("microphone")
        || lowercase.contains("input device")
        || lowercase.contains("permission")
    {
        (
            format!(
                "Sarah could not use the microphone. Check Omega's microphone permission and the selected input device. ({message})"
            ),
            Some("Open Collaboration settings to choose or test a microphone.".into()),
        )
    } else if lowercase.contains("speaker") || lowercase.contains("output device") {
        (
            format!(
                "Sarah could not use the selected speaker. Choose or test another output device. ({message})"
            ),
            Some("Open Collaboration settings to choose or test a speaker.".into()),
        )
    } else {
        (
            format!("Sarah voice connection failed. Check your network and retry. ({message})"),
            Some("Retry the voice session when connectivity is restored.".into()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn managed_gateway_is_same_origin_and_never_puts_credentials_in_the_url() {
        let url = voice_gateway_url("https://openagents.com/account?token=forbidden")
            .expect("valid managed gateway URL");
        assert_eq!(url.as_str(), "wss://openagents.com/api/sarah/realtime");
        assert!(!url.as_str().contains("token"));
    }

    #[test]
    fn editor_command_allowlist_has_explicit_confirmation_policy() {
        assert_eq!(
            SarahEditorCommand::ReadContext { max_chars: None }.confirmation(),
            CommandConfirmation::None
        );
        assert_eq!(
            SarahEditorCommand::ReplaceSelection { text: "new".into() }.confirmation(),
            CommandConfirmation::Destructive
        );
        assert_eq!(
            SarahEditorCommand::Action {
                action: ApprovedEditorAction::SaveActiveFile
            }
            .confirmation(),
            CommandConfirmation::ExternalEffect
        );
    }

    #[test]
    fn arbitrary_model_commands_are_not_decodable() {
        let command = serde_json::from_value::<SarahEditorCommand>(json!({
            "name": "run_shell",
            "command": "rm -rf /"
        }));
        assert!(command.is_err());
    }

    #[test]
    fn oversized_edits_fail_validation() {
        let command = SarahEditorCommand::Insert {
            text: "x".repeat(MAX_EDITOR_TEXT_BYTES + 1),
        };
        assert!(command.validate().is_err());
    }

    #[test]
    fn command_contract_uses_camel_case_fields_and_snake_case_names() {
        let command = serde_json::to_value(SarahEditorCommand::ReadContext {
            max_chars: Some(1024),
        })
        .expect("serialize editor command");
        assert_eq!(command["name"], "read_context");
        assert_eq!(command["maxChars"], 1024);
        assert!(command.get("max_chars").is_none());

        let result = serde_json::to_value(VoiceCommandResult::completed(
            "command.fixture".into(),
            None,
        ))
        .expect("serialize command result");
        assert_eq!(result["requestId"], "command.fixture");
        assert!(result.get("request_id").is_none());
    }

    #[test]
    fn gateway_text_fields_are_bounded() {
        assert!(validate_gateway_text("fixture", "ok", 2).is_ok());
        assert!(validate_gateway_text("fixture", "", 2).is_err());
        assert!(validate_gateway_text("fixture", "too long", 2).is_err());
    }

    #[test]
    fn pcm16_round_trip_preserves_sign_and_bounds() {
        let input = [-1.0, -0.5, 0.0, 0.5, 1.0];
        let decoded = decode_pcm16(&encode_pcm16(&input));
        assert_eq!(decoded.len(), input.len());
        for (actual, expected) in decoded.iter().zip(input) {
            assert!((actual - expected).abs() < 0.001);
        }
    }

    #[test]
    fn start_session_pins_the_managed_model_and_bridge_capabilities() {
        let value = serde_json::to_value(ClientMessage::StartSession {
            session: StartSession {
                protocol_version: SARAH_VOICE_PROTOCOL_VERSION,
                client_session_id: "client.fixture".into(),
                model: SARAH_VOICE_MODEL,
                input_audio: AudioFormat::default(),
                output_audio: AudioFormat::default(),
                editor_bridge: EditorBridgeCapabilities::default(),
            },
        })
        .expect("serialize start session");
        assert_eq!(value["type"], "session.start");
        assert_eq!(value["model"], "gpt-realtime-2.1");
        assert_eq!(value["editorBridge"]["protocolVersion"], 1);
    }
}
