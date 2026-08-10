use std::collections::{HashMap, VecDeque};
use std::io::{BufRead as _, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{App, AppContext as _, Context, Entity, Global, Subscription, TaskExt as _};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const CREDENTIAL_KEY: &str = "omega://nautilus/hyperliquid-testnet-private-key";
pub const ENABLE_ENVIRONMENT_VARIABLE: &str = "OMEGA_NAUTILUS_SIDECAR";
pub const NETWORK_ENVIRONMENT_VARIABLE: &str = "OMEGA_NAUTILUS_NETWORK";
const EVENT_PREFIX: &str = "OMEGA_NAUTILUS_EVENT ";
const EVENT_SCHEMA: &str = "omega.nautilus.lifecycle.v1";
const STREAM_PREFIX: &str = "OMEGA_NAUTILUS_STREAM ";
const STREAM_SCHEMA: &str = "omega.nautilus.stream.v1";
const DEFAULT_RECONCILIATION_LOOKBACK_MINUTES: u16 = 60;
const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(15);
const MONITOR_INTERVAL: Duration = Duration::from_secs(2);
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const TRADE_BUFFER_CAPACITY: usize = 2_048;
const TRADES_PER_FRAME: usize = 256;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    Testnet,
}

impl Network {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "testnet" => Ok(Self::Testnet),
            "mainnet" => bail!("Nautilus mainnet is disabled; only testnet is permitted"),
            _ => bail!("unsupported Nautilus network {value:?}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct NautilusConfig {
    pub network: Network,
    pub python: PathBuf,
    pub engine: PathBuf,
    pub reconciliation_lookback_minutes: u16,
    pub health_timeout: Duration,
}

impl NautilusConfig {
    pub fn from_process_environment() -> Result<Option<Self>> {
        if std::env::var(ENABLE_ENVIRONMENT_VARIABLE).as_deref() != Ok("1") {
            return Ok(None);
        }
        let network = Network::parse(
            &std::env::var(NETWORK_ENVIRONMENT_VARIABLE).unwrap_or_else(|_| "testnet".into()),
        )?;
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .ok_or_else(|| anyhow!("Nautilus crate has no repository root"))?
            .to_path_buf();
        Ok(Some(Self {
            network,
            python: repository_root.join("sidecar/nautilus/.venv/bin/python"),
            engine: repository_root.join("sidecar/nautilus/engine.py"),
            reconciliation_lookback_minutes: DEFAULT_RECONCILIATION_LOOKBACK_MINUTES,
            health_timeout: HEALTH_TIMEOUT,
        }))
    }
}

pub struct PrivateKey(Zeroizing<String>);

impl PrivateKey {
    pub fn new(value: Vec<u8>) -> Result<Self> {
        let value = String::from_utf8(value).context("Hyperliquid credential is not UTF-8")?;
        if !value.starts_with("0x") || value.len() != 66 {
            bail!("Hyperliquid testnet private key has an invalid shape");
        }
        Ok(Self(Zeroizing::new(value)))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleEvent {
    Starting {
        schema: String,
        generation: u64,
        network: Network,
    },
    Healthy {
        schema: String,
        generation: u64,
        network: Network,
        venue: String,
        reconciliation_lookback_minutes: u16,
    },
    Stopped {
        schema: String,
        generation: u64,
        network: Network,
    },
}

impl LifecycleEvent {
    fn validate(&self, expected_generation: u64) -> Result<()> {
        let (schema, generation) = match self {
            Self::Starting {
                schema, generation, ..
            }
            | Self::Healthy {
                schema, generation, ..
            }
            | Self::Stopped {
                schema, generation, ..
            } => (schema, generation),
        };
        if schema != EVENT_SCHEMA {
            bail!("Nautilus lifecycle schema is not supported");
        }
        if *generation != expected_generation {
            bail!("Nautilus lifecycle event has a stale generation");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BookAction {
    Add,
    Update,
    Delete,
    Clear,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BookSide {
    Buy,
    Sell,
    NoOrderSide,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BookOrder {
    pub side: BookSide,
    pub price: String,
    pub size: String,
    pub order_id: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BookDelta {
    #[serde(rename = "type")]
    pub data_type: String,
    pub instrument_id: String,
    pub action: BookAction,
    pub order: Option<BookOrder>,
    pub flags: u8,
    pub sequence: u64,
    pub ts_event: u64,
    pub ts_init: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    Quote {
        schema: String,
        generation: u64,
        sequence: u64,
        network: Network,
        instrument_id: String,
        bid_price: String,
        ask_price: String,
        bid_size: String,
        ask_size: String,
        ts_event: u64,
        ts_init: u64,
    },
    Trade {
        schema: String,
        generation: u64,
        sequence: u64,
        network: Network,
        instrument_id: String,
        price: String,
        size: String,
        aggressor_side: String,
        trade_id: String,
        ts_event: u64,
        ts_init: u64,
    },
    Book {
        schema: String,
        generation: u64,
        sequence: u64,
        network: Network,
        instrument_id: String,
        deltas: Vec<BookDelta>,
        ts_event: u64,
        ts_init: u64,
    },
    Account {
        schema: String,
        generation: u64,
        sequence: u64,
        network: Network,
        #[serde(flatten)]
        state: serde_json::Map<String, serde_json::Value>,
    },
    Order {
        schema: String,
        generation: u64,
        sequence: u64,
        network: Network,
        #[serde(flatten)]
        state: serde_json::Map<String, serde_json::Value>,
    },
    OrderState {
        schema: String,
        generation: u64,
        sequence: u64,
        network: Network,
        venue: String,
        orders: Vec<serde_json::Value>,
        ts_init: u64,
    },
    Position {
        schema: String,
        generation: u64,
        sequence: u64,
        network: Network,
        #[serde(flatten)]
        state: serde_json::Map<String, serde_json::Value>,
    },
    PositionState {
        schema: String,
        generation: u64,
        sequence: u64,
        network: Network,
        venue: String,
        positions: Vec<serde_json::Value>,
        ts_init: u64,
    },
    Fill {
        schema: String,
        generation: u64,
        sequence: u64,
        network: Network,
        #[serde(flatten)]
        state: serde_json::Map<String, serde_json::Value>,
    },
}

impl StreamEvent {
    fn validate(&self, expected_generation: u64) -> Result<()> {
        let (schema, generation, network) = match self {
            Self::Quote {
                schema,
                generation,
                network,
                ..
            }
            | Self::Trade {
                schema,
                generation,
                network,
                ..
            }
            | Self::Book {
                schema,
                generation,
                network,
                ..
            }
            | Self::Account {
                schema,
                generation,
                network,
                ..
            }
            | Self::Order {
                schema,
                generation,
                network,
                ..
            }
            | Self::OrderState {
                schema,
                generation,
                network,
                ..
            }
            | Self::Position {
                schema,
                generation,
                network,
                ..
            }
            | Self::PositionState {
                schema,
                generation,
                network,
                ..
            }
            | Self::Fill {
                schema,
                generation,
                network,
                ..
            } => (schema, generation, network),
        };
        if schema != STREAM_SCHEMA {
            bail!("Nautilus stream schema is not supported");
        }
        if *generation != expected_generation {
            bail!("Nautilus stream event has a stale generation");
        }
        if *network != Network::Testnet {
            bail!("Nautilus stream event is not testnet");
        }
        Ok(())
    }

    fn is_lossless_state(&self) -> bool {
        matches!(
            self,
            Self::Account { .. }
                | Self::Order { .. }
                | Self::OrderState { .. }
                | Self::Position { .. }
                | Self::PositionState { .. }
                | Self::Fill { .. }
        )
    }
}

#[derive(Default)]
struct StreamBuffer {
    latest_quote: Option<StreamEvent>,
    latest_book: Option<StreamEvent>,
    trades: VecDeque<StreamEvent>,
    lossless_state: VecDeque<StreamEvent>,
}

impl StreamBuffer {
    fn ingest(&mut self, event: StreamEvent) {
        if event.is_lossless_state() {
            self.lossless_state.push_back(event);
        } else {
            match event {
                event @ StreamEvent::Quote { .. } => self.latest_quote = Some(event),
                event @ StreamEvent::Book { .. } => self.latest_book = Some(event),
                event @ StreamEvent::Trade { .. } => {
                    if self.trades.len() == TRADE_BUFFER_CAPACITY {
                        self.trades.pop_front();
                    }
                    self.trades.push_back(event);
                }
                _ => {}
            }
        }
    }

    fn take_frame(&mut self) -> StreamFrame {
        StreamFrame {
            quote: self.latest_quote.take(),
            book: self.latest_book.take(),
            trades: self
                .trades
                .drain(..self.trades.len().min(TRADES_PER_FRAME))
                .collect(),
            state: self.lossless_state.drain(..).collect(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct StreamFrame {
    pub quote: Option<StreamEvent>,
    pub book: Option<StreamEvent>,
    pub trades: Vec<StreamEvent>,
    pub state: Vec<StreamEvent>,
}

impl StreamFrame {
    fn is_empty(&self) -> bool {
        self.quote.is_none()
            && self.book.is_none()
            && self.trades.is_empty()
            && self.state.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderableBookLevel {
    pub price: String,
    pub size: String,
}

#[derive(Default)]
pub struct NautilusStreamSource {
    latest_quote: Option<StreamEvent>,
    bids: HashMap<String, String>,
    asks: HashMap<String, String>,
    trade_count: u64,
    state_event_count: u64,
    frame_count: u64,
}

impl NautilusStreamSource {
    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalNautilusStreamSource>()
            .map(|source| source.0.clone())
    }

    pub fn latest_quote(&self) -> Option<&StreamEvent> {
        self.latest_quote.as_ref()
    }

    pub fn book_levels(&self) -> (Vec<RenderableBookLevel>, Vec<RenderableBookLevel>) {
        let levels = |side: &HashMap<String, String>| {
            side.iter()
                .map(|(price, size)| RenderableBookLevel {
                    price: price.clone(),
                    size: size.clone(),
                })
                .collect()
        };
        (levels(&self.bids), levels(&self.asks))
    }

    pub fn counts(&self) -> (u64, u64, u64) {
        (self.trade_count, self.state_event_count, self.frame_count)
    }

    fn apply_frame(&mut self, frame: StreamFrame, cx: &mut Context<Self>) {
        if let Some(quote) = frame.quote {
            self.latest_quote = Some(quote);
        }
        if let Some(StreamEvent::Book { deltas, .. }) = frame.book {
            for delta in deltas {
                let Some(order) = delta.order else {
                    if delta.action == BookAction::Clear {
                        self.bids.clear();
                        self.asks.clear();
                    }
                    continue;
                };
                let side = match order.side {
                    BookSide::Buy => &mut self.bids,
                    BookSide::Sell => &mut self.asks,
                    BookSide::NoOrderSide => continue,
                };
                match delta.action {
                    BookAction::Add | BookAction::Update => {
                        side.insert(order.price, order.size);
                    }
                    BookAction::Delete => {
                        side.remove(&order.price);
                    }
                    BookAction::Clear => side.clear(),
                }
            }
        }
        self.trade_count = self.trade_count.saturating_add(frame.trades.len() as u64);
        self.state_event_count = self
            .state_event_count
            .saturating_add(frame.state.len() as u64);
        self.frame_count = self.frame_count.saturating_add(1);
        cx.notify();
    }
}

struct GlobalNautilusStreamSource(Entity<NautilusStreamSource>);

impl Global for GlobalNautilusStreamSource {}

pub struct NautilusSupervisor {
    config: NautilusConfig,
    private_key: PrivateKey,
    child: Option<Child>,
    events: Option<mpsc::Receiver<Result<LifecycleEvent>>>,
    generation: u64,
    last_health: Option<LifecycleEvent>,
    stream: Arc<Mutex<StreamBuffer>>,
}

impl NautilusSupervisor {
    pub fn new(config: NautilusConfig, private_key: PrivateKey) -> Result<Self> {
        if config.network != Network::Testnet {
            bail!("Nautilus mainnet is disabled; only testnet is permitted");
        }
        Ok(Self {
            config,
            private_key,
            child: None,
            events: None,
            generation: 0,
            last_health: None,
            stream: Arc::new(Mutex::new(StreamBuffer::default())),
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn stream(&self) -> Arc<Mutex<StreamBuffer>> {
        self.stream.clone()
    }

    pub fn take_stream_frame(&self) -> Result<StreamFrame> {
        Ok(self
            .stream
            .lock()
            .map_err(|_| anyhow!("Nautilus stream buffer lock is poisoned"))?
            .take_frame())
    }

    // This whole synchronous lifecycle is run inside `smol::unblock`; using
    // the async process wrapper here would force child ownership across two
    // runtimes and make the app-quit path unable to synchronously reap it.
    #[allow(clippy::disallowed_methods)]
    pub fn start(&mut self) -> Result<LifecycleEvent> {
        if self.child.is_some() {
            bail!("Nautilus sidecar is already running");
        }
        self.generation = self.generation.saturating_add(1).max(1);
        let mut command = Command::new(&self.config.python);
        command
            .arg(&self.config.engine)
            .arg("--network")
            .arg("testnet")
            .arg("--generation")
            .arg(self.generation.to_string())
            .arg("--reconciliation-lookback-minutes")
            .arg(self.config.reconciliation_lookback_minutes.to_string())
            .env("HYPERLIQUID_TESTNET_PK", self.private_key.0.as_str())
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().with_context(|| {
            format!(
                "start Nautilus testnet sidecar with {}",
                self.config.python.display()
            )
        })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Nautilus sidecar stdout is unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("Nautilus sidecar stderr is unavailable"))?;
        let (sender, receiver) = mpsc::channel();
        let stream = self.stream.clone();
        let expected_generation = self.generation;
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = match line {
                    Ok(line) => line,
                    Err(error) => {
                        if sender.send(Err(error.into())).is_err() {
                            return;
                        }
                        return;
                    }
                };
                if let Some(payload) = line.strip_prefix(EVENT_PREFIX) {
                    let event =
                        serde_json::from_str(payload).context("decode Nautilus lifecycle event");
                    if sender.send(event).is_err() {
                        return;
                    }
                } else if let Some(payload) = line.strip_prefix(STREAM_PREFIX) {
                    let event = serde_json::from_str::<StreamEvent>(payload)
                        .context("decode Nautilus stream event")
                        .and_then(|event| {
                            event.validate(expected_generation)?;
                            Ok(event)
                        });
                    match event {
                        Ok(event) => match stream.lock() {
                            Ok(mut stream) => stream.ingest(event),
                            Err(_) => {
                                log::error!("Nautilus stream buffer lock is poisoned");
                                return;
                            }
                        },
                        Err(error) => log::warn!("Rejected Nautilus stream event: {error:#}"),
                    }
                }
            }
        });
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                match line {
                    Ok(line) => log::info!("Nautilus sidecar: {line}"),
                    Err(error) => {
                        log::warn!("Nautilus sidecar stderr read failed: {error}");
                        return;
                    }
                }
            }
        });
        self.child = Some(child);
        self.events = Some(receiver);
        self.wait_for_health()
    }

    pub fn ensure_healthy(&mut self) -> Result<LifecycleEvent> {
        let crashed = match self.child.as_mut() {
            Some(child) => child
                .try_wait()
                .context("inspect Nautilus sidecar")?
                .is_some(),
            None => true,
        };
        if crashed {
            self.child.take();
            self.events.take();
            self.last_health.take();
            return self.start();
        }
        self.last_health
            .clone()
            .ok_or_else(|| anyhow!("Nautilus sidecar has not reported health"))
    }

    pub fn stop(&mut self) -> Result<()> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        self.events.take();
        self.last_health.take();
        request_clean_stop(&mut child)?;
        let deadline = Instant::now() + SHUTDOWN_GRACE_PERIOD;
        while Instant::now() < deadline {
            if child
                .try_wait()
                .context("inspect Nautilus shutdown")?
                .is_some()
            {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        child.kill().context("kill unresponsive Nautilus sidecar")?;
        child.wait().context("reap killed Nautilus sidecar")?;
        bail!("Nautilus sidecar did not stop within the grace period")
    }

    fn wait_for_health(&mut self) -> Result<LifecycleEvent> {
        let receiver = self
            .events
            .as_ref()
            .ok_or_else(|| anyhow!("Nautilus lifecycle event channel is unavailable"))?;
        let deadline = Instant::now() + self.config.health_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = receiver
                .recv_timeout(remaining)
                .context("Nautilus sidecar health timed out")??;
            event.validate(self.generation)?;
            if matches!(event, LifecycleEvent::Healthy { .. }) {
                self.last_health = Some(event.clone());
                return Ok(event);
            }
        }
    }
}

impl Drop for NautilusSupervisor {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            log::warn!("Nautilus sidecar cleanup failed: {error:#}");
        }
    }
}

#[cfg(unix)]
fn request_clean_stop(child: &mut Child) -> Result<()> {
    // SAFETY: Nautilus owns SIGTERM shutdown; this targets the exact PID from Child.
    let result = unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(error).context("request Nautilus sidecar shutdown")
}

#[cfg(not(unix))]
fn request_clean_stop(child: &mut Child) -> Result<()> {
    child.kill().context("terminate Nautilus sidecar")
}

struct NautilusLifecycle {
    _quit_subscription: Subscription,
}

impl Global for NautilusLifecycle {}

pub fn init(cx: &mut App) {
    let config = match NautilusConfig::from_process_environment() {
        Ok(Some(config)) => config,
        Ok(None) => return,
        Err(error) => {
            log::error!("Refusing Nautilus sidecar configuration: {error:#}");
            return;
        }
    };
    let supervisor = Arc::new(Mutex::new(None::<NautilusSupervisor>));
    let stream_source = cx.new(|_| NautilusStreamSource::default());
    cx.set_global(GlobalNautilusStreamSource(stream_source.clone()));
    let shutting_down = Arc::new(AtomicBool::new(false));
    let quit_subscription = cx.on_app_quit({
        let supervisor = supervisor.clone();
        let shutting_down = shutting_down.clone();
        move |_| {
            shutting_down.store(true, Ordering::SeqCst);
            let supervisor = supervisor.clone();
            async move {
                if let Err(error) = smol::unblock(move || -> Result<()> {
                    let mut guard = supervisor
                        .lock()
                        .map_err(|_| anyhow!("Nautilus supervisor lock is poisoned"))?;
                    if let Some(supervisor) = guard.as_mut() {
                        supervisor.stop()?;
                    }
                    Ok(())
                })
                .await
                {
                    log::warn!("Nautilus sidecar shutdown failed: {error:#}");
                }
            }
        }
    });
    cx.set_global(NautilusLifecycle {
        _quit_subscription: quit_subscription,
    });
    let credentials = zed_credentials_provider::local_credentials(cx);
    let background_executor = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        let (_, private_key) = credentials
            .read_credentials(CREDENTIAL_KEY, cx)
            .await?
            .ok_or_else(|| anyhow!("Hyperliquid testnet credential is not configured"))?;
        let sidecar = NautilusSupervisor::new(config, PrivateKey::new(private_key)?)?;
        let stream = sidecar.stream();
        {
            let supervisor = supervisor.clone();
            smol::unblock(move || -> Result<()> {
                let mut guard = supervisor
                    .lock()
                    .map_err(|_| anyhow!("Nautilus supervisor lock is poisoned"))?;
                *guard = Some(sidecar);
                guard
                    .as_mut()
                    .ok_or_else(|| anyhow!("Nautilus supervisor is unavailable"))?
                    .start()?;
                Ok(())
            })
            .await?;
        }
        let mut last_monitor = Instant::now();
        while !shutting_down.load(Ordering::SeqCst) {
            background_executor.timer(FRAME_INTERVAL).await;
            let frame = stream
                .lock()
                .map_err(|_| anyhow!("Nautilus stream buffer lock is poisoned"))?
                .take_frame();
            if !frame.is_empty() {
                stream_source.update(cx, |source, cx| source.apply_frame(frame, cx));
            }
            if last_monitor.elapsed() >= MONITOR_INTERVAL {
                let supervisor = supervisor.clone();
                smol::unblock(move || -> Result<()> {
                    supervisor
                        .lock()
                        .map_err(|_| anyhow!("Nautilus supervisor lock is poisoned"))?
                        .as_mut()
                        .ok_or_else(|| anyhow!("Nautilus supervisor is unavailable"))?
                        .ensure_healthy()?;
                    Ok(())
                })
                .await?;
                last_monitor = Instant::now();
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .detach_and_log_err(cx);
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gpui::TestAppContext;

    use super::*;

    fn private_key() -> PrivateKey {
        PrivateKey::new(format!("0x{}", "1".repeat(64)).into_bytes()).expect("test key")
    }

    #[test]
    fn mainnet_is_hard_refused() {
        assert!(Network::parse("mainnet").is_err());
        assert!(Network::parse("testnet").is_ok());
    }

    #[test]
    fn lifecycle_events_require_the_version_and_generation() {
        let event = LifecycleEvent::Healthy {
            schema: EVENT_SCHEMA.into(),
            generation: 2,
            network: Network::Testnet,
            venue: "hyperliquid".into(),
            reconciliation_lookback_minutes: 60,
        };
        assert!(event.validate(2).is_ok());
        assert!(event.validate(1).is_err());
    }

    fn quote(sequence: u64, bid_price: &str) -> StreamEvent {
        serde_json::from_value(serde_json::json!({
            "type": "quote",
            "schema": STREAM_SCHEMA,
            "generation": 3,
            "sequence": sequence,
            "network": "testnet",
            "instrument_id": "BTC-USD-PERP.HYPERLIQUID",
            "bid_price": bid_price,
            "ask_price": "65001.0",
            "bid_size": "1.0",
            "ask_size": "2.0",
            "ts_event": sequence,
            "ts_init": sequence,
        }))
        .expect("valid quote")
    }

    fn order_state(sequence: u64) -> StreamEvent {
        serde_json::from_value(serde_json::json!({
            "type": "order_state",
            "schema": STREAM_SCHEMA,
            "generation": 3,
            "sequence": sequence,
            "network": "testnet",
            "venue": "HYPERLIQUID",
            "orders": [],
            "ts_init": sequence,
        }))
        .expect("valid order state")
    }

    #[test]
    fn stream_events_are_versioned_generation_fenced_and_testnet_only() {
        let event = quote(1, "65000.0");
        assert!(event.validate(3).is_ok());
        assert!(event.validate(2).is_err());

        let mainnet = serde_json::json!({
            "type": "quote",
            "schema": STREAM_SCHEMA,
            "generation": 3,
            "sequence": 1,
            "network": "mainnet",
            "instrument_id": "BTC-USD-PERP.HYPERLIQUID",
            "bid_price": "65000.0",
            "ask_price": "65001.0",
            "bid_size": "1.0",
            "ask_size": "2.0",
            "ts_event": 1,
            "ts_init": 1,
        });
        assert!(serde_json::from_value::<StreamEvent>(mainnet).is_err());
    }

    #[test]
    fn market_snapshots_coalesce_while_state_events_are_lossless() {
        let mut stream = StreamBuffer::default();
        stream.ingest(quote(1, "65000.0"));
        stream.ingest(order_state(2));
        stream.ingest(quote(3, "65002.0"));
        stream.ingest(order_state(4));

        let frame = stream.take_frame();
        assert!(matches!(
            frame.quote,
            Some(StreamEvent::Quote { sequence: 3, .. })
        ));
        assert_eq!(frame.state.len(), 2);
        assert!(stream.take_frame().is_empty());
    }

    #[gpui::test]
    async fn app_source_reconstructs_a_renderable_book_once_per_frame(cx: &mut TestAppContext) {
        let source = cx.new(|_| NautilusStreamSource::default());
        let book = serde_json::from_value(serde_json::json!({
            "type": "book",
            "schema": STREAM_SCHEMA,
            "generation": 3,
            "sequence": 5,
            "network": "testnet",
            "instrument_id": "BTC-USD-PERP.HYPERLIQUID",
            "deltas": [{
                "type": "OrderBookDelta",
                "instrument_id": "BTC-USD-PERP.HYPERLIQUID",
                "action": "ADD",
                "order": {"side": "BUY", "price": "65000.0", "size": "1.25", "order_id": 0},
                "flags": 0,
                "sequence": 1,
                "ts_event": 5,
                "ts_init": 5
            }],
            "ts_event": 5,
            "ts_init": 5
        }))
        .expect("valid book event");
        source.update(cx, |source, cx| {
            source.apply_frame(
                StreamFrame {
                    quote: Some(quote(4, "64999.0")),
                    book: Some(book),
                    trades: Vec::new(),
                    state: vec![order_state(6)],
                },
                cx,
            );
        });

        source.read_with(cx, |source, _| {
            let (bids, asks) = source.book_levels();
            assert_eq!(
                bids,
                vec![RenderableBookLevel {
                    price: "65000.0".into(),
                    size: "1.25".into(),
                }]
            );
            assert!(asks.is_empty());
            assert_eq!(source.counts(), (0, 1, 1));
            assert!(source.latest_quote().is_some());
        });
    }

    #[cfg(unix)]
    #[test]
    fn crashed_sidecar_restarts_with_a_new_generation_and_stops_cleanly() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let script = temporary_directory.path().join("fake-engine.sh");
        fs::write(
            &script,
            r#"#!/bin/sh
generation=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--generation" ]; then generation="$2"; shift 2; else shift; fi
done
printf 'OMEGA_NAUTILUS_EVENT {"type":"starting","schema":"omega.nautilus.lifecycle.v1","generation":%s,"network":"testnet"}\n' "$generation"
printf 'OMEGA_NAUTILUS_EVENT {"type":"healthy","schema":"omega.nautilus.lifecycle.v1","generation":%s,"network":"testnet","venue":"hyperliquid","reconciliation_lookback_minutes":60}\n' "$generation"
trap 'exit 0' TERM
while :; do sleep 1; done
"#,
        )
        .expect("write fake engine");
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700))
            .expect("make fake engine executable");
        let config = NautilusConfig {
            network: Network::Testnet,
            python: script,
            engine: PathBuf::from("ignored"),
            reconciliation_lookback_minutes: 60,
            health_timeout: Duration::from_secs(2),
        };
        let mut supervisor = NautilusSupervisor::new(config, private_key()).expect("supervisor");
        supervisor.start().expect("first start");
        assert_eq!(supervisor.generation(), 1);
        let child = supervisor.child.as_mut().expect("running child");
        child.kill().expect("crash fake engine");
        child.wait().expect("reap crashed fake engine");
        supervisor.ensure_healthy().expect("restart after crash");
        assert_eq!(supervisor.generation(), 2);
        supervisor.stop().expect("clean stop");
        assert!(supervisor.child.is_none());
    }
}
