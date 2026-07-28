//! Opt-in measurement harness for scans of a real directory.
//!
//! Ignored by default because it walks a caller-supplied directory that only
//! exists on the operator's machine, and because a large one takes minutes.
//! Run it with:
//!
//! ```sh
//! OMEGA_SCAN_MEASURE_PATH=$HOME/some/project \
//!   cargo test -p worktree --features test-support --test integration \
//!   -- scan_measurement --ignored --nocapture
//! ```

use fs::{Fs, RealFs};
use gpui::TestAppContext;
use settings::{Settings as _, SettingsStore, WorktreeId};
use std::{path::Path, sync::Arc, time::Instant};
use worktree::Worktree;

#[gpui::test]
#[ignore]
async fn scan_measurement(cx: &mut TestAppContext) {
    let root = std::env::var("OMEGA_SCAN_MEASURE_PATH")
        .expect("set OMEGA_SCAN_MEASURE_PATH to the directory to scan");

    // The scan is real filesystem work on a real executor, not simulated time.
    cx.executor().allow_parking();

    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        worktree::WorktreeSettings::register(cx);
    });

    let fs = Arc::new(RealFs::new(None, cx.executor())) as Arc<dyn Fs>;
    let started = Instant::now();
    let tree = Worktree::local(
        Path::new(&root),
        true,
        fs,
        Default::default(),
        true,
        WorktreeId::from_proto(0),
        &mut cx.to_async(),
    )
    .await
    .unwrap();
    cx.read(|cx| tree.read(cx).as_local().unwrap().scan_complete())
        .await;

    tree.read_with(cx, |tree, _| {
        let total = tree.entry_count();
        let visible = tree.visible_entry_count();
        println!(
            "MEASURE root={root} elapsed={:.1}s entries={total} visible={visible} \
             ignored={} truncated_at={:?}",
            started.elapsed().as_secs_f64(),
            total - visible,
            tree.scan_truncated_at(),
        );
    });
}
