use std::sync::Arc;

use agent_client_protocol::schema::v1 as acp;
use credentials_provider::CredentialsProvider;
use gpui::{App, AsyncApp, Task};
use http_client::HttpClient;
use language_model::LanguageModelToolResultContent;
use lnmarkets_client::{
    CREDENTIAL_STORAGE_URL, LnMarketsClient, Network, NewSwapRequest, StoredCredentials,
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
    async fn authenticated(&self, cx: &AsyncApp) -> Result<(LnMarketsClient, Network), String> {
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
        Ok((
            LnMarketsClient::authenticated(self.http_client.clone(), network, credentials),
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
    http_client: Arc<dyn HttpClient>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LnMarketsMarketDataInput {
    /// LN Markets environment to query.
    network: LnMarketsNetworkInput,
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
            Ok(input) => match input.network {
                LnMarketsNetworkInput::Signet => "Read LN Markets signet prices".into(),
                LnMarketsNetworkInput::Mainnet => "Read LN Markets mainnet prices".into(),
            },
            Err(_) => "Read LN Markets prices".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |_cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            let network = Network::from(input.network);
            let client = LnMarketsClient::public(self.http_client.clone(), network);
            let ticker = client
                .ticker()
                .await
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            let best_price = client
                .best_price()
                .await
                .map_err(|error| LnMarketsToolOutput::error(error.to_string()))?;
            Ok(LnMarketsToolOutput::success(json!({
                "schema": "omega.lnmarkets.market-data.v1",
                "network": network,
                "ticker": ticker,
                "synthetic_usd_best_price": best_price,
            })))
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
            Ok(input) => format!("Swap {} {:?} on LN Markets", input.amount, input.in_asset).into(),
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
            let (client, network) = self
                .client
                .authenticated(cx)
                .await
                .map_err(LnMarketsToolOutput::error)?;
            if network == Network::Mainnet {
                return Ok(LnMarketsToolOutput::success(json!({
                    "schema": "omega.lnmarkets.warning.v1",
                    "network": "mainnet",
                    "blocked": true,
                    "warning": "Mainnet LN Markets swaps are blocked. No request was sent.",
                })));
            }
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
                "network": network,
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
            http_client: http_client.clone(),
        },
        LnMarketsSwapTool {
            client: ToolClient {
                http_client,
                credentials_provider,
            },
        },
    )
}
