use std::collections::{HashMap, VecDeque};
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{App, AppContext as _, Context, Entity, Global, Subscription, TaskExt as _};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize as _, Zeroizing};

mod agent_wallet;

pub use agent_wallet::{
    AGENT_WALLET_AUTHORITY_COPY, AgentApprovalStatus, AgentWalletHaltReason, AgentWalletSummary,
    OfficialOpenOrder, OfficialPosition, OfficialVenueState, StoredAgentWallet, credential_key,
    evaluate_extra_agents, evaluate_official_venue_state, generate_and_store_agent_wallet,
    load_agent_wallet, probe_official_venue_state, refresh_agent_wallet_approval,
};

pub const ENABLE_ENVIRONMENT_VARIABLE: &str = "OMEGA_NAUTILUS_SIDECAR";
pub const NETWORK_ENVIRONMENT_VARIABLE: &str = "OMEGA_NAUTILUS_NETWORK";
const EVENT_PREFIX: &str = "OMEGA_NAUTILUS_EVENT ";
const EVENT_SCHEMA: &str = "omega.nautilus.lifecycle.v1";
const STREAM_PREFIX: &str = "OMEGA_NAUTILUS_STREAM ";
const STREAM_SCHEMA: &str = "omega.nautilus.stream.v1";
const COMMAND_SCHEMA: &str = "omega.nautilus.command.v1";
const DEFAULT_RECONCILIATION_LOOKBACK_MINUTES: u16 = 60;
const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(15);
const MONITOR_INTERVAL: Duration = Duration::from_secs(2);
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const TRADE_BUFFER_CAPACITY: usize = 2_048;
const TRADES_PER_FRAME: usize = 256;
const STATE_SNAPSHOT_CAPACITY: usize = 2_048;
const MARKET_SNAPSHOT_TRADE_CAPACITY: usize = 4_096;
const MARKET_SNAPSHOT_FILL_CAPACITY: usize = 1_024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    Testnet,
    Mainnet,
}

impl Network {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "testnet" => Ok(Self::Testnet),
            "mainnet" => Ok(Self::Mainnet),
            _ => bail!("unsupported Nautilus network {value:?}"),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Testnet => "Testnet",
            Self::Mainnet => "Mainnet",
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
        if network == Network::Mainnet {
            bail!("Nautilus mainnet is disabled until the mainnet graduation gate passes");
        }
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

    pub fn ethereum_address(&self) -> Result<String> {
        let encoded = self
            .0
            .strip_prefix("0x")
            .context("Hyperliquid private key has no hexadecimal prefix")?;
        let bytes = hex::decode(encoded).context("decode Hyperliquid private key")?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow!("Hyperliquid private key is not 32 bytes"))?;
        let secret_key = secp256k1::SecretKey::from_byte_array(bytes)
            .context("parse Hyperliquid private key")?;
        let public_key =
            secp256k1::PublicKey::from_secret_key(&secp256k1::Secp256k1::new(), &secret_key);
        agent_wallet::ethereum_address(&public_key)
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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandType {
    PlaceOrder,
    CancelOrder,
    StartStrategy,
    StopStrategy,
    SetStrategyParameters,
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
    StrategyState {
        schema: String,
        generation: u64,
        sequence: u64,
        network: Network,
        strategy_id: String,
        phase: String,
        running: bool,
        halted_reason: Option<String>,
        mandate_revision: u64,
        quote_ticks: u64,
        trade_ticks: u64,
        book_ticks: u64,
        action_count: u64,
        active_client_order_id: Option<String>,
        #[serde(default)]
        budget_wait_until_ns: Option<u64>,
        ts_init: u64,
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
            }
            | Self::StrategyState {
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
                | Self::StrategyState { .. }
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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyParameters {
    pub min_reprice_interval_ms: u64,
    pub quote_offset_bps: u32,
    pub order_quantity: String,
    pub position_headroom_usd: u64,
    pub order_budget: u32,
    pub mandate_revision: u64,
    pub cost_path: String,
    pub cost_clip_usd: u64,
    pub cost_sample_count: u32,
    pub measured_round_trip_cost_micros_bps: i64,
    pub cost_margin_bps: u32,
    pub admission_floor_bps: u32,
    pub cost_evidence_sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderTimeInForce {
    Gtc,
    Ioc,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum NautilusCommand {
    PlaceOrder {
        client_order_id: String,
        instrument_id: String,
        side: OrderSide,
        quantity: String,
        price: String,
        time_in_force: OrderTimeInForce,
        post_only: bool,
        reduce_only: bool,
    },
    CancelOrder {
        client_order_id: String,
    },
    StartStrategy {
        strategy_id: String,
        cost_evidence_sha256: String,
    },
    StopStrategy {
        strategy_id: String,
    },
    SetStrategyParameters {
        strategy_id: String,
        parameters: StrategyParameters,
    },
}

impl NautilusCommand {
    fn command_type(&self) -> CommandType {
        match self {
            Self::PlaceOrder { .. } => CommandType::PlaceOrder,
            Self::CancelOrder { .. } => CommandType::CancelOrder,
            Self::StartStrategy { .. } => CommandType::StartStrategy,
            Self::StopStrategy { .. } => CommandType::StopStrategy,
            Self::SetStrategyParameters { .. } => CommandType::SetStrategyParameters,
        }
    }

    fn validate(&self) -> Result<()> {
        match self {
            Self::PlaceOrder {
                client_order_id,
                instrument_id,
                quantity,
                price,
                ..
            } => {
                validate_identifier("client order ID", client_order_id)?;
                validate_identifier("instrument ID", instrument_id)?;
                validate_decimal("quantity", quantity)?;
                validate_decimal("price", price)?;
            }
            Self::CancelOrder { client_order_id } => {
                validate_identifier("client order ID", client_order_id)?;
            }
            Self::StartStrategy {
                strategy_id,
                cost_evidence_sha256,
            } => {
                validate_identifier("strategy ID", strategy_id)?;
                if cost_evidence_sha256.len() != 64
                    || !cost_evidence_sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    bail!("strategy start has no measured cost evidence binding");
                }
            }
            Self::StopStrategy { strategy_id }
            | Self::SetStrategyParameters { strategy_id, .. } => {
                validate_identifier("strategy ID", strategy_id)?;
            }
        }
        if let Self::SetStrategyParameters { parameters, .. } = self {
            let measured_ceiling_bps = parameters
                .measured_round_trip_cost_micros_bps
                .max(0)
                .checked_add(999_999)
                .context("measured cost ceiling overflowed")?
                / 1_000_000;
            let expected_floor = u32::try_from(measured_ceiling_bps)?
                .checked_add(parameters.cost_margin_bps)
                .context("measured cost admission floor overflowed")?;
            let digest_is_lower_hex = parameters.cost_evidence_sha256.len() == 64
                && parameters
                    .cost_evidence_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
            if !(100..=60_000).contains(&parameters.min_reprice_interval_ms)
                || parameters.quote_offset_bps > 1_000
                || parameters.order_quantity != "0.001"
                || parameters.position_headroom_usd == 0
                || !(1..=100).contains(&parameters.order_budget)
                || parameters.mandate_revision == 0
                || parameters.cost_path != "maker_taker"
                || parameters.cost_clip_usd != 65
                || parameters.cost_sample_count < 5
                || parameters.cost_margin_bps == 0
                || parameters.admission_floor_bps != expected_floor
                || parameters.quote_offset_bps < parameters.admission_floor_bps
                || !digest_is_lower_hex
            {
                bail!("bounded quote strategy parameters exceed the testnet envelope");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    pub command_id: String,
    #[serde(flatten)]
    pub command: NautilusCommand,
}

impl CommandRequest {
    fn validate(&self) -> Result<()> {
        validate_identifier("command ID", &self.command_id)?;
        self.command.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderableBookLevel {
    pub price: String,
    pub size: String,
}

/// The retained, render-facing view of the typed testnet stream. Market data
/// may be coalesced before it reaches a frame, while account/order/position/
/// fill state is retained from the lossless queue. Consumers never parse the
/// sidecar's raw NDJSON or infer venue state from lifecycle logs.
#[derive(Clone, Debug, Default)]
pub struct NautilusMarketSnapshot {
    pub latest_quote: Option<StreamEvent>,
    pub recent_trades: Vec<StreamEvent>,
    pub bids: Vec<RenderableBookLevel>,
    pub asks: Vec<RenderableBookLevel>,
    pub account: Option<StreamEvent>,
    pub orders: Vec<serde_json::Value>,
    pub positions: Vec<serde_json::Value>,
    pub recent_fills: Vec<StreamEvent>,
    pub frame_count: u64,
}

#[derive(Default)]
pub struct NautilusStreamSource {
    latest_quote: Option<StreamEvent>,
    bids: HashMap<String, String>,
    asks: HashMap<String, String>,
    recent_trades: VecDeque<StreamEvent>,
    account: Option<StreamEvent>,
    orders: Vec<serde_json::Value>,
    positions: Vec<serde_json::Value>,
    recent_fills: VecDeque<StreamEvent>,
    trade_count: u64,
    state_event_count: u64,
    frame_count: u64,
    state: VecDeque<StreamEvent>,
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

    /// Lossless venue/account state retained for governance consumers. The
    /// sidecar remains the sole stdout reader; governance observes this typed
    /// projection and never competes with the hot stream loop.
    pub fn state_snapshot(&self) -> Vec<StreamEvent> {
        self.state.iter().cloned().collect()
    }

    pub fn market_snapshot(&self) -> NautilusMarketSnapshot {
        let mut bids = self
            .bids
            .iter()
            .map(|(price, size)| RenderableBookLevel {
                price: price.clone(),
                size: size.clone(),
            })
            .collect::<Vec<_>>();
        let mut asks = self
            .asks
            .iter()
            .map(|(price, size)| RenderableBookLevel {
                price: price.clone(),
                size: size.clone(),
            })
            .collect::<Vec<_>>();
        let price_order = |left: &RenderableBookLevel, right: &RenderableBookLevel| {
            left.price
                .parse::<f64>()
                .ok()
                .zip(right.price.parse::<f64>().ok())
                .and_then(|(left, right)| left.partial_cmp(&right))
                .unwrap_or(std::cmp::Ordering::Equal)
        };
        bids.sort_by(|left, right| price_order(right, left));
        asks.sort_by(price_order);
        NautilusMarketSnapshot {
            latest_quote: self.latest_quote.clone(),
            recent_trades: self.recent_trades.iter().cloned().collect(),
            bids,
            asks,
            account: self.account.clone(),
            orders: self.orders.clone(),
            positions: self.positions.clone(),
            recent_fills: self.recent_fills.iter().cloned().collect(),
            frame_count: self.frame_count,
        }
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
        for trade in frame.trades {
            if self.recent_trades.len() >= MARKET_SNAPSHOT_TRADE_CAPACITY {
                self.recent_trades.pop_front();
            }
            self.recent_trades.push_back(trade);
        }
        let state_count = frame.state.len();
        for event in frame.state {
            if self.state.len() == STATE_SNAPSHOT_CAPACITY {
                self.state.pop_front();
            }
            self.state.push_back(event.clone());
            match event {
                event @ StreamEvent::Account { .. } => self.account = Some(event),
                StreamEvent::OrderState { orders, .. } => self.orders = orders,
                StreamEvent::PositionState { positions, .. } => self.positions = positions,
                event @ StreamEvent::Fill { .. } => {
                    if self.recent_fills.len() >= MARKET_SNAPSHOT_FILL_CAPACITY {
                        self.recent_fills.pop_front();
                    }
                    self.recent_fills.push_back(event);
                }
                StreamEvent::Order { .. }
                | StreamEvent::Position { .. }
                | StreamEvent::StrategyState { .. } => {}
                StreamEvent::Quote { .. }
                | StreamEvent::Trade { .. }
                | StreamEvent::Book { .. } => {}
            }
        }
        self.state_event_count = self.state_event_count.saturating_add(state_count as u64);
        self.frame_count = self.frame_count.saturating_add(1);
        cx.notify();
    }
}

struct GlobalNautilusStreamSource(Entity<NautilusStreamSource>);

impl Global for GlobalNautilusStreamSource {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandOutcome {
    OrderAccepted {
        client_order_id: String,
        venue_order_id: String,
    },
    OrderCanceled {
        client_order_id: String,
        venue_order_id: String,
    },
    StrategyStarted {
        running: bool,
    },
    StrategyStopped {
        running: bool,
    },
    StrategyParametersApplied {
        parameters: StrategyParameters,
    },
    Refused {
        reason_code: RefusalReason,
    },
    Unknown {
        reason_code: UnknownReason,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    InvalidCommand,
    InvalidEnvelope,
    InvalidCommandId,
    UnknownStrategy,
    InvalidOrder,
    InvalidParameters,
    OrderNotFound,
    RiskDenied,
    VenueRejected,
    CancelRejected,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    DispatchFailed,
    PostDispatchRiskDenied,
    PostDispatchVenueRejected,
    PostDispatchCancelRejected,
    TransportClosed,
    Timeout,
    MalformedOutcome,
}

impl RefusalReason {
    fn post_dispatch_unknown(self) -> Option<UnknownReason> {
        match self {
            Self::RiskDenied => Some(UnknownReason::PostDispatchRiskDenied),
            Self::VenueRejected => Some(UnknownReason::PostDispatchVenueRejected),
            Self::CancelRejected => Some(UnknownReason::PostDispatchCancelRejected),
            Self::InvalidCommand
            | Self::InvalidEnvelope
            | Self::InvalidCommandId
            | Self::UnknownStrategy
            | Self::InvalidOrder
            | Self::InvalidParameters
            | Self::OrderNotFound => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommandReceipt {
    pub command_id: String,
    pub command_type: CommandType,
    pub acknowledged: bool,
    pub sent: bool,
    pub outcome: CommandOutcome,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum CommandEvent {
    Acknowledged {
        schema: String,
        generation: u64,
        network: Network,
        command_id: String,
        command_type: CommandType,
    },
    Sent {
        schema: String,
        generation: u64,
        network: Network,
        command_id: String,
        command_type: CommandType,
        client_order_id: String,
        mutation_state: MutationState,
    },
    OrderAccepted {
        schema: String,
        generation: u64,
        network: Network,
        command_id: String,
        command_type: CommandType,
        client_order_id: String,
        venue_order_id: String,
    },
    OrderCanceled {
        schema: String,
        generation: u64,
        network: Network,
        command_id: String,
        command_type: CommandType,
        client_order_id: String,
        venue_order_id: String,
    },
    StrategyStarted {
        schema: String,
        generation: u64,
        network: Network,
        command_id: String,
        command_type: CommandType,
        running: bool,
    },
    StrategyStopped {
        schema: String,
        generation: u64,
        network: Network,
        command_id: String,
        command_type: CommandType,
        running: bool,
    },
    StrategyParametersApplied {
        schema: String,
        generation: u64,
        network: Network,
        command_id: String,
        command_type: CommandType,
        parameters: StrategyParameters,
    },
    Refused {
        schema: String,
        generation: u64,
        network: Network,
        command_id: String,
        command_type: CommandType,
        reason_code: RefusalReason,
    },
    Unknown {
        schema: String,
        generation: u64,
        network: Network,
        command_id: String,
        command_type: CommandType,
        client_order_id: String,
        mutation_state: MutationState,
        reason_code: UnknownReason,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum MutationState {
    Sent,
    Unknown,
}

impl CommandEvent {
    fn metadata(&self) -> (&str, u64, Network, &str, CommandType) {
        match self {
            Self::Acknowledged {
                schema,
                generation,
                network,
                command_id,
                command_type,
            }
            | Self::Sent {
                schema,
                generation,
                network,
                command_id,
                command_type,
                ..
            }
            | Self::OrderAccepted {
                schema,
                generation,
                network,
                command_id,
                command_type,
                ..
            }
            | Self::OrderCanceled {
                schema,
                generation,
                network,
                command_id,
                command_type,
                ..
            }
            | Self::StrategyStarted {
                schema,
                generation,
                network,
                command_id,
                command_type,
                ..
            }
            | Self::StrategyStopped {
                schema,
                generation,
                network,
                command_id,
                command_type,
                ..
            }
            | Self::StrategyParametersApplied {
                schema,
                generation,
                network,
                command_id,
                command_type,
                ..
            }
            | Self::Refused {
                schema,
                generation,
                network,
                command_id,
                command_type,
                ..
            }
            | Self::Unknown {
                schema,
                generation,
                network,
                command_id,
                command_type,
                ..
            } => (schema, *generation, *network, command_id, *command_type),
        }
    }

    fn validate(&self, expected_generation: u64) -> Result<()> {
        let (schema, generation, network, _, _) = self.metadata();
        if schema != COMMAND_SCHEMA {
            bail!("Nautilus command schema is not supported");
        }
        if generation != expected_generation {
            bail!("Nautilus command event has a stale generation");
        }
        if network != Network::Testnet {
            bail!("Nautilus command event is not testnet");
        }
        Ok(())
    }
}

enum SidecarEvent {
    Lifecycle(LifecycleEvent),
    Command(CommandEvent),
}

#[derive(Deserialize)]
struct EventSchema {
    schema: String,
}

#[derive(Serialize)]
struct CommandEnvelope<'a> {
    schema: &'static str,
    generation: u64,
    network: Network,
    command_id: &'a str,
    #[serde(flatten)]
    command: &'a NautilusCommand,
}

#[derive(Serialize)]
struct BootstrapEnvelope<'a> {
    schema: &'static str,
    network: Network,
    private_key: &'a str,
    owner_address: &'a str,
    agent_address: Option<&'a str>,
    agent_name: Option<&'a str>,
}

const BOOTSTRAP_SCHEMA: &str = "omega.nautilus.bootstrap.v1";

pub struct NautilusSupervisor {
    config: NautilusConfig,
    private_key: PrivateKey,
    agent_wallet: Option<AgentWalletSummary>,
    child: Option<Child>,
    child_stdin: Option<ChildStdin>,
    events: Option<mpsc::Receiver<Result<SidecarEvent>>>,
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
            agent_wallet: None,
            child: None,
            child_stdin: None,
            events: None,
            generation: 0,
            last_health: None,
            stream: Arc::new(Mutex::new(StreamBuffer::default())),
        })
    }

    pub fn with_agent_wallet(config: NautilusConfig, wallet: StoredAgentWallet) -> Result<Self> {
        if config.network != wallet.network {
            bail!("Nautilus configuration and agent-wallet networks do not match");
        }
        if let Some(halt) = wallet.summary().halt_reason(config.network) {
            bail!("Hyperliquid agent wallet is halted: {}", halt.code());
        }
        if !matches!(wallet.approval, AgentApprovalStatus::Approved { .. }) {
            bail!("Hyperliquid agent wallet has not been approved by its owner");
        }
        let private_key = PrivateKey::new(wallet.private_key_bytes())?;
        let mut supervisor = Self::new(config, private_key)?;
        supervisor.agent_wallet = Some(wallet.summary());
        Ok(supervisor)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn stream(&self) -> Arc<Mutex<StreamBuffer>> {
        self.stream.clone()
    }

    fn bootstrap_owner_address(&self) -> Result<String> {
        if let Some(wallet) = &self.agent_wallet {
            Ok(wallet.owner_address.clone())
        } else {
            self.private_key.ethereum_address()
        }
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
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .stdin(Stdio::piped())
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
        let child_stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Nautilus sidecar stdin is unavailable"))?;
        let mut child_stdin = child_stdin;
        let owner_address = self.bootstrap_owner_address()?;
        let bootstrap = BootstrapEnvelope {
            schema: BOOTSTRAP_SCHEMA,
            network: self.config.network,
            private_key: self.private_key.0.as_str(),
            owner_address: &owner_address,
            agent_address: self
                .agent_wallet
                .as_ref()
                .map(|wallet| wallet.agent_address.as_str()),
            agent_name: self
                .agent_wallet
                .as_ref()
                .map(|wallet| wallet.agent_name.as_str()),
        };
        let mut bootstrap = serde_json::to_vec(&bootstrap).context("encode Nautilus bootstrap")?;
        let write_result = child_stdin
            .write_all(&bootstrap)
            .and_then(|()| child_stdin.write_all(b"\n"))
            .and_then(|()| child_stdin.flush());
        bootstrap.zeroize();
        write_result.context("inject Nautilus bootstrap through stdin")?;
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
                    let event = decode_sidecar_event(payload);
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
        self.child_stdin = Some(child_stdin);
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
            self.child_stdin.take();
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
        self.child_stdin.take();
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
            if let SidecarEvent::Lifecycle(event) = event {
                event.validate(self.generation)?;
                if matches!(event, LifecycleEvent::Healthy { .. }) {
                    self.last_health = Some(event.clone());
                    return Ok(event);
                }
            }
        }
    }

    pub fn send_command(&mut self, request: CommandRequest) -> Result<CommandReceipt> {
        request.validate()?;
        let command_type = request.command.command_type();
        let envelope = CommandEnvelope {
            schema: COMMAND_SCHEMA,
            generation: self.generation,
            network: Network::Testnet,
            command_id: &request.command_id,
            command: &request.command,
        };
        let payload = serde_json::to_vec(&envelope).context("encode Nautilus command")?;
        let Some(child_stdin) = self.child_stdin.as_mut() else {
            bail!("Nautilus command channel is unavailable");
        };
        if child_stdin
            .write_all(&payload)
            .and_then(|()| child_stdin.write_all(b"\n"))
            .and_then(|()| child_stdin.flush())
            .is_err()
        {
            return Ok(unknown_receipt(
                request.command_id,
                command_type,
                false,
                false,
                UnknownReason::TransportClosed,
            ));
        }

        let receiver = self
            .events
            .as_ref()
            .ok_or_else(|| anyhow!("Nautilus sidecar event channel is unavailable"))?;
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        let mut acknowledged = false;
        let mut sent = false;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = match receiver.recv_timeout(remaining) {
                Ok(Ok(event)) => event,
                Ok(Err(_)) => {
                    return Ok(unknown_receipt(
                        request.command_id,
                        command_type,
                        acknowledged,
                        sent,
                        UnknownReason::MalformedOutcome,
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Ok(unknown_receipt(
                        request.command_id,
                        command_type,
                        acknowledged,
                        sent,
                        UnknownReason::Timeout,
                    ));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Ok(unknown_receipt(
                        request.command_id,
                        command_type,
                        acknowledged,
                        sent,
                        UnknownReason::TransportClosed,
                    ));
                }
            };
            let SidecarEvent::Command(event) = event else {
                continue;
            };
            if event.validate(self.generation).is_err() {
                return Ok(unknown_receipt(
                    request.command_id,
                    command_type,
                    acknowledged,
                    sent,
                    UnknownReason::MalformedOutcome,
                ));
            }
            let (_, _, _, event_command_id, event_command_type) = event.metadata();
            if event_command_id != request.command_id {
                continue;
            }
            if event_command_type != command_type {
                return Ok(unknown_receipt(
                    request.command_id,
                    command_type,
                    acknowledged,
                    sent,
                    UnknownReason::MalformedOutcome,
                ));
            }
            match event {
                CommandEvent::Acknowledged { .. } => acknowledged = true,
                CommandEvent::Sent {
                    mutation_state: MutationState::Sent,
                    client_order_id,
                    ..
                } => {
                    validate_identifier("sent client order ID", &client_order_id)?;
                    sent = true;
                }
                CommandEvent::Sent { .. } => {
                    return Ok(unknown_receipt(
                        request.command_id,
                        command_type,
                        acknowledged,
                        sent,
                        UnknownReason::MalformedOutcome,
                    ));
                }
                CommandEvent::OrderAccepted {
                    client_order_id,
                    venue_order_id,
                    ..
                } => {
                    return Ok(CommandReceipt {
                        command_id: request.command_id,
                        command_type,
                        acknowledged,
                        sent,
                        outcome: CommandOutcome::OrderAccepted {
                            client_order_id,
                            venue_order_id,
                        },
                    });
                }
                CommandEvent::OrderCanceled {
                    client_order_id,
                    venue_order_id,
                    ..
                } => {
                    return Ok(CommandReceipt {
                        command_id: request.command_id,
                        command_type,
                        acknowledged,
                        sent,
                        outcome: CommandOutcome::OrderCanceled {
                            client_order_id,
                            venue_order_id,
                        },
                    });
                }
                CommandEvent::StrategyStarted { running, .. } => {
                    return Ok(CommandReceipt {
                        command_id: request.command_id,
                        command_type,
                        acknowledged,
                        sent,
                        outcome: CommandOutcome::StrategyStarted { running },
                    });
                }
                CommandEvent::StrategyStopped { running, .. } => {
                    return Ok(CommandReceipt {
                        command_id: request.command_id,
                        command_type,
                        acknowledged,
                        sent,
                        outcome: CommandOutcome::StrategyStopped { running },
                    });
                }
                CommandEvent::StrategyParametersApplied { parameters, .. } => {
                    return Ok(CommandReceipt {
                        command_id: request.command_id,
                        command_type,
                        acknowledged,
                        sent,
                        outcome: CommandOutcome::StrategyParametersApplied { parameters },
                    });
                }
                CommandEvent::Refused { reason_code, .. } => {
                    if let Some(reason_code) = reason_code.post_dispatch_unknown() {
                        return Ok(unknown_receipt(
                            request.command_id,
                            command_type,
                            acknowledged,
                            sent,
                            reason_code,
                        ));
                    }
                    return Ok(CommandReceipt {
                        command_id: request.command_id,
                        command_type,
                        acknowledged,
                        sent,
                        outcome: CommandOutcome::Refused { reason_code },
                    });
                }
                CommandEvent::Unknown {
                    mutation_state: MutationState::Unknown,
                    reason_code,
                    client_order_id,
                    ..
                } => {
                    validate_identifier("unknown client order ID", &client_order_id)?;
                    return Ok(unknown_receipt(
                        request.command_id,
                        command_type,
                        acknowledged,
                        sent,
                        reason_code,
                    ));
                }
                CommandEvent::Unknown { .. } => {
                    return Ok(unknown_receipt(
                        request.command_id,
                        command_type,
                        acknowledged,
                        sent,
                        UnknownReason::MalformedOutcome,
                    ));
                }
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

fn decode_sidecar_event(payload: &str) -> Result<SidecarEvent> {
    let event_schema: EventSchema =
        serde_json::from_str(payload).context("decode Nautilus event schema")?;
    match event_schema.schema.as_str() {
        EVENT_SCHEMA => serde_json::from_str(payload)
            .map(SidecarEvent::Lifecycle)
            .context("decode Nautilus lifecycle event"),
        COMMAND_SCHEMA => serde_json::from_str(payload)
            .map(SidecarEvent::Command)
            .context("decode Nautilus command event"),
        _ => bail!("Nautilus event schema is not supported"),
    }
}

fn unknown_receipt(
    command_id: String,
    command_type: CommandType,
    acknowledged: bool,
    sent: bool,
    reason_code: UnknownReason,
) -> CommandReceipt {
    CommandReceipt {
        command_id,
        command_type,
        acknowledged,
        sent,
        outcome: CommandOutcome::Unknown { reason_code },
    }
}

fn validate_identifier(name: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 128 {
        bail!("{name} must contain from 1 through 128 bytes");
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        bail!("{name} contains unsupported characters");
    }
    Ok(())
}

fn validate_decimal(name: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
        || value.bytes().filter(|byte| *byte == b'.').count() > 1
    {
        bail!("{name} must be a positive decimal string");
    }
    let parsed: f64 = value
        .parse()
        .with_context(|| format!("parse {name} decimal"))?;
    if !parsed.is_finite() || parsed <= 0.0 {
        bail!("{name} must be a positive finite decimal");
    }
    Ok(())
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

#[derive(Clone)]
pub struct NautilusCommandChannel {
    supervisor: Arc<Mutex<Option<NautilusSupervisor>>>,
}

impl NautilusCommandChannel {
    pub async fn send(&self, request: CommandRequest) -> Result<CommandReceipt> {
        let supervisor = self.supervisor.clone();
        smol::unblock(move || {
            supervisor
                .lock()
                .map_err(|_| anyhow!("Nautilus supervisor lock is poisoned"))?
                .as_mut()
                .ok_or_else(|| anyhow!("Nautilus supervisor is unavailable"))?
                .send_command(request)
        })
        .await
    }
}

struct NautilusLifecycle {
    command_channel: NautilusCommandChannel,
    _quit_subscription: Subscription,
}

impl Global for NautilusLifecycle {}

#[derive(Clone, Debug)]
pub struct NautilusCredentialSnapshot {
    pub selected_network: Network,
    pub testnet: Option<AgentWalletSummary>,
    pub mainnet: Option<AgentWalletSummary>,
    pub halt: Option<AgentWalletHaltReason>,
    pub wakeup: Option<agent_wakeup::WakeupSource>,
    pub error: Option<String>,
    pub loading: bool,
}

impl Default for NautilusCredentialSnapshot {
    fn default() -> Self {
        Self {
            selected_network: Network::Testnet,
            testnet: None,
            mainnet: None,
            halt: None,
            wakeup: None,
            error: None,
            loading: true,
        }
    }
}

pub struct NautilusCredentialState {
    snapshot: NautilusCredentialSnapshot,
}

impl NautilusCredentialState {
    pub fn snapshot(&self) -> NautilusCredentialSnapshot {
        self.snapshot.clone()
    }

    fn replace(&mut self, snapshot: NautilusCredentialSnapshot, cx: &mut Context<Self>) {
        self.snapshot = snapshot;
        cx.notify();
    }

    pub fn apply_wallet(&mut self, wallet: AgentWalletSummary, cx: &mut Context<Self>) {
        let network = wallet.network;
        let halt = wallet.halt_reason(network);
        match wallet.network {
            Network::Testnet => self.snapshot.testnet = Some(wallet),
            Network::Mainnet => self.snapshot.mainnet = Some(wallet),
        }
        if self.snapshot.selected_network == network {
            self.snapshot.wakeup = halt
                .as_ref()
                .map(|halt| credential_halt_wakeup(network, halt));
            self.snapshot.halt = halt;
        }
        self.snapshot.error = None;
        self.snapshot.loading = false;
        cx.notify();
    }

    pub fn apply_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.snapshot.error = Some(error);
        self.snapshot.loading = false;
        cx.notify();
    }
}

struct GlobalNautilusCredentialState(Entity<NautilusCredentialState>);

impl Global for GlobalNautilusCredentialState {}

pub fn credential_state(cx: &App) -> Option<Entity<NautilusCredentialState>> {
    cx.try_global::<GlobalNautilusCredentialState>()
        .map(|state| state.0.clone())
}

fn now_ms() -> Result<i64> {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis();
    i64::try_from(milliseconds).context("system timestamp exceeds i64")
}

fn credential_halt_wakeup(
    network: Network,
    halt: &AgentWalletHaltReason,
) -> agent_wakeup::WakeupSource {
    agent_wakeup::WakeupSource::CredentialHalt {
        venue: "hyperliquid".to_owned(),
        network: network.label().to_ascii_lowercase(),
        reason_code: halt.code().to_owned(),
    }
}

pub fn command_channel(cx: &App) -> Option<NautilusCommandChannel> {
    cx.try_global::<NautilusLifecycle>()
        .map(|lifecycle| lifecycle.command_channel.clone())
}

pub fn init(cx: &mut App) {
    let credential_state = cx.new(|_| NautilusCredentialState {
        snapshot: NautilusCredentialSnapshot::default(),
    });
    cx.set_global(GlobalNautilusCredentialState(credential_state.clone()));
    let credentials = zed_credentials_provider::agent_wallet_credentials(cx);
    let config = match NautilusConfig::from_process_environment() {
        Ok(Some(config)) => config,
        Ok(None) => {
            cx.spawn(async move |cx| {
                let testnet = load_agent_wallet(&credentials, Network::Testnet, cx)
                    .await?
                    .map(|wallet| wallet.summary());
                let mainnet = load_agent_wallet(&credentials, Network::Mainnet, cx)
                    .await?
                    .map(|wallet| wallet.summary());
                credential_state.update(cx, |state, cx| {
                    state.replace(
                        NautilusCredentialSnapshot {
                            testnet,
                            mainnet,
                            loading: false,
                            ..NautilusCredentialSnapshot::default()
                        },
                        cx,
                    );
                });
                Ok::<(), anyhow::Error>(())
            })
            .detach_and_log_err(cx);
            return;
        }
        Err(error) => {
            log::error!("Refusing Nautilus sidecar configuration: {error:#}");
            credential_state.update(cx, |state, cx| {
                state.replace(
                    NautilusCredentialSnapshot {
                        error: Some(error.to_string()),
                        loading: false,
                        ..NautilusCredentialSnapshot::default()
                    },
                    cx,
                );
            });
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
        command_channel: NautilusCommandChannel {
            supervisor: supervisor.clone(),
        },
        _quit_subscription: quit_subscription,
    });
    let http_client = cx.http_client();
    let background_executor = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        let selected_network = config.network;
        let testnet_wallet = load_agent_wallet(&credentials, Network::Testnet, cx).await?;
        let mainnet_wallet = load_agent_wallet(&credentials, Network::Mainnet, cx).await?;
        let selected_configured = match selected_network {
            Network::Testnet => testnet_wallet.is_some(),
            Network::Mainnet => mainnet_wallet.is_some(),
        };
        if !selected_configured {
            credential_state.update(cx, |state, cx| {
                state.replace(
                    NautilusCredentialSnapshot {
                        selected_network,
                        testnet: None,
                        mainnet: None,
                        error: Some(format!(
                            "Hyperliquid {} agent wallet is not configured",
                            selected_network.label().to_ascii_lowercase()
                        )),
                        loading: false,
                        halt: None,
                        wakeup: None,
                    },
                    cx,
                );
            });
            return Ok::<(), anyhow::Error>(());
        }
        let refreshed = refresh_agent_wallet_approval(
            http_client,
            &credentials,
            selected_network,
            now_ms()?,
            cx,
        )
        .await?;
        let halt = refreshed.halt_reason(selected_network);
        credential_state.update(cx, |state, cx| {
            state.replace(
                NautilusCredentialSnapshot {
                    selected_network,
                    testnet: (selected_network == Network::Testnet)
                        .then_some(refreshed.clone())
                        .or_else(|| testnet_wallet.as_ref().map(StoredAgentWallet::summary)),
                    mainnet: (selected_network == Network::Mainnet)
                        .then_some(refreshed.clone())
                        .or_else(|| mainnet_wallet.as_ref().map(StoredAgentWallet::summary)),
                    wakeup: halt
                        .as_ref()
                        .map(|halt| credential_halt_wakeup(selected_network, halt)),
                    halt: halt.clone(),
                    error: None,
                    loading: false,
                },
                cx,
            );
        });
        if halt.is_some() {
            return Ok::<(), anyhow::Error>(());
        }
        let selected_wallet = load_agent_wallet(&credentials, selected_network, cx)
            .await?
            .ok_or_else(|| anyhow!("Hyperliquid agent wallet disappeared after approval probe"))?;
        let sidecar = NautilusSupervisor::with_agent_wallet(config, selected_wallet)?;
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

    fn measured_strategy_parameters() -> StrategyParameters {
        StrategyParameters {
            min_reprice_interval_ms: 500,
            quote_offset_bps: 100,
            order_quantity: "0.001".into(),
            position_headroom_usd: 100,
            order_budget: 6,
            mandate_revision: 1,
            cost_path: "maker_taker".into(),
            cost_clip_usd: 65,
            cost_sample_count: 5,
            measured_round_trip_cost_micros_bps: 6_000_000,
            cost_margin_bps: 3,
            admission_floor_bps: 9,
            cost_evidence_sha256: "a".repeat(64),
        }
    }

    #[test]
    fn strategy_parameters_bind_measured_cost_floor_and_margin() {
        let command = |parameters| NautilusCommand::SetStrategyParameters {
            strategy_id: "OMEGA-BOUNDED-QUOTE-001".into(),
            parameters,
        };
        assert!(command(measured_strategy_parameters()).validate().is_ok());

        let mut below_floor = measured_strategy_parameters();
        below_floor.quote_offset_bps = 8;
        assert!(command(below_floor).validate().is_err());

        let mut fee_schedule_substitute = measured_strategy_parameters();
        fee_schedule_substitute.measured_round_trip_cost_micros_bps = 0;
        assert!(command(fee_schedule_substitute).validate().is_err());

        let mut unbound = measured_strategy_parameters();
        unbound.cost_evidence_sha256 = "0".repeat(63);
        assert!(command(unbound).validate().is_err());

        let start = |cost_evidence_sha256| NautilusCommand::StartStrategy {
            strategy_id: "OMEGA-BOUNDED-QUOTE-001".into(),
            cost_evidence_sha256,
        };
        assert!(start("a".repeat(64)).validate().is_ok());
        assert!(start("a".repeat(63)).validate().is_err());
    }

    #[test]
    fn mainnet_is_hard_refused() {
        assert_eq!(
            Network::parse("mainnet").expect("named mainnet"),
            Network::Mainnet
        );
        assert!(Network::parse("testnet").is_ok());
        let config = NautilusConfig {
            network: Network::Mainnet,
            python: PathBuf::from("python"),
            engine: PathBuf::from("engine.py"),
            reconciliation_lookback_minutes: 60,
            health_timeout: Duration::from_secs(5),
        };
        assert!(NautilusSupervisor::new(config, private_key()).is_err());
    }

    #[test]
    fn bootstrap_always_binds_an_owner_address() {
        let key = private_key();
        let derived_owner_address = key.ethereum_address().expect("owner address");
        let config = NautilusConfig {
            network: Network::Testnet,
            python: PathBuf::from("python"),
            engine: PathBuf::from("engine.py"),
            reconciliation_lookback_minutes: 60,
            health_timeout: Duration::from_secs(5),
        };
        let supervisor = NautilusSupervisor::new(config.clone(), key).expect("owner supervisor");
        assert_eq!(
            supervisor
                .bootstrap_owner_address()
                .expect("bootstrap owner"),
            derived_owner_address
        );

        let approved_owner = "0x2222222222222222222222222222222222222222";
        let mut wallet =
            StoredAgentWallet::generate(Network::Testnet, approved_owner, 1).expect("agent wallet");
        wallet.approval = AgentApprovalStatus::Approved { valid_until_ms: 2 };
        let supervisor =
            NautilusSupervisor::with_agent_wallet(config, wallet).expect("agent supervisor");
        assert_eq!(
            supervisor
                .bootstrap_owner_address()
                .expect("agent bootstrap owner"),
            approved_owner
        );
    }

    #[test]
    fn expired_agent_wallet_emits_typed_credential_wakeup() {
        let source = credential_halt_wakeup(
            Network::Testnet,
            &AgentWalletHaltReason::Expired {
                valid_until_ms: 1_000,
            },
        );
        assert_eq!(
            source,
            agent_wakeup::WakeupSource::CredentialHalt {
                venue: "hyperliquid".to_owned(),
                network: "testnet".to_owned(),
                reason_code: "agent_wallet_expired".to_owned(),
            }
        );
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
        let mainnet = serde_json::from_value::<StreamEvent>(mainnet)
            .expect("mainnet remains a named network for strict mismatch diagnostics");
        assert!(mainnet.validate(3).is_err());
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

    #[test]
    fn strategy_state_is_versioned_and_lossless() {
        let event: StreamEvent = serde_json::from_value(serde_json::json!({
            "type": "strategy_state",
            "schema": STREAM_SCHEMA,
            "generation": 3,
            "sequence": 9,
            "network": "testnet",
            "strategy_id": "OMEGA-BOUNDED-QUOTE-001",
            "phase": "halted",
            "running": true,
            "halted_reason": "hourly_order_limit",
            "mandate_revision": 4,
            "quote_ticks": 81,
            "trade_ticks": 12,
            "book_ticks": 73,
            "action_count": 6,
            "active_client_order_id": null,
            "ts_init": 99,
        }))
        .expect("valid strategy state");
        assert!(event.validate(3).is_ok());
        let mut stream = StreamBuffer::default();
        stream.ingest(event);
        assert!(matches!(
            stream.take_frame().state.as_slice(),
            [StreamEvent::StrategyState {
                quote_ticks: 81,
                ..
            }]
        ));
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
                    state: vec![
                        serde_json::from_value(serde_json::json!({
                            "type": "account",
                            "schema": STREAM_SCHEMA,
                            "generation": 3,
                            "sequence": 6,
                            "network": "testnet",
                            "account_id": "HYPERLIQUID-001",
                            "balances": [],
                        }))
                        .expect("valid account event"),
                        order_state(7),
                        serde_json::from_value(serde_json::json!({
                            "type": "position_state",
                            "schema": STREAM_SCHEMA,
                            "generation": 3,
                            "sequence": 8,
                            "network": "testnet",
                            "venue": "HYPERLIQUID",
                            "positions": [{"instrument_id": "BTC-USD-PERP.HYPERLIQUID"}],
                            "ts_init": 8,
                        }))
                        .expect("valid position state"),
                    ],
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
            assert_eq!(source.counts(), (0, 3, 1));
            assert!(source.latest_quote().is_some());
            let snapshot = source.market_snapshot();
            assert!(snapshot.account.is_some());
            assert!(snapshot.orders.is_empty());
            assert_eq!(snapshot.positions.len(), 1);
        });
    }

    #[test]
    fn command_envelope_is_typed_versioned_and_testnet_only() {
        let request = CommandRequest {
            command_id: "place-1".into(),
            command: NautilusCommand::PlaceOrder {
                client_order_id: "O-OMEGA-287-1".into(),
                instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
                side: OrderSide::Buy,
                quantity: "0.001".into(),
                price: "60000.0".into(),
                time_in_force: OrderTimeInForce::Gtc,
                post_only: true,
                reduce_only: false,
            },
        };
        request.validate().expect("valid command");
        let encoded = serde_json::to_value(CommandEnvelope {
            schema: COMMAND_SCHEMA,
            generation: 3,
            network: Network::Testnet,
            command_id: &request.command_id,
            command: &request.command,
        })
        .expect("serialize command");
        assert_eq!(encoded["schema"], COMMAND_SCHEMA);
        assert_eq!(encoded["generation"], 3);
        assert_eq!(encoded["network"], "testnet");
        assert_eq!(encoded["type"], "place_order");
        assert_eq!(encoded["client_order_id"], "O-OMEGA-287-1");
        assert!(encoded.get("message").is_none());
    }

    #[test]
    fn post_dispatch_failures_are_never_refused() {
        for (refusal, unknown) in [
            (
                RefusalReason::RiskDenied,
                UnknownReason::PostDispatchRiskDenied,
            ),
            (
                RefusalReason::VenueRejected,
                UnknownReason::PostDispatchVenueRejected,
            ),
            (
                RefusalReason::CancelRejected,
                UnknownReason::PostDispatchCancelRejected,
            ),
        ] {
            assert_eq!(refusal.post_dispatch_unknown(), Some(unknown));
        }
        assert_eq!(RefusalReason::InvalidOrder.post_dispatch_unknown(), None);
        assert_eq!(RefusalReason::OrderNotFound.post_dispatch_unknown(), None);
    }

    #[cfg(unix)]
    #[test]
    fn commands_cross_one_stdio_channel_with_typed_outcomes() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let script = temporary_directory.path().join("fake-command-engine.sh");
        fs::write(
            &script,
            r#"#!/bin/sh
generation=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--generation" ]; then generation="$2"; shift 2; else shift; fi
done
printf 'OMEGA_NAUTILUS_EVENT {"type":"healthy","schema":"omega.nautilus.lifecycle.v1","generation":%s,"network":"testnet","venue":"hyperliquid","reconciliation_lookback_minutes":60}\n' "$generation"
trap 'exit 0' TERM
while IFS= read -r command; do
  case "$command" in
    *'"command_id":"place-1"'*)
      printf 'OMEGA_NAUTILUS_EVENT {"type":"acknowledged","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"place-1","command_type":"place_order"}\n' "$generation"
      printf 'OMEGA_NAUTILUS_EVENT {"type":"sent","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"place-1","command_type":"place_order","client_order_id":"O-OMEGA-287-1","mutation_state":"sent"}\n' "$generation"
      printf 'OMEGA_NAUTILUS_EVENT {"type":"order_accepted","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"place-1","command_type":"place_order","client_order_id":"O-OMEGA-287-1","venue_order_id":"venue-1"}\n' "$generation"
      ;;
    *'"command_id":"cancel-1"'*)
      printf 'OMEGA_NAUTILUS_EVENT {"type":"acknowledged","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"cancel-1","command_type":"cancel_order"}\n' "$generation"
      printf 'OMEGA_NAUTILUS_EVENT {"type":"sent","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"cancel-1","command_type":"cancel_order","client_order_id":"O-OMEGA-287-1","mutation_state":"sent"}\n' "$generation"
      printf 'OMEGA_NAUTILUS_EVENT {"type":"order_canceled","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"cancel-1","command_type":"cancel_order","client_order_id":"O-OMEGA-287-1","venue_order_id":"venue-1"}\n' "$generation"
      ;;
    *'"command_id":"legacy-risk-1"'*)
      printf 'OMEGA_NAUTILUS_EVENT {"type":"acknowledged","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"legacy-risk-1","command_type":"place_order"}\n' "$generation"
      printf 'OMEGA_NAUTILUS_EVENT {"type":"refused","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"legacy-risk-1","command_type":"place_order","reason_code":"risk_denied"}\n' "$generation"
      ;;
    *'"command_id":"post-dispatch-risk-1"'*)
      printf 'OMEGA_NAUTILUS_EVENT {"type":"acknowledged","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"post-dispatch-risk-1","command_type":"place_order"}\n' "$generation"
      printf 'OMEGA_NAUTILUS_EVENT {"type":"sent","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"post-dispatch-risk-1","command_type":"place_order","client_order_id":"O-OMEGA-287-RISK-2","mutation_state":"sent"}\n' "$generation"
      printf 'OMEGA_NAUTILUS_EVENT {"type":"unknown","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"post-dispatch-risk-1","command_type":"place_order","client_order_id":"O-OMEGA-287-RISK-2","mutation_state":"unknown","reason_code":"post_dispatch_risk_denied"}\n' "$generation"
      ;;
    *'"command_id":"invalid-1"'*)
      printf 'OMEGA_NAUTILUS_EVENT {"type":"refused","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"invalid-1","command_type":"place_order","reason_code":"invalid_order"}\n' "$generation"
      ;;
    *'"command_id":"params-1"'*)
      printf 'OMEGA_NAUTILUS_EVENT {"type":"acknowledged","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"params-1","command_type":"set_strategy_parameters"}\n' "$generation"
      printf 'OMEGA_NAUTILUS_EVENT {"type":"strategy_parameters_applied","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"params-1","command_type":"set_strategy_parameters","parameters":{"min_reprice_interval_ms":500,"quote_offset_bps":100,"order_quantity":"0.001","position_headroom_usd":100,"order_budget":6,"mandate_revision":1,"cost_path":"maker_taker","cost_clip_usd":65,"cost_sample_count":5,"measured_round_trip_cost_micros_bps":6000000,"cost_margin_bps":3,"admission_floor_bps":9,"cost_evidence_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}\n' "$generation"
      ;;
    *'"command_id":"start-1"'*)
      printf 'OMEGA_NAUTILUS_EVENT {"type":"acknowledged","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"start-1","command_type":"start_strategy"}\n' "$generation"
      printf 'OMEGA_NAUTILUS_EVENT {"type":"strategy_started","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"start-1","command_type":"start_strategy","running":true}\n' "$generation"
      ;;
    *'"command_id":"stop-1"'*)
      printf 'OMEGA_NAUTILUS_EVENT {"type":"acknowledged","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"stop-1","command_type":"stop_strategy"}\n' "$generation"
      printf 'OMEGA_NAUTILUS_EVENT {"type":"strategy_stopped","schema":"omega.nautilus.command.v1","generation":%s,"network":"testnet","command_id":"stop-1","command_type":"stop_strategy","running":false}\n' "$generation"
      ;;
  esac
done
"#,
        )
        .expect("write fake command engine");
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700))
            .expect("make fake engine executable");
        let config = NautilusConfig {
            network: Network::Testnet,
            python: script,
            engine: PathBuf::from("ignored"),
            reconciliation_lookback_minutes: 60,
            health_timeout: Duration::from_secs(5),
        };
        let mut supervisor = NautilusSupervisor::new(config, private_key()).expect("supervisor");
        supervisor.start().expect("start fake engine");

        let place = supervisor
            .send_command(CommandRequest {
                command_id: "place-1".into(),
                command: NautilusCommand::PlaceOrder {
                    client_order_id: "O-OMEGA-287-1".into(),
                    instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
                    side: OrderSide::Buy,
                    quantity: "0.001".into(),
                    price: "60000".into(),
                    time_in_force: OrderTimeInForce::Gtc,
                    post_only: true,
                    reduce_only: false,
                },
            })
            .expect("place command");
        assert!(place.acknowledged);
        assert!(place.sent);
        assert!(matches!(
            place.outcome,
            CommandOutcome::OrderAccepted { .. }
        ));

        let cancel = supervisor
            .send_command(CommandRequest {
                command_id: "cancel-1".into(),
                command: NautilusCommand::CancelOrder {
                    client_order_id: "O-OMEGA-287-1".into(),
                },
            })
            .expect("cancel command");
        assert!(cancel.acknowledged);
        assert!(cancel.sent);
        assert!(matches!(
            cancel.outcome,
            CommandOutcome::OrderCanceled { .. }
        ));

        let legacy_risk = supervisor
            .send_command(CommandRequest {
                command_id: "legacy-risk-1".into(),
                command: NautilusCommand::PlaceOrder {
                    client_order_id: "O-OMEGA-287-RISK-1".into(),
                    instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
                    side: OrderSide::Buy,
                    quantity: "0.001".into(),
                    price: "60000".into(),
                    time_in_force: OrderTimeInForce::Ioc,
                    post_only: false,
                    reduce_only: false,
                },
            })
            .expect("legacy risk command");
        assert!(legacy_risk.acknowledged);
        assert!(!legacy_risk.sent);
        assert!(matches!(
            legacy_risk.outcome,
            CommandOutcome::Unknown {
                reason_code: UnknownReason::PostDispatchRiskDenied
            }
        ));

        let post_dispatch_risk = supervisor
            .send_command(CommandRequest {
                command_id: "post-dispatch-risk-1".into(),
                command: NautilusCommand::PlaceOrder {
                    client_order_id: "O-OMEGA-287-RISK-2".into(),
                    instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
                    side: OrderSide::Buy,
                    quantity: "0.001".into(),
                    price: "60000".into(),
                    time_in_force: OrderTimeInForce::Ioc,
                    post_only: false,
                    reduce_only: false,
                },
            })
            .expect("post-dispatch risk command");
        assert!(post_dispatch_risk.acknowledged);
        assert!(post_dispatch_risk.sent);
        assert!(matches!(
            post_dispatch_risk.outcome,
            CommandOutcome::Unknown {
                reason_code: UnknownReason::PostDispatchRiskDenied
            }
        ));

        let invalid = supervisor
            .send_command(CommandRequest {
                command_id: "invalid-1".into(),
                command: NautilusCommand::PlaceOrder {
                    client_order_id: "O-OMEGA-287-INVALID".into(),
                    instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
                    side: OrderSide::Buy,
                    quantity: "0.001".into(),
                    price: "60000".into(),
                    time_in_force: OrderTimeInForce::Gtc,
                    post_only: true,
                    reduce_only: false,
                },
            })
            .expect("invalid command");
        assert!(!invalid.acknowledged);
        assert!(!invalid.sent);
        assert!(matches!(
            invalid.outcome,
            CommandOutcome::Refused {
                reason_code: RefusalReason::InvalidOrder
            }
        ));

        let parameters = supervisor
            .send_command(CommandRequest {
                command_id: "params-1".into(),
                command: NautilusCommand::SetStrategyParameters {
                    strategy_id: "OMEGA-BOUNDED-QUOTE-001".into(),
                    parameters: measured_strategy_parameters(),
                },
            })
            .expect("parameter command");
        assert!(matches!(
            parameters.outcome,
            CommandOutcome::StrategyParametersApplied { .. }
        ));

        let start = supervisor
            .send_command(CommandRequest {
                command_id: "start-1".into(),
                command: NautilusCommand::StartStrategy {
                    strategy_id: "OMEGA-BOUNDED-QUOTE-001".into(),
                    cost_evidence_sha256: "a".repeat(64),
                },
            })
            .expect("start command");
        assert!(matches!(
            start.outcome,
            CommandOutcome::StrategyStarted { running: true }
        ));

        let stop = supervisor
            .send_command(CommandRequest {
                command_id: "stop-1".into(),
                command: NautilusCommand::StopStrategy {
                    strategy_id: "OMEGA-BOUNDED-QUOTE-001".into(),
                },
            })
            .expect("stop command");
        assert!(matches!(
            stop.outcome,
            CommandOutcome::StrategyStopped { running: false }
        ));
        supervisor.stop().expect("stop fake engine");
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
            health_timeout: Duration::from_secs(5),
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
