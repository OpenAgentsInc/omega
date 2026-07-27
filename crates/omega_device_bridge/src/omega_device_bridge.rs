use std::collections::{HashSet, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_io::{Async, Timer};
use async_tungstenite::accept_async_with_config;
use async_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};
use futures::{FutureExt, StreamExt, pin_mut, select};
use nostr::Event;
use qrcode::{QrCode, types::Color};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const PROTOCOL: &str = "openagents.omega.device_bridge.v1";
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
pub const DEVICE_PROOF_KIND: u16 = 27_272;
pub const PAIRING_BOOTSTRAP_SCHEMA: &str = "openagents.omega.device_pairing.v1";
const MAX_PROOF_AGE_SECONDS: u64 = 300;
const MAX_FUTURE_PROOF_SECONDS: u64 = 30;
const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_DELTA_CAPACITY: usize = 1_024;
const MAX_RECENT_PROOFS: usize = 4_096;
const MAX_PROJECTED_THREADS: usize = 64;
const MAX_PROJECTED_RUNS: usize = 64;
const MAX_TRANSCRIPT_MESSAGES: usize = 64;
const MAX_TRANSCRIPT_TEXT_BYTES: usize = 8 * 1024;
const MAX_RECEIPT_REFS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PairingBootstrap {
    pub schema: String,
    pub magic_dns_name: String,
    pub port: u16,
    pub protocol: String,
    pub host_public_key_hex: String,
    pub pairing_secret: String,
    pub generation: u64,
    pub issued_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingQr {
    pub width: usize,
    pub modules: Vec<bool>,
}

impl PairingBootstrap {
    pub fn validate(&self, now_millis: u64) -> Result<(), BridgeError> {
        if self.schema != PAIRING_BOOTSTRAP_SCHEMA
            || !valid_magic_dns_name(&self.magic_dns_name)
            || self.port == 0
            || self.protocol != PROTOCOL
            || self.host_public_key_hex.len() != 64
            || !self
                .host_public_key_hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || self.pairing_secret.len() != 64
            || !self
                .pairing_secret
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || self.generation == 0
            || self.issued_at > now_millis
            || now_millis >= self.expires_at
        {
            return Err(BridgeError::InvalidPairingBootstrap);
        }
        Ok(())
    }

    pub fn qr(&self) -> Result<PairingQr, BridgeError> {
        let payload = serde_json::to_vec(self)?;
        let code = QrCode::new(payload).map_err(|_| BridgeError::PairingQrTooLarge)?;
        Ok(PairingQr {
            width: code.width(),
            modules: code
                .to_colors()
                .into_iter()
                .map(|color| color == Color::Dark)
                .collect(),
        })
    }
}

fn valid_magic_dns_name(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 253
        || value.starts_with('.')
        || value.ends_with('.')
        || value.contains("://")
    {
        return false;
    }
    value.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeBindHost(IpAddr);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BindRefusal {
    #[error("{0:?} is not a literal IP address")]
    NotAnAddress(String),
    #[error("{0} is neither loopback nor a tailnet address")]
    NotLoopbackOrTailnet(IpAddr),
}

impl BridgeBindHost {
    pub const LOOPBACK: Self = Self(IpAddr::V4(Ipv4Addr::LOCALHOST));

    pub fn new(raw: &str) -> Result<Self, BindRefusal> {
        let address = raw
            .parse::<IpAddr>()
            .map_err(|_| BindRefusal::NotAnAddress(raw.to_owned()))?;
        if address.is_loopback() || is_tailnet(address) {
            Ok(Self(address))
        } else {
            Err(BindRefusal::NotLoopbackOrTailnet(address))
        }
    }

    pub const fn address(self) -> IpAddr {
        self.0
    }
}

fn is_tailnet(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let bits = u32::from(address);
            bits & 0xffc0_0000 == u32::from(Ipv4Addr::new(100, 64, 0, 0))
        }
        IpAddr::V6(address) => {
            let segments = address.segments();
            segments[0] == 0xfd7a && segments[1] == 0x115c && segments[2] == 0xa1e0
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: BridgeBindHost,
    pub port: u16,
    pub heartbeat_interval: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: BridgeBindHost::LOOPBACK,
            port: 0,
            heartbeat_interval: DEFAULT_HEARTBEAT_INTERVAL,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cursor {
    pub generation: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutorDisclosure {
    pub executor_id: String,
    pub executor_name: String,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadState {
    Idle,
    Queued,
    Running,
    Waiting,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MirrorMessage {
    pub message_ref: String,
    pub role: MessageRole,
    pub text: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MirrorThread {
    pub thread_ref: String,
    pub title: String,
    pub executor: ExecutorDisclosure,
    pub state: ThreadState,
    pub transcript: Vec<MirrorMessage>,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MirrorRun {
    pub run_ref: String,
    pub title: String,
    pub lane: String,
    pub state: RunState,
    pub receipt_refs: Vec<String>,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MirrorHealth {
    pub engine_up: bool,
    pub engine_generation: u64,
    pub lane_ready: bool,
    pub observed_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MirrorSnapshot {
    pub desktop_name: String,
    pub generation: u64,
    pub sequence: u64,
    pub threads: Vec<MirrorThread>,
    pub runs: Vec<MirrorRun>,
    pub health: MirrorHealth,
    pub projected_at: u64,
}

impl MirrorSnapshot {
    pub fn empty(desktop_name: impl Into<String>, generation: u64, projected_at: u64) -> Self {
        Self {
            desktop_name: desktop_name.into(),
            generation,
            sequence: 0,
            threads: Vec::new(),
            runs: Vec::new(),
            health: MirrorHealth {
                engine_up: false,
                engine_generation: generation,
                lane_ready: false,
                observed_at: projected_at,
            },
            projected_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MirrorChange {
    ThreadUpsert {
        thread: MirrorThread,
    },
    ThreadRemove {
        #[serde(rename = "threadRef")]
        thread_ref: String,
    },
    TranscriptAppend {
        #[serde(rename = "threadRef")]
        thread_ref: String,
        message: MirrorMessage,
        #[serde(rename = "updatedAt")]
        updated_at: u64,
    },
    RunUpsert {
        run: MirrorRun,
    },
    RunRemove {
        #[serde(rename = "runRef")]
        run_ref: String,
    },
    Health {
        health: MirrorHealth,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Grant {
        admitted: bool,
        #[serde(rename = "grantRef", skip_serializing_if = "Option::is_none")]
        grant_ref: Option<String>,
        #[serde(rename = "hostPublicKeyHex", skip_serializing_if = "Option::is_none")]
        host_public_key_hex: Option<String>,
        #[serde(rename = "devicePublicKeyHex", skip_serializing_if = "Option::is_none")]
        device_public_key_hex: Option<String>,
        #[serde(rename = "expiresAt", skip_serializing_if = "Option::is_none")]
        expires_at: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        generation: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<GrantRefusalReason>,
    },
    Snapshot {
        snapshot: MirrorSnapshot,
    },
    Delta {
        generation: u64,
        sequence: u64,
        change: MirrorChange,
    },
    Heartbeat {
        generation: u64,
        sequence: u64,
        #[serde(rename = "sentAt")]
        sent_at: u64,
    },
    Bye {
        reason: ByeReason,
    },
}

impl ServerFrame {
    fn accepted(admission: &GrantAdmission) -> Self {
        Self::Grant {
            admitted: true,
            grant_ref: Some(admission.grant_ref.clone()),
            host_public_key_hex: Some(admission.host_public_key_hex.clone()),
            device_public_key_hex: Some(admission.device_public_key_hex.clone()),
            expires_at: Some(admission.expires_at),
            generation: Some(admission.generation),
            reason: None,
        }
    }

    fn refused(reason: GrantRefusalReason) -> Self {
        Self::Grant {
            admitted: false,
            grant_ref: None,
            host_public_key_hex: None,
            device_public_key_hex: None,
            expires_at: None,
            generation: None,
            reason: Some(reason),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantRefusalReason {
    InvalidProof,
    GrantMissing,
    GrantExpired,
    GrantRevoked,
    ProtocolUnsupported,
    PairingExpired,
    PairingRefused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ByeReason {
    ClientClosed,
    HostShutdown,
    GrantRevoked,
    GrantExpired,
    ProtocolError,
    ResnapshotRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantAdmission {
    pub grant_ref: String,
    pub host_public_key_hex: String,
    pub device_public_key_hex: String,
    pub expires_at: u64,
    pub generation: u64,
}

pub trait GrantAuthority: Send + Sync + 'static {
    fn authorize(
        &self,
        device_public_key_hex: &str,
        host_public_key_hex: &str,
        grant_ref: Option<&str>,
        pairing_secret: Option<&str>,
        now_millis: u64,
    ) -> Result<GrantAdmission, GrantRefusalReason>;
}

#[derive(Debug, Clone)]
struct DeltaRecord {
    generation: u64,
    sequence: u64,
    change: MirrorChange,
}

#[derive(Debug)]
struct JournalState {
    snapshot: MirrorSnapshot,
    deltas: VecDeque<DeltaRecord>,
    delta_capacity: usize,
}

#[derive(Debug, Clone)]
pub struct ProjectionJournal {
    state: Arc<RwLock<JournalState>>,
}

impl ProjectionJournal {
    pub fn new(snapshot: MirrorSnapshot) -> Self {
        Self::with_capacity(snapshot, DEFAULT_DELTA_CAPACITY)
    }

    pub fn with_capacity(snapshot: MirrorSnapshot, delta_capacity: usize) -> Self {
        Self {
            state: Arc::new(RwLock::new(JournalState {
                snapshot,
                deltas: VecDeque::new(),
                delta_capacity: delta_capacity.max(1),
            })),
        }
    }

    pub fn replace_snapshot(&self, mut snapshot: MirrorSnapshot) -> Result<(), BridgeError> {
        validate_snapshot(&snapshot)?;
        let mut state = self.state.write().map_err(|_| BridgeError::StatePoisoned)?;
        snapshot.sequence = 0;
        state.snapshot = snapshot;
        state.deltas.clear();
        Ok(())
    }

    pub fn publish(&self, change: MirrorChange) -> Result<Cursor, BridgeError> {
        let mut state = self.state.write().map_err(|_| BridgeError::StatePoisoned)?;
        let generation = state.snapshot.generation;
        let sequence = state
            .snapshot
            .sequence
            .checked_add(1)
            .ok_or(BridgeError::SequenceExhausted)?;
        let mut next_snapshot = state.snapshot.clone();
        next_snapshot.sequence = sequence;
        apply_change(&mut next_snapshot, &change);
        validate_snapshot(&next_snapshot)?;
        validate_frame_size(&ServerFrame::Delta {
            generation,
            sequence,
            change: change.clone(),
        })?;
        state.snapshot = next_snapshot;
        state.deltas.push_back(DeltaRecord {
            generation,
            sequence,
            change,
        });
        while state.deltas.len() > state.delta_capacity {
            state.deltas.pop_front();
        }
        Ok(Cursor {
            generation,
            sequence,
        })
    }

    fn frames_after(&self, cursor: Option<Cursor>) -> Result<Vec<ServerFrame>, BridgeError> {
        let state = self.state.read().map_err(|_| BridgeError::StatePoisoned)?;
        let Some(cursor) = cursor else {
            return Ok(vec![ServerFrame::Snapshot {
                snapshot: state.snapshot.clone(),
            }]);
        };
        if cursor.generation != state.snapshot.generation
            || cursor.sequence > state.snapshot.sequence
        {
            return Ok(vec![ServerFrame::Snapshot {
                snapshot: state.snapshot.clone(),
            }]);
        }
        if cursor.sequence == state.snapshot.sequence {
            return Ok(Vec::new());
        }
        let expected = cursor
            .sequence
            .checked_add(1)
            .ok_or(BridgeError::SequenceExhausted)?;
        if state
            .deltas
            .front()
            .is_none_or(|record| record.sequence > expected)
        {
            return Ok(vec![ServerFrame::Snapshot {
                snapshot: state.snapshot.clone(),
            }]);
        }
        Ok(state
            .deltas
            .iter()
            .filter(|record| record.generation == cursor.generation && record.sequence >= expected)
            .map(|record| ServerFrame::Delta {
                generation: record.generation,
                sequence: record.sequence,
                change: record.change.clone(),
            })
            .collect())
    }

    pub fn cursor(&self) -> Result<Cursor, BridgeError> {
        let state = self.state.read().map_err(|_| BridgeError::StatePoisoned)?;
        Ok(Cursor {
            generation: state.snapshot.generation,
            sequence: state.snapshot.sequence,
        })
    }

    pub fn snapshot(&self) -> Result<MirrorSnapshot, BridgeError> {
        self.state
            .read()
            .map(|state| state.snapshot.clone())
            .map_err(|_| BridgeError::StatePoisoned)
    }
}

fn validate_snapshot(snapshot: &MirrorSnapshot) -> Result<(), BridgeError> {
    if snapshot.threads.len() > MAX_PROJECTED_THREADS {
        return Err(BridgeError::ProjectionOutOfBounds("thread count"));
    }
    if snapshot.runs.len() > MAX_PROJECTED_RUNS {
        return Err(BridgeError::ProjectionOutOfBounds("run count"));
    }
    if snapshot.threads.iter().any(|thread| {
        thread.transcript.len() > MAX_TRANSCRIPT_MESSAGES
            || thread
                .transcript
                .iter()
                .any(|message| message.text.len() > MAX_TRANSCRIPT_TEXT_BYTES)
    }) {
        return Err(BridgeError::ProjectionOutOfBounds("transcript preview"));
    }
    if snapshot
        .runs
        .iter()
        .any(|run| run.receipt_refs.len() > MAX_RECEIPT_REFS)
    {
        return Err(BridgeError::ProjectionOutOfBounds(
            "receipt reference count",
        ));
    }
    validate_frame_size(&ServerFrame::Snapshot {
        snapshot: snapshot.clone(),
    })
}

fn validate_frame_size(frame: &ServerFrame) -> Result<(), BridgeError> {
    let encoded = serde_json::to_vec(frame)?;
    if encoded.len() > MAX_FRAME_BYTES {
        return Err(BridgeError::ProjectionFrameTooLarge {
            encoded_bytes: encoded.len(),
        });
    }
    Ok(())
}

fn apply_change(snapshot: &mut MirrorSnapshot, change: &MirrorChange) {
    match change {
        MirrorChange::ThreadUpsert { thread } => {
            if let Some(existing) = snapshot
                .threads
                .iter_mut()
                .find(|existing| existing.thread_ref == thread.thread_ref)
            {
                *existing = thread.clone();
            } else {
                snapshot.threads.push(thread.clone());
            }
        }
        MirrorChange::ThreadRemove { thread_ref } => {
            snapshot
                .threads
                .retain(|thread| &thread.thread_ref != thread_ref);
        }
        MirrorChange::TranscriptAppend {
            thread_ref,
            message,
            updated_at,
        } => {
            if let Some(thread) = snapshot
                .threads
                .iter_mut()
                .find(|thread| &thread.thread_ref == thread_ref)
            {
                thread.transcript.push(message.clone());
                thread.updated_at = *updated_at;
            }
        }
        MirrorChange::RunUpsert { run } => {
            if let Some(existing) = snapshot
                .runs
                .iter_mut()
                .find(|existing| existing.run_ref == run.run_ref)
            {
                *existing = run.clone();
            } else {
                snapshot.runs.push(run.clone());
            }
        }
        MirrorChange::RunRemove { run_ref } => {
            snapshot.runs.retain(|run| &run.run_ref != run_ref);
        }
        MirrorChange::Health { health } => snapshot.health = health.clone(),
    }
}

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("device bridge I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("device bridge WebSocket failed: {0}")]
    WebSocket(#[source] Box<async_tungstenite::tungstenite::Error>),
    #[error("device bridge frame could not be encoded: {0}")]
    Json(#[from] serde_json::Error),
    #[error("device bridge shared state is poisoned")]
    StatePoisoned,
    #[error("device bridge sequence is exhausted")]
    SequenceExhausted,
    #[error("device bridge projection exceeds the {0} limit")]
    ProjectionOutOfBounds(&'static str),
    #[error(
        "device bridge projection frame is {encoded_bytes} bytes, above the {MAX_FRAME_BYTES}-byte limit"
    )]
    ProjectionFrameTooLarge { encoded_bytes: usize },
    #[error("device bridge background thread panicked")]
    ThreadPanicked,
    #[error("device pairing bootstrap is invalid or expired")]
    InvalidPairingBootstrap,
    #[error("device pairing bootstrap is too large for a QR code")]
    PairingQrTooLarge,
}

impl From<async_tungstenite::tungstenite::Error> for BridgeError {
    fn from(error: async_tungstenite::tungstenite::Error) -> Self {
        Self::WebSocket(Box::new(error))
    }
}

pub struct ServerHandle {
    local_address: SocketAddr,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), BridgeError>>>,
}

impl ServerHandle {
    pub fn spawn(
        config: ServerConfig,
        authority: Arc<dyn GrantAuthority>,
        journal: ProjectionJournal,
    ) -> Result<Self, BridgeError> {
        let listener = TcpListener::bind(SocketAddr::new(config.host.address(), config.port))?;
        listener.set_nonblocking(true)?;
        let local_address = listener.local_addr()?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = shutdown.clone();
        let thread = std::thread::Builder::new()
            .name("omega-device-bridge".into())
            .spawn(move || {
                smol_run_server(listener, authority, journal, config, thread_shutdown)
            })?;
        Ok(Self {
            local_address,
            shutdown,
            thread: Some(thread),
        })
    }

    pub const fn local_address(&self) -> SocketAddr {
        self.local_address
    }

    pub fn shutdown(mut self) -> Result<(), BridgeError> {
        self.stop_and_join()
    }

    fn stop_and_join(&mut self) -> Result<(), BridgeError> {
        self.shutdown.store(true, Ordering::Release);
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        thread.join().map_err(|_| BridgeError::ThreadPanicked)?
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        if let Err(error) = self.stop_and_join() {
            log::error!("device bridge shutdown failed: {error}");
        }
    }
}

fn smol_run_server(
    listener: TcpListener,
    authority: Arc<dyn GrantAuthority>,
    journal: ProjectionJournal,
    config: ServerConfig,
    shutdown: Arc<AtomicBool>,
) -> Result<(), BridgeError> {
    smol::block_on(async move {
        let listener = Async::new(listener)?;
        let recent_proofs = Arc::new(Mutex::new(RecentProofs::default()));
        while !shutdown.load(Ordering::Acquire) {
            let accept = listener.accept().fuse();
            let tick = FutureExt::fuse(network_timer(Duration::from_millis(50)));
            pin_mut!(accept, tick);
            select! {
                accepted = accept => {
                    let (stream, _) = accepted?;
                    let authority = authority.clone();
                    let journal = journal.clone();
                    let shutdown = shutdown.clone();
                    let recent_proofs = recent_proofs.clone();
                    smol::spawn(async move {
                        if let Err(error) = serve_connection(
                            stream,
                            authority,
                            journal,
                            config.heartbeat_interval,
                            shutdown,
                            recent_proofs,
                        ).await {
                            log::warn!("device bridge session failed: {error}");
                        }
                    }).detach();
                }
                _ = tick => {}
            }
        }
        Ok(())
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HelloFrame {
    #[serde(rename = "type")]
    frame_type: String,
    protocol: String,
    device_public_key_hex: String,
    host_public_key_hex: String,
    grant_ref: Option<String>,
    pairing_secret: Option<String>,
    resume_cursor: Option<Cursor>,
    proof: Event,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProofContent {
    protocol: String,
    host_public_key_hex: String,
    grant_ref: Option<String>,
    pairing_secret_digest: Option<String>,
    resume_cursor: Option<Cursor>,
    nonce: String,
}

#[derive(Default)]
struct RecentProofs {
    ids: HashSet<String>,
    order: VecDeque<String>,
}

impl RecentProofs {
    fn admit(&mut self, event_id: String) -> bool {
        if !self.ids.insert(event_id.clone()) {
            return false;
        }
        self.order.push_back(event_id);
        while self.order.len() > MAX_RECENT_PROOFS {
            if let Some(expired) = self.order.pop_front() {
                self.ids.remove(&expired);
            }
        }
        true
    }
}

async fn serve_connection(
    stream: Async<std::net::TcpStream>,
    authority: Arc<dyn GrantAuthority>,
    journal: ProjectionJournal,
    heartbeat_interval: Duration,
    shutdown: Arc<AtomicBool>,
    recent_proofs: Arc<Mutex<RecentProofs>>,
) -> Result<(), BridgeError> {
    let websocket_config = WebSocketConfig::default()
        .max_frame_size(Some(MAX_FRAME_BYTES))
        .max_message_size(Some(MAX_FRAME_BYTES));
    let mut socket = accept_async_with_config(stream, Some(websocket_config)).await?;
    let hello_message = socket.next().await;
    let Some(hello_message) = hello_message else {
        return Ok(());
    };
    let hello_message = hello_message?;
    let Message::Text(text) = hello_message else {
        send_frame(
            &mut socket,
            &ServerFrame::refused(GrantRefusalReason::ProtocolUnsupported),
        )
        .await?;
        return Ok(());
    };
    if text.len() > MAX_FRAME_BYTES {
        send_frame(
            &mut socket,
            &ServerFrame::refused(GrantRefusalReason::ProtocolUnsupported),
        )
        .await?;
        return Ok(());
    }
    let hello = match serde_json::from_str::<HelloFrame>(&text) {
        Ok(hello) => hello,
        Err(_) => {
            send_frame(
                &mut socket,
                &ServerFrame::refused(GrantRefusalReason::ProtocolUnsupported),
            )
            .await?;
            return Ok(());
        }
    };
    if hello.frame_type != "hello" || hello.protocol != PROTOCOL {
        send_frame(
            &mut socket,
            &ServerFrame::refused(GrantRefusalReason::ProtocolUnsupported),
        )
        .await?;
        return Ok(());
    }
    let now_millis = unix_millis();
    if !verify_hello_proof(&hello, now_millis)
        || !recent_proofs
            .lock()
            .map_err(|_| BridgeError::StatePoisoned)?
            .admit(hello.proof.id.to_hex())
    {
        send_frame(
            &mut socket,
            &ServerFrame::refused(GrantRefusalReason::InvalidProof),
        )
        .await?;
        return Ok(());
    }
    let admission = match authority.authorize(
        &hello.device_public_key_hex,
        &hello.host_public_key_hex,
        hello.grant_ref.as_deref(),
        hello.pairing_secret.as_deref(),
        now_millis,
    ) {
        Ok(admission) => admission,
        Err(reason) => {
            send_frame(&mut socket, &ServerFrame::refused(reason)).await?;
            return Ok(());
        }
    };
    send_frame(&mut socket, &ServerFrame::accepted(&admission)).await?;
    let mut cursor = hello.resume_cursor;
    send_projection_frames(&mut socket, &journal, &mut cursor).await?;

    loop {
        if shutdown.load(Ordering::Acquire) {
            send_frame(
                &mut socket,
                &ServerFrame::Bye {
                    reason: ByeReason::HostShutdown,
                },
            )
            .await?;
            return Ok(());
        }
        let incoming = socket.next().fuse();
        let tick = FutureExt::fuse(network_timer(heartbeat_interval));
        pin_mut!(incoming, tick);
        select! {
            message = incoming => {
                let Some(message) = message else {
                    return Ok(());
                };
                match message? {
                    Message::Text(text) if text.len() <= MAX_FRAME_BYTES => {
                        let value = serde_json::from_str::<Value>(&text);
                        let frame_type = value
                            .as_ref()
                            .ok()
                            .and_then(|value| value.get("type"))
                            .and_then(Value::as_str);
                        match frame_type {
                            Some("heartbeat") => {}
                            Some("bye") => return Ok(()),
                            _ => {
                                send_frame(
                                    &mut socket,
                                    &ServerFrame::Bye {
                                        reason: ByeReason::ProtocolError,
                                    },
                                ).await?;
                                return Ok(());
                            }
                        }
                    }
                    Message::Close(_) => return Ok(()),
                    Message::Ping(payload) => socket.send(Message::Pong(payload)).await?,
                    Message::Pong(_) => {}
                    _ => {
                        send_frame(
                            &mut socket,
                            &ServerFrame::Bye {
                                reason: ByeReason::ProtocolError,
                            },
                        ).await?;
                        return Ok(());
                    }
                }
            }
            _ = tick => {
                let now_millis = unix_millis();
                match authority.authorize(
                    &admission.device_public_key_hex,
                    &admission.host_public_key_hex,
                    Some(&admission.grant_ref),
                    None,
                    now_millis,
                ) {
                    Ok(_) => {}
                    Err(GrantRefusalReason::GrantExpired) => {
                        send_frame(
                            &mut socket,
                            &ServerFrame::Bye {
                                reason: ByeReason::GrantExpired,
                            },
                        ).await?;
                        return Ok(());
                    }
                    Err(_) => {
                        send_frame(
                            &mut socket,
                            &ServerFrame::Bye {
                                reason: ByeReason::GrantRevoked,
                            },
                        ).await?;
                        return Ok(());
                    }
                }
                send_projection_frames(&mut socket, &journal, &mut cursor).await?;
                let current = journal.cursor()?;
                send_frame(
                    &mut socket,
                    &ServerFrame::Heartbeat {
                        generation: current.generation,
                        sequence: current.sequence,
                        sent_at: now_millis,
                    },
                ).await?;
            }
        }
    }
}

async fn send_projection_frames<S>(
    socket: &mut async_tungstenite::WebSocketStream<S>,
    journal: &ProjectionJournal,
    cursor: &mut Option<Cursor>,
) -> Result<(), BridgeError>
where
    S: futures::AsyncRead + futures::AsyncWrite + Unpin,
{
    for frame in journal.frames_after(*cursor)? {
        match &frame {
            ServerFrame::Snapshot { snapshot } => {
                *cursor = Some(Cursor {
                    generation: snapshot.generation,
                    sequence: snapshot.sequence,
                });
            }
            ServerFrame::Delta {
                generation,
                sequence,
                ..
            } => {
                *cursor = Some(Cursor {
                    generation: *generation,
                    sequence: *sequence,
                });
            }
            _ => {}
        }
        send_frame(socket, &frame).await?;
    }
    Ok(())
}

async fn send_frame<S>(
    socket: &mut async_tungstenite::WebSocketStream<S>,
    frame: &ServerFrame,
) -> Result<(), BridgeError>
where
    S: futures::AsyncRead + futures::AsyncWrite + Unpin,
{
    let encoded = serde_json::to_string(frame)?;
    if encoded.len() > MAX_FRAME_BYTES {
        return Err(BridgeError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "device bridge frame exceeds 64 KiB",
        )));
    }
    socket.send(Message::Text(encoded.into())).await?;
    Ok(())
}

fn verify_hello_proof(hello: &HelloFrame, now_millis: u64) -> bool {
    if hello.proof.verify().is_err()
        || hello.proof.pubkey.to_hex() != hello.device_public_key_hex
        || hello.proof.kind.as_u16() != DEVICE_PROOF_KIND
    {
        return false;
    }
    let created_at = hello.proof.created_at.as_secs();
    let now_seconds = now_millis / 1_000;
    if created_at.saturating_add(MAX_PROOF_AGE_SECONDS) < now_seconds
        || created_at > now_seconds.saturating_add(MAX_FUTURE_PROOF_SECONDS)
    {
        return false;
    }
    let Ok(content) = serde_json::from_str::<ProofContent>(&hello.proof.content) else {
        return false;
    };
    if content.protocol != PROTOCOL
        || content.host_public_key_hex != hello.host_public_key_hex
        || content.grant_ref != hello.grant_ref
        || content.pairing_secret_digest
            != hello
                .pairing_secret
                .as_ref()
                .map(|secret| format!("{:x}", Sha256::digest(secret.as_bytes())))
        || content.resume_cursor != hello.resume_cursor
        || content.nonce.is_empty()
        || content.nonce.len() > 256
    {
        return false;
    }
    let Ok(proof_value) = serde_json::to_value(&hello.proof) else {
        return false;
    };
    proof_has_tag(&proof_value, "p", &hello.host_public_key_hex)
        && proof_has_tag(&proof_value, "protocol", PROTOCOL)
}

fn proof_has_tag(proof: &Value, name: &str, value: &str) -> bool {
    proof
        .get("tags")
        .and_then(Value::as_array)
        .is_some_and(|tags| {
            tags.iter().any(|tag| {
                tag.as_array().is_some_and(|parts| {
                    parts.first().and_then(Value::as_str) == Some(name)
                        && parts.get(1).and_then(Value::as_str) == Some(value)
                })
            })
        })
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[allow(clippy::disallowed_methods)]
fn network_timer(duration: Duration) -> Timer {
    // The bridge transport is deliberately independent of GPUI.
    Timer::after(duration)
}

#[cfg(test)]
mod tests {
    use async_tungstenite::async_std::connect_async;
    use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
    use serde_json::json;

    use super::*;

    struct TestAuthority {
        admission: GrantAdmission,
        revoked: AtomicBool,
    }

    impl GrantAuthority for TestAuthority {
        fn authorize(
            &self,
            device_public_key_hex: &str,
            host_public_key_hex: &str,
            grant_ref: Option<&str>,
            _pairing_secret: Option<&str>,
            now_millis: u64,
        ) -> Result<GrantAdmission, GrantRefusalReason> {
            if self.revoked.load(Ordering::Acquire) {
                return Err(GrantRefusalReason::GrantRevoked);
            }
            if now_millis >= self.admission.expires_at {
                return Err(GrantRefusalReason::GrantExpired);
            }
            if device_public_key_hex != self.admission.device_public_key_hex
                || host_public_key_hex != self.admission.host_public_key_hex
                || grant_ref != Some(self.admission.grant_ref.as_str())
            {
                return Err(GrantRefusalReason::GrantMissing);
            }
            Ok(self.admission.clone())
        }
    }

    fn signed_hello(
        keys: &Keys,
        host_public_key_hex: &str,
        grant_ref: &str,
        resume_cursor: Option<Cursor>,
    ) -> Value {
        let content = json!({
            "protocol": PROTOCOL,
            "hostPublicKeyHex": host_public_key_hex,
            "grantRef": grant_ref,
            "pairingSecretDigest": null,
            "resumeCursor": resume_cursor,
            "nonce": "nonce-1",
        })
        .to_string();
        let proof = EventBuilder::new(Kind::Custom(DEVICE_PROOF_KIND), content)
            .tags([
                Tag::public_key(nostr::PublicKey::from_hex(host_public_key_hex).expect("host key")),
                Tag::custom(nostr::TagKind::custom("protocol"), [PROTOCOL.to_string()]),
            ])
            .custom_created_at(Timestamp::now())
            .sign_with_keys(keys)
            .expect("signed proof");
        json!({
            "type": "hello",
            "protocol": PROTOCOL,
            "devicePublicKeyHex": keys.public_key().to_hex(),
            "hostPublicKeyHex": host_public_key_hex,
            "grantRef": grant_ref,
            "pairingSecret": null,
            "resumeCursor": resume_cursor,
            "proof": serde_json::from_str::<Value>(&proof.as_json()).expect("proof json"),
        })
    }

    async fn next_frame<S>(socket: &mut async_tungstenite::WebSocketStream<S>) -> ServerFrame
    where
        S: futures::AsyncRead + futures::AsyncWrite + Unpin,
    {
        let message = socket
            .next()
            .await
            .expect("server frame")
            .expect("valid websocket frame");
        let Message::Text(text) = message else {
            panic!("expected text frame");
        };
        serde_json::from_str(&text).expect("typed server frame")
    }

    #[test]
    fn bind_policy_accepts_only_loopback_and_tailnet() {
        assert!(BridgeBindHost::new("127.0.0.1").is_ok());
        assert!(BridgeBindHost::new("::1").is_ok());
        assert!(BridgeBindHost::new("100.64.0.1").is_ok());
        assert!(BridgeBindHost::new("100.127.255.254").is_ok());
        assert!(BridgeBindHost::new("fd7a:115c:a1e0::1").is_ok());
        assert!(BridgeBindHost::new("0.0.0.0").is_err());
        assert!(BridgeBindHost::new("192.168.1.10").is_err());
        assert!(BridgeBindHost::new("100.128.0.1").is_err());
        assert!(BridgeBindHost::new("localhost").is_err());
    }

    #[test]
    fn pairing_bootstrap_is_bounded_and_renders_a_qr() {
        let bootstrap = PairingBootstrap {
            schema: PAIRING_BOOTSTRAP_SCHEMA.into(),
            magic_dns_name: "omega-primary.tail1234.ts.net".into(),
            port: 4317,
            protocol: PROTOCOL.into(),
            host_public_key_hex: "1".repeat(64),
            pairing_secret: "2".repeat(64),
            generation: 7,
            issued_at: 1_000,
            expires_at: 301_000,
        };
        bootstrap.validate(1_000).expect("valid bootstrap");
        let qr = bootstrap.qr().expect("QR");
        assert!(qr.width >= 21);
        assert_eq!(qr.modules.len(), qr.width * qr.width);

        let mut expired = bootstrap;
        expired.expires_at = 1_000;
        assert!(expired.validate(1_000).is_err());
    }

    #[test]
    fn a_delta_gap_forces_a_snapshot_and_a_contiguous_cursor_resumes() {
        let journal = ProjectionJournal::with_capacity(MirrorSnapshot::empty("Omega", 7, 1), 2);
        for observed_at in 1..=3 {
            journal
                .publish(MirrorChange::Health {
                    health: MirrorHealth {
                        engine_up: true,
                        engine_generation: 7,
                        lane_ready: true,
                        observed_at,
                    },
                })
                .expect("publish");
        }
        assert!(matches!(
            journal
                .frames_after(Some(Cursor {
                    generation: 7,
                    sequence: 0
                }))
                .expect("gap frames")
                .as_slice(),
            [ServerFrame::Snapshot { .. }]
        ));
        let frames = journal
            .frames_after(Some(Cursor {
                generation: 7,
                sequence: 1,
            }))
            .expect("resume frames");
        assert_eq!(frames.len(), 2);
        assert!(matches!(
            frames.as_slice(),
            [
                ServerFrame::Delta { sequence: 2, .. },
                ServerFrame::Delta { sequence: 3, .. }
            ]
        ));
    }

    #[test]
    fn projection_journey_covers_thread_run_receipt_and_engine_restart() {
        let journal = ProjectionJournal::with_capacity(MirrorSnapshot::empty("Omega", 7, 1), 16);
        journal
            .publish(MirrorChange::ThreadUpsert {
                thread: MirrorThread {
                    thread_ref: "thread.1".into(),
                    title: "Mobile projection".into(),
                    executor: ExecutorDisclosure {
                        executor_id: "claude-acp".into(),
                        executor_name: "Claude Code".into(),
                        model_id: Some("claude-opus".into()),
                        model_name: Some("Claude Opus".into()),
                    },
                    state: ThreadState::Running,
                    transcript: vec![MirrorMessage {
                        message_ref: "message.1".into(),
                        role: MessageRole::User,
                        text: "Inspect the workspace".into(),
                        created_at: 2,
                    }],
                    updated_at: 2,
                },
            })
            .expect("thread start");
        journal
            .publish(MirrorChange::TranscriptAppend {
                thread_ref: "thread.1".into(),
                message: MirrorMessage {
                    message_ref: "message.2".into(),
                    role: MessageRole::Assistant,
                    text: "The workspace contains".into(),
                    created_at: 3,
                },
                updated_at: 3,
            })
            .expect("stream transcript");
        journal
            .publish(MirrorChange::RunUpsert {
                run: MirrorRun {
                    run_ref: "run.1".into(),
                    title: "Mobile projection".into(),
                    lane: "claude-local".into(),
                    state: RunState::Completed,
                    receipt_refs: vec!["receipt.1".into()],
                    updated_at: 4,
                },
            })
            .expect("run completion");

        let snapshot = journal.snapshot().expect("snapshot");
        assert_eq!(
            snapshot.threads[0].executor.model_id.as_deref(),
            Some("claude-opus")
        );
        assert_eq!(snapshot.threads[0].transcript.len(), 2);
        assert_eq!(snapshot.runs[0].receipt_refs, ["receipt.1"]);

        let before_restart = journal.cursor().expect("cursor");
        let mut restarted = snapshot;
        restarted.generation = 8;
        restarted.health = MirrorHealth {
            engine_up: true,
            engine_generation: 8,
            lane_ready: true,
            observed_at: 5,
        };
        journal.replace_snapshot(restarted).expect("engine restart");
        assert!(matches!(
            journal
                .frames_after(Some(before_restart))
                .expect("old generation resnapshot")
                .as_slice(),
            [ServerFrame::Snapshot { snapshot }]
                if snapshot.generation == 8 && snapshot.sequence == 0
        ));
    }

    #[test]
    fn oversized_transcript_preview_is_rejected_without_advancing_the_cursor() {
        let journal = ProjectionJournal::new(MirrorSnapshot::empty("Omega", 1, 1));
        let cursor = journal.cursor().expect("initial cursor");
        let error = journal
            .publish(MirrorChange::ThreadUpsert {
                thread: MirrorThread {
                    thread_ref: "thread.oversized".into(),
                    title: "Oversized".into(),
                    executor: ExecutorDisclosure {
                        executor_id: "omega-agent".into(),
                        executor_name: "Omega".into(),
                        model_id: None,
                        model_name: None,
                    },
                    state: ThreadState::Running,
                    transcript: vec![MirrorMessage {
                        message_ref: "message.oversized".into(),
                        role: MessageRole::Assistant,
                        text: "x".repeat(MAX_TRANSCRIPT_TEXT_BYTES + 1),
                        created_at: 2,
                    }],
                    updated_at: 2,
                },
            })
            .expect_err("oversized transcript must fail");
        assert!(matches!(
            error,
            BridgeError::ProjectionOutOfBounds("transcript preview")
        ));
        assert_eq!(journal.cursor().expect("unchanged cursor"), cursor);
        assert!(
            journal
                .snapshot()
                .expect("unchanged snapshot")
                .threads
                .is_empty()
        );
    }

    #[test]
    fn websocket_auth_resume_no_command_and_revocation_are_enforced() {
        smol::block_on(async {
            let device_keys = Keys::generate();
            let host_keys = Keys::generate();
            let now = unix_millis();
            let admission = GrantAdmission {
                grant_ref: "grant.test.1".into(),
                host_public_key_hex: host_keys.public_key().to_hex(),
                device_public_key_hex: device_keys.public_key().to_hex(),
                expires_at: now + 60_000,
                generation: 4,
            };
            let authority = Arc::new(TestAuthority {
                admission: admission.clone(),
                revoked: AtomicBool::new(false),
            });
            let journal =
                ProjectionJournal::with_capacity(MirrorSnapshot::empty("Omega", 4, now), 8);
            journal
                .publish(MirrorChange::Health {
                    health: MirrorHealth {
                        engine_up: true,
                        engine_generation: 4,
                        lane_ready: true,
                        observed_at: now,
                    },
                })
                .expect("delta one");
            let handle = ServerHandle::spawn(
                ServerConfig {
                    heartbeat_interval: Duration::from_millis(20),
                    ..ServerConfig::default()
                },
                authority.clone(),
                journal.clone(),
            )
            .expect("server");
            let url = format!("ws://{}", handle.local_address());

            let (mut socket, _) = connect_async(&url).await.expect("connect");
            let initial_hello = signed_hello(
                &device_keys,
                &admission.host_public_key_hex,
                &admission.grant_ref,
                None,
            );
            socket
                .send(Message::Text(initial_hello.to_string().into()))
                .await
                .expect("hello");
            assert!(matches!(
                next_frame(&mut socket).await,
                ServerFrame::Grant { admitted: true, .. }
            ));
            assert!(matches!(
                next_frame(&mut socket).await,
                ServerFrame::Snapshot { .. }
            ));
            journal
                .publish(MirrorChange::Health {
                    health: MirrorHealth {
                        engine_up: true,
                        engine_generation: 4,
                        lane_ready: false,
                        observed_at: now + 1,
                    },
                })
                .expect("delta two");
            loop {
                if matches!(
                    next_frame(&mut socket).await,
                    ServerFrame::Delta { sequence: 2, .. }
                ) {
                    break;
                }
            }
            socket
                .send(Message::Text(
                    json!({"type": "command", "method": "run"})
                        .to_string()
                        .into(),
                ))
                .await
                .expect("command");
            loop {
                if matches!(
                    next_frame(&mut socket).await,
                    ServerFrame::Bye {
                        reason: ByeReason::ProtocolError
                    }
                ) {
                    break;
                }
            }

            let (mut replayed, _) = connect_async(&url).await.expect("replay connect");
            replayed
                .send(Message::Text(initial_hello.to_string().into()))
                .await
                .expect("replayed hello");
            assert_eq!(
                next_frame(&mut replayed).await,
                ServerFrame::refused(GrantRefusalReason::InvalidProof)
            );

            let (mut resumed, _) = connect_async(&url).await.expect("resume connect");
            resumed
                .send(Message::Text(
                    signed_hello(
                        &device_keys,
                        &admission.host_public_key_hex,
                        &admission.grant_ref,
                        Some(Cursor {
                            generation: 4,
                            sequence: 1,
                        }),
                    )
                    .to_string()
                    .into(),
                ))
                .await
                .expect("resume hello");
            assert!(matches!(
                next_frame(&mut resumed).await,
                ServerFrame::Grant { admitted: true, .. }
            ));
            assert!(matches!(
                next_frame(&mut resumed).await,
                ServerFrame::Delta { sequence: 2, .. }
            ));
            authority.revoked.store(true, Ordering::Release);
            loop {
                if matches!(
                    next_frame(&mut resumed).await,
                    ServerFrame::Bye {
                        reason: ByeReason::GrantRevoked
                    }
                ) {
                    break;
                }
            }
            handle.shutdown().expect("shutdown");
        });
    }

    #[test]
    fn revoked_grant_is_refused_at_admission() {
        smol::block_on(async {
            let device_keys = Keys::generate();
            let host_keys = Keys::generate();
            let now = unix_millis();
            let admission = GrantAdmission {
                grant_ref: "grant.test.revoked".into(),
                host_public_key_hex: host_keys.public_key().to_hex(),
                device_public_key_hex: device_keys.public_key().to_hex(),
                expires_at: now + 60_000,
                generation: 1,
            };
            let authority = Arc::new(TestAuthority {
                admission: admission.clone(),
                revoked: AtomicBool::new(true),
            });
            let handle = ServerHandle::spawn(
                ServerConfig::default(),
                authority,
                ProjectionJournal::new(MirrorSnapshot::empty("Omega", 1, now)),
            )
            .expect("server");
            let (mut socket, _) = connect_async(format!("ws://{}", handle.local_address()))
                .await
                .expect("connect");
            socket
                .send(Message::Text(
                    signed_hello(
                        &device_keys,
                        &admission.host_public_key_hex,
                        &admission.grant_ref,
                        None,
                    )
                    .to_string()
                    .into(),
                ))
                .await
                .expect("hello");
            assert_eq!(
                next_frame(&mut socket).await,
                ServerFrame::refused(GrantRefusalReason::GrantRevoked)
            );
            handle.shutdown().expect("shutdown");
        });
    }
}
