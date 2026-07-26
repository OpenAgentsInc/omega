//! omega#81 / `OMEGA-DELTA-0025`, through a filesystem.
//!
//! `crates/omega_harness` proves the decisions. These prove that the code which
//! actually stands between an installed harness and a spawned process reads the
//! same ledger, measures the same bytes, and refuses in the same cases — on a
//! filesystem, across independent calls that share nothing but the disk.

use std::{path::Path, sync::Arc};

use fs::{FakeFs, Fs};
use gpui::TestAppContext;
use omega_harness::{
    HarnessDistribution, HarnessPinLedger, LoadedPinLedger, MaintenanceAction, MeasuredDigest,
    PinControl, PinState, encode_harness_pin_ledger, latest_record_for,
};
use project::harness_maintenance::{
    NPX_RESOLVER, authorize_installed_harness, authorize_package_manager_launch,
    authorize_version_fetch, load_pin_ledger, measure_installed_tree, pin_installed_harness,
    pin_ledger_path, read_front_door_state, receipt_log_path, reprobe_installed_harness,
    resolve_channel, unpin_harness,
};

const HARNESS: &str = "codex-acp";

fn install_dir() -> std::path::PathBuf {
    paths::external_agents_dir()
        .join("registry")
        .join(HARNESS)
        .join("v_0.9.4_abc_def")
}

async fn install(fs: &Arc<FakeFs>, binary: &[u8]) {
    let dir = install_dir();
    fs.create_dir(&dir).await.expect("install directory");
    fs.insert_file(dir.join("codex-acp"), binary.to_vec()).await;
    fs.insert_file(dir.join("LICENSE"), b"GPL".to_vec()).await;
}

async fn write_pin(fs: &Arc<FakeFs>, version: &str, digest: &MeasuredDigest) {
    let mut ledger = HarnessPinLedger::empty();
    ledger.set_pin(HARNESS, version, digest);
    let path = pin_ledger_path();
    fs.create_dir(path.parent().expect("parent"))
        .await
        .expect("ledger directory");
    fs.write(
        &path,
        encode_harness_pin_ledger(&ledger).expect("encodes").as_bytes(),
    )
    .await
    .expect("the ledger is written");
}

async fn receipt_log(fs: &Arc<FakeFs>) -> String {
    fs.load(&receipt_log_path()).await.unwrap_or_default()
}

#[gpui::test]
async fn a_measured_tree_binds_every_file_in_it(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;

    let first = measure_installed_tree(fs.as_ref(), &install_dir())
        .await
        .expect("an installed tree measures");
    let again = measure_installed_tree(fs.as_ref(), &install_dir())
        .await
        .expect("measuring twice agrees");
    assert_eq!(first, again);

    // One byte of the executable.
    fs.insert_file(install_dir().join("codex-acp"), b"binary v0.9.5".to_vec())
        .await;
    let swapped = measure_installed_tree(fs.as_ref(), &install_dir())
        .await
        .expect("measures");
    assert_ne!(first, swapped);

    // A file beside the executable, which a single-file digest would miss.
    fs.insert_file(install_dir().join("codex-acp"), b"binary v0.9.4".to_vec())
        .await;
    fs.insert_file(install_dir().join("plugin.so"), b"extra".to_vec())
        .await;
    let with_sidecar = measure_installed_tree(fs.as_ref(), &install_dir())
        .await
        .expect("measures");
    assert_ne!(
        first, with_sidecar,
        "a file added beside the executable must change the tree digest"
    );
}

#[gpui::test]
async fn a_tree_that_is_not_there_measures_nothing(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    assert_eq!(
        measure_installed_tree(fs.as_ref(), &install_dir()).await,
        None
    );
}

/// The acceptance sentence's first half: installing is one action and produces
/// a receipt bound to the bytes that will run.
#[gpui::test]
async fn an_install_on_a_clean_machine_produces_a_receipt(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;

    let digest = authorize_installed_harness(
        fs.clone(),
        HARNESS,
        "0.9.4",
        &install_dir(),
        MaintenanceAction::Install,
        1_784_894_400_000,
    )
    .await
    .expect("an unpinned install is permitted");

    let log = receipt_log(&fs).await;
    assert_eq!(log.lines().count(), 1, "one action, one record");
    assert!(log.contains(digest.as_str()), "the receipt binds the digest");
    assert!(log.contains("\"action\":\"install\""));
    assert!(!log.contains("/Users/"), "a receipt carries no path");

    let provenance = latest_record_for(&log, HARNESS);
    assert_eq!(
        omega_harness::verify_installation(&provenance, &digest),
        omega_harness::ProvenanceVerdict::Verified {
            digest: digest.as_str().to_string()
        }
    );
}

/// The acceptance sentence's second half, and the falsifier: a pinned harness
/// whose bytes were replaced does not run, the refusal names why, and it is
/// recorded.
#[gpui::test]
async fn a_pinned_harness_whose_bytes_changed_does_not_run(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;
    let installed = measure_installed_tree(fs.as_ref(), &install_dir())
        .await
        .expect("measures");
    write_pin(&fs, "0.9.4", &installed).await;

    // Still the pinned bytes: permitted.
    authorize_installed_harness(
        fs.clone(),
        HARNESS,
        "0.9.4",
        &install_dir(),
        MaintenanceAction::Verify,
        1_784_894_400_000,
    )
    .await
    .expect("a pin freezes a harness, it does not disable it");

    // The release is re-tagged in place: same version, different bytes.
    fs.insert_file(
        install_dir().join("codex-acp"),
        b"binary v0.9.4, rebuilt".to_vec(),
    )
    .await;

    let error = authorize_installed_harness(
        fs.clone(),
        HARNESS,
        "0.9.4",
        &install_dir(),
        MaintenanceAction::Verify,
        1_784_894_400_001,
    )
    .await
    .expect_err("substituted bytes must not run");
    let reason = error.to_string();
    assert!(reason.contains("0.9.4"), "{reason:?}");
    assert!(reason.contains("replaced"), "{reason:?}");
    assert!(!reason.contains('/'), "{reason:?} carries a path");

    let log = receipt_log(&fs).await;
    assert!(log.contains("\"reasonClass\":\"pinned_digest\""), "{log}");
    assert_eq!(
        log.lines().count(),
        2,
        "the refusal is recorded, not only raised"
    );
}

/// A pin blocks the fetch as well as the run, so a frozen harness does not
/// quietly download the release the registry now advertises.
#[gpui::test]
async fn a_pin_blocks_the_fetch_of_a_different_version(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    write_pin(&fs, "0.9.4", &MeasuredDigest::measure(b"pinned tree")).await;

    let error = authorize_version_fetch(fs.clone(), HARNESS, "0.9.5", 1_784_894_400_000)
        .await
        .expect_err("a pinned harness does not fetch a different version");
    let reason = error.to_string();
    assert!(reason.contains("0.9.4") && reason.contains("0.9.5"), "{reason:?}");

    authorize_version_fetch(fs.clone(), HARNESS, "0.9.4", 1_784_894_400_001)
        .await
        .expect("the pinned version itself is still fetchable");

    authorize_version_fetch(fs.clone(), "gemini-cli", "2.0.0", 1_784_894_400_002)
        .await
        .expect("an unpinned harness is unaffected");

    let log = receipt_log(&fs).await;
    assert_eq!(
        log.lines().count(),
        1,
        "only the refusal is a maintenance action"
    );
    assert!(log.contains("\"reasonClass\":\"pinned_version\""), "{log}");
}

/// Nothing in the enforcement path holds the ledger between calls, so the pin
/// is re-read from disk every time it is consulted. That is the same statement
/// as surviving a restart, and this asserts it against a filesystem: a second
/// independent call, sharing nothing with the first, still refuses.
#[gpui::test]
async fn the_pin_is_re_read_from_disk_on_every_decision(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;
    write_pin(&fs, "0.9.4", &MeasuredDigest::measure(b"different bytes")).await;

    for attempt in 0..3 {
        authorize_installed_harness(
            fs.clone(),
            HARNESS,
            "0.9.4",
            &install_dir(),
            MaintenanceAction::Verify,
            1_784_894_400_000 + attempt,
        )
        .await
        .expect_err("the pin holds across independent calls");
    }

    // And removing the ledger unfreezes it — the pin is that file and nothing
    // else, so the refusal was not coming from somewhere it could not be undone.
    fs.remove_file(&pin_ledger_path(), Default::default())
        .await
        .expect("the ledger is removed");
    authorize_installed_harness(
        fs.clone(),
        HARNESS,
        "0.9.4",
        &install_dir(),
        MaintenanceAction::Verify,
        1_784_894_400_010,
    )
    .await
    .expect("an unpinned harness runs");
}

/// A corrupt ledger is not an absent one. Truncating the file must not be a way
/// to unfreeze every harness on the machine.
#[gpui::test]
async fn a_corrupt_ledger_refuses_rather_than_reading_as_unpinned(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;
    let path = pin_ledger_path();
    fs.create_dir(path.parent().expect("parent"))
        .await
        .expect("directory");
    fs.write(&path, b"{ \"schema\": \"openagents.omega.harness.pins.v1\", \"pins\": [")
        .await
        .expect("written");

    assert_eq!(
        load_pin_ledger(fs.as_ref(), &path).await.pin_state(HARNESS),
        PinState::Unreadable
    );
    let error = authorize_installed_harness(
        fs.clone(),
        HARNESS,
        "0.9.4",
        &install_dir(),
        MaintenanceAction::Verify,
        1_784_894_400_000,
    )
    .await
    .expect_err("an unreadable ledger fails closed");
    assert!(error.to_string().contains("pin ledger"), "{error}");
}

#[gpui::test]
async fn a_missing_ledger_file_is_absent_rather_than_unreadable(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    assert_eq!(
        load_pin_ledger(fs.as_ref(), Path::new("/nowhere/omega-harness-pins.json")).await,
        LoadedPinLedger::Absent
    );
}

// ---------------------------------------------------------------------------
// The owner's controls. omega#81 deliverable 4, `OMEGA-DELTA-0033`.
//
// The first landing of omega#81 could decide all of this and could not do any
// of it: the ledger was a JSON file with no writer in production. These prove
// the two controls the front door offers move the same ledger the gate reads.
// ---------------------------------------------------------------------------

#[gpui::test]
async fn a_pin_taken_from_the_front_door_blocks_the_next_version(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;

    pin_installed_harness(fs.clone(), HARNESS, "0.9.4", &install_dir(), 1_784_894_400_000)
        .await
        .expect("a measurable tree can be pinned");

    // The pin the control wrote is the pin the gate reads: same file, same
    // reader, no shared process state.
    let ledger = load_pin_ledger(fs.as_ref(), &pin_ledger_path()).await;
    let PinState::Pinned(pin) = ledger.pin_state(HARNESS) else {
        panic!("the control did not write a pin the gate can read");
    };
    assert_eq!(pin.version, "0.9.4");
    assert_eq!(
        pin.digest,
        measure_installed_tree(fs.as_ref(), &install_dir())
            .await
            .expect("measurable")
            .as_str(),
        "the pin froze bytes other than the ones it measured"
    );

    // And the acceptance sentence's second half: the pinned version blocks an
    // unwanted update, before any bytes move.
    let refused = authorize_version_fetch(fs.clone(), HARNESS, "0.9.5", 1_784_894_400_001)
        .await
        .expect_err("a pin blocks the version it did not name");
    assert!(refused.to_string().contains("0.9.4"), "{refused}");
    assert!(refused.to_string().contains("0.9.5"), "{refused}");
}

#[gpui::test]
async fn taking_a_pin_writes_a_receipt_for_the_measurement_that_authorised_it(
    cx: &mut TestAppContext,
) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;

    pin_installed_harness(fs.clone(), HARNESS, "0.9.4", &install_dir(), 1_784_894_400_000)
        .await
        .expect("pinned");

    let log = receipt_log(&fs).await;
    assert!(
        log.contains("\"action\":\"reprobe_capability\""),
        "a pin was taken with no recorded measurement behind it: {log}"
    );
    assert!(log.contains("\"kind\":\"applied\""), "{log}");
}

/// A pin against bytes nobody read would be a freeze with no meaning. The
/// control refuses rather than recording the version alone.
#[gpui::test]
async fn a_tree_that_cannot_be_measured_cannot_be_pinned(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    let error = pin_installed_harness(
        fs.clone(),
        HARNESS,
        "0.9.4",
        Path::new("/nowhere/at/all"),
        1_784_894_400_000,
    )
    .await
    .expect_err("an unmeasurable tree cannot be pinned");
    assert!(error.to_string().contains("could not read"), "{error}");
    assert_eq!(
        load_pin_ledger(fs.as_ref(), &pin_ledger_path())
            .await
            .pin_state(HARNESS),
        PinState::Unpinned,
        "a refused pin still wrote to the ledger"
    );
}

/// Re-pointing a freeze at whatever is installed now, in one click, would undo
/// the freeze in the exact case it exists for.
#[gpui::test]
async fn a_pin_is_not_silently_moved_onto_a_different_release(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;
    pin_installed_harness(fs.clone(), HARNESS, "0.9.4", &install_dir(), 1_784_894_400_000)
        .await
        .expect("pinned");

    let error = pin_installed_harness(fs.clone(), HARNESS, "0.9.5", &install_dir(), 1_784_894_400_001)
        .await
        .expect_err("a second pin is refused");
    assert!(error.to_string().contains("already pinned"), "{error}");
    let ledger = load_pin_ledger(fs.as_ref(), &pin_ledger_path()).await;
    let PinState::Pinned(pin) = ledger.pin_state(HARNESS) else {
        panic!("the original pin vanished");
    };
    assert_eq!(pin.version, "0.9.4");
}

#[gpui::test]
async fn removing_a_pin_lets_the_blocked_version_through(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;
    pin_installed_harness(fs.clone(), HARNESS, "0.9.4", &install_dir(), 1_784_894_400_000)
        .await
        .expect("pinned");
    authorize_version_fetch(fs.clone(), HARNESS, "0.9.5", 1_784_894_400_001)
        .await
        .expect_err("blocked while pinned");

    unpin_harness(fs.clone(), HARNESS)
        .await
        .expect("the pin is removed");

    authorize_version_fetch(fs.clone(), HARNESS, "0.9.5", 1_784_894_400_002)
        .await
        .expect("an unpinned harness may fetch the version the registry offers");
}

/// A ledger this build cannot read must not be rewritten from the subset it
/// could parse. That is not removal — it is deletion of everything unreadable.
#[gpui::test]
async fn neither_control_rewrites_a_ledger_it_cannot_read(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;
    let path = pin_ledger_path();
    fs.create_dir(path.parent().expect("parent"))
        .await
        .expect("directory");
    fs.write(&path, b"{ not a ledger").await.expect("written");

    for error in [
        pin_installed_harness(fs.clone(), HARNESS, "0.9.4", &install_dir(), 1_784_894_400_000)
            .await
            .expect_err("pin refuses"),
        unpin_harness(fs.clone(), HARNESS)
            .await
            .expect_err("unpin refuses"),
    ] {
        assert!(error.to_string().contains("cannot read the pin ledger"), "{error}");
    }
    assert_eq!(
        fs.load(&path).await.expect("still there"),
        "{ not a ledger",
        "an unreadable ledger was overwritten"
    );
}

// ---------------------------------------------------------------------------
// The npx gap. omega#81, `OMEGA-DELTA-0033`.
// ---------------------------------------------------------------------------

/// Before this, pinning an npx harness did nothing at all: nothing in that
/// launch path consulted the ledger, so the owner's "not that one" was silently
/// discarded and `npm` chose.
#[gpui::test]
async fn a_pinned_package_manager_harness_refuses_to_launch(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;
    pin_installed_harness(fs.clone(), HARNESS, "0.9.4", &install_dir(), 1_784_894_400_000)
        .await
        .expect("pinned");

    let error =
        authorize_package_manager_launch(fs.clone(), HARNESS, NPX_RESOLVER, 1_784_894_400_001)
            .await
            .expect_err("a pinned npx harness does not launch");
    assert!(error.to_string().contains("npx"), "{error}");

    let log = receipt_log(&fs).await;
    assert!(
        log.contains("\"reasonClass\":\"unpinnable_distribution\""),
        "the refusal left no record: {log}"
    );
}

/// The honest limit, asserted rather than left to a reader's assumption: this
/// gate raises no bar on an unpinned npx harness.
#[gpui::test]
async fn an_unpinned_package_manager_harness_still_launches(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    authorize_package_manager_launch(fs.clone(), HARNESS, NPX_RESOLVER, 1_784_894_400_000)
        .await
        .expect("an unpinned npx harness launches");
}

// ---------------------------------------------------------------------------
// Resolving the channel. omega#81 deliverable 1, `OMEGA-DELTA-0033`.
// ---------------------------------------------------------------------------

/// A frozen harness must not be *offered* the version the registry now names.
/// The offer would lead to a refusal on the next launch — the front door
/// promising what the gate then takes back.
#[gpui::test]
async fn a_frozen_harness_is_not_offered_the_version_it_would_refuse(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;
    pin_installed_harness(fs.clone(), HARNESS, "0.9.4", &install_dir(), 1_784_894_400_000)
        .await
        .expect("pinned");

    let refusal = resolve_channel(fs.clone(), HARNESS, "0.9.5", 1_784_894_400_001)
        .await
        .expect("a pinned harness refuses the channel's answer");
    assert_eq!(refusal.reason_class(), "pinned_version");

    let log = receipt_log(&fs).await;
    assert!(
        log.contains("\"action\":\"resolve_channel\""),
        "an update that never started left no trace it was considered: {log}"
    );

    // Unpinned, the same answer is offerable.
    unpin_harness(fs.clone(), HARNESS).await.expect("unpinned");
    assert!(
        resolve_channel(fs.clone(), HARNESS, "0.9.5", 1_784_894_400_002)
            .await
            .is_none()
    );
}

// ---------------------------------------------------------------------------
// What the front door reads. omega#81 deliverable 4, `OMEGA-DELTA-0033`.
// ---------------------------------------------------------------------------

/// The row and the gate read the same three inputs off the same disk. A front
/// door that measured a different tree than the launch path gates would show a
/// control that looks live and then fails.
#[gpui::test]
async fn the_front_door_reads_the_state_the_gate_would_enforce(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;

    let before = read_front_door_state(
        fs.as_ref(),
        HARNESS,
        "0.9.4",
        HarnessDistribution::OwnedTree,
        Some(&install_dir()),
    )
    .await;
    assert!(before.launch.is_enabled());
    assert!(matches!(before.pin_control, PinControl::Take { .. }));

    pin_installed_harness(fs.clone(), HARNESS, "0.9.4", &install_dir(), 1_784_894_400_000)
        .await
        .expect("pinned");

    let pinned = read_front_door_state(
        fs.as_ref(),
        HARNESS,
        "0.9.5",
        HarnessDistribution::OwnedTree,
        Some(&install_dir()),
    )
    .await;
    assert!(!pinned.launch.is_enabled(), "a pin did not reach the row");
    let reason = pinned.launch.reason().expect("a disabled row has a sentence");
    assert!(reason.contains("0.9.4") && reason.contains("0.9.5"), "{reason}");
    assert_eq!(
        pinned.pin_control,
        PinControl::Remove {
            pinned_version: "0.9.4".to_string()
        }
    );

    // And the row's verdict is the launch path's verdict, taken independently.
    let launch = authorize_installed_harness(
        fs.clone(),
        HARNESS,
        "0.9.5",
        &install_dir(),
        MaintenanceAction::Verify,
        1_784_894_400_002,
    )
    .await;
    assert!(launch.is_err(), "the row refused and the gate did not");
}

/// After a real measurement is recorded, the row says the installed bytes are
/// the recorded bytes — and stops saying so the moment they are not.
#[gpui::test]
async fn the_row_reports_provenance_and_notices_a_swap(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    install(&fs, b"binary v0.9.4").await;

    reprobe_installed_harness(fs.clone(), HARNESS, "0.9.4", &install_dir(), 1_784_894_400_000)
        .await
        .expect("measured");

    let verified = read_front_door_state(
        fs.as_ref(),
        HARNESS,
        "0.9.4",
        HarnessDistribution::OwnedTree,
        Some(&install_dir()),
    )
    .await;
    assert!(matches!(
        verified.provenance,
        omega_harness::ProvenanceVerdict::Verified { .. }
    ));

    fs.insert_file(install_dir().join("codex-acp"), b"swapped".to_vec())
        .await;
    let swapped = read_front_door_state(
        fs.as_ref(),
        HARNESS,
        "0.9.4",
        HarnessDistribution::OwnedTree,
        Some(&install_dir()),
    )
    .await;
    assert_eq!(
        swapped.provenance,
        omega_harness::ProvenanceVerdict::Refused(omega_harness::ProvenanceGap::DigestMismatch)
    );
}

// ---------------------------------------------------------------------------
// The live proof. omega#81, `OMEGA-DELTA-0033`.
//
// Everything above this line runs against `FakeFs`. The first landing of
// omega#81 had nothing below it, and the acceptance sentence says "from a clean
// machine" — so this one downloads a real release from the live ACP registry
// onto a real filesystem, through the same downloader
// `LocalRegistryArchiveAgent::get_command` uses, and puts the real gate between
// the extracted tree and the command.
//
// Ignored by default: it needs the network, and a test suite that silently
// depends on a third party's release assets is a test suite that goes red for
// reasons that are not about this repository. Run it with
// `cargo test -p project --features test-support --test integration -- --ignored live_`.
// ---------------------------------------------------------------------------

/// What the live ACP registry says about one binary-distributed harness.
struct LiveHarness {
    id: String,
    version: String,
    archive: String,
    sha256: Option<String>,
}

async fn live_registry_harness(http: &dyn http_client::HttpClient) -> Option<LiveHarness> {
    use futures::AsyncReadExt as _;

    let mut response = http
        .get(
            "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json",
            Default::default(),
            true,
        )
        .await
        .expect("the live ACP registry answers");
    let mut body = String::new();
    response
        .body_mut()
        .read_to_string(&mut body)
        .await
        .expect("the registry document reads");
    let document: serde_json::Value = serde_json::from_str(&body).expect("the registry is JSON");

    let platform = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "windows"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    let key = format!("{platform}-{arch}");

    document
        .get("agents")?
        .as_array()?
        .iter()
        .find_map(|agent| {
            // A `.tar.gz` keeps this proof about the gate rather than about
            // archive formats, and `sha256`-free entries keep it from depending
            // on a checksum a publisher may re-cut.
            let target = agent.get("distribution")?.get("binary")?.get(&key)?;
            let archive = target.get("archive")?.as_str()?;
            if !archive.ends_with(".tar.gz") {
                return None;
            }
            Some(LiveHarness {
                id: agent.get("id")?.as_str()?.to_string(),
                version: agent.get("version")?.as_str()?.to_string(),
                archive: archive.to_string(),
                sha256: target
                    .get("sha256")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
            })
        })
}

#[gpui::test]
#[ignore = "live: fetches the ACP registry and downloads a real harness release"]
async fn live_a_real_registry_install_produces_a_receipt_and_a_pin_blocks_the_next_version(
    cx: &mut TestAppContext,
) {
    // Real network and real disk, so the deterministic test scheduler has to be
    // told this test parks. Everything above this line runs without it.
    cx.executor().allow_parking();

    let data_dir = tempfile::tempdir().expect("a clean machine");
    // Redirect every path this test touches. Without this the proof would write
    // pins and receipts into the developer's own Omega installation.
    paths::set_custom_data_dir(data_dir.path().to_str().expect("utf-8 temp dir"));

    let fs: Arc<dyn Fs> = Arc::new(fs::RealFs::new(None, cx.executor()));
    let http = reqwest_client::ReqwestClient::user_agent("omega-harness-live-proof")
        .expect("an http client");

    let Some(harness) = live_registry_harness(&http).await else {
        panic!("the live registry offers no .tar.gz binary target for this platform");
    };
    let version_dir = paths::external_agents_dir()
        .join("registry")
        .join(&harness.id)
        .join(format!("v_{}_live", harness.version));

    // The pin is consulted before any bytes move, exactly as the launch path
    // consults it. On a clean machine nothing is frozen, so the fetch proceeds.
    authorize_version_fetch(fs.clone(), &harness.id, &harness.version, 1_784_894_400_000)
        .await
        .expect("a clean machine has no pin to block the first install");

    // `LocalRegistryArchiveAgent::get_command` creates the installation
    // directory before it downloads into a version directory under it.
    fs.create_dir(version_dir.parent().expect("an installation directory"))
        .await
        .expect("the installation directory is created");

    http_client::github_download::download_server_binary(
        &http,
        &harness.archive,
        harness.sha256.as_deref(),
        &version_dir,
        http_client::github::AssetKind::TarGz,
    )
    .await
    .unwrap_or_else(|error| panic!("downloading {}: {error}", harness.archive));

    // The enforcement point, on real extracted bytes.
    let installed = authorize_installed_harness(
        fs.clone(),
        &harness.id,
        &harness.version,
        &version_dir,
        MaintenanceAction::Install,
        1_784_894_400_001,
    )
    .await
    .expect("a real installed tree is measurable and unpinned");

    // The receipt is on disk, and it binds the bytes that were extracted.
    let log = fs.load(&receipt_log_path()).await.unwrap_or_default();
    // Printed because this test's whole point is that the receipt is real. Run
    // with `--nocapture` to read the bytes that were written.
    eprintln!("live install receipt log:\n{log}");
    assert!(
        log.contains(&format!("\"harnessRef\":\"harness.{}\"", harness.id)),
        "no receipt names the harness that was installed: {log}"
    );
    assert!(
        log.contains(installed.as_str()),
        "the receipt does not carry the digest the install measured: {log}"
    );

    // One action freezes it, at bytes this host read rather than at a version
    // that was typed.
    pin_installed_harness(
        fs.clone(),
        &harness.id,
        &harness.version,
        &version_dir,
        1_784_894_400_002,
    )
    .await
    .expect("a real installed tree can be pinned");

    // And the pin blocks an unwanted update with a sentence that names both
    // versions — the acceptance sentence of omega#81, on a real machine.
    let refusal = authorize_version_fetch(
        fs.clone(),
        &harness.id,
        "99.99.99",
        1_784_894_400_003,
    )
    .await
    .expect_err("a pinned harness refuses the version the registry would offer");
    assert!(refusal.to_string().contains(&harness.version), "{refusal}");
    assert!(refusal.to_string().contains("99.99.99"), "{refusal}");

    // The falsifier omega#81 names, on real bytes: replace the executable after
    // the install receipt was written and the harness does not run.
    let swapped = version_dir.join("omega-live-proof-swap");
    fs.write(&swapped, b"a file the pin never measured")
        .await
        .expect("the tree is writable");
    let after_swap = authorize_installed_harness(
        fs.clone(),
        &harness.id,
        &harness.version,
        &version_dir,
        MaintenanceAction::Verify,
        1_784_894_400_004,
    )
    .await
    .expect_err("bytes added after the pin was taken do not run");
    assert!(
        after_swap.to_string().contains("replaced"),
        "the refusal did not name the substitution: {after_swap}"
    );
}
