use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail};
use async_tungstenite::tungstenite::{
    Message, client::IntoClientRequest as _, http::HeaderValue, protocol::WebSocketConfig,
};
use cpal::DeviceId;
use futures::{FutureExt as _, StreamExt as _, pin_mut, select_biased};
use rodio::{buffer::SamplesBuffer, nz};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use url::Url;

use audio::RodioExt as _;
use omega_effectd::{
    ManagedSarahVoiceSession, SARAH_VOICE_SESSION_HEADER, SARAH_VOICE_TICKET_HEADER,
};

pub const SARAH_VOICE_MODEL: &str = "gpt-realtime-2.1";
pub const SARAH_AUDIO_SAMPLE_RATE: u32 = 24_000;
const MANAGED_SARAH_PROTOCOL: &str = "openagents.sarah.voice.v1";
const AUDIO_PROTOCOL: &str = "openagents.audio.v1";
const AUDIO_MEDIA_MAGIC: &[u8; 4] = b"OAA1";
const MICROPHONE_CHUNK_SAMPLES: usize = 480;
const MAX_EDITOR_TEXT_BYTES: usize = 64 * 1024;
const MAX_AGENT_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_CONTEXT_CHARS: u32 = 16 * 1024;
const MAX_GATEWAY_FRAME_BYTES: usize = 256 * 1024;
const MAX_PROTOCOL_ID_BYTES: usize = 256;
const MAX_TRANSCRIPT_TEXT_BYTES: usize = 64 * 1024;
const MAX_ERROR_TEXT_BYTES: usize = 4 * 1024;
const VOICE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const VOICE_TRANSCRIPT_SCHEMA: &str = "openagents.sarah.voice.transcript.v1";

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
    StartAgentThread {
        message: String,
        presentation: AgentThreadPresentation,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentThreadPresentation {
    Foreground,
    Background,
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
            }
            | Self::StartAgentThread { .. } => CommandConfirmation::ExternalEffect,
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
            Self::StartAgentThread { message, .. } => {
                if message.trim().is_empty() {
                    bail!("the Omega Agent message must not be empty");
                }
                if message.len() > MAX_AGENT_MESSAGE_BYTES {
                    bail!(
                        "the Omega Agent message exceeds the {MAX_AGENT_MESSAGE_BYTES}-byte limit"
                    );
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
            Self::StartAgentThread { presentation, .. } => match presentation {
                AgentThreadPresentation::Foreground => {
                    "Create an Omega Agent thread, submit this message, and open it now?".into()
                }
                AgentThreadPresentation::Background => {
                    "Create an Omega Agent thread and submit this message in the background?".into()
                }
            },
            Self::ReadContext { .. } | Self::Navigate { .. } | Self::Insert { .. } => {
                "Run this Sarah editor command?".into()
            }
        }
    }

    pub fn confirmation_detail(&self) -> Option<&str> {
        match self {
            Self::StartAgentThread { message, .. } => Some(message),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoiceCommandRequest {
    pub request_id: String,
    pub command: SarahEditorCommand,
    pub expected_path: Option<String>,
    pub expires_at_ms: Option<u64>,
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
    CommandProposal(VoiceCommandRequest),
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
    CommandDecision { request_id: String, approved: bool },
    CommandResult(VoiceCommandResult),
    Close,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GatewayVoiceIdentity {
    owner_ref: String,
    device_ref: String,
    thread_ref: String,
    session_ref: String,
    generation: u32,
}

impl From<&ManagedSarahVoiceSession> for GatewayVoiceIdentity {
    fn from(session: &ManagedSarahVoiceSession) -> Self {
        Self {
            owner_ref: session.owner_ref.clone(),
            device_ref: session.device_ref.clone(),
            thread_ref: session.thread_ref.clone(),
            session_ref: session.session_ref.clone(),
            generation: session.generation,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GatewayServerEnvelope {
    schema: String,
    identity: GatewayVoiceIdentity,
    sequence: u64,
    #[serde(flatten)]
    message: GatewayServerMessage,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "_tag",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum GatewayServerMessage {
    SessionReady {
        model: String,
        expires_at_ms: u64,
        reserved_credit_msat: u64,
    },
    Lifecycle {
        state: GatewayLifecycleState,
    },
    TranscriptDelta {
        source: GatewayTranscriptSource,
        utterance_ref: String,
        text: String,
    },
    TranscriptFinal {
        source: GatewayTranscriptSource,
        utterance_ref: String,
        text: String,
    },
    InterruptAck,
    ToolProposal {
        proposal_ref: String,
        proposal_digest: String,
        command: Value,
        confirmation_required: bool,
        expires_at_ms: u64,
    },
    ToolExecute {
        proposal_ref: String,
        proposal_digest: String,
        command: Value,
    },
    ToolOutcomeRef {
        proposal_ref: String,
        outcome_ref: String,
    },
    AudioAck {
        acknowledged_client_sequence: u64,
    },
    Heartbeat,
    Error {
        code: GatewayErrorCode,
        retryable: bool,
    },
    Closing {
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GatewayErrorCode {
    InvalidFrame,
    SequenceGap,
    ToolNotAllowed,
    ConfirmationRequired,
    ProviderUnavailable,
    CreditLimit,
    SessionExpired,
    Internal,
}

impl GatewayErrorCode {
    fn label(self) -> &'static str {
        match self {
            Self::InvalidFrame => "invalid frame",
            Self::SequenceGap => "sequence gap",
            Self::ToolNotAllowed => "tool not allowed",
            Self::ConfirmationRequired => "confirmation required",
            Self::ProviderUnavailable => "provider unavailable",
            Self::CreditLimit => "credit limit reached",
            Self::SessionExpired => "session expired",
            Self::Internal => "internal error",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GatewayLifecycleState {
    Connecting,
    Listening,
    Thinking,
    Speaking,
    Interrupted,
    Closing,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GatewayTranscriptSource {
    User,
    Assistant,
}

pub struct SarahVoiceConnection {
    pub controls: async_channel::Sender<SarahVoiceControl>,
    pub events: async_channel::Receiver<SarahVoiceEvent>,
}

struct PendingGatewayTool {
    proposal_digest: String,
    command: Value,
    confirmation_required: bool,
    confirmation_granted: bool,
    outcome_sent: bool,
    expires_at_ms: u64,
}

pub struct ManagedSarahVoiceClient {
    endpoint: Url,
    managed_session: ManagedSarahVoiceSession,
    input_device_id: Option<DeviceId>,
    output_device_id: Option<DeviceId>,
}

impl ManagedSarahVoiceClient {
    pub fn from_managed_session(
        managed_session: ManagedSarahVoiceSession,
        input_device_id: Option<DeviceId>,
        output_device_id: Option<DeviceId>,
    ) -> Result<Self> {
        let endpoint =
            Url::parse(&managed_session.gateway_url).context("parsing Sarah gateway URL")?;
        if endpoint.scheme() != "wss"
            || endpoint.host_str() != Some("openagents.com")
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            bail!("OpenAgents returned an invalid Sarah gateway URL");
        }
        Ok(Self {
            endpoint,
            managed_session,
            input_device_id,
            output_device_id,
        })
    }

    pub fn connect(self) -> SarahVoiceConnection {
        let (control_sender, control_receiver) = async_channel::bounded(32);
        let (event_sender, event_receiver) = async_channel::bounded(128);
        smol::spawn(async move {
            if let Err(error) = self.run(control_receiver, event_sender.clone()).await {
                log::error!("Sarah voice session failed: {error:#}");
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
        let echo_canceller = audio::EchoCanceller::default();
        let microphone = MicrophoneCapture::start(self.input_device_id, echo_canceller.clone())
            .context("opening the selected microphone")?;
        let mut playback = VoicePlayback::open(self.output_device_id, echo_canceller)
            .context("opening the selected speaker")?;

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
            SARAH_VOICE_SESSION_HEADER,
            HeaderValue::from_str(&self.managed_session.session_ref)
                .context("encoding Sarah voice session reference")?,
        );
        request.headers_mut().insert(
            SARAH_VOICE_TICKET_HEADER,
            HeaderValue::from_str(&self.managed_session.ticket)
                .context("encoding Sarah voice ticket")?,
        );

        let websocket_config = WebSocketConfig::default()
            .max_message_size(Some(MAX_GATEWAY_FRAME_BYTES))
            .max_frame_size(Some(MAX_GATEWAY_FRAME_BYTES));
        let (mut socket, _) = async_tungstenite::async_std::connect_async_with_config(
            request,
            Some(websocket_config),
        )
        .await
        .context("connecting to the managed Sarah voice gateway")?;
        let identity = GatewayVoiceIdentity::from(&self.managed_session);
        let transcript_log = LocalVoiceTranscriptLog::new(
            paths::data_dir().join("voice").join("transcripts.jsonl"),
            identity.session_ref.clone(),
        );
        let mut control_sequence = 0_u64;
        let mut audio_sequence = 0_u64;
        let mut expected_server_sequence = 0_u64;
        let mut expected_output_audio_sequence = 0_u64;
        let mut pending_tools = HashMap::<String, PendingGatewayTool>::new();
        send_gateway_control(
            &mut socket,
            &identity,
            &mut control_sequence,
            serde_json::json!({
                "_tag": "session_hello",
                "disclosureRef": self.managed_session.disclosure_ref,
            }),
        )
        .await?;
        let mut next_heartbeat_at = Instant::now() + VOICE_HEARTBEAT_INTERVAL;

        loop {
            let incoming = socket.next().fuse();
            let control = controls.recv().fuse();
            let audio = microphone.receiver.recv().fuse();
            let heartbeat = futures::FutureExt::fuse(smol::Timer::after(
                next_heartbeat_at.saturating_duration_since(Instant::now()),
            ));
            pin_mut!(incoming, control, audio, heartbeat);
            select_biased! {
                control = control => {
                    match control {
                        Ok(SarahVoiceControl::SetMuted(muted)) => {
                            microphone.muted.store(muted, Ordering::Release);
                        }
                        Ok(SarahVoiceControl::Interrupt) => {
                            playback = playback.reopen()?;
                            send_gateway_control(
                                &mut socket,
                                &identity,
                                &mut control_sequence,
                                serde_json::json!({ "_tag": "interrupt" }),
                            ).await?;
                        }
                        Ok(SarahVoiceControl::CommandDecision {
                            request_id,
                            approved,
                        }) => {
                            send_gateway_tool_decision(
                                &mut socket,
                                &identity,
                                &mut control_sequence,
                                &mut pending_tools,
                                &request_id,
                                approved,
                            ).await?;
                        }
                        Ok(SarahVoiceControl::CommandResult(result)) => {
                            send_gateway_tool_result(
                                &mut socket,
                                &identity,
                                &mut control_sequence,
                                &mut pending_tools,
                                result,
                            ).await?;
                        }
                        Ok(SarahVoiceControl::Close) | Err(_) => {
                            if let Err(error) =
                                send_gateway_control(
                                    &mut socket,
                                    &identity,
                                    &mut control_sequence,
                                    serde_json::json!({
                                        "_tag": "close",
                                        "reason": "user_stop",
                                    }),
                                ).await
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
                incoming = incoming => {
                    let Some(incoming) = incoming else {
                        events.send(SarahVoiceEvent::Ended {
                            reason: Some("gateway_disconnected".into()),
                        }).await.context("reporting Sarah voice disconnect")?;
                        return Ok(());
                    };
                    match incoming.context("reading Sarah voice gateway frame")? {
                        Message::Text(text) => {
                            match handle_server_message(
                                text.as_str(),
                                &events,
                                &identity,
                                &mut expected_server_sequence,
                                &mut pending_tools,
                                &transcript_log,
                            ).await? {
                                GatewayServerAction::Continue => {}
                                GatewayServerAction::StopPlayback => {
                                    playback = playback.reopen()?;
                                }
                                GatewayServerAction::End => return Ok(()),
                            }
                        }
                        Message::Binary(bytes) => {
                            let audio = decode_server_audio(
                                &bytes,
                                &identity,
                                &mut expected_output_audio_sequence,
                            )?;
                            playback.play_pcm16(audio)?;
                        }
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
                _ = heartbeat => {
                    send_gateway_control(
                        &mut socket,
                        &identity,
                        &mut control_sequence,
                        serde_json::json!({ "_tag": "heartbeat" }),
                    )
                    .await
                    .context("sending Sarah voice heartbeat")?;
                    next_heartbeat_at = Instant::now() + VOICE_HEARTBEAT_INTERVAL;
                }
                audio = audio => {
                    let audio = audio.context("microphone capture stopped")?;
                    let frame = encode_client_audio(&identity, audio_sequence, &audio)?;
                    audio_sequence = audio_sequence.saturating_add(1);
                    socket
                        .send(Message::Binary(frame.into()))
                        .await
                        .context("sending microphone audio")?;
                }
            }
        }
    }
}

async fn send_gateway_control(
    socket: &mut async_tungstenite::WebSocketStream<async_tungstenite::async_std::ConnectStream>,
    identity: &GatewayVoiceIdentity,
    sequence: &mut u64,
    message: Value,
) -> Result<()> {
    let mut envelope = serde_json::Map::from_iter([
        (
            "schema".to_string(),
            Value::String(MANAGED_SARAH_PROTOCOL.to_string()),
        ),
        (
            "identity".to_string(),
            serde_json::to_value(identity).context("encoding Sarah voice identity")?,
        ),
        ("sequence".to_string(), Value::from(*sequence)),
    ]);
    let Value::Object(message) = message else {
        bail!("Sarah voice control payload was not an object");
    };
    envelope.extend(message);
    let text = serde_json::to_string(&envelope).context("encoding Sarah voice control message")?;
    socket
        .send(Message::Text(text.into()))
        .await
        .context("sending Sarah voice control message")?;
    *sequence = sequence.saturating_add(1);
    Ok(())
}

async fn send_gateway_tool_result(
    socket: &mut async_tungstenite::WebSocketStream<async_tungstenite::async_std::ConnectStream>,
    identity: &GatewayVoiceIdentity,
    sequence: &mut u64,
    pending_tools: &mut HashMap<String, PendingGatewayTool>,
    result: VoiceCommandResult,
) -> Result<()> {
    let Some(tool) = pending_tools.get_mut(&result.request_id) else {
        bail!("Sarah voice tool result did not match a pending proposal");
    };
    if tool.outcome_sent {
        bail!("Sarah voice tool result was already sent");
    }
    if tool.confirmation_required && !tool.confirmation_granted {
        bail!("Sarah voice tool result arrived before confirmation");
    }
    let summary = result
        .message
        .or_else(|| {
            result
                .output
                .and_then(|output| serde_json::to_string(&output).ok())
        })
        .unwrap_or_else(|| "Command completed.".to_string());
    let summary = truncate_utf8_bytes(summary, 2_048);
    send_gateway_control(
        socket,
        identity,
        sequence,
        serde_json::json!({
            "_tag": "tool_outcome",
            "proposalRef": result.request_id,
            "proposalDigest": tool.proposal_digest,
            "outcomeRef": format!("omega-outcome-{}", uuid::Uuid::new_v4()),
            "ok": result.status == CommandResultStatus::Completed,
            "summary": summary,
        }),
    )
    .await?;
    tool.outcome_sent = true;
    Ok(())
}

async fn send_gateway_tool_decision(
    socket: &mut async_tungstenite::WebSocketStream<async_tungstenite::async_std::ConnectStream>,
    identity: &GatewayVoiceIdentity,
    sequence: &mut u64,
    pending_tools: &mut HashMap<String, PendingGatewayTool>,
    request_id: &str,
    approved: bool,
) -> Result<()> {
    let Some(tool) = pending_tools.get_mut(request_id) else {
        bail!("Sarah voice decision did not match a pending proposal");
    };
    if !tool.confirmation_required || tool.confirmation_granted || tool.outcome_sent {
        bail!("Sarah voice proposal was not awaiting confirmation");
    }
    send_gateway_control(
        socket,
        identity,
        sequence,
        serde_json::json!({
            "_tag": "tool_decision",
            "proposalRef": request_id,
            "proposalDigest": tool.proposal_digest,
            "decision": if approved { "confirm" } else { "decline" },
        }),
    )
    .await?;
    if approved {
        tool.confirmation_granted = true;
    } else {
        pending_tools.remove(request_id);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GatewayServerAction {
    Continue,
    StopPlayback,
    End,
}

async fn handle_server_message(
    text: &str,
    events: &async_channel::Sender<SarahVoiceEvent>,
    identity: &GatewayVoiceIdentity,
    expected_sequence: &mut u64,
    pending_tools: &mut HashMap<String, PendingGatewayTool>,
    transcript_log: &LocalVoiceTranscriptLog,
) -> Result<GatewayServerAction> {
    let envelope = decode_gateway_server_envelope(text)?;
    if envelope.schema != MANAGED_SARAH_PROTOCOL
        || envelope.identity != *identity
        || envelope.sequence != *expected_sequence
    {
        bail!("Sarah voice gateway sent an invalid identity or sequence");
    }
    *expected_sequence = expected_sequence.saturating_add(1);
    match envelope.message {
        GatewayServerMessage::SessionReady {
            model,
            expires_at_ms,
            reserved_credit_msat,
        } => {
            if model != SARAH_VOICE_MODEL || expires_at_ms == 0 || reserved_credit_msat == 0 {
                bail!("Sarah voice gateway returned invalid session readiness");
            }
            events
                .send(SarahVoiceEvent::Ready {
                    session_id: identity.session_ref.clone(),
                })
                .await
                .context("reporting Sarah voice readiness")?;
        }
        GatewayServerMessage::Lifecycle { state } => {
            let state = match state {
                GatewayLifecycleState::Connecting => SarahVoiceState::Connecting,
                GatewayLifecycleState::Listening | GatewayLifecycleState::Interrupted => {
                    SarahVoiceState::Listening
                }
                GatewayLifecycleState::Thinking => SarahVoiceState::UserSpeaking,
                GatewayLifecycleState::Speaking => SarahVoiceState::SarahSpeaking,
                GatewayLifecycleState::Closing => SarahVoiceState::Ending,
            };
            events
                .send(SarahVoiceEvent::State(state))
                .await
                .context("reporting Sarah voice state")?;
        }
        GatewayServerMessage::TranscriptDelta {
            source,
            utterance_ref,
            text,
        } => {
            validate_gateway_text("transcript item id", &utterance_ref, MAX_PROTOCOL_ID_BYTES)?;
            validate_gateway_text("transcript delta", &text, MAX_TRANSCRIPT_TEXT_BYTES)?;
            events
                .send(SarahVoiceEvent::TranscriptDelta {
                    item_id: utterance_ref,
                    participant: gateway_participant(source),
                    delta: text,
                })
                .await
                .context("reporting Sarah transcript delta")?;
        }
        GatewayServerMessage::TranscriptFinal {
            source,
            utterance_ref,
            text,
        } => {
            validate_gateway_text("transcript item id", &utterance_ref, MAX_PROTOCOL_ID_BYTES)?;
            if text.trim().is_empty() {
                return Ok(GatewayServerAction::Continue);
            }
            if text.len() > MAX_TRANSCRIPT_TEXT_BYTES {
                bail!(
                    "Sarah voice completed transcript exceeded the {MAX_TRANSCRIPT_TEXT_BYTES}-byte limit"
                );
            }
            let participant = gateway_participant(source);
            transcript_log.append(&utterance_ref, participant, &text)?;
            events
                .send(SarahVoiceEvent::TranscriptCompleted {
                    item_id: utterance_ref,
                    participant,
                    text,
                })
                .await
                .context("reporting Sarah transcript completion")?;
        }
        GatewayServerMessage::ToolProposal {
            proposal_ref,
            proposal_digest,
            command,
            confirmation_required,
            expires_at_ms,
        } => {
            validate_proposal(&proposal_ref, &proposal_digest)?;
            if expires_at_ms == 0 {
                bail!("Sarah gateway tool proposal expiry was invalid");
            }
            let gateway_command = command;
            let (command, expected_path) = decode_gateway_command(gateway_command.clone())?;
            if confirmation_required != (command.confirmation() != CommandConfirmation::None) {
                bail!("Sarah gateway confirmation policy did not match Omega's policy");
            }
            let previous = pending_tools.insert(
                proposal_ref.clone(),
                PendingGatewayTool {
                    proposal_digest,
                    command: gateway_command,
                    confirmation_required,
                    confirmation_granted: false,
                    outcome_sent: false,
                    expires_at_ms,
                },
            );
            if previous.is_some() {
                bail!("Sarah gateway reused a tool proposal reference");
            }
            if confirmation_required {
                events
                    .send(SarahVoiceEvent::CommandProposal(VoiceCommandRequest {
                        request_id: proposal_ref,
                        command,
                        expected_path,
                        expires_at_ms: Some(expires_at_ms),
                    }))
                    .await
                    .context("reporting Sarah editor command proposal")?;
            }
        }
        GatewayServerMessage::ToolExecute {
            proposal_ref,
            proposal_digest,
            command,
        } => {
            validate_proposal(&proposal_ref, &proposal_digest)?;
            let Some(tool) = pending_tools.get(&proposal_ref) else {
                bail!("Sarah gateway executed an unknown tool proposal");
            };
            if tool.proposal_digest != proposal_digest {
                bail!("Sarah gateway changed a tool proposal digest");
            }
            if tool.command != command {
                bail!("Sarah gateway changed a tool proposal command");
            }
            if tool.confirmation_required && !tool.confirmation_granted {
                bail!("Sarah gateway executed a tool before confirmation");
            }
            if !tool.outcome_sent {
                let (command, expected_path) = decode_gateway_command(command)?;
                events
                    .send(SarahVoiceEvent::CommandRequest(VoiceCommandRequest {
                        request_id: proposal_ref,
                        command,
                        expected_path,
                        expires_at_ms: Some(tool.expires_at_ms),
                    }))
                    .await
                    .context("reporting Sarah editor command execution")?;
            }
        }
        GatewayServerMessage::ToolOutcomeRef {
            proposal_ref,
            outcome_ref,
        } => {
            validate_gateway_text("tool proposal id", &proposal_ref, MAX_PROTOCOL_ID_BYTES)?;
            validate_gateway_text("tool outcome id", &outcome_ref, MAX_PROTOCOL_ID_BYTES)?;
            pending_tools.remove(&proposal_ref);
        }
        GatewayServerMessage::Error { code, retryable } => {
            events
                .send(SarahVoiceEvent::Error {
                    message: format!("Sarah voice gateway reported {}.", code.label()),
                    retryable,
                    action: Some("Retry if the problem persists.".to_string()),
                })
                .await
                .context("reporting Sarah voice gateway error")?;
        }
        GatewayServerMessage::Closing { reason } => {
            validate_gateway_text("session end reason", &reason, MAX_ERROR_TEXT_BYTES)?;
            events
                .send(SarahVoiceEvent::Ended {
                    reason: Some(reason),
                })
                .await
                .context("reporting Sarah voice end")?;
            return Ok(GatewayServerAction::End);
        }
        GatewayServerMessage::InterruptAck => return Ok(GatewayServerAction::StopPlayback),
        GatewayServerMessage::Heartbeat => {}
        GatewayServerMessage::AudioAck {
            acknowledged_client_sequence,
        } => {
            if acknowledged_client_sequence > 9_007_199_254_740_991 {
                bail!("Sarah voice gateway sent an invalid audio acknowledgement");
            }
        }
    }
    Ok(GatewayServerAction::Continue)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalVoiceTranscriptEntry<'a> {
    schema: &'static str,
    recorded_at: String,
    session_ref: &'a str,
    utterance_ref: &'a str,
    participant: VoiceParticipant,
    text: &'a str,
}

struct LocalVoiceTranscriptLog {
    path: PathBuf,
    session_ref: String,
}

impl LocalVoiceTranscriptLog {
    fn new(path: PathBuf, session_ref: String) -> Self {
        Self { path, session_ref }
    }

    fn append(&self, utterance_ref: &str, participant: VoiceParticipant, text: &str) -> Result<()> {
        append_local_voice_transcript(
            &self.path,
            &LocalVoiceTranscriptEntry {
                schema: VOICE_TRANSCRIPT_SCHEMA,
                recorded_at: chrono::Utc::now().to_rfc3339(),
                session_ref: &self.session_ref,
                utterance_ref,
                participant,
                text,
            },
        )
    }
}

fn append_local_voice_transcript(path: &Path, entry: &LocalVoiceTranscriptEntry<'_>) -> Result<()> {
    let parent = path
        .parent()
        .context("Sarah voice transcript path had no parent directory")?;
    std::fs::create_dir_all(parent).context("creating the local Sarah transcript directory")?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .context("opening the local Sarah transcript log")?;
    serde_json::to_writer(&mut file, entry).context("encoding a local Sarah transcript entry")?;
    file.write_all(b"\n")
        .context("writing a local Sarah transcript entry")?;
    file.sync_data()
        .context("persisting a local Sarah transcript entry")?;
    Ok(())
}

fn decode_gateway_server_envelope(text: &str) -> Result<GatewayServerEnvelope> {
    let value: Value =
        serde_json::from_str(text).context("decoding Sarah voice gateway message")?;
    let object = value
        .as_object()
        .context("Sarah voice gateway message was not an object")?;
    let tag = object
        .get("_tag")
        .and_then(Value::as_str)
        .context("Sarah voice gateway message omitted its tag")?;
    let variant_fields: &[&str] = match tag {
        "session_ready" => &["model", "expiresAtMs", "reservedCreditMsat"],
        "lifecycle" => &["state"],
        "transcript_delta" | "transcript_final" => &["source", "utteranceRef", "text"],
        "interrupt_ack" | "heartbeat" => &[],
        "tool_proposal" => &[
            "proposalRef",
            "proposalDigest",
            "command",
            "confirmationRequired",
            "expiresAtMs",
        ],
        "tool_execute" => &["proposalRef", "proposalDigest", "command"],
        "tool_outcome_ref" => &["proposalRef", "outcomeRef"],
        "audio_ack" => &["acknowledgedClientSequence"],
        "error" => &["code", "retryable"],
        "closing" => &["reason"],
        _ => bail!("Sarah voice gateway message had an unknown tag"),
    };
    if object.keys().any(|field| {
        !matches!(field.as_str(), "schema" | "identity" | "sequence" | "_tag")
            && !variant_fields.contains(&field.as_str())
    }) {
        bail!("Sarah voice gateway message contained an unknown field");
    }
    serde_json::from_value(value).context("decoding typed Sarah voice gateway message")
}

fn gateway_participant(source: GatewayTranscriptSource) -> VoiceParticipant {
    match source {
        GatewayTranscriptSource::User => VoiceParticipant::User,
        GatewayTranscriptSource::Assistant => VoiceParticipant::Sarah,
    }
}

fn validate_proposal(proposal_ref: &str, proposal_digest: &str) -> Result<()> {
    validate_gateway_text("tool proposal id", proposal_ref, MAX_PROTOCOL_ID_BYTES)?;
    if proposal_digest.len() != 64
        || !proposal_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        bail!("Sarah voice tool proposal digest was invalid");
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(
    tag = "_tag",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum GatewayEditorCommand {
    ContextRead {
        target: GatewayEditorTarget,
        start_line: u32,
        end_line: u32,
    },
    OpenPath {
        target: GatewayEditorTarget,
    },
    RevealRange {
        target: GatewayEditorTarget,
        start_line: u32,
        end_line: u32,
    },
    ReplaceSelection {
        target: GatewayEditorTarget,
        replacement: String,
    },
    SaveDocument {
        target: GatewayEditorTarget,
    },
    StartAgentThread {
        message: String,
        presentation: AgentThreadPresentation,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GatewayEditorTarget {
    workspace_ref: String,
    path: String,
    #[serde(default)]
    document_version: Option<String>,
}

fn decode_gateway_command(value: Value) -> Result<(SarahEditorCommand, Option<String>)> {
    let command: GatewayEditorCommand =
        serde_json::from_value(value).context("decoding Sarah gateway editor command")?;
    let (command, expected_path) = match command {
        GatewayEditorCommand::ContextRead {
            target,
            start_line,
            end_line,
        } => {
            validate_gateway_target(&target)?;
            if start_line == 0 || end_line < start_line || end_line - start_line > 500 {
                bail!("Sarah gateway context range was invalid");
            }
            (
                SarahEditorCommand::ReadContext {
                    max_chars: Some(MAX_CONTEXT_CHARS),
                },
                Some(target.path),
            )
        }
        GatewayEditorCommand::OpenPath { target } => {
            validate_gateway_target(&target)?;
            bail!("opening another file is not on Omega's local Sarah allowlist")
        }
        GatewayEditorCommand::RevealRange {
            target,
            start_line,
            end_line,
        } => {
            validate_gateway_target(&target)?;
            if start_line == 0 || end_line < start_line || end_line - start_line > 500 {
                bail!("Sarah gateway reveal range was invalid");
            }
            (
                SarahEditorCommand::Navigate {
                    line: start_line - 1,
                    column: 0,
                },
                Some(target.path),
            )
        }
        GatewayEditorCommand::ReplaceSelection {
            target,
            replacement,
        } => {
            validate_gateway_target(&target)?;
            (
                SarahEditorCommand::ReplaceSelection { text: replacement },
                Some(target.path),
            )
        }
        GatewayEditorCommand::SaveDocument { target } => {
            validate_gateway_target(&target)?;
            (
                SarahEditorCommand::Action {
                    action: ApprovedEditorAction::SaveActiveFile,
                },
                Some(target.path),
            )
        }
        GatewayEditorCommand::StartAgentThread {
            message,
            presentation,
        } => (
            SarahEditorCommand::StartAgentThread {
                message,
                presentation,
            },
            None,
        ),
    };
    command.validate()?;
    Ok((command, expected_path))
}

fn validate_gateway_target(target: &GatewayEditorTarget) -> Result<()> {
    validate_gateway_text(
        "workspace reference",
        &target.workspace_ref,
        MAX_PROTOCOL_ID_BYTES,
    )?;
    validate_gateway_text("editor path", &target.path, 1_024)?;
    if target.path.starts_with('/')
        || target
            .path
            .split(['/', '\\'])
            .any(|segment| segment == "..")
        || target.path.as_bytes().contains(&0)
    {
        bail!("Sarah gateway editor path was not workspace-relative");
    }
    if target
        .document_version
        .as_ref()
        .is_some_and(|version| version.len() > 2_048)
    {
        bail!("Sarah gateway document version was too large");
    }
    Ok(())
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

fn truncate_utf8_bytes(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.truncate(end);
    text
}

fn encode_client_audio(
    identity: &GatewayVoiceIdentity,
    sequence: u64,
    audio: &[u8],
) -> Result<Vec<u8>> {
    if audio.is_empty() || audio.len() > 24_000 || !audio.len().is_multiple_of(2) {
        bail!("microphone audio frame had an invalid size");
    }
    let header = serde_json::to_vec(&serde_json::json!({
        "schema": AUDIO_PROTOCOL,
        "kind": "client_audio",
        "identity": identity,
        "sequence": sequence,
        "codec": "pcm_s16le",
        "sampleRateHz": SARAH_AUDIO_SAMPLE_RATE,
        "channels": 1,
        "payloadLength": audio.len(),
        "sha256": format!("{:x}", Sha256::digest(audio)),
    }))
    .context("encoding Sarah audio header")?;
    let header_length = u32::try_from(header.len()).context("Sarah audio header was too large")?;
    let mut frame = Vec::with_capacity(8 + header.len() + audio.len());
    frame.extend_from_slice(AUDIO_MEDIA_MAGIC);
    frame.extend_from_slice(&header_length.to_be_bytes());
    frame.extend_from_slice(&header);
    frame.extend_from_slice(audio);
    Ok(frame)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GatewayAudioHeader {
    schema: String,
    kind: String,
    identity: GatewayVoiceIdentity,
    sequence: u64,
    turn_ref: String,
    speech_ref: String,
    codec: String,
    sample_rate_hz: u32,
    channels: u8,
    payload_length: usize,
    sha256: String,
}

fn decode_server_audio<'a>(
    frame: &'a [u8],
    identity: &GatewayVoiceIdentity,
    expected_sequence: &mut u64,
) -> Result<&'a [u8]> {
    if frame.len() < 8 || frame.get(..4) != Some(AUDIO_MEDIA_MAGIC.as_slice()) {
        bail!("Sarah audio frame had an invalid media marker");
    }
    let header_length_bytes: [u8; 4] = frame
        .get(4..8)
        .context("Sarah audio frame omitted its header length")?
        .try_into()
        .context("Sarah audio frame had an invalid header length")?;
    let header_length = u32::from_be_bytes(header_length_bytes) as usize;
    if !(2..=8_192).contains(&header_length) || 8 + header_length > frame.len() {
        bail!("Sarah audio frame header length was invalid");
    }
    let header: GatewayAudioHeader = serde_json::from_slice(
        frame
            .get(8..8 + header_length)
            .context("Sarah audio frame omitted its header")?,
    )
    .context("decoding Sarah audio header")?;
    let audio = frame
        .get(8 + header_length..)
        .context("Sarah audio frame omitted its payload")?;
    validate_gateway_text(
        "audio turn reference",
        &header.turn_ref,
        MAX_PROTOCOL_ID_BYTES,
    )?;
    validate_gateway_text(
        "audio speech reference",
        &header.speech_ref,
        MAX_PROTOCOL_ID_BYTES,
    )?;
    if header.schema != AUDIO_PROTOCOL
        || header.kind != "server_tts"
        || header.identity != *identity
        || header.sequence != *expected_sequence
        || header.codec != "pcm_s16le"
        || header.sample_rate_hz != SARAH_AUDIO_SAMPLE_RATE
        || header.channels != 1
        || header.payload_length != audio.len()
        || audio.is_empty()
        || audio.len() > 24_000
        || audio.len() % 2 != 0
        || header.sha256 != format!("{:x}", Sha256::digest(audio))
    {
        bail!("Sarah audio frame failed validation");
    }
    *expected_sequence = expected_sequence.saturating_add(1);
    Ok(audio)
}

struct MicrophoneCapture {
    receiver: async_channel::Receiver<Vec<u8>>,
    muted: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    _thread: Option<thread::JoinHandle<()>>,
}

impl MicrophoneCapture {
    fn start(
        input_device_id: Option<DeviceId>,
        mut echo_canceller: audio::EchoCanceller,
    ) -> Result<Self> {
        let microphone = audio::open_input_stream(input_device_id)?;
        let processing_failed = Arc::new(AtomicBool::new(false));
        let processing_failed_for_audio = processing_failed.clone();
        let mut microphone = microphone
            .constant_params(nz!(2), nz!(48_000))
            .process_buffer::<960, _>(move |buffer| {
                let mut pcm16: [i16; 960] = std::array::from_fn(|index| {
                    (buffer[index].clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
                });
                if let Err(error) = echo_canceller.process_stream(&mut pcm16) {
                    log::error!("Sarah microphone echo cancellation failed: {error:#}");
                    processing_failed_for_audio.store(true, Ordering::Release);
                    return;
                }
                for (sample, processed) in buffer.iter_mut().zip(pcm16) {
                    *sample = processed as f32 / i16::MAX as f32;
                }
            })
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
                    while !stop.load(Ordering::Acquire)
                        && !processing_failed.load(Ordering::Acquire)
                    {
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
    echo_canceller: audio::EchoCanceller,
    player: rodio::Player,
    _output: rodio::MixerDeviceSink,
}

impl VoicePlayback {
    fn open(
        output_device_id: Option<DeviceId>,
        echo_canceller: audio::EchoCanceller,
    ) -> Result<Self> {
        let (output, mixer) =
            audio::open_output_stream(output_device_id.clone(), echo_canceller.clone())?;
        let player = rodio::Player::connect_new(&mixer);
        Ok(Self {
            output_device_id,
            echo_canceller,
            player,
            _output: output,
        })
    }

    fn reopen(self) -> Result<Self> {
        Self::open(self.output_device_id, self.echo_canceller)
    }

    fn play_pcm16(&self, bytes: &[u8]) -> Result<()> {
        append_pcm16(&self.player, bytes)
    }
}

fn append_pcm16(player: &rodio::Player, bytes: &[u8]) -> Result<()> {
    if !bytes.len().is_multiple_of(2) {
        bail!("Sarah audio frame had an odd byte count");
    }
    let samples = decode_pcm16(bytes);
    player.append(SamplesBuffer::new(nz!(1), nz!(24_000), samples));
    Ok(())
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
    fn local_transcript_log_appends_durable_json_lines() {
        let directory = tempfile::tempdir().expect("create transcript test directory");
        let path = directory.path().join("voice/transcripts.jsonl");
        let log = LocalVoiceTranscriptLog::new(path.clone(), "session-1".into());

        log.append("user-1", VoiceParticipant::User, "Hello")
            .expect("append user transcript");
        log.append("sarah-1", VoiceParticipant::Sarah, "Hi there")
            .expect("append Sarah transcript");

        let contents = std::fs::read_to_string(path).expect("read transcript log");
        let entries = contents
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("decode transcript entry"))
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["schema"], VOICE_TRANSCRIPT_SCHEMA);
        assert_eq!(entries[0]["sessionRef"], "session-1");
        assert_eq!(entries[0]["utteranceRef"], "user-1");
        assert_eq!(entries[0]["participant"], "user");
        assert_eq!(entries[0]["text"], "Hello");
        assert_eq!(entries[1]["participant"], "sarah");
        assert_eq!(entries[1]["text"], "Hi there");
    }

    #[test]
    fn empty_final_transcript_does_not_end_the_voice_session() {
        smol::block_on(async {
            let directory = tempfile::tempdir().expect("create transcript test directory");
            let identity = GatewayVoiceIdentity {
                owner_ref: "owner-1".into(),
                device_ref: "device-1".into(),
                thread_ref: "thread-1".into(),
                session_ref: "session-1".into(),
                generation: 1,
            };
            let transcript_log = LocalVoiceTranscriptLog::new(
                directory.path().join("voice/transcripts.jsonl"),
                identity.session_ref.clone(),
            );
            let (events, received_events) = async_channel::bounded(1);
            let mut expected_sequence = 0;
            let mut pending_tools = HashMap::new();
            let envelope = json!({
                "schema": MANAGED_SARAH_PROTOCOL,
                "identity": identity,
                "sequence": 0,
                "_tag": "transcript_final",
                "source": "user",
                "utteranceRef": "utterance-1",
                "text": "",
            });

            let should_end = handle_server_message(
                &envelope.to_string(),
                &events,
                &identity,
                &mut expected_sequence,
                &mut pending_tools,
                &transcript_log,
            )
            .await
            .expect("accept empty final transcript");

            assert_eq!(should_end, GatewayServerAction::Continue);
            assert_eq!(expected_sequence, 1);
            assert!(received_events.is_empty());
            assert!(!transcript_log.path.exists());
        });
    }

    #[test]
    fn interrupt_ack_stops_current_playback() {
        smol::block_on(async {
            let directory = tempfile::tempdir().expect("create transcript directory");
            let transcript_log = LocalVoiceTranscriptLog::new(
                directory.path().join("voice/transcripts.jsonl"),
                "session-1".into(),
            );
            let identity = GatewayVoiceIdentity {
                owner_ref: "owner-1".into(),
                device_ref: "device-1".into(),
                thread_ref: "thread-1".into(),
                session_ref: "session-1".into(),
                generation: 1,
            };
            let (events, _) = async_channel::bounded(1);
            let mut expected_sequence = 0;
            let mut pending_tools = HashMap::new();
            let envelope = json!({
                "schema": MANAGED_SARAH_PROTOCOL,
                "identity": identity,
                "sequence": 0,
                "_tag": "interrupt_ack",
            });

            let action = handle_server_message(
                &envelope.to_string(),
                &events,
                &identity,
                &mut expected_sequence,
                &mut pending_tools,
                &transcript_log,
            )
            .await
            .expect("accept interrupt acknowledgement");

            assert_eq!(action, GatewayServerAction::StopPlayback);
        });
    }

    #[test]
    fn playback_queues_adjacent_pcm_chunks_instead_of_mixing_them() {
        let (player, mut queued_audio) = rodio::Player::new();
        let first = encode_pcm16(&[0.25, 0.5]);
        let second = encode_pcm16(&[-0.25, -0.5]);

        append_pcm16(&player, &first).expect("append first PCM chunk");
        append_pcm16(&player, &second).expect("append second PCM chunk");

        let actual = queued_audio.by_ref().take(4).collect::<Vec<_>>();
        let expected = decode_pcm16(&[first, second].concat());
        assert_eq!(actual, expected);
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
        assert_eq!(
            SarahEditorCommand::StartAgentThread {
                message: "Investigate the test failure".into(),
                presentation: AgentThreadPresentation::Background,
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
    fn agent_thread_command_is_bounded_and_cannot_select_an_agent_or_model() {
        let empty = SarahEditorCommand::StartAgentThread {
            message: "   ".into(),
            presentation: AgentThreadPresentation::Foreground,
        };
        assert!(empty.validate().is_err());

        let oversized = SarahEditorCommand::StartAgentThread {
            message: "x".repeat(MAX_AGENT_MESSAGE_BYTES + 1),
            presentation: AgentThreadPresentation::Background,
        };
        assert!(oversized.validate().is_err());

        let arbitrary_dispatch = serde_json::from_value::<SarahEditorCommand>(json!({
            "name": "start_agent_thread",
            "message": "Do the work",
            "presentation": "background",
            "agent": "some-arbitrary-agent",
            "model": "some-arbitrary-model"
        }));
        assert!(arbitrary_dispatch.is_err());
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

        let command = serde_json::to_value(SarahEditorCommand::StartAgentThread {
            message: "Explain this code".into(),
            presentation: AgentThreadPresentation::Foreground,
        })
        .expect("serialize agent-thread command");
        assert_eq!(command["name"], "start_agent_thread");
        assert_eq!(command["presentation"], "foreground");
        assert_eq!(
            SarahEditorCommand::StartAgentThread {
                message: "Explain this code".into(),
                presentation: AgentThreadPresentation::Foreground,
            }
            .confirmation_detail(),
            Some("Explain this code")
        );
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
    fn managed_audio_envelope_binds_identity_sequence_and_digest() {
        let identity = GatewayVoiceIdentity {
            owner_ref: "owner".into(),
            device_ref: "device".into(),
            thread_ref: "thread".into(),
            session_ref: "session".into(),
            generation: 1,
        };
        let frame = encode_client_audio(&identity, 7, &[0, 0, 1, 0]).expect("encode client audio");
        assert_eq!(frame.get(..4), Some(AUDIO_MEDIA_MAGIC.as_slice()));
        let length = u32::from_be_bytes(
            frame[4..8]
                .try_into()
                .expect("four-byte media header length"),
        ) as usize;
        let header: Value =
            serde_json::from_slice(&frame[8..8 + length]).expect("decode media header");
        assert_eq!(header["schema"], AUDIO_PROTOCOL);
        assert_eq!(header["sequence"], 7);
        assert_eq!(header["identity"]["deviceRef"], "device");
        assert_eq!(
            header["sha256"],
            format!("{:x}", Sha256::digest([0, 0, 1, 0]))
        );
    }

    #[test]
    fn managed_server_envelope_and_agent_command_are_closed_and_typed() {
        let envelope = json!({
            "schema": MANAGED_SARAH_PROTOCOL,
            "identity": {
                "ownerRef": "owner",
                "deviceRef": "device",
                "threadRef": "thread",
                "sessionRef": "session",
                "generation": 1
            },
            "sequence": 0,
            "_tag": "session_ready",
            "model": SARAH_VOICE_MODEL,
            "expiresAtMs": 100,
            "reservedCreditMsat": 10
        });
        assert!(decode_gateway_server_envelope(&envelope.to_string()).is_ok());
        let mut unknown_field = envelope;
        unknown_field["untrusted"] = json!("value");
        assert!(decode_gateway_server_envelope(&unknown_field.to_string()).is_err());

        let (command, expected_path) = decode_gateway_command(json!({
            "_tag": "start_agent_thread",
            "message": "Investigate the failing test",
            "presentation": "background"
        }))
        .expect("decode managed Agent-thread command");
        assert!(matches!(
            command,
            SarahEditorCommand::StartAgentThread {
                presentation: AgentThreadPresentation::Background,
                ..
            }
        ));
        assert_eq!(expected_path, None);
        assert!(
            decode_gateway_command(json!({
                "_tag": "start_agent_thread",
                "message": "Do work",
                "presentation": "background",
                "model": "arbitrary"
            }))
            .is_err()
        );
    }
}
