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
    HARNESS_MAINTENANCE_LOG_FILE_NAME, HARNESS_PIN_LEDGER_FILE_NAME, HarnessDistribution,
    HarnessFrontDoorState, HarnessMaintenanceReceipt, HarnessPinLedger, HarnessPinLedgerError,
    InstallationProvenance, LoadedPinLedger, MaintenanceAction, MaintenanceDecision,
    MaintenanceRefusal, MeasuredDigest, admits_package_manager_launch, admits_version,
    decide_maintenance, encode_harness_pin_ledger, harness_front_door_state, latest_record_for,
    receipt_for_decision, receipt_log_line,
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

/// The resolver name recorded for npm-distributed harnesses.
pub const NPX_RESOLVER: &str = "npx";

/// The gate for a harness whose bytes no directory of Omega's holds.
///
/// `LocalRegistryNpxAgent` resolves its package inside the node runtime's own
/// cache, at exec time, against a version *range* rather than an exact version
/// (see `bounded_npm_package_spec`). There is no tree to measure, so
/// [`authorize_installed_harness`] cannot be the gate here — routing this path
/// through it would refuse every npx harness on every machine, which is a
/// larger change than omega#81 asks for and would be dishonest about what the
/// measurement proved.
///
/// What this closes instead is the state where an owner froze such a harness
/// and Omega ran whatever `npm` resolved anyway. A pin that cannot be enforced
/// refuses. Every call writes a receipt, permitted or refused, so the log
/// records that this launch was *not* attested rather than leaving a silence a
/// reader would mistake for an attested one.
pub async fn authorize_package_manager_launch(
    fs: Arc<dyn Fs>,
    harness_id: &str,
    resolver: &str,
    now_ms: u64,
) -> Result<()> {
    let ledger = load_pin_ledger(fs.as_ref(), &pin_ledger_path()).await;
    let Some(refusal) = admits_package_manager_launch(ledger.pin_state(harness_id), resolver)
    else {
        return Ok(());
    };
    record_refusal(
        fs.as_ref(),
        harness_id,
        now_ms,
        MaintenanceAction::Verify,
        &refusal,
    )
    .await;
    anyhow::bail!("{}", refusal.reason())
}

/// Decide whether a version the registry channel now names may be offered.
///
/// Separate from the launch gates, and it runs when nothing is about to launch:
/// the registry document refreshes in the background and the store decides
/// whether to tell the owner an update is available. A harness frozen at
/// another version must not be offered one — the offer would lead to a refusal
/// on the next launch, which is the front door promising something the gate
/// then takes back.
///
/// Returns the refusal when the channel's answer is not offerable. The refusal
/// is recorded under [`MaintenanceAction::ResolveChannel`], because an update
/// that is never started leaves no other trace that it was considered.
pub async fn resolve_channel(
    fs: Arc<dyn Fs>,
    harness_id: &str,
    resolved_version: &str,
    now_ms: u64,
) -> Option<MaintenanceRefusal> {
    let ledger = load_pin_ledger(fs.as_ref(), &pin_ledger_path()).await;
    let refusal = admits_version(ledger.pin_state(harness_id), resolved_version)?;
    record_refusal(
        fs.as_ref(),
        harness_id,
        now_ms,
        MaintenanceAction::ResolveChannel,
        &refusal,
    )
    .await;
    Some(refusal)
}

/// Re-establish what an installed tree is, on the owner's request.
///
/// The same measurement the launch path takes, under its own action, with
/// nothing about to run. It is what the front door's verify control does and
/// what a caller runs after an update rather than carrying the prior
/// measurement forward.
pub async fn reprobe_installed_harness(
    fs: Arc<dyn Fs>,
    harness_id: &str,
    version: &str,
    installed_dir: &Path,
    now_ms: u64,
) -> Result<MeasuredDigest> {
    authorize_installed_harness(
        fs,
        harness_id,
        version,
        installed_dir,
        MaintenanceAction::ReprobeCapability,
        now_ms,
    )
    .await
}

/// Write one refusal to the log.
///
/// Goes through [`receipt_for_decision`] like every other write in this module,
/// so there is still exactly one receipt writer and it still takes a decision
/// rather than a caller's belief about one.
async fn record_refusal(
    fs: &dyn Fs,
    harness_id: &str,
    now_ms: u64,
    action: MaintenanceAction,
    refusal: &MaintenanceRefusal,
) {
    let decision = MaintenanceDecision::Refused(refusal.clone());
    if let Some(receipt) =
        receipt_for_decision(LOCAL_HOST_REF, harness_id, now_ms, action, &decision).log_err()
    {
        append_receipt(fs, &receipt_log_path(), &receipt).await;
    }
}

/// Persist a pin ledger.
///
/// [`encode_harness_pin_ledger`] routes through its own reader before returning,
/// so this cannot leave a file the next launch would read as
/// [`LoadedPinLedger::Unreadable`] — which would freeze every harness on the
/// machine rather than the one the owner meant.
pub async fn write_pin_ledger(fs: &dyn Fs, ledger: &HarnessPinLedger) -> Result<()> {
    let path = pin_ledger_path();
    let contents = encode_harness_pin_ledger(ledger)?;
    if let Some(parent) = path.parent() {
        fs.create_dir(parent).await?;
    }
    fs.write(&path, contents.as_bytes()).await?;
    Ok(())
}

/// Freeze a harness at the bytes this host measures now. The front door's
/// "Pin" control.
///
/// The measurement is taken through [`authorize_installed_harness`], not beside
/// it. That means taking a pin runs the real gate and writes a real receipt,
/// and a tree the gate refuses cannot be pinned — a pin recorded against bytes
/// the host would not run is a pin that means nothing.
///
/// It refuses to overwrite an existing pin. Moving a pin from one release to
/// another is remove-then-pin, two deliberate actions, because one click that
/// silently re-pointed a freeze at whatever is currently installed would undo
/// the freeze in the exact case it exists for.
pub async fn pin_installed_harness(
    fs: Arc<dyn Fs>,
    harness_id: &str,
    version: &str,
    installed_dir: &Path,
    now_ms: u64,
) -> Result<()> {
    let loaded = load_pin_ledger(fs.as_ref(), &pin_ledger_path()).await;
    let mut ledger = match loaded {
        LoadedPinLedger::Absent => HarnessPinLedger::empty(),
        LoadedPinLedger::Loaded(ledger) => ledger,
        LoadedPinLedger::Unreadable(error) => {
            anyhow::bail!(
                "Omega cannot read the pin ledger ({error}), so it will not rewrite it. \
                 Writing would drop pins it cannot see."
            )
        }
    };
    anyhow::ensure!(
        ledger.pin(harness_id).is_none(),
        "{harness_id} is already pinned. Remove the pin before taking a new one."
    );

    let digest = authorize_installed_harness(
        fs.clone(),
        harness_id,
        version,
        installed_dir,
        MaintenanceAction::ReprobeCapability,
        now_ms,
    )
    .await?;

    ledger.set_pin(harness_id, version, &digest);
    write_pin_ledger(fs.as_ref(), &ledger).await
}

/// Unfreeze a harness. The front door's "Remove pin" control.
///
/// An unreadable ledger refuses here too. "Remove the pin" on a file whose pins
/// cannot be read would mean rewriting it from the subset this build could
/// parse, which is not removal — it is deletion of everything unreadable.
pub async fn unpin_harness(fs: Arc<dyn Fs>, harness_id: &str) -> Result<()> {
    let loaded = load_pin_ledger(fs.as_ref(), &pin_ledger_path()).await;
    let mut ledger = match loaded {
        LoadedPinLedger::Absent => return Ok(()),
        LoadedPinLedger::Loaded(ledger) => ledger,
        LoadedPinLedger::Unreadable(error) => {
            anyhow::bail!(
                "Omega cannot read the pin ledger ({error}), so it will not rewrite it. \
                 Writing would drop pins it cannot see."
            )
        }
    };
    if !ledger.remove_pin(harness_id) {
        return Ok(());
    }
    write_pin_ledger(fs.as_ref(), &ledger).await
}

/// Read everything the front door shows for one harness.
///
/// Reads only. Nothing here writes a receipt, because looking at a settings
/// page is not a maintenance action and a log that recorded every render would
/// bury the actions that mattered.
pub async fn read_front_door_state(
    fs: &dyn Fs,
    harness_id: &str,
    version: &str,
    distribution: HarnessDistribution,
    installed_dir: Option<&Path>,
) -> HarnessFrontDoorState {
    let ledger = load_pin_ledger(fs, &pin_ledger_path()).await;
    let measured = match installed_dir {
        Some(dir) if fs.is_dir(dir).await => measure_installed_tree(fs, dir).await,
        _ => None,
    };
    let log = fs.load(&receipt_log_path()).await.unwrap_or_default();
    let provenance = if log.is_empty() {
        InstallationProvenance::Unattested
    } else {
        latest_record_for(&log, harness_id)
    };

    harness_front_door_state(
        harness_id,
        version,
        distribution,
        ledger.pin_state(harness_id),
        measured.as_ref(),
        &provenance,
    )
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
