use std::{sync::Arc, time::Duration};

use agent_client_protocol::schema::v1 as acp;
use credentials_provider::CredentialsProvider;
use gpui::{App, AsyncApp, Task};
use http_client::HttpClient;
use language_model::LanguageModelToolResultContent;
use lnmarkets::{
    AccountHistoryQuery, CREDENTIAL_STORAGE_URL, CandleResolution, CandlesQuery, Credentials,
    LightningDepositsQuery, LnMarketsClient, LnMarketsStreamClient, Network, NewSwapRequest,
    NotificationsQuery, Pagination, StoredCredentials, StreamTopic, http_transport,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ui::SharedString;

use crate::{AgentTool, ToolCallEventStream, ToolInput};

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
}

impl From<LnMarketsToolOutput> for LanguageModelToolResultContent {
    fn from(output: LnMarketsToolOutput) -> Self {
        serde_json::to_string_pretty(&output.0)
            .unwrap_or_else(|error| format!("Failed to serialize LN Markets output: {error}"))
            .into()
    }
}

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
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

pub fn lnmarkets_tools(
    http_client: Arc<dyn HttpClient>,
    credentials_provider: Arc<dyn CredentialsProvider>,
) -> (
    LnMarketsAccountTool,
    LnMarketsMarketDataTool,
    LnMarketsSwapTool,
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
                http_client,
                credentials_provider,
            },
        },
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

fn tool_section<T: Serialize>(result: Result<T, lnmarkets::Error>) -> Value {
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
}
