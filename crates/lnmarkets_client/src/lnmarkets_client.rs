use std::{
    fmt, io,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_tungstenite::{
    WebSocketStream,
    async_std::{ConnectStream, connect_async},
    tungstenite::Message as WebSocketMessage,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use futures::{FutureExt as _, StreamExt as _, future::BoxFuture, lock::Mutex, select};
use hmac::{Hmac, Mac as _};
use http::{HeaderMap, Method, Request, Response, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Number;
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroize as _;

pub const CREDENTIAL_STORAGE_URL: &str = "https://lnmarkets.com/api/v3";

const MAX_RESPONSE_BYTES: u64 = 1_048_576;
const MAX_ATTEMPTS: usize = 4;
const AUTHENTICATED_REQUEST_INTERVAL: Duration = Duration::from_millis(50);
const PUBLIC_REQUEST_INTERVAL: Duration = Duration::from_millis(250);

type HmacSha256 = Hmac<Sha256>;

pub trait HttpTransport: Send + Sync {
    fn send(
        &self,
        request: Request<Vec<u8>>,
    ) -> BoxFuture<'static, anyhow::Result<Response<Vec<u8>>>>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    #[default]
    Signet,
    Mainnet,
}

impl Network {
    pub const fn rest_api_url(self) -> &'static str {
        match self {
            Self::Signet => "https://api.signet.lnmarkets.com/v3",
            Self::Mainnet => "https://api.lnmarkets.com/v3",
        }
    }

    pub const fn stream_api_url(self) -> &'static str {
        match self {
            Self::Signet => "wss://stream.signet.lnmarkets.com/v1",
            Self::Mainnet => "wss://stream.lnmarkets.com/v1",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Signet => "Signet",
            Self::Mainnet => "Mainnet",
        }
    }
}

#[derive(Clone)]
pub struct Credentials {
    access_key: String,
    secret: String,
    passphrase: String,
}

impl Credentials {
    pub fn new(
        access_key: impl Into<String>,
        secret: impl Into<String>,
        passphrase: impl Into<String>,
    ) -> Result<Self, Error> {
        let credentials = Self {
            access_key: access_key.into(),
            secret: secret.into(),
            passphrase: passphrase.into(),
        };
        if credentials.access_key.trim().is_empty()
            || credentials.secret.trim().is_empty()
            || credentials.passphrase.trim().is_empty()
        {
            return Err(Error::InvalidCredentials);
        }
        Ok(credentials)
    }

    pub fn access_key(&self) -> &str {
        &self.access_key
    }

    pub fn passphrase(&self) -> &str {
        &self.passphrase
    }

    pub fn secret_value(&self) -> &str {
        &self.secret
    }

    fn secret(&self) -> &str {
        &self.secret
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("access_key", &"[REDACTED]")
            .field("secret", &"[REDACTED]")
            .field("passphrase", &"[REDACTED]")
            .finish()
    }
}

impl Drop for Credentials {
    fn drop(&mut self) {
        self.access_key.zeroize();
        self.secret.zeroize();
        self.passphrase.zeroize();
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct StoredCredentials {
    version: u8,
    pub network: Network,
    access_key: String,
    secret: String,
    passphrase: String,
}

impl fmt::Debug for StoredCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredCredentials")
            .field("version", &self.version)
            .field("network", &self.network)
            .field("access_key", &"[REDACTED]")
            .field("secret", &"[REDACTED]")
            .field("passphrase", &"[REDACTED]")
            .finish()
    }
}

impl StoredCredentials {
    pub fn new(network: Network, credentials: &Credentials) -> Self {
        Self {
            version: 1,
            network,
            access_key: credentials.access_key.clone(),
            secret: credentials.secret.clone(),
            passphrase: credentials.passphrase.clone(),
        }
    }

    pub fn credentials(&self) -> Result<Credentials, Error> {
        Credentials::new(
            self.access_key.clone(),
            self.secret.clone(),
            self.passphrase.clone(),
        )
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        serde_json::to_vec(self).map_err(Error::Serialize)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let stored: Self = serde_json::from_slice(bytes).map_err(Error::Deserialize)?;
        if stored.version != 1 {
            return Err(Error::UnsupportedCredentialVersion(stored.version));
        }
        stored.credentials()?;
        Ok(stored)
    }
}

impl Drop for StoredCredentials {
    fn drop(&mut self) {
        self.access_key.zeroize();
        self.secret.zeroize();
        self.passphrase.zeroize();
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("LN Markets credentials require a key, secret, and passphrase")]
    InvalidCredentials,
    #[error("unsupported stored LN Markets credential version {0}")]
    UnsupportedCredentialVersion(u8),
    #[error("this LN Markets endpoint requires credentials")]
    AuthenticationRequired,
    #[error("invalid LN Markets amount: {0}")]
    InvalidAmount(String),
    #[error("failed to build LN Markets request: {0}")]
    BuildRequest(#[source] http::Error),
    #[error("failed to send LN Markets request: {0}")]
    Send(#[source] anyhow::Error),
    #[error("failed to read LN Markets response: {0}")]
    ReadResponse(#[source] io::Error),
    #[error("LN Markets returned HTTP {status}: {message}")]
    Api { status: StatusCode, message: String },
    #[error("failed to serialize LN Markets data: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to encode LN Markets query: {0}")]
    EncodeQuery(#[source] serde_urlencoded::ser::Error),
    #[error("failed to deserialize LN Markets data: {0}")]
    Deserialize(#[source] serde_json::Error),
    #[error("system clock is before the Unix epoch")]
    InvalidSystemTime,
    #[error("failed to initialize LN Markets request signer")]
    InvalidSigningKey,
    #[error("LN Markets request body was not valid UTF-8")]
    InvalidRequestBody,
    #[error("LN Markets response exceeded {MAX_RESPONSE_BYTES} bytes")]
    ResponseTooLarge,
    #[error("LN Markets request exhausted its retry policy")]
    RetryExhausted,
    #[error("failed to connect to the LN Markets Stream API: {0}")]
    StreamConnect(#[source] Box<async_tungstenite::tungstenite::Error>),
    #[error("LN Markets Stream request timed out")]
    StreamTimeout,
    #[error("LN Markets Stream connection closed: {0}")]
    StreamClosed(String),
    #[error("LN Markets Stream returned JSON-RPC error {code}: {message}")]
    StreamRpc { code: i64, message: String },
    #[error("invalid LN Markets Stream frame: {0}")]
    InvalidStreamFrame(#[source] serde_json::Error),
    #[error("unsupported LN Markets Stream topic `{0}`")]
    InvalidStreamTopic(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecimalAmount(Number);

impl DecimalAmount {
    pub fn as_number(&self) -> &Number {
        &self.0
    }
}

impl FromStr for DecimalAmount {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let number = Number::from_str(value)
            .map_err(|_| Error::InvalidAmount("use a positive decimal number".into()))?;
        let positive = number.as_u64().is_some_and(|value| value > 0)
            || number.as_i64().is_some_and(|value| value > 0)
            || number.as_f64().is_some_and(|value| value > 0.0);
        if !positive {
            return Err(Error::InvalidAmount(
                "amount must be greater than zero".into(),
            ));
        }
        Ok(Self(number))
    }
}

impl fmt::Display for DecimalAmount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Serialize for DecimalAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DecimalAmount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self(Number::deserialize(deserializer)?))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub balance: DecimalAmount,
    pub email: Option<String>,
    pub fee_tier: DecimalAmount,
    pub id: String,
    pub linking_public_key: Option<String>,
    pub synthetic_usd_balance: DecimalAmount,
    pub username: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerTime {
    pub time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitcoinAddress {
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LightningDeposit {
    pub id: String,
    pub created_at: String,
    pub amount: DecimalAmount,
    pub payment_hash: String,
    pub settled_at: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LightningWithdrawal {
    pub amount: DecimalAmount,
    pub created_at: String,
    pub destination: Option<String>,
    pub fee: DecimalAmount,
    pub id: String,
    pub payment_hash: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnChainDeposit {
    pub amount: DecimalAmount,
    pub block_height: Option<u64>,
    pub confirmations: u64,
    pub created_at: String,
    pub id: String,
    pub status: String,
    pub tx_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnChainWithdrawal {
    pub address: String,
    pub amount: DecimalAmount,
    pub created_at: String,
    pub fee: Option<DecimalAmount>,
    pub id: String,
    pub status: String,
    pub tx_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountNotification {
    pub id: String,
    pub created_at: String,
    pub event: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BestPrice {
    pub ask_price: DecimalAmount,
    pub bid_price: DecimalAmount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ticker {
    pub funding_rate: DecimalAmount,
    pub funding_time: String,
    pub index: DecimalAmount,
    pub last_price: DecimalAmount,
    pub prices: Vec<TickerPrice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TickerPrice {
    pub ask_price: DecimalAmount,
    pub bid_price: DecimalAmount,
    pub max_size: DecimalAmount,
    pub min_size: DecimalAmount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingSettlement {
    pub id: String,
    pub time: String,
    pub funding_rate: DecimalAmount,
    pub fixing_price: DecimalAmount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub direction: DecimalAmount,
    pub pl: DecimalAmount,
    pub username: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Leaderboard {
    pub daily: Vec<LeaderboardEntry>,
    pub weekly: Vec<LeaderboardEntry>,
    pub monthly: Vec<LeaderboardEntry>,
    #[serde(rename = "all-time")]
    pub all_time: Vec<LeaderboardEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candle {
    pub close: DecimalAmount,
    pub high: DecimalAmount,
    pub low: DecimalAmount,
    pub open: DecimalAmount,
    pub time: String,
    pub volume: DecimalAmount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandleResolution {
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

impl fmt::Display for CandleResolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::OneMinute => "1m",
            Self::ThreeMinutes => "3m",
            Self::FiveMinutes => "5m",
            Self::TenMinutes => "10m",
            Self::FifteenMinutes => "15m",
            Self::ThirtyMinutes => "30m",
            Self::FortyFiveMinutes => "45m",
            Self::OneHour => "1h",
            Self::TwoHours => "2h",
            Self::ThreeHours => "3h",
            Self::FourHours => "4h",
            Self::OneDay => "1d",
            Self::OneWeek => "1w",
            Self::OneMonth => "1month",
            Self::ThreeMonths => "3months",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CandlesQuery {
    pub from: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(rename = "range")]
    pub resolution: CandleResolution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FuturesCrossPosition {
    pub delta_pl: DecimalAmount,
    pub entry_price: Option<DecimalAmount>,
    pub funding_fees: DecimalAmount,
    pub id: String,
    pub initial_margin: DecimalAmount,
    pub leverage: DecimalAmount,
    pub liquidation: Option<DecimalAmount>,
    pub maintenance_margin: DecimalAmount,
    pub margin: DecimalAmount,
    pub quantity: DecimalAmount,
    pub running_margin: DecimalAmount,
    pub total_pl: DecimalAmount,
    pub trading_fees: DecimalAmount,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FuturesCrossOrder {
    pub canceled: bool,
    pub canceled_at: Option<String>,
    pub created_at: String,
    pub filled: bool,
    pub filled_at: Option<String>,
    pub id: String,
    pub open: bool,
    pub price: DecimalAmount,
    pub quantity: DecimalAmount,
    pub side: String,
    pub trading_fee: DecimalAmount,
    #[serde(rename = "type")]
    pub order_type: String,
    #[serde(default, alias = "uid")]
    pub client_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FuturesCrossTransfer {
    pub amount: DecimalAmount,
    pub id: String,
    pub time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FuturesFundingFee {
    pub fee: DecimalAmount,
    pub settlement_id: String,
    pub time: String,
    pub trade_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FuturesIsolatedTrade {
    pub canceled: bool,
    pub closed: bool,
    pub closed_at: Option<String>,
    pub closing_fee: DecimalAmount,
    pub created_at: String,
    pub entry_margin: Option<DecimalAmount>,
    pub entry_price: Option<DecimalAmount>,
    pub exit_price: Option<DecimalAmount>,
    pub filled_at: Option<String>,
    pub id: String,
    pub leverage: DecimalAmount,
    pub liquidation: DecimalAmount,
    pub maintenance_margin: DecimalAmount,
    pub margin: DecimalAmount,
    pub open: bool,
    pub opening_fee: DecimalAmount,
    pub pl: DecimalAmount,
    pub price: DecimalAmount,
    pub quantity: DecimalAmount,
    pub running: bool,
    pub side: String,
    pub stoploss: DecimalAmount,
    #[serde(default)]
    pub stoploss_trailing_distance: Option<DecimalAmount>,
    #[serde(default)]
    pub sum_cash_in_margin: Option<DecimalAmount>,
    #[serde(default)]
    pub sum_cash_in_pl: Option<DecimalAmount>,
    pub sum_funding_fees: DecimalAmount,
    pub takeprofit: DecimalAmount,
    #[serde(rename = "type")]
    pub trade_type: String,
    #[serde(default, alias = "uid")]
    pub client_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paginated<T> {
    pub data: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Pagination {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LightningDepositsQuery {
    #[serde(flatten)]
    pub pagination: Pagination,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settled: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AccountHistoryQuery {
    #[serde(flatten)]
    pub pagination: Pagination,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct NotificationsQuery {
    #[serde(flatten)]
    pub pagination: Pagination,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Asset {
    BTC,
    USD,
}

impl Asset {
    pub const fn opposite(self) -> Self {
        match self {
            Self::BTC => Self::USD,
            Self::USD => Self::BTC,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Swap {
    pub created_at: String,
    pub id: String,
    pub in_amount: DecimalAmount,
    pub in_asset: Asset,
    pub out_amount: DecimalAmount,
    pub out_asset: Asset,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSwapRequest {
    pub in_amount: DecimalAmount,
    pub in_asset: Asset,
    pub out_asset: Asset,
}

impl NewSwapRequest {
    pub fn bitcoin_to_synthetic_usd(amount_sats: u64) -> Result<Self, Error> {
        if amount_sats < 1_000 {
            return Err(Error::InvalidAmount(
                "Bitcoin swap input must be at least 1,000 sats".into(),
            ));
        }
        Ok(Self {
            in_amount: amount_sats.to_string().parse()?,
            in_asset: Asset::BTC,
            out_asset: Asset::USD,
        })
    }

    pub fn synthetic_usd_to_bitcoin(amount_cents: u64) -> Result<Self, Error> {
        if amount_cents == 0 {
            return Err(Error::InvalidAmount(
                "Synthetic USD swap input must be at least 1 cent".into(),
            ));
        }
        Ok(Self {
            in_amount: amount_cents.to_string().parse()?,
            in_asset: Asset::USD,
            out_asset: Asset::BTC,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSwapResult {
    pub in_amount: DecimalAmount,
    pub in_asset: Asset,
    pub out_amount: DecimalAmount,
    pub out_asset: Asset,
}

pub struct LnMarketsClient {
    http_transport: Arc<dyn HttpTransport>,
    network: Network,
    credentials: Option<Credentials>,
    next_authenticated_request: Arc<Mutex<Instant>>,
    next_public_request: Arc<Mutex<Instant>>,
}

impl LnMarketsClient {
    pub fn public(http_transport: Arc<dyn HttpTransport>, network: Network) -> Self {
        let now = Instant::now();
        Self {
            http_transport,
            network,
            credentials: None,
            next_authenticated_request: Arc::new(Mutex::new(now)),
            next_public_request: Arc::new(Mutex::new(now)),
        }
    }

    pub fn authenticated(
        http_transport: Arc<dyn HttpTransport>,
        network: Network,
        credentials: Credentials,
    ) -> Self {
        let mut client = Self::public(http_transport, network);
        client.credentials = Some(credentials);
        client
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub async fn ping(&self) -> Result<String, Error> {
        self.get_public("/ping", "").await
    }

    pub async fn server_time(&self) -> Result<ServerTime, Error> {
        self.get_public("/time", "").await
    }

    pub async fn account(&self) -> Result<Account, Error> {
        self.get_authenticated("/account", "").await
    }

    pub async fn bitcoin_address(&self) -> Result<BitcoinAddress, Error> {
        self.get_authenticated("/account/address/bitcoin", "").await
    }

    pub async fn lightning_deposits(
        &self,
        query: &LightningDepositsQuery,
    ) -> Result<Paginated<LightningDeposit>, Error> {
        let query = encoded_query(query)?;
        self.get_authenticated("/account/deposits/lightning", &query)
            .await
    }

    pub async fn lightning_withdrawals(
        &self,
        query: &AccountHistoryQuery,
    ) -> Result<Paginated<LightningWithdrawal>, Error> {
        let query = encoded_query(query)?;
        self.get_authenticated("/account/withdrawals/lightning", &query)
            .await
    }

    pub async fn on_chain_deposits(
        &self,
        query: &AccountHistoryQuery,
    ) -> Result<Paginated<OnChainDeposit>, Error> {
        let query = encoded_query(query)?;
        self.get_authenticated("/account/deposits/on-chain", &query)
            .await
    }

    pub async fn on_chain_withdrawals(
        &self,
        query: &AccountHistoryQuery,
    ) -> Result<Paginated<OnChainWithdrawal>, Error> {
        let query = encoded_query(query)?;
        self.get_authenticated("/account/withdrawals/on-chain", &query)
            .await
    }

    pub async fn notifications(
        &self,
        query: &NotificationsQuery,
    ) -> Result<Paginated<AccountNotification>, Error> {
        let query = encoded_query(query)?;
        self.get_authenticated("/account/notifications", &query)
            .await
    }

    pub async fn ticker(&self) -> Result<Ticker, Error> {
        self.get_public("/futures/ticker", "").await
    }

    pub async fn leaderboard(&self) -> Result<Leaderboard, Error> {
        self.get_public("/futures/leaderboard", "").await
    }

    pub async fn funding_settlements(
        &self,
        pagination: &Pagination,
    ) -> Result<Paginated<FundingSettlement>, Error> {
        let query = encoded_query(pagination)?;
        self.get_public("/futures/funding-settlements", &query)
            .await
    }

    pub async fn candles(&self, query: &CandlesQuery) -> Result<Paginated<Candle>, Error> {
        let query = encoded_query(query)?;
        self.get_public("/futures/candles", &query).await
    }

    pub async fn best_price(&self) -> Result<BestPrice, Error> {
        self.get_public("/synthetic-usd/best-price", "").await
    }

    pub async fn swaps(&self, pagination: &Pagination) -> Result<Paginated<Swap>, Error> {
        let query = encoded_query(pagination)?;
        self.get_authenticated("/synthetic-usd/swaps", &query).await
    }

    pub async fn cross_position(&self) -> Result<FuturesCrossPosition, Error> {
        self.get_authenticated("/futures/cross/position", "").await
    }

    pub async fn cross_open_orders(&self) -> Result<Vec<FuturesCrossOrder>, Error> {
        self.get_authenticated("/futures/cross/orders/open", "")
            .await
    }

    pub async fn cross_filled_orders(
        &self,
        pagination: &Pagination,
    ) -> Result<Paginated<FuturesCrossOrder>, Error> {
        let query = encoded_query(pagination)?;
        self.get_authenticated("/futures/cross/orders/filled", &query)
            .await
    }

    pub async fn cross_funding_fees(
        &self,
        pagination: &Pagination,
    ) -> Result<Paginated<FuturesFundingFee>, Error> {
        let query = encoded_query(pagination)?;
        self.get_authenticated("/futures/cross/funding-fees", &query)
            .await
    }

    pub async fn cross_transfers(
        &self,
        pagination: &Pagination,
    ) -> Result<Paginated<FuturesCrossTransfer>, Error> {
        let query = encoded_query(pagination)?;
        self.get_authenticated("/futures/cross/transfers", &query)
            .await
    }

    pub async fn isolated_open_trades(&self) -> Result<Vec<FuturesIsolatedTrade>, Error> {
        self.get_authenticated("/futures/isolated/trades/open", "")
            .await
    }

    pub async fn isolated_running_trades(&self) -> Result<Vec<FuturesIsolatedTrade>, Error> {
        self.get_authenticated("/futures/isolated/trades/running", "")
            .await
    }

    pub async fn isolated_closed_trades(
        &self,
        pagination: &Pagination,
    ) -> Result<Paginated<FuturesIsolatedTrade>, Error> {
        let query = encoded_query(pagination)?;
        self.get_authenticated("/futures/isolated/trades/closed", &query)
            .await
    }

    pub async fn isolated_canceled_trades(
        &self,
        pagination: &Pagination,
    ) -> Result<Paginated<FuturesIsolatedTrade>, Error> {
        let query = encoded_query(pagination)?;
        self.get_authenticated("/futures/isolated/trades/canceled", &query)
            .await
    }

    pub async fn isolated_funding_fees(
        &self,
        pagination: &Pagination,
        trade_id: Option<&str>,
    ) -> Result<Paginated<FuturesFundingFee>, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Query<'a> {
            #[serde(flatten)]
            pagination: &'a Pagination,
            #[serde(skip_serializing_if = "Option::is_none")]
            trade_id: Option<&'a str>,
        }

        let query = encoded_query(&Query {
            pagination,
            trade_id,
        })?;
        self.get_authenticated("/futures/isolated/funding-fees", &query)
            .await
    }

    pub async fn new_swap(&self, swap: &NewSwapRequest) -> Result<NewSwapResult, Error> {
        let body = serde_json::to_vec(swap).map_err(Error::Serialize)?;
        self.request_json(Method::POST, "/synthetic-usd/swap", "", body, true)
            .await
    }

    async fn get_public<T: DeserializeOwned>(&self, path: &str, query: &str) -> Result<T, Error> {
        self.request_json(Method::GET, path, query, Vec::new(), false)
            .await
    }

    async fn get_authenticated<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &str,
    ) -> Result<T, Error> {
        self.request_json(Method::GET, path, query, Vec::new(), true)
            .await
    }

    async fn request_json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &str,
        body: Vec<u8>,
        authenticated: bool,
    ) -> Result<T, Error> {
        let may_retry = method == Method::GET;
        let credentials = if authenticated {
            Some(
                self.credentials
                    .as_ref()
                    .ok_or(Error::AuthenticationRequired)?,
            )
        } else {
            None
        };

        for attempt in 0..MAX_ATTEMPTS {
            self.wait_for_request_slot(authenticated).await;
            let request = build_request(
                self.network,
                method.clone(),
                path,
                query,
                &body,
                credentials,
            )?;
            match self.http_transport.send(request).await {
                Ok(response) => {
                    if response.status().is_success() {
                        return parse_success_response(response).await;
                    }
                    if may_retry
                        && is_retryable_status(response.status())
                        && attempt + 1 < MAX_ATTEMPTS
                    {
                        let delay = retry_delay(attempt, response.headers());
                        drain_response(response).await?;
                        async_io::Timer::at(Instant::now() + delay).await;
                        continue;
                    }
                    return Err(parse_error_response(response).await?);
                }
                Err(error)
                    if may_retry && is_connection_error(&error) && attempt + 1 < MAX_ATTEMPTS =>
                {
                    async_io::Timer::at(Instant::now() + retry_delay(attempt, &Default::default()))
                        .await;
                }
                Err(error) => return Err(Error::Send(error)),
            }
        }
        Err(Error::RetryExhausted)
    }

    async fn wait_for_request_slot(&self, authenticated: bool) {
        let (slot, interval) = if authenticated {
            (
                &self.next_authenticated_request,
                AUTHENTICATED_REQUEST_INTERVAL,
            )
        } else {
            (&self.next_public_request, PUBLIC_REQUEST_INTERVAL)
        };
        let mut next_request = slot.lock().await;
        let now = Instant::now();
        if *next_request > now {
            async_io::Timer::at(*next_request).await;
        }
        *next_request = Instant::now() + interval;
    }
}

const STREAM_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_STREAM_FRAME_BYTES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StreamTopic(String);

impl StreamTopic {
    pub fn new(topic: impl Into<String>) -> Result<Self, Error> {
        let topic = topic.into();
        if is_supported_stream_topic(&topic) {
            Ok(Self(topic))
        } else {
            Err(Error::InvalidStreamTopic(topic))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_private(&self) -> bool {
        matches!(
            self.0.as_str(),
            "wallet/deposit"
                | "wallet/withdrawal"
                | "futures/inverse/btc_usd/isolated/trades"
                | "futures/inverse/btc_usd/cross/orders"
                | "futures/inverse/btc_usd/cross/position"
        )
    }
}

impl fmt::Display for StreamTopic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for StreamTopic {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    pub topic: StreamTopic,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamAuthentication {
    pub authenticated: bool,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamSubscription {
    pub subscribed: Vec<StreamTopic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamHello {
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamTime {
    pub time: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamIdentity {
    pub api_key: String,
    pub user_id: String,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamUnsubscription {
    pub unsubscribed: Vec<StreamTopic>,
}

pub struct LnMarketsStreamClient {
    socket: WebSocketStream<ConnectStream>,
    next_request_id: u64,
}

fn stream_error(error: async_tungstenite::tungstenite::Error) -> Error {
    Error::StreamConnect(Box::new(error))
}

impl LnMarketsStreamClient {
    pub async fn connect(network: Network) -> Result<Self, Error> {
        let connect = connect_async(network.stream_api_url()).fuse();
        let timeout =
            futures::FutureExt::fuse(async_io::Timer::at(Instant::now() + STREAM_REQUEST_TIMEOUT));
        futures::pin_mut!(connect, timeout);
        let (socket, _response) = select! {
            result = connect => result.map_err(stream_error)?,
            _ = timeout => return Err(Error::StreamTimeout),
        };
        Ok(Self {
            socket,
            next_request_id: 1,
        })
    }

    pub async fn hello(
        &mut self,
        client_name: &str,
        client_version: &str,
    ) -> Result<StreamHello, Error> {
        self.request(
            "hello",
            Some(serde_json::json!({
                "clientName": client_name,
                "clientVersion": client_version,
            })),
        )
        .await
    }

    pub async fn ping(&mut self) -> Result<String, Error> {
        self.request("ping", None).await
    }

    pub async fn server_time(&mut self) -> Result<StreamTime, Error> {
        self.request("time", None).await
    }

    pub async fn authenticate(
        &mut self,
        credentials: &Credentials,
    ) -> Result<StreamAuthentication, Error> {
        let timestamp = current_timestamp_millis()?;
        let nonce = format!("{:032x}", rand::random::<u128>());
        let signature = stream_signature(credentials.secret(), &timestamp, &nonce)?;
        self.request(
            "authenticate",
            Some(serde_json::json!({
                "key": credentials.access_key(),
                "signature": signature,
                "timestamp": timestamp.parse::<u64>().map_err(|_| Error::InvalidSystemTime)?,
                "passphrase": credentials.passphrase(),
                "nonce": nonce,
            })),
        )
        .await
    }

    pub async fn subscribe(&mut self, topics: &[StreamTopic]) -> Result<StreamSubscription, Error> {
        if topics.is_empty() {
            return Err(Error::InvalidStreamTopic(
                "at least one topic is required".into(),
            ));
        }
        self.request("subscribe", Some(serde_json::json!({ "topics": topics })))
            .await
    }

    pub async fn whoami(&mut self) -> Result<StreamIdentity, Error> {
        self.request("whoami", None).await
    }

    pub async fn unsubscribe(
        &mut self,
        topics: &[StreamTopic],
    ) -> Result<StreamUnsubscription, Error> {
        self.request("unsubscribe", Some(serde_json::json!({ "topics": topics })))
            .await
    }

    pub async fn unsubscribe_all(&mut self) -> Result<StreamUnsubscription, Error> {
        self.request("unsubscribeAll", None).await
    }

    pub async fn collect_events(
        &mut self,
        max_events: usize,
        timeout: Duration,
    ) -> Result<Vec<StreamEvent>, Error> {
        let max_events = max_events.clamp(1, 100);
        let deadline = Instant::now() + timeout.min(Duration::from_secs(30));
        let mut events = Vec::with_capacity(max_events);
        while events.len() < max_events {
            let next = self.socket.next().fuse();
            let timeout = futures::FutureExt::fuse(async_io::Timer::at(deadline));
            futures::pin_mut!(next, timeout);
            let message = select! {
                message = next => match message {
                    Some(message) => message.map_err(stream_error)?,
                    None => return Err(Error::StreamClosed("connection ended".into())),
                },
                _ = timeout => break,
            };
            if let Some(event) = self.handle_event_message(message).await? {
                events.push(event);
            }
        }
        Ok(events)
    }

    pub async fn close(mut self) -> Result<(), Error> {
        self.socket.close(None).await.map_err(stream_error)
    }

    async fn request<T: DeserializeOwned>(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<T, Error> {
        let id = self.next_request_id.to_string();
        self.next_request_id = self.next_request_id.saturating_add(1);
        let mut frame = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        if let Some(params) = params {
            frame["params"] = params;
        }
        let payload = serde_json::to_string(&frame).map_err(Error::Serialize)?;
        self.socket
            .send(WebSocketMessage::Text(payload.into()))
            .await
            .map_err(stream_error)?;

        let deadline = Instant::now() + STREAM_REQUEST_TIMEOUT;
        loop {
            let next = self.socket.next().fuse();
            let timeout = futures::FutureExt::fuse(async_io::Timer::at(deadline));
            futures::pin_mut!(next, timeout);
            let message = select! {
                message = next => match message {
                    Some(message) => message.map_err(stream_error)?,
                    None => return Err(Error::StreamClosed("connection ended".into())),
                },
                _ = timeout => return Err(Error::StreamTimeout),
            };
            let Some(frame) = self.decode_json_message(message).await? else {
                continue;
            };
            if frame.id.as_deref() != Some(id.as_str()) {
                continue;
            }
            if let Some(error) = frame.error {
                return Err(Error::StreamRpc {
                    code: error.code,
                    message: error.message,
                });
            }
            let result = frame.result.ok_or_else(|| {
                Error::InvalidStreamFrame(serde_json::Error::io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "response has no result",
                )))
            })?;
            return serde_json::from_value(result).map_err(Error::InvalidStreamFrame);
        }
    }

    async fn handle_event_message(
        &mut self,
        message: WebSocketMessage,
    ) -> Result<Option<StreamEvent>, Error> {
        let Some(frame) = self.decode_json_message(message).await? else {
            return Ok(None);
        };
        let Some(params) = frame.params else {
            return Ok(None);
        };
        if frame.method.as_deref() != Some("subscription") {
            return Ok(None);
        }
        Ok(Some(StreamEvent {
            topic: StreamTopic::new(params.topic)?,
            data: params.data,
        }))
    }

    async fn decode_json_message(
        &mut self,
        message: WebSocketMessage,
    ) -> Result<Option<StreamRpcFrame>, Error> {
        let bytes = match message {
            WebSocketMessage::Text(text) => text.as_bytes().to_vec(),
            WebSocketMessage::Binary(bytes) => bytes.to_vec(),
            WebSocketMessage::Ping(bytes) => {
                self.socket
                    .send(WebSocketMessage::Pong(bytes))
                    .await
                    .map_err(stream_error)?;
                return Ok(None);
            }
            WebSocketMessage::Pong(_) => return Ok(None),
            WebSocketMessage::Close(frame) => {
                return Err(Error::StreamClosed(
                    frame
                        .map(|frame| frame.reason.to_string())
                        .unwrap_or_else(|| "server closed the connection".into()),
                ));
            }
            _ => return Ok(None),
        };
        if bytes.len() > MAX_STREAM_FRAME_BYTES {
            return Err(Error::StreamClosed("frame exceeded 64 KiB".into()));
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(Error::InvalidStreamFrame)
    }
}

#[derive(Debug, Deserialize)]
struct StreamRpcFrame {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<StreamRpcError>,
    #[serde(default)]
    params: Option<StreamRpcSubscription>,
}

#[derive(Debug, Deserialize)]
struct StreamRpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Deserialize)]
struct StreamRpcSubscription {
    topic: String,
    data: serde_json::Value,
}

fn is_supported_stream_topic(topic: &str) -> bool {
    matches!(
        topic,
        "announcements"
            | "wallet/deposit"
            | "wallet/withdrawal"
            | "futures/inverse/btc_usd/ticker"
            | "futures/inverse/btc_usd/lastPrice"
            | "futures/inverse/btc_usd/index"
            | "futures/inverse/btc_usd/buckets"
            | "futures/inverse/btc_usd/funding"
            | "futures/inverse/btc_usd/isolated/trades"
            | "futures/inverse/btc_usd/cross/orders"
            | "futures/inverse/btc_usd/cross/position"
    ) || topic
        .strip_prefix("futures/inverse/btc_usd/ohlc/")
        .is_some_and(|resolution| {
            matches!(
                resolution,
                "1m" | "3m"
                    | "5m"
                    | "10m"
                    | "15m"
                    | "30m"
                    | "45m"
                    | "1h"
                    | "2h"
                    | "3h"
                    | "4h"
                    | "1d"
                    | "1w"
                    | "1month"
                    | "3months"
            )
        })
}

pub fn rest_signature(
    secret: &str,
    timestamp: &str,
    method: &Method,
    path: &str,
    data: &str,
) -> Result<String, Error> {
    let payload = format!(
        "{}{}{}{}",
        timestamp,
        method.as_str().to_lowercase(),
        path,
        data
    );
    let mut hmac =
        HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| Error::InvalidSigningKey)?;
    hmac.update(payload.as_bytes());
    Ok(BASE64_STANDARD.encode(hmac.finalize().into_bytes()))
}

pub fn stream_signature(secret: &str, timestamp: &str, nonce: &str) -> Result<String, Error> {
    let mut hmac =
        HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| Error::InvalidSigningKey)?;
    hmac.update(format!("{timestamp}{nonce}").as_bytes());
    Ok(BASE64_STANDARD.encode(hmac.finalize().into_bytes()))
}

fn build_request(
    network: Network,
    method: Method,
    path: &str,
    query: &str,
    body: &[u8],
    credentials: Option<&Credentials>,
) -> Result<Request<Vec<u8>>, Error> {
    let canonical_path = format!("/v3{path}");
    let uri = format!("{}{}{}", network.rest_api_url(), path, query);
    let mut builder = Request::builder()
        .method(method.clone())
        .uri(uri)
        .header("Accept", "application/json");
    if !body.is_empty() {
        builder = builder.header("Content-Type", "application/json");
    }
    if let Some(credentials) = credentials {
        let timestamp = current_timestamp_millis()?;
        let data = if method == Method::GET || method == Method::DELETE {
            query
        } else {
            std::str::from_utf8(body).map_err(|_| Error::InvalidRequestBody)?
        };
        let signature = rest_signature(
            credentials.secret(),
            &timestamp,
            &method,
            &canonical_path,
            data,
        )?;
        builder = builder
            .header("LNM-ACCESS-KEY", credentials.access_key())
            .header("LNM-ACCESS-PASSPHRASE", credentials.passphrase())
            .header("LNM-ACCESS-TIMESTAMP", timestamp)
            .header("LNM-ACCESS-SIGNATURE", signature);
    }
    builder.body(body.to_vec()).map_err(Error::BuildRequest)
}

fn current_timestamp_millis() -> Result<String, Error> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::InvalidSystemTime)?;
    Ok(duration.as_millis().to_string())
}

fn encoded_query<T: Serialize>(value: &T) -> Result<String, Error> {
    let query = serde_urlencoded::to_string(value).map_err(Error::EncodeQuery)?;
    if query.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("?{query}"))
    }
}

async fn parse_success_response<T: DeserializeOwned>(
    response: Response<Vec<u8>>,
) -> Result<T, Error> {
    let bytes = read_bounded_body(response.body())?;
    serde_json::from_slice(&bytes).map_err(Error::Deserialize)
}

async fn parse_error_response(response: Response<Vec<u8>>) -> Result<Error, Error> {
    let status = response.status();
    let bytes = read_bounded_body(response.body())?;
    let message = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .or_else(|| value.get("error"))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
    Ok(Error::Api {
        status,
        message: message.chars().take(512).collect(),
    })
}

async fn drain_response(response: Response<Vec<u8>>) -> Result<(), Error> {
    read_bounded_body(response.body()).map(|_| ())
}

fn read_bounded_body(body: &[u8]) -> Result<&[u8], Error> {
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    Ok(body)
}

fn is_retryable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 502 | 503 | 504)
}

fn retry_delay(attempt: usize, headers: &HeaderMap) -> Duration {
    if let Some(seconds) = headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        return Duration::from_secs(seconds.min(30));
    }
    let exponent = 1_u64 << attempt.min(3);
    Duration::from_secs(exponent) + Duration::from_millis(rand::random_range(0..=250))
}

fn is_connection_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<io::Error>().is_some_and(|error| {
            matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::NotConnected
                    | io::ErrorKind::BrokenPipe
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        sync::{Arc, Mutex as StdMutex},
    };

    use super::*;

    #[test]
    fn client_has_no_in_tree_dependencies() {
        let metadata = cargo_metadata::MetadataCommand::new()
            .no_deps()
            .exec()
            .expect("workspace metadata");
        let package = metadata
            .packages
            .iter()
            .find(|package| package.name.as_str() == "lnmarkets_client")
            .expect("lnmarkets_client package");
        let in_tree_dependencies = package
            .dependencies
            .iter()
            .filter_map(|dependency| dependency.path.as_ref().map(|_| dependency.name.as_str()))
            .collect::<Vec<_>>();
        assert!(
            in_tree_dependencies.is_empty(),
            "lnmarkets_client must not depend on Omega crates: {in_tree_dependencies:?}"
        );
    }

    type Handler = dyn Fn(Request<Vec<u8>>) -> BoxFuture<'static, anyhow::Result<Response<Vec<u8>>>>
        + Send
        + Sync;

    struct FakeTransport {
        handler: Arc<Handler>,
    }

    impl FakeTransport {
        fn create<Callback, ResponseFuture>(callback: Callback) -> Arc<Self>
        where
            Callback: Fn(Request<Vec<u8>>) -> ResponseFuture + Send + Sync + 'static,
            ResponseFuture: Future<Output = anyhow::Result<Response<Vec<u8>>>> + Send + 'static,
        {
            Arc::new(Self {
                handler: Arc::new(move |request| callback(request).boxed()),
            })
        }
    }

    impl HttpTransport for FakeTransport {
        fn send(
            &self,
            request: Request<Vec<u8>>,
        ) -> BoxFuture<'static, anyhow::Result<Response<Vec<u8>>>> {
            (self.handler)(request)
        }
    }

    fn response(status: u16, body: &str) -> anyhow::Result<Response<Vec<u8>>> {
        Ok(Response::builder()
            .status(status)
            .header("Content-Type", "application/json")
            .body(body.as_bytes().to_vec())?)
    }

    #[test]
    fn official_hmac_vector_matches() {
        let mut hmac = HmacSha256::new_from_slice(b"test-secret").expect("valid key");
        hmac.update(b"payload-1");
        assert_eq!(
            BASE64_STANDARD.encode(hmac.finalize().into_bytes()),
            "JJ8q7bXc7Kkj7cjj1EgBqA9pn70I9b2B8iLeDwDtQ2Y="
        );
    }

    #[test]
    fn authenticated_account_request_uses_v3_signature() {
        smol::block_on(async {
            let client = FakeTransport::create(|request| async move {
                assert_eq!(request.method(), Method::GET);
                assert_eq!(request.uri().path(), "/v3/account");
                let timestamp = request
                    .headers()
                    .get("LNM-ACCESS-TIMESTAMP")
                    .expect("timestamp")
                    .to_str()
                    .expect("ASCII timestamp");
                let expected = rest_signature("secret", timestamp, &Method::GET, "/v3/account", "")
                    .expect("signature");
                assert_eq!(request.headers()["LNM-ACCESS-SIGNATURE"], expected.as_str());
                response(
                    200,
                    r#"{"balance":1200,"email":null,"feeTier":1,"id":"a","linkingPublicKey":null,"syntheticUsdBalance":2.5,"username":"omega"}"#,
                )
            });
            let credentials = Credentials::new("key", "secret", "passphrase").expect("credentials");
            let client = LnMarketsClient::authenticated(client, Network::Signet, credentials);
            let account = client.account().await.expect("account");
            assert_eq!(account.username, "omega");
        });
    }

    #[test]
    fn public_ticker_parses_without_authentication_headers() {
        smol::block_on(async {
            let client = FakeTransport::create(|request| async move {
                assert_eq!(request.method(), Method::GET);
                assert_eq!(request.uri().path(), "/v3/futures/ticker");
                assert!(request.headers().get("LNM-ACCESS-KEY").is_none());
                assert!(request.headers().get("LNM-ACCESS-SIGNATURE").is_none());
                response(
                    200,
                    r#"{"fundingRate":0.0001,"fundingTime":"2026-08-09T08:00:00.000Z","index":64737,"lastPrice":64733,"prices":[{"askPrice":64732,"bidPrice":64723,"minSize":0,"maxSize":1000}]}"#,
                )
            });
            let client = LnMarketsClient::public(client, Network::Signet);
            let ticker = client.ticker().await.expect("ticker");
            assert_eq!(ticker.prices.len(), 1);
            assert_eq!(ticker.index.to_string(), "64737");
        });
    }

    #[test]
    fn authentication_failure_is_not_retried() {
        smol::block_on(async {
            let requests = Arc::new(StdMutex::new(0));
            let client = FakeTransport::create({
                let requests = requests.clone();
                move |_| {
                    let requests = requests.clone();
                    async move {
                        *requests.lock().expect("request counter") += 1;
                        response(401, r#"{"message":"Unauthorized"}"#)
                    }
                }
            });
            let credentials = Credentials::new("key", "secret", "passphrase").expect("credentials");
            let client = LnMarketsClient::authenticated(client, Network::Signet, credentials);
            let error = client.account().await.expect_err("401");
            assert!(
                matches!(error, Error::Api { status, .. } if status == StatusCode::UNAUTHORIZED)
            );
            assert_eq!(*requests.lock().expect("request counter"), 1);
        });
    }

    #[test]
    fn swap_body_and_signature_use_identical_compact_json() {
        smol::block_on(async {
            let client = FakeTransport::create(|request| async move {
                let body = std::str::from_utf8(request.body()).expect("UTF-8 body");
                assert_eq!(
                    body,
                    r#"{"inAmount":1000,"inAsset":"BTC","outAsset":"USD"}"#
                );
                let timestamp = request.headers()["LNM-ACCESS-TIMESTAMP"]
                    .to_str()
                    .expect("timestamp");
                let expected = rest_signature(
                    "secret",
                    timestamp,
                    &Method::POST,
                    "/v3/synthetic-usd/swap",
                    &body,
                )
                .expect("signature");
                assert_eq!(request.headers()["LNM-ACCESS-SIGNATURE"], expected);
                response(
                    200,
                    r#"{"inAmount":1000,"inAsset":"BTC","outAmount":0.55,"outAsset":"USD"}"#,
                )
            });
            let credentials = Credentials::new("key", "secret", "passphrase").expect("credentials");
            let client = LnMarketsClient::authenticated(client, Network::Signet, credentials);
            let result = client
                .new_swap(&NewSwapRequest::bitcoin_to_synthetic_usd(1_000).expect("swap"))
                .await
                .expect("result");
            assert_eq!(result.in_asset, Asset::BTC);
        });
    }

    #[test]
    fn mainnet_swap_uses_the_production_api_host() {
        smol::block_on(async {
            let client = FakeTransport::create(|request| async move {
                assert_eq!(request.method(), Method::POST);
                assert_eq!(request.uri().scheme_str(), Some("https"));
                assert_eq!(request.uri().host(), Some("api.lnmarkets.com"));
                assert_eq!(request.uri().path(), "/v3/synthetic-usd/swap");
                response(
                    200,
                    r#"{"inAmount":1000,"inAsset":"BTC","outAmount":0.55,"outAsset":"USD"}"#,
                )
            });
            let credentials = Credentials::new("key", "secret", "passphrase").expect("credentials");
            let client = LnMarketsClient::authenticated(client, Network::Mainnet, credentials);
            let result = client
                .new_swap(&NewSwapRequest::bitcoin_to_synthetic_usd(1_000).expect("swap"))
                .await
                .expect("result");
            assert_eq!(result.out_asset, Asset::USD);
        });
    }

    #[test]
    fn a_swap_post_is_never_retried() {
        smol::block_on(async {
            let requests = Arc::new(StdMutex::new(0));
            let client = FakeTransport::create({
                let requests = requests.clone();
                move |_| {
                    let requests = requests.clone();
                    async move {
                        *requests.lock().expect("request counter") += 1;
                        response(503, r#"{"message":"Service unavailable"}"#)
                    }
                }
            });
            let credentials = Credentials::new("key", "secret", "passphrase").expect("credentials");
            let client = LnMarketsClient::authenticated(client, Network::Mainnet, credentials);
            let error = client
                .new_swap(&NewSwapRequest::bitcoin_to_synthetic_usd(1_000).expect("swap"))
                .await
                .expect_err("503");
            assert!(matches!(error, Error::Api { status, .. } if status.as_u16() == 503));
            assert_eq!(*requests.lock().expect("request counter"), 1);
        });
    }

    #[test]
    fn candles_send_the_documented_cursor_query_and_parse_ohlcv() {
        smol::block_on(async {
            let client = FakeTransport::create(|request| async move {
                assert_eq!(request.method(), Method::GET);
                assert_eq!(request.uri().path(), "/v3/futures/candles");
                assert_eq!(
                    request.uri().query(),
                    Some("from=2026-08-08T00%3A00%3A00Z&limit=3&cursor=cursor-1&range=1h")
                );
                response(
                    200,
                    r#"{"data":[{"time":"2026-08-09T05:00:00.000Z","open":64710.5,"high":64714.5,"low":64708.5,"close":64710,"volume":669418}],"nextCursor":"2026-08-09T05:00:00.000Z"}"#,
                )
            });
            let client = LnMarketsClient::public(client, Network::Signet);
            let candles = client
                .candles(&CandlesQuery {
                    from: "2026-08-08T00:00:00Z".into(),
                    to: None,
                    limit: Some(3),
                    cursor: Some("cursor-1".into()),
                    resolution: CandleResolution::OneHour,
                })
                .await
                .expect("candles");
            assert_eq!(candles.data.len(), 1);
            assert_eq!(candles.data[0].volume.to_string(), "669418");
        });
    }

    #[test]
    fn portfolio_reads_use_authenticated_v3_routes() {
        smol::block_on(async {
            let client = FakeTransport::create(|request| async move {
                assert_eq!(request.method(), Method::GET);
                assert_eq!(request.uri().path(), "/v3/futures/cross/position");
                assert!(request.headers().get("LNM-ACCESS-SIGNATURE").is_some());
                response(
                    200,
                    r#"{"deltaPl":3,"entryPrice":64710.5,"fundingFees":2,"id":"position","initialMargin":1000,"leverage":10,"liquidation":58000,"maintenanceMargin":20,"margin":1200,"quantity":10000,"runningMargin":1000,"totalPl":200,"tradingFees":5,"updatedAt":"2026-08-09T05:00:00.000Z"}"#,
                )
            });
            let credentials = Credentials::new("key", "secret", "passphrase").expect("credentials");
            let client = LnMarketsClient::authenticated(client, Network::Signet, credentials);
            let position = client.cross_position().await.expect("position");
            assert_eq!(position.quantity.to_string(), "10000");
            assert_eq!(position.total_pl.to_string(), "200");
        });
    }

    #[test]
    fn account_history_reads_send_filters_and_parse_wallet_activity() {
        smol::block_on(async {
            let client = FakeTransport::create(|request| async move {
                assert_eq!(request.method(), Method::GET);
                assert_eq!(request.uri().path(), "/v3/account/withdrawals/lightning");
                assert_eq!(request.uri().query(), Some("limit=25&status=processed"));
                assert!(request.headers().get("LNM-ACCESS-SIGNATURE").is_some());
                response(
                    200,
                    r#"{"data":[{"amount":1200,"createdAt":"2026-08-09T05:00:00.000Z","destination":null,"fee":2,"id":"withdrawal","paymentHash":"hash","status":"processed"}],"nextCursor":null}"#,
                )
            });
            let credentials = Credentials::new("key", "secret", "passphrase").expect("credentials");
            let client = LnMarketsClient::authenticated(client, Network::Signet, credentials);
            let withdrawals = client
                .lightning_withdrawals(&AccountHistoryQuery {
                    pagination: Pagination {
                        limit: Some(25),
                        ..Pagination::default()
                    },
                    status: Some("processed".into()),
                })
                .await
                .expect("withdrawals");
            assert_eq!(withdrawals.data.len(), 1);
            assert_eq!(withdrawals.data[0].amount.to_string(), "1200");
        });
    }

    #[test]
    fn stream_topics_cover_public_and_private_contracts() {
        let ticker = StreamTopic::new("futures/inverse/btc_usd/ticker").expect("ticker");
        assert!(!ticker.is_private());
        let ohlc = StreamTopic::new("futures/inverse/btc_usd/ohlc/1m").expect("ohlc");
        assert!(!ohlc.is_private());
        let position =
            StreamTopic::new("futures/inverse/btc_usd/cross/position").expect("position");
        assert!(position.is_private());
        assert!(StreamTopic::new("futures/inverse/btc_usd/ohlc/2m").is_err());
    }

    #[test]
    #[ignore = "contacts the public LN Markets Signet Stream API"]
    fn live_signet_stream_delivers_market_events() {
        smol::block_on(async {
            let mut client = LnMarketsStreamClient::connect(Network::Signet)
                .await
                .expect("connect");
            let hello = client
                .hello("omega-test", env!("CARGO_PKG_VERSION"))
                .await
                .expect("hello");
            assert_eq!(hello.version, "1.0.0");
            assert_eq!(client.ping().await.expect("ping"), "pong");
            assert!(client.server_time().await.expect("time").time > 0);
            let topics = [
                StreamTopic::new("futures/inverse/btc_usd/ticker").expect("ticker"),
                StreamTopic::new("futures/inverse/btc_usd/lastPrice").expect("last price"),
                StreamTopic::new("futures/inverse/btc_usd/index").expect("index"),
                StreamTopic::new("futures/inverse/btc_usd/buckets").expect("buckets"),
                StreamTopic::new("futures/inverse/btc_usd/funding").expect("funding"),
                StreamTopic::new("futures/inverse/btc_usd/ohlc/1m").expect("ohlc"),
            ];
            let subscription = client.subscribe(&topics).await.expect("subscribe");
            assert_eq!(subscription.subscribed.len(), topics.len());
            let events = client
                .collect_events(1, Duration::from_secs(10))
                .await
                .expect("events");
            let unsubscription = client.unsubscribe(&topics).await.expect("unsubscribe");
            assert_eq!(unsubscription.unsubscribed.len(), topics.len());
            assert!(
                client
                    .unsubscribe_all()
                    .await
                    .expect("unsubscribe all")
                    .unsubscribed
                    .is_empty()
            );
            client.close().await.expect("close");
            assert!(!events.is_empty(), "the Signet stream emitted no events");
        });
    }

    #[test]
    fn stored_credentials_round_trip_without_debug_disclosure() {
        let credentials = Credentials::new("actual-key", "actual-secret", "actual-passphrase")
            .expect("credentials");
        let stored = StoredCredentials::new(Network::Signet, &credentials);
        let decoded = StoredCredentials::decode(&stored.encode().expect("encode")).expect("decode");
        assert_eq!(decoded.network, Network::Signet);
        let debug = format!("{credentials:?}");
        assert!(!debug.contains("actual-key"));
        assert!(!debug.contains("actual-secret"));
        assert!(!debug.contains("actual-passphrase"));
    }
}
