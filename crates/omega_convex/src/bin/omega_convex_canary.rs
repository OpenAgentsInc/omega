use std::{
    collections::BTreeMap,
    io::Read as _,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use anyhow::{Context as _, Result, anyhow};
use async_tungstenite::{
    accept_async,
    async_std::connect_async,
    tungstenite::{client::IntoClientRequest as _, http::HeaderValue},
};
use convex::{
    AuthenticationToken, ConvexClient, ConvexClientBuilder, FunctionResult, Value, WebSocketState,
    base_client::AuthTokenFetcher,
};
use futures::{FutureExt as _, StreamExt as _, select};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

const MAX_TOKEN_BYTES: u64 = 16 * 1024;

fn main() -> Result<()> {
    let mut token = String::new();
    std::io::stdin()
        .take(MAX_TOKEN_BYTES)
        .read_to_string(&mut token)
        .context("reading the canary JWT from stdin")?;
    let token = token.trim().to_string();
    if token.is_empty() {
        anyhow::bail!("a server-scoped Convex JWT is required on stdin");
    }
    let upstream = std::env::var("OMEGA_CONVEX_CANARY_URL")
        .unwrap_or_else(|_| "https://convex.openagents.com".to_string());
    let tenant = std::env::var("OMEGA_CONVEX_CANARY_TENANT")
        .context("OMEGA_CONVEX_CANARY_TENANT is required")?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("building the canary Tokio runtime")?;
    runtime.block_on(run(upstream, tenant, token))
}

async fn run(upstream: String, tenant: String, token: String) -> Result<()> {
    let _tls_config = http_client_tls::tls_config();
    let proxy = DropProxy::start(&upstream)?;
    let (state_tx, mut state_rx) = tokio::sync::mpsc::channel(16);
    let auth_fetches = Arc::new(AtomicUsize::new(0));
    let mut subscriber = ConvexClientBuilder::new(&proxy.local_url)
        .with_client_id("omega-native-canary-0.2")
        .with_on_state_change(state_tx)
        .build()
        .await?;
    let fetch_count = auth_fetches.clone();
    let subscription_token = token.clone();
    let fetcher: AuthTokenFetcher = Box::new(move |_force_refresh| {
        let token = subscription_token.clone();
        fetch_count.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(AuthenticationToken::User(token)) })
    });
    subscriber.set_auth_callback(Some(fetcher)).await;

    let mut inbox_args = BTreeMap::new();
    inbox_args.insert("limit".to_string(), Value::from(200.0));
    let mut subscription = subscriber
        .subscribe("workShells:attentionInbox", inbox_args)
        .await?;
    let initial = next_snapshot(&mut subscription, "initial subscription").await?;

    let mut mutator = ConvexClient::new(&upstream).await?;
    mutator.set_auth(Some(token)).await;
    let canary_id = format!("omega-convex-canary-{}", Uuid::new_v4());
    let create_id = format!("cmd-create-{canary_id}");
    let create = command_args(
        &tenant,
        &create_id,
        "work.shell.create",
        serde_json::json!({
            "aggregate": { "aggregateType": "issue", "aggregateId": canary_id },
            "status": "created-by-web-side-canary",
            "attentionState": "working"
        }),
        None,
    )?;
    ensure_admitted(
        mutator.mutation("workShells:create", create).await?,
        "create",
    )?;
    let created = wait_for_row(
        &mut subscription,
        &canary_id,
        Some("created-by-web-side-canary"),
    )
    .await?;

    while state_rx.try_recv().is_ok() {}
    proxy.drop_active_connection()?;
    let mut saw_connecting = false;
    let mut saw_reconnected = false;
    let reconnect_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < reconnect_deadline {
        let state = tokio::time::timeout(Duration::from_secs(5), state_rx.recv())
            .await
            .context("waiting for Convex reconnect state")?
            .ok_or_else(|| anyhow!("Convex state channel ended during reconnect"))?;
        match state {
            WebSocketState::Connecting => saw_connecting = true,
            WebSocketState::Connected if saw_connecting => {
                saw_reconnected = true;
                break;
            }
            WebSocketState::Connected => {}
        }
    }
    if !saw_connecting || !saw_reconnected {
        anyhow::bail!("official client did not report a complete reconnect cycle");
    }
    if auth_fetches.load(Ordering::SeqCst) < 2 {
        anyhow::bail!("official client did not refresh auth on reconnect");
    }

    let update_id = format!("cmd-update-{canary_id}");
    let update = command_args(
        &tenant,
        &update_id,
        "work.shell.update",
        serde_json::json!({
            "aggregate": { "aggregateType": "issue", "aggregateId": canary_id },
            "patch": { "status": "observed-after-reconnect", "attentionState": "ready" }
        }),
        Some(1),
    )?;
    ensure_admitted(
        mutator.mutation("workShells:update", update).await?,
        "update",
    )?;
    let updated = wait_for_row(
        &mut subscription,
        &canary_id,
        Some("observed-after-reconnect"),
    )
    .await?;

    println!(
        "{}",
        serde_json::json!({
            "protocol": "convex-rust-0.10.4",
            "initialRows": initial.len(),
            "aggregateId": canary_id,
            "createdGeneration": row_generation(&created),
            "updatedGeneration": row_generation(&updated),
            "sawConnecting": saw_connecting,
            "sawReconnected": saw_reconnected,
            "authFetches": auth_fetches.load(Ordering::SeqCst),
            "mutationEnvelope": "admitted-and-observed"
        })
    );
    Ok(())
}

fn command_args(
    tenant: &str,
    command_id: &str,
    capability: &str,
    payload: serde_json::Value,
    expected_generation: Option<u64>,
) -> Result<BTreeMap<String, Value>> {
    let fingerprint = format!("{:x}", Sha256::digest(payload.to_string().as_bytes()));
    let mut envelope = serde_json::json!({
        "commandId": command_id,
        "name": capability,
        "payloadFingerprint": fingerprint,
        "actorId": tenant,
        "targetWorkspaceId": tenant
    });
    if let Some(generation) = expected_generation {
        envelope["expectedGeneration"] = serde_json::json!(generation);
    }
    let authority_receipt = serde_json::json!({
        "receiptId": format!("ar_{command_id}"),
        "capability": capability,
        "decision": "allowed",
        "reason": "production Rust SDK compatibility canary",
        "principal": { "id": tenant, "workspaceId": tenant }
    });
    let mut object = payload
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("canary payload must be an object"))?;
    object.insert("envelope".to_string(), envelope);
    object.insert("authorityReceipt".to_string(), authority_receipt);
    match Value::try_from(serde_json::Value::Object(object))? {
        Value::Object(args) => Ok(args),
        _ => anyhow::bail!("canary command arguments were not an object"),
    }
}

fn ensure_admitted(result: FunctionResult, operation: &str) -> Result<()> {
    let value = match result {
        FunctionResult::Value(value) => value,
        FunctionResult::ErrorMessage(message) => {
            anyhow::bail!("{operation} mutation failed: {message}")
        }
        FunctionResult::ConvexError(error) => {
            anyhow::bail!("{operation} mutation failed: {}", error.message)
        }
    };
    let json: serde_json::Value = value.into();
    if json
        .pointer("/receipt/decision")
        .and_then(|value| value.as_str())
        != Some("admitted")
    {
        anyhow::bail!("{operation} mutation did not return an admitted receipt");
    }
    Ok(())
}

async fn next_snapshot(
    subscription: &mut convex::QuerySubscription,
    label: &str,
) -> Result<Vec<serde_json::Value>> {
    let result = tokio::time::timeout(Duration::from_secs(30), subscription.next())
        .await
        .with_context(|| format!("waiting for {label}"))?
        .ok_or_else(|| anyhow!("Convex subscription ended while waiting for {label}"))?;
    let value = match result {
        FunctionResult::Value(value) => value,
        FunctionResult::ErrorMessage(message) => anyhow::bail!("{label} failed: {message}"),
        FunctionResult::ConvexError(error) => anyhow::bail!("{label} failed: {}", error.message),
    };
    let json: serde_json::Value = value.into();
    serde_json::from_value(json).with_context(|| format!("decoding {label}"))
}

async fn wait_for_row(
    subscription: &mut convex::QuerySubscription,
    aggregate_id: &str,
    status: Option<&str>,
) -> Result<serde_json::Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for live row {aggregate_id}");
        }
        let rows = next_snapshot(subscription, "live inbox update").await?;
        if let Some(row) = rows.into_iter().find(|row| {
            row.get("aggregateId").and_then(|value| value.as_str()) == Some(aggregate_id)
                && status.is_none_or(|expected| {
                    row.get("status").and_then(|value| value.as_str()) == Some(expected)
                })
        }) {
            return Ok(row);
        }
    }
}

fn row_generation(row: &serde_json::Value) -> Option<f64> {
    row.get("generation").and_then(|value| value.as_f64())
}

struct DropProxy {
    local_url: String,
    drop_connection_tx: async_channel::Sender<()>,
    shutdown_tx: async_channel::Sender<()>,
    thread: Option<thread::JoinHandle<()>>,
}

impl DropProxy {
    fn start(deployment_url: &str) -> Result<Self> {
        let upstream_ws = deployment_url
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        let upstream_ws = format!("{}/api/sync", upstream_ws.trim_end_matches('/'));
        let (address_tx, address_rx) = mpsc::sync_channel(1);
        let (drop_connection_tx, drop_connection_rx) = async_channel::bounded(1);
        let (shutdown_tx, shutdown_rx) = async_channel::bounded(1);
        let handle = thread::Builder::new()
            .name("omega-convex-drop-proxy".to_string())
            .spawn(move || {
                if let Err(error) = async_std::task::block_on(proxy_loop(
                    upstream_ws,
                    address_tx,
                    drop_connection_rx,
                    shutdown_rx,
                )) {
                    eprintln!("Convex drop proxy stopped: {error:#}");
                }
            })
            .context("starting the Convex drop proxy")?;
        let address = address_rx
            .recv_timeout(Duration::from_secs(5))
            .context("waiting for the Convex drop proxy")?;
        Ok(Self {
            local_url: format!("http://{address}"),
            drop_connection_tx,
            shutdown_tx,
            thread: Some(handle),
        })
    }

    fn drop_active_connection(&self) -> Result<()> {
        self.drop_connection_tx
            .send_blocking(())
            .context("requesting a forced WebSocket drop")
    }
}

impl Drop for DropProxy {
    fn drop(&mut self) {
        let _proxy_already_stopped = self.shutdown_tx.send_blocking(());
        let _detached_teardown = self.thread.take();
    }
}

async fn proxy_loop(
    upstream_ws: String,
    address_tx: mpsc::SyncSender<std::net::SocketAddr>,
    drop_connection_rx: async_channel::Receiver<()>,
    shutdown_rx: async_channel::Receiver<()>,
) -> Result<()> {
    let listener = async_std::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    address_tx
        .send(listener.local_addr()?)
        .context("publishing the Convex drop proxy address")?;
    loop {
        let accept = listener.accept().fuse();
        let shutdown = shutdown_rx.recv().fuse();
        futures::pin_mut!(accept, shutdown);
        let stream = select! {
            accepted = accept => accepted?.0,
            _ = shutdown => return Ok(()),
        };
        let client = accept_async(stream).await?;
        let mut request = upstream_ws.clone().into_client_request()?;
        request.headers_mut().insert(
            "Convex-Client",
            HeaderValue::from_static("omega-native-canary-proxy-0.2"),
        );
        let (upstream, _) = connect_async(request).await?;
        let (mut client_write, mut client_read) = client.split();
        let (mut upstream_write, mut upstream_read) = upstream.split();
        let client_to_upstream = async {
            while let Some(message) = client_read.next().await {
                upstream_write.send(message?).await?;
            }
            Result::<()>::Ok(())
        }
        .fuse();
        let upstream_to_client = async {
            while let Some(message) = upstream_read.next().await {
                client_write.send(message?).await?;
            }
            Result::<()>::Ok(())
        }
        .fuse();
        let forced_drop = drop_connection_rx.recv().fuse();
        let shutdown = shutdown_rx.recv().fuse();
        futures::pin_mut!(
            client_to_upstream,
            upstream_to_client,
            forced_drop,
            shutdown
        );
        select! {
            result = client_to_upstream => result?,
            result = upstream_to_client => result?,
            _ = forced_drop => {},
            _ = shutdown => return Ok(()),
        }
    }
}
