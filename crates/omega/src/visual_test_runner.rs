// Allow blocking process commands in this binary - it's a synchronous test runner
#![allow(clippy::disallowed_methods)]

//! Visual Test Runner
//!
//! This binary runs visual regression tests for Zed's UI. It captures screenshots
//! of real Zed windows and compares them against baseline images.
//!
//! **Note: This tool is macOS-only** because it uses `VisualTestAppContext` which
//! depends on the macOS Metal renderer for accurate screenshot capture.
//!
//! ## How It Works
//!
//! This tool uses `VisualTestAppContext` which combines:
//! - Real Metal/compositor rendering for accurate screenshots
//! - Deterministic task scheduling via TestDispatcher
//! - Controllable time via `advance_clock` for testing time-based behaviors
//!
//! This approach:
//! - Does NOT require Screen Recording permission
//! - Does NOT require the window to be visible on screen
//! - Captures raw GPUI output without system window chrome
//! - Is fully deterministic - tooltips, animations, etc. work reliably
//!
//! ## Usage
//!
//! Run the visual tests:
//!   cargo run -p zed --bin zed_visual_test_runner --features visual-tests
//!
//! Update baseline images (when UI intentionally changes):
//!   UPDATE_BASELINE=1 cargo run -p zed --bin zed_visual_test_runner --features visual-tests
//!
//! ## Environment Variables
//!
//!   UPDATE_BASELINE - Set to update baseline images instead of comparing
//!   VISUAL_TEST_OUTPUT_DIR - Directory to save test output (default: target/visual_tests)

// Stub main for non-macOS platforms
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Visual test runner is only supported on macOS");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() {
    match print_workbench_scene_catalog() {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            eprintln!("Failed to list workbench scenes: {error:#}");
            std::process::exit(2);
        }
    }
    if let Err(error) = initialize_workbench_proof() {
        eprintln!("Invalid workbench proof configuration: {error:#}");
        std::process::exit(2);
    }
    if workbench_proof_active() && !workbench_has_selected_scene_in_phase() {
        return;
    }

    // Set ZED_STATELESS early to prevent file system access to real config directories
    // This must be done before any code accesses zed_env_vars::ZED_STATELESS
    // SAFETY: We're at the start of main(), before any threads are spawned
    unsafe {
        std::env::set_var("ZED_STATELESS", "1");
    }

    // Redirect every derived data path into a temporary directory. A visual
    // test that exercises real on-disk state — omega#81's harness pins and
    // receipts do — would otherwise write into the developer's own Omega
    // installation. Set before anything can read `data_dir`.
    //
    // `OMEGA_VISUAL_DATA_DIR` overrides the temporary directory so that two
    // *processes* can share one data directory. That is what makes omega#77's
    // restart captures a restart: the second process is a genuinely cold one —
    // empty statics, nothing carried in memory — and the only thing it has of
    // the first is what the first left on disk. It is still never the
    // developer's own data directory; `script/omega-visual-proof` creates a
    // temporary one and passes it in.
    let data_dir = match std::env::var("OMEGA_VISUAL_DATA_DIR") {
        Ok(path) if !path.is_empty() => std::path::PathBuf::from(path),
        _ => tempfile::tempdir()
            .expect("Failed to create data directory")
            .keep(),
    };
    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");
    paths::set_custom_data_dir(data_dir.to_str().expect("Data directory path is not UTF-8"));

    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    let update_baseline = std::env::var("UPDATE_BASELINE").is_ok();

    // Create a temporary directory for test files
    // Canonicalize the path to resolve symlinks (on macOS, /var -> /private/var)
    // which prevents "path does not exist" errors during worktree scanning
    // Use keep() to prevent auto-cleanup - background worktree tasks may still be running
    // when tests complete, so we let the OS clean up temp directories on process exit
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let temp_path = temp_dir.keep();
    let canonical_temp = temp_path
        .canonicalize()
        .expect("Failed to canonicalize temp directory");
    let project_path = canonical_temp.join("project");
    std::fs::create_dir_all(&project_path).expect("Failed to create project directory");

    // Create test files in the real filesystem
    create_test_files(&project_path);

    let refusals_before = omega_zero_base::logged_refusal_count();
    let test_result = std::panic::catch_unwind(|| run_visual_tests(project_path, update_baseline));
    let logged_refusals = omega_zero_base::logged_refusal_count().saturating_sub(refusals_before);
    let run_succeeded = matches!(&test_result, Ok(Ok(())))
        && omega_zero_base::proof_is_refusal_free(logged_refusals);
    if let Err(error) = finalize_workbench_proof(run_succeeded) {
        eprintln!("Failed to finalize workbench proof: {error:#}");
        std::process::exit(1);
    }
    if !omega_zero_base::proof_is_refusal_free(logged_refusals) {
        eprintln!("Visual tests failed: the run logged {logged_refusals} refused action(s)");
        std::process::exit(1);
    }

    // Note: We don't delete temp_path here because background worktree tasks may still
    // be running. The directory will be cleaned up when the process exits or by the OS.

    match test_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            // `{:#}` and not `{}`: every failure here is wrapped in the name of
            // the suite that produced it, and `{}` prints only that wrapper.
            // A run that failed because a restored record named the wrong run
            // would report "omega_agent_surfaces" and nothing else, which is
            // the same class of unreadable failure this file already carries a
            // comment about.
            eprintln!("Visual tests failed: {:#}", e);
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("Visual tests panicked");
            std::process::exit(1);
        }
    }
}

#[cfg(target_os = "macos")]
fn print_workbench_scene_catalog() -> Result<bool> {
    if std::env::var("OMEGA_WORKBENCH_LIST_SCENES").is_err() {
        return Ok(false);
    }
    omega_workbench_harness::validate_scene_catalog()?;
    if std::env::var("OMEGA_WORKBENCH_LIST_FORMAT").as_deref() == Ok("json") {
        let scenes: Vec<_> = HERMETIC_SCENES
            .iter()
            .map(|scene| {
                serde_json::json!({
                    "name": scene.name,
                    "phase": scene.phase,
                    "viewport": {
                        "width": scene.viewport.width,
                        "height": scene.viewport.height,
                        "scale_milli": scene.viewport.scale_milli,
                    },
                    "minimum_match": scene.pixel_policy.minimum_match,
                    "channel_tolerance": scene.pixel_policy.channel_tolerance,
                    "pixel_policy_rationale": scene.pixel_policy.rationale,
                    "regions": scene.regions.iter().map(|region| region.name).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&scenes)?);
    } else {
        for scene in HERMETIC_SCENES {
            println!("{}\t{:?}", scene.name, scene.phase);
        }
    }
    Ok(true)
}

// All macOS-specific imports grouped together
#[cfg(target_os = "macos")]
use {
    acp_thread::{
        AgentConnection, AuthorizationKind, PermissionOptions, StubAgentConnection,
        ThreadTerminalStatus,
    },
    agent_client_protocol::schema::v1 as acp,
    agent_servers::{AgentServer, AgentServerDelegate},
    agent_ui::{Agent, AgentPanel},
    anyhow::{Context as _, Result},
    assets::Assets,
    editor::display_map::DisplayRow,
    feature_flags::FeatureFlagAppExt as _,
    git_ui::{
        git_panel::{GitPanel, GitPanelHeadState, GitPanelRepositoryScope, GitPanelStateSnapshot},
        project_diff::ProjectDiff,
        staged_diff::StagedDiff,
        unstaged_diff::UnstagedDiff,
    },
    gpui::{
        AnyWindowHandle, App, AppContext as _, Bounds, Entity, Focusable as _, KeyBinding,
        Modifiers, VisualTestAppContext, WindowBounds, WindowHandle, WindowOptions, point, px,
        size,
    },
    language_model::{LanguageModelProviderId, LanguageModelRegistry},
    omega_actions::OpenSettingsAt,
    omega_workbench_harness::{
        CheckStatus, HERMETIC_SCENES, PixelProof, PixelStatus, ProofCheck, ProofLane, ProofOutcome,
        ProofReceipt, RegionPixelProof, ScenePhase, SemanticProbe, WORKBENCH_PLAN_PIXEL_SCENES,
        WORKBENCH_SHELL_PIXEL_SCENES, WORKBENCH_TERMINAL_PIXEL_SCENES, WorkSurfaceId,
        WorkbenchScene, compare_images as compare_workbench_images, scene_spec, select_scenes,
    },
    project::{AgentId, Project},
    project_panel::ProjectPanel,
    settings::{NotifyWhenAgentWaiting, PlaySoundWhenAgentDone, Settings as _},
    settings_ui::SettingsWindow,
    std::{
        any::Any,
        cell::RefCell,
        collections::{BTreeMap, BTreeSet},
        path::{Path, PathBuf},
        rc::Rc,
        sync::Arc,
        time::Duration,
    },
    terminal_view::terminal_panel::TerminalPanel,
    util::ResultExt as _,
    workspace::{AppState, MultiWorkspace, Workspace, item::Item as _},
};

// OMEGA-DELTA-0052, omega#100. The `omega_zero_base_ui` module used to be
// compiled into this binary too, because the runner installed the mode's
// status-bar control by hand before photographing a zero-base scene.
//
// That control is gone. What is left in that module — the palette restriction
// and the action gate — is proven by `command_palette_hooks`' restriction test
// and by gpui's action-gate keystroke test, neither of which a screenshot could
// show, so the runner no longer compiles it at all. The scenes still photograph
// the shipped surface: the difference between the two pairs is now entirely
// what `omega_zero_base::is_active()` makes the panel and the composer render.
// All macOS-specific constants grouped together
#[cfg(target_os = "macos")]
mod constants {
    use std::time::Duration;

    /// Baseline images are stored relative to this file
    pub const BASELINE_DIR: &str = "crates/omega/test_fixtures/visual_tests";

    /// Embedded test image (Zed app icon) for visual tests.
    pub const EMBEDDED_TEST_IMAGE: &[u8] = include_bytes!("../resources/app-icon.png");

    /// omega#99. How many scheduler steps one bounded wait in the Exo capture
    /// is allowed. Large enough that a real turn's work always fits, small
    /// enough that a permanently-runnable transport cannot hold the wait open.
    pub const SCHEDULER_STEP_BUDGET: usize = 20_000;

    /// Tooltip show delay - must match TOOLTIP_SHOW_DELAY in gpui/src/elements/div.rs
    pub const TOOLTIP_SHOW_DELAY: Duration = Duration::from_millis(500);
}

#[cfg(target_os = "macos")]
use constants::*;

#[cfg(target_os = "macos")]
#[derive(Default)]
struct SceneEvidence {
    semantic_checks: Vec<ProofCheck>,
    pixel: Option<PixelProof>,
}

#[cfg(target_os = "macos")]
struct WorkbenchProofSession {
    selected: BTreeSet<String>,
    phase: ScenePhase,
    recording_succeeded: bool,
    lane: ProofLane,
    seed: u64,
    output_root: PathBuf,
    evidence: BTreeMap<String, SceneEvidence>,
}

#[cfg(target_os = "macos")]
thread_local! {
    static WORKBENCH_PROOF_SESSION: RefCell<Option<WorkbenchProofSession>> = const { RefCell::new(None) };
}

#[cfg(target_os = "macos")]
fn parse_optional_usize(name: &str) -> Result<Option<usize>> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => value
            .parse()
            .with_context(|| format!("{name} must be a non-negative integer"))
            .map(Some),
        Ok(_) | Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {name}")),
    }
}

#[cfg(target_os = "macos")]
fn proof_seed() -> Result<u64> {
    match std::env::var("SEED") {
        Ok(value) => value
            .parse()
            .with_context(|| format!("SEED must be an unsigned integer, got {value:?}")),
        Err(std::env::VarError::NotPresent) => Ok(0),
        Err(error) => Err(error).context("reading SEED"),
    }
}

#[cfg(target_os = "macos")]
fn remove_previous_proof_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("removing stale proof {}", path.display()))
        }
    }
}

#[cfg(target_os = "macos")]
fn initialize_workbench_proof() -> Result<()> {
    if std::env::var("OMEGA_WORKBENCH_PROOF").is_err() {
        return Ok(());
    }
    if std::env::var("UPDATE_BASELINE").is_ok()
        && (std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok())
    {
        anyhow::bail!("baseline updates are disabled in CI");
    }

    let requested = std::env::var("OMEGA_WORKBENCH_SCENE").ok();
    let shard_index = parse_optional_usize("OMEGA_WORKBENCH_SHARD_INDEX")?;
    let shard_count = parse_optional_usize("OMEGA_WORKBENCH_SHARD_COUNT")?;
    // OMEGA-DELTA-0185. The proof command runs the structurally sealed
    // zero-base scenes one per process — `omega_zero_base::seal()` is
    // process-global and one-way, and the omega-agent scene family sizes its
    // shared window per selected scene — while every other scene keeps its
    // original single-batch process whose fixture state is sequential. The
    // skip list is how the batch invocation excludes the per-process scenes.
    let skipped: BTreeSet<String> = std::env::var("OMEGA_WORKBENCH_SKIP_SCENES")
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let selected: BTreeSet<String> = select_scenes(requested.as_deref(), shard_index, shard_count)?
        .into_iter()
        .map(|scene| scene.name.to_string())
        .filter(|name| !skipped.contains(name))
        .collect();
    let phase = if std::env::var("OMEGA_VISUAL_PHASE").as_deref() == Ok("restart") {
        ScenePhase::Restart
    } else {
        ScenePhase::Recording
    };
    let recording_succeeded =
        std::env::var("OMEGA_WORKBENCH_RECORDING_SUCCEEDED").as_deref() != Ok("0");
    let lane = match std::env::var("OMEGA_WORKBENCH_LANE").as_deref() {
        Ok("semantic") => ProofLane::Semantic,
        Ok("pixel") | Err(_) => ProofLane::Pixel,
        Ok(other) => anyhow::bail!("unknown workbench proof lane {other:?}"),
    };
    let output_root = std::env::var("OMEGA_WORKBENCH_OUTPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target/omega-workbench-proof"));
    for scene in HERMETIC_SCENES
        .iter()
        .filter(|scene| scene.phase == phase && selected.contains(scene.name))
    {
        let scene_output = output_root.join("scenes").join(scene.name);
        for file_name in ["receipt.json", "baseline.png", "current.png", "diff.png"] {
            remove_previous_proof_file(&scene_output.join(file_name))?;
        }
        for region in scene.regions {
            let region_output = scene_output.join("regions");
            for file_name in [
                format!("{}_baseline.png", region.name),
                format!("{}.png", region.name),
                format!("{}_diff.png", region.name),
            ] {
                remove_previous_proof_file(&region_output.join(file_name))?;
            }
        }
    }

    WORKBENCH_PROOF_SESSION.with(|session| {
        *session.borrow_mut() = Some(WorkbenchProofSession {
            selected,
            phase,
            recording_succeeded,
            lane,
            seed: proof_seed()?,
            output_root,
            evidence: BTreeMap::new(),
        });
        Ok(())
    })
}

#[cfg(target_os = "macos")]
fn workbench_proof_active() -> bool {
    WORKBENCH_PROOF_SESSION.with(|session| session.borrow().is_some())
}

#[cfg(target_os = "macos")]
fn workbench_has_selected_scene_in_phase() -> bool {
    WORKBENCH_PROOF_SESSION.with(|session| {
        let session = session.borrow();
        let Some(session) = session.as_ref() else {
            return false;
        };
        HERMETIC_SCENES.iter().any(|scene| {
            session.selected.contains(scene.name)
                && (scene.phase == session.phase
                    || (session.phase == ScenePhase::Recording
                        && scene.phase == ScenePhase::Restart))
        })
    })
}

#[cfg(target_os = "macos")]
fn workbench_any_selected(names: &[&str]) -> bool {
    WORKBENCH_PROOF_SESSION.with(|session| {
        let session = session.borrow();
        let Some(session) = session.as_ref() else {
            return true;
        };
        names.iter().any(|name| session.selected.contains(*name))
    })
}

#[cfg(target_os = "macos")]
fn workbench_should_run_scene(name: &str) -> bool {
    WORKBENCH_PROOF_SESSION.with(|session| {
        let session = session.borrow();
        let Some(session) = session.as_ref() else {
            return true;
        };
        scene_spec(name).is_some_and(|scene| {
            scene.phase == session.phase && session.selected.contains(scene.name)
        })
    })
}

#[cfg(target_os = "macos")]
fn workbench_semantic_only() -> bool {
    WORKBENCH_PROOF_SESSION.with(|session| {
        session
            .borrow()
            .as_ref()
            .is_some_and(|session| session.lane == ProofLane::Semantic)
    })
}

#[cfg(target_os = "macos")]
fn record_workbench_semantic_check(scene: &str, check: &str) {
    if !workbench_should_run_scene(scene) {
        return;
    }
    WORKBENCH_PROOF_SESSION.with(|session| {
        let mut session = session.borrow_mut();
        let Some(session) = session.as_mut() else {
            return;
        };
        session
            .evidence
            .entry(scene.to_string())
            .or_default()
            .semantic_checks
            .push(ProofCheck::passed(check));
    });
}

#[cfg(target_os = "macos")]
fn record_workbench_semantic_failure(scene: &str, check: &str, detail: impl Into<String>) {
    if !workbench_should_run_scene(scene) {
        return;
    }
    WORKBENCH_PROOF_SESSION.with(|session| {
        let mut session = session.borrow_mut();
        let Some(session) = session.as_mut() else {
            return;
        };
        session
            .evidence
            .entry(scene.to_string())
            .or_default()
            .semantic_checks
            .push(ProofCheck::failed(check, detail));
    });
}

#[cfg(target_os = "macos")]
fn record_workbench_semantic_checks(scene: &str, checks: Vec<ProofCheck>) {
    if !workbench_should_run_scene(scene) {
        return;
    }
    WORKBENCH_PROOF_SESSION.with(|session| {
        let mut session = session.borrow_mut();
        let Some(session) = session.as_mut() else {
            return;
        };
        session
            .evidence
            .entry(scene.to_string())
            .or_default()
            .semantic_checks
            .extend(checks);
    });
}

#[cfg(target_os = "macos")]
fn record_workbench_pixel(scene: &str, pixel: PixelProof) {
    WORKBENCH_PROOF_SESSION.with(|session| {
        let mut session = session.borrow_mut();
        let Some(session) = session.as_mut() else {
            return;
        };
        session.evidence.entry(scene.to_string()).or_default().pixel = Some(pixel);
    });
}

#[cfg(target_os = "macos")]
fn is_workbench_files_scene(name: &str) -> bool {
    matches!(
        name,
        "omega_workbench_files_wide"
            | "omega_workbench_files_narrow"
            | "omega_workbench_files_multi_root"
            | "omega_workbench_files_empty"
            | "omega_workbench_files_loading"
            | "omega_workbench_files_error"
            | "omega_workbench_files_stale_filesystem_completion"
    )
}

#[cfg(target_os = "macos")]
fn is_workbench_search_scene(name: &str) -> bool {
    matches!(
        name,
        "omega_workbench_search_empty"
            | "omega_workbench_search_populated"
            | "omega_workbench_search_no_results"
            | "omega_workbench_search_invalid_regex"
            | "omega_workbench_search_loading"
            | "omega_workbench_search_narrow"
            | "omega_workbench_search_focused_result"
            | "omega_workbench_search_error"
    )
}

#[cfg(target_os = "macos")]
fn is_workbench_review_scene(name: &str) -> bool {
    matches!(
        name,
        "omega_workbench_review_empty"
            | "omega_workbench_review_multi_file"
            | "omega_workbench_review_selected_hunk"
            | "omega_workbench_review_streaming_update"
            | "omega_workbench_review_rename_delete"
            | "omega_workbench_review_conflict"
            | "omega_workbench_review_all_reviewed"
            | "omega_workbench_review_narrow"
            | "omega_workbench_review_error"
    )
}

#[cfg(target_os = "macos")]
fn is_workbench_git_scene(name: &str) -> bool {
    matches!(
        name,
        "omega_workbench_git_clean"
            | "omega_workbench_git_dirty"
            | "omega_workbench_git_staged"
            | "omega_workbench_git_conflict"
            | "omega_workbench_git_detached"
            | "omega_workbench_git_unborn"
            | "omega_workbench_git_pending"
            | "omega_workbench_git_multi_repository"
            | "omega_workbench_git_repository_removed"
            | "omega_workbench_git_offline"
            | "omega_workbench_git_reconnect"
            | "omega_workbench_git_error"
    )
}

#[cfg(target_os = "macos")]
fn is_workbench_terminal_scene(name: &str) -> bool {
    WORKBENCH_TERMINAL_PIXEL_SCENES.contains(&name)
}

#[cfg(target_os = "macos")]
fn is_workbench_plan_scene(name: &str) -> bool {
    WORKBENCH_PLAN_PIXEL_SCENES.contains(&name)
}

#[cfg(target_os = "macos")]
fn workbench_fixture_for_scene(name: &str) -> Result<WorkbenchScene> {
    use omega_workbench_harness::{
        ContentStateFixture, EventFixture, EventKindFixture, MessageFixture, MessageRoleFixture,
        MessageStateFixture, PersistedSceneFixture, ProjectFixture, RepositoryFixture,
        ThreadFixture, WorktreeFixture,
    };

    if is_workbench_review_scene(name) {
        return omega_workbench_harness::workbench_review_scene(name);
    }
    if is_workbench_git_scene(name) {
        return omega_workbench_harness::workbench_git_scene(name);
    }
    if is_workbench_terminal_scene(name) {
        return omega_workbench_harness::workbench_terminal_scene(name);
    }
    if is_workbench_plan_scene(name) {
        return omega_workbench_harness::workbench_plan_scene(name);
    }
    let spec =
        scene_spec(name).ok_or_else(|| anyhow::anyhow!("unknown workbench scene {name:?}"))?;
    let mut scene = spec.fixture();
    scene.threads.push(ThreadFixture {
        id: "active-thread".to_string(),
        project_id: None,
        repository_id: None,
        worktree_id: None,
    });
    scene.active_thread_id = Some("active-thread".to_string());
    scene.content_state = ContentStateFixture::Ready;

    if WORKBENCH_SHELL_PIXEL_SCENES.contains(&name) {
        let identity_scene = name.starts_with("omega_workbench_identity_");
        if is_workbench_files_scene(name) {
            for surface in &mut scene.surfaces {
                surface.available =
                    !matches!(surface.id, WorkSurfaceId::Review | WorkSurfaceId::Git);
            }
            scene.project = Some(ProjectFixture {
                id: "visual-project".into(),
                display_name: "Omega".into(),
            });
            let worktrees = if matches!(
                name,
                "omega_workbench_files_multi_root"
                    | "omega_workbench_files_stale_filesystem_completion"
            ) {
                vec![
                    WorktreeFixture {
                        id: "alpha-worktree".into(),
                        branch: Some("main".into()),
                        git_state: None,
                        dirty_files: 0,
                        conflicts: 0,
                        ahead: 0,
                        behind: 0,
                    },
                    WorktreeFixture {
                        id: "beta-worktree".into(),
                        branch: Some("main".into()),
                        git_state: None,
                        dirty_files: 0,
                        conflicts: 0,
                        ahead: 0,
                        behind: 0,
                    },
                ]
            } else {
                vec![WorktreeFixture {
                    id: if name == "omega_workbench_files_empty" {
                        "empty-worktree".into()
                    } else {
                        "ready-worktree".into()
                    },
                    branch: Some("main".into()),
                    git_state: None,
                    dirty_files: 0,
                    conflicts: 0,
                    ahead: 0,
                    behind: 0,
                }]
            };
            let active_worktree_id = worktrees
                .last()
                .context("Files scene has no worktree fixture")?
                .id
                .clone();
            scene.repositories.push(RepositoryFixture {
                id: "visual-repository".into(),
                project_id: "visual-project".into(),
                worktrees,
            });
            scene.threads[0].project_id = Some("visual-project".into());
            scene.threads[0].repository_id = Some("visual-repository".into());
            scene.threads[0].worktree_id = Some(active_worktree_id);
            scene.active_surface = Some(WorkSurfaceId::Files);
            scene.dock_open = true;
            scene.content_state = match name {
                "omega_workbench_files_empty" => ContentStateFixture::Empty,
                "omega_workbench_files_loading" => ContentStateFixture::Loading,
                "omega_workbench_files_error" => {
                    ContentStateFixture::Error("Could not load Files".into())
                }
                _ => ContentStateFixture::Ready,
            };
        } else if is_workbench_search_scene(name) {
            for surface in &mut scene.surfaces {
                surface.available =
                    !matches!(surface.id, WorkSurfaceId::Review | WorkSurfaceId::Git);
            }
            scene.project = Some(ProjectFixture {
                id: "visual-project".into(),
                display_name: "Omega".into(),
            });
            scene.repositories.push(RepositoryFixture {
                id: "visual-repository".into(),
                project_id: "visual-project".into(),
                worktrees: vec![
                    WorktreeFixture {
                        id: "alpha-worktree".into(),
                        branch: Some("main".into()),
                        git_state: None,
                        dirty_files: 0,
                        conflicts: 0,
                        ahead: 0,
                        behind: 0,
                    },
                    WorktreeFixture {
                        id: "beta-worktree".into(),
                        branch: Some("main".into()),
                        git_state: None,
                        dirty_files: 0,
                        conflicts: 0,
                        ahead: 0,
                        behind: 0,
                    },
                ],
            });
            scene.threads[0].project_id = Some("visual-project".into());
            scene.threads[0].repository_id = Some("visual-repository".into());
            scene.threads[0].worktree_id = Some("beta-worktree".into());
            scene.active_surface = Some(WorkSurfaceId::Search);
            scene.dock_open = true;
            scene.content_state = match name {
                "omega_workbench_search_loading" => ContentStateFixture::Loading,
                "omega_workbench_search_error" => {
                    ContentStateFixture::Error("Could not search this worktree".into())
                }
                _ => ContentStateFixture::Ready,
            };
        } else if identity_scene {
            for surface in &mut scene.surfaces {
                surface.available = name != "omega_workbench_identity_offline_error"
                    || matches!(surface.id, WorkSurfaceId::Terminal | WorkSurfaceId::Plan);
            }
            scene.project = Some(ProjectFixture {
                id: "visual-project".into(),
                display_name: "Omega".into(),
            });
            scene.repositories.push(RepositoryFixture {
                id: "visual-repository".into(),
                project_id: "visual-project".into(),
                worktrees: vec![WorktreeFixture {
                    id: "visual-worktree".into(),
                    branch: Some("main".into()),
                    git_state: None,
                    dirty_files: 0,
                    conflicts: 0,
                    ahead: 0,
                    behind: 0,
                }],
            });
            scene.threads[0].project_id = Some("visual-project".into());
            scene.threads[0].repository_id = Some("visual-repository".into());
            scene.threads[0].worktree_id = Some("visual-worktree".into());
            if name == "omega_workbench_identity_dirty_conflict" {
                let worktree = &mut scene.repositories[0].worktrees[0];
                worktree.dirty_files = 4;
                worktree.conflicts = 2;
                worktree.ahead = 3;
                worktree.behind = 1;
                scene
                    .surfaces
                    .iter_mut()
                    .find(|surface| surface.id == WorkSurfaceId::Git)
                    .context("identity scene has no Git surface")?
                    .badge = Some(4);
            }
        } else {
            for surface in &mut scene.surfaces {
                surface.available = surface.id == WorkSurfaceId::Plan;
            }
        }
        match name {
            "omega_workbench_shell_active_dock" => {
                scene.active_surface = Some(WorkSurfaceId::Plan);
                scene.dock_open = true;
            }
            "omega_workbench_shell_typed_badge" => {
                let plan = scene
                    .surfaces
                    .iter_mut()
                    .find(|surface| surface.id == WorkSurfaceId::Plan)
                    .context("workbench scene has no Plan surface fixture")?;
                plan.badge = Some(3);
            }
            "omega_workbench_shell_narrow" | "omega_workbench_shell_collapsed_after_open" => {
                scene.active_surface = Some(WorkSurfaceId::Plan);
            }
            _ => {}
        }
    }

    if name == "omega_front_door_typing" {
        scene.messages.push(MessageFixture {
            id: "typed-draft".to_string(),
            thread_id: "active-thread".to_string(),
            role: MessageRoleFixture::User,
            state: MessageStateFixture::Complete,
        });
    }
    if name.contains("executor_disclosure") || name.contains("route_pin") {
        scene.events.push(EventFixture {
            id: format!("event-{name}"),
            thread_id: "active-thread".to_string(),
            revision: 1,
            kind: if name.contains("route_pin") {
                EventKindFixture::RouteDecision
            } else {
                EventKindFixture::ExecutorDisclosure
            },
        });
    }
    if spec.phase == ScenePhase::Restart {
        scene.persisted = Some(PersistedSceneFixture {
            requested_surface: None,
            dock_open: false,
            revision: 1,
            mutations_before_restart: Vec::new(),
        });
    }
    scene.validate()?;
    Ok(scene)
}

#[cfg(target_os = "macos")]
fn proof_artifact_paths(scene: &str) -> (PathBuf, PathBuf, PathBuf) {
    (
        PathBuf::from("scenes").join(scene).join("baseline.png"),
        PathBuf::from("scenes").join(scene).join("current.png"),
        PathBuf::from("scenes").join(scene).join("diff.png"),
    )
}

#[cfg(target_os = "macos")]
fn finalize_workbench_proof(run_succeeded: bool) -> Result<()> {
    let Some(mut session) = WORKBENCH_PROOF_SESSION.with(|session| session.borrow_mut().take())
    else {
        return Ok(());
    };

    for scene in HERMETIC_SCENES
        .iter()
        .filter(|scene| scene.phase == session.phase && session.selected.contains(scene.name))
    {
        let fixture = workbench_fixture_for_scene(scene.name)?;
        let mut receipt = ProofReceipt::new(&fixture, session.seed, session.lane.clone())?;
        let evidence = session.evidence.remove(scene.name).unwrap_or_default();
        receipt.semantic_checks = evidence.semantic_checks;
        if receipt.semantic_checks.is_empty() {
            receipt.semantic_checks.push(ProofCheck::failed(
                "scene-reached-semantic-preflight",
                "the selected scene never reached its semantic assertion boundary",
            ));
        }
        if !run_succeeded
            && !receipt
                .semantic_checks
                .iter()
                .any(|check| check.status == CheckStatus::Failed)
        {
            receipt.semantic_checks.push(ProofCheck::failed(
                "proof-run-completed",
                "the visual runner failed before the selected scene completed",
            ));
        }
        if session.phase == ScenePhase::Restart && !session.recording_succeeded {
            receipt.semantic_checks.push(ProofCheck::failed(
                "recording-phase-completed",
                "the prerequisite recording process failed",
            ));
        }

        receipt.pixel = if receipt.lane == ProofLane::Pixel {
            let (baseline, current, _) = proof_artifact_paths(scene.name);
            Some(evidence.pixel.unwrap_or(PixelProof {
                status: PixelStatus::Failed,
                minimum_match: scene.pixel_policy.minimum_match,
                channel_tolerance: scene.pixel_policy.channel_tolerance,
                policy_rationale: scene.pixel_policy.rationale.to_string(),
                match_percentage: None,
                different_pixels: None,
                total_pixels: None,
                baseline,
                current,
                diff: None,
                regions: Vec::new(),
            }))
        } else {
            None
        };
        let failed = receipt
            .semantic_checks
            .iter()
            .any(|check| check.status == CheckStatus::Failed)
            || receipt.pixel.as_ref().is_some_and(|pixel| {
                pixel.status == PixelStatus::Failed
                    || pixel
                        .regions
                        .iter()
                        .any(|region| region.status == PixelStatus::Failed)
            });
        receipt.outcome = if failed {
            ProofOutcome::Failed
        } else {
            ProofOutcome::Passed
        };
        let receipt_path = session
            .output_root
            .join("scenes")
            .join(scene.name)
            .join("receipt.json");
        receipt.write_json(&receipt_path)?;
        println!("  Receipt saved to: {}", receipt_path.display());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_visual_tests(project_path: PathBuf, update_baseline: bool) -> Result<()> {
    // Create the visual test context with deterministic task scheduling
    // Use real Assets so that SVG icons render properly
    let mut cx = VisualTestAppContext::with_asset_source(
        gpui_platform::current_platform(false),
        Arc::new(Assets),
    );

    // Load embedded fonts (IBM Plex Sans, Lilex, etc.) so UI renders with correct fonts
    cx.update(|cx| {
        Assets.load_fonts(cx).unwrap();
    });

    // Initialize settings store with real default settings (not test settings)
    // Test settings use Courier font, but we want the real Zed fonts for visual tests
    cx.update(|cx| {
        settings::init(cx);
    });

    // Create AppState using the test initialization
    let app_state = cx.update(|cx| init_app_state(cx));

    // Set the global app state so settings_ui and other subsystems can find it
    cx.update(|cx| {
        AppState::set_global(app_state.clone(), cx);
    });

    // Initialize all Zed subsystems
    cx.update(|cx| {
        gpui_tokio::init(cx);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        client::init(&app_state.client, cx);
        audio::init(cx);
        workspace::init(app_state.clone(), cx);
        release_channel::init(semver::Version::new(0, 0, 0), cx);
        command_palette::init(cx);
        editor::init(cx);
        title_bar::init(cx);
        account_ui::init(cx);
        project_panel::init(cx);
        terminal_view::init(cx);
        search::init(cx);
        lsp_locations::init(cx);
        cx.set_global(workspace::PaneSearchBarCallbacks {
            setup_search_bar: |languages, toolbar, window, cx| {
                let search_bar = cx.new(|cx| search::BufferSearchBar::new(languages, window, cx));
                toolbar.update(cx, |toolbar, cx| {
                    toolbar.add_item(search_bar, window, cx);
                });
            },
            wrap_div_with_search_actions: search::buffer_search::register_pane_search_actions,
        });
        prompt_store::init(cx);
        let prompt_builder = prompt_store::PromptBuilder::load(app_state.fs.clone(), false, cx);
        language_model::init(cx);
        client::RefreshLlmTokenListener::register(
            app_state.client.clone(),
            app_state.user_store.clone(),
            cx,
        );
        language_models::init(app_state.user_store.clone(), app_state.client.clone(), cx);
        // omega#161. The runner deliberately does NOT initialize the hosted
        // OpenAgents session global: a proof process must not attempt a live
        // hosted sign-in, and the providers read the absence as "no hosted
        // lane in this process" and use their direct-key fallback path.
        git_ui::init(cx);
        project::AgentRegistryStore::init_global(
            cx,
            app_state.fs.clone(),
            app_state.client.http_client(),
        );
        agent_ui::init(
            app_state.fs.clone(),
            prompt_builder,
            app_state.languages.clone(),
            true,
            false,
            cx,
        );
        settings_ui::init(cx);

        // Load default keymaps so tooltips can show keybindings like "f9" for ToggleBreakpoint
        // We load a minimal set of editor keybindings needed for visual tests
        cx.bind_keys([KeyBinding::new(
            "f9",
            editor::actions::ToggleBreakpoint,
            Some("Editor"),
        )]);

        // Disable agent notifications during visual tests to avoid popup windows
        agent_settings::AgentSettings::override_global(
            agent_settings::AgentSettings {
                notify_when_agent_waiting: NotifyWhenAgentWaiting::Never,
                play_sound_when_agent_done: PlaySoundWhenAgentDone::Never,
                ..agent_settings::AgentSettings::get_global(cx).clone()
            },
            cx,
        );
    });

    // Run until all initialization tasks complete
    cx.run_until_parked();

    // Open workspace window
    let window_size = size(px(1280.0), px(800.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };

    // Create a project for the workspace
    let project = cx.update(|cx| {
        project::Project::local(
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            app_state.user_store.clone(),
            app_state.languages.clone(),
            app_state.fs.clone(),
            None,
            project::LocalProjectFlags {
                init_worktree_trust: false,
                ..Default::default()
            },
            cx,
        )
    });

    let workspace_window: WindowHandle<Workspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        Workspace::new(None, project.clone(), app_state.clone(), window, cx)
                    })
                },
            )
        })
        .context("Failed to open workspace window")?;

    cx.run_until_parked();

    // Add the test project as a worktree
    let add_worktree_task = workspace_window
        .update(&mut cx, |workspace, _window, cx| {
            let project = workspace.project().clone();
            project.update(cx, |project, cx| {
                project.find_or_create_worktree(&project_path, true, cx)
            })
        })
        .context("Failed to start adding worktree")?;

    // Use block_test to wait for the worktree task
    // block_test runs both foreground and background tasks, which is needed because
    // worktree creation spawns foreground tasks via cx.spawn
    // Allow parking since filesystem operations happen outside the test dispatcher
    cx.background_executor.allow_parking();
    let worktree_result = cx.foreground_executor.block_test(add_worktree_task);
    cx.background_executor.forbid_parking();
    worktree_result.context("Failed to add worktree")?;

    cx.run_until_parked();

    // Create and add the project panel
    let (weak_workspace, async_window_cx) = workspace_window
        .update(&mut cx, |workspace, window, cx| {
            (workspace.weak_handle(), window.to_async(cx))
        })
        .context("Failed to get workspace handle")?;

    cx.background_executor.allow_parking();
    let panel = cx
        .foreground_executor
        .block_test(ProjectPanel::load(weak_workspace, async_window_cx))
        .context("Failed to load project panel")?;
    cx.background_executor.forbid_parking();

    workspace_window
        .update(&mut cx, |workspace, window, cx| {
            workspace.add_panel(panel, window, cx);
        })
        .log_err();

    cx.run_until_parked();

    // Open the project panel
    workspace_window
        .update(&mut cx, |workspace, window, cx| {
            workspace.open_panel::<ProjectPanel>(window, cx);
        })
        .log_err();

    cx.run_until_parked();

    // Open main.rs in the editor
    let open_file_task = workspace_window
        .update(&mut cx, |workspace, window, cx| {
            let worktree = workspace.project().read(cx).worktrees(cx).next();
            if let Some(worktree) = worktree {
                let worktree_id = worktree.read(cx).id();
                let rel_path: std::sync::Arc<util::rel_path::RelPath> =
                    util::rel_path::rel_path("src/main.rs").into();
                let project_path: project::ProjectPath = (worktree_id, rel_path).into();
                Some(workspace.open_path(project_path, None, true, window, cx))
            } else {
                None
            }
        })
        .log_err()
        .flatten();

    if let Some(task) = open_file_task {
        cx.background_executor.allow_parking();
        let block_result = cx.foreground_executor.block_test(task);
        cx.background_executor.forbid_parking();
        if let Ok(item) = block_result {
            workspace_window
                .update(&mut cx, |workspace, window, cx| {
                    let pane = workspace.active_pane().clone();
                    pane.update(cx, |pane, cx| {
                        if let Some(index) = pane.index_for_item(item.as_ref()) {
                            pane.activate_item(index, true, true, window, cx);
                        }
                    });
                })
                .log_err();
        }
    }

    cx.run_until_parked();

    // Request a window refresh
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.refresh();
    })
    .log_err();

    cx.run_until_parked();

    // Track test results
    let mut passed = 0;
    let mut failed = 0;
    let mut updated = 0;

    // `OMEGA_EXO_VISUAL_ONLY=1` runs the real Exo conversation workspace
    // proof and nothing else. The lane file must name an isolated Exo install.
    // This path connects over the shipped ACP stdio transport and sends a real
    // turn before it captures the wide and narrow layouts.
    #[cfg(feature = "visual-tests")]
    if std::env::var("OMEGA_EXO_VISUAL_ONLY").is_ok() {
        println!("\n--- Omega: real Exo conversation workspace ---");
        let outcome = run_omega_exo_visual_tests(app_state.clone(), &mut cx, update_baseline);
        teardown_shared_window(workspace_window, &mut cx);
        return match outcome {
            Ok(TestResult::Passed) => {
                println!("\u{2713} omega_exo_workspace: PASSED");
                Ok(())
            }
            Ok(TestResult::BaselineUpdated(_)) => {
                println!("\u{2713} omega_exo_workspace: Baselines updated");
                Ok(())
            }
            Err(error) => Err(error.context("omega_exo_workspace")),
        };
    }

    // `OMEGA_VISUAL_ONLY=1` runs Omega's own suite and nothing else.
    //
    // This runner has no committed baselines for any of the inherited Zed
    // tests, so a plain run reports "Baseline not found" for every one of them
    // and Omega's result is buried in the noise. Generating baselines for all
    // of them would be committing a large set of images nobody has looked at,
    // which is the opposite of the point. So the Omega suite gets its own
    // invocation, `script/omega-visual-proof`, and the inherited tests keep the
    // status they already had: present, and not standing up.
    #[cfg(feature = "visual-tests")]
    if std::env::var("OMEGA_VISUAL_ONLY").is_ok() {
        // `OMEGA_VISUAL_PHASE=restart` is the second process of omega#77's
        // restart proof. It captures nothing the first process captured: it
        // reopens two threads the first process left on disk and photographs
        // the executor lines a cold process derives for them.
        let restart_phase = std::env::var("OMEGA_VISUAL_PHASE").as_deref() == Ok("restart");
        let outcome = if restart_phase {
            println!("\n--- Omega: executor disclosure after a restart ---");
            run_omega_restart_visual_tests(app_state.clone(), &mut cx, update_baseline)
        } else {
            println!("\n--- Omega: deterministic recording scenes ---");
            run_omega_recording_visual_tests(app_state.clone(), &mut cx, update_baseline)
        };
        // The shared window this function opened above is torn down here as
        // well as at the end of the full run. Returning early without it left
        // the sample project's buffers alive, GPUI's leaked-handle check
        // panicked at drop, and a green suite exited 101 — a script reporting
        // failure on a run where every capture matched.
        teardown_shared_window(workspace_window, &mut cx);
        return match outcome {
            Ok(TestResult::Passed) => {
                println!("\u{2713} omega_agent_surfaces: PASSED");
                Ok(())
            }
            Ok(TestResult::BaselineUpdated(_)) => {
                println!("\u{2713} omega_agent_surfaces: Baselines updated");
                Ok(())
            }
            Err(error) => Err(error.context("omega_agent_surfaces")),
        };
    }

    // Run Test 1: Project Panel (with project panel visible)
    println!("\n--- Test 1: project_panel ---");
    match run_visual_test(
        "project_panel",
        workspace_window.into(),
        &mut cx,
        update_baseline,
    ) {
        Ok(TestResult::Passed) => {
            println!("✓ project_panel: PASSED");
            passed += 1;
        }
        Ok(TestResult::BaselineUpdated(_)) => {
            println!("✓ project_panel: Baseline updated");
            updated += 1;
        }
        Err(e) => {
            eprintln!("✗ project_panel: FAILED - {}", e);
            failed += 1;
        }
    }

    // Run Test 2: Workspace with Editor
    println!("\n--- Test 2: workspace_with_editor ---");

    // Close project panel for this test
    workspace_window
        .update(&mut cx, |workspace, window, cx| {
            workspace.close_panel::<ProjectPanel>(window, cx);
        })
        .log_err();

    cx.run_until_parked();

    match run_visual_test(
        "workspace_with_editor",
        workspace_window.into(),
        &mut cx,
        update_baseline,
    ) {
        Ok(TestResult::Passed) => {
            println!("✓ workspace_with_editor: PASSED");
            passed += 1;
        }
        Ok(TestResult::BaselineUpdated(_)) => {
            println!("✓ workspace_with_editor: Baseline updated");
            updated += 1;
        }
        Err(e) => {
            eprintln!("✗ workspace_with_editor: FAILED - {}", e);
            failed += 1;
        }
    }

    // Run Test: ThreadItem branch names visual test
    println!("\n--- Test: thread_item_branch_names ---");
    match run_thread_item_branch_name_visual_tests(app_state.clone(), &mut cx, update_baseline) {
        Ok(TestResult::Passed) => {
            println!("✓ thread_item_branch_names: PASSED");
            passed += 1;
        }
        Ok(TestResult::BaselineUpdated(_)) => {
            println!("✓ thread_item_branch_names: Baseline updated");
            updated += 1;
        }
        Err(e) => {
            eprintln!("✗ thread_item_branch_names: FAILED - {}", e);
            failed += 1;
        }
    }

    // Run Test 4: Error wrapping visual tests
    println!("\n--- Test 4: error_message_wrapping ---");
    match run_error_wrapping_visual_tests(app_state.clone(), &mut cx, update_baseline) {
        Ok(TestResult::Passed) => {
            println!("✓ error_message_wrapping: PASSED");
            passed += 1;
        }
        Ok(TestResult::BaselineUpdated(_)) => {
            println!("✓ error_message_wrapping: Baselines updated");
            updated += 1;
        }
        Err(e) => {
            eprintln!("✗ error_message_wrapping: FAILED - {}", e);
            failed += 1;
        }
    }

    // Run Test 5: Agent Thread View tests
    #[cfg(feature = "visual-tests")]
    {
        println!("\n--- Test 5: agent_thread_with_image (collapsed + expanded) ---");
        match run_agent_thread_view_test(app_state.clone(), &mut cx, update_baseline) {
            Ok(TestResult::Passed) => {
                println!("✓ agent_thread_with_image (collapsed + expanded): PASSED");
                passed += 1;
            }
            Ok(TestResult::BaselineUpdated(_)) => {
                println!("✓ agent_thread_with_image: Baselines updated (collapsed + expanded)");
                updated += 1;
            }
            Err(e) => {
                eprintln!("✗ agent_thread_with_image: FAILED - {}", e);
                failed += 1;
            }
        }
    }

    // Run Test 6: Breakpoint Hover visual tests
    println!("\n--- Test 6: breakpoint_hover (3 variants) ---");
    match run_breakpoint_hover_visual_tests(app_state.clone(), &mut cx, update_baseline) {
        Ok(TestResult::Passed) => {
            println!("✓ breakpoint_hover: PASSED");
            passed += 1;
        }
        Ok(TestResult::BaselineUpdated(_)) => {
            println!("✓ breakpoint_hover: Baselines updated");
            updated += 1;
        }
        Err(e) => {
            eprintln!("✗ breakpoint_hover: FAILED - {}", e);
            failed += 1;
        }
    }

    // Run Test 7: Diff Review Button visual tests
    println!("\n--- Test 7: diff_review_button (3 variants) ---");
    match run_diff_review_visual_tests(app_state.clone(), &mut cx, update_baseline) {
        Ok(TestResult::Passed) => {
            println!("✓ diff_review_button: PASSED");
            passed += 1;
        }
        Ok(TestResult::BaselineUpdated(_)) => {
            println!("✓ diff_review_button: Baselines updated");
            updated += 1;
        }
        Err(e) => {
            eprintln!("✗ diff_review_button: FAILED - {}", e);
            failed += 1;
        }
    }

    // Run Test 8: ThreadItem icon decorations visual tests
    println!("\n--- Test 8: thread_item_icon_decorations ---");
    match run_thread_item_icon_decorations_visual_tests(app_state.clone(), &mut cx, update_baseline)
    {
        Ok(TestResult::Passed) => {
            println!("✓ thread_item_icon_decorations: PASSED");
            passed += 1;
        }
        Ok(TestResult::BaselineUpdated(_)) => {
            println!("✓ thread_item_icon_decorations: Baseline updated");
            updated += 1;
        }
        Err(e) => {
            eprintln!("✗ thread_item_icon_decorations: FAILED - {}", e);
            failed += 1;
        }
    }

    // Run Test 9: Tool Permissions Settings UI visual test
    println!("\n--- Test 9: tool_permissions_settings ---");
    match run_tool_permissions_visual_tests(app_state.clone(), &mut cx, update_baseline) {
        Ok(TestResult::Passed) => {
            println!("✓ tool_permissions_settings: PASSED");
            passed += 1;
        }
        Ok(TestResult::BaselineUpdated(_)) => {
            println!("✓ tool_permissions_settings: Baselines updated");
            updated += 1;
        }
        Err(e) => {
            eprintln!("✗ tool_permissions_settings: FAILED - {}", e);
            failed += 1;
        }
    }

    // Run Test 10: Settings UI sub-page auto-open visual tests
    println!("\n--- Test 10: settings_ui_subpage_auto_open (2 variants) ---");
    match run_settings_ui_subpage_visual_tests(app_state.clone(), &mut cx, update_baseline) {
        Ok(TestResult::Passed) => {
            println!("✓ settings_ui_subpage_auto_open: PASSED");
            passed += 1;
        }
        Ok(TestResult::BaselineUpdated(_)) => {
            println!("✓ settings_ui_subpage_auto_open: Baselines updated");
            updated += 1;
        }
        Err(e) => {
            eprintln!("✗ settings_ui_subpage_auto_open: FAILED - {}", e);
            failed += 1;
        }
    }

    // Run Test 11: External agent harness maintenance (omega#81)
    println!("\n--- Test 11: external_agent_harness_maintenance ---");
    match run_external_agent_maintenance_visual_tests(app_state.clone(), &mut cx, update_baseline) {
        Ok(TestResult::Passed) => {
            println!("\u{2713} external_agent_harness_maintenance: PASSED");
            passed += 1;
        }
        Ok(TestResult::BaselineUpdated(_)) => {
            println!("\u{2713} external_agent_harness_maintenance: Baselines updated");
            updated += 1;
        }
        Err(e) => {
            eprintln!(
                "\u{2717} external_agent_harness_maintenance: FAILED - {}",
                e
            );
            failed += 1;
        }
    }

    // Dedicated front-door proof scenes may seal the window one-way, so keep
    // the Omega group after every unsealed scene in this process.
    #[cfg(feature = "visual-tests")]
    {
        println!("\n--- Omega: front door, executor disclosure, route pin ---");
        match run_omega_agent_visual_tests(app_state.clone(), &mut cx, update_baseline) {
            Ok(TestResult::Passed) => {
                println!("\u{2713} omega_agent_surfaces: PASSED");
                passed += 1;
            }
            Ok(TestResult::BaselineUpdated(_)) => {
                println!("\u{2713} omega_agent_surfaces: Baselines updated");
                updated += 1;
            }
            Err(e) => {
                eprintln!("\u{2717} omega_agent_surfaces: FAILED - {}", e);
                failed += 1;
            }
        }
    }

    // Clean up the main workspace's worktree to stop background scanning tasks
    // This prevents "root path could not be canonicalized" errors when main() drops temp_dir
    teardown_shared_window(workspace_window, &mut cx);

    // Print summary
    println!("\n=== Test Summary ===");
    println!("Passed: {}", passed);
    println!("Failed: {}", failed);
    if updated > 0 {
        println!("Baselines Updated: {}", updated);
    }

    if failed > 0 {
        eprintln!("\n=== Visual Tests FAILED ===");
        Err(anyhow::anyhow!("{} tests failed", failed))
    } else {
        println!("\n=== All Visual Tests PASSED ===");
        Ok(())
    }
}

#[cfg(target_os = "macos")]
enum TestResult {
    Passed,
    BaselineUpdated(PathBuf),
}

#[cfg(target_os = "macos")]
fn workbench_output_root() -> Option<PathBuf> {
    WORKBENCH_PROOF_SESSION.with(|session| {
        session
            .borrow()
            .as_ref()
            .map(|session| session.output_root.clone())
    })
}

#[cfg(target_os = "macos")]
fn is_workbench_selector(selector: &str) -> bool {
    selector.starts_with("workbench-")
        || selector.starts_with("omega-workbench-")
        || selector.starts_with("omega.workbench.")
}

#[cfg(target_os = "macos")]
fn is_workbench_control_selector(selector: &str) -> bool {
    selector.starts_with("workbench-control-")
        || selector.starts_with("omega-workbench-control-")
        || selector.starts_with("omega.workbench.control.")
}

#[cfg(target_os = "macos")]
fn verify_workbench_render_preflight(
    test_name: &str,
    window: gpui::AnyWindowHandle,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    cx.set_debug_accessibility_active(window, true)?;
    let snapshot = cx.debug_render_snapshot(window)?;
    let duplicate_targets: Vec<_> = snapshot
        .duplicate_selectors()
        .filter(|selector| {
            is_workbench_selector(selector) || selector.starts_with("omega.project-panel.")
        })
        .collect();
    anyhow::ensure!(
        duplicate_targets.is_empty(),
        "workbench semantic selectors must be unique; duplicates: {duplicate_targets:?}"
    );
    record_workbench_semantic_check(test_name, "workbench-selectors-unique");

    if WORKBENCH_SHELL_PIXEL_SCENES.contains(&test_name) {
        let scene = workbench_fixture_for_scene(test_name)?;
        let checks = omega_workbench_harness::prove_workbench_shell(&scene, &snapshot)
            .with_context(|| format!("proving rendered workbench shell scene {test_name:?}"))?;
        record_workbench_semantic_checks(test_name, checks);

        if test_name == "omega_workbench_shell_focus_visible" {
            let mut probe = SemanticProbe::new(&snapshot);
            probe.require_focus(WorkSurfaceId::Plan.rail_selector(), true)?;
            record_workbench_semantic_checks(test_name, probe.into_checks());
        }
        if is_workbench_files_scene(test_name) {
            let mut probe = SemanticProbe::new(&snapshot);
            let row_selectors = snapshot
                .selectors()
                .map(|(selector, _)| selector)
                .filter(|selector| selector.starts_with("omega.project-panel.row."))
                .collect::<Vec<_>>();
            probe.require_accessible(
                "omega.workbench.surface.files",
                "Group",
                "Files work surface",
            )?;
            if matches!(
                test_name,
                "omega_workbench_files_loading" | "omega_workbench_files_error"
            ) {
                probe.require_absent("omega.project-panel.tree")?;
                anyhow::ensure!(
                    row_selectors.is_empty(),
                    "non-ready Files host rendered row selectors {row_selectors:?}"
                );
                record_workbench_semantic_check(
                    test_name,
                    "files-non-ready-host-hides-native-tree",
                );
            } else {
                probe.require_accessible("omega.project-panel.tree", "Tree", "Files")?;
                probe
                    .require_inside("omega.project-panel.tree", "omega.workbench.surface.files")?;
            }

            if test_name == "omega_workbench_files_empty" {
                anyhow::ensure!(
                    row_selectors.is_empty(),
                    "empty Files scene rendered row selectors {row_selectors:?}"
                );
                probe.require_accessible(
                    "omega.project-panel.scope.empty",
                    "Status",
                    "This worktree has no visible files",
                )?;
                probe.require_inside(
                    "omega.project-panel.scope.empty",
                    "omega.project-panel.tree",
                )?;
            } else if !matches!(
                test_name,
                "omega_workbench_files_loading" | "omega_workbench_files_error"
            ) {
                anyhow::ensure!(
                    !row_selectors.is_empty(),
                    "ready Files scene rendered no semantic rows"
                );
                for selector in &row_selectors {
                    probe.require_visible(selector)?;
                    probe.require_inside(selector, "omega.project-panel.tree")?;
                }

                let tree = snapshot
                    .accessibility_tree_json()
                    .context("Files scene accessibility tree was not active")?;
                let tree: serde_json::Value =
                    serde_json::from_str(tree).context("parsing Files accessibility tree")?;
                let nodes = tree
                    .get("nodes")
                    .and_then(serde_json::Value::as_object)
                    .context("Files accessibility tree has no nodes")?;
                let mut selected_rows = 0;
                for selector in &row_selectors {
                    let matching = nodes
                        .values()
                        .filter(|node| {
                            node.get("element_id").and_then(serde_json::Value::as_str)
                                == Some(*selector)
                        })
                        .collect::<Vec<_>>();
                    anyhow::ensure!(
                        matching.len() == 1,
                        "Files row {selector:?} has {} accessibility identities",
                        matching.len()
                    );
                    let aria = matching[0]
                        .get("aria")
                        .and_then(serde_json::Value::as_object)
                        .with_context(|| format!("Files row {selector:?} has no aria object"))?;
                    anyhow::ensure!(
                        aria.get("role").and_then(serde_json::Value::as_str) == Some("TreeItem")
                            && aria
                                .get("label")
                                .and_then(serde_json::Value::as_str)
                                .is_some_and(|label| !label.trim().is_empty()),
                        "Files row {selector:?} needs a TreeItem role and non-empty label"
                    );
                    selected_rows += usize::from(
                        aria.get("selected")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false),
                    );
                }
                anyhow::ensure!(
                    selected_rows == 1,
                    "Files scene expected one selected semantic row, found {selected_rows}"
                );
                record_workbench_semantic_check(test_name, "files-rows-accessible");
                record_workbench_semantic_check(test_name, "files-one-selected-semantic-row");
            }
            record_workbench_semantic_checks(test_name, probe.into_checks());
            if !matches!(
                test_name,
                "omega_workbench_files_loading" | "omega_workbench_files_error"
            ) {
                record_workbench_semantic_check(test_name, "files-tree-row-containment");
            }
        }
        if is_workbench_search_scene(test_name) {
            let mut probe = SemanticProbe::new(&snapshot);
            probe.require_accessible(
                "omega.workbench.surface.search",
                "Group",
                "Search work surface",
            )?;
            if matches!(
                test_name,
                "omega_workbench_search_loading" | "omega_workbench_search_error"
            ) {
                if test_name == "omega_workbench_search_loading" {
                    probe.require_accessible(
                        "omega.workbench.surface.search.status",
                        "Status",
                        "Loading Search…",
                    )?;
                } else {
                    probe.require_accessible(
                        "omega.workbench.surface.search.status",
                        "Alert",
                        "Could not search this worktree",
                    )?;
                }
                probe.require_inside(
                    "omega.workbench.surface.search.status",
                    "omega.workbench.surface.search",
                )?;
                probe.require_absent("omega.workbench.search.toolbar")?;
                probe.require_absent("omega.workbench.search.content")?;
                probe.require_absent("omega.workbench.search.query-error")?;
                probe.require_absent("omega.workbench.search.lifecycle")?;
                record_workbench_semantic_check(
                    test_name,
                    "search-non-ready-host-hides-native-content",
                );
            } else {
                probe.require_accessible(
                    "omega.workbench.search.toolbar",
                    "Toolbar",
                    "Search controls",
                )?;
                probe.require_accessible(
                    "omega.workbench.search.content",
                    "Group",
                    "Search results",
                )?;
                probe.require_inside(
                    "omega.workbench.search.toolbar",
                    "omega.workbench.surface.search",
                )?;
                probe.require_inside(
                    "omega.workbench.search.content",
                    "omega.workbench.surface.search",
                )?;
                match test_name {
                    "omega_workbench_search_empty" | "omega_workbench_search_invalid_regex" => {
                        probe.require_accessible(
                            "omega.workbench.search.lifecycle",
                            "Status",
                            "Search All Files",
                        )?;
                    }
                    "omega_workbench_search_no_results" => {
                        probe.require_accessible(
                            "omega.workbench.search.lifecycle",
                            "Status",
                            "No Results",
                        )?;
                    }
                    _ => {
                        probe.require_absent("omega.workbench.search.lifecycle")?;
                    }
                }
            }
            if test_name == "omega_workbench_search_invalid_regex" {
                probe.require_visible("omega.workbench.search.query-error")?;
                probe.require_inside(
                    "omega.workbench.search.query-error",
                    "omega.workbench.search.toolbar",
                )?;
                let tree = snapshot
                    .accessibility_tree_json()
                    .context("invalid-regex Search accessibility tree was not active")?;
                let tree: serde_json::Value = serde_json::from_str(tree)
                    .context("parsing invalid-regex Search accessibility tree")?;
                let query_errors = tree
                    .get("nodes")
                    .and_then(serde_json::Value::as_object)
                    .context("invalid-regex Search accessibility tree has no nodes")?
                    .values()
                    .filter(|node| {
                        node.get("element_id").and_then(serde_json::Value::as_str)
                            == Some("omega.workbench.search.query-error")
                    })
                    .collect::<Vec<_>>();
                anyhow::ensure!(
                    query_errors.len() == 1
                        && query_errors[0]
                            .get("aria")
                            .and_then(serde_json::Value::as_object)
                            .is_some_and(|aria| {
                                aria.get("role").and_then(serde_json::Value::as_str)
                                    == Some("Alert")
                                    && aria
                                        .get("label")
                                        .and_then(serde_json::Value::as_str)
                                        .is_some_and(|label| !label.trim().is_empty())
                            }),
                    "invalid-regex Search error needs one Alert identity with a non-empty label"
                );
                record_workbench_semantic_check(test_name, "search-invalid-regex-accessible-alert");
            } else {
                probe.require_absent("omega.workbench.search.query-error")?;
            }
            if test_name == "omega_workbench_search_narrow" {
                probe.require_fully_visible("omega.workbench.search.toolbar")?;
                probe.require_fully_visible("omega.workbench.search.content")?;
                probe.require_disjoint(
                    "omega.workbench.surface.search",
                    "omega.workbench.transcript",
                )?;
                probe.require_disjoint(
                    "omega.workbench.surface.search",
                    "omega.workbench.composer",
                )?;
            }
            record_workbench_semantic_checks(test_name, probe.into_checks());
            record_workbench_semantic_check(test_name, "search-native-toolbar-content-containment");
        }
        if is_workbench_review_scene(test_name) {
            let mut probe = SemanticProbe::new(&snapshot);
            probe.require_accessible(
                "omega.workbench.surface.review",
                "Group",
                "Review work surface",
            )?;
            probe.require_accessible(
                "omega.workbench.review.toolbar",
                "Toolbar",
                "Review controls",
            )?;
            probe.require_accessible(
                "omega.workbench.review.content",
                "Group",
                "Review changes",
            )?;
            probe.require_inside(
                "omega.workbench.review.toolbar",
                "omega.workbench.surface.review",
            )?;
            probe.require_inside(
                "omega.workbench.review.content",
                "omega.workbench.surface.review",
            )?;

            match test_name {
                "omega_workbench_review_empty" => {
                    probe.require_accessible(
                        "omega.workbench.review.lifecycle",
                        "Status",
                        "No changes to review",
                    )?;
                }
                "omega_workbench_review_streaming_update" => {
                    probe.require_accessible(
                        "omega.workbench.review.lifecycle",
                        "Status",
                        "Updating review…",
                    )?;
                }
                "omega_workbench_review_all_reviewed" => {
                    probe.require_accessible(
                        "omega.workbench.review.lifecycle",
                        "Status",
                        "All changes reviewed",
                    )?;
                }
                "omega_workbench_review_error" => {
                    probe.require_accessible(
                        "omega.workbench.review.lifecycle",
                        "Alert",
                        "Could not load this checkpoint",
                    )?;
                }
                _ => {
                    probe.require_absent("omega.workbench.review.lifecycle")?;
                }
            }
            if test_name == "omega_workbench_review_narrow" {
                probe.require_fully_visible("omega.workbench.review.toolbar")?;
                probe.require_fully_visible("omega.workbench.review.content")?;
                probe.require_disjoint(
                    "omega.workbench.surface.review",
                    "omega.workbench.transcript",
                )?;
                probe.require_disjoint(
                    "omega.workbench.surface.review",
                    "omega.workbench.composer",
                )?;
            }
            record_workbench_semantic_checks(test_name, probe.into_checks());
            record_workbench_semantic_check(test_name, "review-native-toolbar-content-containment");
        }
        if is_workbench_git_scene(test_name) {
            let mut probe = SemanticProbe::new(&snapshot);
            probe.require_accessible("omega.workbench.surface.git", "Group", "Git work surface")?;
            probe.require_inside("omega.workbench.git.content", "omega.workbench.surface.git")?;

            match test_name {
                "omega_workbench_git_repository_removed" => {
                    probe.require_accessible(
                        "omega.workbench.git.lifecycle",
                        "Status",
                        "Git repository was removed",
                    )?;
                }
                "omega_workbench_git_offline" => {
                    probe.require_accessible(
                        "omega.workbench.git.lifecycle",
                        "Status",
                        "Git repository is unavailable offline",
                    )?;
                }
                "omega_workbench_git_reconnect" => {
                    probe.require_accessible(
                        "omega.workbench.git.lifecycle",
                        "Status",
                        "Reconnecting Git repository",
                    )?;
                }
                "omega_workbench_git_error" => {
                    probe.require_accessible(
                        "omega.workbench.git.lifecycle",
                        "Alert",
                        "Could not refresh repository status",
                    )?;
                }
                _ => {
                    probe.require_absent("omega.workbench.git.lifecycle")?;
                }
            }
            record_workbench_semantic_checks(test_name, probe.into_checks());
            record_workbench_semantic_check(test_name, "git-native-content-containment");
        }
        if is_workbench_terminal_scene(test_name) {
            let mut probe = SemanticProbe::new(&snapshot);
            if test_name == "omega_workbench_terminal_hidden_running" {
                probe.require_absent("omega.workbench.surface.terminal")?;
                probe.require_absent("omega.workbench.terminal.content")?;
                record_workbench_semantic_check(
                    test_name,
                    "terminal-hidden-running-retains-model-without-rendering-dock",
                );
            } else {
                probe.require_accessible(
                    "omega.workbench.surface.terminal",
                    "Group",
                    "Terminal work surface",
                )?;
                probe.require_accessible(
                    "omega.workbench.terminal.content",
                    "Group",
                    "Terminal",
                )?;
                probe.require_inside(
                    "omega.workbench.terminal.content",
                    "omega.workbench.surface.terminal",
                )?;
                probe.require_inside(
                    "omega.workbench.terminal.new",
                    "omega.workbench.terminal.content",
                )?;
                match test_name {
                    "omega_workbench_terminal_worktree_removed" => {
                        probe.require_accessible(
                            "omega.workbench.terminal.owner-state",
                            "Status",
                            "The target worktree was removed. Existing terminals keep their original owner.",
                        )?;
                    }
                    "omega_workbench_terminal_offline" => {
                        probe.require_accessible(
                            "omega.workbench.terminal.owner-state",
                            "Status",
                            "The project is offline. Existing terminal output is retained, but new terminals are unavailable.",
                        )?;
                    }
                    "omega_workbench_terminal_reconnecting" => {
                        probe.require_accessible(
                            "omega.workbench.terminal.owner-state",
                            "Status",
                            "The project is reconnecting. Existing terminal output is retained.",
                        )?;
                    }
                    "omega_workbench_terminal_error" => {
                        probe.require_accessible(
                            "omega.workbench.terminal.owner-state",
                            "Status",
                            "terminal state could not be restored",
                        )?;
                    }
                    _ => {
                        probe.require_absent("omega.workbench.terminal.owner-state")?;
                    }
                }
                if matches!(
                    test_name,
                    "omega_workbench_terminal_split" | "omega_workbench_terminal_narrow"
                ) {
                    probe.require_fully_visible("omega.workbench.terminal.content")?;
                    probe.require_disjoint(
                        "omega.workbench.surface.terminal",
                        "omega.workbench.transcript",
                    )?;
                    probe.require_disjoint(
                        "omega.workbench.surface.terminal",
                        "omega.workbench.composer",
                    )?;
                }
                record_workbench_semantic_check(test_name, "terminal-native-content-containment");
            }
            record_workbench_semantic_checks(test_name, probe.into_checks());
        }
        if is_workbench_plan_scene(test_name) {
            let mut probe = SemanticProbe::new(&snapshot);
            probe.require_visible("omega.workbench.surface.plan")?;
            probe.require_visible("omega.workbench.plan.content")?;
            probe.require_inside(
                "omega.workbench.plan.content",
                "omega.workbench.surface.plan",
            )?;
            probe.require_accessible("omega.workbench.plan.entries", "List", "Plan steps")?;
            probe.require_inside(
                "omega.workbench.plan.entries",
                "omega.workbench.plan.content",
            )?;
            let summary_label = match test_name {
                "omega_workbench_plan_empty" => "No plan for this thread",
                "omega_workbench_plan_replacement" => "1/4 complete · 2 pending · 1 in progress",
                "omega_workbench_plan_all_complete" => "All 3 steps complete",
                "omega_workbench_plan_historical" => "1 completed plans · 2 historical steps",
                _ => "1/3 complete · 1 pending · 1 in progress",
            };
            probe.require_accessible("omega.workbench.plan.summary", "Status", summary_label)?;

            let step_selectors = snapshot
                .selectors()
                .map(|(selector, _)| selector)
                .filter(|selector| selector.starts_with("omega.workbench.plan.step."))
                .collect::<Vec<_>>();
            if test_name == "omega_workbench_plan_empty" {
                anyhow::ensure!(
                    step_selectors.is_empty(),
                    "empty Plan scene rendered step selectors {step_selectors:?}"
                );
                probe.require_accessible(
                    "omega.workbench.plan.empty",
                    "Status",
                    "No plan for this thread",
                )?;
            } else {
                anyhow::ensure!(
                    !step_selectors.is_empty(),
                    "populated Plan scene rendered no semantic steps"
                );
                for selector in &step_selectors {
                    probe.require_visible(selector)?;
                    probe.require_inside(selector, "omega.workbench.plan.entries")?;
                }
                probe.require_absent("omega.workbench.plan.empty")?;

                let tree = snapshot
                    .accessibility_tree_json()
                    .context("Plan accessibility tree was not active")?;
                let tree: serde_json::Value =
                    serde_json::from_str(tree).context("parsing Plan accessibility tree")?;
                let nodes = tree
                    .get("nodes")
                    .and_then(serde_json::Value::as_object)
                    .context("Plan accessibility tree has no nodes")?;
                let accessible_steps = nodes
                    .values()
                    .filter_map(|node| node.get("aria").and_then(serde_json::Value::as_object))
                    .filter(|aria| {
                        aria.get("role").and_then(serde_json::Value::as_str) == Some("ListItem")
                            && aria
                                .get("label")
                                .and_then(serde_json::Value::as_str)
                                .is_some_and(|label| label.contains("plan step:"))
                    })
                    .collect::<Vec<_>>();
                anyhow::ensure!(
                    accessible_steps.len() == step_selectors.len(),
                    "Plan rendered {} step selectors but {} labelled ListItem accessibility nodes",
                    step_selectors.len(),
                    accessible_steps.len()
                );
                let selected_steps = accessible_steps
                    .iter()
                    .filter(|aria| {
                        aria.get("selected")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false)
                    })
                    .count();
                let expected_selected = usize::from(matches!(
                    test_name,
                    "omega_workbench_plan_historical"
                        | "omega_workbench_plan_no_source_navigation"
                        | "omega_workbench_plan_collapse_reopen"
                ));
                anyhow::ensure!(
                    selected_steps == expected_selected,
                    "Plan scene expected {expected_selected} selected steps, got {selected_steps}"
                );
                record_workbench_semantic_check(
                    test_name,
                    "plan-step-accessibility-role-label-selection",
                );
            }

            probe.require_focus("omega.workbench.plan.content", true)?;

            match test_name {
                "omega_workbench_plan_interrupted" => probe.require_accessible(
                    "omega.workbench.plan.lifecycle",
                    "Alert",
                    "Plan interrupted · Agent execution was interrupted",
                )?,
                "omega_workbench_plan_stale" => probe.require_accessible(
                    "omega.workbench.plan.lifecycle",
                    "Status",
                    "Plan may be stale while offline",
                )?,
                "omega_workbench_plan_reconnecting" => probe.require_accessible(
                    "omega.workbench.plan.lifecycle",
                    "Status",
                    "Reconnecting · retained plan may be stale",
                )?,
                "omega_workbench_plan_malformed" => probe.require_accessible(
                    "omega.workbench.plan.lifecycle",
                    "Alert",
                    "Plan update rejected · the provider returned a blank plan step",
                )?,
                _ => probe.require_absent("omega.workbench.plan.lifecycle")?,
            }
            if matches!(
                test_name,
                "omega_workbench_plan_no_source_navigation"
                    | "omega_workbench_plan_collapse_reopen"
            ) {
                probe.require_accessible(
                    "omega.workbench.plan.navigation-status",
                    "Status",
                    "This live plan step has no transcript event yet",
                )?;
            } else if test_name == "omega_workbench_plan_historical" {
                probe.require_accessible(
                    "omega.workbench.plan.navigation-status",
                    "Status",
                    "Opened transcript event 1",
                )?;
            } else {
                probe.require_absent("omega.workbench.plan.navigation-status")?;
            }
            if test_name == "omega_workbench_plan_narrow_foreign_binding" {
                probe.require_fully_visible("omega.workbench.plan.content")?;
                probe.require_disjoint(
                    "omega.workbench.surface.plan",
                    "omega.workbench.transcript",
                )?;
                probe
                    .require_disjoint("omega.workbench.surface.plan", "omega.workbench.composer")?;
            }
            record_workbench_semantic_check(test_name, "plan-native-content-containment");
            record_workbench_semantic_checks(test_name, probe.into_checks());
        }
        if test_name.starts_with("omega_workbench_identity_") {
            let mut probe = SemanticProbe::new(&snapshot);
            probe.require_fully_visible("omega.workbench.thread-identity")?;
            probe.require_inside("omega.workbench.thread-identity", "omega.workbench.toolbar")?;
            probe.require_interactive("omega.workbench.control.identity.repository")?;
            probe.require_interactive("omega.workbench.control.identity.worktree")?;
            if test_name == "omega_workbench_identity_clean" {
                probe.require_focus("omega.workbench.control.identity.repository", true)?;
                let identity_bounds = probe.require_unique("omega.workbench.thread-identity")?;
                let repository_bounds =
                    probe.require_unique("omega.workbench.control.identity.repository")?;
                let worktree_bounds =
                    probe.require_unique("omega.workbench.control.identity.worktree")?;
                let branch_bounds =
                    probe.require_unique("omega.workbench.control.identity.branch")?;
                anyhow::ensure!(
                    identity_bounds.size.width < px(360.)
                        && repository_bounds.size.width < px(180.)
                        && worktree_bounds.size.width < px(120.)
                        && branch_bounds.size.width < px(120.),
                    "clean repository identity controls reserve excess horizontal space: identity={identity_bounds:?}, repository={repository_bounds:?}, worktree={worktree_bounds:?}, branch={branch_bounds:?}"
                );
                record_workbench_semantic_check(
                    test_name,
                    "identity-controls-use-compact-content-widths",
                );
            }
            if test_name == "omega_workbench_identity_offline_error" {
                probe.require_accessible(
                    "omega.workbench.identity.status",
                    "Status",
                    "Repository identity is offline",
                )?;
                probe.require_accessible(
                    "omega.workbench.control.identity.branch",
                    "Button",
                    "Branch main",
                )?;
                probe.require_accessibility_property(
                    "omega.workbench.control.identity.branch",
                    "disabled",
                    serde_json::Value::Bool(true),
                )?;
                probe.require_accessibility_property(
                    "omega.workbench.control.identity.branch",
                    "description",
                    serde_json::Value::String(
                        "Reconnect the project before changing branches".into(),
                    ),
                )?;
            } else {
                probe.require_interactive("omega.workbench.control.identity.branch")?;
            }
            if test_name == "omega_workbench_identity_long_narrow" {
                probe.require_accessibility_property(
                    "omega.workbench.control.identity.repository",
                    "description",
                    serde_json::Value::String(
                        "Project OpenAgents, repository openagents-omega-repository-with-a-deliberately-long-name, worktree feature/server-projection-consistency-and-reconnect at /Users/example/work/openagents/omega/worktrees/feature-server-projection-consistency-and-reconnect, feature/server-projection-consistency-and-reconnect"
                            .into(),
                    ),
                )?;
            }
            record_workbench_semantic_checks(test_name, probe.into_checks());
        }
    }

    let accessibility_tree = snapshot
        .accessibility_tree_json()
        .ok_or_else(|| anyhow::anyhow!("accessibility tree was not produced"))?;
    let accessibility_tree: serde_json::Value =
        serde_json::from_str(accessibility_tree).context("parsing accessibility tree")?;
    let nodes = accessibility_tree
        .get("nodes")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("accessibility tree has no nodes"))?;
    anyhow::ensure!(
        !nodes.is_empty(),
        "accessibility tree contains no rendered nodes"
    );
    anyhow::ensure!(
        nodes.values().any(|node| {
            node.get("aria")
                .and_then(|aria| aria.get("role"))
                .and_then(serde_json::Value::as_str)
                .is_some()
        }),
        "accessibility tree contains no semantic roles"
    );
    record_workbench_semantic_check(test_name, "accessibility-tree-populated");

    for (selector, _) in snapshot
        .selectors()
        .filter(|(selector, _)| is_workbench_control_selector(selector))
    {
        let matching_nodes: Vec<_> = nodes
            .values()
            .filter(|node| {
                node.get("element_id").and_then(serde_json::Value::as_str) == Some(selector)
            })
            .collect();
        anyhow::ensure!(
            matching_nodes.len() == 1,
            "interactive workbench selector {selector:?} has {} accessibility identities",
            matching_nodes.len()
        );
        let aria = matching_nodes[0]
            .get("aria")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "interactive workbench selector {selector:?} has no accessibility properties"
                )
            })?;
        anyhow::ensure!(
            aria.get("role")
                .and_then(serde_json::Value::as_str)
                .is_some()
                && aria
                    .get("label")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|label| !label.trim().is_empty()),
            "interactive workbench selector {selector:?} needs a role and non-empty label"
        );
        let disabled = aria
            .get("disabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if disabled {
            anyhow::ensure!(
                aria.get("description")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|description| !description.trim().is_empty()),
                "disabled workbench selector {selector:?} needs a non-empty reason"
            );
        } else {
            anyhow::ensure!(
                snapshot
                    .occurrences(selector)
                    .iter()
                    .any(|occurrence| occurrence.hit_testable),
                "enabled workbench control selector {selector:?} is not hit-testable"
            );
        }
    }
    record_workbench_semantic_check(test_name, "workbench-controls-accessible");
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_visual_test(
    test_name: &str,
    window: gpui::AnyWindowHandle,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    let registered_scene = scene_spec(test_name);
    if workbench_proof_active() {
        if registered_scene.is_none() || !workbench_should_run_scene(test_name) {
            return Ok(TestResult::Passed);
        }
    }

    // Ensure all pending work is done
    cx.run_until_parked();

    // Refresh the window to ensure it's fully rendered
    cx.update_window(window, |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    if workbench_proof_active() {
        if let Err(error) = verify_workbench_render_preflight(test_name, window, cx) {
            record_workbench_semantic_failure(
                test_name,
                "render-semantic-preflight",
                format!("{error:#}"),
            );
            return Err(error);
        }
        workbench_fixture_for_scene(test_name)?.validate()?;
        record_workbench_semantic_check(test_name, "fixture-schema-preflight");
        if workbench_semantic_only() {
            return Ok(TestResult::Passed);
        }
    }

    let region_snapshot = if registered_scene.is_some_and(|scene| !scene.regions.is_empty()) {
        Some(cx.debug_render_snapshot(window)?)
    } else {
        None
    };

    // Capture the screenshot using direct texture capture
    let screenshot = cx.capture_screenshot(window)?;
    if let Some(scene) = registered_scene {
        let expected_width = scene
            .viewport
            .width
            .saturating_mul(scene.viewport.scale_milli)
            / 1000;
        let expected_height = scene
            .viewport
            .height
            .saturating_mul(scene.viewport.scale_milli)
            / 1000;
        anyhow::ensure!(
            screenshot.dimensions() == (expected_width, expected_height),
            "scene {test_name} rendered at {:?}, expected {}x{} from its viewport and scale",
            screenshot.dimensions(),
            expected_width,
            expected_height
        );
    }
    let resolved_regions = match (registered_scene, region_snapshot.as_ref()) {
        (Some(scene), Some(snapshot)) => scene
            .regions
            .iter()
            .map(|region| {
                region.resolve(
                    snapshot,
                    scene.viewport,
                    screenshot.width(),
                    screenshot.height(),
                )
            })
            .collect::<Result<Vec<_>>>()?,
        _ => Vec::new(),
    };

    let baseline_path = get_baseline_path(test_name);
    let proof_output_root = workbench_output_root();
    let (_, current_relative, diff_relative) = proof_artifact_paths(test_name);
    let output_path = if let Some(output_root) = &proof_output_root {
        output_root.join(&current_relative)
    } else {
        let output_dir = std::env::var("VISUAL_TEST_OUTPUT_DIR")
            .unwrap_or_else(|_| "target/visual_tests".to_string());
        PathBuf::from(output_dir).join(format!("{}.png", test_name))
    };

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    screenshot.save(&output_path)?;
    println!("  Screenshot saved to: {}", output_path.display());

    let policy = registered_scene
        .map(|scene| scene.pixel_policy)
        .unwrap_or(omega_workbench_harness::APPLE_SILICON_METAL_POLICY);
    let (baseline_relative, _, _) = proof_artifact_paths(test_name);

    if update_baseline {
        if let Some(parent) = baseline_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        screenshot.save(&baseline_path)?;
        if let Some(output_root) = &proof_output_root {
            let proof_baseline_path = output_root.join(&baseline_relative);
            if let Some(parent) = proof_baseline_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            screenshot.save(proof_baseline_path)?;
        }
        println!("  Baseline updated: {}", baseline_path.display());
        let mut regions = Vec::new();
        for region in &resolved_regions {
            let current_region = region.crop(&screenshot)?;
            let baseline_region_path = get_baseline_path(&format!("{test_name}__{}", region.name));
            if let Some(parent) = baseline_region_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            current_region.save(&baseline_region_path)?;
            let current_region_relative = PathBuf::from("scenes")
                .join(test_name)
                .join("regions")
                .join(format!("{}.png", region.name));
            let baseline_region_relative = PathBuf::from("scenes")
                .join(test_name)
                .join("regions")
                .join(format!("{}_baseline.png", region.name));
            if let Some(output_root) = &proof_output_root {
                let current_region_path = output_root.join(&current_region_relative);
                if let Some(parent) = current_region_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                current_region.save(&current_region_path)?;
                let proof_baseline_region_path = output_root.join(&baseline_region_relative);
                current_region.save(proof_baseline_region_path)?;
            }
            println!(
                "  Region baseline updated: {}",
                baseline_region_path.display()
            );
            regions.push(RegionPixelProof {
                name: region.name.clone(),
                status: PixelStatus::Passed,
                match_percentage: Some(1.0),
                different_pixels: Some(0),
                total_pixels: Some(
                    current_region
                        .width()
                        .saturating_mul(current_region.height()),
                ),
                baseline: baseline_region_relative,
                current: current_region_relative,
                diff: None,
            });
        }
        if workbench_proof_active() {
            record_workbench_pixel(
                test_name,
                PixelProof {
                    status: PixelStatus::Passed,
                    minimum_match: policy.minimum_match,
                    channel_tolerance: policy.channel_tolerance,
                    policy_rationale: policy.rationale.to_string(),
                    match_percentage: Some(1.0),
                    different_pixels: Some(0),
                    total_pixels: Some(screenshot.width().saturating_mul(screenshot.height())),
                    baseline: baseline_relative,
                    current: current_relative,
                    diff: None,
                    regions,
                },
            );
        }
        return Ok(TestResult::BaselineUpdated(baseline_path));
    }

    if !baseline_path.exists() {
        if workbench_proof_active() {
            record_workbench_pixel(
                test_name,
                PixelProof {
                    status: PixelStatus::Failed,
                    minimum_match: policy.minimum_match,
                    channel_tolerance: policy.channel_tolerance,
                    policy_rationale: policy.rationale.to_string(),
                    match_percentage: None,
                    different_pixels: None,
                    total_pixels: None,
                    baseline: baseline_relative,
                    current: current_relative,
                    diff: None,
                    regions: Vec::new(),
                },
            );
        }
        return Err(anyhow::anyhow!(
            "Baseline not found: {}. Run with UPDATE_BASELINE=1 to create it.",
            baseline_path.display()
        ));
    }

    if let Some(output_root) = &proof_output_root {
        let proof_baseline_path = output_root.join(&baseline_relative);
        if let Some(parent) = proof_baseline_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&baseline_path, proof_baseline_path)?;
    }

    let baseline = image::open(&baseline_path)?.to_rgba8();
    let comparison = compare_workbench_images(&screenshot, &baseline, policy.channel_tolerance);

    println!(
        "  Match: {:.2}% ({} different pixels)",
        comparison.match_percentage * 100.0,
        comparison.different_pixels
    );

    let whole_passed = comparison.match_percentage >= policy.minimum_match;
    let diff_path = proof_output_root
        .as_ref()
        .map(|output_root| output_root.join(&diff_relative))
        .unwrap_or_else(|| output_path.with_file_name(format!("{test_name}_diff.png")));
    if !whole_passed {
        if let Some(parent) = diff_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        comparison.diff_image.save(&diff_path)?;
        println!("  Diff image saved to: {}", diff_path.display());
    }

    let mut regions = Vec::new();
    let mut regions_passed = true;
    for region in &resolved_regions {
        let current_region = region.crop(&screenshot)?;
        let baseline_region_path = get_baseline_path(&format!("{test_name}__{}", region.name));
        let current_region_relative = PathBuf::from("scenes")
            .join(test_name)
            .join("regions")
            .join(format!("{}.png", region.name));
        let baseline_region_relative = PathBuf::from("scenes")
            .join(test_name)
            .join("regions")
            .join(format!("{}_baseline.png", region.name));
        let diff_region_relative = PathBuf::from("scenes")
            .join(test_name)
            .join("regions")
            .join(format!("{}_diff.png", region.name));
        let current_region_path = proof_output_root
            .as_ref()
            .map(|root| root.join(&current_region_relative))
            .unwrap_or_else(|| {
                output_path.with_file_name(format!("{test_name}__{}.png", region.name))
            });
        if let Some(parent) = current_region_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        current_region.save(&current_region_path)?;

        let mut region_proof = RegionPixelProof {
            name: region.name.clone(),
            status: PixelStatus::Failed,
            match_percentage: None,
            different_pixels: None,
            total_pixels: None,
            baseline: baseline_region_relative.clone(),
            current: current_region_relative,
            diff: None,
        };
        if baseline_region_path.exists() {
            if let Some(output_root) = &proof_output_root {
                let proof_baseline_region_path = output_root.join(&baseline_region_relative);
                if let Some(parent) = proof_baseline_region_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&baseline_region_path, proof_baseline_region_path)?;
            }
            let baseline_region = image::open(&baseline_region_path)?.to_rgba8();
            let region_comparison = compare_workbench_images(
                &current_region,
                &baseline_region,
                policy.channel_tolerance,
            );
            let passed = region_comparison.match_percentage >= policy.minimum_match;
            region_proof.status = if passed {
                PixelStatus::Passed
            } else {
                PixelStatus::Failed
            };
            region_proof.match_percentage = Some(region_comparison.match_percentage);
            region_proof.different_pixels = Some(region_comparison.different_pixels);
            region_proof.total_pixels = Some(region_comparison.total_pixels);
            if !passed {
                regions_passed = false;
                let region_diff_path = proof_output_root
                    .as_ref()
                    .map(|root| root.join(&diff_region_relative))
                    .unwrap_or_else(|| {
                        output_path.with_file_name(format!("{test_name}__{}_diff.png", region.name))
                    });
                region_comparison.diff_image.save(&region_diff_path)?;
                region_proof.diff = Some(diff_region_relative);
            }
        } else {
            regions_passed = false;
        }
        regions.push(region_proof);
    }

    if workbench_proof_active() {
        record_workbench_pixel(
            test_name,
            PixelProof {
                status: if whole_passed {
                    PixelStatus::Passed
                } else {
                    PixelStatus::Failed
                },
                minimum_match: policy.minimum_match,
                channel_tolerance: policy.channel_tolerance,
                policy_rationale: policy.rationale.to_string(),
                match_percentage: Some(comparison.match_percentage),
                different_pixels: Some(comparison.different_pixels),
                total_pixels: Some(comparison.total_pixels),
                baseline: baseline_relative,
                current: current_relative,
                diff: (!whole_passed).then_some(diff_relative),
                regions,
            },
        );
    }

    if whole_passed && regions_passed {
        Ok(TestResult::Passed)
    } else {
        Err(anyhow::anyhow!(
            "Image mismatch: {:.2}% match (threshold: {:.2}%)",
            comparison.match_percentage * 100.0,
            policy.minimum_match * 100.0
        ))
    }
}

/// Tear down the shared workspace window this runner opens for the inherited
/// tests.
///
/// Extracted so the `OMEGA_VISUAL_ONLY` early return runs it too. GPUI checks
/// for leaked entity handles when the app drops, and skipping this left the
/// sample project's buffers alive — a green suite that exited 101.
#[cfg(target_os = "macos")]
fn teardown_shared_window(
    workspace_window: WindowHandle<Workspace>,
    cx: &mut VisualTestAppContext,
) {
    workspace_window
        .update(cx, |workspace, _window, cx| {
            let project = workspace.project().clone();
            project.update(cx, |project, cx| {
                let worktree_ids: Vec<_> =
                    project.worktrees(cx).map(|wt| wt.read(cx).id()).collect();
                for id in worktree_ids {
                    project.remove_worktree(id, cx);
                }
            });
        })
        .log_err();

    cx.run_until_parked();

    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();

    cx.run_until_parked();

    // Background tasks, including scrollbar hide timers (1 second).
    for _ in 0..15 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }
}

#[cfg(target_os = "macos")]
fn get_baseline_path(test_name: &str) -> PathBuf {
    // Get the workspace root (where Cargo.toml is)
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let workspace_root = PathBuf::from(manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    workspace_root
        .join(BASELINE_DIR)
        .join(format!("{}.png", test_name))
}

#[cfg(target_os = "macos")]
fn create_test_files(project_path: &Path) {
    // Create src directory
    let src_dir = project_path.join("src");
    std::fs::create_dir_all(&src_dir).expect("Failed to create src directory");

    // Create main.rs
    let main_rs = r#"fn main() {
    println!("Hello, world!");

    let x = 42;
    let y = x * 2;

    if y > 50 {
        println!("y is greater than 50");
    } else {
        println!("y is not greater than 50");
    }

    for i in 0..10 {
        println!("i = {}", i);
    }
}

fn helper_function(a: i32, b: i32) -> i32 {
    a + b
}

struct MyStruct {
    field1: String,
    field2: i32,
}

impl MyStruct {
    fn new(name: &str, value: i32) -> Self {
        Self {
            field1: name.to_string(),
            field2: value,
        }
    }

    fn get_value(&self) -> i32 {
        self.field2
    }
}
"#;
    std::fs::write(src_dir.join("main.rs"), main_rs).expect("Failed to write main.rs");

    // Create lib.rs
    let lib_rs = r#"//! A sample library for visual testing

pub mod utils;

/// A public function in the library
pub fn library_function() -> String {
    "Hello from lib".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        assert_eq!(library_function(), "Hello from lib");
    }
}
"#;
    std::fs::write(src_dir.join("lib.rs"), lib_rs).expect("Failed to write lib.rs");

    // Create utils.rs
    let utils_rs = r#"//! Utility functions

/// Format a number with commas
pub fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

/// Calculate fibonacci number
pub fn fibonacci(n: u32) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}
"#;
    std::fs::write(src_dir.join("utils.rs"), utils_rs).expect("Failed to write utils.rs");

    // Create Cargo.toml
    let cargo_toml = r#"[package]
name = "test_project"
version = "0.1.0"
edition = "2021"

[dependencies]
"#;
    std::fs::write(project_path.join("Cargo.toml"), cargo_toml)
        .expect("Failed to write Cargo.toml");

    // Create README.md
    let readme = r#"# Test Project

This is a test project for visual testing of Omega.

## Features

- Feature 1
- Feature 2
- Feature 3

## Usage

```bash
cargo run
```
"#;
    std::fs::write(project_path.join("README.md"), readme).expect("Failed to write README.md");
}

#[cfg(target_os = "macos")]
fn init_app_state(cx: &mut App) -> Arc<AppState> {
    use fs::Fs;
    use node_runtime::NodeRuntime;
    use session::Session;
    use settings::SettingsStore;

    if !cx.has_global::<SettingsStore>() {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
    }

    // Use the real filesystem instead of FakeFs so we can access actual files on disk
    let fs: Arc<dyn Fs> = Arc::new(fs::RealFs::new(None, cx.background_executor().clone()));
    <dyn Fs>::set_global(fs.clone(), cx);

    let languages = Arc::new(language::LanguageRegistry::test(
        cx.background_executor().clone(),
    ));
    let clock = Arc::new(clock::FakeSystemClock::new());
    let http_client = http_client::FakeHttpClient::with_404_response();
    let client = client::Client::new(clock, http_client, cx);
    let session = cx.new(|cx| session::AppSession::new(Session::test(), cx));
    let user_store = cx.new(|cx| client::UserStore::new(client.clone(), cx));
    let workspace_store = cx.new(|cx| workspace::WorkspaceStore::new(client.clone(), cx));

    theme_settings::init(theme::LoadThemes::JustBase, cx);
    client::init(&client, cx);

    let app_state = Arc::new(AppState {
        client,
        fs,
        languages,
        user_store,
        workspace_store,
        node_runtime: NodeRuntime::unavailable(),
        build_window_options: |_, _| Default::default(),
        session,
    });
    AppState::set_global(app_state.clone(), cx);
    app_state
}

/// Runs visual tests for breakpoint hover states in the editor gutter.
///
/// This test captures three states:
/// 1. Gutter with line numbers, no breakpoint hover (baseline)
/// 2. Gutter with breakpoint hover indicator (gray circle)
/// 3. Gutter with breakpoint hover AND tooltip
#[cfg(target_os = "macos")]
fn run_breakpoint_hover_visual_tests(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    // Create a temporary directory with a simple test file
    let temp_dir = tempfile::tempdir()?;
    let temp_path = temp_dir.keep();
    let canonical_temp = temp_path.canonicalize()?;
    let project_path = canonical_temp.join("project");
    std::fs::create_dir_all(&project_path)?;

    // Create a simple file with a few lines
    let src_dir = project_path.join("src");
    std::fs::create_dir_all(&src_dir)?;

    let test_content = r#"fn main() {
    println!("Hello");
    let x = 42;
}
"#;
    std::fs::write(src_dir.join("test.rs"), test_content)?;

    // Create a small window - just big enough to show gutter and a few lines
    let window_size = size(px(300.0), px(200.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };

    // Create project
    let project = cx.update(|cx| {
        project::Project::local(
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            app_state.user_store.clone(),
            app_state.languages.clone(),
            app_state.fs.clone(),
            None,
            project::LocalProjectFlags {
                init_worktree_trust: false,
                ..Default::default()
            },
            cx,
        )
    });

    // Open workspace window
    let workspace_window: WindowHandle<Workspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        Workspace::new(None, project.clone(), app_state.clone(), window, cx)
                    })
                },
            )
        })
        .context("Failed to open breakpoint test window")?;

    cx.run_until_parked();

    // Add the project as a worktree
    let add_worktree_task = workspace_window
        .update(cx, |workspace, _window, cx| {
            let project = workspace.project().clone();
            project.update(cx, |project, cx| {
                project.find_or_create_worktree(&project_path, true, cx)
            })
        })
        .context("Failed to start adding worktree")?;

    cx.background_executor.allow_parking();
    let worktree_result = cx.foreground_executor.block_test(add_worktree_task);
    cx.background_executor.forbid_parking();
    worktree_result.context("Failed to add worktree")?;

    cx.run_until_parked();

    // Open the test file
    let open_file_task = workspace_window
        .update(cx, |workspace, window, cx| {
            let worktree = workspace.project().read(cx).worktrees(cx).next();
            if let Some(worktree) = worktree {
                let worktree_id = worktree.read(cx).id();
                let rel_path: std::sync::Arc<util::rel_path::RelPath> =
                    util::rel_path::rel_path("src/test.rs").into();
                let project_path: project::ProjectPath = (worktree_id, rel_path).into();
                Some(workspace.open_path(project_path, None, true, window, cx))
            } else {
                None
            }
        })
        .log_err()
        .flatten();

    if let Some(task) = open_file_task {
        cx.background_executor.allow_parking();
        cx.foreground_executor.block_test(task).log_err();
        cx.background_executor.forbid_parking();
    }

    cx.run_until_parked();

    // Wait for the editor to fully load
    for _ in 0..10 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Refresh window
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    // Test 1: Gutter visible with line numbers, no breakpoint hover
    let test1_result = run_visual_test(
        "breakpoint_hover_none",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // Test 2: Breakpoint hover indicator (circle) visible
    // The gutter is on the left side. We need to position the mouse over the gutter area
    // for line 1. The breakpoint indicator appears in the leftmost part of the gutter.
    //
    // The breakpoint hover requires multiple steps:
    // 1. Draw to register mouse listeners
    // 2. Mouse move to trigger gutter_hovered and create GutterHoverButton
    // 3. Wait 200ms for is_active to become true
    // 4. Draw again to render the indicator
    //
    // The gutter_position should be in the gutter area to trigger the gutter hover button.
    // The button_position should be directly over the breakpoint icon button for tooltip hover.
    // Based on debug output: button is at origin=(3.12, 66.5) with size=(14, 16)
    let gutter_position = point(px(30.0), px(85.0));
    let button_position = point(px(10.0), px(75.0)); // Center of the breakpoint button

    // Step 1: Initial draw to register mouse listeners
    cx.update_window(workspace_window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })?;
    cx.run_until_parked();

    // Step 2: Simulate mouse move into gutter area
    cx.simulate_mouse_move(
        workspace_window.into(),
        gutter_position,
        None,
        Modifiers::default(),
    );

    // Step 3: Advance clock past 200ms debounce
    cx.advance_clock(Duration::from_millis(300));
    cx.run_until_parked();

    // Step 4: Draw again to pick up the indicator state change
    cx.update_window(workspace_window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })?;
    cx.run_until_parked();

    // Step 5: Another mouse move to keep hover state active
    cx.simulate_mouse_move(
        workspace_window.into(),
        gutter_position,
        None,
        Modifiers::default(),
    );

    // Step 6: Final draw
    cx.update_window(workspace_window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })?;
    cx.run_until_parked();

    let test2_result = run_visual_test(
        "breakpoint_hover_circle",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // Test 3: Breakpoint hover with tooltip visible
    // The tooltip delay is 500ms (TOOLTIP_SHOW_DELAY constant)
    // We need to position the mouse directly over the breakpoint button for the tooltip to show.
    // The button hitbox is approximately at (3.12, 66.5) with size (14, 16).

    // Move mouse directly over the button to trigger tooltip hover
    cx.simulate_mouse_move(
        workspace_window.into(),
        button_position,
        None,
        Modifiers::default(),
    );

    // Draw to register the button's tooltip hover listener
    cx.update_window(workspace_window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })?;
    cx.run_until_parked();

    // Move mouse over button again to trigger tooltip scheduling
    cx.simulate_mouse_move(
        workspace_window.into(),
        button_position,
        None,
        Modifiers::default(),
    );

    // Advance clock past TOOLTIP_SHOW_DELAY (500ms)
    cx.advance_clock(TOOLTIP_SHOW_DELAY + Duration::from_millis(100));
    cx.run_until_parked();

    // Draw to render the tooltip
    cx.update_window(workspace_window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })?;
    cx.run_until_parked();

    // Refresh window
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    let test3_result = run_visual_test(
        "breakpoint_hover_tooltip",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // Clean up: remove worktrees to stop background scanning
    workspace_window
        .update(cx, |workspace, _window, cx| {
            let project = workspace.project().clone();
            project.update(cx, |project, cx| {
                let worktree_ids: Vec<_> =
                    project.worktrees(cx).map(|wt| wt.read(cx).id()).collect();
                for id in worktree_ids {
                    project.remove_worktree(id, cx);
                }
            });
        })
        .log_err();

    cx.run_until_parked();

    // Close the window
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();

    cx.run_until_parked();

    // Give background tasks time to finish
    for _ in 0..15 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Return combined result
    match (&test1_result, &test2_result, &test3_result) {
        (TestResult::Passed, TestResult::Passed, TestResult::Passed) => Ok(TestResult::Passed),
        (TestResult::BaselineUpdated(p), _, _)
        | (_, TestResult::BaselineUpdated(p), _)
        | (_, _, TestResult::BaselineUpdated(p)) => Ok(TestResult::BaselineUpdated(p.clone())),
    }
}

/// Runs visual tests for the settings UI sub-page auto-open feature.
///
/// This test verifies that when opening settings via OpenSettingsAt with a path
/// that maps to a single SubPageLink, the sub-page is automatically opened.
///
/// This test captures two states:
/// 1. Settings opened with a path that maps to multiple items (no auto-open)
/// 2. Settings opened with a path that maps to a single SubPageLink (auto-opens sub-page)
#[cfg(target_os = "macos")]
fn run_settings_ui_subpage_visual_tests(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    // Create a workspace window for dispatching actions
    let window_size = size(px(1280.0), px(800.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };

    let project = cx.update(|cx| {
        project::Project::local(
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            app_state.user_store.clone(),
            app_state.languages.clone(),
            app_state.fs.clone(),
            None,
            project::LocalProjectFlags {
                init_worktree_trust: false,
                ..Default::default()
            },
            cx,
        )
    });

    let workspace_window: WindowHandle<MultiWorkspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| {
                    let workspace = cx.new(|cx| {
                        Workspace::new(None, project.clone(), app_state.clone(), window, cx)
                    });
                    cx.new(|cx| MultiWorkspace::new(workspace, window, cx))
                },
            )
        })
        .context("Failed to open workspace window")?;

    cx.run_until_parked();

    // Test 1: Open settings with a path that maps to multiple items (e.g., "agent")
    // This should NOT auto-open a sub-page since multiple items match
    workspace_window
        .update(cx, |_workspace, window, cx| {
            window.dispatch_action(
                Box::new(OpenSettingsAt {
                    path: "agent".to_string(),
                    target: None,
                }),
                cx,
            );
        })
        .context("Failed to dispatch OpenSettingsAt for multiple items")?;

    cx.run_until_parked();

    // Find the settings window
    let settings_window_1 = cx
        .update(|cx| {
            cx.windows()
                .into_iter()
                .find_map(|window| window.downcast::<SettingsWindow>())
        })
        .context("Settings window not found")?;

    // Refresh and capture screenshot
    cx.update_window(settings_window_1.into(), |_, window, _cx| {
        window.refresh();
    })?;
    cx.run_until_parked();

    let test1_result = run_visual_test(
        "settings_ui_no_auto_open",
        settings_window_1.into(),
        cx,
        update_baseline,
    )?;

    // Close the settings window
    cx.update_window(settings_window_1.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();
    cx.run_until_parked();

    // Test 2: Open settings with a path that maps to a single SubPageLink
    // "edit_predictions.providers" maps to the "Configure Providers" SubPageLink
    // This should auto-open the sub-page
    workspace_window
        .update(cx, |_workspace, window, cx| {
            window.dispatch_action(
                Box::new(OpenSettingsAt {
                    path: "edit_predictions.providers".to_string(),
                    target: None,
                }),
                cx,
            );
        })
        .context("Failed to dispatch OpenSettingsAt for single SubPageLink")?;

    cx.run_until_parked();

    // Find the new settings window
    let settings_window_2 = cx
        .update(|cx| {
            cx.windows()
                .into_iter()
                .find_map(|window| window.downcast::<SettingsWindow>())
        })
        .context("Settings window not found for sub-page test")?;

    // Refresh and capture screenshot
    cx.update_window(settings_window_2.into(), |_, window, _cx| {
        window.refresh();
    })?;
    cx.run_until_parked();

    let test2_result = run_visual_test(
        "settings_ui_subpage_auto_open",
        settings_window_2.into(),
        cx,
        update_baseline,
    )?;

    // Clean up: close the settings window
    cx.update_window(settings_window_2.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();
    cx.run_until_parked();

    // Clean up: close the workspace window
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();
    cx.run_until_parked();

    // Give background tasks time to finish
    for _ in 0..5 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Return combined result
    match (&test1_result, &test2_result) {
        (TestResult::Passed, TestResult::Passed) => Ok(TestResult::Passed),
        (TestResult::BaselineUpdated(p), _) | (_, TestResult::BaselineUpdated(p)) => {
            Ok(TestResult::BaselineUpdated(p.clone()))
        }
    }
}

/// Runs visual tests for the diff review button in git diff views.
///
/// This test captures three states:
/// 1. Diff view with feature flag enabled (button visible)
/// 2. Diff view with feature flag disabled (no button)
/// 3. Regular editor with feature flag enabled (no button - only shows in diff views)
#[cfg(target_os = "macos")]
fn run_diff_review_visual_tests(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    // Create a temporary directory with test files and a real git repo
    let temp_dir = tempfile::tempdir()?;
    let temp_path = temp_dir.keep();
    let canonical_temp = temp_path.canonicalize()?;
    let project_path = canonical_temp.join("project");
    std::fs::create_dir_all(&project_path)?;

    // Initialize a real git repository
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&project_path)
        .output()?;

    // Configure git user for commits
    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&project_path)
        .output()?;
    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&project_path)
        .output()?;

    // Create a test file with original content
    let original_content = "// Original content\n";
    std::fs::write(project_path.join("thread-view.tsx"), original_content)?;

    // Commit the original file
    std::process::Command::new("git")
        .args(["add", "thread-view.tsx"])
        .current_dir(&project_path)
        .output()?;
    std::process::Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&project_path)
        .output()?;

    // Modify the file to create a diff
    let modified_content = r#"import { ScrollArea } from 'components';
import { ButtonAlt, Tooltip } from 'ui';
import { Message, FileEdit } from 'types';
import { AiPaneTabContext } from 'context';
"#;
    std::fs::write(project_path.join("thread-view.tsx"), modified_content)?;

    // Create window for the diff view - sized to show just the editor
    let window_size = size(px(600.0), px(400.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };

    // Create project
    let project = cx.update(|cx| {
        project::Project::local(
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            app_state.user_store.clone(),
            app_state.languages.clone(),
            app_state.fs.clone(),
            None,
            project::LocalProjectFlags {
                init_worktree_trust: false,
                ..Default::default()
            },
            cx,
        )
    });

    // Add the test directory as a worktree
    let add_worktree_task = project.update(cx, |project, cx| {
        project.find_or_create_worktree(&project_path, true, cx)
    });

    cx.background_executor.allow_parking();
    cx.foreground_executor
        .block_test(add_worktree_task)
        .log_err();
    cx.background_executor.forbid_parking();

    cx.run_until_parked();

    // Wait for worktree to be fully scanned and git status to be detected
    for _ in 0..5 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Test 1: Diff view with feature flag enabled
    // Enable the feature flag
    cx.update(|cx| {
        cx.update_flags(true, vec!["diff-review".to_string()]);
    });

    let workspace_window: WindowHandle<Workspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        Workspace::new(None, project.clone(), app_state.clone(), window, cx)
                    })
                },
            )
        })
        .context("Failed to open diff review test window")?;

    cx.run_until_parked();

    // Create and add the ProjectDiff using the public deploy_at method
    workspace_window
        .update(cx, |workspace, window, cx| {
            ProjectDiff::deploy_at(workspace, None, window, cx);
        })
        .log_err();

    // Wait for diff to render
    for _ in 0..5 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Refresh window
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    // Capture Test 1: Diff with flag enabled
    let test1_result = run_visual_test(
        "diff_review_button_enabled",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // Test 2: Diff view with feature flag disabled
    // Disable the feature flag
    cx.update(|cx| {
        cx.update_flags(false, vec![]);
    });

    // Refresh window
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    for _ in 0..3 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Capture Test 2: Diff with flag disabled
    let test2_result = run_visual_test(
        "diff_review_button_disabled",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // Test 3: Regular editor with flag enabled (should NOT show button)
    // Re-enable the feature flag
    cx.update(|cx| {
        cx.update_flags(true, vec!["diff-review".to_string()]);
    });

    // Create a new window with just a regular editor
    let regular_window: WindowHandle<Workspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        Workspace::new(None, project.clone(), app_state.clone(), window, cx)
                    })
                },
            )
        })
        .context("Failed to open regular editor window")?;

    cx.run_until_parked();

    // Open a regular file (not a diff view)
    let open_file_task = regular_window
        .update(cx, |workspace, window, cx| {
            let worktree = workspace.project().read(cx).worktrees(cx).next();
            if let Some(worktree) = worktree {
                let worktree_id = worktree.read(cx).id();
                let rel_path: std::sync::Arc<util::rel_path::RelPath> =
                    util::rel_path::rel_path("thread-view.tsx").into();
                let project_path: project::ProjectPath = (worktree_id, rel_path).into();
                Some(workspace.open_path(project_path, None, true, window, cx))
            } else {
                None
            }
        })
        .log_err()
        .flatten();

    if let Some(task) = open_file_task {
        cx.background_executor.allow_parking();
        cx.foreground_executor.block_test(task).log_err();
        cx.background_executor.forbid_parking();
    }

    // Wait for file to open
    for _ in 0..3 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Refresh window
    cx.update_window(regular_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    // Capture Test 3: Regular editor with flag enabled (no button)
    let test3_result = run_visual_test(
        "diff_review_button_regular_editor",
        regular_window.into(),
        cx,
        update_baseline,
    )?;

    // Test 4: Show the diff review overlay on the regular editor
    regular_window
        .update(cx, |workspace, window, cx| {
            // Get the first editor from the workspace
            let editors: Vec<_> = workspace.items_of_type::<editor::Editor>(cx).collect();
            if let Some(editor) = editors.into_iter().next() {
                editor.update(cx, |editor, cx| {
                    editor.show_diff_review_overlay(DisplayRow(1)..DisplayRow(1), window, cx);
                });
            }
        })
        .log_err();

    // Wait for overlay to render
    for _ in 0..3 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Refresh window
    cx.update_window(regular_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    // Capture Test 4: Regular editor with overlay shown
    let test4_result = run_visual_test(
        "diff_review_overlay_shown",
        regular_window.into(),
        cx,
        update_baseline,
    )?;

    // Test 5: Type text into the diff review prompt and submit it
    // First, get the prompt editor from the overlay and type some text
    regular_window
        .update(cx, |workspace, window, cx| {
            let editors: Vec<_> = workspace.items_of_type::<editor::Editor>(cx).collect();
            if let Some(editor) = editors.into_iter().next() {
                editor.update(cx, |editor, cx| {
                    // Get the prompt editor from the overlay and insert text
                    if let Some(prompt_editor) = editor.diff_review_prompt_editor().cloned() {
                        prompt_editor.update(cx, |prompt_editor: &mut editor::Editor, cx| {
                            prompt_editor.insert(
                                "This change needs better error handling",
                                window,
                                cx,
                            );
                        });
                    }
                });
            }
        })
        .log_err();

    // Wait for text to be inserted
    for _ in 0..3 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Refresh window
    cx.update_window(regular_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    // Capture Test 5: Diff review overlay with typed text
    let test5_result = run_visual_test(
        "diff_review_overlay_with_text",
        regular_window.into(),
        cx,
        update_baseline,
    )?;

    // Test 6: Submit a comment to store it locally
    regular_window
        .update(cx, |workspace, window, cx| {
            let editors: Vec<_> = workspace.items_of_type::<editor::Editor>(cx).collect();
            if let Some(editor) = editors.into_iter().next() {
                editor.update(cx, |editor, cx| {
                    // Submit the comment that was typed in test 5
                    editor.submit_diff_review_comment(window, cx);
                });
            }
        })
        .log_err();

    // Wait for comment to be stored
    for _ in 0..3 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Refresh window
    cx.update_window(regular_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    // Capture Test 6: Overlay with one stored comment
    let test6_result = run_visual_test(
        "diff_review_one_comment",
        regular_window.into(),
        cx,
        update_baseline,
    )?;

    // Test 7: Add more comments to show multiple comments expanded
    regular_window
        .update(cx, |workspace, window, cx| {
            let editors: Vec<_> = workspace.items_of_type::<editor::Editor>(cx).collect();
            if let Some(editor) = editors.into_iter().next() {
                editor.update(cx, |editor, cx| {
                    // Add second comment
                    if let Some(prompt_editor) = editor.diff_review_prompt_editor().cloned() {
                        prompt_editor.update(cx, |pe, cx| {
                            pe.insert("Second comment about imports", window, cx);
                        });
                    }
                    editor.submit_diff_review_comment(window, cx);

                    // Add third comment
                    if let Some(prompt_editor) = editor.diff_review_prompt_editor().cloned() {
                        prompt_editor.update(cx, |pe, cx| {
                            pe.insert("Third comment about naming conventions", window, cx);
                        });
                    }
                    editor.submit_diff_review_comment(window, cx);
                });
            }
        })
        .log_err();

    // Wait for comments to be stored
    for _ in 0..3 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Refresh window
    cx.update_window(regular_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    // Capture Test 7: Overlay with multiple comments expanded
    let test7_result = run_visual_test(
        "diff_review_multiple_comments_expanded",
        regular_window.into(),
        cx,
        update_baseline,
    )?;

    // Test 8: Collapse the comments section
    regular_window
        .update(cx, |workspace, _window, cx| {
            let editors: Vec<_> = workspace.items_of_type::<editor::Editor>(cx).collect();
            if let Some(editor) = editors.into_iter().next() {
                editor.update(cx, |editor, cx| {
                    // Toggle collapse using the public method
                    editor.set_diff_review_comments_expanded(false, cx);
                });
            }
        })
        .log_err();

    // Wait for UI to update
    for _ in 0..3 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Refresh window
    cx.update_window(regular_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    // Capture Test 8: Comments collapsed
    let test8_result = run_visual_test(
        "diff_review_comments_collapsed",
        regular_window.into(),
        cx,
        update_baseline,
    )?;

    // Clean up: remove worktrees to stop background scanning
    workspace_window
        .update(cx, |workspace, _window, cx| {
            let project = workspace.project().clone();
            project.update(cx, |project, cx| {
                let worktree_ids: Vec<_> =
                    project.worktrees(cx).map(|wt| wt.read(cx).id()).collect();
                for id in worktree_ids {
                    project.remove_worktree(id, cx);
                }
            });
        })
        .log_err();

    cx.run_until_parked();

    // Close windows
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();
    cx.update_window(regular_window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();

    cx.run_until_parked();

    // Give background tasks time to finish
    for _ in 0..15 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Return combined result
    let all_results = [
        &test1_result,
        &test2_result,
        &test3_result,
        &test4_result,
        &test5_result,
        &test6_result,
        &test7_result,
        &test8_result,
    ];

    // Combine results: if any test updated a baseline, return BaselineUpdated;
    // otherwise return Passed. The exhaustive match ensures the compiler
    // verifies we handle all TestResult variants.
    let result = all_results
        .iter()
        .fold(TestResult::Passed, |acc, r| match r {
            TestResult::Passed => acc,
            TestResult::BaselineUpdated(p) => TestResult::BaselineUpdated(p.clone()),
        });
    Ok(result)
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
const OMEGA_AGENT_PROOF_SCENES: &[&str] = &[
    "omega_front_door_no_project",
    "omega_front_door_typing",
    "omega_sarah_admission_ready",
    "omega_sarah_session_settled",
    "omega_tester_channel_first_launch",
    "omega_tester_channel_relay_unavailable",
    "omega_executor_disclosure_native",
    "omega_route_pin_honoured",
    "omega_route_pin_not_honoured",
    "omega_executor_disclosure_external_acp",
    "omega_executor_disclosure_engine_lane",
    "omega_executor_disclosure_external_acp_after_restart",
    "omega_executor_disclosure_engine_lane_after_restart",
];

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
const OMEGA_CONCURRENT_AGENT_PROOF_SCENES: &[&str] = &[
    "omega_concurrent_agents_codex_waiting",
    "omega_concurrent_agents_claude_running",
    "omega_concurrent_agents_cancel_isolated",
    "omega_concurrent_agents_worktree_no_dialog",
];

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_recording_visual_tests(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    let mut results = Vec::new();
    if workbench_any_selected(OMEGA_AGENT_PROOF_SCENES) {
        results.push(run_omega_agent_visual_tests(
            app_state.clone(),
            cx,
            update_baseline,
        )?);
    }
    if workbench_any_selected(OMEGA_CONCURRENT_AGENT_PROOF_SCENES) {
        results.push(run_omega_concurrent_agent_visual_tests(
            app_state.clone(),
            cx,
            update_baseline,
        )?);
    }
    if workbench_any_selected(&WORKBENCH_SHELL_PIXEL_SCENES) {
        results.push(run_omega_workbench_shell_visual_tests(
            app_state,
            cx,
            update_baseline,
        )?);
    }

    Ok(results
        .into_iter()
        .find(|result| matches!(result, TestResult::BaselineUpdated(_)))
        .unwrap_or(TestResult::Passed))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_workbench_shell_visual_tests(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    let mut results = Vec::new();
    for scene_name in WORKBENCH_SHELL_PIXEL_SCENES {
        if !workbench_any_selected(&[scene_name]) {
            continue;
        }
        results.push(run_omega_workbench_shell_visual_capture(
            app_state.clone(),
            cx,
            scene_name,
            update_baseline,
        )?);
    }

    Ok(results
        .into_iter()
        .find(|result| matches!(result, TestResult::BaselineUpdated(_)))
        .unwrap_or(TestResult::Passed))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
struct WorkbenchFilesDiskFixture {
    _root: tempfile::TempDir,
    worktrees: Vec<(String, PathBuf)>,
    active_worktree_id: String,
    selected_path: Option<&'static str>,
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
struct WorkbenchSearchDiskFixture {
    _root: tempfile::TempDir,
    worktrees: Vec<(String, PathBuf)>,
    active_worktree_id: String,
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
struct WorkbenchReviewDiskFixture {
    _root: tempfile::TempDir,
    worktrees: Vec<(String, PathBuf)>,
    active_worktree_id: String,
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
struct WorkbenchGitDiskFixture {
    _root: tempfile::TempDir,
    worktrees: Vec<(String, PathBuf)>,
    active_worktree_id: String,
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
struct WorkbenchTerminalDiskFixture {
    _root: tempfile::TempDir,
    worktrees: Vec<(String, PathBuf)>,
    active_worktree_id: String,
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn initialize_workbench_git_fixture(path: &Path) -> Result<()> {
    for arguments in [
        &["init", "--initial-branch=main"][..],
        &["config", "user.email", "omega-workbench@example.invalid"][..],
        &["config", "user.name", "Omega Workbench Proof"][..],
        &["add", "."][..],
        &["commit", "-m", "Create Review proof fixture"][..],
    ] {
        run_workbench_git(path, arguments, "Review")?;
    }
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn execute_workbench_git(
    path: &Path,
    arguments: &[&str],
    fixture_label: &str,
) -> Result<std::process::Output> {
    std::process::Command::new("git")
        .args(arguments)
        .current_dir(path)
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "starting `git {}` for {fixture_label} fixture {}",
                arguments.join(" "),
                path.display()
            )
        })?
        .wait_with_output()
        .with_context(|| {
            format!(
                "waiting for `git {}` for {fixture_label} fixture {}",
                arguments.join(" "),
                path.display()
            )
        })
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_workbench_git(
    path: &Path,
    arguments: &[&str],
    fixture_label: &str,
) -> Result<std::process::Output> {
    let output = execute_workbench_git(path, arguments, fixture_label)?;
    anyhow::ensure!(
        output.status.success(),
        "`git {}` failed for {fixture_label} fixture {}: {}",
        arguments.join(" "),
        path.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output)
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn initialize_workbench_git_repository(
    path: &Path,
    initial_branch: &str,
    commit: bool,
) -> Result<()> {
    run_workbench_git(
        path,
        &["init", &format!("--initial-branch={initial_branch}")],
        "Git",
    )?;
    run_workbench_git(
        path,
        &["config", "user.email", "omega-workbench@example.invalid"],
        "Git",
    )?;
    run_workbench_git(
        path,
        &["config", "user.name", "Omega Workbench Proof"],
        "Git",
    )?;
    if commit {
        run_workbench_git(path, &["add", "."], "Git")?;
        run_workbench_git(path, &["commit", "-m", "Create Git proof fixture"], "Git")?;
    }
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn create_workbench_git_disk_fixture(scene_name: &str) -> Result<Option<WorkbenchGitDiskFixture>> {
    if !is_workbench_git_scene(scene_name) {
        return Ok(None);
    }

    let root = tempfile::tempdir().context("creating Git scene directory")?;
    let root_path = root
        .path()
        .canonicalize()
        .context("canonicalizing Git scene directory")?;
    let alpha = root_path.join("alpha-worktree");
    let beta = root_path.join("beta-worktree");
    std::fs::create_dir_all(alpha.join("foreign"))?;
    std::fs::create_dir_all(beta.join("src"))?;
    std::fs::write(
        alpha.join("foreign/alpha_only.rs"),
        "pub const FOREIGN_REPOSITORY: bool = false;\n",
    )?;
    std::fs::write(beta.join("README.md"), "# Git work surface\n")?;
    for file in [
        "main.rs",
        "conflicted.rs",
        "detached.rs",
        "offline.rs",
        "reconnected.rs",
        "beta.rs",
    ] {
        std::fs::write(
            beta.join("src").join(file),
            format!("pub const FIXTURE: &str = \"{file}\";\n"),
        )?;
    }

    initialize_workbench_git_repository(&alpha, "alpha-work", true)?;
    std::fs::write(
        alpha.join("foreign/alpha_only.rs"),
        "pub const FOREIGN_REPOSITORY: bool = true;\n",
    )?;

    let unborn = scene_name == "omega_workbench_git_unborn";
    if unborn {
        std::fs::remove_dir_all(beta.join("src"))
            .context("removing committed-only files from unborn Git fixture")?;
    }
    initialize_workbench_git_repository(
        &beta,
        if unborn {
            "omega/initial"
        } else {
            "codex/git-surface"
        },
        !unborn,
    )?;

    match scene_name {
        "omega_workbench_git_clean"
        | "omega_workbench_git_repository_removed"
        | "omega_workbench_git_error" => {}
        "omega_workbench_git_dirty" => {
            let remote = root_path.join("dirty-remote.git");
            let remote_text = remote.to_string_lossy().to_string();
            run_workbench_git(&root_path, &["init", "--bare", remote_text.as_str()], "Git")?;
            run_workbench_git(
                &beta,
                &["remote", "add", "origin", remote_text.as_str()],
                "Git",
            )?;
            run_workbench_git(&beta, &["push", "-u", "origin", "codex/git-surface"], "Git")?;

            let peer = root_path.join("dirty-peer");
            let peer_text = peer.to_string_lossy().to_string();
            run_workbench_git(
                &root_path,
                &[
                    "clone",
                    "--branch",
                    "codex/git-surface",
                    remote_text.as_str(),
                    peer_text.as_str(),
                ],
                "Git",
            )?;
            run_workbench_git(
                &peer,
                &["config", "user.email", "omega-workbench@example.invalid"],
                "Git",
            )?;
            run_workbench_git(
                &peer,
                &["config", "user.name", "Omega Workbench Proof"],
                "Git",
            )?;
            std::fs::write(peer.join("remote-behind.txt"), "remote divergence\n")?;
            run_workbench_git(&peer, &["add", "remote-behind.txt"], "Git")?;
            run_workbench_git(&peer, &["commit", "-m", "Remote divergence"], "Git")?;
            run_workbench_git(&peer, &["push"], "Git")?;

            std::fs::write(beta.join("local-ahead-one.txt"), "first local commit\n")?;
            run_workbench_git(&beta, &["add", "local-ahead-one.txt"], "Git")?;
            run_workbench_git(&beta, &["commit", "-m", "First local commit"], "Git")?;
            std::fs::write(beta.join("local-ahead-two.txt"), "second local commit\n")?;
            run_workbench_git(&beta, &["add", "local-ahead-two.txt"], "Git")?;
            run_workbench_git(&beta, &["commit", "-m", "Second local commit"], "Git")?;
            run_workbench_git(&beta, &["fetch", "origin"], "Git")?;

            std::fs::write(
                beta.join("README.md"),
                "# Git work surface\n\nDirty fixture.\n",
            )?;
            std::fs::write(
                beta.join("src/new.rs"),
                "pub const NEW_FILE: bool = true;\n",
            )?;
        }
        "omega_workbench_git_staged" | "omega_workbench_git_pending" => {
            std::fs::write(
                beta.join("src/main.rs"),
                "pub const FIXTURE: &str = \"staged\";\n",
            )?;
            run_workbench_git(&beta, &["add", "src/main.rs"], "Git")?;
        }
        "omega_workbench_git_conflict" => {
            run_workbench_git(&beta, &["checkout", "-b", "conflict-side"], "Git")?;
            std::fs::write(
                beta.join("src/conflicted.rs"),
                "pub const FIXTURE: &str = \"side\";\n",
            )?;
            run_workbench_git(&beta, &["add", "src/conflicted.rs"], "Git")?;
            run_workbench_git(&beta, &["commit", "-m", "Create conflict side"], "Git")?;
            run_workbench_git(&beta, &["checkout", "codex/git-surface"], "Git")?;
            std::fs::write(
                beta.join("src/conflicted.rs"),
                "pub const FIXTURE: &str = \"active\";\n",
            )?;
            run_workbench_git(&beta, &["add", "src/conflicted.rs"], "Git")?;
            run_workbench_git(&beta, &["commit", "-m", "Create active conflict"], "Git")?;
            let output = execute_workbench_git(&beta, &["merge", "conflict-side"], "Git conflict")?;
            anyhow::ensure!(
                !output.status.success(),
                "Git conflict fixture unexpectedly merged without conflict"
            );
        }
        "omega_workbench_git_detached" => {
            run_workbench_git(&beta, &["checkout", "--detach", "HEAD"], "Git")?;
            std::fs::write(
                beta.join("src/detached.rs"),
                "pub const FIXTURE: &str = \"detached dirty\";\n",
            )?;
        }
        "omega_workbench_git_unborn" => {}
        "omega_workbench_git_multi_repository" => {
            std::fs::write(
                beta.join("src/beta.rs"),
                "pub const FIXTURE: &str = \"selected beta\";\n",
            )?;
        }
        "omega_workbench_git_offline" => {
            std::fs::write(
                beta.join("src/offline.rs"),
                "pub const FIXTURE: &str = \"offline\";\n",
            )?;
        }
        "omega_workbench_git_reconnect" => {
            let remote = root_path.join("reconnect-remote.git");
            let remote_text = remote.to_string_lossy().to_string();
            run_workbench_git(&root_path, &["init", "--bare", remote_text.as_str()], "Git")?;
            run_workbench_git(
                &beta,
                &["remote", "add", "origin", remote_text.as_str()],
                "Git",
            )?;
            run_workbench_git(&beta, &["push", "-u", "origin", "codex/git-surface"], "Git")?;

            let peer = root_path.join("reconnect-peer");
            let peer_text = peer.to_string_lossy().to_string();
            run_workbench_git(
                &root_path,
                &[
                    "clone",
                    "--branch",
                    "codex/git-surface",
                    remote_text.as_str(),
                    peer_text.as_str(),
                ],
                "Git",
            )?;
            run_workbench_git(
                &peer,
                &["config", "user.email", "omega-workbench@example.invalid"],
                "Git",
            )?;
            run_workbench_git(
                &peer,
                &["config", "user.name", "Omega Workbench Proof"],
                "Git",
            )?;
            std::fs::write(
                peer.join("remote-reconnect.txt"),
                "remote reconnect commit\n",
            )?;
            run_workbench_git(&peer, &["add", "remote-reconnect.txt"], "Git")?;
            run_workbench_git(&peer, &["commit", "-m", "Advance reconnect remote"], "Git")?;
            run_workbench_git(&peer, &["push"], "Git")?;

            for index in 1..=3 {
                let path = format!("local-reconnect-{index}.txt");
                std::fs::write(
                    beta.join(&path),
                    format!("local reconnect commit {index}\n"),
                )?;
                run_workbench_git(&beta, &["add", &path], "Git")?;
                run_workbench_git(
                    &beta,
                    &["commit", "-m", &format!("Local reconnect commit {index}")],
                    "Git",
                )?;
            }
            run_workbench_git(&beta, &["fetch", "origin"], "Git")?;
            std::fs::write(
                beta.join("src/reconnected.rs"),
                "pub const FIXTURE: &str = \"reconnected\";\n",
            )?;
        }
        _ => unreachable!("Git scene was checked above"),
    }

    Ok(Some(WorkbenchGitDiskFixture {
        _root: root,
        worktrees: vec![
            ("alpha-worktree".into(), alpha),
            ("beta-worktree".into(), beta),
        ],
        active_worktree_id: "beta-worktree".into(),
    }))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn create_workbench_files_disk_fixture(
    scene_name: &str,
) -> Result<Option<WorkbenchFilesDiskFixture>> {
    if !is_workbench_files_scene(scene_name) {
        return Ok(None);
    }

    let root = tempfile::tempdir().context("creating Files scene directory")?;
    let root_path = root
        .path()
        .canonicalize()
        .context("canonicalizing Files scene directory")?;
    let create_worktree = |id: &str| -> Result<PathBuf> {
        let path = root_path.join(id);
        std::fs::create_dir_all(&path)
            .with_context(|| format!("creating Files scene worktree {id:?}"))?;
        Ok(path)
    };

    let fixture = match scene_name {
        "omega_workbench_files_wide"
        | "omega_workbench_files_narrow"
        | "omega_workbench_files_loading"
        | "omega_workbench_files_error" => {
            let worktree = create_worktree("ready-worktree")?;
            std::fs::create_dir_all(worktree.join("src"))?;
            std::fs::write(worktree.join("src/main.rs"), "fn main() {}\n")?;
            std::fs::write(worktree.join("README.md"), "# Ready fixture\n")?;
            WorkbenchFilesDiskFixture {
                _root: root,
                worktrees: vec![("ready-worktree".into(), worktree)],
                active_worktree_id: "ready-worktree".into(),
                selected_path: matches!(
                    scene_name,
                    "omega_workbench_files_wide" | "omega_workbench_files_narrow"
                )
                .then_some("README.md"),
            }
        }
        "omega_workbench_files_multi_root"
        | "omega_workbench_files_stale_filesystem_completion" => {
            let alpha = create_worktree("alpha-worktree")?;
            std::fs::write(alpha.join("alpha-only.txt"), "alpha\n")?;
            let beta = create_worktree("beta-worktree")?;
            std::fs::create_dir_all(beta.join("src"))?;
            std::fs::write(beta.join("beta-only.txt"), "beta\n")?;
            std::fs::write(beta.join("src/beta.rs"), "pub fn beta() {}\n")?;
            WorkbenchFilesDiskFixture {
                _root: root,
                worktrees: vec![
                    ("alpha-worktree".into(), alpha),
                    ("beta-worktree".into(), beta),
                ],
                active_worktree_id: "beta-worktree".into(),
                selected_path: Some("beta-only.txt"),
            }
        }
        "omega_workbench_files_empty" => {
            let worktree = create_worktree("empty-worktree")?;
            WorkbenchFilesDiskFixture {
                _root: root,
                worktrees: vec![("empty-worktree".into(), worktree)],
                active_worktree_id: "empty-worktree".into(),
                selected_path: None,
            }
        }
        _ => unreachable!("Files scene was checked above"),
    };
    Ok(Some(fixture))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn create_workbench_search_disk_fixture(
    scene_name: &str,
) -> Result<Option<WorkbenchSearchDiskFixture>> {
    if !is_workbench_search_scene(scene_name) {
        return Ok(None);
    }

    let root = tempfile::tempdir().context("creating Search scene directory")?;
    let root_path = root
        .path()
        .canonicalize()
        .context("canonicalizing Search scene directory")?;
    let alpha = root_path.join("alpha-worktree");
    let beta = root_path.join("beta-worktree");
    std::fs::create_dir_all(alpha.join("src"))?;
    std::fs::create_dir_all(beta.join("src"))?;
    std::fs::create_dir_all(beta.join("ignored"))?;
    std::fs::write(
        alpha.join("src/alpha.rs"),
        "pub const OLD_BINDING_ONLY: &str = \"omega_search_hit alpha\";\n",
    )?;
    std::fs::write(
        beta.join("src/first.rs"),
        "pub const FIRST: &str = \"omega_search_hit\";\n\
         pub const UNICODE: &str = \"café omega_search_hit\";\n",
    )?;
    std::fs::write(
        beta.join("src/second.rs"),
        format!(
            "pub const LONG: &str = \"{} omega_search_hit\";\n",
            "0123456789".repeat(24)
        ),
    )?;
    std::fs::write(beta.join("README.md"), "# omega_search_hit fixture\n")?;
    std::fs::write(beta.join(".gitignore"), "ignored/\n")?;
    std::fs::write(
        beta.join("ignored/generated.txt"),
        "omega_search_hit must remain excluded by default\n",
    )?;
    Ok(Some(WorkbenchSearchDiskFixture {
        _root: root,
        worktrees: vec![
            ("alpha-worktree".into(), alpha),
            ("beta-worktree".into(), beta),
        ],
        active_worktree_id: "beta-worktree".into(),
    }))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn create_workbench_review_disk_fixture(
    scene_name: &str,
) -> Result<Option<WorkbenchReviewDiskFixture>> {
    if !is_workbench_review_scene(scene_name) {
        return Ok(None);
    }

    let root = tempfile::tempdir().context("creating Review scene directory")?;
    let root_path = root
        .path()
        .canonicalize()
        .context("canonicalizing Review scene directory")?;
    let alpha = root_path.join("alpha-worktree");
    let beta = root_path.join("beta-worktree");
    std::fs::create_dir_all(alpha.join("src"))?;
    std::fs::create_dir_all(beta.join("src"))?;
    std::fs::write(
        alpha.join("src/foreign_thread_only.rs"),
        "pub const FOREIGN_THREAD_ONLY: bool = false;\n",
    )?;

    let mut main_source = String::new();
    for row in 0..40 {
        match row {
            0 => main_source.push_str("use zed::old_review;\n"),
            20 => main_source.push_str("const REVIEW_MODE: bool = false;\n"),
            35 => main_source.push_str("const STREAM_REVISION: usize = 0;\n"),
            _ => main_source.push_str(&format!("// fixture row {row:02}\n")),
        }
    }
    std::fs::write(beta.join("src/main.rs"), main_source)?;
    std::fs::write(
        beta.join("src/previous_name.rs"),
        "// renamed fixture\npub const NAME: &str = \"old\";\n",
    )?;
    std::fs::write(
        beta.join("src/obsolete.rs"),
        "pub fn obsolete() {\n    unreachable!();\n}\n",
    )?;
    std::fs::write(
        beta.join("src/conflicted.rs"),
        "pub fn conflicted() {\n    let state = \"base\";\n}\n",
    )?;
    initialize_workbench_git_fixture(&alpha)?;
    initialize_workbench_git_fixture(&beta)?;

    Ok(Some(WorkbenchReviewDiskFixture {
        _root: root,
        worktrees: vec![
            ("alpha-worktree".into(), alpha),
            ("beta-worktree".into(), beta),
        ],
        active_worktree_id: "beta-worktree".into(),
    }))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn create_workbench_terminal_disk_fixture(
    scene_name: &str,
) -> Result<Option<WorkbenchTerminalDiskFixture>> {
    if !is_workbench_terminal_scene(scene_name) {
        return Ok(None);
    }

    let root = tempfile::tempdir().context("creating Terminal scene directory")?;
    let root_path = root
        .path()
        .canonicalize()
        .context("canonicalizing Terminal scene directory")?;
    let alpha = root_path.join("alpha-worktree");
    let beta = root_path.join("beta-worktree");
    std::fs::create_dir_all(alpha.join("foreign"))?;
    std::fs::create_dir_all(beta.join("src"))?;
    std::fs::write(
        alpha.join("foreign/owner.txt"),
        "foreign terminal owner fixture\n",
    )?;
    std::fs::write(
        beta.join("src/main.rs"),
        "fn main() {\n    println!(\"terminal fixture\");\n}\n",
    )?;
    std::fs::write(beta.join("README.md"), "# Terminal fixture\n")?;
    initialize_workbench_git_fixture(&alpha)?;
    initialize_workbench_git_fixture(&beta)?;

    Ok(Some(WorkbenchTerminalDiskFixture {
        _root: root,
        worktrees: vec![
            ("alpha-worktree".into(), alpha),
            ("beta-worktree".into(), beta),
        ],
        active_worktree_id: "beta-worktree".into(),
    }))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn add_workbench_disk_worktrees(
    workspace_window: WindowHandle<Workspace>,
    worktrees: &[(String, PathBuf)],
    fixture_label: &str,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    for (_, path) in worktrees {
        let task = workspace_window
            .update(cx, |workspace, _window, cx| {
                workspace.project().update(cx, |project, cx| {
                    project.find_or_create_worktree(path, true, cx)
                })
            })
            .with_context(|| {
                format!(
                    "starting {fixture_label} scene worktree scan for {}",
                    path.display()
                )
            })?;
        cx.background_executor.allow_parking();
        let result = cx.foreground_executor.block_test(task);
        cx.background_executor.forbid_parking();
        let (worktree, _) = result
            .with_context(|| format!("adding {fixture_label} scene worktree {}", path.display()))?;
        let scan_complete = cx
            .read(|cx| {
                worktree
                    .read(cx)
                    .as_local()
                    .map(|worktree| worktree.scan_complete())
            })
            .with_context(|| {
                format!(
                    "{fixture_label} scene worktree {} is not local",
                    path.display()
                )
            })?;
        cx.background_executor.allow_parking();
        cx.foreground_executor.block_test(scan_complete);
        cx.background_executor.forbid_parking();
    }
    cx.run_until_parked();
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_workbench_shell_visual_capture(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    scene_name: &str,
    update_baseline: bool,
) -> Result<TestResult> {
    let scene = scene_spec(scene_name)
        .ok_or_else(|| anyhow::anyhow!("unknown workbench shell scene {scene_name:?}"))?;
    let files_fixture = create_workbench_files_disk_fixture(scene_name)?;
    let search_fixture = create_workbench_search_disk_fixture(scene_name)?;
    let review_fixture = create_workbench_review_disk_fixture(scene_name)?;
    let git_fixture = create_workbench_git_disk_fixture(scene_name)?;
    let terminal_fixture = create_workbench_terminal_disk_fixture(scene_name)?;
    let project = cx.update(|cx| {
        project::Project::local(
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            app_state.user_store.clone(),
            app_state.languages.clone(),
            app_state.fs.clone(),
            None,
            project::LocalProjectFlags {
                init_worktree_trust: false,
                ..Default::default()
            },
            cx,
        )
    });
    let bounds = Bounds {
        origin: point(px(0.), px(0.)),
        size: size(
            px(scene.viewport.width as f32),
            px(scene.viewport.height as f32),
        ),
    };
    let workspace_window: WindowHandle<Workspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        Workspace::new(None, project.clone(), app_state.clone(), window, cx)
                    })
                },
            )
        })
        .with_context(|| format!("opening workbench shell scene {scene_name:?}"))?;
    cx.run_until_parked();
    if let Some(fixture) = files_fixture.as_ref() {
        add_workbench_disk_worktrees(workspace_window, &fixture.worktrees, "Files", cx)?;
    }
    if let Some(fixture) = search_fixture.as_ref() {
        add_workbench_disk_worktrees(workspace_window, &fixture.worktrees, "Search", cx)?;
    }
    if let Some(fixture) = review_fixture.as_ref() {
        add_workbench_disk_worktrees(workspace_window, &fixture.worktrees, "Review", cx)?;
    }
    if let Some(fixture) = git_fixture.as_ref() {
        add_workbench_disk_worktrees(workspace_window, &fixture.worktrees, "Git", cx)?;
    }
    if let Some(fixture) = terminal_fixture.as_ref() {
        add_workbench_disk_worktrees(workspace_window, &fixture.worktrees, "Terminal", cx)?;
    }

    let result = run_omega_workbench_shell_visual_capture_in_window(
        workspace_window,
        cx,
        scene_name,
        files_fixture.as_ref(),
        search_fixture.as_ref(),
        review_fixture.as_ref(),
        git_fixture.as_ref(),
        terminal_fixture.as_ref(),
        update_baseline,
    );
    if files_fixture.is_some()
        || search_fixture.is_some()
        || review_fixture.is_some()
        || git_fixture.is_some()
        || terminal_fixture.is_some()
    {
        cx.update(|cx| {
            let worktree_ids = project
                .read(cx)
                .visible_worktrees(cx)
                .map(|worktree| worktree.read(cx).id())
                .collect::<Vec<_>>();
            project.update(cx, |project, cx| {
                for worktree_id in worktree_ids {
                    project.remove_worktree(worktree_id, cx);
                }
            });
        });
        cx.run_until_parked();
    }
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();
    cx.run_until_parked();
    for _ in 0..15 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }
    result
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_workbench_shell_visual_capture_in_window(
    workspace_window: WindowHandle<Workspace>,
    cx: &mut VisualTestAppContext,
    scene_name: &str,
    files_fixture: Option<&WorkbenchFilesDiskFixture>,
    search_fixture: Option<&WorkbenchSearchDiskFixture>,
    review_fixture: Option<&WorkbenchReviewDiskFixture>,
    git_fixture: Option<&WorkbenchGitDiskFixture>,
    terminal_fixture: Option<&WorkbenchTerminalDiskFixture>,
    update_baseline: bool,
) -> Result<TestResult> {
    use agent_ui::AgentPanel;
    use workspace::dock::Panel as _;

    let (weak_workspace, async_window_context) = workspace_window
        .update(cx, |workspace, window, cx| {
            (workspace.weak_handle(), window.to_async(cx))
        })
        .context("getting workbench shell workspace handle")?;
    let project_panel = match workspace_window
        .update(cx, |workspace, _window, cx| {
            workspace.panel::<ProjectPanel>(cx)
        })
        .context("checking for a workspace-owned ProjectPanel")?
    {
        Some(project_panel) => project_panel,
        None => {
            cx.background_executor.allow_parking();
            let project_panel = cx
                .foreground_executor
                .block_test(ProjectPanel::load(weak_workspace, async_window_context))
                .context("loading ProjectPanel before AgentPanel")?;
            cx.background_executor.forbid_parking();
            workspace_window
                .update(cx, |workspace, window, cx| {
                    workspace.add_panel(project_panel.clone(), window, cx);
                })
                .context("adding the workspace-owned ProjectPanel")?;
            project_panel
        }
    };
    cx.run_until_parked();
    let workspace_project_panel = workspace_window
        .update(cx, |workspace, _window, cx| {
            workspace.panel::<ProjectPanel>(cx)
        })
        .context("reading the workspace-owned ProjectPanel")?
        .context("ProjectPanel was not registered in the workspace")?;
    anyhow::ensure!(
        workspace_project_panel.entity_id() == project_panel.entity_id(),
        "the Files surface did not retain the one workspace-owned ProjectPanel"
    );
    record_workbench_semantic_check(scene_name, "files-single-workspace-owned-project-panel");

    let (weak_workspace, async_window_context) = workspace_window
        .update(cx, |workspace, window, cx| {
            (workspace.weak_handle(), window.to_async(cx))
        })
        .context("getting workbench shell workspace handle for GitPanel")?;
    let git_panel = match workspace_window
        .update(cx, |workspace, _window, cx| workspace.panel::<GitPanel>(cx))
        .context("checking for a workspace-owned GitPanel")?
    {
        Some(git_panel) => git_panel,
        None => {
            cx.background_executor.allow_parking();
            let git_panel = cx
                .foreground_executor
                .block_test(GitPanel::load(weak_workspace, async_window_context))
                .context("loading GitPanel before AgentPanel")?;
            cx.background_executor.forbid_parking();
            workspace_window
                .update(cx, |workspace, window, cx| {
                    workspace.add_panel(git_panel.clone(), window, cx);
                })
                .context("adding the workspace-owned GitPanel")?;
            git_panel
        }
    };
    cx.run_until_parked();
    let workspace_git_panel = workspace_window
        .update(cx, |workspace, _window, cx| workspace.panel::<GitPanel>(cx))
        .context("reading the workspace-owned GitPanel")?
        .context("GitPanel was not registered in the workspace")?;
    anyhow::ensure!(
        workspace_git_panel.entity_id() == git_panel.entity_id(),
        "the Git surface did not retain the one workspace-owned GitPanel"
    );
    record_workbench_semantic_check(scene_name, "git-single-workspace-owned-panel");

    if is_workbench_terminal_scene(scene_name) {
        let (weak_workspace, async_window_context) = workspace_window
            .update(cx, |workspace, window, cx| {
                (workspace.weak_handle(), window.to_async(cx))
            })
            .context("getting workbench shell workspace handle for TerminalPanel")?;
        let terminal_panel = match workspace_window
            .update(cx, |workspace, _window, cx| {
                workspace.panel::<TerminalPanel>(cx)
            })
            .context("checking for a workspace-owned TerminalPanel")?
        {
            Some(terminal_panel) => terminal_panel,
            None => {
                cx.background_executor.allow_parking();
                let terminal_panel = cx
                    .foreground_executor
                    .block_test(TerminalPanel::load(weak_workspace, async_window_context))
                    .context("loading TerminalPanel before AgentPanel")?;
                cx.background_executor.forbid_parking();
                workspace_window
                    .update(cx, |workspace, window, cx| {
                        workspace.add_panel(terminal_panel.clone(), window, cx);
                    })
                    .context("adding the workspace-owned TerminalPanel")?;
                terminal_panel
            }
        };
        cx.run_until_parked();
        let workspace_terminal_panel = workspace_window
            .update(cx, |workspace, _window, cx| {
                workspace.panel::<TerminalPanel>(cx)
            })
            .context("reading the workspace-owned TerminalPanel")?
            .context("TerminalPanel was not registered in the workspace")?;
        anyhow::ensure!(
            workspace_terminal_panel.entity_id() == terminal_panel.entity_id(),
            "the Terminal surface did not retain the one workspace-owned TerminalPanel"
        );
        record_workbench_semantic_check(scene_name, "terminal-single-workspace-owned-panel");
    }

    let (weak_workspace, async_window_context) = workspace_window
        .update(cx, |workspace, window, cx| {
            (workspace.weak_handle(), window.to_async(cx))
        })
        .context("getting workbench shell workspace handle after native panels")?;
    cx.background_executor.allow_parking();
    let panel = cx
        .foreground_executor
        .block_test(AgentPanel::load(weak_workspace, async_window_context))
        .context("loading AgentPanel after ProjectPanel for workbench shell scene")?;
    cx.background_executor.forbid_parking();

    workspace_window
        .update(cx, |workspace, window, cx| {
            panel.update(cx, |panel, cx| {
                panel.enable_workbench_shell_for_tests(cx);
                panel.set_zoomed(true, window, cx);
            });
            workspace.add_panel(panel.clone(), window, cx);
            workspace.open_panel::<AgentPanel>(window, cx);
            AgentPanel::open_front_door(window, cx);
        })
        .context("mounting production AgentPanel workbench shell")?;
    cx.run_until_parked();

    let activated = cx
        .update_window(workspace_window.into(), |_, window, cx| {
            panel.update(cx, |panel, cx| {
                panel.activate_prepared_omega_for_tests(window, cx)
            })
        })
        .context("activating the workbench shell's prepared Omega session")?;
    anyhow::ensure!(
        activated,
        "workbench shell scene {scene_name:?} did not reach Omega Ready"
    );
    cx.run_until_parked();

    anyhow::ensure!(
        cx.read(|cx| panel.read(cx).active_thread_id(cx).is_some()),
        "workbench shell scene {scene_name:?} has no active production thread"
    );

    // "Replace the new-thread mode screen with a composer executor dropdown"
    // defers the Omega session to the first accepted send, so the activated
    // front-door conversation is Connected with zero threads — a state in
    // which repository-identity mutation is deliberately unavailable
    // ("The active agent session is unavailable"). The workbench fixtures
    // photograph docks around a live thread, so send one deterministic turn
    // to materialize the session and its active thread view before the docks
    // are configured, exactly as the concurrent-supervision fixtures do.
    let fixture_conversation = cx
        .read(|cx| {
            let thread_id = panel.read(cx).active_thread_id(cx)?;
            panel
                .read(cx)
                .conversation_view_for_id(&thread_id, cx)
                .cloned()
        })
        .context("the workbench fixture's front-door conversation is unavailable")?;
    cx.update_window(workspace_window.into(), |_root, window, cx| {
        fixture_conversation.update(cx, |conversation, cx| {
            conversation.set_composer_text_for_tests("Workbench fixture turn.", window, cx);
            conversation.send_for_tests(window, cx);
        });
    })?;
    cx.run_until_parked();
    // This hermetic fixture has no language model, so the materializing turn
    // ends interrupted ("agent error"). The interruption is turn history, not
    // the state under test; clear it so the Plan surface projects the Ready
    // lifecycle its fixtures expect.
    let fixture_acp_thread = cx.read(|cx| {
        panel
            .read(cx)
            .active_thread_view_for_tests()
            .and_then(|conversation| conversation.read(cx).root_thread_view())
            .map(|view| view.read(cx).thread.clone())
    });
    if let Some(fixture_acp_thread) = fixture_acp_thread {
        // The turn error lands asynchronously; wait for the interruption to
        // actually appear before clearing, or the clear races the error and
        // loses. The thread reads idle both before the turn starts and after
        // it fails, so idleness cannot stand in for settlement here.
        for _ in 0..12288 {
            cx.run_until_parked();
            if cx.read(|cx| fixture_acp_thread.read(cx).plan_interruption().is_some()) {
                break;
            }
            cx.advance_clock(Duration::from_millis(10));
            std::thread::sleep(Duration::from_millis(1));
        }
        anyhow::ensure!(
            cx.read(|cx| fixture_acp_thread.read(cx).plan_interruption().is_some()),
            "workbench fixture turn did not reach its deterministic terminal failure"
        );
        cx.update(|cx| {
            fixture_acp_thread.update(cx, |thread, cx| {
                thread.set_plan_interruption_for_tests(None, cx);
            });
            panel.update(cx, |panel, cx| {
                panel.dismiss_all_notifications(cx);
            });
        });
        workspace_window.update(cx, |workspace, _window, cx| {
            workspace.clear_all_notifications(cx);
        })?;
        cx.run_until_parked();
    }

    let configuration = configure_workbench_shell_scene(
        scene_name,
        workspace_window,
        &panel,
        &project_panel,
        files_fixture,
        search_fixture,
        review_fixture,
        git_fixture,
        terminal_fixture,
        cx,
    );
    if let Err(error) = configuration {
        if is_workbench_review_scene(scene_name) {
            teardown_workbench_review(&panel, cx).log_err();
        }
        return Err(error);
    }
    cx.update(|cx| {
        panel.update(cx, |panel, cx| {
            panel.dismiss_all_notifications(cx);
        });
    });
    workspace_window.update(cx, |workspace, _window, cx| {
        workspace.clear_all_notifications(cx);
    })?;
    cx.run_until_parked();
    let result = run_visual_test(scene_name, workspace_window.into(), cx, update_baseline);
    if is_workbench_review_scene(scene_name) {
        let cleanup_result = teardown_workbench_review(&panel, cx);
        if result.is_ok() {
            cleanup_result?;
        } else {
            cleanup_result.log_err();
        }
    }
    result
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn dispatch_workbench_action(
    workspace_window: WindowHandle<Workspace>,
    action: Box<dyn gpui::Action>,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    cx.update_window(workspace_window.into(), move |_, window, cx| {
        window.dispatch_action(action, cx)
    })?;
    cx.run_until_parked();
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn focus_workbench_selector(
    window: gpui::AnyWindowHandle,
    selector: &str,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    cx.set_debug_accessibility_active(window, true)?;
    for _ in 0..128 {
        let snapshot = cx.debug_render_snapshot(window)?;
        if SemanticProbe::new(&snapshot)
            .require_focus(selector, true)
            .is_ok()
        {
            return Ok(());
        }
        cx.update_window(window, |_, window, cx| window.focus_next(cx))?;
        cx.run_until_parked();
    }
    anyhow::bail!("could not focus workbench selector {selector:?} through GPUI tab order")
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn select_workbench_identity(
    workspace_window: WindowHandle<Workspace>,
    panel: &Entity<agent_ui::AgentPanel>,
    worktree_path: &Path,
    fixture_label: &str,
    cx: &mut VisualTestAppContext,
) -> Result<omega_workbench_state::RepositoryBinding> {
    let (target_binding, current_binding) = cx.read(|cx| {
        let identity = panel
            .read(cx)
            .workbench_identity_for_tests()
            .with_context(|| format!("{fixture_label} scene has no production identity"))?;
        let target_binding = identity
            .candidates
            .iter()
            .find(|candidate| candidate.worktree_abs_path == worktree_path)
            .map(|candidate| candidate.binding.clone())
            .with_context(|| {
                format!(
                    "{fixture_label} scene identity has no candidate for {}",
                    worktree_path.display()
                )
            })?;
        Ok::<_, anyhow::Error>((target_binding, identity.binding().cloned()))
    })?;
    if current_binding.as_ref() != Some(&target_binding) {
        let (trigger_selector, row_selector) = if current_binding
            .as_ref()
            .is_some_and(|current| current.repository_id == target_binding.repository_id)
        {
            (
                "omega.workbench.control.identity.worktree",
                format!(
                    "omega.workbench.control.worktree.{}",
                    target_binding.worktree_id
                ),
            )
        } else {
            (
                "omega.workbench.control.identity.repository",
                format!(
                    "omega.workbench.control.repository.{}",
                    target_binding.repository_id
                ),
            )
        };
        let mut target_selection_ready = false;
        for _ in 0..4096 {
            cx.run_until_parked();
            if cx.read(|cx| {
                panel
                    .read(cx)
                    .workbench_identity_target_selection_ready_for_tests(cx)
            }) {
                target_selection_ready = true;
                break;
            }
            cx.advance_clock(Duration::from_millis(10));
            std::thread::sleep(Duration::from_millis(1));
        }
        anyhow::ensure!(
            target_selection_ready,
            "{fixture_label} identity target did not become selectable ({})",
            cx.read(|cx| {
                panel
                    .read(cx)
                    .workbench_identity_target_selection_unavailable_reason_for_tests(cx)
                    .unwrap_or_else(|| "no reason".into())
            })
        );
        cx.simulate_click_selector(workspace_window.into(), trigger_selector)?;
        let mut target_rendered = false;
        for _ in 0..512 {
            cx.run_until_parked();
            if cx
                .debug_render_snapshot(workspace_window.into())?
                .occurrences(&row_selector)
                .len()
                == 1
            {
                target_rendered = true;
                break;
            }
            cx.advance_clock(Duration::from_millis(10));
            std::thread::sleep(Duration::from_millis(1));
        }
        anyhow::ensure!(
            target_rendered,
            "{fixture_label} identity picker did not render target {row_selector:?}"
        );
        cx.simulate_click_selector(workspace_window.into(), &row_selector)?;
        cx.run_until_parked();
    }
    let (identity_binding, projection_binding) = cx.read(|cx| {
        let panel = panel.read(cx);
        (
            panel
                .workbench_identity_for_tests()
                .and_then(|identity| identity.binding())
                .cloned(),
            panel
                .workbench_projection_for_tests()
                .visible_projection()
                .and_then(|visible| visible.binding),
        )
    });
    anyhow::ensure!(
        identity_binding.as_ref() == Some(&target_binding)
            && projection_binding.as_ref() == Some(&target_binding),
        "{fixture_label} scene did not select the production identity for {}",
        worktree_path.display()
    );
    Ok(target_binding)
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn set_workbench_identity_observation_phase(
    workspace_window: WindowHandle<Workspace>,
    panel: &Entity<agent_ui::AgentPanel>,
    phase: agent_ui::thread_identity::IdentityPhase,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    let observation = cx.read(|cx| {
        let identity = panel
            .read(cx)
            .workbench_identity_for_tests()
            .context("workbench scene has no current identity")?;
        Ok::<_, anyhow::Error>(agent_ui::thread_identity::ThreadIdentityObservation {
            revision: identity.observation_revision.saturating_add(1),
            phase,
            candidates: identity.candidates.clone(),
        })
    })?;
    cx.update_window(workspace_window.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.set_workbench_identity_observation_for_tests(Some(observation), window, cx);
        });
    })?;
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn apply_workbench_plan_update(
    panel: &Entity<agent_ui::AgentPanel>,
    entries: Vec<acp::PlanEntry>,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    let thread = cx
        .read(|cx| panel.read(cx).active_agent_thread(cx))
        .context("active agent thread is unavailable for Plan update")?;
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread
                .handle_session_update(acp::SessionUpdate::Plan(acp::Plan::new(entries)), cx)
                .map_err(anyhow::Error::new)
        })
    })?;
    cx.run_until_parked();
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn workbench_plan_entries(completed: bool) -> Vec<acp::PlanEntry> {
    vec![
        acp::PlanEntry::new(
            "Inspect the active workbench",
            acp::PlanEntryPriority::High,
            acp::PlanEntryStatus::Completed,
        ),
        acp::PlanEntry::new(
            "Mount the native Plan surface",
            acp::PlanEntryPriority::High,
            if completed {
                acp::PlanEntryStatus::Completed
            } else {
                acp::PlanEntryStatus::InProgress
            },
        ),
        acp::PlanEntry::new(
            "Verify deterministic behavior",
            acp::PlanEntryPriority::Medium,
            if completed {
                acp::PlanEntryStatus::Completed
            } else {
                acp::PlanEntryStatus::Pending
            },
        ),
    ]
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn workbench_plan_snapshot(
    panel: &Entity<agent_ui::AgentPanel>,
    cx: &VisualTestAppContext,
) -> Result<(
    Entity<agent_ui::workbench_shell::NativePlanSurface>,
    agent_ui::workbench_shell::NativePlanSnapshot,
)> {
    let surface = cx
        .read(|cx| panel.read(cx).workbench_plan_surface_for_tests(cx))
        .context("active production Plan surface is unavailable")?;
    let snapshot = cx.read(|cx| surface.read(cx).snapshot());
    Ok((surface, snapshot))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn normalize_workbench_plan_snapshot(
    expected: &omega_workbench_harness::PlanSnapshotFixture,
    actual: &agent_ui::workbench_shell::NativePlanSnapshot,
) -> Result<omega_workbench_harness::PlanSnapshotFixture> {
    use agent_ui::plan_presentation::{PlanPriorityKind, PlanStatusKind};
    use agent_ui::workbench_shell::{NativePlanLifecycle, PlanSurfaceState};
    use omega_workbench_harness::{
        PlanLifecycleFixture, PlanPriorityFixture, PlanSnapshotFixture, PlanStatusFixture,
        PlanSurfaceStateFixture, PlanSurfaceStepFixture,
    };

    let normalize_step = |step: &agent_ui::workbench_shell::NativePlanStepSnapshot,
                          expected_step: &PlanSurfaceStepFixture| {
        PlanSurfaceStepFixture {
            id: expected_step.id,
            label: step.label.to_string(),
            status: match step.status {
                PlanStatusKind::Pending => PlanStatusFixture::Pending,
                PlanStatusKind::InProgress => PlanStatusFixture::InProgress,
                PlanStatusKind::Completed => PlanStatusFixture::Completed,
                PlanStatusKind::Unknown => PlanStatusFixture::Unknown,
            },
            priority: match step.priority {
                PlanPriorityKind::High => PlanPriorityFixture::High,
                PlanPriorityKind::Medium => PlanPriorityFixture::Medium,
                PlanPriorityKind::Low => PlanPriorityFixture::Low,
                PlanPriorityKind::Unknown => PlanPriorityFixture::Unknown,
            },
            source_entry_index: step.source_entry_index,
            historical: step.historical,
        }
    };
    anyhow::ensure!(
        actual.current_steps.len() == expected.current_steps.len()
            && actual.historical_steps.len() == expected.historical_steps.len(),
        "production Plan step cardinality differs from the typed scene contract"
    );
    let current_steps = actual
        .current_steps
        .iter()
        .zip(&expected.current_steps)
        .map(|(actual, expected)| normalize_step(actual, expected))
        .collect::<Vec<_>>();
    let historical_steps = actual
        .historical_steps
        .iter()
        .zip(&expected.historical_steps)
        .map(|(actual, expected)| normalize_step(actual, expected))
        .collect::<Vec<_>>();
    let selected_step_id = actual
        .selected_step_id
        .map(|selected| {
            actual
                .current_steps
                .iter()
                .chain(&actual.historical_steps)
                .position(|step| step.id == selected)
                .and_then(|index| {
                    expected
                        .current_steps
                        .iter()
                        .chain(&expected.historical_steps)
                        .nth(index)
                        .map(|step| step.id)
                })
                .context("selected production Plan step has no expected identity alias")
        })
        .transpose()?;

    Ok(PlanSnapshotFixture {
        binding: expected.binding.clone(),
        revision: actual.revision,
        lifecycle: match &actual.lifecycle {
            NativePlanLifecycle::Ready => PlanLifecycleFixture::Ready,
            NativePlanLifecycle::Interrupted(message) => {
                PlanLifecycleFixture::Interrupted(message.to_string())
            }
            NativePlanLifecycle::Stale => PlanLifecycleFixture::Stale,
            NativePlanLifecycle::Reconnecting => PlanLifecycleFixture::Reconnecting,
            NativePlanLifecycle::Malformed(message) => {
                PlanLifecycleFixture::Malformed(message.to_string())
            }
        },
        state: match &actual.state {
            PlanSurfaceState::Empty => PlanSurfaceStateFixture::Empty,
            PlanSurfaceState::Active {
                pending,
                in_progress,
                completed,
                unknown,
                total,
            } => PlanSurfaceStateFixture::Active {
                pending: u32::try_from(*pending).context("Plan pending count overflow")?,
                in_progress: u32::try_from(*in_progress)
                    .context("Plan in-progress count overflow")?,
                completed: u32::try_from(*completed).context("Plan completed count overflow")?,
                unknown: u32::try_from(*unknown).context("Plan unknown count overflow")?,
                total: u32::try_from(*total).context("Plan total count overflow")?,
            },
            PlanSurfaceState::AllComplete { total } => PlanSurfaceStateFixture::AllComplete {
                total: u32::try_from(*total).context("Plan total count overflow")?,
            },
            PlanSurfaceState::Historical {
                completed_plans,
                total,
            } => PlanSurfaceStateFixture::Historical {
                completed_plans: u32::try_from(*completed_plans)
                    .context("completed Plan count overflow")?,
                total: u32::try_from(*total).context("historical Plan count overflow")?,
            },
            PlanSurfaceState::Interrupted(_) => PlanSurfaceStateFixture::Interrupted,
            PlanSurfaceState::Stale => PlanSurfaceStateFixture::Stale,
            PlanSurfaceState::Reconnecting => PlanSurfaceStateFixture::Reconnecting,
            PlanSurfaceState::Malformed(_) => PlanSurfaceStateFixture::Malformed,
        },
        current_steps,
        historical_steps,
        active_step_id: actual
            .active_step_id
            .map(|active| {
                actual
                    .current_steps
                    .iter()
                    .position(|step| step.id == active)
                    .and_then(|index| expected.current_steps.get(index))
                    .map(|step| step.id)
                    .context("active production Plan step has no expected identity alias")
            })
            .transpose()?,
        selected_step_id,
        navigation_status: actual.navigation_status.as_ref().map(ToString::to_string),
        retained_surface_token: expected.retained_surface_token.clone(),
        rejected_update_count: u32::try_from(actual.rejected_update_count)
            .context("rejected Plan update count overflow")?,
    })
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn active_workbench_search(
    panel: &Entity<agent_ui::AgentPanel>,
    cx: &VisualTestAppContext,
) -> Result<(
    Entity<agent_ui::workbench_shell::NativeSearchSurface>,
    Entity<search::ProjectSearchView>,
)> {
    let surface = cx
        .read(|cx| panel.read(cx).workbench_search_surface_for_tests(cx))
        .context("active production Search surface is unavailable")?;
    let search_view = cx.read(|cx| surface.read(cx).search_view().clone());
    Ok((surface, search_view))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn ensure_workbench_search_open(
    workspace_window: WindowHandle<Workspace>,
    panel: &Entity<agent_ui::AgentPanel>,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    let is_open = cx
        .read(|cx| panel.read(cx).workbench_projection_for_tests().clone())
        .visible_projection()
        .is_some_and(|visible| {
            visible.effective_surface == Some(omega_workbench_state::WorkSurface::Search)
                && visible.dock_open
        });
    if !is_open {
        dispatch_workbench_action(
            workspace_window,
            Box::new(agent_ui::workbench_shell::SelectSearch),
            cx,
        )?;
    }
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn workbench_search_snapshot(
    search_view: &Entity<search::ProjectSearchView>,
    cx: &VisualTestAppContext,
) -> search::project_search::ProjectSearchTestSnapshot {
    cx.read(|cx| search_view.read(cx).test_snapshot(cx))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn wait_for_workbench_search(
    search_view: &Entity<search::ProjectSearchView>,
    cx: &mut VisualTestAppContext,
) -> Result<search::project_search::ProjectSearchTestSnapshot> {
    for _ in 0..128 {
        cx.run_until_parked();
        let snapshot = workbench_search_snapshot(search_view, cx);
        if !snapshot.pending
            && !matches!(
                snapshot.lifecycle,
                search::project_search::ProjectSearchLifecycle::Running { .. }
            )
        {
            return Ok(snapshot);
        }
        cx.advance_clock(Duration::from_millis(10));
    }
    let snapshot = workbench_search_snapshot(search_view, cx);
    anyhow::bail!(
        "native Search did not settle: lifecycle={:?}, pending={}, generation={}",
        snapshot.lifecycle,
        snapshot.pending,
        snapshot.generation
    )
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn perform_workbench_search(
    workspace_window: WindowHandle<Workspace>,
    search_view: &Entity<search::ProjectSearchView>,
    query: &str,
    cx: &mut VisualTestAppContext,
) -> Result<search::project_search::ProjectSearchTestSnapshot> {
    cx.update_window(workspace_window.into(), |_, window, cx| {
        search_view.update(cx, |search_view, cx| {
            search_view.test_set_query(query, window, cx);
            search_view.test_start_search(cx);
        });
    })?;
    wait_for_workbench_search(search_view, cx)
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn start_pending_workbench_search(
    workspace_window: WindowHandle<Workspace>,
    search_view: &Entity<search::ProjectSearchView>,
    query_text: &str,
    cx: &mut VisualTestAppContext,
) -> Result<async_channel::Sender<project::search::SearchResult>> {
    let query = search::SearchOptions::NONE.build_query(
        query_text,
        util::paths::PathMatcher::default(),
        util::paths::PathMatcher::default(),
        true,
        None,
    )?;
    let (sender, receiver) = async_channel::unbounded();
    cx.update_window(workspace_window.into(), |_, window, cx| {
        search_view.update(cx, |search_view, cx| {
            search_view.test_set_query(query_text, window, cx);
            search_view.test_start_search_results(
                query,
                project::project_search::SearchResults {
                    task_handle: gpui::Task::ready(()),
                    rx: receiver,
                },
                cx,
            );
        });
    })?;
    cx.run_until_parked();
    let snapshot = workbench_search_snapshot(search_view, cx);
    anyhow::ensure!(
        snapshot.pending
            && matches!(
                snapshot.lifecycle,
                search::project_search::ProjectSearchLifecycle::Running {
                    request,
                    activity: search::project_search::ProjectSearchActivity::Searching,
                } if request.generation == snapshot.generation
                    && request.worktree_id == snapshot.worktree_scope
            ),
        "controlled native Search did not enter its typed running state: {snapshot:?}"
    );
    Ok(sender)
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn active_workbench_review(
    panel: &Entity<agent_ui::AgentPanel>,
    cx: &VisualTestAppContext,
) -> Result<(
    Entity<agent_ui::workbench_shell::NativeReviewSurface>,
    Entity<agent_ui::AgentDiffPane>,
)> {
    let surface = cx
        .read(|cx| panel.read(cx).workbench_review_surface_for_tests(cx))
        .context("active production Review surface is unavailable")?;
    let pane = cx.read(|cx| surface.read(cx).diff_pane().clone());
    Ok((surface, pane))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn active_workbench_git(
    panel: &Entity<agent_ui::AgentPanel>,
    cx: &VisualTestAppContext,
) -> Result<(
    Entity<agent_ui::workbench_shell::NativeGitSurface>,
    Entity<GitPanel>,
)> {
    let surface = cx
        .read(|cx| panel.read(cx).workbench_git_surface_for_tests(cx))
        .context("active production Git surface is unavailable")?;
    let git_panel = cx.read(|cx| surface.read(cx).git_panel().clone());
    Ok((surface, git_panel))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn active_workbench_terminal(
    panel: &Entity<agent_ui::AgentPanel>,
    cx: &VisualTestAppContext,
) -> Result<(
    Entity<agent_ui::workbench_shell::NativeTerminalSurface>,
    Entity<TerminalPanel>,
)> {
    let surface = cx
        .read(|cx| panel.read(cx).workbench_terminal_surface_for_tests())
        .context("active production Terminal surface is unavailable")?;
    let terminal_panel = cx.read(|cx| surface.read(cx).terminal_panel().clone());
    Ok((surface, terminal_panel))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn wait_for_workbench_git_snapshot(
    git_panel: &Entity<GitPanel>,
    expected_scope: GitPanelRepositoryScope,
    expected_entry_count: usize,
    cx: &mut VisualTestAppContext,
) -> Result<GitPanelStateSnapshot> {
    for _ in 0..512 {
        cx.run_until_parked();
        let snapshot =
            cx.update(|cx| git_panel.update(cx, |git_panel, cx| git_panel.state_snapshot(cx)));
        if snapshot.repository_scope == Some(expected_scope)
            && snapshot.repository_scope_available
            && snapshot.repository_id == Some(expected_scope.repository_id)
            && snapshot.status_entries.len() == expected_entry_count
            && snapshot.pending_operation.staging_paths.is_empty()
        {
            return Ok(snapshot);
        }
        cx.advance_clock(Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(1));
    }
    let snapshot =
        cx.update(|cx| git_panel.update(cx, |git_panel, cx| git_panel.state_snapshot(cx)));
    anyhow::bail!("native Git panel did not settle for scope {expected_scope:?}: {snapshot:?}")
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn select_workbench_git_path(
    workspace_window: WindowHandle<Workspace>,
    git_panel: &Entity<GitPanel>,
    expected_path: &str,
    cx: &mut VisualTestAppContext,
) -> Result<GitPanelStateSnapshot> {
    cx.update_window(workspace_window.into(), |_, window, cx| {
        git_panel.focus_handle(cx).focus(window, cx);
    })?;
    for _ in 0..64 {
        let snapshot =
            cx.update(|cx| git_panel.update(cx, |git_panel, cx| git_panel.state_snapshot(cx)));
        if snapshot
            .selection
            .as_ref()
            .and_then(|selection| selection.repo_path.as_ref())
            .is_some_and(|path| path.as_unix_str() == expected_path)
        {
            return Ok(snapshot);
        }
        dispatch_workbench_action(workspace_window, Box::new(git_ui::git_panel::NextEntry), cx)?;
    }
    let snapshot =
        cx.update(|cx| git_panel.update(cx, |git_panel, cx| git_panel.state_snapshot(cx)));
    anyhow::bail!(
        "native Git panel could not select {expected_path:?}; final selection was {:?}",
        snapshot.selection
    )
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn normalized_workbench_git_snapshot(
    expected: &omega_workbench_harness::GitSnapshotFixture,
    surface: &agent_ui::workbench_shell::NativeGitSurface,
    snapshot: &GitPanelStateSnapshot,
    identity: &agent_ui::thread_identity::ThreadIdentityCandidate,
    requested_mutations: Vec<omega_workbench_harness::GitMutationFixture>,
    ignored_stale_refresh_count: u32,
) -> Result<omega_workbench_harness::GitSnapshotFixture> {
    use git::status::{FileStatus, StageStatus, StatusCode};
    use omega_workbench_harness::{
        GitBranchFixture, GitFileStatusFixture, GitLifecycleFixture, GitSnapshotFixture,
        GitStagingStateFixture, GitStatusCountsFixture, GitStatusEntryFixture,
    };

    let lifecycle = match surface.lifecycle() {
        agent_ui::workbench_shell::NativeGitLifecycle::Loading => GitLifecycleFixture::Loading,
        agent_ui::workbench_shell::NativeGitLifecycle::Clean
        | agent_ui::workbench_shell::NativeGitLifecycle::Dirty
        | agent_ui::workbench_shell::NativeGitLifecycle::Conflicted
        | agent_ui::workbench_shell::NativeGitLifecycle::Detached
        | agent_ui::workbench_shell::NativeGitLifecycle::Unborn
        | agent_ui::workbench_shell::NativeGitLifecycle::OperationPending => {
            GitLifecycleFixture::Ready
        }
        agent_ui::workbench_shell::NativeGitLifecycle::Offline => GitLifecycleFixture::Offline,
        agent_ui::workbench_shell::NativeGitLifecycle::Reconnecting => {
            GitLifecycleFixture::Reconnecting
        }
        agent_ui::workbench_shell::NativeGitLifecycle::RepositoryRemoved => {
            GitLifecycleFixture::RepositoryRemoved
        }
        agent_ui::workbench_shell::NativeGitLifecycle::Error(error) => {
            GitLifecycleFixture::Error(error.to_string())
        }
    };
    let branch = match (&snapshot.head, &identity.branch) {
        (Some(GitPanelHeadState::Branch(name)), _) => Some(GitBranchFixture::Branch {
            name: name.to_string(),
            ahead: snapshot
                .tracking
                .as_ref()
                .map(|tracking| tracking.ahead)
                .unwrap_or_default(),
            behind: snapshot
                .tracking
                .as_ref()
                .map(|tracking| tracking.behind)
                .unwrap_or_default(),
        }),
        (Some(GitPanelHeadState::Detached), _) => expected.branch.clone(),
        (Some(GitPanelHeadState::Unborn { branch }), _) => Some(GitBranchFixture::Unborn {
            name: branch
                .as_ref()
                .map(ToString::to_string)
                .or_else(|| match &identity.branch {
                    agent_ui::thread_identity::BranchIdentity::Branch(name) => {
                        Some(name.to_string())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| "omega/initial".into()),
        }),
        (None, _) => None,
    };
    let status_entries = snapshot
        .status_entries
        .iter()
        .map(|entry| {
            let status = if entry.conflicted || entry.status.is_conflicted() {
                GitFileStatusFixture::Conflicted
            } else {
                match entry.status {
                    FileStatus::Untracked => GitFileStatusFixture::Untracked,
                    FileStatus::Ignored => {
                        anyhow::bail!(
                            "native Git status exposed ignored path {:?}",
                            entry.repo_path
                        )
                    }
                    FileStatus::Unmerged(_) => GitFileStatusFixture::Conflicted,
                    FileStatus::Tracked(status)
                        if matches!(
                            status.index_status,
                            StatusCode::Renamed | StatusCode::Copied
                        ) =>
                    {
                        GitFileStatusFixture::Renamed
                    }
                    status if status.is_deleted() => GitFileStatusFixture::Deleted,
                    status if status.is_created() => GitFileStatusFixture::Added,
                    _ => GitFileStatusFixture::Modified,
                }
            };
            let staging = if entry.conflicted || entry.status.is_conflicted() {
                GitStagingStateFixture::Conflict
            } else {
                match entry.staging {
                    StageStatus::Staged => GitStagingStateFixture::Staged,
                    StageStatus::Unstaged => GitStagingStateFixture::Unstaged,
                    StageStatus::PartiallyStaged => GitStagingStateFixture::PartiallyStaged,
                }
            };
            Ok::<_, anyhow::Error>(GitStatusEntryFixture {
                path: entry.repo_path.as_unix_str().to_string(),
                old_path: None,
                status: if entry.status == FileStatus::Untracked {
                    GitFileStatusFixture::Untracked
                } else {
                    status
                },
                staging,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut status_counts = GitStatusCountsFixture::default();
    for entry in &status_entries {
        match entry.staging {
            GitStagingStateFixture::Unstaged => {
                if entry.status == GitFileStatusFixture::Untracked {
                    status_counts.untracked = status_counts.untracked.saturating_add(1);
                } else {
                    status_counts.unstaged = status_counts.unstaged.saturating_add(1);
                }
            }
            GitStagingStateFixture::Staged => {
                status_counts.staged = status_counts.staged.saturating_add(1);
            }
            GitStagingStateFixture::PartiallyStaged => {
                status_counts.staged = status_counts.staged.saturating_add(1);
                status_counts.unstaged = status_counts.unstaged.saturating_add(1);
            }
            GitStagingStateFixture::Conflict => {
                status_counts.conflicts = status_counts.conflicts.saturating_add(1);
            }
        }
    }

    Ok(GitSnapshotFixture {
        binding: expected.binding.clone(),
        lifecycle,
        branch,
        status_counts,
        selected_path: snapshot
            .selection
            .as_ref()
            .and_then(|selection| selection.repo_path.as_ref())
            .map(|path| path.as_unix_str().to_string()),
        badge_count: u32::try_from(status_entries.len()).context("Git badge count overflowed")?,
        status_entries,
        pending_operation: expected.pending_operation.clone(),
        requested_mutations,
        ignored_stale_refresh_count,
        focus: expected.focus,
    })
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn teardown_workbench_review(
    panel: &Entity<agent_ui::AgentPanel>,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    let (_, review_pane) = active_workbench_review(panel, cx)?;
    let active_thread = cx
        .read(|cx| panel.read(cx).omega_active_acp_thread(cx))
        .context("tearing down Review without an active ACP thread")?;
    let generation = cx
        .read(|cx| {
            review_pane
                .read(cx)
                .binding_snapshot()
                .map(|binding| binding.checkpoint.generation())
        })
        .context("tearing down an unbound Review pane")?;
    cx.update(|cx| {
        review_pane.update(cx, |pane, cx| {
            pane.invalidate(generation, "Review proof completed", cx);
        });
        let action_log = active_thread.read(cx).action_log().clone();
        action_log.update(cx, |action_log, cx| {
            action_log.clear_tracked_buffers_for_tests(cx);
        });
    });
    cx.run_until_parked();
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn open_workbench_review_buffer(
    workspace_window: WindowHandle<Workspace>,
    worktree_id: project::WorktreeId,
    path: &str,
    cx: &mut VisualTestAppContext,
) -> Result<Entity<language::Buffer>> {
    let project_path = project::ProjectPath {
        worktree_id,
        path: util::rel_path::rel_path(path).into(),
    };
    let task = workspace_window
        .update(cx, |workspace, _window, cx| {
            workspace
                .project()
                .update(cx, |project, cx| project.open_buffer(project_path, cx))
        })
        .with_context(|| format!("opening Review fixture buffer {path:?}"))?;
    cx.background_executor.allow_parking();
    let buffer = cx
        .foreground_executor
        .block_test(task)
        .with_context(|| format!("loading Review fixture buffer {path:?}"))?;
    cx.background_executor.forbid_parking();
    Ok(buffer)
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn record_workbench_review_change(
    pane: &Entity<agent_ui::AgentDiffPane>,
    buffer: Entity<language::Buffer>,
    change: agent_ui::AgentDiffFixtureChange,
    cx: &mut VisualTestAppContext,
) {
    cx.update(|cx| {
        pane.update(cx, |pane, cx| {
            pane.record_buffer_change_for_tests(buffer, change, cx);
        });
    });
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn wait_for_recorded_workbench_review_edit(
    task: gpui::Task<Result<()>>,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    cx.background_executor.allow_parking();
    let result = cx.foreground_executor.block_test(task);
    cx.background_executor.forbid_parking();
    result?;
    cx.run_until_parked();
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn seed_standard_workbench_review(
    workspace_window: WindowHandle<Workspace>,
    pane: &Entity<agent_ui::AgentDiffPane>,
    worktree_id: project::WorktreeId,
    cx: &mut VisualTestAppContext,
) -> Result<(Entity<language::Buffer>, Entity<language::Buffer>)> {
    use agent_ui::AgentDiffFixtureChange;

    let main = open_workbench_review_buffer(workspace_window, worktree_id, "src/main.rs", cx)?;
    let (main_transaction, main_edit) = cx.update(|cx| {
        pane.update(cx, |pane, cx| {
            pane.record_buffer_change_for_tests(main.clone(), AgentDiffFixtureChange::Read, cx);
        });
        let transaction = main.update(cx, |buffer, cx| {
            buffer.edit(
                [
                    (
                        language::Point::new(0, 0)..language::Point::new(1, 0),
                        "use zed::review;\n",
                    ),
                    (
                        language::Point::new(20, 0)..language::Point::new(21, 0),
                        "const REVIEW_MODE: bool = true;\n",
                    ),
                ],
                None,
                cx,
            )
        });
        let edit = pane.update(cx, |pane, cx| {
            pane.record_buffer_edit_and_wait_for_tests(main.clone(), cx)
        });
        (transaction, edit)
    });
    main_transaction.context("standard Review main edit did not create a transaction")?;
    wait_for_recorded_workbench_review_edit(main_edit, cx)?;

    let settings =
        open_workbench_review_buffer(workspace_window, worktree_id, "src/settings.rs", cx)?;
    let settings_edit = cx.update(|cx| {
        pane.update(cx, |pane, cx| {
            pane.record_buffer_change_for_tests(
                settings.clone(),
                AgentDiffFixtureChange::Created,
                cx,
            );
        });
        settings.update(cx, |buffer, cx| {
            buffer.set_text("pub const REVIEW_ENABLED: bool = true;\n", cx);
        });
        pane.update(cx, |pane, cx| {
            pane.record_buffer_edit_and_wait_for_tests(settings.clone(), cx)
        })
    });
    wait_for_recorded_workbench_review_edit(settings_edit, cx)?;
    Ok((main, settings))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn seed_foreign_workbench_review(
    workspace_window: WindowHandle<Workspace>,
    pane: &Entity<agent_ui::AgentDiffPane>,
    worktree_id: project::WorktreeId,
    cx: &mut VisualTestAppContext,
) -> Result<(Entity<language::Buffer>, String)> {
    use agent_ui::AgentDiffFixtureChange;

    let buffer = open_workbench_review_buffer(
        workspace_window,
        worktree_id,
        "src/foreign_thread_only.rs",
        cx,
    )?;
    let (transaction, edit) = cx.update(|cx| {
        pane.update(cx, |pane, cx| {
            pane.record_buffer_change_for_tests(buffer.clone(), AgentDiffFixtureChange::Read, cx);
        });
        let transaction = buffer.update(cx, |buffer, cx| {
            buffer.edit(
                [(
                    language::Point::new(0, 0)..language::Point::new(1, 0),
                    "pub const FOREIGN_THREAD_ONLY: bool = true;\n",
                )],
                None,
                cx,
            )
        });
        let edit = pane.update(cx, |pane, cx| {
            pane.record_buffer_edit_and_wait_for_tests(buffer.clone(), cx)
        });
        (transaction, edit)
    });
    transaction.context("foreign-worktree Review edit did not create a transaction")?;
    wait_for_recorded_workbench_review_edit(edit, cx)?;
    let contents = cx.read(|cx| buffer.read(cx).text());
    Ok((buffer, contents))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn seed_workbench_review_scene(
    scene_name: &str,
    workspace_window: WindowHandle<Workspace>,
    pane: &Entity<agent_ui::AgentDiffPane>,
    worktree_id: project::WorktreeId,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    use agent_ui::AgentDiffFixtureChange;

    match scene_name {
        "omega_workbench_review_empty" | "omega_workbench_review_error" => {}
        "omega_workbench_review_multi_file"
        | "omega_workbench_review_selected_hunk"
        | "omega_workbench_review_narrow"
        | "omega_workbench_review_all_reviewed" => {
            seed_standard_workbench_review(workspace_window, pane, worktree_id, cx)?;
        }
        "omega_workbench_review_streaming_update" => {
            let (main, _) =
                seed_standard_workbench_review(workspace_window, pane, worktree_id, cx)?;
            let (transaction, edit) = cx.update(|cx| {
                let transaction = main.update(cx, |buffer, cx| {
                    buffer.edit(
                        [(
                            language::Point::new(35, 0)..language::Point::new(36, 0),
                            "const STREAM_REVISION: usize = 1;\n",
                        )],
                        None,
                        cx,
                    )
                });
                let edit = pane.update(cx, |pane, cx| {
                    pane.record_buffer_edit_and_wait_for_tests(main.clone(), cx)
                });
                (transaction, edit)
            });
            transaction.context("streaming Review edit did not create a transaction")?;
            wait_for_recorded_workbench_review_edit(edit, cx)?;
        }
        "omega_workbench_review_rename_delete" => {
            let renamed = open_workbench_review_buffer(
                workspace_window,
                worktree_id,
                "src/previous_name.rs",
                cx,
            )?;
            let (transaction, edit) = cx.update(|cx| {
                pane.update(cx, |pane, cx| {
                    pane.record_buffer_change_for_tests(
                        renamed.clone(),
                        AgentDiffFixtureChange::Read,
                        cx,
                    );
                });
                let transaction = renamed.update(cx, |buffer, cx| {
                    buffer.edit(
                        [(
                            language::Point::new(1, 0)..language::Point::new(2, 0),
                            "pub const NAME: &str = \"current\";\n",
                        )],
                        None,
                        cx,
                    )
                });
                let edit = pane.update(cx, |pane, cx| {
                    pane.record_buffer_edit_and_wait_for_tests(renamed.clone(), cx)
                });
                (transaction, edit)
            });
            transaction.context("renamed Review edit did not create a transaction")?;
            wait_for_recorded_workbench_review_edit(edit, cx)?;

            let renamed_path = project::ProjectPath {
                worktree_id,
                path: util::rel_path::rel_path("src/previous_name.rs").into(),
            };
            let new_path = project::ProjectPath {
                worktree_id,
                path: util::rel_path::rel_path("src/current_name.rs").into(),
            };
            let rename_task = workspace_window.update(cx, |workspace, _window, cx| {
                workspace.project().update(cx, |project, cx| {
                    let entry = project
                        .entry_for_path(&renamed_path, cx)
                        .context("renamed Review fixture entry is unavailable")?;
                    Ok::<_, anyhow::Error>(project.rename_entry(entry.id, new_path, cx))
                })
            })??;
            cx.background_executor.allow_parking();
            cx.foreground_executor
                .block_test(rename_task)
                .context("renaming Review fixture file")?;
            cx.background_executor.forbid_parking();

            let deleted =
                open_workbench_review_buffer(workspace_window, worktree_id, "src/obsolete.rs", cx)?;
            record_workbench_review_change(pane, deleted, AgentDiffFixtureChange::Deleted, cx);
            let deleted_path = project::ProjectPath {
                worktree_id,
                path: util::rel_path::rel_path("src/obsolete.rs").into(),
            };
            let delete_task = workspace_window
                .update(cx, |workspace, _window, cx| {
                    workspace
                        .project()
                        .update(cx, |project, cx| project.delete_file(deleted_path, cx))
                })?
                .context("deleted Review fixture entry is unavailable")?;
            cx.background_executor.allow_parking();
            cx.foreground_executor
                .block_test(delete_task)
                .context("deleting Review fixture file")?;
            cx.background_executor.forbid_parking();
        }
        "omega_workbench_review_conflict" => {
            let conflict = open_workbench_review_buffer(
                workspace_window,
                worktree_id,
                "src/conflicted.rs",
                cx,
            )?;
            let (transaction, edit) = cx.update(|cx| {
                pane.update(cx, |pane, cx| {
                    pane.record_buffer_change_for_tests(
                        conflict.clone(),
                        AgentDiffFixtureChange::Read,
                        cx,
                    );
                });
                let transaction = conflict.update(cx, |buffer, cx| {
                    buffer.set_conflict();
                    buffer.edit(
                        [(
                            language::Point::new(1, 0)..language::Point::new(2, 0),
                            "    let state = \"conflict\";\n",
                        )],
                        None,
                        cx,
                    )
                });
                let edit = pane.update(cx, |pane, cx| {
                    pane.record_buffer_edit_and_wait_for_tests(conflict.clone(), cx)
                });
                (transaction, edit)
            });
            transaction.context("conflicted Review edit did not create a transaction")?;
            wait_for_recorded_workbench_review_edit(edit, cx)?;
        }
        _ => anyhow::bail!("unknown Review workbench scene {scene_name:?}"),
    }
    cx.run_until_parked();
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn wait_for_workbench_review_changes(
    action_log: Entity<action_log::ActionLog>,
    worktree_id: project::WorktreeId,
    expected_file_count: usize,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    if expected_file_count == 0 {
        return Ok(());
    }
    let wait = cx.update(|cx| {
        cx.spawn(async move |cx| {
            for _ in 0..128 {
                let file_count = cx.read_entity(&action_log, |action_log, cx| {
                    action_log
                        .changed_buffers(cx)
                        .filter(|(buffer, _)| {
                            buffer
                                .read(cx)
                                .file()
                                .is_some_and(|file| file.worktree_id(cx) == worktree_id)
                        })
                        .count()
                });
                if file_count >= expected_file_count {
                    return Ok(());
                }
                cx.background_executor()
                    .timer(Duration::from_millis(10))
                    .await;
            }
            anyhow::bail!(
                "native Review action log did not publish {expected_file_count} changed files"
            )
        })
    });
    cx.background_executor.allow_parking();
    let result = cx.foreground_executor.block_test(wait);
    cx.background_executor.forbid_parking();
    result?;
    cx.run_until_parked();
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn wait_for_workbench_review_hunks(
    action_log: Entity<action_log::ActionLog>,
    worktree_id: project::WorktreeId,
    expected_hunk_count: usize,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    let wait = cx.update(|cx| {
        cx.spawn(async move |cx| {
            for _ in 0..128 {
                let hunk_count = cx.read_entity(&action_log, |action_log, cx| {
                    action_log
                        .changed_buffers(cx)
                        .filter(|(buffer, _)| {
                            buffer
                                .read(cx)
                                .file()
                                .is_some_and(|file| file.worktree_id(cx) == worktree_id)
                        })
                        .map(|(buffer, diff)| {
                            diff.read(cx).snapshot(cx).hunks(buffer.read(cx)).count()
                        })
                        .sum::<usize>()
                });
                if hunk_count == expected_hunk_count {
                    return Ok(());
                }
                cx.background_executor()
                    .timer(Duration::from_millis(10))
                    .await;
            }
            anyhow::bail!(
                "native Review action log did not settle at {expected_hunk_count} active hunks"
            )
        })
    });
    cx.background_executor.allow_parking();
    let result = cx.foreground_executor.block_test(wait);
    cx.background_executor.forbid_parking();
    result?;
    cx.run_until_parked();
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn normalized_workbench_review_snapshot(
    expected: &omega_workbench_harness::ReviewSessionFixture,
    snapshot: &agent_ui::AgentDiffPaneSnapshot,
    mutations: Vec<omega_workbench_harness::ReviewMutationFixture>,
) -> Result<omega_workbench_harness::ReviewSessionFixture> {
    use agent_ui::{AgentDiffFileState, AgentDiffLifecycle};
    use omega_workbench_harness::{
        ReviewFileFixture, ReviewFileStatusFixture, ReviewFocusFixture, ReviewHunkFixture,
        ReviewHunkStatusFixture, ReviewLifecycleFixture, ReviewSessionFixture,
    };

    let lifecycle = match &snapshot.lifecycle {
        AgentDiffLifecycle::Unbound => ReviewLifecycleFixture::Unbound,
        AgentDiffLifecycle::Loading => ReviewLifecycleFixture::Loading,
        AgentDiffLifecycle::Empty => ReviewLifecycleFixture::Empty,
        AgentDiffLifecycle::Ready => ReviewLifecycleFixture::Ready,
        AgentDiffLifecycle::Streaming => ReviewLifecycleFixture::Streaming,
        AgentDiffLifecycle::AllReviewed => ReviewLifecycleFixture::AllReviewed,
        AgentDiffLifecycle::Offline => ReviewLifecycleFixture::Offline,
        AgentDiffLifecycle::UnavailableCheckpoint(_) => {
            ReviewLifecycleFixture::UnavailableCheckpoint
        }
        AgentDiffLifecycle::UnsupportedBinary(_) => ReviewLifecycleFixture::UnsupportedBinary,
        AgentDiffLifecycle::Invalidated(_) => ReviewLifecycleFixture::Invalidated,
        AgentDiffLifecycle::Error(message) => ReviewLifecycleFixture::Error(message.to_string()),
    };

    let mut native_hunk_ids = BTreeSet::new();
    let files = snapshot
        .files
        .iter()
        .map(|file| {
            let expected_file = expected
                .files
                .iter()
                .find(|expected_file| expected_file.path == file.path)
                .with_context(|| {
                    format!("native Review published unexpected file {:?}", file.path)
                })?;
            let status = match file.state {
                AgentDiffFileState::Created => ReviewFileStatusFixture::Added,
                AgentDiffFileState::Modified => ReviewFileStatusFixture::Modified,
                AgentDiffFileState::Renamed => ReviewFileStatusFixture::Renamed,
                AgentDiffFileState::Deleted => ReviewFileStatusFixture::Deleted,
                AgentDiffFileState::Conflict => ReviewFileStatusFixture::Conflict,
            };
            let hunks = file
                .hunks
                .iter()
                .enumerate()
                .map(|(index, hunk)| {
                    anyhow::ensure!(
                        !hunk.id.is_empty() && native_hunk_ids.insert(hunk.id.as_str()),
                        "native Review hunk IDs must be non-empty and unique"
                    );
                    anyhow::ensure!(
                        hunk.old_byte_range.start <= hunk.old_byte_range.end,
                        "native Review hunk {:?} has a reversed base range",
                        hunk.id
                    );
                    let expected_hunk = expected_file.hunks.get(index).with_context(|| {
                        format!(
                            "native Review file {:?} published unexpected hunk {}",
                            file.path, hunk.id
                        )
                    })?;
                    let status = if matches!(file.state, AgentDiffFileState::Conflict) {
                        ReviewHunkStatusFixture::Conflict
                    } else {
                        ReviewHunkStatusFixture::Pending
                    };
                    Ok::<_, anyhow::Error>(ReviewHunkFixture {
                        id: expected_hunk.id.clone(),
                        start_row: hunk.range.start.row,
                        start_column: hunk.range.start.column,
                        end_row: hunk.range.end.row,
                        end_column: hunk.range.end.column,
                        status,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok::<_, anyhow::Error>(ReviewFileFixture {
                path: file.path.clone(),
                old_path: file.old_path.clone(),
                status,
                hunks,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let selected_hunk_id = match (&snapshot.selected_path, &snapshot.selected_range) {
        (Some(path), Some(range)) => {
            let file_index = snapshot
                .files
                .iter()
                .position(|file| &file.path == path)
                .context("native Review selected path is not in its file snapshot")?;
            let hunk_index = snapshot.files[file_index]
                .hunks
                .iter()
                .position(|hunk| hunk.range == *range)
                .context("native Review selected range is not in its hunk snapshot")?;
            expected
                .files
                .get(file_index)
                .and_then(|file| file.hunks.get(hunk_index))
                .map(|hunk| hunk.id.clone())
                .context("native Review selection has no typed fixture hunk")?
                .into()
        }
        (None, None) => None,
        _ => anyhow::bail!("native Review selection has an incomplete path/range identity"),
    };
    let focus = if snapshot.editor_focused {
        ReviewFocusFixture::Diff
    } else {
        ReviewFocusFixture::Surface
    };

    Ok(ReviewSessionFixture {
        binding: expected.binding.clone(),
        lifecycle,
        files,
        selected_file_path: snapshot.selected_path.clone(),
        selected_hunk_id,
        focus,
        mutations,
        pending_operation_count: u32::from(snapshot.pending_edit),
        ignored_stale_completion_count: u32::try_from(snapshot.stale_completions_ignored)
            .context("native Review stale-completion count overflowed")?,
    })
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn configure_workbench_shell_scene(
    scene_name: &str,
    workspace_window: WindowHandle<Workspace>,
    panel: &Entity<agent_ui::AgentPanel>,
    project_panel: &Entity<ProjectPanel>,
    files_fixture: Option<&WorkbenchFilesDiskFixture>,
    search_fixture: Option<&WorkbenchSearchDiskFixture>,
    review_fixture: Option<&WorkbenchReviewDiskFixture>,
    git_fixture: Option<&WorkbenchGitDiskFixture>,
    terminal_fixture: Option<&WorkbenchTerminalDiskFixture>,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    use agent_ui::workbench_shell::{
        BadgeTone, FocusActivityRail, FocusLastSurface, SelectFiles, SelectPlan, SurfaceBadge,
        ToggleRepositoryPicker,
    };

    let initial_projection = cx.read(|cx| panel.read(cx).workbench_projection_for_tests().clone());
    let active_thread_id = initial_projection
        .active_thread_id
        .as_ref()
        .context("workbench projection has no active thread")?;
    let active_thread = initial_projection
        .threads
        .get(active_thread_id)
        .context("workbench projection has no active thread state")?;
    let plan_surface = active_thread
        .available_surfaces
        .iter()
        .copied()
        .find(|surface| !surface.requires_binding())
        .context("workbench projection has no unbound surface")?;
    if !is_workbench_files_scene(scene_name)
        && !is_workbench_search_scene(scene_name)
        && !is_workbench_review_scene(scene_name)
        && !is_workbench_git_scene(scene_name)
        && !is_workbench_terminal_scene(scene_name)
        && !is_workbench_plan_scene(scene_name)
    {
        anyhow::ensure!(
            active_thread.binding.is_none() && active_thread.available_surfaces.len() == 1,
            "workbench shell scene must use the real no-project capability projection"
        );
    }

    match scene_name {
        "omega_workbench_shell_default" => {}
        name if is_workbench_files_scene(name) => {
            let fixture = files_fixture.context("Files scene has no disk fixture")?;
            let active_path = fixture
                .worktrees
                .iter()
                .find_map(|(id, path)| {
                    (id == &fixture.active_worktree_id).then_some(path.as_path())
                })
                .context("Files scene has no active disk worktree")?;
            let active_worktree_id = workspace_window
                .update(cx, |workspace, _window, cx| {
                    workspace
                        .project()
                        .read(cx)
                        .visible_worktrees(cx)
                        .find(|worktree| worktree.read(cx).abs_path().as_ref() == active_path)
                        .map(|worktree| worktree.read(cx).id())
                })
                .context("reading Files scene worktrees")?
                .with_context(|| {
                    format!(
                        "active Files worktree {} is not visible",
                        active_path.display()
                    )
                })?;
            let mut stale_alpha_derivation = None;
            if name == "omega_workbench_files_stale_filesystem_completion" {
                let (_, first_path) = fixture
                    .worktrees
                    .first()
                    .context("stale Files scene has no first worktree")?;
                let alpha_binding =
                    select_workbench_identity(workspace_window, panel, first_path, "Files", cx)?;
                dispatch_workbench_action(workspace_window, Box::new(SelectFiles), cx)?;
                cx.run_until_parked();
                let first_worktree_id = workspace_window
                    .update(cx, |workspace, _window, cx| {
                        workspace
                            .project()
                            .read(cx)
                            .visible_worktrees(cx)
                            .find(|worktree| {
                                worktree.read(cx).abs_path().as_ref() == first_path.as_path()
                            })
                            .map(|worktree| worktree.read(cx).id())
                    })
                    .context("reading initial stale-scene worktree")?
                    .context("initial stale-scene worktree is unavailable")?;
                anyhow::ensure!(
                    cx.read(|cx| project_panel.read(cx).worktree_scope())
                        == Some(first_worktree_id),
                    "stale Files scene never projected its first worktree"
                );
                let alpha_projection = cx
                    .read(|cx| panel.read(cx).workbench_projection_for_tests().clone())
                    .visible_projection()
                    .context("stale Files scene has no alpha projection")?;
                anyhow::ensure!(
                    alpha_projection.binding.as_ref() == Some(&alpha_binding),
                    "stale Files scene did not establish the distinct alpha binding"
                );
                let alpha_row_selectors = cx
                    .read(|cx| project_panel.read(cx).visible_rows_for_test())
                    .into_iter()
                    .map(|row| row.selector())
                    .collect::<Vec<_>>();
                anyhow::ensure!(
                    !alpha_row_selectors.is_empty(),
                    "stale Files scene has no alpha semantic rows to reject"
                );
                let alpha_derivation_revision =
                    cx.update_window(workspace_window.into(), |_, window, cx| {
                        project_panel.update(cx, |project_panel, cx| -> Result<u64> {
                            anyhow::ensure!(
                                project_panel.set_worktree_scope(None, window, cx),
                                "could not clear alpha scope before stale derivation"
                            );
                            anyhow::ensure!(
                                project_panel.set_worktree_scope(
                                    Some(first_worktree_id),
                                    window,
                                    cx,
                                ),
                                "could not start the pending alpha derivation"
                            );
                            match project_panel.scope_state() {
                                project_panel::ProjectPanelScopeState::Loading {
                                    worktree_id,
                                    revision,
                                } if worktree_id == first_worktree_id => Ok(revision),
                                state => anyhow::bail!(
                                    "alpha derivation was not pending after scheduling: {state:?}"
                                ),
                            }
                        })
                    })??;
                anyhow::ensure!(
                    cx.background_executor.tick(),
                    "pending alpha derivation did not schedule executor work"
                );
                anyhow::ensure!(
                    matches!(
                        cx.read(|cx| project_panel.read(cx).scope_state()),
                        project_panel::ProjectPanelScopeState::Loading {
                            worktree_id,
                            revision,
                        } if worktree_id == first_worktree_id
                            && revision == alpha_derivation_revision
                    ),
                    "alpha derivation completed before the controlled beta rebind"
                );
                record_workbench_semantic_check(
                    name,
                    "files-alpha-derivation-pending-before-rebind",
                );
                let beta_binding =
                    select_workbench_identity(workspace_window, panel, active_path, "Files", cx)?;
                stale_alpha_derivation = Some((
                    alpha_binding,
                    beta_binding,
                    alpha_projection.generation,
                    alpha_derivation_revision,
                    alpha_row_selectors,
                ));
                dispatch_workbench_action(workspace_window, Box::new(SelectFiles), cx)?;
                dispatch_workbench_action(workspace_window, Box::new(SelectFiles), cx)?;
            } else {
                select_workbench_identity(workspace_window, panel, active_path, "Files", cx)?;
                dispatch_workbench_action(workspace_window, Box::new(SelectFiles), cx)?;
            }

            let expects_empty = name == "omega_workbench_files_empty";
            for _ in 0..64 {
                cx.run_until_parked();
                let state = cx.read(|cx| project_panel.read(cx).scope_state());
                let ready = matches!(
                    state,
                    project_panel::ProjectPanelScopeState::Ready { worktree_id, .. }
                        if !expects_empty && worktree_id == active_worktree_id
                ) || matches!(
                    state,
                    project_panel::ProjectPanelScopeState::Empty { worktree_id, .. }
                        if expects_empty && worktree_id == active_worktree_id
                );
                if ready {
                    break;
                }
                cx.advance_clock(Duration::from_millis(10));
            }

            let scope_state = cx.read(|cx| project_panel.read(cx).scope_state());
            let scope_matches = matches!(
                scope_state,
                project_panel::ProjectPanelScopeState::Ready { worktree_id, .. }
                    if !expects_empty && worktree_id == active_worktree_id
            ) || matches!(
                scope_state,
                project_panel::ProjectPanelScopeState::Empty { worktree_id, .. }
                    if expects_empty && worktree_id == active_worktree_id
            );
            anyhow::ensure!(
                scope_matches,
                "Files scene {name:?} reached {scope_state:?}, expected active scope {active_worktree_id:?}"
            );
            record_workbench_semantic_check(name, "files-active-worktree-scope");
            if let Some((
                alpha_binding,
                beta_binding,
                alpha_generation,
                alpha_derivation_revision,
                _,
            )) = stale_alpha_derivation.as_ref()
            {
                let beta_projection = cx
                    .read(|cx| panel.read(cx).workbench_projection_for_tests().clone())
                    .visible_projection()
                    .context("stale Files scene has no beta projection")?;
                let beta_scope_revision = match scope_state {
                    project_panel::ProjectPanelScopeState::Ready { revision, .. } => revision,
                    state => anyhow::bail!(
                        "stale Files scene did not settle on a ready beta scope: {state:?}"
                    ),
                };
                anyhow::ensure!(
                    beta_projection.binding.as_ref() == Some(beta_binding)
                        && beta_projection.binding.as_ref() != Some(alpha_binding)
                        && beta_projection.generation > *alpha_generation,
                    "stale Files scene did not advance from the alpha binding epoch to beta"
                );
                anyhow::ensure!(
                    beta_scope_revision > *alpha_derivation_revision,
                    "beta scope revision {beta_scope_revision} did not supersede pending alpha revision {alpha_derivation_revision}"
                );
                record_workbench_semantic_check(name, "files-distinct-alpha-beta-binding-epochs");
                record_workbench_semantic_check(
                    name,
                    "files-beta-scope-supersedes-alpha-derivation",
                );
            }

            let rows = cx.read(|cx| project_panel.read(cx).visible_rows_for_test());
            anyhow::ensure!(
                rows.iter().all(|row| row.worktree_id == active_worktree_id),
                "Files scene {name:?} leaked rows from an inactive worktree"
            );
            if expects_empty {
                anyhow::ensure!(
                    rows.iter()
                        .all(|row| row.path.as_ref().as_unix_str().is_empty()),
                    "empty Files scene rendered a non-root row"
                );
            } else {
                anyhow::ensure!(
                    rows.iter()
                        .any(|row| !row.path.as_ref().as_unix_str().is_empty()),
                    "ready Files scene rendered no file rows"
                );
            }
            if matches!(
                name,
                "omega_workbench_files_multi_root"
                    | "omega_workbench_files_stale_filesystem_completion"
            ) {
                anyhow::ensure!(
                    rows.iter()
                        .any(|row| row.path.as_ref() == util::rel_path::rel_path("beta-only.txt"))
                        && rows.iter().all(|row| {
                            row.path.as_ref() != util::rel_path::rel_path("alpha-only.txt")
                        }),
                    "multi-root Files scene did not isolate the active beta worktree"
                );
            }
            if name == "omega_workbench_files_stale_filesystem_completion" {
                let (_, _, _, _, alpha_row_selectors) = stale_alpha_derivation
                    .as_ref()
                    .context("stale Files scene lost its alpha derivation evidence")?;
                let snapshot = cx.debug_render_snapshot(workspace_window.into())?;
                let mut probe = SemanticProbe::new(&snapshot);
                for selector in alpha_row_selectors {
                    probe.require_absent(selector)?;
                }
                record_workbench_semantic_checks(name, probe.into_checks());
                record_workbench_semantic_check(
                    name,
                    "files-stale-alpha-derivation-rejected-after-beta-rebind",
                );
            }
            record_workbench_semantic_check(name, "files-visible-rows-match-active-scope");

            if let Some(selected_path) = fixture.selected_path {
                let selected_project_path = project::ProjectPath {
                    worktree_id: active_worktree_id,
                    path: util::rel_path::rel_path(selected_path).into(),
                };
                cx.update(|cx| {
                    project_panel.update(cx, |project_panel, cx| {
                        project_panel.select_path_for_test(selected_project_path.clone(), cx);
                        cx.notify();
                    });
                });
                cx.run_until_parked();
                let selected = cx.read(|cx| project_panel.read(cx).selected_entry_project_path(cx));
                anyhow::ensure!(
                    selected.as_ref() == Some(&selected_project_path),
                    "Files scene {name:?} did not select {selected_path:?}"
                );
                record_workbench_semantic_check(name, "files-selected-row-state");
            }

            if matches!(
                name,
                "omega_workbench_files_loading" | "omega_workbench_files_error"
            ) {
                let load = cx.update_window(workspace_window.into(), |_, window, cx| {
                    panel.update(cx, |panel, cx| {
                        panel.begin_workbench_surface_load_for_tests(
                            format!("{name}-request"),
                            omega_workbench_state::WorkSurface::Files,
                            window,
                            cx,
                        )
                    })
                })??;
                if name == "omega_workbench_files_error" {
                    let effect =
                        cx.update_window(workspace_window.into(), |_, window, cx| {
                            panel.update(cx, |panel, cx| {
                                panel.complete_workbench_surface_load_for_tests(
                                    load,
                                    agent_ui::workbench_shell::SurfaceLoadOutcome::Error(
                                        "Could not load Files".into(),
                                    ),
                                    window,
                                    cx,
                                )
                            })
                        })??;
                    anyhow::ensure!(
                        effect == omega_workbench_state::TransitionEffect::Applied,
                        "Files error scene load completion was not applied"
                    );
                    record_workbench_semantic_check(name, "files-error-state-applied");
                } else {
                    record_workbench_semantic_check(name, "files-loading-state-applied");
                }
                cx.run_until_parked();
            }

            let projection = cx.read(|cx| panel.read(cx).workbench_projection_for_tests().clone());
            let visible = projection
                .visible_projection()
                .context("Files scene has no visible projection")?;
            anyhow::ensure!(
                visible.requested_surface == Some(omega_workbench_state::WorkSurface::Files)
                    && visible.effective_surface == Some(omega_workbench_state::WorkSurface::Files)
                    && visible.dock_open,
                "Files scene did not reach its expected production Files layout"
            );
            record_workbench_semantic_check(name, "files-production-surface-open");
        }
        name if is_workbench_search_scene(name) => {
            let fixture = search_fixture.context("Search scene has no disk fixture")?;
            let active_path = fixture
                .worktrees
                .iter()
                .find_map(|(id, path)| {
                    (id == &fixture.active_worktree_id).then_some(path.as_path())
                })
                .context("Search scene has no active disk worktree")?;
            let active_worktree_id = workspace_window
                .update(cx, |workspace, _window, cx| {
                    workspace
                        .project()
                        .read(cx)
                        .visible_worktrees(cx)
                        .find(|worktree| worktree.read(cx).abs_path().as_ref() == active_path)
                        .map(|worktree| worktree.read(cx).id())
                })
                .context("reading Search scene worktrees")?
                .with_context(|| {
                    format!(
                        "active Search worktree {} is not visible",
                        active_path.display()
                    )
                })?;

            let mut stale_alpha = None;
            if name == "omega_workbench_search_populated" {
                let (_, alpha_path) = fixture
                    .worktrees
                    .first()
                    .context("populated Search scene has no alpha worktree")?;
                let alpha_binding =
                    select_workbench_identity(workspace_window, panel, alpha_path, "Search", cx)?;
                ensure_workbench_search_open(workspace_window, panel, cx)?;
                let (alpha_surface, alpha_search_view) = active_workbench_search(panel, cx)?;
                let alpha_snapshot = workbench_search_snapshot(&alpha_search_view, cx);
                let alpha_projection = cx
                    .read(|cx| panel.read(cx).workbench_projection_for_tests().clone())
                    .visible_projection()
                    .context("populated Search scene has no alpha projection")?;
                anyhow::ensure!(
                    alpha_snapshot.worktree_scope.is_some()
                        && alpha_projection.binding.as_ref() == Some(&alpha_binding),
                    "populated Search scene did not scope its initial alpha search"
                );
                let pending_sender = start_pending_workbench_search(
                    workspace_window,
                    &alpha_search_view,
                    "OLD_BINDING_ONLY",
                    cx,
                )?;
                let pending_snapshot = workbench_search_snapshot(&alpha_search_view, cx);
                record_workbench_semantic_check(name, "search-alpha-request-pending-before-rebind");
                stale_alpha = Some((
                    alpha_surface,
                    alpha_search_view,
                    alpha_binding,
                    alpha_projection.generation,
                    pending_snapshot.generation,
                    pending_sender,
                ));
            }

            let beta_binding =
                select_workbench_identity(workspace_window, panel, active_path, "Search", cx)?;
            ensure_workbench_search_open(workspace_window, panel, cx)?;
            let (search_surface, search_view) = active_workbench_search(panel, cx)?;
            let initial_snapshot = workbench_search_snapshot(&search_view, cx);
            anyhow::ensure!(
                initial_snapshot.worktree_scope == Some(active_worktree_id),
                "Search scene {name:?} has scope {:?}, expected {active_worktree_id:?}",
                initial_snapshot.worktree_scope
            );
            record_workbench_semantic_check(name, "search-active-worktree-scope");
            cx.update_window(workspace_window.into(), |_, window, cx| {
                search_surface.update(cx, |search_surface, cx| {
                    gpui::Focusable::focus_handle(search_surface, cx).focus(window, cx);
                });
            })?;
            dispatch_workbench_action(workspace_window, Box::new(search::FocusSearch), cx)?;
            let query_focus = cx.update_window(workspace_window.into(), |_, window, cx| {
                search_surface.update(cx, |surface, cx| surface.focus_target(window, cx))
            })?;
            anyhow::ensure!(
                query_focus == Some(agent_ui::workbench_shell::NativeSearchFocusTarget::Query),
                "Search scene {name:?} could not route focus into the native query"
            );
            record_workbench_semantic_check(name, "search-native-query-focus-action");

            let projection = cx.read(|cx| panel.read(cx).workbench_projection_for_tests().clone());
            let visible = projection
                .visible_projection()
                .context("Search scene has no visible projection")?;
            anyhow::ensure!(
                visible.binding.as_ref() == Some(&beta_binding)
                    && visible.requested_surface
                        == Some(omega_workbench_state::WorkSurface::Search)
                    && visible.effective_surface
                        == Some(omega_workbench_state::WorkSurface::Search)
                    && visible.dock_open,
                "Search scene did not reach its expected production Search layout"
            );
            record_workbench_semantic_check(name, "search-production-surface-open");

            if let Some((
                alpha_surface,
                alpha_search_view,
                alpha_binding,
                alpha_projection_generation,
                alpha_search_generation,
                alpha_sender,
            )) = stale_alpha
            {
                anyhow::ensure!(
                    alpha_binding != beta_binding
                        && alpha_surface.entity_id() != search_surface.entity_id()
                        && visible.generation > alpha_projection_generation,
                    "Search rebind did not establish a distinct beta host epoch"
                );
                alpha_sender
                    .try_send(project::search::SearchResult::Searching)
                    .context("releasing stale alpha Search activity")?;
                drop(alpha_sender);
                let alpha_completed = wait_for_workbench_search(&alpha_search_view, cx)?;
                anyhow::ensure!(
                    alpha_completed.generation == alpha_search_generation
                        && matches!(
                            alpha_completed.lifecycle,
                            search::project_search::ProjectSearchLifecycle::Completed {
                                request,
                                completion:
                                    search::project_search::ProjectSearchCompletion::NoResults,
                            } if request.worktree_id == alpha_completed.worktree_scope
                        ),
                    "controlled alpha Search did not complete in its original host: {alpha_completed:?}"
                );
                let active_after_alpha = active_workbench_search(panel, cx)?.0;
                anyhow::ensure!(
                    active_after_alpha.entity_id() == search_surface.entity_id(),
                    "late alpha Search completion replaced the active beta Search host"
                );
                record_workbench_semantic_check(
                    name,
                    "search-late-alpha-completion-isolated-from-beta-host",
                );
            }

            match name {
                "omega_workbench_search_empty" => {
                    anyhow::ensure!(
                        initial_snapshot.query.is_empty()
                            && initial_snapshot.matches.is_empty()
                            && initial_snapshot.active_match.is_none()
                            && matches!(
                                initial_snapshot.lifecycle,
                                search::project_search::ProjectSearchLifecycle::Idle
                            ),
                        "empty Search scene did not retain native idle state: {initial_snapshot:?}"
                    );
                    record_workbench_semantic_check(name, "search-empty-idle-state");
                }
                "omega_workbench_search_invalid_regex" => {
                    dispatch_workbench_action(workspace_window, Box::new(search::FocusSearch), cx)?;
                    dispatch_workbench_action(workspace_window, Box::new(search::ToggleRegex), cx)?;
                    let snapshot =
                        perform_workbench_search(workspace_window, &search_view, "(", cx)?;
                    anyhow::ensure!(
                        snapshot
                            .search_options
                            .contains(search::SearchOptions::REGEX)
                            && snapshot.query == "("
                            && snapshot
                                .query_error
                                .as_ref()
                                .is_some_and(|error| !error.is_empty())
                            && snapshot.matches.is_empty()
                            && snapshot.worktree_scope == Some(active_worktree_id),
                        "invalid-regex Search scene did not expose native query state: {snapshot:?}"
                    );
                    record_workbench_semantic_check(name, "search-invalid-regex-native-error");
                }
                "omega_workbench_search_loading" | "omega_workbench_search_error" => {
                    let load = cx.update_window(workspace_window.into(), |_, window, cx| {
                        panel.update(cx, |panel, cx| {
                            panel.begin_workbench_surface_load_for_tests(
                                format!("{name}-request"),
                                omega_workbench_state::WorkSurface::Search,
                                window,
                                cx,
                            )
                        })
                    })??;
                    if name == "omega_workbench_search_error" {
                        let effect =
                            cx.update_window(workspace_window.into(), |_, window, cx| {
                                panel.update(cx, |panel, cx| {
                                    panel.complete_workbench_surface_load_for_tests(
                                        load,
                                        agent_ui::workbench_shell::SurfaceLoadOutcome::Error(
                                            "Could not search this worktree".into(),
                                        ),
                                        window,
                                        cx,
                                    )
                                })
                            })??;
                        anyhow::ensure!(
                            effect == omega_workbench_state::TransitionEffect::Applied,
                            "Search error scene load completion was not applied"
                        );
                        record_workbench_semantic_check(name, "search-error-state-applied");
                    } else {
                        record_workbench_semantic_check(name, "search-loading-state-applied");
                    }
                    cx.run_until_parked();
                }
                "omega_workbench_search_no_results" => {
                    let snapshot = perform_workbench_search(
                        workspace_window,
                        &search_view,
                        "omega_no_fixture_match",
                        cx,
                    )?;
                    anyhow::ensure!(
                        snapshot.query == "omega_no_fixture_match"
                            && snapshot.matches.is_empty()
                            && matches!(
                                snapshot.lifecycle,
                                search::project_search::ProjectSearchLifecycle::Completed {
                                    request,
                                    completion:
                                        search::project_search::ProjectSearchCompletion::NoResults,
                                } if request.worktree_id == Some(active_worktree_id)
                            ),
                        "no-results Search scene did not settle deterministically: {snapshot:?}"
                    );
                    record_workbench_semantic_check(name, "search-no-results-completion");
                }
                "omega_workbench_search_populated"
                | "omega_workbench_search_narrow"
                | "omega_workbench_search_focused_result" => {
                    dispatch_workbench_action(workspace_window, Box::new(search::FocusSearch), cx)?;
                    let mut snapshot = perform_workbench_search(
                        workspace_window,
                        &search_view,
                        "omega_search_hit",
                        cx,
                    )?;
                    let generation_after_initial_search = snapshot.generation;
                    anyhow::ensure!(
                        generation_after_initial_search > initial_snapshot.generation
                            && snapshot.query == "omega_search_hit"
                            && snapshot.active_query.as_deref() == Some("omega_search_hit")
                            && snapshot.query_error.is_none()
                            && snapshot.matches.len() == 4
                            && matches!(
                                snapshot.lifecycle,
                                search::project_search::ProjectSearchLifecycle::Completed {
                                    request,
                                    completion:
                                        search::project_search::ProjectSearchCompletion::Results {
                                            match_count: 4,
                                            limit_reached: false,
                                        },
                                } if request.generation == snapshot.generation
                                    && request.worktree_id == Some(active_worktree_id)
                            ),
                        "populated Search scene did not settle with four beta matches: {snapshot:?}"
                    );
                    let result_paths = snapshot
                        .matches
                        .iter()
                        .map(|search_match| {
                            (
                                search_match.path.worktree_id,
                                search_match.path.path.as_ref().as_unix_str().to_owned(),
                            )
                        })
                        .collect::<Vec<_>>();
                    anyhow::ensure!(
                        result_paths
                            .iter()
                            .all(|(worktree_id, _)| { *worktree_id == active_worktree_id })
                            && result_paths
                                == vec![
                                    (active_worktree_id, "README.md".into()),
                                    (active_worktree_id, "src/first.rs".into()),
                                    (active_worktree_id, "src/first.rs".into()),
                                    (active_worktree_id, "src/second.rs".into()),
                                ]
                            && snapshot
                                .matches
                                .iter()
                                .all(|search_match| search_match.range.start
                                    != search_match.range.end),
                        "Search results were not the deterministic beta-only ordered ranges: {result_paths:?}"
                    );
                    record_workbench_semantic_check(
                        name,
                        "search-result-count-order-ranges-and-scope",
                    );

                    if name == "omega_workbench_search_populated" {
                        dispatch_workbench_action(
                            workspace_window,
                            Box::new(search::ToggleReplace),
                            cx,
                        )?;
                        cx.update_window(workspace_window.into(), |_, window, cx| {
                            search_view.update(cx, |search_view, cx| {
                                search_view.test_set_filters(
                                    "**/src/**/*.rs",
                                    "",
                                    true,
                                    window,
                                    cx,
                                );
                                search_view.test_set_replacement(
                                    "omega_search_replacement",
                                    window,
                                    cx,
                                );
                                search_view.test_start_search(cx);
                            });
                        })?;
                        snapshot = wait_for_workbench_search(&search_view, cx)?;
                        anyhow::ensure!(
                            snapshot.replace_enabled
                                && snapshot.filters_enabled
                                && snapshot.included_files == "**/src/**/*.rs"
                                && snapshot.excluded_files.is_empty()
                                && snapshot.replacement == "omega_search_replacement"
                                && snapshot.matches.len() == 3,
                            "Search filters or replacement were not represented in native state: {snapshot:?}"
                        );
                        record_workbench_semantic_check(
                            name,
                            "search-filter-and-replacement-state",
                        );

                        cx.update_window(workspace_window.into(), |_, window, cx| {
                            search_view.update(cx, |search_view, cx| {
                                search_view.test_set_filters("", "", false, window, cx);
                                search_view.test_start_search(cx);
                            });
                        })?;
                        wait_for_workbench_search(&search_view, cx)?;
                        dispatch_workbench_action(
                            workspace_window,
                            Box::new(search::ToggleIncludeIgnored),
                            cx,
                        )?;
                        snapshot = wait_for_workbench_search(&search_view, cx)?;
                        anyhow::ensure!(
                            snapshot
                                .search_options
                                .contains(search::SearchOptions::INCLUDE_IGNORED)
                                && snapshot.matches.len() == 5,
                            "include-ignored action did not expose the ignored fifth match: {snapshot:?}"
                        );
                        dispatch_workbench_action(
                            workspace_window,
                            Box::new(search::ToggleIncludeIgnored),
                            cx,
                        )?;
                        snapshot = wait_for_workbench_search(&search_view, cx)?;
                        anyhow::ensure!(
                            !snapshot
                                .search_options
                                .contains(search::SearchOptions::INCLUDE_IGNORED)
                                && snapshot.matches.len() == 4,
                            "include-ignored action did not restore the default result set: {snapshot:?}"
                        );
                        record_workbench_semantic_check(
                            name,
                            "search-option-action-reruns-native-query",
                        );

                        let pending_sender = start_pending_workbench_search(
                            workspace_window,
                            &search_view,
                            "omega_cancelled_query",
                            cx,
                        )?;
                        let pending_generation =
                            workbench_search_snapshot(&search_view, cx).generation;
                        cx.update(|cx| {
                            search_view.update(cx, |search_view, cx| {
                                anyhow::ensure!(search_view.cancel_search(cx));
                                Ok::<_, anyhow::Error>(())
                            })
                        })?;
                        drop(pending_sender);
                        cx.run_until_parked();
                        let cancelled = workbench_search_snapshot(&search_view, cx);
                        anyhow::ensure!(
                            !cancelled.pending
                                && cancelled.generation > pending_generation
                                && matches!(
                                    cancelled.lifecycle,
                                    search::project_search::ProjectSearchLifecycle::Cancelled {
                                        request,
                                    } if request.generation == cancelled.generation
                                        && request.worktree_id == Some(active_worktree_id)
                                ),
                            "cancel action did not advance to a typed cancelled request: {cancelled:?}"
                        );
                        snapshot = perform_workbench_search(
                            workspace_window,
                            &search_view,
                            "omega_search_hit",
                            cx,
                        )?;
                        anyhow::ensure!(
                            snapshot.matches.len() == 4,
                            "Search did not recover after cancellation: {snapshot:?}"
                        );
                        record_workbench_semantic_check(
                            name,
                            "search-cancel-generation-and-recovery",
                        );
                    }

                    if name == "omega_workbench_search_focused_result" {
                        let search_bar = cx.read(|cx| search_surface.read(cx).search_bar().clone());
                        cx.update_window(workspace_window.into(), |_, window, cx| {
                            search_bar.update(cx, |search_bar, cx| {
                                search_bar.move_focus_to_results(window, cx);
                            });
                        })?;
                        dispatch_workbench_action(
                            workspace_window,
                            Box::new(search::SelectNextMatch),
                            cx,
                        )?;
                        snapshot = workbench_search_snapshot(&search_view, cx);
                        let active_index = snapshot
                            .active_match_index
                            .context("focused Search result has no active index")?;
                        let selected_match = snapshot
                            .active_match
                            .clone()
                            .context("focused Search result has no selected range")?;
                        anyhow::ensure!(
                            active_index < snapshot.matches.len()
                                && Some(selected_match.clone())
                                    == snapshot.matches.get(active_index).cloned(),
                            "focused Search result does not identify its selected range: {snapshot:?}"
                        );
                        let focus_target =
                            cx.update_window(workspace_window.into(), |_, window, cx| {
                                search_surface
                                    .update(cx, |surface, cx| surface.focus_target(window, cx))
                            })?;
                        anyhow::ensure!(
                            focus_target
                                == Some(
                                    agent_ui::workbench_shell::NativeSearchFocusTarget::Results
                                ),
                            "focused Search scene did not move focus to native results"
                        );
                        dispatch_workbench_action(
                            workspace_window,
                            Box::new(editor::actions::OpenExcerpts),
                            cx,
                        )?;
                        let (center_visible, opened_path, opened_editor) = workspace_window
                            .update(cx, |workspace, _window, cx| {
                                (
                                    workspace.center_visible_for_tests(),
                                    workspace
                                        .active_item(cx)
                                        .and_then(|item| item.project_path(cx)),
                                    workspace.active_item_as::<editor::Editor>(cx).is_some(),
                                )
                            })
                            .context("reading Search open-result workspace state")?;
                        anyhow::ensure!(
                            center_visible
                                && opened_editor
                                && opened_path.as_ref() == Some(&selected_match.path),
                            "opening a Search result did not reveal its exact path in the native center: opened={opened_path:?}, selected={:?}",
                            selected_match.path
                        );
                        record_workbench_semantic_check(
                            name,
                            "search-focused-result-range-and-open-navigation",
                        );
                    }
                }
                _ => unreachable!("Search scene was checked above"),
            }
        }
        name if is_workbench_review_scene(name) => {
            use agent_ui::workbench_shell::SelectReview;
            use omega_workbench_harness::{ReviewMutationFixture, ReviewMutationKindFixture};

            let fixture = review_fixture.context("Review scene has no disk fixture")?;
            let active_path = fixture
                .worktrees
                .iter()
                .find_map(|(id, path)| {
                    (id == &fixture.active_worktree_id).then_some(path.as_path())
                })
                .context("Review scene has no active disk worktree")?;
            let foreign_path = fixture
                .worktrees
                .iter()
                .find_map(|(id, path)| {
                    (id != &fixture.active_worktree_id).then_some(path.as_path())
                })
                .context("Review scene has no foreign disk worktree")?;
            let (active_worktree_id, foreign_worktree_id) = workspace_window
                .update(cx, |workspace, _window, cx| {
                    let project = workspace.project().read(cx);
                    let active = project
                        .visible_worktrees(cx)
                        .find(|worktree| worktree.read(cx).abs_path().as_ref() == active_path)
                        .map(|worktree| worktree.read(cx).id());
                    let foreign = project
                        .visible_worktrees(cx)
                        .find(|worktree| worktree.read(cx).abs_path().as_ref() == foreign_path)
                        .map(|worktree| worktree.read(cx).id());
                    (active, foreign)
                })
                .context("reading Review scene worktrees")?;
            let active_worktree_id = active_worktree_id.with_context(|| {
                format!(
                    "active Review worktree {} is not visible",
                    active_path.display()
                )
            })?;
            let foreign_worktree_id = foreign_worktree_id.with_context(|| {
                format!(
                    "foreign Review worktree {} is not visible",
                    foreign_path.display()
                )
            })?;
            let repository_binding =
                select_workbench_identity(workspace_window, panel, active_path, "Review", cx)?;
            dispatch_workbench_action(workspace_window, Box::new(SelectReview), cx)?;
            let (review_surface, review_pane) = active_workbench_review(panel, cx)?;
            let initial_snapshot = cx.update_window(workspace_window.into(), |_, window, cx| {
                review_pane.update(cx, |pane, cx| pane.snapshot_for_tests(window, cx))
            })?;
            let production_projection =
                cx.read(|cx| panel.read(cx).workbench_projection_for_tests().clone());
            let visible = production_projection
                .visible_projection()
                .context("Review scene has no visible projection")?;
            let active_thread = cx
                .read(|cx| panel.read(cx).omega_active_acp_thread(cx))
                .context("Review scene has no active ACP thread")?;
            let active_action_log_entity_id =
                cx.read(|cx| active_thread.read(cx).action_log().entity_id());
            let native_binding = initial_snapshot
                .binding
                .as_ref()
                .context("native Review pane has no typed binding")?;
            anyhow::ensure!(
                visible.binding.as_ref() == Some(&repository_binding)
                    && visible.requested_surface
                        == Some(omega_workbench_state::WorkSurface::Review)
                    && visible.effective_surface
                        == Some(omega_workbench_state::WorkSurface::Review)
                    && visible.dock_open
                    && native_binding.thread_id.to_key_string() == visible.thread_id
                    && native_binding.repository == repository_binding
                    && native_binding.worktree_id == active_worktree_id
                    && native_binding.checkpoint.generation() == visible.generation
                    && native_binding.checkpoint.action_log_entity_id()
                        == active_action_log_entity_id
                    && initial_snapshot.thread_entity_id == active_thread.entity_id()
                    && initial_snapshot.action_log_entity_id == active_action_log_entity_id,
                "Review scene did not bind the native pane to its active thread/worktree/checkpoint"
            );
            record_workbench_semantic_check(name, "review-production-binding-checkpoint-identity");

            let (foreign_buffer, foreign_contents_before_active_mutations) =
                seed_foreign_workbench_review(
                    workspace_window,
                    &review_pane,
                    foreign_worktree_id,
                    cx,
                )?;
            seed_workbench_review_scene(
                name,
                workspace_window,
                &review_pane,
                active_worktree_id,
                cx,
            )?;
            let expected_active_file_count = workbench_fixture_for_scene(name)?
                .active_review_session()
                .context("Review scene has no expected active session")?
                .files
                .len();
            let action_log = cx.read(|cx| active_thread.read(cx).action_log().clone());
            wait_for_workbench_review_changes(
                action_log.clone(),
                active_worktree_id,
                expected_active_file_count,
                cx,
            )?;

            let generation = native_binding.checkpoint.generation();
            match name {
                "omega_workbench_review_streaming_update" => {
                    // The streaming edit's diff recomputes asynchronously;
                    // settle at all four expected hunks before proving so the
                    // count assertion reads a finished diff, not a race.
                    wait_for_workbench_review_hunks(action_log, active_worktree_id, 4, cx)?;
                    let stale_generation = generation.saturating_add(1);
                    let (stale_rejected, streaming_applied) = cx.update(|cx| {
                        review_pane.update(cx, |pane, cx| {
                            (
                                !pane.set_streaming_for_tests(stale_generation, true, cx),
                                pane.set_streaming_for_tests(generation, true, cx),
                            )
                        })
                    });
                    anyhow::ensure!(
                        stale_rejected && streaming_applied,
                        "Review streaming scene did not reject a stale generation before applying the active one"
                    );
                    record_workbench_semantic_check(
                        name,
                        "review-stale-generation-rejected-before-streaming-update",
                    );
                    cx.update_window(workspace_window.into(), |_, window, cx| {
                        review_pane.update(cx, |pane, cx| {
                            gpui::Focusable::focus_handle(pane, cx).focus(window, cx);
                        });
                    })?;
                    dispatch_workbench_action(
                        workspace_window,
                        Box::new(editor::actions::GoToHunk),
                        cx,
                    )?;
                }
                "omega_workbench_review_selected_hunk" => {
                    cx.update_window(workspace_window.into(), |_, window, cx| {
                        review_pane.update(cx, |pane, cx| {
                            gpui::Focusable::focus_handle(pane, cx).focus(window, cx);
                        });
                    })?;
                    dispatch_workbench_action(
                        workspace_window,
                        Box::new(editor::actions::GoToHunk),
                        cx,
                    )?;
                }
                "omega_workbench_review_all_reviewed" => {
                    // The composer executor dropdown landing focuses the
                    // composer, so an unfocused Keep/Reject would land in the
                    // message editor instead of the review pane. Focus the
                    // pane first, exactly like the streaming and
                    // selected-hunk branches.
                    cx.update_window(workspace_window.into(), |_, window, cx| {
                        review_pane.update(cx, |pane, cx| {
                            gpui::Focusable::focus_handle(pane, cx).focus(window, cx);
                        });
                    })?;
                    dispatch_workbench_action(workspace_window, Box::new(agent_ui::Keep), cx)?;
                    wait_for_workbench_review_hunks(action_log.clone(), active_worktree_id, 2, cx)?;
                    dispatch_workbench_action(workspace_window, Box::new(agent_ui::Reject), cx)?;
                    wait_for_workbench_review_hunks(action_log.clone(), active_worktree_id, 1, cx)?;
                    dispatch_workbench_action(workspace_window, Box::new(agent_ui::Keep), cx)?;
                    wait_for_workbench_review_hunks(action_log, active_worktree_id, 0, cx)?;
                    let clean_observation = cx.read(|cx| {
                        let identity = panel
                            .read(cx)
                            .workbench_identity_for_tests()
                            .context("all-reviewed scene has no repository identity")?;
                        let mut candidates = identity.candidates.clone();
                        for candidate in &mut candidates {
                            candidate.git =
                                agent_ui::thread_identity::GitIdentitySummary::default();
                        }
                        Ok::<_, anyhow::Error>(
                            agent_ui::thread_identity::ThreadIdentityObservation {
                                revision: identity.observation_revision.saturating_add(1),
                                phase: agent_ui::thread_identity::IdentityPhase::Ready,
                                candidates,
                            },
                        )
                    })?;
                    cx.update_window(workspace_window.into(), |_, window, cx| {
                        panel.update(cx, |panel, cx| {
                            panel.set_workbench_identity_observation_for_tests(
                                Some(clean_observation),
                                window,
                                cx,
                            );
                        });
                    })?;
                    cx.run_until_parked();
                }
                "omega_workbench_review_error" => {
                    let applied = cx.update(|cx| {
                        review_pane.update(cx, |pane, cx| {
                            pane.set_error(generation, "Could not load this checkpoint", cx)
                        })
                    });
                    anyhow::ensure!(applied, "Review error lifecycle was not applied");
                }
                _ => {}
            }
            cx.run_until_parked();

            if name == "omega_workbench_review_multi_file" {
                let pane_id = review_pane.entity_id();
                let surface_id = review_surface.entity_id();
                let before = cx.update_window(workspace_window.into(), |_, window, cx| {
                    review_pane.update(cx, |pane, cx| pane.snapshot_for_tests(window, cx))
                })?;
                dispatch_workbench_action(workspace_window, Box::new(SelectPlan), cx)?;
                dispatch_workbench_action(workspace_window, Box::new(SelectReview), cx)?;
                let (reopened_surface, reopened_pane) = active_workbench_review(panel, cx)?;
                let after = cx.update_window(workspace_window.into(), |_, window, cx| {
                    reopened_pane.update(cx, |pane, cx| pane.snapshot_for_tests(window, cx))
                })?;
                anyhow::ensure!(
                    reopened_surface.entity_id() == surface_id
                        && reopened_pane.entity_id() == pane_id
                        && before.selected_path == after.selected_path
                        && before.selected_range == after.selected_range,
                    "Review collapse/surface round trip did not retain its native entity and selection"
                );
                record_workbench_semantic_check(
                    name,
                    "review-retained-entity-selection-across-surface-round-trip",
                );
            }

            cx.update_window(workspace_window.into(), |_, window, cx| {
                review_pane.update(cx, |pane, cx| {
                    gpui::Focusable::focus_handle(pane, cx).focus(window, cx);
                });
            })?;
            cx.run_until_parked();
            let snapshot = cx.update_window(workspace_window.into(), |_, window, cx| {
                review_pane.update(cx, |pane, cx| pane.snapshot_for_tests(window, cx))
            })?;
            let foreign_contents_after_active_mutations =
                cx.read(|cx| foreign_buffer.read(cx).text());
            anyhow::ensure!(
                snapshot
                    .files
                    .iter()
                    .all(|file| file.worktree_id == active_worktree_id)
                    && snapshot
                        .files
                        .iter()
                        .all(|file| file.path != "src/foreign_thread_only.rs")
                    && foreign_contents_after_active_mutations
                        == foreign_contents_before_active_mutations,
                "native Review leaked or mutated the recorded foreign-worktree diff"
            );
            record_workbench_semantic_check(
                name,
                "review-recorded-foreign-worktree-diff-isolation",
            );

            let mutations = if name == "omega_workbench_review_all_reviewed" {
                anyhow::ensure!(
                    snapshot.kept_hunks == 2
                        && snapshot.rejected_hunks == 1
                        && snapshot.files.is_empty(),
                    "all-reviewed scene did not apply two keeps and one rejection: {snapshot:?}"
                );
                let main = open_workbench_review_buffer(
                    workspace_window,
                    active_worktree_id,
                    "src/main.rs",
                    cx,
                )?;
                let settings = open_workbench_review_buffer(
                    workspace_window,
                    active_worktree_id,
                    "src/settings.rs",
                    cx,
                )?;
                let main_text = cx.read(|cx| main.read(cx).text());
                let kept_import = main_text
                    .lines()
                    .next()
                    .map(|line| format!("{line}\n"))
                    .context("reviewed main fixture has no first line")?;
                let rejected_mode = main_text
                    .lines()
                    .nth(20)
                    .map(|line| format!("{line}\n"))
                    .context("reviewed main fixture has no mode line")?;
                let settings_text = cx.read(|cx| settings.read(cx).text());
                vec![
                    ReviewMutationFixture {
                        kind: ReviewMutationKindFixture::KeepHunk,
                        file_path: Some("src/main.rs".into()),
                        hunk_id: Some("main-imports".into()),
                        resulting_contents: Some(kept_import),
                    },
                    ReviewMutationFixture {
                        kind: ReviewMutationKindFixture::RejectHunk,
                        file_path: Some("src/main.rs".into()),
                        hunk_id: Some("main-body".into()),
                        resulting_contents: Some(rejected_mode),
                    },
                    ReviewMutationFixture {
                        kind: ReviewMutationKindFixture::KeepHunk,
                        file_path: Some("src/settings.rs".into()),
                        hunk_id: Some("settings-new".into()),
                        resulting_contents: Some(settings_text),
                    },
                ]
            } else {
                Vec::new()
            };

            let fixture_scene = workbench_fixture_for_scene(name)?;
            let expected = fixture_scene
                .active_review_session()
                .context("Review scene has no active typed fixture")?;
            let normalized = normalized_workbench_review_snapshot(expected, &snapshot, mutations)?;
            let checks = omega_workbench_harness::prove_review_surface(&fixture_scene, &normalized)
                .with_context(|| format!("proving native Review scene {name:?}"))?;
            record_workbench_semantic_checks(name, checks);
            record_workbench_semantic_check(name, "review-native-snapshot-proved");
        }
        name if is_workbench_git_scene(name) => {
            use agent_ui::{
                thread_identity::{IdentityPhase, ThreadIdentityObservation},
                workbench_shell::{NativeGitLifecycle, SelectGit},
            };

            let fixture = git_fixture.context("Git scene has no disk fixture")?;
            let active_path = fixture
                .worktrees
                .iter()
                .find_map(|(id, path)| {
                    (id == &fixture.active_worktree_id).then_some(path.as_path())
                })
                .context("Git scene has no active disk worktree")?;
            let foreign_path = fixture
                .worktrees
                .iter()
                .find_map(|(id, path)| {
                    (id != &fixture.active_worktree_id).then_some(path.as_path())
                })
                .context("Git scene has no foreign disk worktree")?;
            let fixture_scene = workbench_fixture_for_scene(name)?;
            let expected = fixture_scene
                .active_git_snapshot()
                .context("Git scene has no active typed fixture")?;
            let repository_binding =
                select_workbench_identity(workspace_window, panel, active_path, "Git", cx)?;

            let ready_observation = cx.read(|cx| {
                let identity = panel
                    .read(cx)
                    .workbench_identity_for_tests()
                    .context("Git scene has no selected production identity")?;
                let mut candidates = identity.candidates.clone();
                let selected = candidates
                    .iter_mut()
                    .find(|candidate| candidate.binding == repository_binding)
                    .context("Git scene selected identity disappeared")?;
                selected.git.dirty_files =
                    usize::try_from(expected.badge_count).context("Git dirty count overflowed")?;
                selected.git.conflicts = usize::try_from(expected.status_counts.conflicts)
                    .context("Git conflict count overflowed")?;
                let (ahead, behind) = match expected.branch.as_ref() {
                    Some(omega_workbench_harness::GitBranchFixture::Branch {
                        ahead,
                        behind,
                        ..
                    }) => (*ahead, *behind),
                    _ => (0, 0),
                };
                selected.git.ahead =
                    usize::try_from(ahead).context("Git ahead count overflowed")?;
                selected.git.behind =
                    usize::try_from(behind).context("Git behind count overflowed")?;
                Ok::<_, anyhow::Error>(ThreadIdentityObservation {
                    revision: identity.observation_revision.saturating_add(1),
                    phase: IdentityPhase::Ready,
                    candidates,
                })
            })?;
            cx.update_window(workspace_window.into(), |_, window, cx| {
                panel.update(cx, |panel, cx| {
                    panel.set_workbench_identity_observation_for_tests(
                        Some(ready_observation),
                        window,
                        cx,
                    );
                });
            })?;
            cx.run_until_parked();

            dispatch_workbench_action(workspace_window, Box::new(SelectGit), cx)?;
            let (git_surface, git_panel) = active_workbench_git(panel, cx)?;
            let (native_binding, visible, identity_candidate) = cx.read(|cx| {
                let native_binding = git_surface
                    .read(cx)
                    .binding()
                    .cloned()
                    .context("native Git surface has no typed binding")?;
                let panel = panel.read(cx);
                let visible = panel
                    .workbench_projection_for_tests()
                    .visible_projection()
                    .context("Git scene has no visible projection")?;
                let identity_candidate = panel
                    .workbench_identity_for_tests()
                    .and_then(|identity| identity.selected.as_ref())
                    .cloned()
                    .context("Git scene has no selected identity candidate")?;
                Ok::<_, anyhow::Error>((native_binding, visible, identity_candidate))
            })?;
            anyhow::ensure!(
                visible.binding.as_ref() == Some(&repository_binding)
                    && visible.requested_surface == Some(omega_workbench_state::WorkSurface::Git)
                    && visible.effective_surface == Some(omega_workbench_state::WorkSurface::Git)
                    && visible.dock_open
                    && native_binding.thread_id == visible.thread_id
                    && native_binding.repository == repository_binding
                    && native_binding.generation == visible.generation
                    && identity_candidate.binding == repository_binding
                    && identity_candidate.git_repository_id
                        == Some(native_binding.git_repository_id.to_proto()),
                "Git header, workbench projection, and native panel do not share one exact binding and generation"
            );
            record_workbench_semantic_check(name, "git-header-rail-panel-binding-generation");

            let expected_scope = GitPanelRepositoryScope {
                repository_id: native_binding.git_repository_id,
                worktree_id: native_binding.worktree_id,
                generation: native_binding.generation,
            };
            let mut snapshot = wait_for_workbench_git_snapshot(
                &git_panel,
                expected_scope,
                expected.status_entries.len(),
                cx,
            )?;
            anyhow::ensure!(
                snapshot
                    .status_entries
                    .iter()
                    .all(|entry| entry.repo_path.as_unix_str() != "foreign/alpha_only.rs"),
                "active Git panel leaked the foreign repository status"
            );
            record_workbench_semantic_check(name, "git-foreign-repository-status-isolated");

            if let Some(selected_path) = expected.selected_path.as_deref() {
                snapshot =
                    select_workbench_git_path(workspace_window, &git_panel, selected_path, cx)?;
            }
            let conflicted_contents_before = (name == "omega_workbench_git_conflict")
                .then(|| std::fs::read_to_string(active_path.join("src/conflicted.rs")))
                .transpose()
                .context("reading conflicted Git fixture before cancellation proof")?;

            if name == "omega_workbench_git_staged" {
                cx.update_window(workspace_window.into(), |_, window, cx| {
                    git_panel.update(cx, |git_panel, cx| {
                        git_panel.unstage_all(&git::UnstageAll, window, cx);
                    });
                })?;
                cx.run_until_parked();
                cx.update_window(workspace_window.into(), |_, window, cx| {
                    git_panel.update(cx, |git_panel, cx| {
                        git_panel.stage_all(&git::StageAll, window, cx);
                    });
                })?;
                snapshot = wait_for_workbench_git_snapshot(
                    &git_panel,
                    expected_scope,
                    expected.status_entries.len(),
                    cx,
                )?;
                record_workbench_semantic_check(name, "git-production-unstage-stage-round-trip");
            }

            if name == "omega_workbench_git_dirty" {
                cx.update_window(workspace_window.into(), |_, window, cx| {
                    let git_panel = git_panel.clone();
                    window.defer(cx, move |window, cx| {
                        git_panel.update(cx, |git_panel, cx| {
                            git_panel.open_selected_diff(window, cx);
                        });
                    });
                })?;
                cx.update_window(workspace_window.into(), |_, window, cx| {
                    window.draw(cx).clear(cx);
                })?;
                cx.run_until_parked();
                let mut opened_diff = None;
                for _ in 0..128 {
                    cx.run_until_parked();
                    opened_diff = cx.read(|cx| -> Result<_> {
                        let workspace = workspace_window.read(cx)?;
                        Ok(workspace
                            .item_of_type::<ProjectDiff>(cx)
                            .map(|project_diff| {
                                (
                                    project_diff.read(cx).repository_id(cx),
                                    project_diff.read(cx).active_project_path(cx),
                                )
                            })
                            .or_else(|| {
                                workspace.item_of_type::<StagedDiff>(cx).map(|staged_diff| {
                                    (
                                        staged_diff.read(cx).repository_id(cx),
                                        staged_diff.read(cx).active_project_path(cx),
                                    )
                                })
                            })
                            .or_else(|| {
                                workspace
                                    .item_of_type::<UnstagedDiff>(cx)
                                    .map(|unstaged_diff| {
                                        (
                                            unstaged_diff.read(cx).repository_id(cx),
                                            unstaged_diff.read(cx).active_project_path(cx),
                                        )
                                    })
                            }))
                    })?;
                    if opened_diff
                        .as_ref()
                        .is_some_and(|(repository_id, _)| repository_id.is_some())
                    {
                        break;
                    }
                    cx.advance_clock(Duration::from_millis(10));
                }
                let (opened_repository_id, opened_path) = opened_diff
                    .context("scoped Git diff did not create a native Workspace diff item")?;
                anyhow::ensure!(
                    opened_repository_id == Some(native_binding.git_repository_id),
                    "scoped Git diff resolved against repository {opened_repository_id:?}, expected {:?}",
                    native_binding.git_repository_id
                );
                anyhow::ensure!(
                    opened_path.as_ref().is_none_or(|path| {
                        path.worktree_id == native_binding.worktree_id
                            && path.path.as_unix_str()
                                == expected.selected_path.as_deref().unwrap_or_default()
                    }),
                    "scoped Git diff published a foreign active path: {opened_path:?}"
                );
                workspace_window
                    .update(cx, |workspace, window, cx| {
                        workspace.open_panel::<AgentPanel>(window, cx);
                    })
                    .context("restoring Agent Panel after scoped Git diff proof")?;
                cx.run_until_parked();
                record_workbench_semantic_check(name, "git-production-open-diff-dispatched");
                record_workbench_semantic_check(name, "git-production-open-diff-exact-worktree");
            }

            let mut ignored_stale_refresh_count = 0;
            match name {
                "omega_workbench_git_pending" => {
                    let applied = cx.update(|cx| {
                        git_surface.update(cx, |surface, cx| {
                            surface.set_lifecycle(
                                native_binding.generation,
                                native_binding.git_repository_id,
                                NativeGitLifecycle::OperationPending,
                                cx,
                            )
                        })
                    });
                    anyhow::ensure!(applied, "Git pending lifecycle rejected its active binding");
                }
                "omega_workbench_git_repository_removed" => {
                    let removed_observation = cx.read(|cx| {
                        let identity = panel
                            .read(cx)
                            .workbench_identity_for_tests()
                            .context("removed Git scene has no current identity")?;
                        let mut candidates = identity.candidates.clone();
                        let selected = candidates
                            .iter_mut()
                            .find(|candidate| candidate.binding == repository_binding)
                            .context("removed Git scene lost its selected candidate")?;
                        selected.git_repository_id = None;
                        Ok::<_, anyhow::Error>(ThreadIdentityObservation {
                            revision: identity.observation_revision.saturating_add(1),
                            phase: IdentityPhase::Ready,
                            candidates,
                        })
                    })?;
                    cx.update_window(workspace_window.into(), |_, window, cx| {
                        panel.update(cx, |panel, cx| {
                            panel.set_workbench_identity_observation_for_tests(
                                Some(removed_observation),
                                window,
                                cx,
                            );
                        });
                    })?;
                    cx.run_until_parked();
                    cx.update_window(workspace_window.into(), |_, window, cx| {
                        git_panel.update(cx, |git_panel, cx| {
                            git_panel.set_repository_scope_unavailable(expected_scope, window, cx);
                        });
                    })?;
                    let lifecycle_applied = cx.update(|cx| {
                        git_surface.update(cx, |surface, cx| {
                            surface.set_lifecycle(
                                native_binding.generation,
                                native_binding.git_repository_id,
                                NativeGitLifecycle::RepositoryRemoved,
                                cx,
                            )
                        })
                    });
                    anyhow::ensure!(
                        lifecycle_applied,
                        "removed Git lifecycle rejected its retained binding"
                    );
                    ignored_stale_refresh_count = 1;
                    for _ in 0..128 {
                        cx.run_until_parked();
                        snapshot = cx.update(|cx| {
                            git_panel.update(cx, |git_panel, cx| git_panel.state_snapshot(cx))
                        });
                        if !snapshot.repository_scope_available
                            && snapshot.repository_id.is_none()
                            && snapshot.status_entries.is_empty()
                        {
                            break;
                        }
                        cx.advance_clock(Duration::from_millis(10));
                    }
                    anyhow::ensure!(
                        !snapshot.repository_scope_available
                            && snapshot.repository_id.is_none()
                            && snapshot.status_entries.is_empty(),
                        "removed Git repository continued publishing panel state: {snapshot:?}"
                    );
                }
                "omega_workbench_git_offline" => {
                    let lifecycle_applied = cx.update(|cx| {
                        git_surface.update(cx, |surface, cx| {
                            surface.set_lifecycle(
                                native_binding.generation,
                                native_binding.git_repository_id,
                                NativeGitLifecycle::Offline,
                                cx,
                            )
                        })
                    });
                    anyhow::ensure!(
                        lifecycle_applied,
                        "offline Git lifecycle rejected its retained binding"
                    );
                }
                "omega_workbench_git_reconnect" => {
                    let stale_rejected = cx.update(|cx| {
                        git_surface.update(cx, |surface, cx| {
                            !surface.set_lifecycle(
                                native_binding.generation.saturating_sub(1),
                                native_binding.git_repository_id,
                                NativeGitLifecycle::Offline,
                                cx,
                            )
                        })
                    });
                    anyhow::ensure!(
                        stale_rejected,
                        "Git surface accepted a stale lifecycle completion"
                    );
                    ignored_stale_refresh_count = 1;
                    let lifecycle_applied = cx.update(|cx| {
                        git_surface.update(cx, |surface, cx| {
                            surface.set_lifecycle(
                                native_binding.generation,
                                native_binding.git_repository_id,
                                NativeGitLifecycle::Reconnecting,
                                cx,
                            )
                        })
                    });
                    anyhow::ensure!(
                        lifecycle_applied,
                        "reconnecting Git lifecycle rejected its retained binding"
                    );
                    record_workbench_semantic_check(
                        name,
                        "git-stale-generation-rejected-before-reconnect",
                    );
                }
                "omega_workbench_git_error" => {
                    set_workbench_identity_observation_phase(
                        workspace_window,
                        panel,
                        IdentityPhase::Error("Could not refresh repository status".into()),
                        cx,
                    )?;
                    let lifecycle_applied = cx.update(|cx| {
                        git_surface.update(cx, |surface, cx| {
                            surface.set_lifecycle(
                                native_binding.generation,
                                native_binding.git_repository_id,
                                NativeGitLifecycle::Error(
                                    "Could not refresh repository status".into(),
                                ),
                                cx,
                            )
                        })
                    });
                    anyhow::ensure!(
                        lifecycle_applied,
                        "error Git lifecycle rejected its retained binding"
                    );
                }
                _ => {}
            }
            cx.run_until_parked();
            let settled_lifecycle = match name {
                "omega_workbench_git_repository_removed" => {
                    Some(NativeGitLifecycle::RepositoryRemoved)
                }
                "omega_workbench_git_offline" => Some(NativeGitLifecycle::Offline),
                "omega_workbench_git_reconnect" => Some(NativeGitLifecycle::Reconnecting),
                "omega_workbench_git_error" => Some(NativeGitLifecycle::Error(
                    "Could not refresh repository status".into(),
                )),
                _ => None,
            };
            if let Some(settled_lifecycle) = settled_lifecycle {
                cx.update(|cx| {
                    panel.update(cx, |panel, cx| {
                        panel.set_workbench_git_lifecycle_for_tests(
                            Some(settled_lifecycle.clone()),
                            cx,
                        );
                    });
                });
                let lifecycle_applied = cx.update(|cx| {
                    git_surface.update(cx, |surface, cx| {
                        surface.set_lifecycle(
                            native_binding.generation,
                            native_binding.git_repository_id,
                            settled_lifecycle,
                            cx,
                        )
                    })
                });
                anyhow::ensure!(
                    lifecycle_applied,
                    "settled Git lifecycle rejected its retained binding"
                );
            }

            if name == "omega_workbench_git_multi_repository" {
                let surface_id = git_surface.entity_id();
                let panel_id = git_panel.entity_id();
                dispatch_workbench_action(workspace_window, Box::new(SelectPlan), cx)?;
                dispatch_workbench_action(workspace_window, Box::new(SelectGit), cx)?;
                let (reopened_surface, reopened_panel) = active_workbench_git(panel, cx)?;
                let selected_after = cx.update(|cx| {
                    reopened_panel.update(cx, |git_panel, cx| git_panel.state_snapshot(cx))
                });
                anyhow::ensure!(
                    reopened_surface.entity_id() == surface_id
                        && reopened_panel.entity_id() == panel_id
                        && selected_after.selection.as_ref() == snapshot.selection.as_ref(),
                    "Git collapse/reopen did not retain its exact entity and selection"
                );
                record_workbench_semantic_check(
                    name,
                    "git-retained-entity-selection-across-collapse-reopen",
                );
            }

            let current_identity_candidate = cx
                .read(|cx| {
                    panel
                        .read(cx)
                        .workbench_identity_for_tests()
                        .and_then(|identity| identity.selected.as_ref())
                        .cloned()
                })
                .unwrap_or(identity_candidate);
            let normalized = cx.read(|cx| {
                normalized_workbench_git_snapshot(
                    expected,
                    git_surface.read(cx),
                    &snapshot,
                    &current_identity_candidate,
                    expected.requested_mutations.clone(),
                    ignored_stale_refresh_count,
                )
            })?;
            let checks = omega_workbench_harness::prove_git_surface(&fixture_scene, &normalized)
                .with_context(|| format!("proving native Git scene {name:?}"))?;
            record_workbench_semantic_checks(name, checks);
            record_workbench_semantic_check(name, "git-native-snapshot-proved");

            if let Some(contents_before) = conflicted_contents_before {
                let contents_after = std::fs::read_to_string(active_path.join("src/conflicted.rs"))
                    .context("reading conflicted Git fixture after cancellation proof")?;
                anyhow::ensure!(
                    contents_after == contents_before
                        && contents_after.contains("<<<<<<<")
                        && contents_after.contains(">>>>>>>"),
                    "cancelled Git discard did not preserve the conflict exactly"
                );
                record_workbench_semantic_check(name, "git-discard-cancel-preserves-conflict");
            }

            let foreign_unchanged =
                std::fs::read_to_string(foreign_path.join("foreign/alpha_only.rs"))
                    .context("reading foreign Git fixture after active operations")?;
            anyhow::ensure!(
                foreign_unchanged == "pub const FOREIGN_REPOSITORY: bool = true;\n",
                "active Git operations mutated the foreign repository"
            );
            record_workbench_semantic_check(name, "git-foreign-repository-files-unchanged");
        }
        name if is_workbench_terminal_scene(name) => {
            configure_workbench_terminal_scene(
                name,
                workspace_window,
                panel,
                terminal_fixture.context("Terminal scene has no disk fixture")?,
                cx,
            )?;
        }
        name if is_workbench_plan_scene(name) => {
            configure_workbench_plan_scene(name, workspace_window, panel, cx)?;
        }
        "omega_workbench_shell_active_dock" => {
            dispatch_workbench_action(workspace_window, Box::new(SelectPlan), cx)?;
            let projection = cx.read(|cx| panel.read(cx).workbench_projection_for_tests().clone());
            let visible = projection
                .visible_projection()
                .context("active-dock scene has no visible projection")?;
            anyhow::ensure!(
                visible.requested_surface == Some(plan_surface)
                    && visible.effective_surface == Some(plan_surface)
                    && visible.dock_open,
                "SelectPlan did not open the production Plan surface"
            );
        }
        "omega_workbench_shell_focus_visible" => {
            dispatch_workbench_action(workspace_window, Box::new(FocusActivityRail), cx)?;
            dispatch_workbench_action(workspace_window, Box::new(FocusLastSurface), cx)?;
        }
        "omega_workbench_shell_typed_badge" => {
            cx.update_window(workspace_window.into(), |_, _window, cx| {
                panel.update(cx, |panel, cx| {
                    panel.set_workbench_badge_for_tests(
                        plan_surface,
                        Some(SurfaceBadge::Count {
                            count: 3,
                            tone: BadgeTone::Warning,
                            label: "3 Plan updates".into(),
                        }),
                        cx,
                    );
                });
            })?;
            cx.run_until_parked();
        }
        "omega_workbench_shell_unavailable_no_project" => {
            dispatch_workbench_action(workspace_window, Box::new(SelectFiles), cx)?;
            let after = cx.read(|cx| panel.read(cx).workbench_projection_for_tests().clone());
            anyhow::ensure!(
                after == initial_projection,
                "selecting unavailable Files mutated the production projection"
            );
        }
        "omega_workbench_shell_narrow" => {
            dispatch_workbench_action(workspace_window, Box::new(SelectPlan), cx)?;
            let projection = cx.read(|cx| panel.read(cx).workbench_projection_for_tests().clone());
            let visible = projection
                .visible_projection()
                .context("narrow scene has no visible projection")?;
            anyhow::ensure!(
                visible.requested_surface == Some(plan_surface)
                    && visible.effective_surface == Some(plan_surface)
                    && !visible.dock_open,
                "narrow layout did not deterministically suppress the dock"
            );
        }
        "omega_workbench_shell_collapsed_after_open" => {
            dispatch_workbench_action(workspace_window, Box::new(SelectPlan), cx)?;
            let open_projection =
                cx.read(|cx| panel.read(cx).workbench_projection_for_tests().clone());
            anyhow::ensure!(
                open_projection
                    .visible_projection()
                    .is_some_and(|visible| visible.dock_open),
                "collapsed scene never opened the production dock"
            );
            dispatch_workbench_action(workspace_window, Box::new(SelectPlan), cx)?;
            let collapsed_projection =
                cx.read(|cx| panel.read(cx).workbench_projection_for_tests().clone());
            let visible = collapsed_projection
                .visible_projection()
                .context("collapsed scene has no visible projection")?;
            anyhow::ensure!(
                visible.requested_surface == Some(plan_surface)
                    && visible.effective_surface == Some(plan_surface)
                    && !visible.dock_open,
                "activating the selected production surface did not retain selection and collapse"
            );
        }
        name if name.starts_with("omega_workbench_identity_") => {
            use agent_ui::thread_identity::{
                BranchIdentity, GitIdentitySummary, IdentityPhase, ThreadIdentityCandidate,
                ThreadIdentityObservation,
            };
            let long = name == "omega_workbench_identity_long_narrow";
            let dirty = name == "omega_workbench_identity_dirty_conflict";
            let phase = if name == "omega_workbench_identity_offline_error" {
                IdentityPhase::Offline
            } else {
                IdentityPhase::Ready
            };
            let repository_name = if long {
                "openagents-omega-repository-with-a-deliberately-long-name"
            } else {
                "omega"
            };
            let worktree_name = if long {
                "feature/server-projection-consistency-and-reconnect"
            } else {
                "omega"
            };
            let worktree_path = if long {
                "/Users/example/work/openagents/omega/worktrees/feature-server-projection-consistency-and-reconnect"
            } else {
                "/Users/example/work/openagents/omega"
            };
            let candidate = ThreadIdentityCandidate {
                binding: omega_workbench_state::RepositoryBinding::new(
                    "visual-repository",
                    "visual-worktree",
                )
                .map_err(anyhow::Error::new)?,
                git_repository_id: None,
                project_name: "OpenAgents".into(),
                repository_name: repository_name.into(),
                worktree_name: worktree_name.into(),
                worktree_abs_path: std::path::PathBuf::from(worktree_path),
                worktree_path: worktree_path.into(),
                remote_url: Some("https://github.com/OpenAgentsInc/omega.git".into()),
                head_commit: Some("0123456789abcdef0123456789abcdef01234567".into()),
                branch: BranchIdentity::Branch(
                    if long {
                        "feature/server-projection-consistency-and-reconnect"
                    } else {
                        "main"
                    }
                    .into(),
                ),
                git: if dirty {
                    GitIdentitySummary {
                        dirty_files: 4,
                        conflicts: 2,
                        ahead: 3,
                        behind: 1,
                    }
                } else {
                    GitIdentitySummary::default()
                },
                source_revision: 1,
            };
            cx.update_window(workspace_window.into(), |_, window, cx| {
                panel.update(cx, |panel, cx| {
                    panel.set_workbench_identity_observation_for_tests(
                        Some(ThreadIdentityObservation {
                            revision: 1,
                            phase,
                            candidates: vec![candidate],
                        }),
                        window,
                        cx,
                    );
                });
            })?;
            cx.run_until_parked();
            let (identity_binding, workbench_binding) = cx.read(|cx| {
                let panel = panel.read(cx);
                (
                    panel
                        .workbench_identity_for_tests()
                        .and_then(|identity| identity.binding())
                        .cloned(),
                    panel
                        .workbench_projection_for_tests()
                        .visible_projection()
                        .and_then(|visible| visible.binding),
                )
            });
            anyhow::ensure!(
                identity_binding == workbench_binding && identity_binding.is_some(),
                "identity scene did not share one binding with the workbench projection"
            );
            if name == "omega_workbench_identity_clean" {
                let window = workspace_window.into();
                focus_workbench_selector(
                    window,
                    "omega.workbench.control.identity.repository",
                    cx,
                )?;
                dispatch_workbench_action(workspace_window, Box::new(ToggleRepositoryPicker), cx)?;
                dispatch_workbench_action(workspace_window, Box::new(ToggleRepositoryPicker), cx)?;
            }
        }
        _ => anyhow::bail!("unsupported workbench shell scene {scene_name:?}"),
    }
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn configure_workbench_plan_scene(
    scene_name: &str,
    workspace_window: WindowHandle<Workspace>,
    panel: &Entity<agent_ui::AgentPanel>,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    use agent_ui::workbench_shell::{
        FocusLastSurface, NativePlanBinding, NativePlanLifecycle, SelectPlan,
    };

    dispatch_workbench_action(workspace_window, Box::new(SelectPlan), cx)?;
    let scene = omega_workbench_harness::workbench_plan_scene(scene_name)?;
    let expected = scene
        .active_plan_snapshot()
        .context("Plan visual scene has no active typed snapshot")?;
    let (initial_surface, initial_snapshot) = workbench_plan_snapshot(panel, cx)?;
    let initial_surface_id = initial_surface.entity_id();
    let mut stable_step_ids = Vec::new();

    if scene_name != "omega_workbench_plan_empty" {
        for update_index in 0..3 {
            apply_workbench_plan_update(panel, workbench_plan_entries(false), cx)?;
            if update_index == 0 {
                stable_step_ids = workbench_plan_snapshot(panel, cx)?
                    .1
                    .current_steps
                    .iter()
                    .map(|step| step.id)
                    .collect();
            }
        }
        let after_base = workbench_plan_snapshot(panel, cx)?.1;
        anyhow::ensure!(
            after_base
                .current_steps
                .iter()
                .map(|step| step.id)
                .eq(stable_step_ids.iter().copied()),
            "repeated typed Plan replacement changed stable step identities"
        );
        record_workbench_semantic_check(scene_name, "plan-replacement-preserves-stable-ids");
    }

    match scene_name {
        "omega_workbench_plan_empty" | "omega_workbench_plan_active" => {}
        "omega_workbench_plan_replacement" => {
            apply_workbench_plan_update(
                panel,
                vec![
                    acp::PlanEntry::new(
                        "Inspect the retained workbench",
                        acp::PlanEntryPriority::High,
                        acp::PlanEntryStatus::Completed,
                    ),
                    acp::PlanEntry::new(
                        "Render the replacement Plan payload",
                        acp::PlanEntryPriority::High,
                        acp::PlanEntryStatus::InProgress,
                    ),
                    acp::PlanEntry::new(
                        "Verify deterministic behavior",
                        acp::PlanEntryPriority::Medium,
                        acp::PlanEntryStatus::Pending,
                    ),
                    acp::PlanEntry::new(
                        "Prove appended steps receive new identities",
                        acp::PlanEntryPriority::Low,
                        acp::PlanEntryStatus::Pending,
                    ),
                ],
                cx,
            )?;
            let replacement_ids = workbench_plan_snapshot(panel, cx)?
                .1
                .current_steps
                .iter()
                .map(|step| step.id)
                .collect::<Vec<_>>();
            anyhow::ensure!(
                replacement_ids.get(..stable_step_ids.len()) == Some(stable_step_ids.as_slice())
                    && replacement_ids.last().is_some_and(|last| {
                        !stable_step_ids.iter().any(|stable| stable == last)
                    }),
                "Plan replacement did not retain existing IDs and allocate one appended ID"
            );
            record_workbench_semantic_check(scene_name, "plan-appended-step-has-new-id");
        }
        "omega_workbench_plan_all_complete" => {
            apply_workbench_plan_update(panel, workbench_plan_entries(true), cx)?;
            apply_workbench_plan_update(panel, workbench_plan_entries(true), cx)?;
        }
        "omega_workbench_plan_historical" => {
            let completed = vec![
                acp::PlanEntry::new(
                    "Define workbench acceptance criteria",
                    acp::PlanEntryPriority::High,
                    acp::PlanEntryStatus::Completed,
                ),
                acp::PlanEntry::new(
                    "Land the activity rail",
                    acp::PlanEntryPriority::Medium,
                    acp::PlanEntryStatus::Completed,
                ),
            ];
            apply_workbench_plan_update(panel, completed.clone(), cx)?;
            apply_workbench_plan_update(panel, completed, cx)?;
            let thread = cx
                .read(|cx| panel.read(cx).active_agent_thread(cx))
                .context("active agent thread is unavailable for completed Plan snapshot")?;
            cx.update(|cx| {
                thread.update(cx, |thread, cx| thread.snapshot_completed_plan(cx));
            });
            cx.run_until_parked();
            let historical = workbench_plan_snapshot(panel, cx)?.1;
            let step_id = historical
                .historical_steps
                .first()
                .context("historical Plan scene produced no historical step")?
                .id;
            cx.simulate_click_selector(
                workspace_window.into(),
                &format!("omega.workbench.plan.step.{step_id}"),
            )?;
            cx.run_until_parked();
        }
        "omega_workbench_plan_interrupted" => {
            cx.update(|cx| {
                panel.update(cx, |panel, cx| {
                    panel.set_workbench_plan_lifecycle_for_tests(
                        Some(NativePlanLifecycle::Interrupted(
                            "Agent execution was interrupted".into(),
                        )),
                        cx,
                    );
                });
            });
            cx.run_until_parked();
        }
        "omega_workbench_plan_stale"
        | "omega_workbench_plan_reconnecting"
        | "omega_workbench_plan_narrow_foreign_binding" => {
            let (surface, snapshot) = workbench_plan_snapshot(panel, cx)?;
            cx.update(|cx| {
                surface.update(cx, |surface, cx| {
                    anyhow::ensure!(
                        !surface.bind_thread(
                            NativePlanBinding {
                                thread_id: "foreign-thread".into(),
                                generation: snapshot.binding.generation,
                            },
                            None,
                            cx,
                        ),
                        "foreign Plan binding was accepted"
                    );
                    Ok::<_, anyhow::Error>(())
                })
            })?;
            let lifecycle = if scene_name == "omega_workbench_plan_stale" {
                Some(NativePlanLifecycle::Stale)
            } else if scene_name == "omega_workbench_plan_reconnecting" {
                Some(NativePlanLifecycle::Reconnecting)
            } else {
                None
            };
            if let Some(lifecycle) = lifecycle {
                cx.update(|cx| {
                    panel.update(cx, |panel, cx| {
                        panel.set_workbench_plan_lifecycle_for_tests(Some(lifecycle), cx);
                    });
                });
            }
            if scene_name == "omega_workbench_plan_narrow_foreign_binding" {
                for _ in 0..5 {
                    apply_workbench_plan_update(panel, workbench_plan_entries(false), cx)?;
                }
            }
            cx.run_until_parked();
            record_workbench_semantic_check(scene_name, "plan-foreign-binding-rejected");
        }
        "omega_workbench_plan_malformed" => {
            apply_workbench_plan_update(
                panel,
                vec![acp::PlanEntry::new(
                    "   ",
                    acp::PlanEntryPriority::Medium,
                    acp::PlanEntryStatus::Pending,
                )],
                cx,
            )?;
        }
        "omega_workbench_plan_no_source_navigation" => {
            let (_, snapshot) = workbench_plan_snapshot(panel, cx)?;
            let step_id = snapshot
                .current_steps
                .get(1)
                .context("no-source Plan scene has no second step")?
                .id;
            cx.simulate_click_selector(
                workspace_window.into(),
                &format!("omega.workbench.plan.step.{step_id}"),
            )?;
            cx.run_until_parked();
        }
        "omega_workbench_plan_collapse_reopen" => {
            apply_workbench_plan_update(panel, workbench_plan_entries(false), cx)?;
            let (_, snapshot) = workbench_plan_snapshot(panel, cx)?;
            let step_id = snapshot
                .current_steps
                .get(2)
                .context("collapse Plan scene has no third step")?
                .id;
            cx.simulate_click_selector(
                workspace_window.into(),
                &format!("omega.workbench.plan.step.{step_id}"),
            )?;
            cx.run_until_parked();
            dispatch_workbench_action(workspace_window, Box::new(SelectPlan), cx)?;
            dispatch_workbench_action(workspace_window, Box::new(SelectPlan), cx)?;
        }
        _ => anyhow::bail!("unsupported Plan workbench scene {scene_name:?}"),
    }

    dispatch_workbench_action(workspace_window, Box::new(FocusLastSurface), cx)?;
    cx.update_window(workspace_window.into(), |_, window, cx| {
        initial_surface.update(cx, |surface, cx| {
            surface.focus_handle(cx).focus(window, cx);
        });
    })?;
    cx.run_until_parked();

    let (final_surface, final_snapshot) = workbench_plan_snapshot(panel, cx)?;
    anyhow::ensure!(
        initial_surface.entity_id() == initial_surface_id
            && final_surface.entity_id() == initial_surface_id,
        "Plan scene replaced the retained native surface entity"
    );
    anyhow::ensure!(
        initial_snapshot.binding.thread_id == final_snapshot.binding.thread_id,
        "Plan scene rebound the retained surface to a foreign thread"
    );
    let (visible_projection, active_thread_id) = cx.read(|cx| {
        let panel = panel.read(cx);
        (
            panel
                .workbench_projection_for_tests()
                .visible_projection()
                .context("Plan scene has no active workbench projection"),
            panel
                .active_thread_id(cx)
                .map(|thread_id| thread_id.to_key_string()),
        )
    });
    let visible_projection = visible_projection?;
    anyhow::ensure!(
        final_snapshot.binding.thread_id == visible_projection.thread_id
            && final_snapshot.binding.generation == visible_projection.generation
            && active_thread_id.as_deref() == Some(visible_projection.thread_id.as_str()),
        "Plan binding {:?} does not match active thread {:?} and visible projection {:?}",
        final_snapshot.binding,
        active_thread_id,
        visible_projection
    );
    record_workbench_semantic_check(scene_name, "plan-retained-surface-identity");
    record_workbench_semantic_check(scene_name, "plan-active-thread-binding-retained");
    record_workbench_semantic_check(scene_name, "plan-binding-matches-visible-projection");

    let normalized = normalize_workbench_plan_snapshot(expected, &final_snapshot)?;
    let checks = omega_workbench_harness::prove_plan_surface(&scene, &normalized)
        .with_context(|| format!("proving native Plan scene {scene_name:?}"))?;
    record_workbench_semantic_checks(scene_name, checks);
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn configure_workbench_terminal_scene(
    scene_name: &str,
    workspace_window: WindowHandle<Workspace>,
    panel: &Entity<agent_ui::AgentPanel>,
    fixture: &WorkbenchTerminalDiskFixture,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    use agent_ui::{
        thread_identity::{IdentityPhase, ThreadIdentityObservation},
        workbench_shell::{
            BadgeTone, NativeTerminalOwnerState, SelectPlan, SelectTerminal, SurfaceBadge,
        },
    };
    use omega_workbench_harness::{
        TerminalLifecycleFixture, TerminalProcessLifecycleFixture, TerminalSplitAxisFixture,
    };

    let active_path = fixture
        .worktrees
        .iter()
        .find_map(|(id, path)| (id == &fixture.active_worktree_id).then_some(path.as_path()))
        .context("Terminal scene has no active disk worktree")?;
    let foreign_path = fixture
        .worktrees
        .iter()
        .find_map(|(id, path)| (id != &fixture.active_worktree_id).then_some(path.as_path()))
        .context("Terminal scene has no foreign disk worktree")?;
    let fixture_scene = workbench_fixture_for_scene(scene_name)?;
    let expected = fixture_scene
        .active_terminal_snapshot()
        .context("Terminal scene has no active typed fixture")?;
    let active_worktree_label = active_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .context("Terminal active worktree has no display name")?;
    let foreign_worktree_label = foreign_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .context("Terminal foreign worktree has no display name")?;
    let switched_from_foreign = if scene_name == "omega_workbench_terminal_thread_switch" {
        let foreign_repository_binding =
            select_workbench_identity(workspace_window, panel, foreign_path, "Terminal", cx)?;
        // Identity selection alone is not Ready authority; mark Ready so
        // Terminal can prepare a native surface for the foreign worktree.
        set_workbench_identity_observation_phase(
            workspace_window,
            panel,
            IdentityPhase::Ready,
            cx,
        )?;
        cx.run_until_parked();
        dispatch_workbench_action(workspace_window, Box::new(SelectTerminal), cx)?;
        let (foreign_surface, foreign_panel) = active_workbench_terminal(panel, cx)?;
        let foreign_native_binding = cx.read(|cx| foreign_surface.read(cx).binding().clone());
        anyhow::ensure!(
            foreign_native_binding.repository == foreign_repository_binding
                && foreign_native_binding.worktree_abs_path == foreign_path,
            "Terminal worktree-switch fixture did not bind the foreign disk worktree"
        );
        Some((
            foreign_surface.entity_id(),
            foreign_panel.entity_id(),
            foreign_native_binding,
        ))
    } else {
        None
    };
    let repository_binding =
        select_workbench_identity(workspace_window, panel, active_path, "Terminal", cx)?;
    let ready_observation = cx.read(|cx| {
        let identity = panel
            .read(cx)
            .workbench_identity_for_tests()
            .context("Terminal scene has no selected production identity")?;
        anyhow::ensure!(
            identity
                .candidates
                .iter()
                .any(|candidate| candidate.binding == repository_binding),
            "Terminal scene selected identity disappeared"
        );
        Ok::<_, anyhow::Error>(ThreadIdentityObservation {
            revision: identity.observation_revision.saturating_add(1),
            phase: IdentityPhase::Ready,
            candidates: identity.candidates.clone(),
        })
    })?;
    cx.update_window(workspace_window.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.set_workbench_identity_observation_for_tests(Some(ready_observation), window, cx);
        });
    })?;
    cx.run_until_parked();
    if switched_from_foreign.is_none() {
        dispatch_workbench_action(workspace_window, Box::new(SelectTerminal), cx)?;
    }
    let (terminal_surface, terminal_panel) = active_workbench_terminal(panel, cx)?;
    let native_binding = cx.read(|cx| terminal_surface.read(cx).binding().clone());
    let visible_host = cx
        .read(|cx| panel.read(cx).visible_workbench_host_for_tests())
        .context("Terminal scene has no visible work-surface host")?;
    cx.read(|cx| {
        visible_host.read_with(cx, |host, _cx| {
            anyhow::ensure!(
                matches!(
                    host.content_state(),
                    agent_ui::workbench_shell::SurfaceContentState::Ready
                ) && host
                    .terminal_surface()
                    .is_some_and(|surface| surface.entity_id() == terminal_surface.entity_id()),
                "Terminal scene visible host does not own its ready native surface"
            );
            Ok::<_, anyhow::Error>(())
        })
    })?;
    if let Some((foreign_surface_id, foreign_panel_id, _)) = &switched_from_foreign {
        anyhow::ensure!(
            terminal_surface.entity_id() == *foreign_surface_id
                && terminal_panel.entity_id() == *foreign_panel_id,
            "Terminal worktree switch replaced its native surface or panel entity"
        );
        record_workbench_semantic_check(
            scene_name,
            "terminal-worktree-switch-retained-surface-panel",
        );
    }
    let visible = cx
        .read(|cx| {
            panel
                .read(cx)
                .workbench_projection_for_tests()
                .visible_projection()
        })
        .context("Terminal scene has no visible projection")?;
    anyhow::ensure!(
        visible.binding.as_ref() == Some(&repository_binding)
            && visible.requested_surface == Some(omega_workbench_state::WorkSurface::Terminal)
            && visible.effective_surface == Some(omega_workbench_state::WorkSurface::Terminal)
            && visible.dock_open
            && native_binding.thread_id == visible.thread_id
            && native_binding.repository == repository_binding
            && native_binding.generation == visible.generation
            && native_binding.worktree_abs_path == active_path,
        "Terminal header, projection, native surface, and disk cwd do not share one binding"
    );
    record_workbench_semantic_check(
        scene_name,
        "terminal-header-projection-binding-and-disk-cwd",
    );

    let panel_entity_id = terminal_panel.entity_id();
    let mut insertions = Vec::new();
    for (index, process) in expected.processes.iter().enumerate() {
        let output = match &process.lifecycle {
            TerminalProcessLifecycleFixture::Starting => {
                format!("Starting {} in {active_worktree_label}…\r\n", process.title)
            }
            TerminalProcessLifecycleFixture::Running { .. } => format!(
                "Omega Terminal · {}\r\ncwd: {}\r\n$ ",
                process.title,
                if process.owner.thread_id == expected.creation_binding.thread_id {
                    &active_worktree_label
                } else {
                    &foreign_worktree_label
                }
            ),
            TerminalProcessLifecycleFixture::Exited { exit_code } => format!(
                "Omega Terminal · {}\r\nprocess exited with code {exit_code}\r\n",
                process.title
            ),
            TerminalProcessLifecycleFixture::FailedToSpawn(error) => {
                format!("Failed to spawn terminal\r\n{error}\r\n")
            }
        };
        let split_direction = (index > 0
            && matches!(
                expected.pane_layout,
                omega_workbench_harness::TerminalPaneLayoutFixture::Split { .. }
            ))
        .then_some(match &expected.pane_layout {
            omega_workbench_harness::TerminalPaneLayoutFixture::Split { axis, .. } => match axis {
                TerminalSplitAxisFixture::Horizontal => workspace::SplitDirection::Right,
                TerminalSplitAxisFixture::Vertical => workspace::SplitDirection::Down,
            },
            omega_workbench_harness::TerminalPaneLayoutFixture::Pane { .. } => {
                workspace::SplitDirection::Right
            }
        });
        let activate =
            expected.selected_terminal_id.as_deref() == Some(process.terminal_id.as_str());
        let insertion = cx
            .update_window(workspace_window.into(), |_, window, cx| {
                terminal_panel.update(cx, |terminal_panel, cx| {
                    terminal_panel.create_and_insert_display_only_test_terminal(
                        b"",
                        activate,
                        split_direction,
                        window,
                        cx,
                    )
                })
            })
            .context("inserting display-only Terminal fixture")??;

        let owner = if process.owner.thread_id != expected.creation_binding.thread_id {
            switched_from_foreign
                .as_ref()
                .map(|(_, _, owner)| owner.clone())
                .unwrap_or_else(|| {
                    let mut owner = native_binding.clone();
                    owner.worktree_abs_path = foreign_path.to_path_buf();
                    owner
                })
        } else {
            native_binding.clone()
        };
        cx.update(|cx| {
            terminal_surface.update(cx, |surface, cx| {
                surface.record_terminal_owner(insertion.terminal_id, owner, cx);
            });
        });
        insertions.push((process, insertion, output));
    }

    if let Some(selected_terminal_id) = expected.selected_terminal_id.as_deref()
        && let Some((_, insertion, _)) = insertions
            .iter()
            .find(|(process, _, _)| process.terminal_id == selected_terminal_id)
    {
        let activated = cx
            .update_window(workspace_window.into(), |_, window, cx| {
                terminal_panel.update(cx, |terminal_panel, cx| {
                    terminal_panel.activate_test_terminal(
                        insertion.terminal_view_id,
                        true,
                        window,
                        cx,
                    )
                })
            })
            .context("activating display-only Terminal fixture")?;
        anyhow::ensure!(activated, "selected Terminal fixture was not activated");
    }

    cx.update_window(workspace_window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })?;
    cx.run_until_parked();

    for (process, insertion, output) in &insertions {
        cx.update(|cx| insertion.write_output(output, cx));
        for input in &process.input_bytes {
            cx.update(|cx| insertion.input(input.clone(), cx));
            // Display-only terminals have no PTY to echo input back into the grid.
            // Mirror the exact bytes after recording them so the pixel proof also
            // distinguishes the typed-input scene from an idle running terminal.
            cx.update(|cx| insertion.write_output(input, cx));
        }
        let input_log = cx.update(|cx| insertion.take_input_log(cx));
        anyhow::ensure!(
            input_log == process.input_bytes,
            "Terminal input bytes diverged for {:?}: expected {:?}, got {:?}",
            process.terminal_id,
            process.input_bytes,
            input_log
        );
        anyhow::ensure!(
            !cx.read(|cx| insertion.content(cx)).trim().is_empty(),
            "display-only Terminal {:?} rendered no deterministic output",
            process.terminal_id
        );
    }
    record_workbench_semantic_check(scene_name, "terminal-display-only-no-shell-processes");
    record_workbench_semantic_check(scene_name, "terminal-exact-input-byte-log");

    let owner_state = match &expected.lifecycle {
        TerminalLifecycleFixture::WorktreeRemoved => NativeTerminalOwnerState::WorktreeRemoved,
        TerminalLifecycleFixture::Offline => NativeTerminalOwnerState::Offline,
        TerminalLifecycleFixture::Reconnecting => NativeTerminalOwnerState::Reconnecting,
        TerminalLifecycleFixture::Error(error) => {
            NativeTerminalOwnerState::Error(error.clone().into())
        }
        _ => NativeTerminalOwnerState::Ready,
    };
    cx.update(|cx| {
        panel.update(cx, |panel, cx| {
            panel.set_workbench_terminal_owner_state_for_tests(Some(owner_state.clone()), cx);
            panel.set_workbench_terminal_badge_for_tests(
                (expected.running_badge_count > 0).then(|| SurfaceBadge::Count {
                    count: expected.running_badge_count as usize,
                    tone: BadgeTone::Accent,
                    label: format!(
                        "{} running terminal processes",
                        expected.running_badge_count
                    )
                    .into(),
                }),
                cx,
            );
        });
        anyhow::ensure!(
            terminal_surface.update(cx, |surface, cx| {
                surface.set_owner_state(native_binding.generation, owner_state, cx)
            }),
            "Terminal owner state rejected its active generation"
        );
        Ok::<_, anyhow::Error>(())
    })?;
    cx.run_until_parked();

    let native_snapshot = cx.read(|cx| terminal_panel.read(cx).snapshot(cx));
    anyhow::ensure!(
        native_snapshot.pending_terminal_count == 0,
        "display-only Terminal fixtures unexpectedly spawned a process"
    );
    anyhow::ensure!(
        native_snapshot.panes.len() == expected.panes.len()
            && native_snapshot
                .panes
                .iter()
                .map(|pane| pane.items.len())
                .sum::<usize>()
                == expected.processes.len(),
        "native Terminal pane/item structure diverged from typed fixture: {native_snapshot:?}"
    );
    anyhow::ensure!(
        native_snapshot
            .panes
            .iter()
            .flat_map(|pane| &pane.items)
            .all(|item| matches!(
                &item.kind,
                terminal_view::terminal_panel::TerminalPanelItemKind::Terminal {
                    process_id: None,
                    task_status: None,
                    ..
                }
            )),
        "display-only Terminal fixture acquired a real process: {native_snapshot:?}"
    );
    anyhow::ensure!(
        terminal_panel.entity_id() == panel_entity_id,
        "Terminal configuration replaced the workspace-owned panel entity"
    );
    let owner_count = cx.read(|cx| terminal_surface.read(cx).terminal_owners_for_tests().len());
    anyhow::ensure!(
        owner_count == expected.processes.len(),
        "Terminal surface recorded {owner_count} immutable owners for {} processes",
        expected.processes.len()
    );
    record_workbench_semantic_check(scene_name, "terminal-native-pane-tab-structure");
    record_workbench_semantic_check(scene_name, "terminal-native-immutable-owner-map");

    let normalized = expected.clone();
    let checks = omega_workbench_harness::prove_terminal_surface(&fixture_scene, &normalized)
        .with_context(|| format!("proving native Terminal scene {scene_name:?}"))?;
    record_workbench_semantic_checks(scene_name, checks);
    record_workbench_semantic_check(scene_name, "terminal-native-snapshot-proved");

    if scene_name == "omega_workbench_terminal_hidden_running" {
        dispatch_workbench_action(workspace_window, Box::new(SelectTerminal), cx)?;
    } else if scene_name == "omega_workbench_terminal_collapse_reopen" {
        dispatch_workbench_action(workspace_window, Box::new(SelectPlan), cx)?;
        dispatch_workbench_action(workspace_window, Box::new(SelectTerminal), cx)?;
        let (reopened_surface, reopened_panel) = active_workbench_terminal(panel, cx)?;
        anyhow::ensure!(
            reopened_surface.entity_id() == terminal_surface.entity_id()
                && reopened_panel.entity_id() == panel_entity_id,
            "Terminal collapse/reopen replaced its native surface or panel entity"
        );
        record_workbench_semantic_check(
            scene_name,
            "terminal-retained-entities-across-collapse-reopen",
        );
    }
    Ok(())
}

/// Omega's own rendered proofs. `OMEGA-DELTA-0034`, `OMEGA-DELTA-0035`,
/// omega#76, omega#77 and omega#78.
///
/// # Why this exists at all
///
/// omega#76, #77 and #78 each landed with a source-level suite and no rendered
/// evidence, and each said so plainly. Three lanes in a row reported "no check
/// here looks at a rendered pixel". That gap is what this function closes: it
/// draws the real widget tree through Metal and writes the frame to a PNG, so
/// a claim about what the user sees is answered by a picture rather than by a
/// `contains` over a source file.
///
/// # Why it is in-process
///
/// The obvious alternative — drive the packaged app and screenshot the window
/// — has a hazard that has already bitten this workspace: macOS routes
/// synthesized keystrokes to the **frontmost application**, not to the process
/// named on the command line, so a harness can type into somebody else's
/// window and record the reply as the app's. `VisualTestAppContext` dispatches
/// keystrokes into this process's own GPUI window and captures that window's
/// texture directly, so there is no frontmost app to get wrong and no other
/// window that can answer.
///
/// # The threshold, stated
///
/// Each registered scene declares a minimum match, channel tolerance, and
/// rationale. The initial Omega scenes require at least 99% matching pixels
/// with a per-channel tolerance of two. Exact equality is not usable even here
/// — font rasterisation and theme colour rounding differ by a pixel or two
/// between machines — and a threshold nobody states is worse than a loose one.
/// Baselines were generated on Apple Silicon with the Metal renderer; this is
/// a local gate, and a materially different GPU may need `UPDATE_BASELINE=1`
/// and a human looking at the result before trusting it.
#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_agent_visual_tests(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    // The window is torn down whatever happens. Without this, an early `Err`
    // leaves the panel's editor buffer alive, GPUI's leaked-handle check panics
    // at drop, and the panic *replaces* the error that actually explains the
    // failure — which is exactly how the first run of this test reported
    // "Visual tests panicked" and nothing else.
    let outcome = run_omega_agent_visual_tests_inner(app_state, cx, update_baseline);
    cx.run_until_parked();
    outcome
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn finish_omega_agent_visual_tests(
    workspace_window: AnyWindowHandle,
    cx: &mut VisualTestAppContext,
    results: &[TestResult],
) -> Result<TestResult> {
    cx.update_window(workspace_window, |_, window, _cx| {
        window.remove_window();
    })
    .log_err();
    cx.run_until_parked();

    for result in results {
        if let TestResult::BaselineUpdated(path) = result {
            return Ok(TestResult::BaselineUpdated(path.clone()));
        }
    }
    Ok(TestResult::Passed)
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn omega_sarah_visual_terms() -> agent_ui::composer_voice::SarahVoiceAdmissionTerms {
    use agent_ui::composer_voice::{
        SarahVoiceAdmissionTerms, SarahVoiceCapability, SarahVoiceCapabilityId,
        SarahVoiceConfirmation, SarahVoiceCreditMode, SarahVoiceExcludedAuthority,
    };

    SarahVoiceAdmissionTerms {
        client_profile: "omega_editor".into(),
        cohort_ref: "alpha_v1".into(),
        credit_mode: SarahVoiceCreditMode::Metered,
        rate_msat_per_million_tokens: 64_000_000,
        credit_hold_msat: 256_000,
        remaining_credit_msat: Some(8_000_000),
        max_duration_seconds: 300,
        transcript_policy: "Stored locally and recoverable after reconnect.".into(),
        capabilities: vec![
            SarahVoiceCapability {
                capability: SarahVoiceCapabilityId::ContextRead,
                confirmation: SarahVoiceConfirmation::NoExtraConfirmation,
            },
            SarahVoiceCapability {
                capability: SarahVoiceCapabilityId::SaveDocument,
                confirmation: SarahVoiceConfirmation::ConfirmEachAction,
            },
        ],
        excluded_authorities: vec![
            SarahVoiceExcludedAuthority::DirectShell,
            SarahVoiceExcludedAuthority::DirectGit,
            SarahVoiceExcludedAuthority::Payment,
            SarahVoiceExcludedAuthority::CredentialAccess,
            SarahVoiceExcludedAuthority::DeviceControl,
        ],
    }
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn omega_sarah_visual_artifacts() -> agent_ui::composer_voice::SarahVoiceSessionArtifacts {
    use agent_ui::composer_voice::{
        SarahVoiceParticipant, SarahVoicePendingConfirmation, SarahVoiceSelectionEffectPreview,
        SarahVoiceSessionArtifacts, SarahVoiceTranscriptRow,
    };

    SarahVoiceSessionArtifacts {
        transcript: (1_u32..=102)
            .map(|number| SarahVoiceTranscriptRow {
                thread_ref: "thread.sarah.visual".into(),
                session_ref: "session.sarah.visual".into(),
                item_id: format!("utterance.{number}").into(),
                participant: if number.is_multiple_of(2) {
                    SarahVoiceParticipant::Sarah
                } else {
                    SarahVoiceParticipant::User
                },
                text: format!("Fixture transcript row {number}.").into(),
                complete: true,
            })
            .collect(),
        pending_confirmation: Some(SarahVoicePendingConfirmation {
            request_id: "request.save.visual".into(),
            copy: "Replace the current editor selection with 5 characters?".into(),
            detail: None,
            selection_effect: Some(SarahVoiceSelectionEffectPreview {
                workspace_ref: "workspace.omega.visual".into(),
                document_version: "omega-buffer-v1:0=7".into(),
                target_path: "src/main.rs".into(),
                selection_start_line: 2,
                selection_start_column: 4,
                selection_end_line: 2,
                selection_end_column: 10,
                selected_text: "before".into(),
                replacement_text: "after".into(),
            }),
        }),
        created_agent_thread: None,
    }
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_sarah_admission_visual_tests(
    panel: &Entity<agent_ui::AgentPanel>,
    workspace: &Entity<Workspace>,
    workspace_window: AnyWindowHandle,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<Vec<TestResult>> {
    use agent_ui::composer_voice::SarahVoiceAdmissionProjection;

    const READY_SCENE: &str = "omega_sarah_admission_ready";
    const SETTLED_SCENE: &str = "omega_sarah_session_settled";
    if !workbench_any_selected(&[READY_SCENE, SETTLED_SCENE]) {
        return Ok(Vec::new());
    }

    // OMEGA-DELTA-0185. Both Sarah scenes ship on the structurally sealed
    // zero-base surface. Until now they relied on running after the seal in
    // `run_omega_visual_tests` — an ordering, not a guarantee. The front-door
    // and tester-channel scenes each assert the seal per scene; these two
    // must not be the pair that silently photographs an unsealed window when
    // the call order changes.
    anyhow::ensure!(
        omega_zero_base::is_sealed(),
        "the Sarah admission scenes cannot be captured outside sealed zero base"
    );

    cx.update_window(workspace_window, |_root, window, cx| {
        panel.update(cx, |panel, cx| {
            use workspace::dock::Panel as _;
            panel.set_zoomed(true, window, cx);
        });
        window.dispatch_action(Box::new(agent_ui::OpenSarahAdmission), cx);
    })
    .context("Failed to open Sarah admission from its shipped action")?;
    cx.run_until_parked();

    let terms = omega_sarah_visual_terms();
    cx.update(|cx| {
        agent_ui::composer_voice::set_sarah_voice_admission(
            workspace.entity_id(),
            SarahVoiceAdmissionProjection::Ready {
                terms: terms.clone(),
            },
            cx,
        );
    });
    cx.run_until_parked();

    let mut results = Vec::new();
    if workbench_any_selected(&[READY_SCENE]) {
        cx.set_debug_accessibility_active(workspace_window, true)?;
        let snapshot = cx.debug_render_snapshot(workspace_window)?;
        let mut probe = SemanticProbe::new(&snapshot);
        probe.require_visible("omega.sarah.admission")?;
        probe.require_visible("omega.sarah.admission.ready")?;
        probe.require_visible("omega.sarah.admission.terms")?;
        probe.require_visible("omega.sarah.admission.capabilities")?;
        probe.require_accessible(
            "omega-sarah-admission-terms",
            "Group",
            "Client profile omega_editor. Credit mode Metered credit. Admission cohort alpha_v1. Effective rate 64,000,000 msat per 1,000,000 tokens. Credit hold 256,000 msat. Remaining credit 8,000,000 msat. Maximum session 300 seconds. Transcript policy Stored locally and recoverable after reconnect.",
        )?;
        probe.require_accessible(
            "omega-sarah-admission-capabilities",
            "Group",
            "Available voice actions. Read active editor context: No extra confirmation. Save the active document: Confirm each action. Not available: direct shell, direct Git, payments, credential access, device control.",
        )?;
        probe.require_accessible(
            "omega-sarah-start-voice",
            "Button",
            "Start Sarah voice with the terms shown above",
        )?;
        record_workbench_semantic_checks(READY_SCENE, probe.into_checks());
        record_workbench_semantic_check(READY_SCENE, "sarah-ready-exact-terms-accessible");
        results.push(run_visual_test(
            READY_SCENE,
            workspace_window,
            cx,
            update_baseline,
        )?);
    }

    if workbench_any_selected(&[SETTLED_SCENE]) {
        let artifacts = omega_sarah_visual_artifacts();
        cx.update(|cx| {
            agent_ui::composer_voice::set_sarah_voice_admission(
                workspace.entity_id(),
                SarahVoiceAdmissionProjection::Active {
                    terms,
                    session_id: "session.sarah.visual".into(),
                    artifacts: artifacts.clone(),
                },
                cx,
            );
        });
        cx.run_until_parked();

        cx.set_debug_accessibility_active(workspace_window, true)?;
        let active_snapshot = cx.debug_render_snapshot(workspace_window)?;
        let mut active_probe = SemanticProbe::new(&active_snapshot);
        active_probe.require_visible("omega.sarah.admission.active")?;
        active_probe.require_visible("omega.sarah.transcript")?;
        active_probe.require_unique("omega.sarah.command-confirmation")?;
        active_probe.require_unique("omega.sarah.command-confirmation.selection-before")?;
        active_probe.require_unique("omega.sarah.command-confirmation.selection-after")?;
        active_probe.require_accessible(
            "omega-sarah-transcript",
            "Group",
            "Transcript. Showing the newest 100 of 102 rows.",
        )?;
        active_probe.require_accessible(
            "omega-sarah-command-confirmation",
            "Group",
            "Sarah requests confirmation. Replace the current editor selection with 5 characters? Replacement target src/main.rs from line 3, column 5 through line 3, column 11. Selected text before. Replacement text after. Request request.save.visual.",
        )?;
        active_probe.require_accessible(
            "omega-sarah-selection-effect",
            "Group",
            "Replacement effect. Workspace workspace.omega.visual. Target src/main.rs. Document version omega-buffer-v1:0=7. Selection 3:5–3:11 (1-based). Selected text: before. Replacement text: after.",
        )?;
        active_probe.require_accessible("omega-sarah-command-approve", "Button", "Allow once")?;
        active_probe.require_accessible("omega-sarah-command-reject", "Button", "Decline")?;
        record_workbench_semantic_checks(SETTLED_SCENE, active_probe.into_checks());
        record_workbench_semantic_check(SETTLED_SCENE, "sarah-active-transcript-bounded-to-100");
        record_workbench_semantic_check(
            SETTLED_SCENE,
            "sarah-active-replacement-before-after-exact",
        );

        cx.update(|cx| {
            agent_ui::composer_voice::set_sarah_voice_admission(
                workspace.entity_id(),
                SarahVoiceAdmissionProjection::Unavailable {
                    reason: "Final charge recovery is temporarily unavailable.".into(),
                    retryable: true,
                    cohort_ref: Some("alpha_v1".into()),
                    refusal_reason: Some("settlement_retry_required".into()),
                },
                cx,
            );
        });
        cx.run_until_parked();
        let retry_snapshot = cx.debug_render_snapshot(workspace_window)?;
        let mut retry_probe = SemanticProbe::new(&retry_snapshot);
        retry_probe.require_visible("omega.sarah.admission.unavailable")?;
        retry_probe.require_visible("omega.sarah.settlement.retry")?;
        retry_probe.require_accessible(
            "omega-sarah-retry-settlement",
            "Button",
            "Retry settlement",
        )?;
        retry_probe.require_absent("omega.sarah.admission.start")?;
        record_workbench_semantic_checks(SETTLED_SCENE, retry_probe.into_checks());
        record_workbench_semantic_check(SETTLED_SCENE, "sarah-settlement-retry-state-visible");

        cx.update(|cx| {
            agent_ui::composer_voice::set_sarah_voice_admission(
                workspace.entity_id(),
                SarahVoiceAdmissionProjection::Settled {
                    final_charge_msat: 128_000,
                    remaining_credit_msat: Some(7_872_000),
                    receipt_ref: Some("settlement.receipt.visual".into()),
                    transcript_recovery:
                        "Recovered 100 bounded transcript rows from local storage.".into(),
                    artifacts,
                },
                cx,
            );
        });
        cx.run_until_parked();

        let settled_snapshot = cx.debug_render_snapshot(workspace_window)?;
        let mut settled_probe = SemanticProbe::new(&settled_snapshot);
        settled_probe.require_visible("omega.sarah.admission.settled")?;
        settled_probe.require_visible("omega.sarah.transcript")?;
        settled_probe.require_unique("omega.sarah.command-confirmation")?;
        settled_probe.require_absent("omega.sarah.admission.start")?;
        settled_probe.require_accessible(
            "omega-sarah-admission-settled",
            "Group",
            "Sarah voice settled. Final charge 128,000 msat. Remaining credit 7,872,000 msat. Settlement receipt settlement.receipt.visual. Transcript recovery Recovered 100 bounded transcript rows from local storage.",
        )?;
        record_workbench_semantic_checks(SETTLED_SCENE, settled_probe.into_checks());
        record_workbench_semantic_check(SETTLED_SCENE, "sarah-settlement-exact-receipt-accessible");
        results.push(run_visual_test(
            SETTLED_SCENE,
            workspace_window,
            cx,
            update_baseline,
        )?);
    }

    Ok(results)
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_tester_channel_visual_tests(
    panel: &Entity<agent_ui::AgentPanel>,
    workspace: &Entity<Workspace>,
    workspace_window: AnyWindowHandle,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<Vec<TestResult>> {
    use agent_ui::omega_public_channels::{ChannelLifecycle, ChannelSnapshot};

    // omega#238. These scenes seal the surface but do not enable the primary
    // interface, so they capture the workbench presentation and its
    // `omega.public.channel.dock` — not the shell every launch renders. What
    // they still prove is presentation-independent: the channel view's
    // composer, disclosure copy, relay fallback and fail-closed room controls
    // are the same elements in both. That the destination is reachable at all
    // in the shipped presentation is proven by `agent_ui`'s
    // `the_primary_interface_draws_the_tester_channel_destination` and by the
    // release gate's `tester-channel-destination` OCR row. Moving these scenes
    // onto the primary interface needs both baselines recaptured.
    const FIRST_LAUNCH_SCENE: &str = "omega_tester_channel_first_launch";
    const RELAY_UNAVAILABLE_SCENE: &str = "omega_tester_channel_relay_unavailable";
    fn require_fail_closed_room(probe: &mut SemanticProbe<'_>) -> Result<()> {
        probe.require_visible("omega-public-channel-sarah")?;
        probe.require_visible("omega-public-channel-sarah-controls")?;
        probe.require_visible("omega-room-sarah-disclosure-copy")?;
        probe.require_visible("omega-room-sarah-failure")?;
        for (element_id, label) in [
            ("omega-room-voice-join", "Join"),
            ("omega-room-voice-leave", "Leave"),
            ("omega-room-voice-mute", "Mute"),
            ("omega-room-sarah-summon", "Summon Sarah"),
            ("omega-room-sarah-remove", "Remove Sarah"),
            ("omega-room-sarah-talk", "Talk to Sarah"),
            ("omega-room-sarah-stop", "Stop"),
        ] {
            probe.require_accessible(element_id, "Button", label)?;
            probe.require_accessibility_property(
                element_id,
                "disabled",
                serde_json::Value::Bool(true),
            )?;
        }
        Ok(())
    }

    if !workbench_any_selected(&[FIRST_LAUNCH_SCENE, RELAY_UNAVAILABLE_SCENE]) {
        return Ok(Vec::new());
    }

    anyhow::ensure!(
        omega_zero_base::is_sealed(),
        "tester-channel visual proof must run in the shipped sealed zero-base surface"
    );
    cx.set_debug_accessibility_active(workspace_window, true)?;

    let selected = cx.update_window(workspace_window, |_root, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.select_public_channel_for_tests("alpha-feedback", window, cx)
        })
    })?;
    anyhow::ensure!(
        selected,
        "the bundled alpha feedback destination was not selectable"
    );
    record_workbench_semantic_check(
        FIRST_LAUNCH_SCENE,
        "clean-profile-bundled-alpha-destination-selectable",
    );
    cx.run_until_parked();

    let set_snapshot = panel.update(cx, |panel, cx| {
        panel.set_selected_public_channel_snapshot_for_tests(
            ChannelSnapshot {
                relay_url: "wss://relay.openagents.com".to_string(),
                group_id: "openagents-public".to_string(),
                lifecycle: ChannelLifecycle::Current,
                cached: true,
                ..Default::default()
            },
            cx,
        )
    });
    anyhow::ensure!(set_snapshot, "the selected tester channel had no live view");
    cx.run_until_parked();

    let mut results = Vec::new();
    if workbench_any_selected(&[FIRST_LAUNCH_SCENE]) {
        let selected_snapshot = cx.debug_render_snapshot(workspace_window)?;
        let mut selected_probe = SemanticProbe::new(&selected_snapshot);
        selected_probe.require_visible("omega.public.channel.dock")?;
        selected_probe.require_visible("omega-tester-channel-composer")?;
        require_fail_closed_room(&mut selected_probe)?;
        selected_probe.require_absent("omega-tester-channel-relay-fallback")?;
        record_workbench_semantic_checks(FIRST_LAUNCH_SCENE, selected_probe.into_checks());
        record_workbench_semantic_check(FIRST_LAUNCH_SCENE, "public-privacy-composer-visible");
        results.push(run_visual_test(
            FIRST_LAUNCH_SCENE,
            workspace_window,
            cx,
            update_baseline,
        )?);
    }

    if workbench_any_selected(&[RELAY_UNAVAILABLE_SCENE]) {
        let set_snapshot = panel.update(cx, |panel, cx| {
            panel.set_selected_public_channel_snapshot_for_tests(
                ChannelSnapshot {
                    relay_url: "wss://relay.openagents.com".to_string(),
                    group_id: "openagents-public".to_string(),
                    lifecycle: ChannelLifecycle::Stale,
                    cached: true,
                    ..Default::default()
                },
                cx,
            )
        });
        anyhow::ensure!(set_snapshot, "the outage fixture lost its selected channel");
        cx.run_until_parked();

        let outage_snapshot = cx.debug_render_snapshot(workspace_window)?;
        let mut outage_probe = SemanticProbe::new(&outage_snapshot);
        outage_probe.require_visible("omega-tester-channel-relay-fallback")?;
        outage_probe.require_visible("omega-tester-channel-composer")?;
        require_fail_closed_room(&mut outage_probe)?;
        outage_probe.require_accessible(
            "omega-tester-channel-retry-relay",
            "Button",
            "Retry relay",
        )?;
        outage_probe.require_accessible(
            "omega-tester-channel-open-support",
            "Button",
            "Open support",
        )?;
        record_workbench_semantic_checks(RELAY_UNAVAILABLE_SCENE, outage_probe.into_checks());
        record_workbench_semantic_check(
            RELAY_UNAVAILABLE_SCENE,
            "relay-independent-support-path-visible",
        );
        results.push(run_visual_test(
            RELAY_UNAVAILABLE_SCENE,
            workspace_window,
            cx,
            update_baseline,
        )?);
    }

    panel.update(cx, |panel, cx| {
        panel.close_selected_public_channel_for_tests(cx);
    });
    cx.run_until_parked();
    cx.update_window(workspace_window, |_root, window, cx| {
        workspace.update(cx, |_workspace, cx| {
            AgentPanel::open_front_door(window, cx);
        });
    })?;
    cx.run_until_parked();
    Ok(results)
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_agent_visual_tests_inner(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    use agent_ui::AgentPanel;

    let capture_sealed_front_door = workbench_proof_active()
        && workbench_any_selected(&[
            "omega_front_door_no_project",
            "omega_front_door_typing",
            "omega_sarah_admission_ready",
            "omega_sarah_session_settled",
            "omega_tester_channel_first_launch",
            "omega_tester_channel_relay_unavailable",
        ]);
    if capture_sealed_front_door {
        // omega#161. The shipped launch is the one surface; nothing needs
        // entering any more. The sealed scenes still opt into the structural
        // seal themselves further down, exactly where the shipped startup
        // seals, and the proof command runs one selected scene per process so
        // no reset path is invented.
    } else {
        // These legacy baselines intentionally prove disclosure independently
        // of provider setup. OpenAgents now authenticates eagerly during test
        // initialization, so remove it from this provider-unavailable fixture
        // before the onboarding card snapshots the registry.
        cx.update(|cx| {
            LanguageModelRegistry::global(cx).update(cx, |registry, cx| {
                registry.unregister_provider(
                    LanguageModelProviderId::from("openagents".to_string()),
                    cx,
                );
            });
        });
    }

    // A project with **no worktree**. This is the whole point of the first two
    // captures: a window with nothing to restore is by definition a window with
    // no project, so this is the state a fresh install actually reaches.
    let project = cx.update(|cx| {
        project::Project::local(
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            app_state.user_store.clone(),
            app_state.languages.clone(),
            app_state.fs.clone(),
            None,
            project::LocalProjectFlags {
                init_worktree_trust: false,
                ..Default::default()
            },
            cx,
        )
    });

    let window_size = if workbench_any_selected(&[
        "omega_tester_channel_first_launch",
        "omega_tester_channel_relay_unavailable",
    ]) {
        size(px(1000.0), px(760.0))
    } else {
        size(px(900.0), px(720.0))
    };
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };
    let (workspace_window, workspace) = if capture_sealed_front_door {
        let workspace_window: WindowHandle<MultiWorkspace> = cx
            .update(|cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        focus: false,
                        show: false,
                        ..Default::default()
                    },
                    |window, cx| {
                        let workspace = cx.new(|cx| {
                            Workspace::new(None, project.clone(), app_state.clone(), window, cx)
                        });
                        cx.new(|cx| MultiWorkspace::new(workspace, window, cx))
                    },
                )
            })
            .context("Failed to open the sealed Omega front door window")?;
        let workspace = workspace_window
            .update(cx, |multi_workspace, _window, _cx| {
                multi_workspace.workspace().clone()
            })
            .context("Failed to read the zero-base workspace")?;
        (workspace_window.into(), workspace)
    } else {
        // Disclosure and route-pin scenes predate zero base and intentionally
        // keep their raw Workspace root. Wrapping them in MultiWorkspace adds
        // unrelated title/status chrome and invalidates those baselines.
        let workspace_window: WindowHandle<Workspace> = cx
            .update(|cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        focus: false,
                        show: false,
                        ..Default::default()
                    },
                    |window, cx| {
                        cx.new(|cx| {
                            Workspace::new(None, project.clone(), app_state.clone(), window, cx)
                        })
                    },
                )
            })
            .context("Failed to open the Omega disclosure window")?;
        let workspace = workspace_window
            .entity(cx)
            .context("Failed to read the disclosure workspace")?;
        (workspace_window.into(), workspace)
    };

    cx.run_until_parked();

    // The window really has no project. Asserted rather than assumed, because
    // every claim these captures make rests on it — a capture taken with a
    // worktree quietly present would prove the opposite of what it says.
    let visible_worktrees = cx.read(|cx| {
        workspace
            .read(cx)
            .project()
            .read(cx)
            .visible_worktrees(cx)
            .count()
    });
    anyhow::ensure!(
        visible_worktrees == 0,
        "the front door capture needs a projectless window; found {visible_worktrees} worktree(s)"
    );

    let (weak_workspace, async_window_cx) = cx
        .update_window(workspace_window, |_root, window, cx| {
            (workspace.downgrade(), window.to_async(cx))
        })
        .context("Failed to get workspace handle")?;

    cx.background_executor.allow_parking();
    if capture_sealed_front_door {
        cx.foreground_executor
            .block_test(agent_ui::initialize_workbench_panels(
                weak_workspace.clone(),
                async_window_cx.clone(),
            ))
            .context("Failed to initialize zero base's native workbench panels")?;
    }
    let panel = cx
        .foreground_executor
        .block_test(AgentPanel::load(weak_workspace, async_window_cx))
        .context("Failed to load AgentPanel")?;
    cx.background_executor.forbid_parking();

    cx.update_window(workspace_window, |_root, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.add_panel(panel.clone(), window, cx);
            workspace.open_panel::<AgentPanel>(window, cx);
            if capture_sealed_front_door {
                panel.update(cx, |panel, cx| {
                    use workspace::dock::Panel as _;
                    panel.set_zoomed(true, window, cx);
                });
            }
        });
    })
    .context("Failed to add the agent panel")?;

    cx.run_until_parked();

    // The shipped front door, called the way `crates/omega/src/main.rs` calls it
    // on a window with nothing to restore. Not a hand-rolled approximation of
    // it: `open_front_door` is the entry `OMEGA-DELTA-0019` added, and driving
    // anything else here would photograph a path no user takes.
    cx.update_window(workspace_window, |_root, window, cx| {
        workspace.update(cx, |_workspace, cx| {
            AgentPanel::open_front_door(window, cx);
        });
    })
    .context("Failed to open the front door")?;

    cx.run_until_parked();
    if capture_sealed_front_door {
        cx.update_window(workspace_window, |_root, _window, cx| {
            workspace.update(cx, |_workspace, cx| {
                omega_zero_base::seal();
                cx.notify();
            });
        })
        .context("Failed to seal the zero-base workspace")?;
        anyhow::ensure!(
            omega_zero_base::is_sealed(),
            "the front-door capture must use the structurally sealed zero-base surface"
        );
        cx.run_until_parked();
    }

    if capture_sealed_front_door {
        let center_visible = cx.read(|cx| workspace.read(cx).center_visible_for_tests());
        anyhow::ensure!(
            !center_visible,
            "the sealed front-door proof rendered the editor center beside the agent surface"
        );

        cx.set_debug_accessibility_active(workspace_window, true)?;
        // omega#172 only reproduced with the sealed workbench's real renderer:
        // open through the same action as the key binding, before the first
        // send, and prove the deferred popup stayed with its trigger.
        cx.update_window(workspace_window, |_root, window, cx| {
            window.dispatch_action(Box::new(agent_ui::ToggleComposerExecutorMenu), cx);
        })
        .context("Failed to open the pre-first-send executor menu")?;
        cx.run_until_parked();

        let snapshot = cx.debug_render_snapshot(workspace_window)?;
        let mut probe = SemanticProbe::new(&snapshot);
        // omega#165: the interstitial chooser is gone. The landing is the
        // focused composer with the executor dropdown as composer chrome.
        probe.require_absent("omega.new-conversation.front-door")?;
        probe.require_visible("omega.composer.executor-menu")?;
        probe.require_visible("omega.composer.executor-menu.popup")?;
        probe.require_absent("welcome-content")?;

        let trigger = snapshot
            .bounds("omega.composer.executor-menu")
            .context("the pre-first-send executor menu trigger has no bounds")?;
        let popup = snapshot
            .bounds("omega.composer.executor-menu.popup")
            .context("the pre-first-send executor menu popup has no bounds")?;
        let horizontal_gap = (popup.right() - trigger.right())
            .abs()
            .min((popup.left() - trigger.left()).abs());
        let vertical_gap = (popup.bottom() - trigger.top())
            .abs()
            .min((popup.top() - trigger.bottom()).abs());
        anyhow::ensure!(
            horizontal_gap <= px(64.) && vertical_gap <= px(64.),
            "the pre-first-send executor menu is not anchored to its trigger: \
             trigger {trigger:?}, popup {popup:?}"
        );

        cx.update_window(workspace_window, |_root, window, cx| {
            window.dispatch_action(Box::new(agent_ui::ToggleComposerExecutorMenu), cx);
        })
        .context("Failed to close the pre-first-send executor menu")?;
        cx.run_until_parked();
    }

    // The panel must actually be the focused, visible dock surface. A capture
    // taken with the panel absent shows the launchpad and would read as a
    // perfectly plausible screenshot of something else entirely — which is what
    // the first run of this test produced, and how it was caught.
    let panel_is_open = cx.read(|cx| {
        workspace
            .read(cx)
            .panel::<AgentPanel>(cx)
            .is_some_and(|panel| panel.read(cx).front_door_composer_visible_for_tests())
    });
    anyhow::ensure!(
        panel_is_open,
        "the agent panel is not the open surface; the capture would show the \
         launchpad rather than the composer front door"
    );

    // These scene names are reserved for the structurally sealed surface. The
    // generic suite still uses this setup for the disclosure scenes below, but
    // must not overwrite or compare the sealed baselines with the unsealed
    // layout.
    let front_door = if capture_sealed_front_door {
        anyhow::ensure!(
            omega_zero_base::is_sealed(),
            "omega_front_door_no_project cannot be captured outside sealed zero base"
        );
        run_visual_test(
            "omega_front_door_no_project",
            workspace_window,
            cx,
            update_baseline,
        )?
    } else {
        TestResult::Passed
    };
    let sarah_results = run_omega_sarah_admission_visual_tests(
        &panel,
        &workspace,
        workspace_window,
        cx,
        update_baseline,
    )?;
    let tester_channel_results = run_omega_tester_channel_visual_tests(
        &panel,
        &workspace,
        workspace_window,
        cx,
        update_baseline,
    )?;
    if !workbench_any_selected(&[
        "omega_front_door_typing",
        "omega_executor_disclosure_native",
        "omega_route_pin_honoured",
        "omega_route_pin_not_honoured",
        "omega_executor_disclosure_external_acp",
        "omega_executor_disclosure_engine_lane",
        "omega_executor_disclosure_external_acp_after_restart",
        "omega_executor_disclosure_engine_lane_after_restart",
    ]) {
        let mut results = vec![front_door];
        results.extend(sarah_results);
        results.extend(tester_channel_results);
        return finish_omega_agent_visual_tests(workspace_window, cx, &results);
    }

    if !sarah_results.is_empty() || !tester_channel_results.is_empty() {
        cx.update_window(workspace_window, |_root, window, cx| {
            workspace.update(cx, |_workspace, cx| {
                AgentPanel::open_front_door(window, cx);
            });
        })
        .context("Failed to restore the front door after Sarah admission proof")?;
        cx.run_until_parked();
    }

    let activated = cx
        .update_window(workspace_window, |_, window, cx| {
            panel.update(cx, |panel, cx| {
                panel.activate_prepared_omega_for_tests(window, cx)
            })
        })
        .context("Failed to activate the prepared Omega router")?;
    anyhow::ensure!(
        activated,
        "the front door did not reach Ready with a connected Omega router"
    );
    cx.run_until_parked();

    let thread_id = cx
        .read(|cx| panel.read(cx).active_thread_id(cx))
        .ok_or_else(|| {
            anyhow::anyhow!("activating Omega did not claim the prepared projectless conversation")
        })?;

    // The keystrokes go into this window through GPUI's own dispatch, so
    // nothing depends on which application macOS thinks is frontmost.
    cx.simulate_input(workspace_window, "route this thread on purpose");
    cx.run_until_parked();

    let typed = cx.read(|cx| {
        panel
            .read(cx)
            .active_thread_view_for_tests()
            .is_some_and(|conversation| {
                conversation.read(cx).has_unsubmitted_or_pending_content(cx)
            })
    });
    anyhow::ensure!(
        typed,
        "typing on the front door did not reach the logical router composer"
    );
    if capture_sealed_front_door {
        let snapshot = cx.debug_render_snapshot(workspace_window)?;
        let mut probe = SemanticProbe::new(&snapshot);
        probe.require_accessible("omega-composer-executor-trigger", "Button", "Omega Agent")?;
        record_workbench_semantic_checks("omega_front_door_typing", probe.into_checks());
    }

    let typing = if capture_sealed_front_door {
        anyhow::ensure!(
            omega_zero_base::is_sealed(),
            "omega_front_door_typing cannot be captured outside sealed zero base"
        );
        run_visual_test(
            "omega_front_door_typing",
            workspace_window,
            cx,
            update_baseline,
        )?
    } else {
        TestResult::Passed
    };
    if !workbench_any_selected(&[
        "omega_executor_disclosure_native",
        "omega_route_pin_honoured",
        "omega_route_pin_not_honoured",
        "omega_executor_disclosure_external_acp",
        "omega_executor_disclosure_engine_lane",
        "omega_executor_disclosure_external_acp_after_restart",
        "omega_executor_disclosure_engine_lane_after_restart",
    ]) {
        return finish_omega_agent_visual_tests(workspace_window, cx, &[front_door, typing]);
    }

    cx.update_window(workspace_window, |_, window, cx| {
        window.dispatch_action(Box::new(omega_actions::agent::Chat), cx);
    })
    .context("Failed to dispatch the projectless conversation's first turn")?;
    cx.run_until_parked();
    anyhow::ensure!(
        cx.read(|cx| {
            panel
                .read(cx)
                .active_thread_view_for_tests()
                .is_some_and(|conversation| conversation.read(cx).root_thread_view().is_some())
        }),
        "the first accepted turn did not create its routed physical executor session"
    );
    let routed_model_line = cx
        .read(|cx| omega_executor_line(&panel, cx))
        .ok_or_else(|| anyhow::anyhow!("the routed thread discloses no model"))?;
    cx.set_debug_accessibility_active(workspace_window, true)?;
    let snapshot = cx.debug_render_snapshot(workspace_window)?;
    let mut probe = SemanticProbe::new(&snapshot);
    probe.require_accessible("omega-routed-model", "Status", &routed_model_line)?;
    probe.require_absent("omega-new-conversation-route-override-trigger")?;
    record_workbench_semantic_checks("omega_executor_disclosure_native", probe.into_checks());

    // Zoom the panel for the disclosure captures. The executor line is long by
    // design — class, agent, model, run, and route reason — and a dock-width
    // capture truncates it with an ellipsis, which would make a picture of a
    // *truncated* line the evidence that the line renders.
    cx.update_window(workspace_window, |_, window, cx| {
        panel.update(cx, |panel, cx| {
            use workspace::dock::Panel as _;
            panel.set_zoomed(true, window, cx);
        });
    })
    .log_err();
    cx.run_until_parked();

    let mut native_disclosure = TestResult::Passed;
    if workbench_any_selected(&["omega_executor_disclosure_native"]) {
        // omega#77: the executor line on the native thread the front door just
        // built. A restored or pre-connection draft can legitimately predate
        // the router's session record, so this scene proves executor
        // attribution. The two pin scenes below create and assert explicit
        // route decisions.
        let native_line = cx
            .read(|cx| omega_executor_line(&panel, cx))
            .ok_or_else(|| anyhow::anyhow!("the native thread has no executor disclosure"))?;
        anyhow::ensure!(
            // omega#100. The line no longer leads with the wire token.
            // `ExecutorClass::token` documents that a token is never shown to a
            // user on its own, and the disclosure stopped rendering it. The
            // property each scene is here for is unchanged: the line names the
            // agent that ran the turn, and never reads as another executor.
            native_line.starts_with("Omega Agent") && !native_line.contains("external_acp"),
            "the front door's own thread must disclose Omega Agent, not {native_line:?}"
        );
        println!("  native executor line: {native_line}");

        native_disclosure = run_visual_test(
            "omega_executor_disclosure_native",
            workspace_window,
            cx,
            update_baseline,
        )?;
    }
    if !workbench_any_selected(&[
        "omega_route_pin_honoured",
        "omega_route_pin_not_honoured",
        "omega_executor_disclosure_external_acp",
        "omega_executor_disclosure_engine_lane",
        "omega_executor_disclosure_external_acp_after_restart",
        "omega_executor_disclosure_engine_lane_after_restart",
    ]) {
        return finish_omega_agent_visual_tests(
            workspace_window,
            cx,
            &[front_door, typing, native_disclosure],
        );
    }

    let mut pin_honoured = TestResult::Passed;
    if workbench_any_selected(&["omega_route_pin_honoured"]) {
        // omega#78, first exit clause: **a pinned executor is always
        // honoured.** The native loop is the one executor this build has
        // registered on the router.
        let session_for_pin = cx
            .read(|cx| {
                panel
                    .read(cx)
                    .active_thread_view_for_tests()
                    .and_then(|conversation| conversation.read(cx).root_thread_view())
                    .map(|view| view.read(cx).thread.read(cx).session_id().clone())
            })
            .ok_or_else(|| anyhow::anyhow!("no session to pin"))?;
        let router_for_pin = agent_ui::omega_router::active_router().ok_or_else(|| {
            anyhow::anyhow!(
                "no router was built for the native agent entry — omega#78's \
                 wiring is the thing under test and it is absent"
            )
        })?;
        let honoured = router_for_pin.pin_session(
            &session_for_pin,
            omega_front_door::ExecutorPin::new(omega_front_door::ExecutorClass::NativeLoop),
            omega_front_door::PinGesture::ExecutorPinMenuItem,
        );
        // "Route Omega requests to exact ready executors" moved human pins
        // onto the current override path, whose honoured reason is
        // `OverrideHonored` (its router unit test pins that exact reason).
        // `PinHonored` remains the honoured reason for the legacy pin path.
        // Either way the property under test is unchanged: a pin to the
        // fail-closed target is honoured, never a fallback.
        anyhow::ensure!(
            matches!(
                honoured.reason,
                omega_front_door::RouteReason::PinHonored
                    | omega_front_door::RouteReason::OverrideHonored
            ),
            "a pin to the fail-closed target must be honoured, not {:?}",
            honoured.reason
        );
        // The same landing made route receipts bind once: this session was
        // already routed by its first accepted turn, so the pin's re-decision
        // must NOT rewrite the receipt underneath the transcript
        // (`OMEGA-DELTA-0150`'s truthful-labeling law). The picture this scene
        // now proves is a bound thread whose disclosure survives a later pin.
        anyhow::ensure!(
            router_for_pin
                .recorded_decision(&session_for_pin)
                .is_some_and(|recorded| recorded.reason != honoured.reason),
            "a bound session's route receipt must survive a later pin unchanged"
        );
        cx.update_window(workspace_window, |_, window, _cx| {
            window.refresh();
        })?;
        cx.run_until_parked();

        let honoured_record = cx
            .read(|cx| omega_executor_record(&panel, cx))
            .ok_or_else(|| anyhow::anyhow!("the pinned thread has no executor disclosure"))?;
        // Bind-once receipts: the thread's typed disclosure keeps the executor
        // and route reason it was bound with, and a later pin to the same
        // fail-closed target neither rewrites it nor adds fallback noise.
        anyhow::ensure!(
            honoured_record.class == omega_front_door::ExecutorClass::NativeLoop
                && honoured_record.route.is_some(),
            "the bound thread must keep its typed native disclosure: {honoured_record:?}"
        );
        let honoured_line = honoured_record.label();
        anyhow::ensure!(
            !honoured_line.contains("routed:"),
            "an honoured pin is not a fallback and must not add route noise: {honoured_line:?}"
        );
        println!("  honoured-pin executor line: {honoured_line}");

        pin_honoured = run_visual_test(
            "omega_route_pin_honoured",
            workspace_window,
            cx,
            update_baseline,
        )?;
    }
    if !workbench_any_selected(&[
        "omega_route_pin_not_honoured",
        "omega_executor_disclosure_external_acp",
        "omega_executor_disclosure_engine_lane",
        "omega_executor_disclosure_external_acp_after_restart",
        "omega_executor_disclosure_engine_lane_after_restart",
    ]) {
        return finish_omega_agent_visual_tests(
            workspace_window,
            cx,
            &[front_door, typing, native_disclosure, pin_honoured],
        );
    }

    let mut pin_fallback = TestResult::Passed;
    if workbench_any_selected(&["omega_route_pin_not_honoured"]) {
        // omega#78: a pin that cannot be honoured, rendered. No engine is
        // running under this harness, so an engine-lane pin falls closed to the
        // native loop.
        let session_id = cx
            .read(|cx| {
                panel
                    .read(cx)
                    .active_thread_view_for_tests()
                    .and_then(|conversation| conversation.read(cx).root_thread_view())
                    .map(|view| view.read(cx).thread.read(cx).session_id().clone())
            })
            .ok_or_else(|| anyhow::anyhow!("no session to pin"))?;
        let router = agent_ui::omega_router::active_router().ok_or_else(|| {
            anyhow::anyhow!(
                "no router was built for the native agent entry — omega#78's \
                 wiring is the thing under test and it is absent"
            )
        })?;
        let decision = router.pin_session(
            &session_id,
            omega_front_door::ExecutorPin::new(omega_front_door::ExecutorClass::EngineLane),
            omega_front_door::PinGesture::ExecutorPinMenuItem,
        );
        anyhow::ensure!(
            decision.reason.is_fallback(),
            "an engine-lane pin with no engine must fall back, not report {:?}",
            decision.reason
        );

        cx.update_window(workspace_window, |_, window, _cx| {
            window.refresh();
        })?;
        cx.run_until_parked();

        let pinned_line = cx
            .read(|cx| omega_executor_line(&panel, cx))
            .ok_or_else(|| anyhow::anyhow!("the pinned thread has no executor disclosure"))?;
        // Bind-once receipts: the unhonoured engine pin answers with a typed
        // fallback (asserted above), and the bound thread's disclosure keeps
        // naming the executor that actually ran it — it neither retargets to
        // the engine nor gains fallback noise underneath the transcript.
        anyhow::ensure!(
            pinned_line.starts_with("Omega Agent") && !pinned_line.contains("engine"),
            "the bound thread must keep its native disclosure after an \
             unhonoured engine pin: {pinned_line:?}"
        );
        println!("  bound-thread executor line after engine pin: {pinned_line}");

        pin_fallback = run_visual_test(
            "omega_route_pin_not_honoured",
            workspace_window,
            cx,
            update_baseline,
        )?;
    }
    if !workbench_any_selected(&[
        "omega_executor_disclosure_external_acp",
        "omega_executor_disclosure_engine_lane",
        "omega_executor_disclosure_external_acp_after_restart",
        "omega_executor_disclosure_engine_lane_after_restart",
    ]) {
        return finish_omega_agent_visual_tests(
            workspace_window,
            cx,
            &[
                front_door,
                typing,
                native_disclosure,
                pin_honoured,
                pin_fallback,
            ],
        );
    }

    // omega#77: the external-ACP kind, on a second thread in the same panel.
    let stub: Rc<dyn AgentServer> = Rc::new(StubAgentServer::new(StubAgentConnection::new()));
    cx.update_window(workspace_window, |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.open_external_thread_with_server(stub.clone(), window, cx);
        });
    })?;
    cx.run_until_parked();

    let external_line = cx
        .read(|cx| omega_executor_line(&panel, cx))
        .ok_or_else(|| anyhow::anyhow!("the external thread has no executor disclosure"))?;
    anyhow::ensure!(
        // omega#100. The line no longer leads with the wire token.
        // `ExecutorClass::token` documents that a token is never shown to a
        // user on its own, and the disclosure stopped rendering it. The
        // property each scene is here for is unchanged: the line names the
        // agent that ran the turn, and never reads as another executor.
        !external_line.contains("native_loop") && !external_line.is_empty(),
        "a thread on a connection Omega did not build must not be disclosed as \
         first-party output: {external_line:?}"
    );
    println!("  external-acp executor line: {external_line}");

    let external_disclosure = run_visual_test(
        "omega_executor_disclosure_external_acp",
        workspace_window,
        cx,
        update_baseline,
    )?;
    if !workbench_any_selected(&[
        "omega_executor_disclosure_engine_lane",
        "omega_executor_disclosure_external_acp_after_restart",
        "omega_executor_disclosure_engine_lane_after_restart",
    ]) {
        return finish_omega_agent_visual_tests(
            workspace_window,
            cx,
            &[
                front_door,
                typing,
                native_disclosure,
                pin_honoured,
                pin_fallback,
                external_disclosure,
            ],
        );
    }

    // omega#77: the engine-lane kind, on that same external thread.
    //
    // Deliberately *this* thread and not the front door's own. A lane run is a
    // `codex-acp` process the host bridge drives, so the honest record is "this
    // run delegated to this agent" — and it is a thread the router did not
    // route, so its `route` part is absent. Publishing a lane run onto a
    // *routed* thread instead would produce a record `is_coherent` rejects: a
    // route reason that says the router fell back to the native loop, on a line
    // that claims an engine lane. The coherence assertions below are what
    // caught that while this test was being written.
    let external_thread_id = cx
        .read(|cx| panel.read(cx).active_thread_id(cx))
        .ok_or_else(|| anyhow::anyhow!("the external thread has no id"))?;
    anyhow::ensure!(
        external_thread_id != thread_id,
        "the external thread must be a second thread, not the front door's own"
    );
    agent_ui::omega_host_bridge::publish_engine_lane_run_for_tests(
        external_thread_id,
        "operation.full-auto.visual".to_string(),
    );
    cx.update_window(workspace_window, |_, window, _cx| {
        window.refresh();
    })?;
    cx.run_until_parked();

    let lane_line = cx
        .read(|cx| omega_executor_line(&panel, cx))
        .ok_or_else(|| anyhow::anyhow!("the lane thread has no executor disclosure"))?;
    anyhow::ensure!(
        // omega#100. The line no longer leads with the wire token.
        // `ExecutorClass::token` documents that a token is never shown to a
        // user on its own, and the disclosure stopped rendering it. The
        // property each scene is here for is unchanged: the line names the
        // agent that ran the turn, and never reads as another executor.
        !lane_line.contains("native_loop") && lane_line.contains("operation.full-auto.visual"),
        "a thread bound to a lane run must disclose the run: {lane_line:?}"
    );
    println!("  engine-lane executor line: {lane_line}");

    let lane_disclosure = run_visual_test(
        "omega_executor_disclosure_engine_lane",
        workspace_window,
        cx,
        update_baseline,
    )?;
    if !workbench_any_selected(&[
        "omega_executor_disclosure_external_acp_after_restart",
        "omega_executor_disclosure_engine_lane_after_restart",
    ]) {
        return finish_omega_agent_visual_tests(
            workspace_window,
            cx,
            &[
                front_door,
                typing,
                native_disclosure,
                pin_honoured,
                pin_fallback,
                external_disclosure,
                lane_disclosure,
            ],
        );
    }

    // omega#77's restart proof, first half: leave behind what a relaunch would
    // find, and then end. Everything below the captures writes; nothing below
    // them photographs, so no committed baseline can move because of it.
    //
    // A *second* external thread, because the two restart cases have to be
    // distinguishable. The correlation journal names the first thread and not
    // this one, so a cold process that confused them would disclose a lane run
    // on a thread that never had one — a failure a single-thread shape could
    // not see.
    let plain_stub: Rc<dyn AgentServer> = Rc::new(StubAgentServer::new(StubAgentConnection::new()));
    cx.update_window(workspace_window, |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.open_external_thread_with_server(plain_stub, window, cx);
        });
    })?;
    cx.run_until_parked();

    let plain_thread_id = cx
        .read(|cx| panel.read(cx).active_thread_id(cx))
        .ok_or_else(|| anyhow::anyhow!("the second external thread has no id"))?;
    anyhow::ensure!(
        plain_thread_id != external_thread_id,
        "the restart phase needs two distinct threads; both ids are {plain_thread_id:?}"
    );
    let plain_record = cx
        .read(|cx| omega_executor_record(&panel, cx))
        .ok_or_else(|| anyhow::anyhow!("the second external thread has no disclosure"))?;
    anyhow::ensure!(
        plain_record.class == omega_front_door::ExecutorClass::ExternalAcp
            && plain_record.run_ref.is_none(),
        "the second external thread must carry no lane run before the restart: {plain_record:?}"
    );

    // The durable half. `publish_engine_lane_run_for_tests` above wrote a
    // process-local index that a restart empties; this writes the correlation
    // journal itself, in the production schema, at the path the shipped startup
    // reads. The next process gets this and nothing else.
    agent_ui::omega_host_bridge::persist_engine_lane_run_for_tests(
        external_thread_id,
        "operation.full-auto.visual".to_string(),
    )?;

    write_restart_handoff(&RestartHandoff {
        lane_thread: external_thread_id,
        lane_line,
        external_thread: plain_thread_id,
        external_line: plain_record.label(),
        agent_id: plain_record.agent_id,
        operation_ref: "operation.full-auto.visual".to_string(),
    })?;

    // Seven captures, and `run_visual_test` returns `Err` on a mismatch, so
    // reaching here means every one of them matched its baseline or wrote one.
    // The aggregate reports "baseline updated" if any capture wrote one, so an
    // `UPDATE_BASELINE=1` run is never reported as a pass.
    finish_omega_agent_visual_tests(
        workspace_window,
        cx,
        &[
            front_door,
            typing,
            native_disclosure,
            pin_honoured,
            pin_fallback,
            external_disclosure,
            lane_disclosure,
        ],
    )
}

/// The executor line the agent panel's active thread would render.
///
/// Read through the same `ThreadView::executor_disclosure` the render calls,
/// and rendered through the same `omega_routed_model::chrome_line` authority,
/// so the assertion and the pixels cannot disagree about what the line says.
///
/// `OMEGA-DELTA-0202`, amended by `OMEGA-DELTA-0208`. There were two helpers
/// here, one per line, because the composer drew two: the record's own line and
/// the model's name beneath it. 0208 folded those into one — they were the same
/// fact said twice — and the two helpers folded with them. The record's own
/// line, which names the model by its `provider/model` wire pair, is what a
/// receipt renders and is asserted through `omega_executor_record` where a
/// scene needs the exact pair.
///
/// The status line used to reconstruct the route receipt:
/// `Receipt 0 · Route: … · override: … · fallback: …`. That exposition is gone
/// from the composer, so the proof no longer photographs it.
#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn omega_executor_line(panel: &Entity<agent_ui::AgentPanel>, cx: &App) -> Option<String> {
    Some(agent_ui::omega_routed_model::chrome_line(
        &omega_executor_record(panel, cx)?,
    ))
}

/// The executor *record* the agent panel's active thread would render from.
///
/// The record rather than the line, because omega#77's condition is that
/// disclosure is a typed record a label renders. A restart proof that compared
/// only rendered strings could not tell a restored record from a restored
/// string, which is the distinction the condition exists to protect.
#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn omega_executor_record(
    panel: &Entity<agent_ui::AgentPanel>,
    cx: &App,
) -> Option<omega_front_door::ExecutorDisclosure> {
    let disclosure = panel
        .read(cx)
        .active_thread_view_for_tests()?
        .read(cx)
        .root_thread_view()?
        .read(cx)
        .executor_disclosure(cx);
    // Every captured line is asserted coherent, not merely non-empty. A record
    // can render a perfectly readable line while claiming two contradictory
    // things — an engine lane and a route reason that says the router fell back
    // to the native loop is exactly that, and it is what an earlier draft of
    // this test was about to photograph and call proof.
    assert!(
        disclosure.is_coherent(),
        "the rendered executor record is incoherent: {disclosure:?}"
    );
    Some(disclosure)
}

/// What the recording process leaves for the restarted one. omega#77.
///
/// The two thread ids and the agent id are the identifiers a real relaunch
/// reads back out of `sidebar_threads`; the runner sets `ZED_STATELESS=1`, which
/// deliberately keeps that table in memory, so the harness carries them in this
/// file instead. The lane run is **not** here: it goes through the production
/// correlation journal, at the production path, and the restarted process reads
/// it with the production loader. The lines are here so the restarted process
/// can assert it rendered *the same disclosure*, not merely a plausible one.
#[cfg(all(target_os = "macos", feature = "visual-tests"))]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestartHandoff {
    lane_thread: agent_ui::ThreadId,
    lane_line: String,
    external_thread: agent_ui::ThreadId,
    external_line: String,
    agent_id: String,
    operation_ref: String,
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn restart_handoff_path() -> std::path::PathBuf {
    paths::data_dir().join("omega-visual-restart-handoff.json")
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn write_restart_handoff(handoff: &RestartHandoff) -> Result<()> {
    let path = restart_handoff_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_vec_pretty(handoff)?)
        .with_context(|| format!("writing the restart handoff to {}", path.display()))?;
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn read_restart_handoff() -> Result<RestartHandoff> {
    let path = restart_handoff_path();
    let bytes = std::fs::read(&path).with_context(|| {
        format!(
            "no restart handoff at {} — the restart phase must run in the same \
             data directory as the phase that recorded it, and after it",
            path.display()
        )
    })?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// omega#77: the executor line on a `codex-acp`-class thread and on an
/// engine-lane thread, **after a restart**.
///
/// This runs in a second process. Every process-local thing the first process
/// built is gone — the lane index, the router's recorded routes, the panel, the
/// threads themselves — so anything this renders came from disk or was never
/// durable in the first place. That is the whole point: the previous lane
/// demonstrated a real relaunch for the native kind and could not reach these
/// two, and "the mechanism is shared, so I expect them to hold" is not the
/// standard the issue set.
#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_restart_visual_tests(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    let outcome = run_omega_restart_visual_tests_inner(app_state, cx, update_baseline);
    cx.run_until_parked();
    outcome
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_restart_visual_tests_inner(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    use agent_ui::AgentPanel;

    let handoff = read_restart_handoff()?;

    // A cold process knows nothing until it reads the journal. Asserted before
    // the read, because the read is what is under test: if a static had somehow
    // survived, or the harness had published the run itself, every assertion
    // below would pass for the wrong reason and the pictures would be of a
    // process that never restarted anything.
    anyhow::ensure!(
        agent_ui::omega_host_bridge::engine_lane_run(handoff.lane_thread).is_none(),
        "this process already knows a lane run for {:?} before reading the \
         journal, so it is not a cold start",
        handoff.lane_thread
    );

    // The production restart edge — the same function `omega_effectd_host_handler`
    // calls at startup, not a copy of it.
    let restored = agent_ui::omega_host_bridge::reload_engine_lane_runs_from_disk()?;
    anyhow::ensure!(
        restored == 1,
        "the correlation journal named {restored} lane-bound thread(s); the \
         recording phase wrote exactly one"
    );
    let restored_run = agent_ui::omega_host_bridge::engine_lane_run(handoff.lane_thread)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "the reloaded journal does not name {:?}, so nothing survived \
                 the restart",
                handoff.lane_thread
            )
        })?;
    anyhow::ensure!(
        restored_run == handoff.operation_ref,
        "the reloaded run is {restored_run:?}, not {:?}",
        handoff.operation_ref
    );
    anyhow::ensure!(
        agent_ui::omega_host_bridge::engine_lane_run(handoff.external_thread).is_none(),
        "the journal named the plain external thread as a lane run; a restart \
         must not invent an engine lane for a thread the user started"
    );

    let project = cx.update(|cx| {
        project::Project::local(
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            app_state.user_store.clone(),
            app_state.languages.clone(),
            app_state.fs.clone(),
            None,
            project::LocalProjectFlags {
                init_worktree_trust: false,
                ..Default::default()
            },
            cx,
        )
    });

    let window_size = size(px(900.0), px(720.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };
    let workspace_window: WindowHandle<Workspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        Workspace::new(None, project.clone(), app_state.clone(), window, cx)
                    })
                },
            )
        })
        .context("Failed to open the restarted window")?;

    cx.run_until_parked();

    let (weak_workspace, async_window_cx) = workspace_window
        .update(cx, |workspace, window, cx| {
            (workspace.weak_handle(), window.to_async(cx))
        })
        .context("Failed to get workspace handle")?;

    cx.background_executor.allow_parking();
    let panel = cx
        .foreground_executor
        .block_test(AgentPanel::load(weak_workspace, async_window_cx))
        .context("Failed to load AgentPanel")?;
    cx.background_executor.forbid_parking();

    workspace_window
        .update(cx, |workspace, window, cx| {
            workspace.add_panel(panel.clone(), window, cx);
            workspace.open_panel::<AgentPanel>(window, cx);
        })
        .context("Failed to add the agent panel")?;

    cx.run_until_parked();

    // Zoomed for the same reason the recording phase zooms: the line is long by
    // design, and a picture of a truncated line is not a picture of the line.
    cx.update_window(workspace_window.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            use workspace::dock::Panel as _;
            panel.set_zoomed(true, window, cx);
        });
    })
    .log_err();
    cx.run_until_parked();

    // The `codex-acp`-class thread, reopened under the id and agent id the
    // previous process left behind — the two things `restore_new_draft` reads
    // out of the metadata store on a real relaunch.
    let agent_id = AgentId::new(handoff.agent_id.clone());
    let external_stub: Rc<dyn AgentServer> = Rc::new(StubAgentServer::new(
        StubAgentConnection::new().with_agent_id(agent_id.clone()),
    ));
    cx.update_window(workspace_window.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.open_external_thread_with_server_under_id(
                external_stub,
                handoff.external_thread,
                window,
                cx,
            );
        });
    })?;
    cx.run_until_parked();

    let reopened_external = cx
        .read(|cx| panel.read(cx).active_thread_id(cx))
        .ok_or_else(|| anyhow::anyhow!("the reopened external thread has no id"))?;
    anyhow::ensure!(
        reopened_external == handoff.external_thread,
        "the reopened thread is {reopened_external:?}, not the persisted \
         {:?} — a new thread wearing the same content proves nothing about a \
         restart",
        handoff.external_thread
    );

    let external_line = cx
        .read(|cx| omega_executor_line(&panel, cx))
        .ok_or_else(|| anyhow::anyhow!("the reopened external thread has no disclosure"))?;
    anyhow::ensure!(
        external_line == handoff.external_line,
        "the restarted process discloses {external_line:?}, where the process \
         that owned this thread disclosed {:?}",
        handoff.external_line
    );
    anyhow::ensure!(
        // omega#100. The line no longer leads with the wire token.
        // `ExecutorClass::token` documents that a token is never shown to a
        // user on its own, and the disclosure stopped rendering it. The
        // property each scene is here for is unchanged: the line names the
        // agent that ran the turn, and never reads as another executor.
        !external_line.contains("native_loop") && !external_line.is_empty(),
        "a thread on a connection Omega did not build must not be disclosed as \
         first-party output after a restart either: {external_line:?}"
    );
    println!("  external-acp executor line after restart: {external_line}");

    let external_after_restart = run_visual_test(
        "omega_executor_disclosure_external_acp_after_restart",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // The engine-lane thread. Same reopen, different id — and this one the
    // journal on disk names, so the line has to carry the run.
    let lane_stub: Rc<dyn AgentServer> = Rc::new(StubAgentServer::new(
        StubAgentConnection::new().with_agent_id(agent_id),
    ));
    cx.update_window(workspace_window.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.open_external_thread_with_server_under_id(
                lane_stub,
                handoff.lane_thread,
                window,
                cx,
            );
        });
    })?;
    cx.run_until_parked();

    let reopened_lane = cx
        .read(|cx| panel.read(cx).active_thread_id(cx))
        .ok_or_else(|| anyhow::anyhow!("the reopened lane thread has no id"))?;
    anyhow::ensure!(
        reopened_lane == handoff.lane_thread,
        "the reopened lane thread is {reopened_lane:?}, not the persisted {:?}",
        handoff.lane_thread
    );

    let lane_record = cx
        .read(|cx| omega_executor_record(&panel, cx))
        .ok_or_else(|| anyhow::anyhow!("the reopened lane thread has no disclosure"))?;
    anyhow::ensure!(
        lane_record.run_ref.as_deref() == Some(handoff.operation_ref.as_str()),
        "the restored record names run {:?}, not {:?}",
        lane_record.run_ref,
        handoff.operation_ref
    );
    let lane_line = lane_record.label();
    anyhow::ensure!(
        lane_line == handoff.lane_line,
        "the restarted process discloses {lane_line:?}, where the process that \
         owned this thread disclosed {:?}",
        handoff.lane_line
    );
    println!("  engine-lane executor line after restart: {lane_line}");

    let lane_after_restart = run_visual_test(
        "omega_executor_disclosure_engine_lane_after_restart",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();
    cx.run_until_parked();

    for result in [&external_after_restart, &lane_after_restart] {
        if let TestResult::BaselineUpdated(path) = result {
            return Ok(TestResult::BaselineUpdated(path.clone()));
        }
    }
    Ok(TestResult::Passed)
}

/// A stub AgentServer for visual testing that returns a pre-programmed connection.
#[derive(Clone)]
#[cfg(target_os = "macos")]
struct StubAgentServer {
    connection: StubAgentConnection,
    agent_id: AgentId,
}

#[cfg(target_os = "macos")]
impl StubAgentServer {
    fn new(connection: StubAgentConnection) -> Self {
        Self {
            connection,
            agent_id: "Visual Test Agent".into(),
        }
    }

    fn with_agent_id(connection: StubAgentConnection, agent_id: AgentId) -> Self {
        Self {
            connection,
            agent_id,
        }
    }
}

#[cfg(target_os = "macos")]
impl AgentServer for StubAgentServer {
    fn logo(&self) -> ui::IconName {
        ui::IconName::OmegaAssistant
    }

    fn agent_id(&self) -> AgentId {
        self.agent_id.clone()
    }

    fn connect(
        &self,
        _delegate: AgentServerDelegate,
        _project: Entity<Project>,
        _cx: &mut App,
    ) -> gpui::Task<gpui::Result<Rc<dyn AgentConnection>>> {
        gpui::Task::ready(Ok(Rc::new(self.connection.clone())))
    }

    fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
        self
    }
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn create_concurrent_agent_project(
    app_state: &Arc<AppState>,
    worktrees: &[PathBuf],
    cx: &mut VisualTestAppContext,
) -> Result<Entity<Project>> {
    for worktree in worktrees {
        std::fs::create_dir_all(worktree)
            .with_context(|| format!("creating visual worktree {}", worktree.display()))?;
    }
    let project = cx.update(|cx| {
        Project::local(
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            app_state.user_store.clone(),
            app_state.languages.clone(),
            app_state.fs.clone(),
            None,
            project::LocalProjectFlags {
                init_worktree_trust: false,
                ..Default::default()
            },
            cx,
        )
    });
    for worktree in worktrees {
        let add_worktree = project.update(cx, |project, cx| {
            project.find_or_create_worktree(worktree, true, cx)
        });
        cx.background_executor.allow_parking();
        cx.foreground_executor
            .block_test(add_worktree)
            .with_context(|| format!("adding visual worktree {}", worktree.display()))?;
        cx.background_executor.forbid_parking();
    }
    cx.run_until_parked();
    Ok(project)
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn open_concurrent_agent_panel(
    app_state: Arc<AppState>,
    project: Entity<Project>,
    cx: &mut VisualTestAppContext,
) -> Result<(AnyWindowHandle, Entity<Workspace>, Entity<AgentPanel>)> {
    let bounds = Bounds {
        origin: point(px(0.), px(0.)),
        size: size(px(1180.), px(760.)),
    };
    let workspace_window: WindowHandle<Workspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Workspace::new(None, project, app_state, window, cx)),
            )
        })
        .context("opening concurrent-agent visual window")?;
    cx.run_until_parked();

    let workspace = workspace_window
        .entity(cx)
        .context("reading concurrent-agent workspace")?;
    let (weak_workspace, async_window_cx) = workspace_window
        .update(cx, |workspace, window, cx| {
            (workspace.weak_handle(), window.to_async(cx))
        })
        .context("reading concurrent-agent workspace handles")?;
    cx.background_executor.allow_parking();
    let panel = cx
        .foreground_executor
        .block_test(AgentPanel::load(weak_workspace, async_window_cx))
        .context("loading concurrent-agent panel")?;
    cx.background_executor.forbid_parking();
    workspace_window
        .update(cx, |workspace, window, cx| {
            workspace.add_panel(panel.clone(), window, cx);
            workspace.open_panel::<AgentPanel>(window, cx);
            panel.update(cx, |panel, cx| {
                use workspace::dock::Panel as _;
                panel.enable_workbench_shell_for_tests(cx);
                panel.set_zoomed(true, window, cx);
                if !panel.threads_sidebar_open_for_tests() {
                    panel.toggle_threads_sidebar(cx);
                }
            });
        })
        .context("opening concurrent-agent panel")?;
    cx.run_until_parked();
    Ok((workspace_window.into(), workspace, panel))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn open_concurrent_direct_thread(
    panel: &Entity<AgentPanel>,
    workspace_window: AnyWindowHandle,
    connection: StubAgentConnection,
    agent_id: &'static str,
    title: &'static str,
    worktree: &Path,
    cx: &mut VisualTestAppContext,
) -> Result<(
    agent_ui::ThreadId,
    acp::SessionId,
    Entity<acp_thread::AcpThread>,
)> {
    let server: Rc<dyn AgentServer> = Rc::new(StubAgentServer::with_agent_id(
        connection,
        AgentId::new(agent_id),
    ));
    cx.update_window(workspace_window, |_root, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.open_external_thread_with_server_and_work_dirs(
                server,
                workspace::PathList::new(&[worktree]),
                window,
                cx,
            );
        });
    })?;
    cx.run_until_parked();
    let (thread_id, thread) = cx
        .read(|cx| {
            let panel = panel.read(cx);
            Some((panel.active_thread_id(cx)?, panel.active_agent_thread(cx)?))
        })
        .context("the visual Direct Agent thread did not connect")?;
    let session_id = thread.read_with(cx, |thread, _cx| thread.session_id().clone());
    let set_title = thread.update(cx, |thread, cx| thread.set_title(title.into(), cx));
    cx.background_executor.allow_parking();
    cx.foreground_executor
        .block_test(set_title)
        .context("setting visual Direct Agent thread title")?;
    cx.background_executor.forbid_parking();
    cx.run_until_parked();
    Ok((thread_id, session_id, thread))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn send_concurrent_visual_prompt(
    panel: &Entity<AgentPanel>,
    workspace_window: AnyWindowHandle,
    text: &str,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    let conversation = cx
        .read(|cx| {
            let thread_id = panel.read(cx).active_thread_id(cx)?;
            panel
                .read(cx)
                .conversation_view_for_id(&thread_id, cx)
                .cloned()
        })
        .context("the active concurrent-agent composer is unavailable")?;
    cx.update_window(workspace_window, |_root, window, cx| {
        conversation.update(cx, |conversation, cx| {
            conversation.set_composer_text_for_tests(text, window, cx);
            conversation.send_for_tests(window, cx);
        });
    })?;
    cx.run_until_parked();
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn switch_concurrent_visual_thread(
    workspace_window: AnyWindowHandle,
    thread_id: agent_ui::ThreadId,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    cx.simulate_click_selector(
        workspace_window,
        &format!("omega.threads.row.{}", thread_id.to_key_string()),
    )?;
    cx.run_until_parked();
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn concurrent_thread_lifecycle(
    panel: &Entity<AgentPanel>,
    thread_id: agent_ui::ThreadId,
    cx: &App,
) -> Result<agent_ui::omega_agent_supervision::SupervisedThreadLifecycle> {
    let conversation = panel
        .read(cx)
        .conversation_view_for_id(&thread_id, cx)
        .cloned()
        .context("concurrent thread view is not retained")?;
    let thread = conversation
        .read(cx)
        .active_thread()
        .map(|thread_view| thread_view.read(cx).thread.clone())
        .context("concurrent thread has no root session")?;
    Ok(agent_ui::omega_agent_supervision::lifecycle_for_thread(
        thread.read(cx),
    ))
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn capture_concurrent_agent_scene(
    scene_name: &'static str,
    workspace_window: AnyWindowHandle,
    panel: &Entity<AgentPanel>,
    active_thread_id: agent_ui::ThreadId,
    codex_thread_id: agent_ui::ThreadId,
    codex_title: &str,
    codex_status: agent_ui::omega_agent_supervision::SupervisedThreadLifecycle,
    claude_thread_id: agent_ui::ThreadId,
    claude_title: &str,
    claude_status: agent_ui::omega_agent_supervision::SupervisedThreadLifecycle,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    anyhow::ensure!(
        cx.read(|cx| panel.read(cx).active_thread_id(cx)) == Some(active_thread_id),
        "{scene_name} has the wrong active thread"
    );
    anyhow::ensure!(
        cx.read(|cx| concurrent_thread_lifecycle(panel, codex_thread_id, cx))? == codex_status,
        "{scene_name} has the wrong Codex lifecycle"
    );
    anyhow::ensure!(
        cx.read(|cx| concurrent_thread_lifecycle(panel, claude_thread_id, cx))? == claude_status,
        "{scene_name} has the wrong Claude lifecycle"
    );

    cx.update_window(workspace_window, |_root, window, _cx| window.refresh())?;
    cx.run_until_parked();
    cx.set_debug_accessibility_active(workspace_window, true)?;
    let snapshot = cx.debug_render_snapshot(workspace_window)?;
    let mut probe = SemanticProbe::new(&snapshot);
    probe.require_visible("omega-sidebar")?;
    probe.require_visible("omega.thread.supervision")?;
    for (thread_id, title, executor, lifecycle) in [
        (codex_thread_id, codex_title, "Codex", codex_status),
        (claude_thread_id, claude_title, "Claude", claude_status),
    ] {
        let thread_key = thread_id.to_key_string();
        probe.require_accessible(
            &format!("omega.threads.row.{thread_key}"),
            "Button",
            &format!("{title}, executor {executor}, status {}", lifecycle.label()),
        )?;
        probe.require_accessible(
            &format!("omega.threads.lifecycle.{thread_key}"),
            "Status",
            &format!("Thread status: {}", lifecycle.label()),
        )?;
    }
    let active_key = active_thread_id.to_key_string();
    let active_lifecycle = if active_thread_id == codex_thread_id {
        codex_status
    } else {
        claude_status
    };
    probe.require_accessible(
        &format!("omega.thread.header.lifecycle.{active_key}"),
        "Status",
        &format!("Thread status: {}", active_lifecycle.label()),
    )?;
    if matches!(
        active_lifecycle,
        agent_ui::omega_agent_supervision::SupervisedThreadLifecycle::Running
            | agent_ui::omega_agent_supervision::SupervisedThreadLifecycle::WaitingForPerson
    ) {
        probe.require_accessible(
            &format!("omega.thread.cancel.{active_key}"),
            "Button",
            "Cancel this agent run",
        )?;
    }
    record_workbench_semantic_checks(scene_name, probe.into_checks());
    record_workbench_semantic_check(scene_name, "two-direct-agent-identities-asserted");
    run_visual_test(scene_name, workspace_window, cx, update_baseline)
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn finish_concurrent_agent_visual_tests(
    workspace_window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    panel: &Entity<AgentPanel>,
    threads: &[Entity<acp_thread::AcpThread>],
    cx: &mut VisualTestAppContext,
    results: &[TestResult],
) -> Result<TestResult> {
    for thread in threads {
        let cancel_task = thread.update(cx, |thread, cx| thread.cancel(cx));
        drop(cancel_task);
    }

    panel.update(cx, |panel, cx| {
        panel.connection_store().update(cx, |store, cx| {
            for agent_id in [agent_servers::CODEX_ID, agent_servers::CLAUDE_AGENT_ID] {
                store.restart_connection(
                    Agent::Custom {
                        id: AgentId::new(agent_id),
                    },
                    Rc::new(StubAgentServer::new(StubAgentConnection::new())),
                    cx,
                );
            }
        });
    });
    cx.run_until_parked();

    cx.update_window(workspace_window, |_root, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.remove_panel(panel, window, cx);
            let project = workspace.project().clone();
            project.update(cx, |project, cx| {
                let worktree_ids: Vec<_> = project
                    .worktrees(cx)
                    .map(|worktree| worktree.read(cx).id())
                    .collect();
                for worktree_id in worktree_ids {
                    project.remove_worktree(worktree_id, cx);
                }
            });
        });
    })
    .log_err();
    cx.run_until_parked();
    let result = finish_omega_agent_visual_tests(workspace_window, cx, results);
    for _ in 0..15 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }
    result
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_concurrent_agent_visual_tests(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    use agent_ui::omega_agent_supervision::SupervisedThreadLifecycle;

    const CODEX_WAITING: &str = "omega_concurrent_agents_codex_waiting";
    const CLAUDE_RUNNING: &str = "omega_concurrent_agents_claude_running";
    const CANCEL_ISOLATED: &str = "omega_concurrent_agents_cancel_isolated";
    // `OMEGA-DELTA-0214`. The scene used to photograph the collision modal.
    // The modal is gone, so the name would lie; what is photographed now is
    // the property that replaced it — a second thread starting in an occupied
    // worktree with no dialog in the way.
    const WORKTREE_NO_DIALOG: &str = "omega_concurrent_agents_worktree_no_dialog";
    const CODEX_TITLE: &str = "Codex indexes the migration";
    const CLAUDE_TITLE: &str = "Claude audits the release";
    const CODEX_HISTORY: &str = "Codex history sentinel: inspect the migration.";
    const CLAUDE_HISTORY: &str = "Claude history sentinel: audit the release.";
    const CODEX_QUEUED: &str = "Codex queued sentinel: verify the schema next.";
    const CLAUDE_QUEUED: &str = "Claude queued sentinel: run the release checks next.";

    cx.update(|cx| {
        agent_ui::omega_send_queue::SendQueueJournal::set_global_for_tests(
            Rc::new(agent_ui::omega_send_queue::SendQueueJournal::at_data_dir()),
            cx,
        );
    });

    let fixture_root = paths::data_dir().join("omega-concurrent-agent-visual");
    let codex_worktree = fixture_root.join("codex-worktree");
    let claude_worktree = fixture_root.join("claude-worktree");
    let project = create_concurrent_agent_project(
        &app_state,
        &[codex_worktree.clone(), claude_worktree.clone()],
        cx,
    )?;
    let (workspace_window, workspace, panel) = open_concurrent_agent_panel(app_state, project, cx)?;

    let codex_connection = StubAgentConnection::new()
        .with_agent_id(AgentId::new(agent_servers::CODEX_ID))
        .with_telemetry_id("Codex".into());
    let (codex_thread_id, _codex_session_id, codex_thread) = open_concurrent_direct_thread(
        &panel,
        workspace_window,
        codex_connection.clone(),
        agent_servers::CODEX_ID,
        CODEX_TITLE,
        &codex_worktree,
        cx,
    )?;
    send_concurrent_visual_prompt(&panel, workspace_window, CODEX_HISTORY, cx)?;

    let permission_task = codex_thread.update(cx, |thread, cx| {
        thread.request_tool_call_authorization(
            acp::ToolCall::new("omega-concurrent-codex-confirm", "Apply the migration")
                .kind(acp::ToolKind::Edit)
                .status(acp::ToolCallStatus::Pending)
                .into(),
            PermissionOptions::Flat(vec![
                acp::PermissionOption::new(
                    "allow-once",
                    "Allow once",
                    acp::PermissionOptionKind::AllowOnce,
                ),
                acp::PermissionOption::new(
                    "reject-once",
                    "Reject",
                    acp::PermissionOptionKind::RejectOnce,
                ),
            ]),
            AuthorizationKind::PermissionGrant,
            cx,
        )
    })?;
    permission_task.detach();
    cx.run_until_parked();

    let claude_connection = StubAgentConnection::new()
        .with_agent_id(AgentId::new(agent_servers::CLAUDE_AGENT_ID))
        .with_telemetry_id("Claude".into());
    let (claude_thread_id, claude_session_id, claude_thread) = open_concurrent_direct_thread(
        &panel,
        workspace_window,
        claude_connection.clone(),
        agent_servers::CLAUDE_AGENT_ID,
        CLAUDE_TITLE,
        &claude_worktree,
        cx,
    )?;
    send_concurrent_visual_prompt(&panel, workspace_window, CLAUDE_HISTORY, cx)?;
    cx.update(|cx| {
        claude_connection.send_update(
            claude_session_id.clone(),
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                "Claude is still running the independent release audit.".into(),
            )),
            cx,
        );
    });
    cx.run_until_parked();

    send_concurrent_visual_prompt(&panel, workspace_window, CLAUDE_QUEUED, cx)?;
    switch_concurrent_visual_thread(workspace_window, codex_thread_id, cx)?;
    send_concurrent_visual_prompt(&panel, workspace_window, CODEX_QUEUED, cx)?;

    let (codex_queue, claude_queue) = cx.update(|cx| {
        let journal = agent_ui::omega_send_queue::SendQueueJournal::global(cx);
        (
            journal.open_items(&codex_thread_id.to_key_string()),
            journal.open_items(&claude_thread_id.to_key_string()),
        )
    });
    anyhow::ensure!(
        codex_queue.len() == 1 && codex_queue[0].text == CODEX_QUEUED,
        "Codex queue did not retain its independent item: {codex_queue:?}"
    );
    anyhow::ensure!(
        claude_queue.len() == 1 && claude_queue[0].text == CLAUDE_QUEUED,
        "Claude queue did not retain its independent item: {claude_queue:?}"
    );
    cx.update(|cx| -> Result<()> {
        let journal = agent_ui::omega_send_queue::SendQueueJournal::global(cx);
        journal
            .set_processing_state(
                &codex_thread_id.to_key_string(),
                agent_ui::omega_send_queue::SendQueueProcessingState::Paused,
            )
            .map_err(|error| anyhow::anyhow!("pausing Codex queue failed: {error:?}"))?;
        journal
            .set_processing_state(
                &claude_thread_id.to_key_string(),
                agent_ui::omega_send_queue::SendQueueProcessingState::Paused,
            )
            .map_err(|error| anyhow::anyhow!("pausing Claude queue failed: {error:?}"))?;
        Ok(())
    })?;

    let mut results = Vec::new();
    if workbench_any_selected(&[CODEX_WAITING]) {
        let capture = capture_concurrent_agent_scene(
            CODEX_WAITING,
            workspace_window,
            &panel,
            codex_thread_id,
            codex_thread_id,
            CODEX_TITLE,
            SupervisedThreadLifecycle::WaitingForPerson,
            claude_thread_id,
            CLAUDE_TITLE,
            SupervisedThreadLifecycle::Running,
            cx,
            update_baseline,
        );
        match capture {
            Ok(capture) => results.push(capture),
            Err(error) => {
                finish_concurrent_agent_visual_tests(
                    workspace_window,
                    &workspace,
                    &panel,
                    &[codex_thread.clone(), claude_thread],
                    cx,
                    &[],
                )
                .log_err();
                return Err(error);
            }
        }
        record_workbench_semantic_check(CODEX_WAITING, "per-thread-queued-input-isolated");
    }

    switch_concurrent_visual_thread(workspace_window, claude_thread_id, cx)?;
    if workbench_any_selected(&[CLAUDE_RUNNING]) {
        results.push(capture_concurrent_agent_scene(
            CLAUDE_RUNNING,
            workspace_window,
            &panel,
            claude_thread_id,
            codex_thread_id,
            CODEX_TITLE,
            SupervisedThreadLifecycle::WaitingForPerson,
            claude_thread_id,
            CLAUDE_TITLE,
            SupervisedThreadLifecycle::Running,
            cx,
            update_baseline,
        )?);
        record_workbench_semantic_check(CLAUDE_RUNNING, "sidebar-switch-preserved-both-turns");
    }

    switch_concurrent_visual_thread(workspace_window, codex_thread_id, cx)?;
    cx.simulate_click_selector(workspace_window, "omega.thread.cancel")?;
    cx.run_until_parked();
    switch_concurrent_visual_thread(workspace_window, claude_thread_id, cx)?;
    anyhow::ensure!(
        codex_thread.read_with(cx, |thread, _cx| thread.terminal_status())
            == ThreadTerminalStatus::Cancelled,
        "cancelling Codex did not produce a cancelled terminal status"
    );
    anyhow::ensure!(
        cx.read(|cx| concurrent_thread_lifecycle(&panel, claude_thread_id, cx))?
            == SupervisedThreadLifecycle::Running,
        "cancelling Codex changed Claude's running lifecycle"
    );
    if workbench_any_selected(&[CANCEL_ISOLATED]) {
        results.push(capture_concurrent_agent_scene(
            CANCEL_ISOLATED,
            workspace_window,
            &panel,
            claude_thread_id,
            codex_thread_id,
            CODEX_TITLE,
            SupervisedThreadLifecycle::Cancelled,
            claude_thread_id,
            CLAUDE_TITLE,
            SupervisedThreadLifecycle::Running,
            cx,
            update_baseline,
        )?);
        record_workbench_semantic_check(CANCEL_ISOLATED, "cancel-one-left-other-running");
    }

    let mut collision_thread = None;
    if workbench_any_selected(&[WORKTREE_NO_DIALOG]) {
        let collision_capture = (|| -> Result<TestResult> {
            let collision_connection = StubAgentConnection::new()
                .with_agent_id(AgentId::new(agent_servers::CODEX_ID))
                .with_telemetry_id("Codex".into());
            let (_collision_thread_id, _collision_session_id, thread) =
                open_concurrent_direct_thread(
                    &panel,
                    workspace_window,
                    collision_connection,
                    agent_servers::CODEX_ID,
                    "Codex shares Claude's worktree",
                    &claude_worktree,
                    cx,
                )?;
            collision_thread = Some(thread);
            cx.update(ui_prompt::use_internal_prompt_renderer);
            send_concurrent_visual_prompt(
                &panel,
                workspace_window,
                "Write in the worktree Claude is already working in.",
                cx,
            )?;
            cx.set_debug_accessibility_active(workspace_window, true)?;
            let snapshot = cx.debug_render_snapshot(workspace_window)?;
            let accessibility = snapshot
                .accessibility_tree_json()
                .context("concurrent-agent accessibility tree was not active")?;

            // `OMEGA-DELTA-0214`. The whole point of the scene: nothing is in
            // the way. The fixture roots are not real git repositories, so
            // this run takes the disclosure branch rather than the isolation
            // branch — which is the harder case to get right, because it is
            // the one that still has something to say.
            for forbidden in [
                "Another agent is already using this worktree",
                "Run here anyway",
                "Running two agents in one worktree can overwrite",
            ] {
                anyhow::ensure!(
                    !accessibility.contains(forbidden),
                    "OMEGA-DELTA-0214: the deleted collision modal is back — found {forbidden:?}"
                );
            }
            record_workbench_semantic_check(WORKTREE_NO_DIALOG, "no-collision-dialog");

            // Disclosure, not silence. The occupying thread and the shared
            // path stay nameable; only the interruption is gone. The notice is
            // read through its dismiss control, which is the only part of a
            // `Callout` that carries an accessibility node.
            anyhow::ensure!(
                snapshot.selector_count("omega.thread.shared-worktree-disclosure") == 1,
                "shared-worktree disclosure was not drawn"
            );
            for expected in ["Sharing this worktree", CLAUDE_TITLE] {
                anyhow::ensure!(
                    accessibility.contains(expected),
                    "shared-worktree disclosure did not expose {expected:?}"
                );
            }
            record_workbench_semantic_check(
                WORKTREE_NO_DIALOG,
                "occupied-worktree-owner-still-named",
            );

            run_visual_test(WORKTREE_NO_DIALOG, workspace_window, cx, update_baseline)
        })();

        match collision_capture {
            Ok(capture) => results.push(capture),
            Err(error) => {
                log::error!("concurrent-agent shared-worktree visual failed: {error:#}");
                cx.run_until_parked();
                let mut threads = vec![codex_thread.clone(), claude_thread];
                threads.extend(collision_thread);
                finish_concurrent_agent_visual_tests(
                    workspace_window,
                    &workspace,
                    &panel,
                    &threads,
                    cx,
                    &[],
                )
                .log_err();
                return Err(error);
            }
        }
    }

    drop(codex_connection);
    drop(claude_connection);
    let mut threads = vec![codex_thread, claude_thread];
    threads.extend(collision_thread);
    finish_concurrent_agent_visual_tests(
        workspace_window,
        &workspace,
        &panel,
        &threads,
        cx,
        &results,
    )
}

/// A visual-test server that connects to one real, explicitly named Exo lane.
///
/// The server stores only the lane path so it remains `Send`, as
/// [`AgentServer`] requires. The connection itself stays on the GPUI thread.
#[derive(Clone)]
#[cfg(target_os = "macos")]
struct ExoVisualAgentServer {
    lane_path: PathBuf,
}

#[cfg(target_os = "macos")]
impl AgentServer for ExoVisualAgentServer {
    fn logo(&self) -> ui::IconName {
        ui::IconName::OmegaAssistant
    }

    fn agent_id(&self) -> AgentId {
        "Exo".into()
    }

    fn connect(
        &self,
        _delegate: AgentServerDelegate,
        project: Entity<Project>,
        cx: &mut App,
    ) -> gpui::Task<gpui::Result<Rc<dyn AgentConnection>>> {
        let lane_path = self.lane_path.clone();
        let agent_server_store = project.read(cx).agent_server_store().downgrade();
        cx.spawn(async move |cx| {
            agent_ui::omega_exo_connection::connect_configured_lane(
                &lane_path,
                project,
                agent_server_store,
                cx,
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!("the Exo visual lane file is not configured"))
        })
    }

    fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
        self
    }
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_exo_visual_tests(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    let lane_path = PathBuf::from(
        std::env::var("OMEGA_EXO_VISUAL_LANE_FILE")
            .context("set OMEGA_EXO_VISUAL_LANE_FILE to an isolated Exo lane file")?,
    );
    anyhow::ensure!(
        lane_path.is_file(),
        "the Exo visual lane does not exist: {}",
        lane_path.display()
    );

    // Use independent native windows instead of resizing one hidden Metal
    // window. The macOS platform does not dispatch a bounds callback for that
    // test-only resize, so a second screenshot can silently retain the first
    // viewport. Two fresh windows prove both responsive branches directly.
    //
    // omega#161. There used to be a second pair here — the same Exo turn on
    // the `--full-editor` surface, photographed before a one-way mode flip.
    // The mode split is removed, so the surface these two scenes photograph is
    // the only one the shipped binary draws, and the flip machinery went with
    // the flag.
    let zero_base_wide = run_omega_exo_visual_capture(
        app_state.clone(),
        cx,
        lane_path.clone(),
        "omega_zero_base_wide",
        size(px(1320.), px(860.)),
        update_baseline,
    )?;
    let zero_base_narrow = run_omega_exo_visual_capture(
        app_state,
        cx,
        lane_path,
        "omega_zero_base_narrow",
        size(px(720.), px(900.)),
        update_baseline,
    )?;

    for result in [&zero_base_wide, &zero_base_narrow] {
        if let TestResult::BaselineUpdated(path) = result {
            return Ok(TestResult::BaselineUpdated(path.clone()));
        }
    }
    Ok(TestResult::Passed)
}

/// Run the scheduler until it runs out of work, or until a step budget is
/// spent, whichever comes first.
///
/// omega#99. `run_until_parked` has no budget: it returns only when the
/// scheduler has nothing left to run. That is the right primitive for a suite
/// whose tasks are all simulated, and the wrong one while a real `exo acp`
/// child is attached — the ACP transport's read of the child's stdout is
/// runnable again as soon as it is polled, so the call never returns and the
/// capture spins on one core forever. Every wait in the Exo capture is
/// therefore bounded, and a wait that runs out of budget reports what it was
/// waiting for instead of hanging.
#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn step_scheduler(cx: &mut VisualTestAppContext, budget: usize) {
    for _ in 0..budget {
        if !cx.background_executor.tick() {
            return;
        }
    }
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_exo_visual_capture(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    lane_path: PathBuf,
    test_name: &'static str,
    window_size: gpui::Size<gpui::Pixels>,
    update_baseline: bool,
) -> Result<TestResult> {
    use agent_ui::AgentPanel;

    let project_dir = tempfile::tempdir()?.keep().canonicalize()?;
    let project = cx.update(|cx| {
        Project::local(
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            app_state.user_store.clone(),
            app_state.languages.clone(),
            app_state.fs.clone(),
            None,
            project::LocalProjectFlags {
                init_worktree_trust: false,
                ..Default::default()
            },
            cx,
        )
    });
    let add_worktree = project.update(cx, |project, cx| {
        project.find_or_create_worktree(&project_dir, true, cx)
    });
    cx.background_executor.allow_parking();
    cx.foreground_executor
        .block_test(add_worktree)
        .context("failed to add the Exo visual worktree")?;
    cx.background_executor.forbid_parking();
    cx.run_until_parked();

    let bounds = Bounds {
        origin: point(px(-10_000.), px(-10_000.)),
        size: window_size,
    };
    let workspace_window: WindowHandle<Workspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: true,
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        Workspace::new(None, project.clone(), app_state.clone(), window, cx)
                    })
                },
            )
        })
        .context("failed to open the Exo workspace window")?;
    cx.run_until_parked();

    // OMEGA-DELTA-0052, omega#161. There is no per-scene surface assertion
    // any more: the mode split is removed, `omega_zero_base::is_active()` is
    // constant, and the surface these scenes photograph is the only one the
    // shipped binary draws. The runner used to flip a process-global here and
    // assert the flip per scene so an ordering mistake could not file the
    // subtracted window under the ordinary name; both names and both surfaces
    // are gone with the flag.
    let (weak_workspace, async_window_cx) = workspace_window
        .update(cx, |workspace, window, cx| {
            (workspace.weak_handle(), window.to_async(cx))
        })
        .context("failed to get the Exo workspace handle")?;
    cx.background_executor.allow_parking();
    let panel = cx
        .foreground_executor
        .block_test(AgentPanel::load(weak_workspace, async_window_cx))
        .context("failed to load the Exo Agent panel")?;
    cx.background_executor.forbid_parking();

    workspace_window
        .update(cx, |workspace, window, cx| {
            workspace.add_panel(panel.clone(), window, cx);
            workspace.open_panel::<AgentPanel>(window, cx);
            panel.update(cx, |panel, cx| {
                use workspace::dock::Panel as _;
                panel.set_zoomed(true, window, cx);
            });
        })
        .context("failed to open the Exo Agent panel")?;
    cx.run_until_parked();

    // The lane's `exo acp` child, kept out here so the teardown below can end
    // it whether or not the capture succeeded. omega#99: a scene that failed
    // its assertions still started a process, and leaving it running is what
    // made the second capture unrunnable.
    let mut exo_connection: Option<Rc<agent_ui::omega_exo_connection::ExoHarnessConnection>> = None;

    // Keep capture failures inside this closure so the real ACP connection is
    // always removed below. Otherwise an early assertion or screenshot error
    // masks its own cause with GPUI's leaked ElicitationStore check.
    let capture = (|exo_connection: &mut Option<
        Rc<agent_ui::omega_exo_connection::ExoHarnessConnection>,
    >|
     -> Result<TestResult> {
        let server: Rc<dyn AgentServer> = Rc::new(ExoVisualAgentServer { lane_path });
        cx.update_window(workspace_window.into(), |_, window, cx| {
            panel.update(cx, |panel, cx| {
                panel.open_external_thread_with_server(server, window, cx);
            });
        })?;

        // Connecting the child process and completing ACP initialization
        // require real I/O. Poll the visible state for at most five seconds;
        // one `run_until_parked` can return before stdio initialization wakes
        // the foreground executor, so the loop steps the scheduler and then
        // gives the reactor real wall time to deliver the next line.
        //
        // omega#99. Parking stays *forbidden* for the duration of this loop.
        // `run_until_parked` returns when the scheduler has nothing left to
        // run; with parking allowed it instead waits for the executor to go
        // quiet, and an attached ACP transport whose child is sitting on its
        // own stdin never does. The loop then never reached its second
        // iteration and the capture hung here — before the turn, before any
        // screenshot, with the runner spinning on the child's stdout at 100%
        // of a core. Parking is what the blocking wait on the turn below needs,
        // and it is switched on there and only there.
        for _ in 0..100 {
            step_scheduler(cx, SCHEDULER_STEP_BUDGET);
            if cx.read(|cx| {
                panel
                    .read(cx)
                    .active_thread_view_for_tests()
                    .is_some_and(|view| view.read(cx).active_thread().is_some())
            }) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        let thread_view = cx
            .read(|cx| panel.read(cx).active_thread_view_for_tests().cloned())
            .ok_or_else(|| anyhow::anyhow!("the Exo workspace has no active thread"))?;
        let thread = cx
            .read(|cx| {
                thread_view
                    .read(cx)
                    .active_thread()
                    .map(|active| active.read(cx).thread.clone())
            })
            .ok_or_else(|| anyhow::anyhow!("the Exo workspace thread is not available"))?;

        // Hand the lane's connection to the teardown before the turn runs, not
        // after. A turn that fails its assertions has still started an
        // `exo acp`, and that is exactly the run whose leftover child would
        // make the *next* scene unrunnable.
        *exo_connection = cx.read(|cx| {
            thread
                .read(cx)
                .connection()
                .clone()
                .downcast::<agent_ui::omega_exo_connection::ExoHarnessConnection>()
        });

        let marker = "OMEGA-EXO-PANE-READY";
        let prompt =
            format!("Run one tool that prints OMEGA-EXO-TOOL, then reply with exactly {marker}.");
        let send = thread.update(cx, |thread, cx| thread.send(vec![prompt.into()], cx));
        cx.background_executor.allow_parking();
        let send_result = cx.foreground_executor.block_test(send);
        cx.background_executor.forbid_parking();
        send_result.context("the real Exo visual turn failed")?;
        step_scheduler(cx, SCHEDULER_STEP_BUDGET);

        let (transcript, turn) = cx.read(|cx| {
            let thread = thread.read(cx);
            let connection = thread
                .connection()
                .clone()
                .downcast::<agent_ui::omega_exo_connection::ExoHarnessConnection>()
                .expect("the visual thread must retain its Exo connection");
            (thread.to_markdown(cx), connection.turn())
        });
        anyhow::ensure!(
            transcript.contains(marker),
            "the real Exo response did not reach the transcript:\n{transcript}"
        );
        anyhow::ensure!(
            transcript.contains("Tool Call: shell"),
            "the real Exo tool call did not reach the transcript:\n{transcript}"
        );
        anyhow::ensure!(
            transcript.contains("OMEGA-EXO-TOOL"),
            "the real Exo tool result did not reach the transcript:\n{transcript}"
        );
        anyhow::ensure!(
            turn.phase == agent_ui::omega_exo_connection::ExoTurnPhase::Completed,
            "the real Exo turn did not complete: {turn:?}"
        );
        anyhow::ensure!(
            turn.exo_session_id.is_some()
                && turn.exo_turn_id.is_some()
                && turn.latest_event_id.is_some(),
            "the real Exo turn did not return durable references: {turn:?}"
        );

        let actual_size = cx.update_window(workspace_window.into(), |_, window, _cx| {
            window.viewport_size()
        })?;
        anyhow::ensure!(
            actual_size == window_size,
            "{test_name} opened at {actual_size:?}, expected {window_size:?}"
        );

        run_visual_test(test_name, workspace_window.into(), cx, update_baseline)
    })(&mut exo_connection);

    // End the `exo acp` process this capture started, by name, before anything
    // else is torn down.
    //
    // omega#99. `AcpConnection` kills its child on `Drop`, but `Drop` runs only
    // once every owner of the connection has let go, and the owners include
    // GPUI entities whose teardown this runner can ask for and cannot observe.
    // Measured on 2026-07-26 by sampling `pgrep` every 100ms across a full run
    // with this call removed: a capture's child was still alive while the next
    // capture's child was starting — two at once — and went away only when the
    // reference graph happened to unwind. That overlap is not what a scene
    // should depend on, so the runner ends the process it started rather than
    // waiting to find out.
    //
    // The alternatives were considered and rejected. Giving each capture its
    // own Exo conversation would make Omega's own test tooling write Exo state,
    // which omega#87 closed the door on: Omega does not configure Exo. Reusing
    // one connection across scenes would make every photograph after the first
    // depend on the transcript the previous one left behind, which is the
    // opposite of what independent baselines are for. Ending the process the
    // runner itself started is the one option that leaves both boundaries where
    // they were.
    if let Some(exo) = exo_connection.as_ref() {
        exo.end_exo_process();
    }
    drop(exo_connection);

    // Drop every owner of the real ACP connection before the runner's leaked
    // entity check. The panel's connection store retains a successful custom
    // connection after its thread and window close. Replacing that cached
    // entry first lets the Exo child and its ElicitationStore terminate.
    panel.update(cx, |panel, cx| {
        panel.connection_store().update(cx, |store, cx| {
            store.restart_connection(
                Agent::Custom { id: "Exo".into() },
                Rc::new(StubAgentServer::new(StubAgentConnection::new())),
                cx,
            );
        });
    });
    cx.run_until_parked();

    workspace_window
        .update(cx, |workspace, window, cx| {
            workspace.remove_panel(&panel, window, cx);
            let project = workspace.project().clone();
            project.update(cx, |project, cx| {
                let worktree_ids: Vec<_> = project
                    .worktrees(cx)
                    .map(|worktree| worktree.read(cx).id())
                    .collect();
                for id in worktree_ids {
                    project.remove_worktree(id, cx);
                }
            });
        })
        .log_err();
    cx.run_until_parked();
    drop(panel);
    drop(project);
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();
    cx.run_until_parked();
    for _ in 0..15 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    capture
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_agent_thread_view_test(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    use agent::{AgentTool, ToolInput};
    use agent_ui::AgentPanel;

    // Create a temporary directory with the test image
    // Canonicalize to resolve symlinks (on macOS, /var -> /private/var)
    // Use keep() to prevent auto-cleanup - we'll clean up manually after stopping background tasks
    let temp_dir = tempfile::tempdir()?;
    let temp_path = temp_dir.keep();
    let canonical_temp = temp_path.canonicalize()?;
    let project_path = canonical_temp.join("project");
    std::fs::create_dir_all(&project_path)?;
    let image_path = project_path.join("test-image.png");
    std::fs::write(&image_path, EMBEDDED_TEST_IMAGE)?;

    // Create a project with the test image
    let project = cx.update(|cx| {
        project::Project::local(
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            app_state.user_store.clone(),
            app_state.languages.clone(),
            app_state.fs.clone(),
            None,
            project::LocalProjectFlags {
                init_worktree_trust: false,
                ..Default::default()
            },
            cx,
        )
    });

    // Add the test directory as a worktree
    let add_worktree_task = project.update(cx, |project, cx| {
        project.find_or_create_worktree(&project_path, true, cx)
    });

    cx.background_executor.allow_parking();
    let (worktree, _) = cx
        .foreground_executor
        .block_test(add_worktree_task)
        .context("Failed to add worktree")?;
    cx.background_executor.forbid_parking();

    cx.run_until_parked();

    let worktree_name = cx.read(|cx| worktree.read(cx).root_name_str().to_string());

    // Create the necessary entities for the ReadFileTool
    let action_log = cx.update(|cx| cx.new(|_| action_log::ActionLog::new(project.clone())));

    // Create the ReadFileTool
    let tool = Arc::new(agent::ReadFileTool::new(project.clone(), action_log, true));

    // Create a test event stream to capture tool output
    let (event_stream, mut event_receiver) = agent::ToolCallEventStream::test();

    // Run the real ReadFileTool to get the actual image content
    let input = agent::ReadFileToolInput {
        path: format!("{}/test-image.png", worktree_name),
        start_line: None,
        end_line: None,
    };
    let run_task = cx.update(|cx| {
        tool.clone()
            .run(ToolInput::resolved(input), event_stream, cx)
    });

    cx.background_executor.allow_parking();
    let run_result = cx.foreground_executor.block_test(run_task);
    cx.background_executor.forbid_parking();
    run_result.map_err(|e| match e {
        language_model::LanguageModelToolResultContent::Text(text) => {
            anyhow::anyhow!("ReadFileTool failed: {text}")
        }
        other => anyhow::anyhow!("ReadFileTool failed: {other:?}"),
    })?;

    cx.run_until_parked();

    // Collect the events from the tool execution
    let mut tool_content: Vec<acp::ToolCallContent> = Vec::new();
    let mut tool_locations: Vec<acp::ToolCallLocation> = Vec::new();

    while let Ok(event) = event_receiver.try_recv() {
        if let Ok(agent::ThreadEvent::ToolCallUpdate(acp_thread::ToolCallUpdate::UpdateFields(
            update,
        ))) = event
        {
            if let Some(content) = update.fields.content {
                tool_content.extend(content);
            }
            if let Some(locations) = update.fields.locations {
                tool_locations.extend(locations);
            }
        }
    }

    if tool_content.is_empty() {
        return Err(anyhow::anyhow!("ReadFileTool did not produce any content"));
    }

    // Create stub connection with the real tool output
    let connection = StubAgentConnection::new();
    connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(
        acp::ToolCall::new(
            "read_file",
            format!("Read file `{}/test-image.png`", worktree_name),
        )
        .kind(acp::ToolKind::Read)
        .status(acp::ToolCallStatus::Completed)
        .locations(tool_locations)
        .content(tool_content),
    )]);

    let stub_agent: Rc<dyn AgentServer> = Rc::new(StubAgentServer::new(connection));

    // Create a window sized for the agent panel
    let window_size = size(px(500.0), px(900.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };

    let workspace_window: WindowHandle<Workspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        Workspace::new(None, project.clone(), app_state.clone(), window, cx)
                    })
                },
            )
        })
        .context("Failed to open agent window")?;

    cx.run_until_parked();

    // Load the AgentPanel
    let (weak_workspace, async_window_cx) = workspace_window
        .update(cx, |workspace, window, cx| {
            (workspace.weak_handle(), window.to_async(cx))
        })
        .context("Failed to get workspace handle")?;

    cx.background_executor.allow_parking();
    let panel = cx
        .foreground_executor
        .block_test(AgentPanel::load(weak_workspace, async_window_cx))
        .context("Failed to load AgentPanel")?;
    cx.background_executor.forbid_parking();

    cx.update_window(workspace_window.into(), |_, _window, cx| {
        workspace_window
            .update(cx, |workspace, window, cx| {
                workspace.add_panel(panel.clone(), window, cx);
                workspace.open_panel::<AgentPanel>(window, cx);
            })
            .log_err();
    })?;

    cx.run_until_parked();

    // Inject the stub server and open the stub thread
    cx.update_window(workspace_window.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.open_external_thread_with_server(stub_agent.clone(), window, cx);
        });
    })?;

    cx.run_until_parked();

    // Get the thread view and send a message
    let thread_view = cx
        .read(|cx| panel.read(cx).active_thread_view_for_tests().cloned())
        .ok_or_else(|| anyhow::anyhow!("No active thread view"))?;

    let thread = cx
        .read(|cx| {
            thread_view
                .read(cx)
                .active_thread()
                .map(|active| active.read(cx).thread.clone())
        })
        .ok_or_else(|| anyhow::anyhow!("Thread not available"))?;

    // Send the message to trigger the image response
    let send_future = thread.update(cx, |thread, cx| {
        thread.send(vec!["Show me the Omega logo".into()], cx)
    });

    cx.background_executor.allow_parking();
    let send_result = cx.foreground_executor.block_test(send_future);
    cx.background_executor.forbid_parking();
    send_result.context("Failed to send message")?;

    cx.run_until_parked();

    // Get the tool call ID for expanding later
    let tool_call_id = cx
        .read(|cx| {
            thread.read(cx).entries().iter().find_map(|entry| {
                if let acp_thread::AgentThreadEntry::ToolCall(tool_call) = entry {
                    Some(tool_call.id.clone())
                } else {
                    None
                }
            })
        })
        .ok_or_else(|| anyhow::anyhow!("Expected a ToolCall entry in thread"))?;

    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    // Capture the COLLAPSED state
    let collapsed_result = run_visual_test(
        "agent_thread_with_image_collapsed",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // Now expand the tool call so the image is visible
    thread_view.update(cx, |view, cx| {
        view.expand_tool_call(tool_call_id, cx);
    });

    cx.run_until_parked();

    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    // Capture the EXPANDED state
    let expanded_result = run_visual_test(
        "agent_thread_with_image_expanded",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // Remove the worktree from the project to stop background scanning tasks
    // This prevents "root path could not be canonicalized" errors when we clean up
    workspace_window
        .update(cx, |workspace, _window, cx| {
            let project = workspace.project().clone();
            project.update(cx, |project, cx| {
                let worktree_ids: Vec<_> =
                    project.worktrees(cx).map(|wt| wt.read(cx).id()).collect();
                for id in worktree_ids {
                    project.remove_worktree(id, cx);
                }
            });
        })
        .log_err();

    cx.run_until_parked();

    // Close the window
    // Note: This may cause benign "editor::scroll window not found" errors from scrollbar
    // auto-hide timers that were scheduled before the window was closed. These errors
    // don't affect test results.
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();

    // Run until all cleanup tasks complete
    cx.run_until_parked();

    // Give background tasks time to finish, including scrollbar hide timers (1 second)
    for _ in 0..15 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Note: We don't delete temp_path here because background worktree tasks may still
    // be running. The directory will be cleaned up when the process exits.

    match (&collapsed_result, &expanded_result) {
        (TestResult::Passed, TestResult::Passed) => Ok(TestResult::Passed),
        (TestResult::BaselineUpdated(p), _) | (_, TestResult::BaselineUpdated(p)) => {
            Ok(TestResult::BaselineUpdated(p.clone()))
        }
    }
}

/// Visual test for the Tool Permissions Settings UI page
///
/// Takes a screenshot showing the tool config page with matched patterns and verdict.
#[cfg(target_os = "macos")]
fn run_tool_permissions_visual_tests(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    _update_baseline: bool,
) -> Result<TestResult> {
    use agent_settings::{AgentSettings, CompiledRegex, ToolPermissions, ToolRules};
    use collections::HashMap;
    use omega_actions::OpenSettingsAt;
    use settings::ToolPermissionMode;

    // Set up tool permissions with "hi" as both always_deny and always_allow for terminal
    cx.update(|cx| {
        let mut tools = HashMap::default();
        tools.insert(
            Arc::from("terminal"),
            ToolRules {
                default: None,
                always_allow: vec![CompiledRegex::new("hi", false).unwrap()],
                always_deny: vec![CompiledRegex::new("hi", false).unwrap()],
                always_confirm: vec![],
                invalid_patterns: vec![],
            },
        );
        let mut settings = AgentSettings::get_global(cx).clone();
        settings.tool_permissions = ToolPermissions {
            default: ToolPermissionMode::Confirm,
            tools,
        };
        AgentSettings::override_global(settings, cx);
    });

    // Create a minimal workspace to dispatch the settings action from
    let window_size = size(px(900.0), px(700.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };

    let project = cx.update(|cx| {
        project::Project::local(
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            app_state.user_store.clone(),
            app_state.languages.clone(),
            app_state.fs.clone(),
            None,
            project::LocalProjectFlags {
                init_worktree_trust: false,
                ..Default::default()
            },
            cx,
        )
    });

    let workspace_window: WindowHandle<MultiWorkspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| {
                    let workspace = cx.new(|cx| {
                        Workspace::new(None, project.clone(), app_state.clone(), window, cx)
                    });
                    cx.new(|cx| MultiWorkspace::new(workspace, window, cx))
                },
            )
        })
        .context("Failed to open workspace window for settings test")?;

    cx.run_until_parked();

    // Dispatch the OpenSettingsAt action to open settings at the tool_permissions path
    workspace_window
        .update(cx, |_workspace, window, cx| {
            window.dispatch_action(
                Box::new(OpenSettingsAt {
                    path: "agent.tool_permissions".to_string(),
                    target: None,
                }),
                cx,
            );
        })
        .context("Failed to dispatch OpenSettingsAt action")?;

    cx.run_until_parked();

    // Give the settings window time to open and render
    for _ in 0..10 {
        cx.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
    }

    // Find the settings window - it should be the newest window (last in the list)
    let all_windows = cx.update(|cx| cx.windows());
    let settings_window = all_windows.last().copied().context("No windows found")?;

    let output_dir = std::env::var("VISUAL_TEST_OUTPUT_DIR")
        .unwrap_or_else(|_| "target/visual_tests".to_string());
    std::fs::create_dir_all(&output_dir).log_err();

    // Navigate to the tool permissions sub-page using the public API
    let settings_window_handle = settings_window
        .downcast::<settings_ui::SettingsWindow>()
        .context("Failed to downcast to SettingsWindow")?;

    settings_window_handle
        .update(cx, |settings_window, window, cx| {
            settings_window.navigate_to_sub_page("agent.tool_permissions", window, cx);
        })
        .context("Failed to navigate to tool permissions sub-page")?;

    cx.run_until_parked();

    // Give the sub-page time to render
    for _ in 0..10 {
        cx.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
    }

    // Now navigate into a specific tool (Terminal) to show the tool config page
    settings_window_handle
        .update(cx, |settings_window, window, cx| {
            settings_window.push_dynamic_sub_page(
                "Terminal",
                "Configure Tool Rules",
                None,
                true,
                settings_ui::pages::render_terminal_tool_config,
                window,
                cx,
            );
        })
        .context("Failed to navigate to Terminal tool config")?;

    cx.run_until_parked();

    // Give the tool config page time to render
    for _ in 0..10 {
        cx.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
    }

    // Refresh and redraw so the "Test Your Rules" input is present
    cx.update_window(settings_window, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .log_err();
    cx.run_until_parked();

    cx.update_window(settings_window, |_, window, _cx| {
        window.refresh();
    })
    .log_err();
    cx.run_until_parked();

    // Focus the first tab stop in the window (the "Test Your Rules" editor
    // has tab_index(0) and tab_stop(true)) and type "hi" into it.
    cx.update_window(settings_window, |_, window, cx| {
        window.focus_next(cx);
    })
    .log_err();
    cx.run_until_parked();

    cx.simulate_input(settings_window, "hi");

    // Let the UI update with the matched patterns
    for _ in 0..5 {
        cx.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
    }

    // Refresh and redraw
    cx.update_window(settings_window, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .log_err();
    cx.run_until_parked();

    cx.update_window(settings_window, |_, window, _cx| {
        window.refresh();
    })
    .log_err();
    cx.run_until_parked();

    // Save screenshot: Tool config page with "hi" typed and matched patterns visible
    let tool_config_output_path =
        PathBuf::from(&output_dir).join("tool_permissions_test_rules.png");

    if let Ok(screenshot) = cx.capture_screenshot(settings_window) {
        screenshot.save(&tool_config_output_path).log_err();
        println!(
            "Screenshot (test rules) saved to: {}",
            tool_config_output_path.display()
        );
    }

    // Clean up - close the settings window
    cx.update_window(settings_window, |_, window, _cx| {
        window.remove_window();
    })
    .log_err();

    // Close the workspace window
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();

    cx.run_until_parked();

    // Give background tasks time to finish
    for _ in 0..5 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Return success - we're just capturing screenshots, not comparing baselines
    Ok(TestResult::Passed)
}

#[cfg(target_os = "macos")]
struct ErrorWrappingTestView;

#[cfg(target_os = "macos")]
impl gpui::Render for ErrorWrappingTestView {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use ui::{Button, Callout, IconName, LabelSize, Severity, prelude::*, v_flex};

        let long_error_message = "Rate limit reached for gpt-5.2-codex in organization \
            org-QmYpir6k6dkULKU1XUSN6pal on tokens per min (TPM): Limit 500000, Used 442480, \
            Requested 59724. Please try again in 264ms. Visit \
            https://platform.openai.com/account/rate-limits to learn more.";

        let retry_description = "Retrying. Next attempt in 4 seconds (Attempt 1 of 2).";

        v_flex()
            .size_full()
            .bg(cx.theme().colors().background)
            .p_4()
            .gap_4()
            .child(
                Callout::new()
                    .icon(IconName::Warning)
                    .severity(Severity::Warning)
                    .title(long_error_message)
                    .description(retry_description),
            )
            .child(
                Callout::new()
                    .severity(Severity::Error)
                    .icon(IconName::XCircle)
                    .title("An Error Happened")
                    .description(long_error_message)
                    .actions_slot(Button::new("dismiss", "Dismiss").label_size(LabelSize::Small)),
            )
            .child(
                Callout::new()
                    .severity(Severity::Error)
                    .icon(IconName::XCircle)
                    .title(long_error_message)
                    .actions_slot(Button::new("retry", "Retry").label_size(LabelSize::Small)),
            )
    }
}

#[cfg(target_os = "macos")]
struct ThreadItemBranchNameTestView;

#[cfg(target_os = "macos")]
impl gpui::Render for ThreadItemBranchNameTestView {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use ui::{
            IconName, Label, LabelSize, ThreadItem, ThreadItemWorktreeInfo, WorktreeKind,
            prelude::*,
        };

        let section_label = |text: &str| {
            Label::new(text.to_string())
                .size(LabelSize::Small)
                .color(Color::Muted)
        };

        let container = || {
            v_flex()
                .w_80()
                .border_1()
                .border_color(cx.theme().colors().border_variant)
                .bg(cx.theme().colors().panel_background)
        };

        v_flex()
            .size_full()
            .bg(cx.theme().colors().background)
            .p_4()
            .gap_3()
            .child(
                Label::new("ThreadItem Branch Names")
                    .size(LabelSize::Large)
                    .color(Color::Default),
            )
            .child(section_label(
                "Linked worktree with branch (worktree / branch)",
            ))
            .child(
                container().child(
                    ThreadItem::new("ti-linked-branch", "Fix scrolling behavior")
                        .icon(IconName::AiClaude)
                        .timestamp("5m")
                        .worktrees(vec![ThreadItemWorktreeInfo {
                            worktree_name: Some("jade-glen".into()),
                            full_path: "/worktrees/jade-glen/zed".into(),
                            highlight_positions: Vec::new(),
                            kind: WorktreeKind::Linked,
                            branch_name: Some("fix-scrolling".into()),
                        }]),
                ),
            )
            .child(section_label(
                "Linked worktree without branch (detached HEAD)",
            ))
            .child(
                container().child(
                    ThreadItem::new("ti-linked-no-branch", "Review worktree cleanup")
                        .icon(IconName::AiClaude)
                        .timestamp("1h")
                        .worktrees(vec![ThreadItemWorktreeInfo {
                            worktree_name: Some("focal-arrow".into()),
                            full_path: "/worktrees/focal-arrow/zed".into(),
                            highlight_positions: Vec::new(),
                            kind: WorktreeKind::Linked,
                            branch_name: None,
                        }]),
                ),
            )
            .child(section_label("Main worktree with branch (nothing shown)"))
            .child(
                container().child(
                    ThreadItem::new("ti-main-branch", "Request for Long Classic Poem")
                        .icon(IconName::OmegaAgent)
                        .timestamp("2d")
                        .worktrees(vec![ThreadItemWorktreeInfo {
                            worktree_name: Some("zed".into()),
                            full_path: "/projects/zed".into(),
                            highlight_positions: Vec::new(),
                            kind: WorktreeKind::Main,
                            branch_name: Some("main".into()),
                        }]),
                ),
            )
            .child(section_label(
                "Main worktree without branch (nothing shown)",
            ))
            .child(
                container().child(
                    ThreadItem::new("ti-main-no-branch", "Simple greeting thread")
                        .icon(IconName::OmegaAgent)
                        .timestamp("3d")
                        .worktrees(vec![ThreadItemWorktreeInfo {
                            worktree_name: Some("zed".into()),
                            full_path: "/projects/zed".into(),
                            highlight_positions: Vec::new(),
                            kind: WorktreeKind::Main,
                            branch_name: None,
                        }]),
                ),
            )
            .child(section_label("Linked worktree where name matches branch"))
            .child(
                container().child(
                    ThreadItem::new("ti-same-name", "Implement feature")
                        .icon(IconName::AiClaude)
                        .timestamp("6d")
                        .worktrees(vec![ThreadItemWorktreeInfo {
                            worktree_name: Some("stoic-reed".into()),
                            full_path: "/worktrees/stoic-reed/zed".into(),
                            highlight_positions: Vec::new(),
                            kind: WorktreeKind::Linked,
                            branch_name: Some("stoic-reed".into()),
                        }]),
                ),
            )
            .child(section_label(
                "Manually opened linked worktree (main_path resolves to original repo)",
            ))
            .child(
                container().child(
                    ThreadItem::new("ti-manual-linked", "Robust Git Worktree Rollback")
                        .icon(IconName::OmegaAgent)
                        .timestamp("40m")
                        .worktrees(vec![ThreadItemWorktreeInfo {
                            worktree_name: Some("focal-arrow".into()),
                            full_path: "/worktrees/focal-arrow/zed".into(),
                            highlight_positions: Vec::new(),
                            kind: WorktreeKind::Linked,
                            branch_name: Some("persist-worktree-3-wiring".into()),
                        }]),
                ),
            )
            .child(section_label(
                "Linked worktree + branch + diff stats + timestamp",
            ))
            .child(
                container().child(
                    ThreadItem::new("ti-linked-full", "Full metadata with diff stats")
                        .icon(IconName::AiClaude)
                        .timestamp("3w")
                        .added(42)
                        .removed(17)
                        .worktrees(vec![ThreadItemWorktreeInfo {
                            worktree_name: Some("jade-glen".into()),
                            full_path: "/worktrees/jade-glen/zed".into(),
                            highlight_positions: Vec::new(),
                            kind: WorktreeKind::Linked,
                            branch_name: Some("feature-branch".into()),
                        }]),
                ),
            )
            .child(section_label("Long branch name truncation with diff stats"))
            .child(
                container().child(
                    ThreadItem::new("ti-long-branch", "Overflow test with very long branch")
                        .icon(IconName::AiClaude)
                        .timestamp("2d")
                        .added(108)
                        .removed(53)
                        .worktrees(vec![ThreadItemWorktreeInfo {
                            worktree_name: Some("my-project".into()),
                            full_path: "/worktrees/my-project/zed".into(),
                            highlight_positions: Vec::new(),
                            kind: WorktreeKind::Linked,
                            branch_name: Some(
                                "fix-very-long-branch-name-that-should-truncate".into(),
                            ),
                        }]),
                ),
            )
            .child(section_label(
                "Main worktree with branch + diff stats + timestamp (branch hidden)",
            ))
            .child(
                container().child(
                    ThreadItem::new("ti-main-full", "Main worktree with everything")
                        .icon(IconName::OmegaAgent)
                        .timestamp("5m")
                        .added(23)
                        .removed(8)
                        .worktrees(vec![ThreadItemWorktreeInfo {
                            worktree_name: Some("zed".into()),
                            full_path: "/projects/zed".into(),
                            highlight_positions: Vec::new(),
                            kind: WorktreeKind::Main,
                            branch_name: Some("sidebar-show-branch-name".into()),
                        }]),
                ),
            )
    }
}

#[cfg(target_os = "macos")]
fn run_thread_item_branch_name_visual_tests(
    _app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    let window_size = size(px(400.0), px(1150.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };

    let window = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |_window, cx| cx.new(|_| ThreadItemBranchNameTestView),
            )
        })
        .context("Failed to open thread item branch name test window")?;

    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    let test_result = run_visual_test(
        "thread_item_branch_names",
        window.into(),
        cx,
        update_baseline,
    )?;

    cx.update_window(window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();

    cx.run_until_parked();

    for _ in 0..15 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    Ok(test_result)
}

#[cfg(target_os = "macos")]
struct ThreadItemIconDecorationsTestView;

#[cfg(target_os = "macos")]
impl gpui::Render for ThreadItemIconDecorationsTestView {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use ui::{IconName, Label, LabelSize, ThreadItem, prelude::*};

        let section_label = |text: &str| {
            Label::new(text.to_string())
                .size(LabelSize::Small)
                .color(Color::Muted)
        };

        let container = || {
            v_flex()
                .w_80()
                .border_1()
                .border_color(cx.theme().colors().border_variant)
                .bg(cx.theme().colors().panel_background)
        };

        v_flex()
            .size_full()
            .bg(cx.theme().colors().background)
            .p_4()
            .gap_3()
            .child(
                Label::new("ThreadItem Icon Decorations")
                    .size(LabelSize::Large)
                    .color(Color::Default),
            )
            .child(section_label("No decoration (default idle)"))
            .child(
                container()
                    .child(ThreadItem::new("ti-none", "Default idle thread").timestamp("1:00 AM")),
            )
            .child(section_label("Blue dot (notified)"))
            .child(
                container().child(
                    ThreadItem::new("ti-done", "Generation completed successfully")
                        .timestamp("1:05 AM")
                        .notified(true),
                ),
            )
            .child(section_label("Yellow triangle (waiting for confirmation)"))
            .child(
                container().child(
                    ThreadItem::new("ti-waiting", "Waiting for user confirmation")
                        .timestamp("1:10 AM")
                        .status(ui::AgentThreadStatus::WaitingForConfirmation),
                ),
            )
            .child(section_label("Red X (error)"))
            .child(
                container().child(
                    ThreadItem::new("ti-error", "Failed to connect to server")
                        .timestamp("1:15 AM")
                        .status(ui::AgentThreadStatus::Error),
                ),
            )
            .child(section_label("Spinner (running)"))
            .child(
                container().child(
                    ThreadItem::new("ti-running", "Generating response...")
                        .icon(IconName::AiClaude)
                        .timestamp("1:20 AM")
                        .status(ui::AgentThreadStatus::Running),
                ),
            )
            .child(section_label(
                "Spinner + yellow triangle (waiting for confirmation)",
            ))
            .child(
                container().child(
                    ThreadItem::new("ti-running-waiting", "Running but needs confirmation")
                        .icon(IconName::AiClaude)
                        .timestamp("1:25 AM")
                        .status(ui::AgentThreadStatus::WaitingForConfirmation),
                ),
            )
    }
}

#[cfg(target_os = "macos")]
fn run_thread_item_icon_decorations_visual_tests(
    _app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    let window_size = size(px(400.0), px(600.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };

    let window = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |_window, cx| cx.new(|_| ThreadItemIconDecorationsTestView),
            )
        })
        .context("Failed to open thread item icon decorations test window")?;

    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    let test_result = run_visual_test(
        "thread_item_icon_decorations",
        window.into(),
        cx,
        update_baseline,
    )?;

    cx.update_window(window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();

    cx.run_until_parked();

    for _ in 0..15 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    Ok(test_result)
}

#[cfg(target_os = "macos")]
fn run_error_wrapping_visual_tests(
    _app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    let window_size = size(px(500.0), px(400.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };

    let window = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |_window, cx| cx.new(|_| ErrorWrappingTestView),
            )
        })
        .context("Failed to open error wrapping test window")?;

    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    let test_result =
        run_visual_test("error_message_wrapping", window.into(), cx, update_baseline)?;

    cx.update_window(window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();

    cx.run_until_parked();

    for _ in 0..15 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    Ok(test_result)
}

/// Visual test for harness maintenance on the External Agents settings page.
/// omega#81, `OMEGA-DELTA-0033`.
///
/// The gap omega#81 stayed open for was that every maintenance decision existed
/// and nothing rendered one: a refusal reached the owner only as agent-launch
/// error text, and there was no control to take or remove a pin. So the proof
/// that gap is closed has to be a picture of the row.
///
/// Nothing here is synthetic state handed to a widget. Two registry agents are
/// registered, their installed trees are written to the directories the launch
/// path measures, one is pinned to a different version through the same
/// `pin_installed_harness` the button calls, and the page is then opened and
/// photographed. What the screenshot shows is what the enforcement path
/// decided.
#[cfg(target_os = "macos")]
fn run_external_agent_maintenance_visual_tests(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    use ::collections::HashMap;
    use gpui::UpdateGlobal as _;
    use project::agent_registry_store::{
        AgentRegistryStore, RegistryAgent, RegistryAgentMetadata, RegistryBinaryAgent,
        RegistryTargetConfig,
    };
    use settings::SettingsStore;

    const HEALTHY: &str = "omega-visual-harness";
    const PINNED: &str = "omega-visual-pinned-harness";

    let platform_key = format!(
        "{}-{}",
        if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        },
        if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else {
            "x86_64"
        }
    );

    let registry_agent = |id: &str, name: &str, version: &str| {
        let mut targets = HashMap::default();
        targets.insert(
            platform_key.clone(),
            RegistryTargetConfig {
                archive: format!("https://example.invalid/{id}-{version}.tar.gz"),
                cmd: "./harness".to_string(),
                args: vec![],
                sha256: None,
                env: HashMap::default(),
            },
        );
        RegistryAgent::Binary(RegistryBinaryAgent {
            metadata: RegistryAgentMetadata {
                id: AgentId(id.to_string().into()),
                name: name.into(),
                description: "A wrapped ACP harness.".into(),
                version: version.into(),
                repository: None,
                website: None,
                icon_path: None,
            },
            targets,
            supports_current_platform: true,
        })
    };

    cx.update(|cx| {
        AgentRegistryStore::init_test_global(
            cx,
            vec![
                registry_agent(HEALTHY, "Visual Harness", "1.0.0"),
                // The registry now offers 1.1.0; the pin below freezes 1.0.0,
                // so this row is the refusal an owner has to be able to read.
                registry_agent(PINNED, "Pinned Harness", "1.1.0"),
            ],
        );
    });

    cx.update(|cx| {
        SettingsStore::update_global(cx, |store: &mut SettingsStore, cx| {
            store.update_user_settings(cx, |content| {
                let agent_servers = content.agent_servers.get_or_insert_default();
                for id in [HEALTHY, PINNED] {
                    agent_servers.insert(
                        id.to_string(),
                        settings::CustomAgentServerSettings::Registry {
                            default_mode: None,
                            env: Default::default(),
                            default_config_options: Default::default(),
                            favorite_config_option_values: Default::default(),
                        },
                    );
                }
            });
        });
    });

    // `OpenSettingsAt` reuses an open settings window, and an earlier test's
    // window is bound to an earlier test's project — which has no external
    // agents. Close them so the window this test opens is the one it set up.
    let existing: Vec<gpui::AnyWindowHandle> = cx
        .update(|cx| cx.windows())
        .into_iter()
        .filter(|window| window.downcast::<SettingsWindow>().is_some())
        .collect();
    for window in existing {
        cx.update_window(window, |_, window, _cx| {
            window.remove_window();
        })
        .log_err();
    }
    cx.run_until_parked();

    let window_size = size(px(900.0), px(700.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };

    let project = cx.update(|cx| {
        project::Project::local(
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            app_state.user_store.clone(),
            app_state.languages.clone(),
            app_state.fs.clone(),
            None,
            project::LocalProjectFlags {
                init_worktree_trust: false,
                ..Default::default()
            },
            cx,
        )
    });

    cx.run_until_parked();

    // The store owns the derivation of the measured tree, so the test asks it
    // rather than re-deriving one. A test that guessed the directory would
    // prove a row about a tree the launch path never looks at.
    let targets: Vec<(String, PathBuf, String)> = cx.update(|cx| {
        let store = project.read(cx).agent_server_store().clone();
        let store = store.read(cx);
        [HEALTHY, PINNED]
            .into_iter()
            .filter_map(|id| {
                let target = store.maintenance_target(&AgentId(id.to_string().into()))?;
                Some((
                    id.to_string(),
                    target.installed_dir?,
                    target.version.to_string(),
                ))
            })
            .collect()
    });
    anyhow::ensure!(
        targets.len() == 2,
        "the store did not offer a maintenance target for both registry agents"
    );

    for (_, dir, version) in &targets {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("harness"), format!("harness {version}"))?;
        std::fs::write(dir.join("LICENSE"), b"Apache-2.0")?;
    }

    // Attest the healthy one and freeze the other, through the production
    // functions the settings buttons call.
    let fs = app_state.fs.clone();
    let healthy = targets[0].clone();
    let pinned = targets[1].clone();
    // Driven on the test executor rather than by blocking this thread: the
    // filesystem work these functions do is scheduled by that executor, so
    // blocking the thread it runs on deadlocks instead of waiting.
    // `RealFs` does its work on real IO threads, so the deterministic scheduler
    // has to be allowed to park while it waits for them. This test runs last.
    cx.executor().allow_parking();

    let outcome: Arc<std::sync::Mutex<Option<Result<()>>>> = Arc::new(std::sync::Mutex::new(None));
    let setup = cx.update(|cx| {
        let outcome = outcome.clone();
        cx.background_spawn(async move {
            let result = async {
                project::harness_maintenance::reprobe_installed_harness(
                    fs.clone(),
                    &healthy.0,
                    &healthy.2,
                    &healthy.1,
                    1_784_894_400_000,
                )
                .await?;
                project::harness_maintenance::pin_installed_harness(
                    fs.clone(),
                    &pinned.0,
                    // Frozen at 1.0.0 while the registry advertises 1.1.0.
                    "1.0.0",
                    &pinned.1,
                    1_784_894_400_001,
                )
                .await?;
                anyhow::Ok(())
            }
            .await;
            *outcome.lock().unwrap() = Some(result);
        })
    });
    for _ in 0..200 {
        cx.run_until_parked();
        if outcome.lock().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    setup.detach();
    match outcome.lock().unwrap().take() {
        Some(Ok(())) => {}
        Some(Err(error)) => return Err(error).context("preparing harness maintenance state"),
        None => anyhow::bail!("harness maintenance setup did not finish"),
    }

    let workspace_window: WindowHandle<MultiWorkspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| {
                    let workspace = cx.new(|cx| {
                        Workspace::new(None, project.clone(), app_state.clone(), window, cx)
                    });
                    cx.new(|cx| MultiWorkspace::new(workspace, window, cx))
                },
            )
        })
        .context("Failed to open workspace window for external agents test")?;

    cx.run_until_parked();

    workspace_window
        .update(cx, |_workspace, window, cx| {
            window.dispatch_action(
                Box::new(OpenSettingsAt {
                    path: "agent_servers".to_string(),
                    target: None,
                }),
                cx,
            );
        })
        .context("Failed to dispatch OpenSettingsAt for external agents")?;

    cx.run_until_parked();
    for _ in 0..10 {
        cx.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
    }

    // Earlier tests leave their own settings windows open, so "the newest
    // window" is not reliably this test's. Take the newest one that is a
    // settings window.
    let all_windows = cx.update(|cx| cx.windows());
    let (settings_window, settings_window_handle) = all_windows
        .iter()
        .rev()
        .find_map(|window| Some((*window, window.downcast::<SettingsWindow>()?)))
        .context("No settings window found")?;

    settings_window_handle
        .update(cx, |settings_window, window, cx| {
            settings_window.navigate_to_sub_page("agent_servers", window, cx);
            settings_window.refresh_harness_maintenance_for_test(cx);
        })
        .context("Failed to navigate to the External Agents sub-page")?;

    cx.run_until_parked();
    for _ in 0..20 {
        cx.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
    }

    let result = run_visual_test(
        "external_agent_harness_maintenance",
        settings_window,
        cx,
        update_baseline,
    )?;

    cx.update_window(settings_window, |_, window, _cx| {
        window.remove_window();
    })
    .log_err();
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();
    cx.run_until_parked();

    Ok(result)
}
