//! Supervise packaged `omega-effectd` over newline-framed JSON stdio.
//!
//! Durable Full Auto run truth lives on disk under the injected data root.
//! This supervisor owns process life, health, restart, and generation fencing.
//! It must never become a second durable run authority (GPUI must not rewrite
//! runs after restart).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{anyhow, bail, Context as _, Result};
use futures::io::{AsyncBufReadExt as _, BufReader};
use futures::{AsyncWriteExt as _, StreamExt as _};
use serde_json::{json, Value};
use smol::process::ChildStdin;
use util::process::Child;
use util::redact::redact_command;

use crate::protocol::{
    request_frame, HealthResult, InitializeResult, ProtocolErrorCode, ResponseFrame, RunSnapshot,
    PROTOCOL_SCHEMA,
};

#[derive(Debug, Clone)]
pub struct OmegaEffectdCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct OmegaEffectdSupervisorOptions {
    pub data_root: PathBuf,
    pub command: OmegaEffectdCommand,
    /// Initial generation. Each successful `restart` increments by one.
    pub initial_generation: u64,
    pub request_timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
    #[error("stale generation")]
    StaleGeneration,
    #[error("protocol error ({code:?}): {message}")]
    Protocol {
        code: ProtocolErrorCode,
        message: String,
    },
}

pub struct OmegaEffectdSupervisor {
    options: OmegaEffectdSupervisorOptions,
    generation: AtomicU64,
    next_request_id: AtomicU64,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout_lines: Option<futures::io::Lines<BufReader<smol::process::ChildStdout>>>,
}

impl OmegaEffectdSupervisor {
    pub fn new(options: OmegaEffectdSupervisorOptions) -> Self {
        let generation = options.initial_generation.max(1);
        Self {
            options,
            generation: AtomicU64::new(generation),
            next_request_id: AtomicU64::new(1),
            child: None,
            stdin: None,
            stdout_lines: None,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn data_root(&self) -> &Path {
        &self.options.data_root
    }

    pub async fn start(&mut self) -> Result<InitializeResult> {
        if self.child.is_some() {
            bail!("omega-effectd is already running");
        }
        self.spawn_child().await?;
        let generation = self.generation();
        let result = self
            .request("initialize", Some(json!({ "generation": generation })), generation)
            .await?;
        serde_json::from_value(result).context("decode initialize result")
    }

    pub async fn health(&mut self) -> Result<HealthResult, SupervisorError> {
        let result = self.request("health", None, self.generation()).await?;
        Ok(serde_json::from_value(result).context("decode health result")?)
    }

    pub async fn list_runs(&mut self) -> Result<Vec<RunSnapshot>, SupervisorError> {
        let result = self.request("list_runs", None, self.generation()).await?;
        let runs = result
            .get("runs")
            .cloned()
            .ok_or_else(|| anyhow!("list_runs missing runs"))?;
        Ok(serde_json::from_value(runs).context("decode list_runs")?)
    }

    pub async fn get_run(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "get_run",
                Some(json!({ "runRef": run_ref })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("run")
            .cloned()
            .ok_or_else(|| anyhow!("get_run missing run"))?)
    }

    pub async fn start_run(&mut self, params: Value) -> Result<Value, SupervisorError> {
        let result = self.request("start", Some(params), self.generation()).await?;
        Ok(result
            .get("run")
            .cloned()
            .ok_or_else(|| anyhow!("start missing run"))?)
    }

    pub async fn pause_run(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        self.mutate_run("pause", run_ref).await
    }

    pub async fn resume_run(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        self.mutate_run("resume", run_ref).await
    }

    pub async fn stop_run(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        self.mutate_run("stop", run_ref).await
    }

    pub async fn retry_run(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        self.mutate_run("retry", run_ref).await
    }

    pub async fn get_capacity(&mut self) -> Result<Value, SupervisorError> {
        self.request("get_capacity", None, self.generation()).await
    }

    pub async fn decide_attention(
        &mut self,
        run_ref: &str,
        permission_granted: bool,
    ) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "decide_attention",
                Some(json!({
                    "runRef": run_ref,
                    "permissionGranted": permission_granted,
                })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("attention")
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub async fn get_report(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "get_report",
                Some(json!({ "runRef": run_ref })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("report")
            .cloned()
            .ok_or_else(|| anyhow!("get_report missing report"))?)
    }

    pub async fn get_receipt(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "get_receipt",
                Some(json!({ "runRef": run_ref })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("receipt")
            .cloned()
            .ok_or_else(|| anyhow!("get_receipt missing receipt"))?)
    }

    pub async fn apply_control_intent(
        &mut self,
        intent_id: &str,
        run_ref: &str,
        action: &str,
    ) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                "apply_control_intent",
                Some(json!({
                    "intentId": intent_id,
                    "runRef": run_ref,
                    "action": action,
                })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("outcome")
            .cloned()
            .ok_or_else(|| anyhow!("apply_control_intent missing outcome"))?)
    }

    pub async fn get_sync_status(&mut self) -> Result<Value, SupervisorError> {
        self.request("get_sync_status", None, self.generation())
            .await
    }

    pub async fn publish_projection(&mut self, run_ref: &str) -> Result<Value, SupervisorError> {
        self.request(
            "publish_projection",
            Some(json!({ "runRef": run_ref })),
            self.generation(),
        )
        .await
    }

    async fn mutate_run(&mut self, method: &str, run_ref: &str) -> Result<Value, SupervisorError> {
        let result = self
            .request(
                method,
                Some(json!({ "runRef": run_ref })),
                self.generation(),
            )
            .await?;
        Ok(result
            .get("run")
            .cloned()
            .ok_or_else(|| anyhow!("{method} missing run"))?)
    }

    pub async fn restart(&mut self) -> Result<InitializeResult> {
        self.stop().await?;
        let next = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.generation.store(next, Ordering::SeqCst);
        self.start().await
    }

    pub async fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            self.stdin.take();
            self.stdout_lines.take();
            #[cfg(unix)]
            {
                let _ = unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
                smol::Timer::after(Duration::from_millis(150)).await;
            }
            let _ = child.kill();
            let _ = child.status().await;
        }
        Ok(())
    }

    async fn spawn_child(&mut self) -> Result<()> {
        std::fs::create_dir_all(&self.options.data_root)
            .with_context(|| format!("create data root {}", self.options.data_root.display()))?;

        let mut command = std::process::Command::new(&self.options.command.program);
        command.args(&self.options.command.args);
        command.env(
            "OPENAGENTS_OMEGA_EFFECTD_DATA_ROOT",
            &self.options.data_root,
        );

        let mut child = Child::spawn(command, Stdio::piped(), Stdio::piped(), Stdio::piped())
            .with_context(|| {
                format!(
                    "spawn omega-effectd {}",
                    redact_command(&format!("{:?}", self.options.command))
                )
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("omega-effectd stdin missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("omega-effectd stdout missing"))?;
        if let Some(stderr) = child.stderr.take() {
            smol::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Some(line) = lines.next().await {
                    if let Ok(line) = line {
                        let _ = redact_command(&line);
                    }
                }
            })
            .detach();
        }

        self.stdin = Some(stdin);
        self.stdout_lines = Some(BufReader::new(stdout).lines());
        self.child = Some(child);
        Ok(())
    }

    async fn request(
        &mut self,
        method: &str,
        params: Option<Value>,
        generation: u64,
    ) -> Result<Value, SupervisorError> {
        let id = self.next_request_id.fetch_add(1, Ordering::SeqCst).to_string();
        let frame = request_frame(id.clone(), generation, method, params);
        let line =
            serde_json::to_string(&frame).map_err(|error| SupervisorError::Anyhow(error.into()))?;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("omega-effectd not started"))?;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;
        stdin
            .flush()
            .await
            .map_err(|error| SupervisorError::Anyhow(error.into()))?;

        let timeout = self.options.request_timeout;
        let response = smol::future::or(
            async {
                loop {
                    let line = self
                        .stdout_lines
                        .as_mut()
                        .ok_or_else(|| anyhow!("omega-effectd stdout missing"))?
                        .next()
                        .await
                        .ok_or_else(|| anyhow!("omega-effectd closed stdout"))?
                        .map_err(|error| anyhow!(error))?;
                    let response: ResponseFrame = serde_json::from_str(&line)
                        .with_context(|| format!("decode response line: {line}"))?;
                    if response.schema != PROTOCOL_SCHEMA || response.kind != "response" {
                        continue;
                    }
                    if response.id != id {
                        continue;
                    }
                    return Ok::<ResponseFrame, anyhow::Error>(response);
                }
            },
            async {
                smol::Timer::after(timeout).await;
                Err(anyhow!("omega-effectd request timed out after {timeout:?}"))
            },
        )
        .await
        .map_err(SupervisorError::Anyhow)?;

        if !response.ok {
            let error = response.error.unwrap_or(crate::protocol::ProtocolError {
                code: ProtocolErrorCode::Internal,
                message: "request failed without error body".to_string(),
            });
            return Err(match error.code {
                ProtocolErrorCode::StaleGeneration => SupervisorError::StaleGeneration,
                code => SupervisorError::Protocol {
                    code,
                    message: error.message,
                },
            });
        }
        response
            .result
            .ok_or_else(|| SupervisorError::Anyhow(anyhow!("ok response missing result")))
    }
}

impl Drop for OmegaEffectdSupervisor {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

/// Shared test helper: fixture command that speaks the framed protocol.
pub fn fixture_command(fixture: &Path) -> OmegaEffectdCommand {
    let node = [
        std::env::var_os("NODE")
            .map(PathBuf::from)
            .filter(|path| path.exists()),
        which_node(),
        Some(PathBuf::from(
            "/Users/christopherdavid/.nvm/versions/node/v25.8.2/bin/node",
        ))
        .filter(|path| path.exists()),
        Some(PathBuf::from("/opt/homebrew/bin/node")).filter(|path| path.exists()),
        Some(PathBuf::from("/usr/local/bin/node")).filter(|path| path.exists()),
    ]
    .into_iter()
    .flatten()
    .next()
    .unwrap_or_else(|| PathBuf::from("node"));

    OmegaEffectdCommand {
        program: node,
        args: vec![fixture.display().to_string()],
    }
}

fn which_node() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("node");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    })
}

pub fn default_options(
    data_root: PathBuf,
    command: OmegaEffectdCommand,
) -> OmegaEffectdSupervisorOptions {
    OmegaEffectdSupervisorOptions {
        data_root,
        command,
        initial_generation: 1,
        request_timeout: Duration::from_secs(5),
    }
}
