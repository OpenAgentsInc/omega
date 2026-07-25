//! The filesystem half of harness maintenance. omega#81, `OMEGA-DELTA-0025`.
//!
//! [`omega_harness`] holds every decision and every contract; this module is
//! the only thing that touches a disk, and it makes no judgements of its own.
//! The split matters because the decision layer is a leaf that a test can run
//! in microseconds, while this layer is bound to `Fs`, `paths`, and the async
//! agent-launch path.
//!
//! The enforcement point is [`authorize_installed_harness`], and it sits
//! between "the harness tree is on disk" and "Omega spawns it". A registry
//! agent runs with the tool permissions of the thread that started it, so the
//! question it answers is not *may we install this* but *may these exact bytes
//! run* — which is why the digest is measured from the installed tree on every
//! launch rather than trusted from the install that produced it.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use fs::Fs;
use futures::StreamExt as _;
use omega_harness::{
    HARNESS_MAINTENANCE_LOG_FILE_NAME, HARNESS_PIN_LEDGER_FILE_NAME, HarnessMaintenanceReceipt,
    HarnessPinLedgerError, LoadedPinLedger, MaintenanceAction, MaintenanceDecision,
    MeasuredDigest, admits_version, decide_maintenance, receipt_for_decision, receipt_log_line,
};
use util::ResultExt as _;

/// The host reference every receipt written on this machine carries.
///
/// A constant rather than a device identity on purpose. The log is local, so
/// the host is whichever machine the file is on; minting a stable device id
/// here would put a correlatable identifier into a file that has no reason to
/// carry one, and `omega#75` is explicit that the first-party agent does not
/// sign with its own principal.
pub const LOCAL_HOST_REF: &str = "host.omega.local";

/// The most files one installed harness tree may hold and still be measured.
///
/// A bound rather than an ambition. Exceeding it yields no measurement, which
/// [`decide_maintenance`] refuses — a harness too large to attest does not run
/// unattested.
pub const MAX_MEASURED_TREE_FILES: usize = 4096;

/// The largest single file that will be read into memory to measure it.
pub const MAX_MEASURED_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// Where the owner's pins live.
pub fn pin_ledger_path() -> PathBuf {
    paths::external_agents_dir().join(HARNESS_PIN_LEDGER_FILE_NAME)
}

/// Where maintenance receipts are appended.
pub fn receipt_log_path() -> PathBuf {
    paths::external_agents_dir().join(HARNESS_MAINTENANCE_LOG_FILE_NAME)
}

/// Read the pin ledger.
///
/// A file that is not there is [`LoadedPinLedger::Absent`]. A file that is
/// there and cannot be read — bad JSON, or an IO error — is
/// [`LoadedPinLedger::Unreadable`], which refuses. Collapsing those two would
/// make deleting one byte of the ledger a way to unfreeze every harness.
pub async fn load_pin_ledger(fs: &dyn Fs, path: &Path) -> LoadedPinLedger {
    if !fs.is_file(path).await {
        return LoadedPinLedger::Absent;
    }
    match fs.load(path).await {
        Ok(contents) => LoadedPinLedger::read(Some(&contents)),
        Err(_) => LoadedPinLedger::Unreadable(HarnessPinLedgerError::InvalidJson),
    }
}

/// Measure every regular file in an installed harness tree.
///
/// `None` is "this host could not read what would run", which the decision
/// layer treats as a refusal rather than as an absence of objection. Every
/// failure path here — an unreadable file, a tree above the bound, a directory
/// that vanished mid-walk — collapses to `None` deliberately: a partial
/// measurement is a wrong measurement, and there is no way to express one.
pub async fn measure_installed_tree(fs: &dyn Fs, dir: &Path) -> Option<MeasuredDigest> {
    let mut files: Vec<(String, MeasuredDigest)> = Vec::new();
    let mut pending: Vec<PathBuf> = vec![dir.to_path_buf()];

    while let Some(current) = pending.pop() {
        let mut entries = fs.read_dir(&current).await.ok()?;
        while let Some(entry) = entries.next().await {
            let entry = entry.ok()?;
            let metadata = fs.metadata(&entry).await.ok()??;
            if metadata.is_dir {
                pending.push(entry);
                continue;
            }
            // A symlink's target is not part of this tree, and following one
            // would let a link planted in the install directory attest bytes
            // from anywhere on the machine. Its presence still changes the
            // tree digest, because the path is recorded.
            let relative = entry.strip_prefix(dir).ok()?.to_str()?.to_string();
            if metadata.is_symlink {
                files.push((relative, MeasuredDigest::measure(b"")));
            } else {
                if metadata.len > MAX_MEASURED_FILE_BYTES {
                    return None;
                }
                files.push((relative, MeasuredDigest::measure(&fs.load_bytes(&entry).await.ok()?)));
            }
            if files.len() > MAX_MEASURED_TREE_FILES {
                return None;
            }
        }
    }

    if files.is_empty() {
        return None;
    }
    Some(MeasuredDigest::measure_tree(&mut files))
}

/// Append one receipt to the log.
///
/// Best-effort: a log that cannot be written must not stop a *refusal* from
/// taking effect, because the refusal is the safety property and the record is
/// the evidence for it. The caller therefore refuses first and records second.
pub async fn append_receipt(fs: &dyn Fs, path: &Path, receipt: &HarnessMaintenanceReceipt) {
    let Some(parent) = path.parent() else {
        return;
    };
    if fs.create_dir(parent).await.log_err().is_none() {
        return;
    }
    let mut existing = fs.load(path).await.unwrap_or_default();
    existing.push_str(&receipt_log_line(receipt));
    fs.write(path, existing.as_bytes()).await.log_err();
}

/// The gate that runs *before* a harness version is fetched.
///
/// Returns the sentence to show if the fetch must not happen. See
/// [`admits_version`] for why this is a prefilter and not the authority.
pub async fn authorize_version_fetch(
    fs: Arc<dyn Fs>,
    harness_id: &str,
    candidate_version: &str,
    now_ms: u64,
) -> Result<()> {
    let ledger = load_pin_ledger(fs.as_ref(), &pin_ledger_path()).await;
    let Some(refusal) = admits_version(ledger.pin_state(harness_id), candidate_version) else {
        return Ok(());
    };
    let decision = MaintenanceDecision::Refused(refusal.clone());
    if let Some(receipt) = receipt_for_decision(
        LOCAL_HOST_REF,
        harness_id,
        now_ms,
        MaintenanceAction::Update,
        &decision,
    )
    .log_err()
    {
        append_receipt(fs.as_ref(), &receipt_log_path(), &receipt).await;
    }
    anyhow::bail!("{}", refusal.reason())
}

/// The gate that runs *after* the tree is on disk and before Omega spawns it.
///
/// This is the enforcement point the omega#81 falsifier names: a binary that
/// will run with full tool permissions does not run unless this host hashed it
/// and the owner's pins admit the hash. Every call writes a receipt, permitted
/// or refused.
pub async fn authorize_installed_harness(
    fs: Arc<dyn Fs>,
    harness_id: &str,
    version: &str,
    installed_dir: &Path,
    action: MaintenanceAction,
    now_ms: u64,
) -> Result<MeasuredDigest> {
    let ledger = load_pin_ledger(fs.as_ref(), &pin_ledger_path()).await;
    let measured = measure_installed_tree(fs.as_ref(), installed_dir).await;
    let candidate = match measured.as_ref() {
        Some(digest) => omega_harness::CandidateArtifact::Measured { version, digest },
        None => omega_harness::CandidateArtifact::Unmeasured { version },
    };

    let decision = decide_maintenance(ledger.pin_state(harness_id), candidate);
    if let Some(receipt) =
        receipt_for_decision(LOCAL_HOST_REF, harness_id, now_ms, action, &decision).log_err()
    {
        append_receipt(fs.as_ref(), &receipt_log_path(), &receipt).await;
    }

    match decision {
        MaintenanceDecision::Permitted { digest, .. } => Ok(digest.clone()),
        MaintenanceDecision::Refused(refusal) => anyhow::bail!("{}", refusal.reason()),
    }
}

/// The host's clock, read once.
///
/// Every receipt this module writes is stamped from one call to this function,
/// made at the moment the action happened. Nothing that reaches the receipt
/// writer carries a time of its own, so there is no path by which a registry
/// document or a settings file can supply one.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}
