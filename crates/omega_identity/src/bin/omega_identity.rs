//! Operator CLI for Omega Nostr identity custody.
//!
//! Ready-state UI only offers Protect, not Reset. Use this tool to inspect
//! and clear channel custody through the typed IdentityService path.

use std::{path::PathBuf, process::ExitCode, str::FromStr};

use anyhow::{Context as _, Result, bail};
use app_identity::AppChannel;
use clap::{Parser, Subcommand};
use omega_identity::{CustodyState, IdentityRef, IdentityService, ReceiptRef};
use serde_json::json;

#[derive(Parser, Debug)]
#[command(name = "omega-identity", about = "Inspect and reset Omega Nostr identity custody")]
struct Args {
    /// Release channel that owns the identity (`dev`, `nightly`, `rc`, `stable`).
    #[arg(long, default_value = "rc")]
    channel: String,

    /// Optional override for the channel data root (parent of `identity/`).
    #[arg(long)]
    data_root: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print custody state and public identity refs as JSON.
    Status,
    /// Record a marker-first reset (relaunch-required). Requires --yes.
    Reset {
        /// Confirm destructive reset of the current Ready identity.
        #[arg(long)]
        yes: bool,
        /// Optional authorization receipt; generated when omitted.
        #[arg(long)]
        receipt: Option<String>,
        /// Optional expected identity_ref; defaults to the inspected Ready identity.
        #[arg(long)]
        identity_ref: Option<String>,
    },
    /// Resume a pending or failed reset (deletes Keychain + public documents).
    Resume,
    /// Acknowledge a completed reset after relaunch proof (Absent).
    Acknowledge,
    /// Reset, resume, and acknowledge in one operator flow. Requires --yes.
    Wipe {
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        receipt: Option<String>,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let channel = AppChannel::from_str(&args.channel)
        .map_err(|_| anyhow::anyhow!("invalid --channel {}; expected dev|nightly|rc|stable", args.channel))?;
    let service = match args.data_root {
        Some(root) => IdentityService::for_channel_data_root(channel, root),
        None => IdentityService::for_channel(channel),
    };

    match args.command {
        Command::Status => {
            let inspection = service
                .inspect_details()
                .context("inspect identity custody")?;
            println!("{}", serde_json::to_string_pretty(&inspection)?);
        }
        Command::Reset {
            yes,
            receipt,
            identity_ref,
        } => {
            if !yes {
                bail!("refusing reset without --yes");
            }
            let result = reset_identity(&service, receipt, identity_ref)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Resume => {
            let result = service
                .resume_pending_reset()
                .context("resume pending identity reset")?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Acknowledge => {
            let result = service
                .acknowledge_relaunch()
                .context("acknowledge completed identity reset")?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Wipe { yes, receipt } => {
            if !yes {
                bail!("refusing wipe without --yes");
            }
            let reset = reset_identity(&service, receipt, None)?;
            let resumed = match service.resume_pending_reset() {
                Ok(result) => result,
                Err(_) => {
                    // inspect() also resumes a pending marker
                    service.inspect().context("inspect after reset marker")?
                }
            };
            let acknowledged = if resumed.state == CustodyState::RelaunchRequired {
                service
                    .acknowledge_relaunch()
                    .context("acknowledge completed identity reset")?
            } else if resumed.state == CustodyState::Absent {
                resumed.clone()
            } else {
                bail!(
                    "reset did not reach relaunch-required or absent; state={:?}",
                    resumed.state
                );
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "channel": channel.storage_slug(),
                    "reset": reset,
                    "resumed": resumed,
                    "acknowledged": acknowledged,
                }))?
            );
        }
    }
    Ok(())
}

fn reset_identity(
    service: &IdentityService,
    receipt: Option<String>,
    identity_ref: Option<String>,
) -> Result<omega_identity::CustodyResult> {
    let inspection = service
        .inspect_details()
        .context("inspect identity before reset")?;
    let expected = match identity_ref {
        Some(value) => IdentityRef::new(value).context("parse --identity-ref")?,
        None => {
            let identity = inspection
                .custody
                .identity
                .as_ref()
                .context("no Ready identity to reset; run status first")?;
            IdentityRef::new(identity.identity_ref().as_str()).context("copy identity_ref")?
        }
    };
    let authorization = match receipt {
        Some(value) => ReceiptRef::new(value).context("parse --receipt")?,
        None => {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .context("system clock")?
                .as_millis();
            ReceiptRef::new(format!("omega-identity-cli-reset-{stamp}"))
                .context("build reset receipt")?
        }
    };
    service
        .reset(&expected, authorization)
        .context("record identity reset marker")
}
