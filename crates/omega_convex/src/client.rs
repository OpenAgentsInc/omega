use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use anyhow::{Context as _, Result};
use convex::{
    AuthenticationToken, ConvexClientBuilder, Value, WebSocketState, base_client::AuthTokenFetcher,
};
use futures::StreamExt as _;
use omega_effectd::OpenAgentsControllerTokenSource;
use tokio::sync::oneshot;

use crate::projection::{AttentionRow, decode_attention_rows};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    Connecting,
    Connected,
}

#[derive(Debug)]
pub enum SubscriptionEvent {
    Connection(ConnectionState),
    Snapshot(Vec<AttentionRow>),
    Failure(String),
}

pub struct SubscriptionWorker {
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl SubscriptionWorker {
    pub fn spawn(
        token_source: OpenAgentsControllerTokenSource,
    ) -> Result<(Self, Receiver<SubscriptionEvent>)> {
        let (event_tx, event_rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let handle = thread::Builder::new()
            .name("omega-convex-inbox".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        send_failure(&event_tx, format!("Convex runtime unavailable: {error}"));
                        return;
                    }
                };
                if let Err(error) =
                    runtime.block_on(run(token_source, event_tx.clone(), shutdown_rx))
                {
                    send_failure(&event_tx, format!("Convex subscription stopped: {error:#}"));
                }
            })
            .context("starting Omega Convex subscription thread")?;
        Ok((
            Self {
                shutdown: Some(shutdown_tx),
                thread: Some(handle),
            },
            event_rx,
        ))
    }
}

impl Drop for SubscriptionWorker {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _worker_already_stopped = shutdown.send(());
        }
        // Joining here would block GPUI while a network syscall unwinds. The
        // owned shutdown signal ends the worker; dropping the handle detaches
        // only that bounded teardown.
        let _detached_teardown = self.thread.take();
    }
}

async fn run(
    token_source: OpenAgentsControllerTokenSource,
    event_tx: Sender<SubscriptionEvent>,
    mut shutdown_rx: oneshot::Receiver<()>,
) -> Result<()> {
    let _tls_config = http_client_tls::tls_config();
    let bootstrap = token_source.fetch().await?;
    let (state_tx, mut state_rx) = tokio::sync::mpsc::channel(8);
    let mut client = ConvexClientBuilder::new(&bootstrap.convex_url)
        .with_client_id("omega-native-0.2")
        .with_on_state_change(state_tx)
        .build()
        .await
        .context("connecting the official Convex Rust client")?;

    let auth_source = token_source.clone();
    let fetcher: AuthTokenFetcher = Box::new(move |_force_refresh| {
        let auth_source = auth_source.clone();
        Box::pin(async move {
            let bootstrap = auth_source.fetch().await?;
            Ok(AuthenticationToken::User(bootstrap.token))
        })
    });
    client.set_auth_callback(Some(fetcher)).await;

    let mut args = BTreeMap::new();
    args.insert("limit".to_string(), Value::from(100.0));
    let mut subscription = client
        .subscribe("workShells:attentionInbox", args)
        .await
        .context("subscribing to workShells:attentionInbox")?;

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => return Ok(()),
            state = state_rx.recv() => {
                let Some(state) = state else { continue };
                let state = match state {
                    WebSocketState::Connecting => ConnectionState::Connecting,
                    WebSocketState::Connected => ConnectionState::Connected,
                };
                if event_tx.send(SubscriptionEvent::Connection(state)).is_err() {
                    return Ok(());
                }
            }
            result = subscription.next() => {
                let Some(result) = result else {
                    anyhow::bail!("Convex inbox subscription ended unexpectedly");
                };
                match decode_attention_rows(result) {
                    Ok(rows) => {
                        if event_tx.send(SubscriptionEvent::Snapshot(rows)).is_err() {
                            return Ok(());
                        }
                    }
                    Err(error) => {
                        send_failure(&event_tx, error.to_string());
                    }
                }
            }
        }
    }
}

fn send_failure(event_tx: &Sender<SubscriptionEvent>, message: String) {
    if event_tx.send(SubscriptionEvent::Failure(message)).is_err() {
        log::debug!("Omega Convex inbox closed before it could receive a failure");
    }
}
