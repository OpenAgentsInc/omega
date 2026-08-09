use std::{
    fmt, io,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use futures::{AsyncReadExt as _, lock::Mutex};
use hmac::{Hmac, Mac as _};
use http_client::{AsyncBody, HttpClient, Method, Request, Response, StatusCode};
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
    BuildRequest(#[source] http_client::http::Error),
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
    http_client: Arc<dyn HttpClient>,
    network: Network,
    credentials: Option<Credentials>,
    next_authenticated_request: Arc<Mutex<Instant>>,
    next_public_request: Arc<Mutex<Instant>>,
}

impl LnMarketsClient {
    pub fn public(http_client: Arc<dyn HttpClient>, network: Network) -> Self {
        let now = Instant::now();
        Self {
            http_client,
            network,
            credentials: None,
            next_authenticated_request: Arc::new(Mutex::new(now)),
            next_public_request: Arc::new(Mutex::new(now)),
        }
    }

    pub fn authenticated(
        http_client: Arc<dyn HttpClient>,
        network: Network,
        credentials: Credentials,
    ) -> Self {
        let mut client = Self::public(http_client, network);
        client.credentials = Some(credentials);
        client
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub async fn account(&self) -> Result<Account, Error> {
        self.get_authenticated("/account", "").await
    }

    pub async fn ticker(&self) -> Result<Ticker, Error> {
        self.get_public("/futures/ticker", "").await
    }

    pub async fn funding_settlements(
        &self,
        pagination: &Pagination,
    ) -> Result<Paginated<FundingSettlement>, Error> {
        let query = encoded_query(pagination)?;
        self.get_public("/futures/funding-settlements", &query)
            .await
    }

    pub async fn best_price(&self) -> Result<BestPrice, Error> {
        self.get_public("/synthetic-usd/best-price", "").await
    }

    pub async fn swaps(&self, pagination: &Pagination) -> Result<Paginated<Swap>, Error> {
        let query = encoded_query(pagination)?;
        self.get_authenticated("/synthetic-usd/swaps", &query).await
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
            match self.http_client.send(request).await {
                Ok(response) => {
                    if response.status().is_success() {
                        return parse_success_response(response).await;
                    }
                    if is_retryable_status(response.status()) && attempt + 1 < MAX_ATTEMPTS {
                        let delay = retry_delay(attempt, response.headers());
                        drain_response(response).await?;
                        async_io::Timer::at(Instant::now() + delay).await;
                        continue;
                    }
                    return Err(parse_error_response(response).await?);
                }
                Err(error) if is_connection_error(&error) && attempt + 1 < MAX_ATTEMPTS => {
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
) -> Result<Request<AsyncBody>, Error> {
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
    builder
        .body(AsyncBody::from(body.to_vec()))
        .map_err(Error::BuildRequest)
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
    mut response: Response<AsyncBody>,
) -> Result<T, Error> {
    let bytes = read_bounded_body(response.body_mut()).await?;
    serde_json::from_slice(&bytes).map_err(Error::Deserialize)
}

async fn parse_error_response(mut response: Response<AsyncBody>) -> Result<Error, Error> {
    let status = response.status();
    let bytes = read_bounded_body(response.body_mut()).await?;
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

async fn drain_response(mut response: Response<AsyncBody>) -> Result<(), Error> {
    read_bounded_body(response.body_mut()).await.map(|_| ())
}

async fn read_bounded_body(body: &mut AsyncBody) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    body.take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(Error::ReadResponse)?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    Ok(bytes)
}

fn is_retryable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 502 | 503 | 504)
}

fn retry_delay(attempt: usize, headers: &http_client::http::HeaderMap) -> Duration {
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
    use std::sync::{Arc, Mutex as StdMutex};

    use http_client::{FakeHttpClient, Response};

    use super::*;

    fn response(status: u16, body: &str) -> anyhow::Result<Response<AsyncBody>> {
        Ok(Response::builder()
            .status(status)
            .header("Content-Type", "application/json")
            .body(AsyncBody::from(body.as_bytes().to_vec()))?)
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
            let client = FakeHttpClient::create(|request| async move {
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
            let client = FakeHttpClient::create(|request| async move {
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
            let client = FakeHttpClient::create({
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
            let client = FakeHttpClient::create(|mut request| async move {
                let mut body = String::new();
                request
                    .body_mut()
                    .read_to_string(&mut body)
                    .await
                    .expect("body");
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
