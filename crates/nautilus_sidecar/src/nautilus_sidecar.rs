use std::io::{BufRead as _, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{App, Global, Subscription, TaskExt as _};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const CREDENTIAL_KEY: &str = "omega://nautilus/hyperliquid-testnet-private-key";
pub const ENABLE_ENVIRONMENT_VARIABLE: &str = "OMEGA_NAUTILUS_SIDECAR";
pub const NETWORK_ENVIRONMENT_VARIABLE: &str = "OMEGA_NAUTILUS_NETWORK";
const EVENT_PREFIX: &str = "OMEGA_NAUTILUS_EVENT ";
const EVENT_SCHEMA: &str = "omega.nautilus.lifecycle.v1";
const DEFAULT_RECONCILIATION_LOOKBACK_MINUTES: u16 = 60;
const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(15);
const MONITOR_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    Testnet,
}

impl Network {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "testnet" => Ok(Self::Testnet),
            "mainnet" => bail!("Nautilus mainnet is disabled; only testnet is permitted"),
            _ => bail!("unsupported Nautilus network {value:?}"),
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

pub struct NautilusSupervisor {
    config: NautilusConfig,
    private_key: PrivateKey,
    child: Option<Child>,
    events: Option<mpsc::Receiver<Result<LifecycleEvent>>>,
    generation: u64,
    last_health: Option<LifecycleEvent>,
}

impl NautilusSupervisor {
    pub fn new(config: NautilusConfig, private_key: PrivateKey) -> Result<Self> {
        if config.network != Network::Testnet {
            bail!("Nautilus mainnet is disabled; only testnet is permitted");
        }
        Ok(Self {
            config,
            private_key,
            child: None,
            events: None,
            generation: 0,
            last_health: None,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
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
            .env("HYPERLIQUID_TESTNET_PK", self.private_key.0.as_str())
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .stdin(Stdio::null())
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
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("Nautilus sidecar stderr is unavailable"))?;
        let (sender, receiver) = mpsc::channel();
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
                let Some(payload) = line.strip_prefix(EVENT_PREFIX) else {
                    continue;
                };
                let event =
                    serde_json::from_str(payload).context("decode Nautilus lifecycle event");
                if sender.send(event).is_err() {
                    return;
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
            event.validate(self.generation)?;
            if matches!(event, LifecycleEvent::Healthy { .. }) {
                self.last_health = Some(event.clone());
                return Ok(event);
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

struct NautilusLifecycle {
    _quit_subscription: Subscription,
}

impl Global for NautilusLifecycle {}

pub fn init(cx: &mut App) {
    let config = match NautilusConfig::from_process_environment() {
        Ok(Some(config)) => config,
        Ok(None) => return,
        Err(error) => {
            log::error!("Refusing Nautilus sidecar configuration: {error:#}");
            return;
        }
    };
    let supervisor = Arc::new(Mutex::new(None::<NautilusSupervisor>));
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
        _quit_subscription: quit_subscription,
    });
    let credentials = zed_credentials_provider::local_credentials(cx);
    let background_executor = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        let (_, private_key) = credentials
            .read_credentials(CREDENTIAL_KEY, cx)
            .await?
            .ok_or_else(|| anyhow!("Hyperliquid testnet credential is not configured"))?;
        let sidecar = NautilusSupervisor::new(config, PrivateKey::new(private_key)?)?;
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
        while !shutting_down.load(Ordering::SeqCst) {
            background_executor.timer(MONITOR_INTERVAL).await;
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
        }
        Ok::<(), anyhow::Error>(())
    })
    .detach_and_log_err(cx);
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn private_key() -> PrivateKey {
        PrivateKey::new(format!("0x{}", "1".repeat(64)).into_bytes()).expect("test key")
    }

    #[test]
    fn mainnet_is_hard_refused() {
        assert!(Network::parse("mainnet").is_err());
        assert!(Network::parse("testnet").is_ok());
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
            health_timeout: Duration::from_secs(2),
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
