use std::{io::Read as _, path::PathBuf, process::ExitCode};

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use omega_identity::{
    AdmittedSigningRequest, IDENTITY_PROOF_KEYRING_ACCOUNT, IDENTITY_PROOF_KEYRING_SERVICE,
    IDENTITY_PROOF_PROTOCOL, IdentityProofService, IdentityRef, ProofCrashBoundary,
    ProofSafeScenario, ReceiptRef, RecoveryPassword, SigningPurpose, UnsignedEventTemplate,
};
use serde::Serialize;
use serde_json::json;

#[derive(Parser, Debug)]
#[command(
    name = "omega-identity-proof",
    about = "Run explicit disposable Omega identity custody proofs"
)]
struct Args {
    #[arg(long)]
    root: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Initialize {
        #[arg(long)]
        confirm_disposable: bool,
    },
    Create {
        #[arg(long)]
        receipt: String,
        #[arg(long, value_enum)]
        crash_after: Option<CrashBoundary>,
    },
    ResumeCreate,
    Inspect,
    ProcessStart {
        #[arg(long, value_enum)]
        crash_after: Option<CrashBoundary>,
    },
    Sign {
        #[arg(long)]
        identity_ref: String,
        #[arg(long)]
        request: String,
    },
    ProbeForged {
        #[arg(long)]
        request: String,
    },
    ProbeStale {
        #[arg(long)]
        stale_identity_ref: String,
        #[arg(long)]
        request: String,
    },
    SimulateSafe {
        #[arg(long, value_enum)]
        scenario: SafeScenario,
    },
    ProtectRecovery {
        #[arg(long)]
        identity_ref: String,
        #[arg(long)]
        password_fd: i32,
    },
    Recover {
        #[arg(long)]
        receipt: String,
        #[arg(long)]
        password_fd: i32,
    },
    ProbeWrongRecovery {
        #[arg(long)]
        password_fd: i32,
    },
    ProbeCorruptRecovery {
        #[arg(long)]
        password_fd: i32,
    },
    Reset {
        #[arg(long)]
        identity_ref: String,
        #[arg(long)]
        receipt: String,
        #[arg(long, value_enum)]
        crash_after: Option<CrashBoundary>,
    },
    ResumeReset {
        #[arg(long, value_enum)]
        crash_after: Option<CrashBoundary>,
    },
    Safety,
}

#[derive(Debug, Copy, Clone, ValueEnum)]
enum CrashBoundary {
    SecretWrite,
    SecretReadBack,
    ManifestCommit,
    ResetMarker,
    ResetCommit,
    RelaunchAcknowledge,
}

#[derive(Debug, Copy, Clone, ValueEnum)]
enum SafeScenario {
    ConflictCustody,
    LostCustody,
    LockedCustody,
    SymlinkRefusal,
    WeakPermissionRefusal,
    KeychainUnavailable,
    CorruptKeychain,
    MalformedEventRejection,
    UnadmittedPurposeRejection,
    ConflictingRecoverySelection,
    LateCompletionFencing,
    SignerCrashBeforeCompletion,
}

impl From<SafeScenario> for ProofSafeScenario {
    fn from(value: SafeScenario) -> Self {
        match value {
            SafeScenario::ConflictCustody => Self::ConflictCustody,
            SafeScenario::LostCustody => Self::LostCustody,
            SafeScenario::LockedCustody => Self::LockedCustody,
            SafeScenario::SymlinkRefusal => Self::SymlinkRefusal,
            SafeScenario::WeakPermissionRefusal => Self::WeakPermissionRefusal,
            SafeScenario::KeychainUnavailable => Self::KeychainUnavailable,
            SafeScenario::CorruptKeychain => Self::CorruptKeychain,
            SafeScenario::MalformedEventRejection => Self::MalformedEventRejection,
            SafeScenario::UnadmittedPurposeRejection => Self::UnadmittedPurposeRejection,
            SafeScenario::ConflictingRecoverySelection => Self::ConflictingRecoverySelection,
            SafeScenario::LateCompletionFencing => Self::LateCompletionFencing,
            SafeScenario::SignerCrashBeforeCompletion => Self::SignerCrashBeforeCompletion,
        }
    }
}

impl From<CrashBoundary> for ProofCrashBoundary {
    fn from(value: CrashBoundary) -> Self {
        match value {
            CrashBoundary::SecretWrite => Self::AfterSecretWrite,
            CrashBoundary::SecretReadBack => Self::AfterSecretReadBack,
            CrashBoundary::ManifestCommit => Self::AfterManifestCommit,
            CrashBoundary::ResetMarker => Self::AfterResetMarker,
            CrashBoundary::ResetCommit => Self::AfterResetCommit,
            CrashBoundary::RelaunchAcknowledge => Self::AfterRelaunchAcknowledge,
        }
    }
}

#[derive(Serialize)]
struct Outcome<'a, T: Serialize> {
    protocol: &'static str,
    outcome: &'a str,
    facts: T,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let failure = Outcome {
                protocol: IDENTITY_PROOF_PROTOCOL,
                outcome: "rejected",
                facts: json!({ "error": format!("{error:#}") }),
            };
            eprintln!(
                "{}",
                serde_json::to_string(&failure).unwrap_or_else(|_| {
                    "{\"protocol\":\"openagents.omega.identity-proof.v1\",\"outcome\":\"rejected\"}".to_string()
                })
            );
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    if let Command::Initialize { confirm_disposable } = &args.command {
        if !confirm_disposable {
            bail!("initialization requires --confirm-disposable");
        }
        IdentityProofService::initialize(&args.root).context("initialize disposable proof root")?;
        return print_outcome(
            "initialized",
            json!({
                "root": args.root,
                "keyring_service": IDENTITY_PROOF_KEYRING_SERVICE,
                "keyring_account": IDENTITY_PROOF_KEYRING_ACCOUNT,
            }),
        );
    }

    let crash_boundary = command_crash_boundary(&args.command);
    let service = IdentityProofService::open(args.root, crash_boundary)
        .context("open disposable proof root")?;
    match args.command {
        Command::Initialize { .. } => bail!("initialize command was not handled"),
        Command::Create { receipt, .. } => {
            let result = service.create(ReceiptRef::new(receipt)?)?;
            print_outcome("create-complete", result)
        }
        Command::ResumeCreate => print_outcome("create-resumed", service.resume_create()?),
        Command::Inspect => print_outcome("inspection", service.inspect()?),
        Command::ProcessStart { .. } => {
            print_outcome("process-start", service.inspect_for_process_start()?)
        }
        Command::Sign {
            identity_ref,
            request,
        } => {
            let signing_request = signing_request(IdentityRef::new(identity_ref)?, request)?;
            print_outcome("signed", service.sign(&signing_request)?)
        }
        Command::ProbeForged { request } => {
            let forged = IdentityRef::new("omega-nostr-forged-request")?;
            rejected_signing_outcome(&service, forged, request, "forged-request-rejected")
        }
        Command::ProbeStale {
            stale_identity_ref,
            request,
        } => rejected_signing_outcome(
            &service,
            IdentityRef::new(stale_identity_ref)?,
            request,
            "stale-request-rejected",
        ),
        Command::SimulateSafe { scenario } => print_outcome(
            "safe-scenario-simulated",
            service.simulate_safe_scenario(scenario.into()),
        ),
        Command::ProtectRecovery {
            identity_ref,
            password_fd,
        } => print_outcome(
            "recovery-protected",
            service.protect_recovery(
                &IdentityRef::new(identity_ref)?,
                read_recovery_password(password_fd)?,
            )?,
        ),
        Command::Recover {
            receipt,
            password_fd,
        } => print_outcome(
            "recovery-complete",
            service.recover(
                read_recovery_password(password_fd)?,
                ReceiptRef::new(receipt)?,
            )?,
        ),
        Command::ProbeWrongRecovery { password_fd } => print_outcome(
            "wrong-recovery-password-rejected",
            service.probe_wrong_recovery_password(read_recovery_password(password_fd)?)?,
        ),
        Command::ProbeCorruptRecovery { password_fd } => print_outcome(
            "corrupt-recovery-artifact-rejected",
            service.probe_corrupt_recovery_artifact(read_recovery_password(password_fd)?)?,
        ),
        Command::Reset {
            identity_ref,
            receipt,
            ..
        } => print_outcome(
            "reset-marked",
            service.reset(&IdentityRef::new(identity_ref)?, ReceiptRef::new(receipt)?)?,
        ),
        Command::ResumeReset { .. } => print_outcome("reset-resumed", service.resume_reset()?),
        Command::Safety => print_outcome(
            "safety-checked",
            json!({
                "keyring_service": IDENTITY_PROOF_KEYRING_SERVICE,
                "keyring_account": IDENTITY_PROOF_KEYRING_ACCOUNT,
                "production_locator_access": "rejected-by-construction",
            }),
        ),
    }
}

fn read_recovery_password(file_descriptor: i32) -> Result<RecoveryPassword> {
    if file_descriptor < 3 {
        bail!("recovery password requires a protected file descriptor");
    }
    #[cfg(unix)]
    {
        use std::fs::File;
        use std::os::fd::FromRawFd as _;
        use zeroize::Zeroizing;

        let mut file = unsafe { File::from_raw_fd(file_descriptor) };
        let mut bytes = Zeroizing::new(Vec::with_capacity(1_025));
        file.by_ref().take(1_025).read_to_end(&mut bytes)?;
        if bytes.len() > 1_024 {
            bail!("recovery password exceeds the proof-driver limit");
        }
        let mut password = Zeroizing::new(
            String::from_utf8(std::mem::take(&mut *bytes))
                .map_err(|_| anyhow::anyhow!("recovery password is not UTF-8"))?,
        );
        return RecoveryPassword::new(std::mem::take(&mut *password))
            .map_err(|error| anyhow::anyhow!(error));
    }
    #[cfg(not(unix))]
    {
        let _ = file_descriptor;
        bail!("protected password file descriptors are unsupported on this platform");
    }
}

fn command_crash_boundary(command: &Command) -> Option<ProofCrashBoundary> {
    match command {
        Command::Create { crash_after, .. }
        | Command::ProcessStart { crash_after }
        | Command::Reset { crash_after, .. }
        | Command::ResumeReset { crash_after } => crash_after.map(Into::into),
        _ => None,
    }
}

fn signing_request(identity_ref: IdentityRef, request: String) -> Result<AdmittedSigningRequest> {
    Ok(AdmittedSigningRequest {
        request_ref: ReceiptRef::new(request)?,
        identity_ref,
        purpose: SigningPurpose::NostrEvent,
        event: UnsignedEventTemplate {
            created_at: 1_700_000_000,
            kind: 1,
            tags: Vec::new(),
            content: "Omega identity proof event".to_string(),
        },
    })
}

fn rejected_signing_outcome(
    service: &IdentityProofService,
    identity_ref: IdentityRef,
    request: String,
    outcome: &str,
) -> Result<()> {
    let request = signing_request(identity_ref, request)?;
    match service.sign(&request) {
        Ok(_) => bail!("forged or stale signing request was unexpectedly admitted"),
        Err(error) => print_outcome(outcome, json!({ "typed_error": error.to_string() })),
    }
}

fn print_outcome<T: Serialize>(outcome: &str, facts: T) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&Outcome {
            protocol: IDENTITY_PROOF_PROTOCOL,
            outcome,
            facts,
        })?
    );
    Ok(())
}
