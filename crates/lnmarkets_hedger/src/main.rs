use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use futures::{FutureExt as _, StreamExt as _};
use lnmarkets_client::{
    Credentials, HttpTransport, LnMarketsClient, TransportFailure, TransportFailureKind,
};
use lnmarkets_hedger::{Hedger, HedgerConfig, LnMarketsHedgeVenue, redactable_summary};
use serde::Deserialize;
use trading_ledger::LedgerStore;
use zeroize::Zeroize as _;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Parser)]
#[command(name = "lnmarkets-hedger")]
struct Arguments {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    ledger: PathBuf,
    #[arg(long, env = "LNMARKETS_HEDGER_CREDENTIALS_FILE")]
    credentials_file: PathBuf,
    #[arg(long, default_value_t = false)]
    once: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialDocument {
    access_key: String,
    secret: String,
    passphrase: String,
}

impl Drop for CredentialDocument {
    fn drop(&mut self) {
        self.access_key.zeroize();
        self.secret.zeroize();
        self.passphrase.zeroize();
    }
}

struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(20))
                .build()
                .context("could not create the hedger HTTP client")?,
        })
    }
}

impl HttpTransport for ReqwestTransport {
    fn send(
        &self,
        request: http::Request<Vec<u8>>,
    ) -> futures::future::BoxFuture<'static, Result<http::Response<Vec<u8>>, TransportFailure>>
    {
        let client = self.client.clone();
        async move {
            let (parts, body) = request.into_parts();
            let response = client
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .body(body)
                .send()
                .await
                .map_err(classify_send_failure)?;
            if response
                .content_length()
                .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
            {
                return Err(TransportFailure::new(
                    TransportFailureKind::Other,
                    anyhow::anyhow!("the LN Markets response is too large"),
                ));
            }
            let status = response.status();
            let version = response.version();
            let headers = response.headers().clone();
            let mut stream = response.bytes_stream();
            let mut response_body = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(classify_read_failure)?;
                if response_body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(TransportFailure::new(
                        TransportFailureKind::Other,
                        anyhow::anyhow!("the LN Markets response is too large"),
                    ));
                }
                response_body.extend_from_slice(&chunk);
            }
            let mut builder = http::Response::builder().status(status).version(version);
            let response_headers = builder.headers_mut().ok_or_else(|| {
                TransportFailure::new(
                    TransportFailureKind::Other,
                    anyhow::anyhow!("could not create the LN Markets response"),
                )
            })?;
            *response_headers = headers;
            builder.body(response_body).map_err(TransportFailure::from)
        }
        .boxed()
    }
}

fn classify_send_failure(error: reqwest::Error) -> TransportFailure {
    let kind = match (error.is_connect(), error.is_timeout()) {
        (true, true) => TransportFailureKind::ConnectTimeout,
        (true, false) => TransportFailureKind::Connect,
        (false, true) => TransportFailureKind::WriteTimeout,
        (false, false) => TransportFailureKind::Other,
    };
    TransportFailure::new(kind, error)
}

fn classify_read_failure(error: reqwest::Error) -> TransportFailure {
    let kind = if error.is_timeout() {
        TransportFailureKind::ReadTimeout
    } else {
        TransportFailureKind::Other
    };
    TransportFailure::new(kind, error)
}

fn read_config(path: &Path) -> Result<HedgerConfig> {
    let bytes = std::fs::read(path).context("could not read the hedger configuration")?;
    let config = serde_json::from_slice::<HedgerConfig>(&bytes)
        .context("could not decode the hedger configuration")?;
    config.validate()?;
    Ok(config)
}

fn read_credentials(path: &Path) -> Result<Credentials> {
    let mut bytes = std::fs::read(path).context("could not read the mounted venue credential")?;
    let decoded = serde_json::from_slice::<CredentialDocument>(&bytes);
    bytes.zeroize();
    let mut decoded = decoded.context("could not decode the mounted venue credential")?;
    let credentials = Credentials::new(
        std::mem::take(&mut decoded.access_key),
        std::mem::take(&mut decoded.secret),
        std::mem::take(&mut decoded.passphrase),
    )?;
    Ok(credentials)
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let config = read_config(&arguments.config)?;
    if config.network != lnmarkets_client::Network::Signet {
        bail!("the incubating provider hedger is restricted to Signet");
    }
    let credentials = read_credentials(&arguments.credentials_file)?;
    let transport: Arc<dyn HttpTransport> = Arc::new(ReqwestTransport::new()?);
    let client = LnMarketsClient::authenticated(transport, config.network, credentials);
    let venue = LnMarketsHedgeVenue::new(client)?;
    let ledger = LedgerStore::open(&arguments.ledger)?;
    let hedger = Hedger::new(config, venue, ledger)?;

    loop {
        let report = hedger
            .run_cycle(chrono::Utc::now().timestamp_millis())
            .await?;
        println!("{}", serde_json::to_string(&redactable_summary(&report))?);
        if arguments.once {
            return Ok(());
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(hedger.config().poll_interval_seconds)) => {}
            signal = tokio::signal::ctrl_c() => {
                signal.context("could not listen for the shutdown signal")?;
                return Ok(());
            }
        }
    }
}
