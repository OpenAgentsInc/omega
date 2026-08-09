use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use agent_client_protocol::schema::v1 as acp;
use credentials_provider::CredentialsProvider;
use gpui::{App, AsyncApp, Task};
use http_client::HttpClient;
use language_model::LanguageModelToolResultContent;
use plugin_api::{
    VenueActionClass, VenueCapabilityError, VenueCapabilityGuard, VenueCapabilityStore,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ui::SharedString;

use agent::{AgentTool, ToolCallEventStream, ToolInput};

use crate::{
    AccountHistoryQuery, CREDENTIAL_STORAGE_URL, CandleResolution, CandlesQuery, Credentials,
    LightningDepositsQuery, LnMarketsClient, LnMarketsStreamClient, Network, NewSwapRequest,
    NotificationsQuery, Pagination, StoredCredentials, StreamTopic, http_transport,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LnMarketsToolOutput(Value);

impl LnMarketsToolOutput {
    fn success(value: Value) -> Self {
        Self(value)
    }

    fn error(message: impl Into<String>) -> Self {
        Self(json!({ "error": message.into() }))
    }

    fn capability_refusal(error: VenueCapabilityError) -> Self {
        Self(json!({
            "error": error.to_string(),
            "error_type": "venue_capability_refusal",
            "refusal": error,
        }))
    }
}

impl From<LnMarketsToolOutput> for LanguageModelToolResultContent {
    fn from(output: LnMarketsToolOutput) -> Self {
        serde_json::to_string_pretty(&output.0)
            .unwrap_or_else(|error| format!("Failed to serialize LN Markets output: {error}"))
            .into()
    }
}

#[derive(Clone)]
struct ToolClient {
    http_client: Arc<dyn HttpClient>,
    credentials_provider: Arc<dyn CredentialsProvider>,
}

impl ToolClient {
    async fn credentials(&self, cx: &AsyncApp) -> Result<(Credentials, Network), String> {
        let (_username, encoded) = self
            .credentials_provider
            .read_credentials(CREDENTIAL_STORAGE_URL, cx)
            .await
            .map_err(|error| format!("could not read LN Markets credentials: {error}"))?
            .ok_or_else(|| {
                "LN Markets is not connected. Add credentials in Settings > API Keys > LN Markets."
                    .to_string()
            })?;
        let stored = StoredCredentials::decode(&encoded).map_err(|error| error.to_string())?;
        let network = stored.network;
        let credentials = stored.credentials().map_err(|error| error.to_string())?;
        Ok((credentials, network))
    }

    async fn authenticated(&self, cx: &AsyncApp) -> Result<(LnMarketsClient, Network), String> {
        let (credentials, network) = self.credentials(cx).await?;
        Ok((
            LnMarketsClient::authenticated(
                http_transport(self.http_client.clone()),
                network,
                credentials,
            ),
            network,
        ))
    }
}

pub struct LnMarketsAccountTool {
    client: ToolClient,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LnMarketsAccountInput {}

impl AgentTool for LnMarketsAccountTool {
    type Input = LnMarketsAccountInput;
    type Output = LnMarketsToolOutput;

    const NAME: &'static str = "lnmarkets_account";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(&self, _input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        "Check LN Markets account".into()
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            input
                .recv()
                .await
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            let (client, network) = self
                .client
                .authenticated(cx)
                .await
                .map_err(LnMarketsToolOutput::error)?;
            let account = client
                .account()
                .await
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            Ok(LnMarketsToolOutput::success(json!({
                "schema": "omega.lnmarkets.account.v1",
                "network": network,
                "account": account,
            })))
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LnMarketsNetworkInput {
    Signet,
    Mainnet,
}

impl From<LnMarketsNetworkInput> for Network {
    fn from(network: LnMarketsNetworkInput) -> Self {
        match network {
            LnMarketsNetworkInput::Signet => Self::Signet,
            LnMarketsNetworkInput::Mainnet => Self::Mainnet,
        }
    }
}

pub struct LnMarketsMarketDataTool {
    client: ToolClient,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LnMarketsMarketDataInput {
    /// LN Markets environment to query.
    network: LnMarketsNetworkInput,
    /// Data view to retrieve. Omit it for the current ticker and synthetic USD quote.
    #[serde(default)]
    request: LnMarketsMarketDataRequest,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LnMarketsMarketDataRequest {
    /// Current ticker, funding, liquidity price tiers, and synthetic USD quote.
    #[default]
    Snapshot,
    /// Historical OHLCV candles and funding settlements.
    History {
        /// Inclusive ISO 8601 start time.
        from: String,
        /// Optional inclusive ISO 8601 end time.
        #[serde(default)]
        to: Option<String>,
        /// Candle resolution.
        resolution: LnMarketsCandleResolution,
        /// Items per result set, from 1 through 1000.
        #[serde(default = "default_history_limit")]
        limit: u16,
        /// Cursor returned by a prior request.
        #[serde(default)]
        cursor: Option<String>,
    },
    /// Account, risk, orders, trades, fees, transfers, and swap history.
    Portfolio {
        /// History items per section, from 1 through 100.
        #[serde(default = "default_portfolio_limit")]
        limit: u16,
    },
    /// A bounded WebSocket snapshot. Public topics need no key; private topics use the configured key.
    Live {
        /// Exact LN Markets Stream topic names.
        topics: Vec<String>,
        /// Stop after this many events, from 1 through 100.
        #[serde(default = "default_live_event_limit")]
        max_events: u16,
        /// Stop waiting after this many seconds, from 1 through 30.
        #[serde(default = "default_live_timeout_seconds")]
        timeout_seconds: u16,
    },
}

impl LnMarketsMarketDataRequest {
    fn label(&self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::History { .. } => "history",
            Self::Portfolio { .. } => "portfolio",
            Self::Live { .. } => "live stream",
        }
    }
}

fn default_history_limit() -> u16 {
    100
}

fn default_portfolio_limit() -> u16 {
    50
}

fn default_live_event_limit() -> u16 {
    12
}

fn default_live_timeout_seconds() -> u16 {
    5
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum LnMarketsCandleResolution {
    #[serde(rename = "1m")]
    OneMinute,
    #[serde(rename = "3m")]
    ThreeMinutes,
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "10m")]
    TenMinutes,
    #[serde(rename = "15m")]
    FifteenMinutes,
    #[serde(rename = "30m")]
    ThirtyMinutes,
    #[serde(rename = "45m")]
    FortyFiveMinutes,
    #[serde(rename = "1h")]
    OneHour,
    #[serde(rename = "2h")]
    TwoHours,
    #[serde(rename = "3h")]
    ThreeHours,
    #[serde(rename = "4h")]
    FourHours,
    #[serde(rename = "1d")]
    OneDay,
    #[serde(rename = "1w")]
    OneWeek,
    #[serde(rename = "1month")]
    OneMonth,
    #[serde(rename = "3months")]
    ThreeMonths,
}

impl From<LnMarketsCandleResolution> for CandleResolution {
    fn from(resolution: LnMarketsCandleResolution) -> Self {
        match resolution {
            LnMarketsCandleResolution::OneMinute => Self::OneMinute,
            LnMarketsCandleResolution::ThreeMinutes => Self::ThreeMinutes,
            LnMarketsCandleResolution::FiveMinutes => Self::FiveMinutes,
            LnMarketsCandleResolution::TenMinutes => Self::TenMinutes,
            LnMarketsCandleResolution::FifteenMinutes => Self::FifteenMinutes,
            LnMarketsCandleResolution::ThirtyMinutes => Self::ThirtyMinutes,
            LnMarketsCandleResolution::FortyFiveMinutes => Self::FortyFiveMinutes,
            LnMarketsCandleResolution::OneHour => Self::OneHour,
            LnMarketsCandleResolution::TwoHours => Self::TwoHours,
            LnMarketsCandleResolution::ThreeHours => Self::ThreeHours,
            LnMarketsCandleResolution::FourHours => Self::FourHours,
            LnMarketsCandleResolution::OneDay => Self::OneDay,
            LnMarketsCandleResolution::OneWeek => Self::OneWeek,
            LnMarketsCandleResolution::OneMonth => Self::OneMonth,
            LnMarketsCandleResolution::ThreeMonths => Self::ThreeMonths,
        }
    }
}

impl AgentTool for LnMarketsMarketDataTool {
    type Input = LnMarketsMarketDataInput;
    type Output = LnMarketsToolOutput;

    const NAME: &'static str = "lnmarkets_market_data";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(&self, input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        match input {
            Ok(input) => format!(
                "Read LN Markets {} {}",
                Network::from(input.network).label(),
                input.request.label()
            )
            .into(),
            Err(_) => "Read LN Markets trading data".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let collector = crate::collector(cx);
        cx.spawn(async move |cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            let network = Network::from(input.network);
            match input.request {
                LnMarketsMarketDataRequest::Snapshot => {
                    let client = LnMarketsClient::public(
                        http_transport(self.client.http_client.clone()),
                        network,
                    );
                    let ping = client
                        .ping()
                        .await
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let server_time = client
                        .server_time()
                        .await
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let ticker = client
                        .ticker()
                        .await
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let best_price = client
                        .best_price()
                        .await
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let leaderboard = client
                        .leaderboard()
                        .await
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    Ok(LnMarketsToolOutput::success(json!({
                        "schema": "omega.lnmarkets.market-data.v2",
                        "view": "snapshot",
                        "network": network,
                        "server": {
                            "status": ping,
                            "time": server_time.time,
                        },
                        "ticker": ticker,
                        "synthetic_usd_best_price": best_price,
                        "leaderboard": leaderboard,
                    })))
                }
                LnMarketsMarketDataRequest::History {
                    from,
                    to,
                    resolution,
                    limit,
                    cursor,
                } => {
                    require_limit("history", limit, 1, 1_000)
                        .map_err(LnMarketsToolOutput::error)?;
                    if resolution == LnMarketsCandleResolution::OneHour
                        && let Some(collector) = collector
                        && collector.health().network == network
                    {
                        let history = collector
                            .history(&from, to.as_deref(), usize::from(limit))
                            .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                        return Ok(LnMarketsToolOutput::success(json!({
                            "schema": "omega.lnmarkets.market-data.v3",
                            "view": "history",
                            "source": "local_collector",
                            "network": network,
                            "collector": collector.health(),
                            "history": history,
                        })));
                    }
                    let client = LnMarketsClient::public(
                        http_transport(self.client.http_client.clone()),
                        network,
                    );
                    let pagination = Pagination {
                        cursor: cursor.clone(),
                        from_: Some(from.clone()),
                        limit: Some(limit),
                        to: to.clone(),
                    };
                    let candles = client
                        .candles(&CandlesQuery {
                            from_: from,
                            to,
                            limit: Some(limit),
                            cursor,
                            resolution: resolution.into(),
                        })
                        .await
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let funding = client
                        .funding_settlements(&pagination)
                        .await
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    Ok(LnMarketsToolOutput::success(json!({
                        "schema": "omega.lnmarkets.market-data.v2",
                        "view": "history",
                        "network": network,
                        "candles": candles,
                        "funding_settlements": funding,
                    })))
                }
                LnMarketsMarketDataRequest::Portfolio { limit } => {
                    require_limit("portfolio", limit, 1, 100)
                        .map_err(LnMarketsToolOutput::error)?;
                    let (client, configured_network) = self
                        .client
                        .authenticated(cx)
                        .await
                        .map_err(LnMarketsToolOutput::error)?;
                    require_matching_network(configured_network, network, "portfolio")
                        .map_err(LnMarketsToolOutput::error)?;
                    let pagination = Pagination {
                        limit: Some(limit),
                        ..Pagination::default()
                    };
                    let account = tool_section(client.account().await);
                    let bitcoin_address = tool_section(client.bitcoin_address().await);
                    let lightning_deposits = tool_section(
                        client
                            .lightning_deposits(&LightningDepositsQuery {
                                pagination: pagination.clone(),
                                settled: None,
                            })
                            .await,
                    );
                    let lightning_withdrawals = tool_section(
                        client
                            .lightning_withdrawals(&AccountHistoryQuery {
                                pagination: pagination.clone(),
                                status: None,
                            })
                            .await,
                    );
                    let on_chain_deposits = tool_section(
                        client
                            .on_chain_deposits(&AccountHistoryQuery {
                                pagination: pagination.clone(),
                                status: None,
                            })
                            .await,
                    );
                    let on_chain_withdrawals = tool_section(
                        client
                            .on_chain_withdrawals(&AccountHistoryQuery {
                                pagination: pagination.clone(),
                                status: None,
                            })
                            .await,
                    );
                    let notifications = tool_section(
                        client
                            .notifications(&NotificationsQuery {
                                pagination: pagination.clone(),
                                read: None,
                            })
                            .await,
                    );
                    let cross_position = tool_section(client.cross_position().await);
                    let cross_open_orders = tool_section(client.cross_open_orders().await);
                    let cross_filled_orders =
                        tool_section(client.cross_filled_orders(&pagination).await);
                    let cross_funding_fees =
                        tool_section(client.cross_funding_fees(&pagination).await);
                    let cross_transfers = tool_section(client.cross_transfers(&pagination).await);
                    let isolated_open_trades = tool_section(client.isolated_open_trades().await);
                    let isolated_running_trades =
                        tool_section(client.isolated_running_trades().await);
                    let isolated_closed_trades =
                        tool_section(client.isolated_closed_trades(&pagination).await);
                    let isolated_canceled_trades =
                        tool_section(client.isolated_canceled_trades(&pagination).await);
                    let isolated_funding_fees =
                        tool_section(client.isolated_funding_fees(&pagination, None).await);
                    let swaps = tool_section(client.swaps(&pagination).await);
                    Ok(LnMarketsToolOutput::success(json!({
                        "schema": "omega.lnmarkets.market-data.v2",
                        "view": "portfolio",
                        "network": configured_network,
                        "account": account,
                        "wallet": {
                            "bitcoin_address": bitcoin_address,
                            "lightning_deposits": lightning_deposits,
                            "lightning_withdrawals": lightning_withdrawals,
                            "on_chain_deposits": on_chain_deposits,
                            "on_chain_withdrawals": on_chain_withdrawals,
                        },
                        "notifications": notifications,
                        "cross": {
                            "position": cross_position,
                            "open_orders": cross_open_orders,
                            "filled_orders": cross_filled_orders,
                            "funding_fees": cross_funding_fees,
                            "transfers": cross_transfers,
                        },
                        "isolated": {
                            "open_trades": isolated_open_trades,
                            "running_trades": isolated_running_trades,
                            "closed_trades": isolated_closed_trades,
                            "canceled_trades": isolated_canceled_trades,
                            "funding_fees": isolated_funding_fees,
                        },
                        "synthetic_usd_swaps": swaps,
                    })))
                }
                LnMarketsMarketDataRequest::Live {
                    topics,
                    max_events,
                    timeout_seconds,
                } => {
                    require_limit("live events", max_events, 1, 100)
                        .map_err(LnMarketsToolOutput::error)?;
                    require_limit("live timeout seconds", timeout_seconds, 1, 30)
                        .map_err(LnMarketsToolOutput::error)?;
                    let topics = topics
                        .into_iter()
                        .map(StreamTopic::new)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let private = topics.iter().any(StreamTopic::is_private);
                    let mut stream = LnMarketsStreamClient::connect(network)
                        .await
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let hello = stream
                        .hello("omega", env!("CARGO_PKG_VERSION"))
                        .await
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let server_time = stream
                        .server_time()
                        .await
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let authentication = if private {
                        let (credentials, configured_network) =
                            self.client
                                .credentials(cx)
                                .await
                                .map_err(LnMarketsToolOutput::error)?;
                        require_matching_network(configured_network, network, "stream")
                            .map_err(LnMarketsToolOutput::error)?;
                        Some(
                            stream
                                .authenticate(&credentials)
                                .await
                                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?,
                        )
                    } else {
                        None
                    };
                    let subscription = stream
                        .subscribe(&topics)
                        .await
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let events = stream
                        .collect_events(
                            usize::from(max_events),
                            Duration::from_secs(u64::from(timeout_seconds)),
                        )
                        .await
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let timed_out = events.len() < usize::from(max_events);
                    stream
                        .close()
                        .await
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    Ok(LnMarketsToolOutput::success(json!({
                        "schema": "omega.lnmarkets.market-data.v2",
                        "view": "live",
                        "network": network,
                        "stream": {
                            "version": hello.version,
                            "server_time_ms": server_time.time,
                        },
                        "authentication": authentication,
                        "subscription": subscription,
                        "events": events,
                        "timed_out": timed_out,
                    })))
                }
            }
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub enum LnMarketsSwapAsset {
    BTC,
    USD,
}

pub struct LnMarketsSwapTool {
    client: ToolClient,
    capability_guard: VenueCapabilityGuard,
}

pub struct LnMarketsFeaturesTool {
    capability_guard: VenueCapabilityGuard,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LnMarketsFeaturesInput {}

impl AgentTool for LnMarketsFeaturesTool {
    type Input = LnMarketsFeaturesInput;
    type Output = LnMarketsToolOutput;

    const NAME: &'static str = "lnmarkets_features";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(&self, _input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        "Read LN Markets derived features".into()
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let collector = crate::collector(cx);
        cx.spawn(async move |_cx| {
            input
                .recv()
                .await
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            let collector = collector.ok_or_else(|| {
                LnMarketsToolOutput::error("LN Markets feature collection has not started")
            })?;
            let health = collector.health();
            let checked_at_ms = unix_timestamp_ms().unwrap_or(i64::MAX);
            let capability_report = self.capability_guard.report(checked_at_ms);
            let features = collector
                .features()
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            let status = if features.is_some() {
                "ready"
            } else if health.last_error.is_some() {
                "degraded"
            } else {
                "collecting"
            };
            Ok(LnMarketsToolOutput::success(json!({
                "schema": "omega.lnmarkets.features.v1",
                "status": status,
                "collector": health,
                "venue_capabilities": capability_report,
                "features": features,
            })))
        })
    }
}

pub struct LnMarketsLedgerTool;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LnMarketsLedgerInput {
    /// Inclusive start timestamp in Unix milliseconds.
    #[serde(default)]
    from_ms: Option<i64>,
    /// Inclusive end timestamp in Unix milliseconds.
    #[serde(default)]
    to_ms: Option<i64>,
    /// Limit attribution to one strategy ID.
    #[serde(default)]
    strategy_id: Option<String>,
    /// Maximum ledger entries to include, from 0 through 100.
    #[serde(default = "default_ledger_entry_limit")]
    entry_limit: u16,
}

fn default_ledger_entry_limit() -> u16 {
    25
}

impl AgentTool for LnMarketsLedgerTool {
    type Input = LnMarketsLedgerInput;
    type Output = LnMarketsToolOutput;

    const NAME: &'static str = "lnmarkets_ledger";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(&self, _input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        "Read LN Markets trading ledger".into()
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let runtime = crate::trading_runtime(cx);
        cx.spawn(async move |_cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            require_limit("ledger entry count", input.entry_limit, 0, 100)
                .map_err(LnMarketsToolOutput::error)?;
            let runtime = runtime.map_err(LnMarketsToolOutput::error)?;
            let query = crate::LedgerQuery {
                from_ms: input.from_ms,
                to_ms: input.to_ms,
                strategy_id: input.strategy_id,
            };
            let report = runtime
                .profit_report(&query)
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            let mut entries = runtime
                .ledger_entries(&query)
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            let total_entry_count = entries.len();
            entries.reverse();
            entries.truncate(usize::from(input.entry_limit));
            Ok(LnMarketsToolOutput::success(json!({
                "schema": "omega.lnmarkets.ledger.v1",
                "status": if total_entry_count == 0 { "empty" } else { "ready" },
                "report": report,
                "total_entry_count": total_entry_count,
                "entries": entries,
            })))
        })
    }
}

pub struct LnMarketsMandateTool;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LnMarketsMandateInput {
    /// Include the append-only revision history.
    #[serde(default)]
    include_history: bool,
}

impl AgentTool for LnMarketsMandateTool {
    type Input = LnMarketsMandateInput;
    type Output = LnMarketsToolOutput;

    const NAME: &'static str = "lnmarkets_mandate";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(&self, _input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        "Read LN Markets trading mandate".into()
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let runtime = crate::trading_runtime(cx);
        cx.spawn(async move |_cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            let runtime = runtime.map_err(LnMarketsToolOutput::error)?;
            let snapshot = runtime
                .mandate_snapshot()
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            let now_ms = unix_timestamp_ms().map_err(LnMarketsToolOutput::error)?;
            let status = match snapshot.mandates.first() {
                None => "missing",
                Some(mandate) if mandate.expires_at_ms <= now_ms => "expired",
                Some(_) => "active",
            };
            let history = if input.include_history {
                Some(
                    runtime
                        .mandate_history()
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?,
                )
            } else {
                None
            };
            Ok(LnMarketsToolOutput::success(json!({
                "schema": "omega.lnmarkets.mandate.v1",
                "status": status,
                "read_only": true,
                "snapshot": snapshot,
                "history": history,
            })))
        })
    }
}

pub struct LnMarketsStrategyTool {
    client: ToolClient,
    session_id: String,
    capability_guard: VenueCapabilityGuard,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LnMarketsStrategyName {
    RebalanceToTarget,
    FundingCarry,
    ThresholdSwing,
}

impl LnMarketsStrategyName {
    fn label(self) -> &'static str {
        match self {
            Self::RebalanceToTarget => "rebalance_to_target",
            Self::FundingCarry => "funding_carry",
            Self::ThresholdSwing => "threshold_swing",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum LnMarketsStrategyInput {
    /// Read both strategy lifecycle states.
    Status,
    /// Replay an exact strategy configuration against collected signet data and store its report.
    Backtest {
        strategy: LnMarketsStrategyName,
        /// Strategy-specific configuration. Its measured round-trip cost must match cost_model.
        config: Value,
        /// Inclusive replay start in Unix milliseconds.
        from_ms: i64,
        /// Inclusive replay end in Unix milliseconds.
        to_ms: i64,
        cost_model: LnMarketsBacktestCostModelInput,
        policy: LnMarketsBacktestPolicyInput,
    },
    /// Read durable backtest reports for the review turn or operator.
    BacktestReports {
        #[serde(default)]
        strategy: Option<LnMarketsStrategyName>,
        /// Number of newest reports, from 1 through 100.
        #[serde(default = "default_backtest_report_limit")]
        limit: u16,
    },
    /// Start a strategy after its stored backtest and mandate gates pass.
    Start {
        strategy: LnMarketsStrategyName,
        /// Strategy-specific configuration.
        config: Value,
    },
    /// Replace the configuration of a running strategy after its backtest gate passes.
    Adjust {
        strategy: LnMarketsStrategyName,
        /// Strategy-specific configuration.
        config: Value,
    },
    /// Halt a strategy immediately.
    Halt {
        strategy: LnMarketsStrategyName,
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LnMarketsBacktestCostModelInput {
    /// Measured venue taker fee in basis points.
    taker_fee_bps: u32,
    /// Measured round-trip spread and slippage cost in basis points.
    observed_round_trip_cost_bps: u32,
    /// Local measurement source, such as a trading-ledger range.
    measurement_source: String,
    /// Measurement time in Unix milliseconds.
    measured_at_ms: i64,
}

impl From<LnMarketsBacktestCostModelInput> for crate::BacktestCostModel {
    fn from(model: LnMarketsBacktestCostModelInput) -> Self {
        Self {
            taker_fee_bps: model.taker_fee_bps,
            observed_round_trip_cost_bps: model.observed_round_trip_cost_bps,
            measurement_source: model.measurement_source,
            measured_at_ms: model.measured_at_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LnMarketsBacktestPolicyInput {
    minimum_trade_count: u64,
    minimum_expectancy_millisats: i64,
    maximum_drawdown_sats: u64,
}

impl From<LnMarketsBacktestPolicyInput> for crate::BacktestPolicy {
    fn from(policy: LnMarketsBacktestPolicyInput) -> Self {
        Self {
            minimum_trade_count: policy.minimum_trade_count,
            minimum_expectancy_millisats: policy.minimum_expectancy_millisats,
            maximum_drawdown_sats: policy.maximum_drawdown_sats,
        }
    }
}

fn default_backtest_report_limit() -> u16 {
    20
}

impl AgentTool for LnMarketsStrategyTool {
    type Input = LnMarketsStrategyInput;
    type Output = LnMarketsToolOutput;

    const NAME: &'static str = "lnmarkets_strategy";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }

    fn initial_title(&self, input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        match input {
            Ok(LnMarketsStrategyInput::Status) => "Read LN Markets strategies".into(),
            Ok(LnMarketsStrategyInput::Backtest { strategy, .. }) => {
                format!("Backtest LN Markets {}", strategy.label()).into()
            }
            Ok(LnMarketsStrategyInput::BacktestReports { .. }) => {
                "Read LN Markets backtests".into()
            }
            Ok(LnMarketsStrategyInput::Start { strategy, .. }) => {
                format!("Start LN Markets {}", strategy.label()).into()
            }
            Ok(LnMarketsStrategyInput::Adjust { strategy, .. }) => {
                format!("Adjust LN Markets {}", strategy.label()).into()
            }
            Ok(LnMarketsStrategyInput::Halt { strategy, .. }) => {
                format!("Halt LN Markets {}", strategy.label()).into()
            }
            Err(_) => "Control LN Markets strategy".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let runtime = crate::trading_runtime(cx);
        let collector = crate::collector(cx);
        cx.spawn(async move |cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            let runtime = runtime.map_err(LnMarketsToolOutput::error)?;
            let (action, strategy) = strategy_action_labels(&input);
            let checked_at_ms = unix_timestamp_ms().unwrap_or(i64::MAX);
            let capability_report = self.capability_guard.report(checked_at_ms);
            if matches!(
                &input,
                LnMarketsStrategyInput::Start { .. } | LnMarketsStrategyInput::Adjust { .. }
            ) {
                self.capability_guard
                    .require_effectful(checked_at_ms)
                    .map_err(LnMarketsToolOutput::capability_refusal)?;
            }
            emit_lnmarkets_strategy_update(
                &event_stream,
                &json!({
                    "schema": "omega.lnmarkets.strategy.v1",
                    "status": pending_strategy_status(action),
                    "phase": "in_progress",
                    "action": action,
                    "strategy": strategy,
                    "venue_capabilities": capability_report,
                    "strategies": runtime.strategy_snapshots(),
                }),
            );
            let result = match input {
                LnMarketsStrategyInput::Status => Ok(()),
                LnMarketsStrategyInput::Backtest {
                    strategy,
                    config,
                    from_ms,
                    to_ms,
                    cost_model,
                    policy,
                } => {
                    let collector = collector.as_ref().ok_or_else(|| {
                        LnMarketsToolOutput::error("LN Markets collector is still starting")
                    })?;
                    let created_at_ms = unix_timestamp_ms().map_err(LnMarketsToolOutput::error)?;
                    let cost_model = cost_model.into();
                    let policy = policy.into();
                    let recorded = match strategy {
                        LnMarketsStrategyName::RebalanceToTarget => {
                            let config = serde_json::from_value(config).map_err(|error| {
                                LnMarketsToolOutput::error(format!(
                                    "invalid rebalance_to_target configuration: {error}"
                                ))
                            })?;
                            runtime.run_rebalance_backtest(
                                collector,
                                config,
                                from_ms,
                                to_ms,
                                cost_model,
                                policy,
                                created_at_ms,
                            )
                        }
                        LnMarketsStrategyName::FundingCarry => {
                            let config = serde_json::from_value(config).map_err(|error| {
                                LnMarketsToolOutput::error(format!(
                                    "invalid funding_carry configuration: {error}"
                                ))
                            })?;
                            runtime.run_funding_backtest(
                                collector,
                                config,
                                from_ms,
                                to_ms,
                                cost_model,
                                policy,
                                created_at_ms,
                            )
                        }
                        LnMarketsStrategyName::ThresholdSwing => {
                            let config = serde_json::from_value(config).map_err(|error| {
                                LnMarketsToolOutput::error(format!(
                                    "invalid threshold_swing configuration: {error}"
                                ))
                            })?;
                            runtime.run_threshold_swing_backtest(
                                collector,
                                config,
                                from_ms,
                                to_ms,
                                cost_model,
                                policy,
                                created_at_ms,
                            )
                        }
                    }
                    .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let output = LnMarketsToolOutput::success(json!({
                        "schema": "omega.lnmarkets.backtest_tool.v1",
                        "phase": "completed",
                        "status": if recorded.report.passed() { "passed" } else { "failed" },
                        "report_digest": recorded.report_digest,
                        "report": recorded.report,
                    }));
                    emit_lnmarkets_strategy_update(&event_stream, &output.0);
                    return Ok(output);
                }
                LnMarketsStrategyInput::BacktestReports { strategy, limit } => {
                    require_limit("backtest report history", limit, 1, 100)
                        .map_err(LnMarketsToolOutput::error)?;
                    let reports = runtime
                        .backtest_reports(strategy.map(LnMarketsStrategyName::label), limit.into())
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let output = LnMarketsToolOutput::success(json!({
                        "schema": "omega.lnmarkets.backtest_history.v1",
                        "phase": "completed",
                        "status": "available",
                        "reports": reports,
                    }));
                    emit_lnmarkets_strategy_update(&event_stream, &output.0);
                    return Ok(output);
                }
                LnMarketsStrategyInput::Start { strategy, config } => {
                    let (client, network) = self
                        .client
                        .authenticated(cx)
                        .await
                        .map_err(LnMarketsToolOutput::error)?;
                    if network != Network::Signet {
                        return Err(LnMarketsToolOutput::error(
                            "automated LN Markets strategies are restricted to signet",
                        ));
                    }
                    runtime
                        .claim_review_session(self.session_id.clone())
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let at_ms = unix_timestamp_ms().map_err(LnMarketsToolOutput::error)?;
                    match strategy {
                        LnMarketsStrategyName::RebalanceToTarget => {
                            let config = serde_json::from_value(config).map_err(|error| {
                                LnMarketsToolOutput::error(format!(
                                    "invalid rebalance_to_target configuration: {error}"
                                ))
                            })?;
                            runtime.start_rebalance(client, config, at_ms, cx).await
                        }
                        LnMarketsStrategyName::FundingCarry => {
                            let config = serde_json::from_value(config).map_err(|error| {
                                LnMarketsToolOutput::error(format!(
                                    "invalid funding_carry configuration: {error}"
                                ))
                            })?;
                            runtime.start_funding(client, config, at_ms, cx).await
                        }
                        LnMarketsStrategyName::ThresholdSwing => {
                            let config = serde_json::from_value(config).map_err(|error| {
                                LnMarketsToolOutput::error(format!(
                                    "invalid threshold_swing configuration: {error}"
                                ))
                            })?;
                            runtime
                                .start_threshold_swing(client, config, at_ms, cx)
                                .await
                        }
                    }
                }
                LnMarketsStrategyInput::Adjust { strategy, config } => {
                    runtime
                        .claim_review_session(self.session_id.clone())
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let at_ms = unix_timestamp_ms().map_err(LnMarketsToolOutput::error)?;
                    match strategy {
                        LnMarketsStrategyName::RebalanceToTarget => {
                            let config = serde_json::from_value(config).map_err(|error| {
                                LnMarketsToolOutput::error(format!(
                                    "invalid rebalance_to_target configuration: {error}"
                                ))
                            })?;
                            runtime.adjust_rebalance(config, at_ms).await
                        }
                        LnMarketsStrategyName::FundingCarry => {
                            let config = serde_json::from_value(config).map_err(|error| {
                                LnMarketsToolOutput::error(format!(
                                    "invalid funding_carry configuration: {error}"
                                ))
                            })?;
                            runtime.adjust_funding(config, at_ms).await
                        }
                        LnMarketsStrategyName::ThresholdSwing => {
                            let config = serde_json::from_value(config).map_err(|error| {
                                LnMarketsToolOutput::error(format!(
                                    "invalid threshold_swing configuration: {error}"
                                ))
                            })?;
                            runtime.adjust_threshold_swing(config, at_ms).await
                        }
                    }
                }
                LnMarketsStrategyInput::Halt { strategy, reason } => {
                    runtime
                        .claim_review_session(self.session_id.clone())
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
                    let at_ms = unix_timestamp_ms().map_err(LnMarketsToolOutput::error)?;
                    match strategy {
                        LnMarketsStrategyName::RebalanceToTarget => {
                            runtime.halt_rebalance(at_ms, reason).await
                        }
                        LnMarketsStrategyName::FundingCarry => {
                            runtime.halt_funding(at_ms, reason).await
                        }
                        LnMarketsStrategyName::ThresholdSwing => {
                            runtime.halt_threshold_swing(at_ms, reason).await
                        }
                    }
                }
            };
            if let Err(error) = result {
                let output = LnMarketsToolOutput::error(error.to_string());
                emit_lnmarkets_strategy_update(
                    &event_stream,
                    &json!({
                    "schema": "omega.lnmarkets.strategy.v1",
                    "status": "error",
                    "phase": "failed",
                        "action": action,
                        "strategy": strategy,
                        "error": error.to_string(),
                        "venue_capabilities": capability_report,
                        "strategies": runtime.strategy_snapshots(),
                    }),
                );
                return Err(output);
            }
            let strategies = runtime.strategy_snapshots();
            let status = completed_strategy_status(action, &strategies);
            let output = LnMarketsToolOutput::success(json!({
                "schema": "omega.lnmarkets.strategy.v1",
                "status": status,
                "phase": "completed",
                "action": action,
                "strategy": strategy,
                "venue_capabilities": capability_report,
                "strategies": strategies,
            }));
            emit_lnmarkets_strategy_update(&event_stream, &output.0);
            Ok(output)
        })
    }
}

fn strategy_action_labels(input: &LnMarketsStrategyInput) -> (&'static str, Option<&'static str>) {
    match input {
        LnMarketsStrategyInput::Status => ("status", None),
        LnMarketsStrategyInput::Backtest { strategy, .. } => ("backtest", Some(strategy.label())),
        LnMarketsStrategyInput::BacktestReports { strategy, .. } => (
            "backtest_reports",
            strategy.map(LnMarketsStrategyName::label),
        ),
        LnMarketsStrategyInput::Start { strategy, .. } => ("start", Some(strategy.label())),
        LnMarketsStrategyInput::Adjust { strategy, .. } => ("adjust", Some(strategy.label())),
        LnMarketsStrategyInput::Halt { strategy, .. } => ("halt", Some(strategy.label())),
    }
}

fn pending_strategy_status(action: &str) -> &'static str {
    match action {
        "backtest" => "backtesting",
        "backtest_reports" => "reading",
        "start" => "starting",
        "adjust" => "adjusting",
        "halt" => "halting",
        _ => "reading",
    }
}

fn completed_strategy_status(
    action: &str,
    strategies: &[crate::StrategyRuntimeSnapshot],
) -> &'static str {
    match action {
        "backtest" => "completed",
        "backtest_reports" => "completed",
        "start" | "adjust" => "running",
        "halt" => "halted",
        _ if strategies
            .iter()
            .any(|strategy| strategy.status == "running") =>
        {
            "running"
        }
        _ if strategies
            .iter()
            .any(|strategy| strategy.status == "halted") =>
        {
            "halted"
        }
        _ => "idle",
    }
}

fn emit_lnmarkets_strategy_update(event_stream: &ToolCallEventStream, value: &Value) {
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("running");
    let content = serde_json::to_string_pretty(value)
        .unwrap_or_else(|error| format!("Failed to serialize LN Markets strategy update: {error}"));
    event_stream.update_fields(
        acp::ToolCallUpdateFields::new()
            .title(format!("LN Markets strategy: {status}"))
            .content(vec![acp::ToolCallContent::Content(acp::Content::new(
                content,
            ))]),
    );
}

fn unix_timestamp_ms() -> Result<i64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?;
    i64::try_from(elapsed.as_millis()).map_err(|_| "system timestamp overflowed i64".to_string())
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LnMarketsSwapInput {
    /// LN Markets environment that will execute the swap. It must match the configured account.
    network: LnMarketsNetworkInput,
    /// Asset the user is spending. BTC amounts are satoshis. USD amounts are cents.
    in_asset: LnMarketsSwapAsset,
    /// Positive whole-number amount. BTC must be at least 1,000 sats; USD is in cents.
    amount: String,
}

impl AgentTool for LnMarketsSwapTool {
    type Input = LnMarketsSwapInput;
    type Output = LnMarketsToolOutput;

    const NAME: &'static str = "lnmarkets_swap";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }

    fn initial_title(&self, input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        match input {
            Ok(input) => format!(
                "Swap {} {:?} on LN Markets {}",
                input.amount,
                input.in_asset,
                Network::from(input.network).label()
            )
            .into(),
            Err(_) => "Swap on LN Markets".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            let requested_network = Network::from(input.network);
            let checked_at_ms = unix_timestamp_ms().unwrap_or(i64::MAX);
            self.capability_guard
                .require_effectful(checked_at_ms)
                .map_err(LnMarketsToolOutput::capability_refusal)?;
            let (client, configured_network) = self
                .client
                .authenticated(cx)
                .await
                .map_err(LnMarketsToolOutput::error)?;
            require_matching_network(configured_network, requested_network, "swap")
                .map_err(LnMarketsToolOutput::error)?;
            let request = match input.in_asset {
                LnMarketsSwapAsset::BTC => {
                    let amount_sats = input.amount.parse::<u64>().map_err(|_| {
                        LnMarketsToolOutput::error("BTC amount must be a whole number of satoshis")
                    })?;
                    NewSwapRequest::bitcoin_to_synthetic_usd(amount_sats)
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?
                }
                LnMarketsSwapAsset::USD => {
                    let amount_cents = input.amount.parse::<u64>().map_err(|_| {
                        LnMarketsToolOutput::error("USD amount must be a whole number of cents")
                    })?;
                    NewSwapRequest::synthetic_usd_to_bitcoin(amount_cents)
                        .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?
                }
            };
            let result = client
                .new_swap(&request)
                .await
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            Ok(LnMarketsToolOutput::success(json!({
                "schema": "omega.lnmarkets.swap.v1",
                "network": configured_network,
                "status": "completed",
                "swap": result,
            })))
        })
    }
}

/// The plugin's agent-tool contribution, consumed by the agent's thread
/// construction through the plugin registry.
pub fn agent_tools_registration() -> agent::PluginAgentTools {
    agent::PluginAgentTools {
        plugin_id: "lnmarkets",
        tool_names: &[
            LnMarketsAccountTool::NAME,
            LnMarketsMarketDataTool::NAME,
            LnMarketsSwapTool::NAME,
            LnMarketsFeaturesTool::NAME,
            LnMarketsLedgerTool::NAME,
            LnMarketsStrategyTool::NAME,
            LnMarketsMandateTool::NAME,
        ],
        build: std::rc::Rc::new(|context, cx| {
            let venue_capabilities = crate::venue_capability_store(cx).unwrap_or_default();
            let (account, market_data, swap, features, ledger, strategy, mandate) = lnmarkets_tools(
                context.http_client.clone(),
                context.credentials_provider.clone(),
                context.session_id.clone(),
                venue_capabilities,
            );
            vec![
                account.erase(),
                market_data.erase(),
                swap.erase(),
                features.erase(),
                ledger.erase(),
                strategy.erase(),
                mandate.erase(),
            ]
        }),
    }
}

pub fn lnmarkets_tools(
    http_client: Arc<dyn HttpClient>,
    credentials_provider: Arc<dyn CredentialsProvider>,
    session_id: String,
    venue_capabilities: VenueCapabilityStore,
) -> (
    LnMarketsAccountTool,
    LnMarketsMarketDataTool,
    LnMarketsSwapTool,
    LnMarketsFeaturesTool,
    LnMarketsLedgerTool,
    LnMarketsStrategyTool,
    LnMarketsMandateTool,
) {
    (
        LnMarketsAccountTool {
            client: ToolClient {
                http_client: http_client.clone(),
                credentials_provider: credentials_provider.clone(),
            },
        },
        LnMarketsMarketDataTool {
            client: ToolClient {
                http_client: http_client.clone(),
                credentials_provider: credentials_provider.clone(),
            },
        },
        LnMarketsSwapTool {
            client: ToolClient {
                http_client: http_client.clone(),
                credentials_provider: credentials_provider.clone(),
            },
            capability_guard: venue_capabilities.guard(
                crate::MANIFEST.id,
                VenueActionClass::AssetSwap,
                crate::CAPABILITY_MAX_AGE_MS,
            ),
        },
        LnMarketsFeaturesTool {
            capability_guard: venue_capabilities.guard(
                crate::MANIFEST.id,
                VenueActionClass::StrategyExecution,
                crate::CAPABILITY_MAX_AGE_MS,
            ),
        },
        LnMarketsLedgerTool,
        LnMarketsStrategyTool {
            client: ToolClient {
                http_client,
                credentials_provider,
            },
            session_id,
            capability_guard: venue_capabilities.guard(
                crate::MANIFEST.id,
                VenueActionClass::StrategyExecution,
                crate::CAPABILITY_MAX_AGE_MS,
            ),
        },
        LnMarketsMandateTool,
    )
}

fn require_limit(label: &str, value: u16, minimum: u16, maximum: u16) -> Result<(), String> {
    if (minimum..=maximum).contains(&value) {
        return Ok(());
    }
    Err(format!(
        "LN Markets {label} must be between {minimum} and {maximum}"
    ))
}

fn tool_section<T: Serialize>(result: Result<T, crate::Error>) -> Value {
    match result {
        Ok(data) => match serde_json::to_value(data) {
            Ok(data) => json!({ "ok": true, "data": data }),
            Err(error) => json!({ "ok": false, "error": error.to_string() }),
        },
        Err(error) => json!({ "ok": false, "error": error.to_string() }),
    }
}

fn require_matching_network(
    configured_network: Network,
    requested_network: Network,
    request_kind: &str,
) -> Result<(), String> {
    if configured_network == requested_network {
        return Ok(());
    }
    Err(format!(
        "LN Markets is configured for {}, but the {request_kind} request selected {}. No {request_kind} request was sent.",
        configured_network.label(),
        requested_network.label()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_data_input_without_a_view_keeps_the_snapshot_contract() {
        let input: LnMarketsMarketDataInput =
            serde_json::from_value(json!({ "network": "signet" })).expect("input");
        assert!(matches!(
            input.request,
            LnMarketsMarketDataRequest::Snapshot
        ));
    }

    #[test]
    fn market_data_limits_are_bounded() {
        assert!(require_limit("history", 1, 1, 1_000).is_ok());
        assert!(require_limit("history", 1_000, 1, 1_000).is_ok());
        assert!(require_limit("history", 0, 1, 1_000).is_err());
        assert!(require_limit("portfolio", 101, 1, 100).is_err());
    }

    #[test]
    fn mainnet_swaps_are_admitted_when_the_configured_network_matches() {
        assert!(require_matching_network(Network::Mainnet, Network::Mainnet, "swap").is_ok());
    }

    #[test]
    fn a_network_mismatch_is_refused_before_execution() {
        let error = require_matching_network(Network::Mainnet, Network::Signet, "swap")
            .expect_err("a signet request must not use a mainnet account");
        assert!(error.contains("No swap request was sent"));
    }

    #[test]
    fn mandate_tool_has_no_mutating_input_shape() {
        let error = serde_json::from_value::<LnMarketsMandateInput>(json!({
            "action": "widen",
            "include_history": false,
        }));
        assert!(error.is_err());
        let schema = schemars::schema_for!(LnMarketsMandateInput);
        let schema = serde_json::to_value(schema).expect("schema");
        assert!(!schema.to_string().contains("widen"));
        assert!(!schema.to_string().contains("mutate"));
    }

    #[test]
    fn strategy_input_names_the_strategy_and_keeps_configuration_typed_at_runtime() {
        let input = serde_json::from_value::<LnMarketsStrategyInput>(json!({
            "action": "start",
            "strategy": "funding_carry",
            "config": { "network": "signet" },
        }))
        .expect("strategy input");
        assert!(matches!(
            input,
            LnMarketsStrategyInput::Start {
                strategy: LnMarketsStrategyName::FundingCarry,
                ..
            }
        ));

        let threshold = serde_json::from_value::<LnMarketsStrategyInput>(json!({
            "action": "adjust",
            "strategy": "threshold_swing",
            "config": { "network": "signet" },
        }))
        .expect("threshold strategy input");
        assert!(matches!(
            threshold,
            LnMarketsStrategyInput::Adjust {
                strategy: LnMarketsStrategyName::ThresholdSwing,
                ..
            }
        ));
    }

    #[test]
    fn strategy_card_status_tracks_the_command_lifecycle() {
        let strategies = vec![crate::StrategyRuntimeSnapshot {
            strategy_id: "funding_carry".to_string(),
            status: "running".to_string(),
            started_at_ms: Some(1),
            halted_at_ms: None,
            halt_reason: None,
            state: None,
            last_action: None,
            lifecycle_event_count: 2,
        }];
        assert_eq!(pending_strategy_status("start"), "starting");
        assert_eq!(pending_strategy_status("adjust"), "adjusting");
        assert_eq!(completed_strategy_status("start", &strategies), "running");
        assert_eq!(completed_strategy_status("halt", &strategies), "halted");
    }
}
