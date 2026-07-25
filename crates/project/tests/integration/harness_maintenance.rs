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
    HarnessPinLedger, LoadedPinLedger, MaintenanceAction, MeasuredDigest, PinState,
    encode_harness_pin_ledger, latest_record_for,
};
use project::harness_maintenance::{
    authorize_installed_harness, authorize_version_fetch, load_pin_ledger, measure_installed_tree,
    pin_ledger_path, receipt_log_path,
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
