use std::{
    fmt, io,
    num::NonZeroU64,
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
use uuid::Uuid;
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

    pub const fn legacy_rest_api_url(self) -> &'static str {
        match self {
            Self::Signet => "https://api.signet.lnmarkets.com/v2",
            Self::Mainnet => "https://api.lnmarkets.com/v2",
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
    #[error("LN Markets Lightning deposits require a 64-character lowercase hex description hash")]
    InvalidDescriptionHash,
    #[error("LN Markets Lightning withdrawals require a BOLT11 invoice")]
    InvalidLightningInvoice,
    #[error("LN Markets on-chain operations require a Bitcoin address")]
    InvalidBitcoinAddress,
    #[error("LN Markets leverage must be between 1 and 100")]
    InvalidLeverage,
    #[error("invalid LN Markets trade ID: {0}")]
    InvalidTradeId(#[source] uuid::Error),
    #[error("invalid LN Markets order ID: {0}")]
    InvalidOrderId(#[source] uuid::Error),
    #[error("invalid LN Markets option trade ID: {0}")]
    InvalidOptionTradeId(#[source] uuid::Error),
    #[error("invalid LN Markets option instrument name `{0}`")]
    InvalidOptionInstrument(String),
    #[error("invalid LN Markets LNURL-auth parameter `{0}`")]
    InvalidLnurlAuthParameter(&'static str),
    #[error("LN Markets cross-margin limit price must be positive and use a 0.5 tick")]
    InvalidCrossLimitPrice,
    #[error("invalid LN Markets isolated trade state flags")]
    InvalidTradeState,
    #[error("trailing stop distance must be between 0.001 and 10")]
    InvalidTrailingStopDistance,
    #[error(
        "LN Markets request names {requested:?}, but the configured account uses {configured:?}"
    )]
    NetworkMismatch {
        requested: Network,
        configured: Network,
    },
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

    fn is_positive(&self) -> bool {
        self.0.as_u64().is_some_and(|value| value > 0)
            || self.0.as_i64().is_some_and(|value| value > 0)
            || self.0.as_f64().is_some_and(|value| value > 0.0)
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

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DescriptionHash(String);

impl DescriptionHash {
    pub fn new(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::InvalidDescriptionHash);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DescriptionHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DescriptionHash([REDACTED])")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct LightningInvoice(String);

impl LightningInvoice {
    pub fn new(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        let lowercase = value.to_ascii_lowercase();
        if value.trim() != value
            || value.chars().any(char::is_whitespace)
            || !matches_bolt11_prefix(&lowercase)
        {
            return Err(Error::InvalidLightningInvoice);
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for LightningInvoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LightningInvoice([REDACTED])")
    }
}

impl<'de> Deserialize<'de> for LightningInvoice {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LightningDestination(String);

impl LightningDestination {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for LightningDestination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LightningDestination([REDACTED])")
    }
}

fn matches_bolt11_prefix(value: &str) -> bool {
    ["lnbc1", "lntb1", "lntbs1", "lnbcrt1"]
        .iter()
        .any(|prefix| value.starts_with(prefix))
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BitcoinAddressValue(String);

impl BitcoinAddressValue {
    pub fn new(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        if value.trim().is_empty()
            || value.trim() != value
            || value.chars().any(char::is_whitespace)
        {
            return Err(Error::InvalidBitcoinAddress);
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BitcoinAddressValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BitcoinAddressValue([REDACTED])")
    }
}

impl<'de> Deserialize<'de> for BitcoinAddressValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
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
    pub address: BitcoinAddressValue,
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
    pub destination: Option<LightningDestination>,
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
    pub address: BitcoinAddressValue,
    pub amount: DecimalAmount,
    pub created_at: String,
    pub fee: Option<DecimalAmount>,
    pub id: String,
    pub status: String,
    pub tx_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LightningDepositRequest {
    amount: NonZeroU64,
    #[serde(skip_serializing_if = "Option::is_none")]
    comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description_hash: Option<DescriptionHash>,
}

impl LightningDepositRequest {
    pub fn new(amount_sats: u64) -> Result<Self, Error> {
        let amount = NonZeroU64::new(amount_sats).ok_or_else(|| {
            Error::InvalidAmount("Lightning deposit amount must be greater than zero".into())
        })?;
        Ok(Self {
            amount,
            comment: None,
            description_hash: None,
        })
    }

    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    pub fn with_description_hash(mut self, description_hash: DescriptionHash) -> Self {
        self.description_hash = Some(description_hash);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LightningDepositInvoice {
    pub deposit_id: String,
    pub payment_request: LightningInvoice,
}

#[derive(Debug, Clone, Serialize)]
pub struct LightningWithdrawalRequest {
    invoice: LightningInvoice,
}

impl LightningWithdrawalRequest {
    pub fn new(invoice: LightningInvoice) -> Self {
        Self { invoice }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LightningWithdrawalResult {
    pub amount: DecimalAmount,
    pub id: String,
    pub max_fees: DecimalAmount,
    pub payment_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OnChainWithdrawalRequest {
    address: BitcoinAddressValue,
    amount: DecimalAmount,
}

impl OnChainWithdrawalRequest {
    pub fn new(address: BitcoinAddressValue, amount: DecimalAmount) -> Self {
        Self { address, amount }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnChainWithdrawalResult {
    pub address: BitcoinAddressValue,
    pub amount: DecimalAmount,
    pub block_id: Option<String>,
    pub confirmation_height: Option<u64>,
    pub created_at: String,
    pub fee: Option<DecimalAmount>,
    pub id: String,
    pub status: String,
    pub tx_id: Option<String>,
    pub uid: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BitcoinAddressFormat {
    P2tr,
    P2wpkh,
}

#[derive(Debug, Clone, Serialize)]
pub struct BitcoinAddressRequest {
    format: BitcoinAddressFormat,
}

impl BitcoinAddressRequest {
    pub fn new(format: BitcoinAddressFormat) -> Self {
        Self { format }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedBitcoinAddress {
    pub address: BitcoinAddressValue,
    pub created_at: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct FuturesCrossOrderId(Uuid);

impl FromStr for FuturesCrossOrderId {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(Error::InvalidOrderId)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct FuturesCrossOrderQuantity(NonZeroU64);

impl FuturesCrossOrderQuantity {
    pub fn new(quantity_usd: u64) -> Result<Self, Error> {
        NonZeroU64::new(quantity_usd)
            .map(Self)
            .ok_or_else(|| Error::InvalidAmount("quantity must be greater than zero".into()))
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct FuturesCrossLimitPrice(DecimalAmount);

impl FuturesCrossLimitPrice {
    pub fn new(price: DecimalAmount) -> Result<Self, Error> {
        let doubled_price = price.as_number().as_f64().map(|price| price * 2.0);
        if doubled_price.is_some_and(|price| price > 0.0 && price.fract() == 0.0) {
            Ok(Self(price))
        } else {
            Err(Error::InvalidCrossLimitPrice)
        }
    }

    pub fn as_amount(&self) -> &DecimalAmount {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FuturesCrossOrderKind {
    Market,
    Limit { price: FuturesCrossLimitPrice },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuturesCrossNewOrderRequest {
    side: FuturesTradeSide,
    quantity: FuturesCrossOrderQuantity,
    kind: FuturesCrossOrderKind,
    client_id: String,
}

impl FuturesCrossNewOrderRequest {
    pub fn market(
        side: FuturesTradeSide,
        quantity: FuturesCrossOrderQuantity,
        client_id: impl Into<String>,
    ) -> Self {
        Self {
            side,
            quantity,
            kind: FuturesCrossOrderKind::Market,
            client_id: client_id.into(),
        }
    }

    pub fn limit(
        side: FuturesTradeSide,
        quantity: FuturesCrossOrderQuantity,
        price: FuturesCrossLimitPrice,
        client_id: impl Into<String>,
    ) -> Self {
        Self {
            side,
            quantity,
            kind: FuturesCrossOrderKind::Limit { price },
            client_id: client_id.into(),
        }
    }

    fn body(&self) -> Result<Vec<u8>, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            side: FuturesTradeSide,
            quantity: u64,
            #[serde(rename = "type")]
            order_type: &'static str,
            #[serde(skip_serializing_if = "Option::is_none")]
            price: Option<&'a DecimalAmount>,
            client_id: &'a str,
        }

        let (order_type, price) = match &self.kind {
            FuturesCrossOrderKind::Market => ("market", None),
            FuturesCrossOrderKind::Limit { price } => ("limit", Some(price.as_amount())),
        };
        serde_json::to_vec(&Body {
            side: self.side,
            quantity: self.quantity.get(),
            order_type,
            price,
            client_id: &self.client_id,
        })
        .map_err(Error::Serialize)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FuturesCrossCancelOrderRequest {
    pub id: FuturesCrossOrderId,
}

impl FuturesCrossCancelOrderRequest {
    pub fn new(id: FuturesCrossOrderId) -> Self {
        Self { id }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FuturesCrossTransferRequest {
    pub amount: NonZeroU64,
}

impl FuturesCrossTransferRequest {
    pub fn new(amount_sats: u64) -> Result<Self, Error> {
        let amount = NonZeroU64::new(amount_sats)
            .ok_or_else(|| Error::InvalidAmount("amount must be greater than zero".into()))?;
        Ok(Self { amount })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FuturesFundingFee {
    pub fee: DecimalAmount,
    pub settlement_id: String,
    pub time: String,
    pub trade_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FuturesIsolatedTradeState {
    Open,
    Running,
    Closed,
    Canceled,
}

impl FuturesIsolatedTradeState {
    fn from_flags(open: bool, running: bool, closed: bool, canceled: bool) -> Result<Self, Error> {
        match (open, running, closed, canceled) {
            (true, false, false, false) => Ok(Self::Open),
            (false, true, false, false) => Ok(Self::Running),
            (true, true, false, false) => Ok(Self::Running),
            (false, false, true, false) => Ok(Self::Closed),
            (false, false, false, true) => Ok(Self::Canceled),
            _ => Err(Error::InvalidTradeState),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "FuturesIsolatedTradeWire")]
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
    #[serde(skip_serializing)]
    pub state: FuturesIsolatedTradeState,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FuturesIsolatedTradeWire {
    canceled: bool,
    closed: bool,
    closed_at: Option<String>,
    closing_fee: DecimalAmount,
    created_at: String,
    entry_margin: Option<DecimalAmount>,
    entry_price: Option<DecimalAmount>,
    exit_price: Option<DecimalAmount>,
    filled_at: Option<String>,
    id: String,
    leverage: DecimalAmount,
    liquidation: DecimalAmount,
    maintenance_margin: DecimalAmount,
    margin: DecimalAmount,
    open: bool,
    opening_fee: DecimalAmount,
    pl: DecimalAmount,
    price: DecimalAmount,
    quantity: DecimalAmount,
    running: bool,
    side: String,
    stoploss: DecimalAmount,
    #[serde(default)]
    stoploss_trailing_distance: Option<DecimalAmount>,
    #[serde(default)]
    sum_cash_in_margin: Option<DecimalAmount>,
    #[serde(default)]
    sum_cash_in_pl: Option<DecimalAmount>,
    sum_funding_fees: DecimalAmount,
    takeprofit: DecimalAmount,
    #[serde(rename = "type")]
    trade_type: String,
    #[serde(default, alias = "uid")]
    client_id: Option<String>,
}

impl TryFrom<FuturesIsolatedTradeWire> for FuturesIsolatedTrade {
    type Error = Error;

    fn try_from(trade: FuturesIsolatedTradeWire) -> Result<Self, Self::Error> {
        let state = FuturesIsolatedTradeState::from_flags(
            trade.open,
            trade.running,
            trade.closed,
            trade.canceled,
        )?;
        Ok(Self {
            canceled: trade.canceled,
            closed: trade.closed,
            closed_at: trade.closed_at,
            closing_fee: trade.closing_fee,
            created_at: trade.created_at,
            entry_margin: trade.entry_margin,
            entry_price: trade.entry_price,
            exit_price: trade.exit_price,
            filled_at: trade.filled_at,
            id: trade.id,
            leverage: trade.leverage,
            liquidation: trade.liquidation,
            maintenance_margin: trade.maintenance_margin,
            margin: trade.margin,
            open: trade.open,
            opening_fee: trade.opening_fee,
            pl: trade.pl,
            price: trade.price,
            quantity: trade.quantity,
            running: trade.running,
            side: trade.side,
            stoploss: trade.stoploss,
            stoploss_trailing_distance: trade.stoploss_trailing_distance,
            sum_cash_in_margin: trade.sum_cash_in_margin,
            sum_cash_in_pl: trade.sum_cash_in_pl,
            sum_funding_fees: trade.sum_funding_fees,
            takeprofit: trade.takeprofit,
            trade_type: trade.trade_type,
            client_id: trade.client_id,
            state,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct FuturesLeverage(u8);

impl FuturesLeverage {
    pub fn new(leverage: u8) -> Result<Self, Error> {
        if (1..=100).contains(&leverage) {
            Ok(Self(leverage))
        } else {
            Err(Error::InvalidLeverage)
        }
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct FuturesTradeId(Uuid);

impl FromStr for FuturesTradeId {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(Error::InvalidTradeId)
    }
}

impl fmt::Display for FuturesTradeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FuturesTradeSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuturesIsolatedTradeSize {
    MarginSats(NonZeroU64),
    QuantityUsd(NonZeroU64),
}

impl FuturesIsolatedTradeSize {
    pub fn margin_sats(amount: u64) -> Result<Self, Error> {
        NonZeroU64::new(amount)
            .map(Self::MarginSats)
            .ok_or_else(|| Error::InvalidAmount("margin must be greater than zero".into()))
    }

    pub fn quantity_usd(amount: u64) -> Result<Self, Error> {
        NonZeroU64::new(amount)
            .map(Self::QuantityUsd)
            .ok_or_else(|| Error::InvalidAmount("quantity must be greater than zero".into()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FuturesIsolatedOrder {
    Market,
    Limit { price: DecimalAmount },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuturesIsolatedNewTradeRequest {
    leverage: FuturesLeverage,
    side: FuturesTradeSide,
    size: FuturesIsolatedTradeSize,
    order: FuturesIsolatedOrder,
    stoploss: Option<DecimalAmount>,
    takeprofit: Option<DecimalAmount>,
    client_id: Option<String>,
}

impl FuturesIsolatedNewTradeRequest {
    pub fn market(
        leverage: FuturesLeverage,
        side: FuturesTradeSide,
        size: FuturesIsolatedTradeSize,
    ) -> Self {
        Self {
            leverage,
            side,
            size,
            order: FuturesIsolatedOrder::Market,
            stoploss: None,
            takeprofit: None,
            client_id: None,
        }
    }

    pub fn limit(
        leverage: FuturesLeverage,
        side: FuturesTradeSide,
        size: FuturesIsolatedTradeSize,
        price: DecimalAmount,
    ) -> Result<Self, Error> {
        if !price.is_positive() {
            return Err(Error::InvalidAmount(
                "limit price must be greater than zero".into(),
            ));
        }
        Ok(Self {
            leverage,
            side,
            size,
            order: FuturesIsolatedOrder::Limit { price },
            stoploss: None,
            takeprofit: None,
            client_id: None,
        })
    }

    pub fn with_stoploss(mut self, stoploss: DecimalAmount) -> Result<Self, Error> {
        if !stoploss.is_positive() {
            return Err(Error::InvalidAmount(
                "stoploss must be greater than zero".into(),
            ));
        }
        self.stoploss = Some(stoploss);
        Ok(self)
    }

    pub fn with_takeprofit(mut self, takeprofit: DecimalAmount) -> Result<Self, Error> {
        if !takeprofit.is_positive() {
            return Err(Error::InvalidAmount(
                "takeprofit must be greater than zero".into(),
            ));
        }
        self.takeprofit = Some(takeprofit);
        Ok(self)
    }

    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = Some(client_id.into());
        self
    }

    fn body(&self) -> Result<Vec<u8>, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            leverage: u8,
            #[serde(skip_serializing_if = "Option::is_none")]
            margin: Option<u64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            price: Option<&'a DecimalAmount>,
            #[serde(skip_serializing_if = "Option::is_none")]
            quantity: Option<u64>,
            side: FuturesTradeSide,
            #[serde(skip_serializing_if = "Option::is_none")]
            stoploss: Option<&'a DecimalAmount>,
            #[serde(skip_serializing_if = "Option::is_none")]
            takeprofit: Option<&'a DecimalAmount>,
            #[serde(rename = "type")]
            trade_type: &'static str,
            #[serde(skip_serializing_if = "Option::is_none")]
            client_id: Option<&'a str>,
        }

        let (margin, quantity) = match self.size {
            FuturesIsolatedTradeSize::MarginSats(amount) => (Some(amount.get()), None),
            FuturesIsolatedTradeSize::QuantityUsd(amount) => (None, Some(amount.get())),
        };
        let (trade_type, price) = match &self.order {
            FuturesIsolatedOrder::Market => ("market", None),
            FuturesIsolatedOrder::Limit { price } => ("limit", Some(price)),
        };
        serde_json::to_vec(&Body {
            leverage: self.leverage.get(),
            margin,
            price,
            quantity,
            side: self.side,
            stoploss: self.stoploss.as_ref(),
            takeprofit: self.takeprofit.as_ref(),
            trade_type,
            client_id: self.client_id.as_deref(),
        })
        .map_err(Error::Serialize)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FuturesIsolatedTradeReference {
    pub id: FuturesTradeId,
}

impl FuturesIsolatedTradeReference {
    pub fn new(id: FuturesTradeId) -> Self {
        Self { id }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FuturesIsolatedAmountRequest {
    pub amount: NonZeroU64,
    pub id: FuturesTradeId,
}

impl FuturesIsolatedAmountRequest {
    pub fn new(id: FuturesTradeId, amount: u64) -> Result<Self, Error> {
        let amount = NonZeroU64::new(amount)
            .ok_or_else(|| Error::InvalidAmount("amount must be greater than zero".into()))?;
        Ok(Self { amount, id })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FuturesStoplossMode {
    Fixed,
    Trailing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FuturesIsolatedStoplossUpdate {
    id: FuturesTradeId,
    value: DecimalAmount,
    mode: FuturesStoplossMode,
}

impl FuturesIsolatedStoplossUpdate {
    pub fn fixed(id: FuturesTradeId, value: DecimalAmount) -> Result<Self, Error> {
        if !value.is_positive() {
            return Err(Error::InvalidAmount(
                "fixed stoploss must be greater than zero".into(),
            ));
        }
        Ok(Self {
            id,
            value,
            mode: FuturesStoplossMode::Fixed,
        })
    }

    pub fn trailing(id: FuturesTradeId, value: DecimalAmount) -> Result<Self, Error> {
        let distance = value.as_number().as_f64();
        if !distance.is_some_and(|distance| (0.001..=10.0).contains(&distance)) {
            return Err(Error::InvalidTrailingStopDistance);
        }
        Ok(Self {
            id,
            value,
            mode: FuturesStoplossMode::Trailing,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FuturesIsolatedTakeprofitUpdate {
    id: FuturesTradeId,
    value: DecimalAmount,
}

impl FuturesIsolatedTakeprofitUpdate {
    pub fn new(id: FuturesTradeId, value: DecimalAmount) -> Result<Self, Error> {
        if !value.is_positive() {
            return Err(Error::InvalidAmount(
                "takeprofit must be greater than zero".into(),
            ));
        }
        Ok(Self { id, value })
    }
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

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct OptionInstrumentName(String);

impl OptionInstrumentName {
    pub fn new(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        let mut parts = value.split('.');
        let asset = parts.next();
        let expiry = parts.next();
        let strike = parts.next();
        let kind = parts.next();
        let valid_expiry = expiry.is_some_and(|expiry| {
            expiry.len() == 10
                && expiry.bytes().enumerate().all(|(index, byte)| {
                    if index == 4 || index == 7 {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit()
                    }
                })
        });
        let valid_strike = strike
            .and_then(|strike| strike.parse::<NonZeroU64>().ok())
            .is_some();
        if asset != Some("BTC")
            || !valid_expiry
            || !valid_strike
            || !matches!(kind, Some("C" | "P"))
            || parts.next().is_some()
        {
            return Err(Error::InvalidOptionInstrument(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OptionInstrumentName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("OptionInstrumentName")
            .field(&self.0)
            .finish()
    }
}

impl fmt::Display for OptionInstrumentName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<'de> Deserialize<'de> for OptionInstrumentName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptionSettlement {
    Physical,
    Cash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptionTradeStatus {
    Running,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptionSide {
    #[serde(rename = "b")]
    Buy,
    #[serde(rename = "s")]
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptionKind {
    #[serde(rename = "c")]
    Call,
    #[serde(rename = "p")]
    Put,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OptionTradeId(Uuid);

impl FromStr for OptionTradeId {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(Error::InvalidOptionTradeId)
    }
}

impl fmt::Display for OptionTradeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionInstrument {
    pub volatility: DecimalAmount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionMarketRange {
    pub min: DecimalAmount,
    pub max: DecimalAmount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionMarketCountLimit {
    pub max: DecimalAmount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionMarketLimits {
    pub margin: OptionMarketRange,
    pub quantity: OptionMarketRange,
    pub count: OptionMarketCountLimit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionMarketFees {
    pub trading: DecimalAmount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionMarket {
    pub active: bool,
    pub limits: OptionMarketLimits,
    pub fees: OptionMarketFees,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionVolatilityIndex {
    pub volatility_index: DecimalAmount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionTrade {
    pub id: OptionTradeId,
    pub uid: Uuid,
    pub forward: DecimalAmount,
    pub forward_point: i64,
    pub domestic: String,
    pub settlement: OptionSettlement,
    pub fixing_price: Option<DecimalAmount>,
    pub creation_ts: DecimalAmount,
    pub expiry_ts: DecimalAmount,
    pub closed_ts: Option<DecimalAmount>,
    pub physical_delivery_id: Option<String>,
    pub leg_id: Uuid,
    pub side: OptionSide,
    #[serde(rename = "type")]
    pub kind: OptionKind,
    pub quantity: i64,
    pub strike: i64,
    pub volatility: DecimalAmount,
    pub margin: i64,
    pub pl: i64,
    pub maintenance_margin: i64,
    pub opening_fee: i64,
    pub closing_fee: i64,
    pub running: bool,
    pub closed: bool,
    pub expired: bool,
    pub exercised: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OptionBuyRequest {
    side: OptionSide,
    quantity: NonZeroU64,
    settlement: OptionSettlement,
    instrument_name: OptionInstrumentName,
}

impl OptionBuyRequest {
    pub fn new(
        instrument_name: OptionInstrumentName,
        quantity: u64,
        settlement: OptionSettlement,
    ) -> Result<Self, Error> {
        let quantity = NonZeroU64::new(quantity).ok_or_else(|| {
            Error::InvalidAmount("option quantity must be greater than zero".into())
        })?;
        Ok(Self {
            side: OptionSide::Buy,
            quantity,
            settlement,
            instrument_name,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OptionSettlementUpdate {
    id: OptionTradeId,
    settlement: OptionSettlement,
}

impl OptionSettlementUpdate {
    pub fn new(id: OptionTradeId, settlement: OptionSettlement) -> Self {
        Self { id, settlement }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct OptionTradesQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<OptionTradeStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u16>,
}

impl OptionTradesQuery {
    pub fn new(status: OptionTradeStatus) -> Self {
        Self {
            status: Some(status),
            ..Self::default()
        }
    }

    pub fn with_time_range(mut self, from: u64, to: u64) -> Self {
        self.from = Some(from);
        self.to = Some(to);
        self
    }

    pub fn with_limit(mut self, limit: u16) -> Result<Self, Error> {
        if limit == 0 || limit > 1_000 {
            return Err(Error::InvalidAmount(
                "option trade limit must be between 1 and 1000".into(),
            ));
        }
        self.limit = Some(limit);
        Ok(self)
    }
}

#[derive(Debug, Clone)]
pub struct OptionCloseAllResult {
    pub trades: Vec<OptionTrade>,
}

impl<'de> Deserialize<'de> for OptionCloseAllResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum WireResult {
            Trades(Vec<OptionTrade>),
            Object { trades: Vec<OptionTrade> },
        }

        Ok(match WireResult::deserialize(deserializer)? {
            WireResult::Trades(trades) | WireResult::Object { trades } => Self { trades },
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LegacySyntheticUsdRequest {
    amount: DecimalAmount,
}

impl LegacySyntheticUsdRequest {
    pub fn new(amount: DecimalAmount) -> Self {
        Self { amount }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LegacyInternalTransferRequest {
    uid: Uuid,
    amount: NonZeroU64,
}

impl LegacyInternalTransferRequest {
    pub fn new(uid: Uuid, amount_sats: u64) -> Result<Self, Error> {
        let amount = NonZeroU64::new(amount_sats).ok_or_else(|| {
            Error::InvalidAmount("internal transfer amount must be greater than zero".into())
        })?;
        Ok(Self { uid, amount })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LnurlAuthChallenge {
    pub lnurl: String,
}

#[derive(Clone, Serialize)]
pub struct LnurlAuthCallback {
    tag: &'static str,
    k1: String,
    hmac: String,
    sig: String,
    key: String,
    #[serde(rename = "token")]
    request_token: bool,
}

impl LnurlAuthCallback {
    pub fn new(
        k1: impl Into<String>,
        hmac: impl Into<String>,
        signature: impl Into<String>,
        linking_public_key: impl Into<String>,
        request_token: bool,
    ) -> Result<Self, Error> {
        let k1 = validate_hex_parameter(k1.into(), 64, "k1")?;
        let hmac = validate_hex_parameter(hmac.into(), 64, "hmac")?;
        let sig = validate_hex_parameter(signature.into(), 2, "sig")?;
        let key = validate_hex_parameter(linking_public_key.into(), 66, "key")?;
        Ok(Self {
            tag: "login",
            k1,
            hmac,
            sig,
            key,
            request_token,
        })
    }
}

impl fmt::Debug for LnurlAuthCallback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LnurlAuthCallback([REDACTED])")
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LnurlAuthToken {
    pub token: String,
}

impl fmt::Debug for LnurlAuthToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LnurlAuthToken([REDACTED])")
    }
}

fn validate_hex_parameter(
    value: String,
    minimum_length: usize,
    name: &'static str,
) -> Result<String, Error> {
    if value.len() < minimum_length
        || !value.len().is_multiple_of(2)
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Error::InvalidLnurlAuthParameter(name));
    }
    Ok(value)
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

    pub async fn deposit_lightning(
        &self,
        network: Network,
        request: &LightningDepositRequest,
    ) -> Result<LightningDepositInvoice, Error> {
        self.account_post(network, "/account/deposit/lightning", request)
            .await
    }

    pub async fn withdraw_lightning(
        &self,
        network: Network,
        request: &LightningWithdrawalRequest,
    ) -> Result<LightningWithdrawalResult, Error> {
        self.account_post(network, "/account/withdraw/lightning", request)
            .await
    }

    pub async fn withdraw_on_chain(
        &self,
        network: Network,
        request: &OnChainWithdrawalRequest,
    ) -> Result<OnChainWithdrawalResult, Error> {
        self.account_post(network, "/account/withdraw/on-chain", request)
            .await
    }

    pub async fn add_bitcoin_address(
        &self,
        network: Network,
        request: &BitcoinAddressRequest,
    ) -> Result<AddedBitcoinAddress, Error> {
        self.account_post(network, "/account/address/bitcoin", request)
            .await
    }

    pub async fn mark_notifications_read(&self, network: Network) -> Result<(), Error> {
        self.require_network(network)?;
        self.request_empty(Method::PUT, "/account/notifications", "", Vec::new(), true)
            .await
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

    pub async fn cross_new_order(
        &self,
        network: Network,
        order: &FuturesCrossNewOrderRequest,
    ) -> Result<FuturesCrossOrder, Error> {
        self.require_network(network)?;
        self.request_json(
            Method::POST,
            "/futures/cross/order",
            "",
            order.body()?,
            true,
        )
        .await
    }

    pub async fn cross_cancel_order(
        &self,
        network: Network,
        order: &FuturesCrossCancelOrderRequest,
    ) -> Result<FuturesCrossOrder, Error> {
        self.cross_post(network, "/futures/cross/order/cancel", order)
            .await
    }

    pub async fn cross_cancel_all_orders(
        &self,
        network: Network,
    ) -> Result<Vec<FuturesCrossOrder>, Error> {
        self.require_network(network)?;
        self.request_json(
            Method::POST,
            "/futures/cross/orders/cancel-all",
            "",
            Vec::new(),
            true,
        )
        .await
    }

    pub async fn cross_close_position(&self, network: Network) -> Result<FuturesCrossOrder, Error> {
        self.require_network(network)?;
        self.request_json(
            Method::POST,
            "/futures/cross/position/close",
            "",
            Vec::new(),
            true,
        )
        .await
    }

    pub async fn cross_set_leverage(
        &self,
        network: Network,
        leverage: FuturesLeverage,
    ) -> Result<FuturesCrossPosition, Error> {
        #[derive(Serialize)]
        struct Request {
            leverage: u8,
        }

        self.require_network(network)?;
        let body = serde_json::to_vec(&Request {
            leverage: leverage.get(),
        })
        .map_err(Error::Serialize)?;
        self.request_json(Method::PUT, "/futures/cross/leverage", "", body, true)
            .await
    }

    pub async fn cross_deposit(
        &self,
        network: Network,
        transfer: &FuturesCrossTransferRequest,
    ) -> Result<FuturesCrossPosition, Error> {
        self.cross_post(network, "/futures/cross/deposit", transfer)
            .await
    }

    pub async fn cross_withdraw(
        &self,
        network: Network,
        transfer: &FuturesCrossTransferRequest,
    ) -> Result<FuturesCrossPosition, Error> {
        self.cross_post(network, "/futures/cross/withdraw", transfer)
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

    pub async fn isolated_new_trade(
        &self,
        network: Network,
        trade: &FuturesIsolatedNewTradeRequest,
    ) -> Result<FuturesIsolatedTrade, Error> {
        self.require_network(network)?;
        self.request_json(
            Method::POST,
            "/futures/isolated/trade",
            "",
            trade.body()?,
            true,
        )
        .await
    }

    pub async fn isolated_close_trade(
        &self,
        network: Network,
        trade: &FuturesIsolatedTradeReference,
    ) -> Result<FuturesIsolatedTrade, Error> {
        self.isolated_trade_post(network, "/futures/isolated/trade/close", trade)
            .await
    }

    pub async fn isolated_cancel_trade(
        &self,
        network: Network,
        trade: &FuturesIsolatedTradeReference,
    ) -> Result<FuturesIsolatedTrade, Error> {
        self.isolated_trade_post(network, "/futures/isolated/trade/cancel", trade)
            .await
    }

    pub async fn isolated_cancel_all_trades(
        &self,
        network: Network,
    ) -> Result<Vec<FuturesIsolatedTrade>, Error> {
        self.require_network(network)?;
        self.request_json(
            Method::POST,
            "/futures/isolated/trades/cancel-all",
            "",
            Vec::new(),
            true,
        )
        .await
    }

    pub async fn isolated_add_margin(
        &self,
        network: Network,
        request: &FuturesIsolatedAmountRequest,
    ) -> Result<FuturesIsolatedTrade, Error> {
        self.isolated_trade_post(network, "/futures/isolated/trade/add-margin", request)
            .await
    }

    pub async fn isolated_cash_in(
        &self,
        network: Network,
        request: &FuturesIsolatedAmountRequest,
    ) -> Result<FuturesIsolatedTrade, Error> {
        self.isolated_trade_post(network, "/futures/isolated/trade/cash-in", request)
            .await
    }

    pub async fn isolated_update_stoploss(
        &self,
        network: Network,
        request: &FuturesIsolatedStoplossUpdate,
    ) -> Result<FuturesIsolatedTrade, Error> {
        self.require_network(network)?;
        let body = serde_json::to_vec(request).map_err(Error::Serialize)?;
        self.request_json(
            Method::PUT,
            "/futures/isolated/trade/stoploss",
            "",
            body,
            true,
        )
        .await
    }

    pub async fn isolated_update_takeprofit(
        &self,
        network: Network,
        request: &FuturesIsolatedTakeprofitUpdate,
    ) -> Result<FuturesIsolatedTrade, Error> {
        self.require_network(network)?;
        let body = serde_json::to_vec(request).map_err(Error::Serialize)?;
        self.request_json(
            Method::PUT,
            "/futures/isolated/trade/takeprofit",
            "",
            body,
            true,
        )
        .await
    }

    pub async fn isolated_remove_stoploss(
        &self,
        network: Network,
        trade: &FuturesIsolatedTradeReference,
    ) -> Result<FuturesIsolatedTrade, Error> {
        self.isolated_trade_delete(network, "/futures/isolated/trade/stoploss", trade)
            .await
    }

    pub async fn isolated_remove_takeprofit(
        &self,
        network: Network,
        trade: &FuturesIsolatedTradeReference,
    ) -> Result<FuturesIsolatedTrade, Error> {
        self.isolated_trade_delete(network, "/futures/isolated/trade/takeprofit", trade)
            .await
    }

    pub async fn new_swap(&self, swap: &NewSwapRequest) -> Result<NewSwapResult, Error> {
        let body = serde_json::to_vec(swap).map_err(Error::Serialize)?;
        self.request_json(Method::POST, "/synthetic-usd/swap", "", body, true)
            .await
    }

    pub async fn option_instruments(&self) -> Result<Vec<OptionInstrumentName>, Error> {
        self.get_v2_public("/options/instruments", "").await
    }

    pub async fn option_instrument(
        &self,
        instrument_name: &OptionInstrumentName,
    ) -> Result<OptionInstrument, Error> {
        #[derive(Serialize)]
        struct Query<'a> {
            instrument_name: &'a str,
        }

        let query = encoded_query(&Query {
            instrument_name: instrument_name.as_str(),
        })?;
        self.get_v2_public("/options/instrument", &query).await
    }

    pub async fn option_market(&self) -> Result<OptionMarket, Error> {
        self.get_v2_public("/options/market", "").await
    }

    pub async fn option_volatility_index(&self) -> Result<OptionVolatilityIndex, Error> {
        self.get_v2_public("/options/volatility-index", "").await
    }

    pub async fn option_trades(
        &self,
        query: &OptionTradesQuery,
    ) -> Result<Vec<OptionTrade>, Error> {
        let query = encoded_query(query)?;
        self.get_v2_authenticated("/options", &query).await
    }

    pub async fn option_trade(&self, id: OptionTradeId) -> Result<OptionTrade, Error> {
        self.get_v2_authenticated(&format!("/options/trades/{id}"), "")
            .await
    }

    pub async fn option_buy(
        &self,
        network: Network,
        request: &OptionBuyRequest,
    ) -> Result<OptionTrade, Error> {
        self.v2_post(network, "/options", request).await
    }

    pub async fn option_update_settlement(
        &self,
        network: Network,
        request: &OptionSettlementUpdate,
    ) -> Result<OptionTrade, Error> {
        self.require_network(network)?;
        let body = serde_json::to_vec(request).map_err(Error::Serialize)?;
        self.request_v2_json(Method::PUT, "/options", "", body, true)
            .await
    }

    pub async fn option_close(
        &self,
        network: Network,
        id: OptionTradeId,
    ) -> Result<OptionTrade, Error> {
        #[derive(Serialize)]
        struct Query {
            id: OptionTradeId,
        }

        self.require_network(network)?;
        let query = encoded_query(&Query { id })?;
        self.request_v2_json(Method::DELETE, "/options", &query, Vec::new(), true)
            .await
    }

    pub async fn option_close_all(&self, network: Network) -> Result<OptionCloseAllResult, Error> {
        self.require_network(network)?;
        self.request_v2_json(Method::DELETE, "/options/all/close", "", Vec::new(), true)
            .await
    }

    pub async fn legacy_deposit_synthetic_usd(
        &self,
        network: Network,
        request: &LegacySyntheticUsdRequest,
    ) -> Result<serde_json::Value, Error> {
        self.v2_post(network, "/user/deposit/susd", request).await
    }

    pub async fn legacy_withdraw_synthetic_usd(
        &self,
        network: Network,
        request: &LegacySyntheticUsdRequest,
    ) -> Result<serde_json::Value, Error> {
        self.v2_post(network, "/user/withdraw/susd", request).await
    }

    pub async fn legacy_internal_transfer(
        &self,
        network: Network,
        request: &LegacyInternalTransferRequest,
    ) -> Result<serde_json::Value, Error> {
        self.v2_post(network, "/user/transfer", request).await
    }

    pub async fn lnurl_auth_challenge(&self) -> Result<LnurlAuthChallenge, Error> {
        self.request_v2_json(Method::POST, "/lnurl/auth", "", Vec::new(), false)
            .await
    }

    pub async fn lnurl_auth_callback(
        &self,
        callback: &LnurlAuthCallback,
    ) -> Result<LnurlAuthToken, Error> {
        let query = encoded_query(callback)?;
        self.get_v2_public("/lnurl/auth", &query).await
    }

    async fn isolated_trade_post<T: Serialize>(
        &self,
        network: Network,
        path: &str,
        request: &T,
    ) -> Result<FuturesIsolatedTrade, Error> {
        self.require_network(network)?;
        let body = serde_json::to_vec(request).map_err(Error::Serialize)?;
        self.request_json(Method::POST, path, "", body, true).await
    }

    async fn cross_post<T: Serialize, R: DeserializeOwned>(
        &self,
        network: Network,
        path: &str,
        request: &T,
    ) -> Result<R, Error> {
        self.require_network(network)?;
        let body = serde_json::to_vec(request).map_err(Error::Serialize)?;
        self.request_json(Method::POST, path, "", body, true).await
    }

    async fn account_post<T: Serialize, R: DeserializeOwned>(
        &self,
        network: Network,
        path: &str,
        request: &T,
    ) -> Result<R, Error> {
        self.require_network(network)?;
        let body = serde_json::to_vec(request).map_err(Error::Serialize)?;
        self.request_json(Method::POST, path, "", body, true).await
    }

    async fn v2_post<T: Serialize, R: DeserializeOwned>(
        &self,
        network: Network,
        path: &str,
        request: &T,
    ) -> Result<R, Error> {
        self.require_network(network)?;
        let body = serde_json::to_vec(request).map_err(Error::Serialize)?;
        self.request_v2_json(Method::POST, path, "", body, true)
            .await
    }

    async fn isolated_trade_delete<T: Serialize>(
        &self,
        network: Network,
        path: &str,
        request: &T,
    ) -> Result<FuturesIsolatedTrade, Error> {
        self.require_network(network)?;
        let query = encoded_query(request)?;
        self.request_json(Method::DELETE, path, &query, Vec::new(), true)
            .await
    }

    fn require_network(&self, requested: Network) -> Result<(), Error> {
        if requested == self.network {
            Ok(())
        } else {
            Err(Error::NetworkMismatch {
                requested,
                configured: self.network,
            })
        }
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

    async fn get_v2_public<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &str,
    ) -> Result<T, Error> {
        self.request_v2_json(Method::GET, path, query, Vec::new(), false)
            .await
    }

    async fn get_v2_authenticated<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &str,
    ) -> Result<T, Error> {
        self.request_v2_json(Method::GET, path, query, Vec::new(), true)
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
        let response = self
            .request(method, path, query, body, authenticated)
            .await?;
        parse_success_response(response).await
    }

    async fn request_empty(
        &self,
        method: Method,
        path: &str,
        query: &str,
        body: Vec<u8>,
        authenticated: bool,
    ) -> Result<(), Error> {
        let response = self
            .request(method, path, query, body, authenticated)
            .await?;
        drain_response(response).await
    }

    async fn request_v2_json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &str,
        body: Vec<u8>,
        authenticated: bool,
    ) -> Result<T, Error> {
        let response = self
            .request_with_version(RestApiVersion::V2, method, path, query, body, authenticated)
            .await?;
        parse_success_response(response).await
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: &str,
        body: Vec<u8>,
        authenticated: bool,
    ) -> Result<Response<Vec<u8>>, Error> {
        self.request_with_version(RestApiVersion::V3, method, path, query, body, authenticated)
            .await
    }

    async fn request_with_version(
        &self,
        version: RestApiVersion,
        method: Method,
        path: &str,
        query: &str,
        body: Vec<u8>,
        authenticated: bool,
    ) -> Result<Response<Vec<u8>>, Error> {
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
            let request = build_versioned_request(
                version,
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
                        return Ok(response);
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

pub fn rest_signature_v2(
    secret: &str,
    timestamp: &str,
    method: &Method,
    path: &str,
    data: &str,
) -> Result<String, Error> {
    let payload = format!("{}{}{}{}", timestamp, method.as_str(), path, data);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestApiVersion {
    V2,
    V3,
}

fn build_versioned_request(
    version: RestApiVersion,
    network: Network,
    method: Method,
    path: &str,
    query: &str,
    body: &[u8],
    credentials: Option<&Credentials>,
) -> Result<Request<Vec<u8>>, Error> {
    let (base_url, canonical_path) = match version {
        RestApiVersion::V2 => (network.legacy_rest_api_url(), format!("/v2{path}")),
        RestApiVersion::V3 => (network.rest_api_url(), format!("/v3{path}")),
    };
    let uri = format!("{base_url}{path}{query}");
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
            match version {
                RestApiVersion::V2 => query.strip_prefix('?').unwrap_or(query),
                RestApiVersion::V3 => query,
            }
        } else {
            std::str::from_utf8(body).map_err(|_| Error::InvalidRequestBody)?
        };
        let signature = match version {
            RestApiVersion::V2 => rest_signature_v2(
                credentials.secret(),
                &timestamp,
                &method,
                &canonical_path,
                data,
            )?,
            RestApiVersion::V3 => rest_signature(
                credentials.secret(),
                &timestamp,
                &method,
                &canonical_path,
                data,
            )?,
        };
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

    const TRADE_ID: &str = "d0b9f9a0-4f6e-4a5a-8b7a-7f0f5f9a8a1e";

    fn isolated_trade_body(state: FuturesIsolatedTradeState) -> String {
        let (open, running, closed, canceled) = match state {
            FuturesIsolatedTradeState::Open => (true, false, false, false),
            FuturesIsolatedTradeState::Running => (false, true, false, false),
            FuturesIsolatedTradeState::Closed => (false, false, true, false),
            FuturesIsolatedTradeState::Canceled => (false, false, false, true),
        };
        format!(
            r#"{{"id":"{TRADE_ID}","type":"market","side":"buy","openingFee":0,"closingFee":0,"maintenanceMargin":100,"quantity":1,"margin":1000,"leverage":10,"price":100000,"liquidation":90000,"stoploss":0,"stoplossTrailingDistance":0.1,"takeprofit":0,"exitPrice":null,"pl":0,"createdAt":"2026-01-01T00:00:00.000Z","filledAt":"2026-01-01T00:00:01.000Z","closedAt":null,"entryPrice":100000,"entryMargin":1000,"open":{open},"running":{running},"canceled":{canceled},"closed":{closed},"sumFundingFees":0,"sumCashInPl":0,"sumCashInMargin":0,"clientId":null}}"#
        )
    }

    fn cross_order_body() -> String {
        format!(
            r#"{{"canceled":false,"canceledAt":null,"createdAt":"2026-01-01T00:00:00.000Z","filled":true,"filledAt":"2026-01-01T00:00:01.000Z","id":"{TRADE_ID}","open":false,"price":64000.5,"quantity":2,"side":"buy","tradingFee":1,"type":"market","clientId":"client-1"}}"#
        )
    }

    fn cross_position_body() -> String {
        format!(
            r#"{{"deltaPl":0,"entryPrice":64000.5,"fundingFees":0,"id":"{TRADE_ID}","initialMargin":1000,"leverage":25,"liquidation":60000,"maintenanceMargin":100,"margin":1000,"quantity":2,"runningMargin":1000,"totalPl":0,"tradingFees":1,"updatedAt":"2026-01-01T00:00:01.000Z"}}"#
        )
    }

    fn option_trade_body() -> String {
        r#"{"id":"49d4f418-5190-40b9-9c32-856381dc8aa2","uid":"c6c1a624-f2b4-48c9-b07a-7fd037770bd2","forward":29840,"forward_point":0,"domestic":"BTC","settlement":"cash","fixing_price":29840,"creation_ts":1689695082638,"expiry_ts":1689753600000,"closed_ts":1689695087302,"physical_delivery_id":null,"leg_id":"a6a05452-d445-4d08-a39a-c73126faa098","side":"b","type":"c","quantity":100,"strike":29000,"volatility":1.1670000553131104,"margin":12744,"pl":-2,"maintenance_margin":0,"opening_fee":172,"closing_fee":172,"running":false,"closed":true,"expired":false,"exercised":false}"#.into()
    }

    fn authenticated_client(http_transport: Arc<dyn HttpTransport>) -> LnMarketsClient {
        let credentials = Credentials::new("key", "secret", "passphrase").expect("credentials");
        LnMarketsClient::authenticated(http_transport, Network::Signet, credentials)
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
    fn v2_signature_vectors_use_uppercase_methods_and_bare_queries() {
        assert_eq!(
            rest_signature_v2(
                "test-secret",
                "1700000000000",
                &Method::GET,
                "/v2/options",
                "status=running&limit=25",
            )
            .expect("GET signature"),
            "s47KWgnjaEKf/CXWR+sxQU6FTJVASRAiKzySdl5C1ek="
        );
        assert_eq!(
            rest_signature_v2(
                "test-secret",
                "1700000000000",
                &Method::POST,
                "/v2/options",
                r#"{"side":"b","quantity":10,"settlement":"physical","instrument_name":"BTC.2026-01-05.43000.C"}"#,
            )
            .expect("POST signature"),
            "HrByDZaaqTeAphr/F/csaj3qh471y3vYOwb+Qevbz+s="
        );
        assert_ne!(
            rest_signature_v2(
                "test-secret",
                "1700000000000",
                &Method::GET,
                "/v2/options",
                "?status=running&limit=25",
            )
            .expect("signature with leading question mark"),
            "s47KWgnjaEKf/CXWR+sxQU6FTJVASRAiKzySdl5C1ek="
        );
    }

    #[test]
    fn option_inputs_and_models_follow_the_legacy_contract() {
        assert!(OptionInstrumentName::new("ETH.2026-01-05.43000.C").is_err());
        assert!(OptionInstrumentName::new("BTC.20260105.43000.C").is_err());
        assert!(OptionInstrumentName::new("BTC.2026-01-05.0.C").is_err());
        assert!(OptionInstrumentName::new("BTC.2026-01-05.43000.X").is_err());
        let instrument = OptionInstrumentName::new("BTC.2026-01-05.43000.C").expect("instrument");
        let request =
            OptionBuyRequest::new(instrument, 10, OptionSettlement::Physical).expect("buy");
        assert_eq!(
            serde_json::to_string(&request).expect("body"),
            r#"{"side":"b","quantity":10,"settlement":"physical","instrument_name":"BTC.2026-01-05.43000.C"}"#
        );
        assert!(
            OptionBuyRequest::new(
                OptionInstrumentName::new("BTC.2026-01-05.43000.P").expect("instrument"),
                0,
                OptionSettlement::Cash,
            )
            .is_err()
        );

        let trade: OptionTrade =
            serde_json::from_str(&option_trade_body()).expect("option trade fixture");
        assert_eq!(trade.side, OptionSide::Buy);
        assert_eq!(trade.kind, OptionKind::Call);
        assert_eq!(trade.settlement, OptionSettlement::Cash);
        assert_eq!(trade.pl, -2);

        let array: OptionCloseAllResult = serde_json::from_str("[]").expect("array result");
        let object: OptionCloseAllResult =
            serde_json::from_str(r#"{"trades":[]}"#).expect("object result");
        assert!(array.trades.is_empty());
        assert!(object.trades.is_empty());
    }

    #[test]
    fn v2_surfaces_use_documented_routes_and_authentication_shapes() {
        smol::block_on(async {
            #[derive(Debug)]
            struct RecordedRequest {
                method: Method,
                path: String,
                query: Option<String>,
                body: String,
                authenticated: bool,
            }

            let requests = Arc::new(StdMutex::new(Vec::new()));
            let transport = FakeTransport::create({
                let requests = requests.clone();
                move |request| {
                    let requests = requests.clone();
                    async move {
                        let path = request.uri().path().to_owned();
                        let method = request.method().clone();
                        requests.lock().expect("requests").push(RecordedRequest {
                            method: method.clone(),
                            path: path.clone(),
                            query: request.uri().query().map(str::to_owned),
                            body: String::from_utf8(request.body().clone()).expect("UTF-8 body"),
                            authenticated: request.headers().contains_key("LNM-ACCESS-SIGNATURE"),
                        });
                        match (method, path.as_str()) {
                            (Method::GET, "/v2/options/instruments") => {
                                response(200, r#"["BTC.2026-01-05.43000.C"]"#)
                            }
                            (Method::GET, "/v2/options/instrument") => {
                                response(200, r#"{"volatility":0.88}"#)
                            }
                            (Method::GET, "/v2/options/market") => response(
                                200,
                                r#"{"active":true,"limits":{"margin":{"min":0,"max":500000},"quantity":{"min":1,"max":200000},"count":{"max":50}},"fees":{"trading":0.0005}}"#,
                            ),
                            (Method::GET, "/v2/options/volatility-index") => {
                                response(200, r#"{"volatilityIndex":0.75}"#)
                            }
                            (Method::GET, "/v2/options") => response(200, "[]"),
                            (Method::DELETE, "/v2/options/all/close") => {
                                response(200, r#"{"trades":[]}"#)
                            }
                            (Method::POST, "/v2/lnurl/auth") => {
                                response(200, r#"{"lnurl":"LNURL1TEST"}"#)
                            }
                            (Method::GET, "/v2/lnurl/auth") => {
                                response(200, r#"{"token":"opaque-token"}"#)
                            }
                            (Method::POST, path) if path.starts_with("/v2/user/") => {
                                response(200, "{}")
                            }
                            (_, "/v2/options")
                            | (
                                Method::GET,
                                "/v2/options/trades/49d4f418-5190-40b9-9c32-856381dc8aa2",
                            ) => response(200, &option_trade_body()),
                            _ => response(404, r#"{"message":"unexpected route"}"#),
                        }
                    }
                }
            });
            let client = authenticated_client(transport);
            let instrument =
                OptionInstrumentName::new("BTC.2026-01-05.43000.C").expect("instrument");
            client.option_instruments().await.expect("instruments");
            client
                .option_instrument(&instrument)
                .await
                .expect("instrument data");
            client.option_market().await.expect("market");
            client
                .option_volatility_index()
                .await
                .expect("volatility index");
            client
                .option_trades(
                    &OptionTradesQuery::new(OptionTradeStatus::Running)
                        .with_limit(25)
                        .expect("limit"),
                )
                .await
                .expect("trades");
            let trade_id = || {
                "49d4f418-5190-40b9-9c32-856381dc8aa2"
                    .parse()
                    .expect("option trade ID")
            };
            client.option_trade(trade_id()).await.expect("trade");
            client
                .option_buy(
                    Network::Signet,
                    &OptionBuyRequest::new(instrument, 10, OptionSettlement::Physical)
                        .expect("buy"),
                )
                .await
                .expect("buy option");
            client
                .option_update_settlement(
                    Network::Signet,
                    &OptionSettlementUpdate::new(trade_id(), OptionSettlement::Cash),
                )
                .await
                .expect("update settlement");
            client
                .option_close(Network::Signet, trade_id())
                .await
                .expect("close option");
            client
                .option_close_all(Network::Signet)
                .await
                .expect("close all options");
            let legacy_amount = LegacySyntheticUsdRequest::new("10.50".parse().expect("amount"));
            client
                .legacy_deposit_synthetic_usd(Network::Signet, &legacy_amount)
                .await
                .expect("deposit sUSD");
            client
                .legacy_withdraw_synthetic_usd(Network::Signet, &legacy_amount)
                .await
                .expect("withdraw sUSD");
            client
                .legacy_internal_transfer(
                    Network::Signet,
                    &LegacyInternalTransferRequest::new(
                        Uuid::parse_str("c6c1a624-f2b4-48c9-b07a-7fd037770bd2").expect("recipient"),
                        1_000,
                    )
                    .expect("transfer"),
                )
                .await
                .expect("internal transfer");
            client
                .lnurl_auth_challenge()
                .await
                .expect("LNURL challenge");
            client
                .lnurl_auth_callback(
                    &LnurlAuthCallback::new(
                        "00".repeat(32),
                        "11".repeat(32),
                        "22".repeat(70),
                        format!("02{}", "33".repeat(32)),
                        true,
                    )
                    .expect("callback"),
                )
                .await
                .expect("LNURL callback");

            let requests = requests.lock().expect("requests");
            assert_eq!(requests.len(), 15);
            assert_eq!(requests[0].path, "/v2/options/instruments");
            assert!(!requests[0].authenticated);
            assert_eq!(
                requests[1].query.as_deref(),
                Some("instrument_name=BTC.2026-01-05.43000.C")
            );
            assert_eq!(
                requests[4].query.as_deref(),
                Some("status=running&limit=25")
            );
            assert!(requests[4].authenticated);
            assert_eq!(requests[6].method, Method::POST);
            assert_eq!(requests[6].path, "/v2/options");
            assert_eq!(
                requests[6].body,
                r#"{"side":"b","quantity":10,"settlement":"physical","instrument_name":"BTC.2026-01-05.43000.C"}"#
            );
            assert_eq!(requests[8].method, Method::DELETE);
            assert_eq!(
                requests[8].query.as_deref(),
                Some("id=49d4f418-5190-40b9-9c32-856381dc8aa2")
            );
            assert_eq!(requests[9].path, "/v2/options/all/close");
            assert_eq!(requests[13].method, Method::POST);
            assert_eq!(requests[13].path, "/v2/lnurl/auth");
            assert!(!requests[13].authenticated);
            assert_eq!(requests[14].method, Method::GET);
            assert!(
                requests[14].query.as_deref().is_some_and(
                    |query| query.contains("tag=login") && query.contains("token=true")
                )
            );
        });
    }

    #[test]
    fn option_buy_is_single_attempt_and_network_mismatch_sends_nothing() {
        smol::block_on(async {
            let request_count = Arc::new(StdMutex::new(0));
            let transport = FakeTransport::create({
                let request_count = request_count.clone();
                move |_| {
                    let request_count = request_count.clone();
                    async move {
                        *request_count.lock().expect("request count") += 1;
                        response(503, r#"{"message":"Service unavailable"}"#)
                    }
                }
            });
            let client = authenticated_client(transport);
            let request = OptionBuyRequest::new(
                OptionInstrumentName::new("BTC.2026-01-05.43000.C").expect("instrument"),
                10,
                OptionSettlement::Cash,
            )
            .expect("buy request");
            let mismatch = client
                .option_buy(Network::Mainnet, &request)
                .await
                .expect_err("network mismatch");
            assert!(matches!(mismatch, Error::NetworkMismatch { .. }));
            assert_eq!(*request_count.lock().expect("request count"), 0);

            let unavailable = client
                .option_buy(Network::Signet, &request)
                .await
                .expect_err("503");
            assert!(matches!(unavailable, Error::Api { status, .. } if status.as_u16() == 503));
            assert_eq!(*request_count.lock().expect("request count"), 1);
        });
    }

    #[test]
    fn isolated_trade_inputs_enforce_sdk_constraints() {
        assert!(matches!(
            FuturesLeverage::new(0),
            Err(Error::InvalidLeverage)
        ));
        assert!(matches!(
            FuturesLeverage::new(101),
            Err(Error::InvalidLeverage)
        ));
        assert_eq!(FuturesLeverage::new(100).expect("leverage").get(), 100);
        assert!(FuturesIsolatedTradeSize::margin_sats(0).is_err());
        assert!(FuturesIsolatedTradeSize::quantity_usd(0).is_err());

        let leverage = FuturesLeverage::new(10).expect("leverage");
        let margin = FuturesIsolatedTradeSize::margin_sats(1_000).expect("margin");
        let market =
            FuturesIsolatedNewTradeRequest::market(leverage, FuturesTradeSide::Buy, margin);
        assert_eq!(
            std::str::from_utf8(&market.body().expect("body")).expect("UTF-8"),
            r#"{"leverage":10,"margin":1000,"side":"buy","type":"market"}"#
        );

        let quantity = FuturesIsolatedTradeSize::quantity_usd(25).expect("quantity");
        let limit = FuturesIsolatedNewTradeRequest::limit(
            leverage,
            FuturesTradeSide::Sell,
            quantity,
            "64000.5".parse().expect("price"),
        )
        .expect("limit order");
        assert_eq!(
            std::str::from_utf8(&limit.body().expect("body")).expect("UTF-8"),
            r#"{"leverage":10,"price":64000.5,"quantity":25,"side":"sell","type":"limit"}"#
        );

        let trade_id = TRADE_ID.parse().expect("trade ID");
        assert!(FuturesIsolatedAmountRequest::new(trade_id, 0).is_err());
        let trade_id = TRADE_ID.parse().expect("trade ID");
        assert!(
            FuturesIsolatedStoplossUpdate::trailing(trade_id, "0.0009".parse().expect("distance"))
                .is_err()
        );
    }

    #[test]
    fn isolated_trade_state_decodes_sdk_boolean_combinations() {
        for expected in [
            FuturesIsolatedTradeState::Open,
            FuturesIsolatedTradeState::Running,
            FuturesIsolatedTradeState::Closed,
            FuturesIsolatedTradeState::Canceled,
        ] {
            let trade: FuturesIsolatedTrade =
                serde_json::from_str(&isolated_trade_body(expected)).expect("trade");
            assert_eq!(trade.state, expected);
        }

        let running_and_open = isolated_trade_body(FuturesIsolatedTradeState::Running)
            .replace(r#""open":false"#, r#""open":true"#);
        let trade: FuturesIsolatedTrade =
            serde_json::from_str(&running_and_open).expect("running trade can remain open");
        assert_eq!(trade.state, FuturesIsolatedTradeState::Running);

        let invalid = isolated_trade_body(FuturesIsolatedTradeState::Open)
            .replace(r#""open":true"#, r#""open":false"#);
        assert!(serde_json::from_str::<FuturesIsolatedTrade>(&invalid).is_err());
    }

    #[test]
    fn isolated_mutation_signature_vectors_cover_post_put_and_delete() {
        assert_eq!(
            rest_signature(
                "test-secret",
                "1700000000000",
                &Method::POST,
                "/v3/futures/isolated/trade",
                r#"{"leverage":10,"margin":1000,"side":"buy","type":"market","clientId":"client-1"}"#,
            )
            .expect("POST signature"),
            "ZDDA6BKkHfwR6T/70eTfMc4EPfGR1RcK4qBfCjOa65s="
        );
        assert_eq!(
            rest_signature(
                "test-secret",
                "1700000000000",
                &Method::PUT,
                "/v3/futures/isolated/trade/stoploss",
                r#"{"id":"d0b9f9a0-4f6e-4a5a-8b7a-7f0f5f9a8a1e","value":0.1,"mode":"trailing"}"#,
            )
            .expect("PUT signature"),
            "vj3wVlzktwWdi3+d/pAFtWjzaQAHDhlAuNPx+VUoVOM="
        );
        assert_eq!(
            rest_signature(
                "test-secret",
                "1700000000000",
                &Method::DELETE,
                "/v3/futures/isolated/trade/stoploss",
                "?id=d0b9f9a0-4f6e-4a5a-8b7a-7f0f5f9a8a1e",
            )
            .expect("DELETE signature"),
            "0IFIBtrTHD68kqd+bcilXY1HVPrqu8/U1Lz5zhHYgYM="
        );
    }

    #[test]
    fn isolated_mutations_use_documented_method_shapes() {
        smol::block_on(async {
            let requests = Arc::new(StdMutex::new(Vec::new()));
            let transport = FakeTransport::create({
                let requests = requests.clone();
                move |request| {
                    let requests = requests.clone();
                    async move {
                        requests.lock().expect("requests").push((
                            request.method().clone(),
                            request.uri().path().to_owned(),
                            request.uri().query().map(str::to_owned),
                            String::from_utf8(request.body().clone()).expect("UTF-8 body"),
                        ));
                        if request.uri().path().ends_with("cancel-all") {
                            response(200, "[]")
                        } else {
                            response(
                                200,
                                &isolated_trade_body(FuturesIsolatedTradeState::Running),
                            )
                        }
                    }
                }
            });
            let client = authenticated_client(transport);
            let trade_id = || TRADE_ID.parse().expect("trade ID");
            let reference = || FuturesIsolatedTradeReference::new(trade_id());
            let order = FuturesIsolatedNewTradeRequest::market(
                FuturesLeverage::new(10).expect("leverage"),
                FuturesTradeSide::Buy,
                FuturesIsolatedTradeSize::margin_sats(1_000).expect("margin"),
            )
            .with_client_id("client-1");
            client
                .isolated_new_trade(Network::Signet, &order)
                .await
                .expect("new trade");
            client
                .isolated_close_trade(Network::Signet, &reference())
                .await
                .expect("close");
            client
                .isolated_cancel_trade(Network::Signet, &reference())
                .await
                .expect("cancel");
            client
                .isolated_cancel_all_trades(Network::Signet)
                .await
                .expect("cancel all");
            client
                .isolated_add_margin(
                    Network::Signet,
                    &FuturesIsolatedAmountRequest::new(trade_id(), 1_000).expect("amount"),
                )
                .await
                .expect("add margin");
            client
                .isolated_cash_in(
                    Network::Signet,
                    &FuturesIsolatedAmountRequest::new(trade_id(), 500).expect("amount"),
                )
                .await
                .expect("cash in");
            client
                .isolated_update_stoploss(
                    Network::Signet,
                    &FuturesIsolatedStoplossUpdate::trailing(
                        trade_id(),
                        "0.1".parse().expect("distance"),
                    )
                    .expect("trailing stop"),
                )
                .await
                .expect("update stoploss");
            client
                .isolated_update_takeprofit(
                    Network::Signet,
                    &FuturesIsolatedTakeprofitUpdate::new(
                        trade_id(),
                        "70000".parse().expect("takeprofit"),
                    )
                    .expect("takeprofit"),
                )
                .await
                .expect("update takeprofit");
            client
                .isolated_remove_stoploss(Network::Signet, &reference())
                .await
                .expect("remove stoploss");
            client
                .isolated_remove_takeprofit(Network::Signet, &reference())
                .await
                .expect("remove takeprofit");

            let requests = requests.lock().expect("requests");
            assert_eq!(requests.len(), 10);
            assert_eq!(
                requests
                    .iter()
                    .map(|(method, path, _, _)| (method, path.as_str()))
                    .collect::<Vec<_>>(),
                vec![
                    (&Method::POST, "/v3/futures/isolated/trade"),
                    (&Method::POST, "/v3/futures/isolated/trade/close"),
                    (&Method::POST, "/v3/futures/isolated/trade/cancel"),
                    (&Method::POST, "/v3/futures/isolated/trades/cancel-all"),
                    (&Method::POST, "/v3/futures/isolated/trade/add-margin"),
                    (&Method::POST, "/v3/futures/isolated/trade/cash-in"),
                    (&Method::PUT, "/v3/futures/isolated/trade/stoploss"),
                    (&Method::PUT, "/v3/futures/isolated/trade/takeprofit"),
                    (&Method::DELETE, "/v3/futures/isolated/trade/stoploss"),
                    (&Method::DELETE, "/v3/futures/isolated/trade/takeprofit"),
                ]
            );
            assert_eq!(
                requests[8].2.as_deref(),
                Some("id=d0b9f9a0-4f6e-4a5a-8b7a-7f0f5f9a8a1e")
            );
            assert!(requests[8].3.is_empty());
        });
    }

    #[test]
    fn isolated_post_is_single_attempt_and_network_mismatch_sends_nothing() {
        smol::block_on(async {
            let requests = Arc::new(StdMutex::new(0));
            let transport = FakeTransport::create({
                let requests = requests.clone();
                move |_| {
                    let requests = requests.clone();
                    async move {
                        *requests.lock().expect("request count") += 1;
                        response(503, r#"{"message":"Service unavailable"}"#)
                    }
                }
            });
            let client = authenticated_client(transport);
            let order = FuturesIsolatedNewTradeRequest::market(
                FuturesLeverage::new(10).expect("leverage"),
                FuturesTradeSide::Buy,
                FuturesIsolatedTradeSize::margin_sats(1_000).expect("margin"),
            );
            let mismatch = client
                .isolated_new_trade(Network::Mainnet, &order)
                .await
                .expect_err("network mismatch");
            assert!(matches!(mismatch, Error::NetworkMismatch { .. }));
            assert_eq!(*requests.lock().expect("request count"), 0);

            let unavailable = client
                .isolated_new_trade(Network::Signet, &order)
                .await
                .expect_err("503");
            assert!(matches!(unavailable, Error::Api { status, .. } if status.as_u16() == 503));
            assert_eq!(*requests.lock().expect("request count"), 1);
        });
    }

    #[test]
    fn cross_margin_inputs_encode_quantity_and_price_constraints() {
        assert!(FuturesCrossOrderQuantity::new(0).is_err());
        assert!(FuturesCrossTransferRequest::new(0).is_err());
        assert!("0".parse::<DecimalAmount>().is_err());
        assert!(FuturesCrossLimitPrice::new("64000.25".parse().expect("price")).is_err());

        let quantity = FuturesCrossOrderQuantity::new(2).expect("quantity");
        let market =
            FuturesCrossNewOrderRequest::market(FuturesTradeSide::Buy, quantity, "client-1");
        assert_eq!(
            std::str::from_utf8(&market.body().expect("body")).expect("UTF-8"),
            r#"{"side":"buy","quantity":2,"type":"market","clientId":"client-1"}"#
        );

        let limit = FuturesCrossNewOrderRequest::limit(
            FuturesTradeSide::Buy,
            quantity,
            FuturesCrossLimitPrice::new("64000.5".parse().expect("price")).expect("tick"),
            "client-1",
        );
        assert_eq!(
            std::str::from_utf8(&limit.body().expect("body")).expect("UTF-8"),
            r#"{"side":"buy","quantity":2,"type":"limit","price":64000.5,"clientId":"client-1"}"#
        );
    }

    #[test]
    fn cross_margin_signature_vectors_cover_post_and_put_bodies() {
        assert_eq!(
            rest_signature(
                "test-secret",
                "1700000000000",
                &Method::POST,
                "/v3/futures/cross/order",
                r#"{"side":"buy","quantity":2,"type":"limit","price":64000.5,"clientId":"client-1"}"#,
            )
            .expect("POST signature"),
            "6eoYpyU6hRWJ5mebpuxh4d/Hx9++27toVFQaTHVAmOM="
        );
        assert_eq!(
            rest_signature(
                "test-secret",
                "1700000000000",
                &Method::PUT,
                "/v3/futures/cross/leverage",
                r#"{"leverage":25}"#,
            )
            .expect("PUT signature"),
            "wtyVqDOxnBZN/Qhd2OPgs0extyzSzigKxM83LKKMkGQ="
        );
    }

    #[test]
    fn cross_margin_mutations_use_documented_method_shapes() {
        smol::block_on(async {
            let requests = Arc::new(StdMutex::new(Vec::new()));
            let transport = FakeTransport::create({
                let requests = requests.clone();
                move |request| {
                    let requests = requests.clone();
                    async move {
                        let path = request.uri().path().to_owned();
                        requests.lock().expect("requests").push((
                            request.method().clone(),
                            path.clone(),
                            String::from_utf8(request.body().clone()).expect("UTF-8 body"),
                        ));
                        if path.ends_with("cancel-all") {
                            response(200, "[]")
                        } else if path.ends_with("leverage")
                            || path.ends_with("deposit")
                            || path.ends_with("withdraw")
                        {
                            response(200, &cross_position_body())
                        } else {
                            response(200, &cross_order_body())
                        }
                    }
                }
            });
            let client = authenticated_client(transport);
            let quantity = FuturesCrossOrderQuantity::new(2).expect("quantity");
            let order =
                FuturesCrossNewOrderRequest::market(FuturesTradeSide::Buy, quantity, "client-1");
            client
                .cross_new_order(Network::Signet, &order)
                .await
                .expect("new order");
            client
                .cross_cancel_order(
                    Network::Signet,
                    &FuturesCrossCancelOrderRequest::new(TRADE_ID.parse().expect("order ID")),
                )
                .await
                .expect("cancel order");
            client
                .cross_cancel_all_orders(Network::Signet)
                .await
                .expect("cancel all");
            client
                .cross_close_position(Network::Signet)
                .await
                .expect("close position");
            client
                .cross_set_leverage(Network::Signet, FuturesLeverage::new(25).expect("leverage"))
                .await
                .expect("set leverage");
            let transfer = FuturesCrossTransferRequest::new(1_000).expect("transfer");
            client
                .cross_deposit(Network::Signet, &transfer)
                .await
                .expect("deposit");
            client
                .cross_withdraw(Network::Signet, &transfer)
                .await
                .expect("withdraw");

            let requests = requests.lock().expect("requests");
            assert_eq!(
                requests
                    .iter()
                    .map(|(method, path, _)| (method, path.as_str()))
                    .collect::<Vec<_>>(),
                vec![
                    (&Method::POST, "/v3/futures/cross/order"),
                    (&Method::POST, "/v3/futures/cross/order/cancel"),
                    (&Method::POST, "/v3/futures/cross/orders/cancel-all"),
                    (&Method::POST, "/v3/futures/cross/position/close"),
                    (&Method::PUT, "/v3/futures/cross/leverage"),
                    (&Method::POST, "/v3/futures/cross/deposit"),
                    (&Method::POST, "/v3/futures/cross/withdraw"),
                ]
            );
            assert_eq!(requests[1].2, format!(r#"{{"id":"{TRADE_ID}"}}"#));
            assert_eq!(requests[4].2, r#"{"leverage":25}"#);
            assert_eq!(requests[5].2, r#"{"amount":1000}"#);
        });
    }

    #[test]
    fn cross_margin_post_is_single_attempt_and_network_mismatch_sends_nothing() {
        smol::block_on(async {
            let requests = Arc::new(StdMutex::new(0));
            let transport = FakeTransport::create({
                let requests = requests.clone();
                move |_| {
                    let requests = requests.clone();
                    async move {
                        *requests.lock().expect("request count") += 1;
                        response(503, r#"{"message":"Service unavailable"}"#)
                    }
                }
            });
            let client = authenticated_client(transport);
            let order = FuturesCrossNewOrderRequest::market(
                FuturesTradeSide::Buy,
                FuturesCrossOrderQuantity::new(2).expect("quantity"),
                "client-1",
            );
            let mismatch = client
                .cross_new_order(Network::Mainnet, &order)
                .await
                .expect_err("network mismatch");
            assert!(matches!(mismatch, Error::NetworkMismatch { .. }));
            assert_eq!(*requests.lock().expect("request count"), 0);

            let unavailable = client
                .cross_new_order(Network::Signet, &order)
                .await
                .expect_err("503");
            assert!(matches!(unavailable, Error::Api { status, .. } if status.as_u16() == 503));
            assert_eq!(*requests.lock().expect("request count"), 1);
        });
    }

    #[test]
    fn wallet_inputs_validate_amounts_hashes_invoices_and_addresses() {
        assert!(LightningDepositRequest::new(0).is_err());
        assert!(DescriptionHash::new("a".repeat(63)).is_err());
        assert!(DescriptionHash::new("A".repeat(64)).is_err());
        assert!(LightningInvoice::new("not-an-invoice").is_err());
        assert!(LightningInvoice::new("lntbs1 invoice").is_err());
        assert!(BitcoinAddressValue::new(" ").is_err());

        let request = LightningDepositRequest::new(1_000)
            .expect("deposit")
            .with_comment("test")
            .with_description_hash(DescriptionHash::new("a".repeat(64)).expect("hash"));
        assert_eq!(
            serde_json::to_string(&request).expect("deposit JSON"),
            format!(
                r#"{{"amount":1000,"comment":"test","descriptionHash":"{}"}}"#,
                "a".repeat(64)
            )
        );
    }

    #[test]
    fn wallet_values_are_redacted_from_debug_output() {
        let invoice_text = "lntbs1invoicecontents";
        let address_text = "tb1qaddresscontents";
        let invoice = LightningInvoice::new(invoice_text).expect("invoice");
        let address = BitcoinAddressValue::new(address_text).expect("address");
        let invoice_request = LightningWithdrawalRequest::new(invoice.clone());
        let address_request = OnChainWithdrawalRequest::new(
            address.clone(),
            "1000".parse().expect("withdrawal amount"),
        );
        let bitcoin_address = BitcoinAddress {
            address: address.clone(),
        };

        for debug_output in [
            format!("{invoice:?}"),
            format!("{address:?}"),
            format!("{invoice_request:?}"),
            format!("{address_request:?}"),
            format!("{bitcoin_address:?}"),
        ] {
            assert!(!debug_output.contains(invoice_text));
            assert!(!debug_output.contains(address_text));
            assert!(debug_output.contains("REDACTED"));
        }
        assert_eq!(invoice.expose(), invoice_text);
        assert_eq!(address.expose(), address_text);
    }

    #[test]
    fn wallet_mutation_signature_vectors_cover_post_and_put_bodies() {
        assert_eq!(
            rest_signature(
                "test-secret",
                "1700000000000",
                &Method::POST,
                "/v3/account/deposit/lightning",
                r#"{"amount":1000,"comment":"test","descriptionHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
            )
            .expect("deposit signature"),
            "a92uKrIluqou/aiohpHtfvfebYD+LrBcxYmG4JQqdEo="
        );
        assert_eq!(
            rest_signature(
                "test-secret",
                "1700000000000",
                &Method::POST,
                "/v3/account/withdraw/lightning",
                r#"{"invoice":"lntbs1exampleinvoice"}"#,
            )
            .expect("withdrawal signature"),
            "LVoGBGkoOLLhNPiWts35TMefk2UKzBV/SKbA/ZHGPro="
        );
        assert_eq!(
            rest_signature(
                "test-secret",
                "1700000000000",
                &Method::PUT,
                "/v3/account/notifications",
                "",
            )
            .expect("notifications signature"),
            "9vHdFYyotpYMCckHFwiBIvLVRl5HpLJ5RJUuORhjJ+U="
        );
    }

    #[test]
    fn wallet_mutations_use_documented_method_shapes() {
        smol::block_on(async {
            let requests = Arc::new(StdMutex::new(Vec::new()));
            let transport = FakeTransport::create({
                let requests = requests.clone();
                move |request| {
                    let requests = requests.clone();
                    async move {
                        let path = request.uri().path().to_owned();
                        requests.lock().expect("requests").push((
                            request.method().clone(),
                            path.clone(),
                            String::from_utf8(request.body().clone()).expect("UTF-8 body"),
                        ));
                        match path.as_str() {
                            "/v3/account/deposit/lightning" => response(
                                200,
                                r#"{"depositId":"deposit-1","paymentRequest":"lntbs1depositinvoice"}"#,
                            ),
                            "/v3/account/withdraw/lightning" => response(
                                200,
                                r#"{"amount":1000,"id":"withdrawal-1","maxFees":10,"paymentHash":"hash-1"}"#,
                            ),
                            "/v3/account/withdraw/on-chain" => response(
                                200,
                                r#"{"address":"tb1qdestination","amount":2000,"createdAt":"2026-01-01T00:00:00Z","fee":null,"id":"withdrawal-2","status":"pending","txId":null,"uid":"user-1","updatedAt":"2026-01-01T00:00:00Z"}"#,
                            ),
                            "/v3/account/address/bitcoin" => response(
                                200,
                                r#"{"address":"tb1qdeposit","createdAt":"2026-01-01T00:00:00Z"}"#,
                            ),
                            "/v3/account/notifications" => response(204, ""),
                            _ => response(404, r#"{"message":"unexpected path"}"#),
                        }
                    }
                }
            });
            let client = authenticated_client(transport);
            client
                .deposit_lightning(
                    Network::Signet,
                    &LightningDepositRequest::new(1_000)
                        .expect("deposit")
                        .with_comment("test"),
                )
                .await
                .expect("deposit invoice");
            client
                .withdraw_lightning(
                    Network::Signet,
                    &LightningWithdrawalRequest::new(
                        LightningInvoice::new("lntbs1withdrawinvoice").expect("invoice"),
                    ),
                )
                .await
                .expect("Lightning withdrawal");
            client
                .withdraw_on_chain(
                    Network::Signet,
                    &OnChainWithdrawalRequest::new(
                        BitcoinAddressValue::new("tb1qdestination").expect("address"),
                        "2000".parse().expect("amount"),
                    ),
                )
                .await
                .expect("on-chain withdrawal");
            client
                .add_bitcoin_address(
                    Network::Signet,
                    &BitcoinAddressRequest::new(BitcoinAddressFormat::P2wpkh),
                )
                .await
                .expect("Bitcoin address");
            client
                .mark_notifications_read(Network::Signet)
                .await
                .expect("mark notifications read");

            let requests = requests.lock().expect("requests");
            assert_eq!(
                requests
                    .iter()
                    .map(|(method, path, _)| (method, path.as_str()))
                    .collect::<Vec<_>>(),
                vec![
                    (&Method::POST, "/v3/account/deposit/lightning"),
                    (&Method::POST, "/v3/account/withdraw/lightning"),
                    (&Method::POST, "/v3/account/withdraw/on-chain"),
                    (&Method::POST, "/v3/account/address/bitcoin"),
                    (&Method::PUT, "/v3/account/notifications"),
                ]
            );
            assert_eq!(requests[0].2, r#"{"amount":1000,"comment":"test"}"#);
            assert_eq!(requests[1].2, r#"{"invoice":"lntbs1withdrawinvoice"}"#);
            assert_eq!(
                requests[2].2,
                r#"{"address":"tb1qdestination","amount":2000}"#
            );
            assert_eq!(requests[3].2, r#"{"format":"p2wpkh"}"#);
            assert!(requests[4].2.is_empty());
        });
    }

    #[test]
    fn wallet_post_is_single_attempt_and_network_mismatch_sends_nothing() {
        smol::block_on(async {
            let requests = Arc::new(StdMutex::new(0));
            let transport = FakeTransport::create({
                let requests = requests.clone();
                move |_| {
                    let requests = requests.clone();
                    async move {
                        *requests.lock().expect("request count") += 1;
                        response(503, r#"{"message":"Service unavailable"}"#)
                    }
                }
            });
            let client = authenticated_client(transport);
            let request = LightningWithdrawalRequest::new(
                LightningInvoice::new("lntbs1withdrawinvoice").expect("invoice"),
            );
            let mismatch = client
                .withdraw_lightning(Network::Mainnet, &request)
                .await
                .expect_err("network mismatch");
            assert!(matches!(mismatch, Error::NetworkMismatch { .. }));
            assert_eq!(*requests.lock().expect("request count"), 0);

            let unavailable = client
                .withdraw_lightning(Network::Signet, &request)
                .await
                .expect_err("503");
            assert!(matches!(unavailable, Error::Api { status, .. } if status.as_u16() == 503));
            assert_eq!(*requests.lock().expect("request count"), 1);
        });
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
