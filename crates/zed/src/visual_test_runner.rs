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

    let test_result = std::panic::catch_unwind(|| run_visual_tests(project_path, update_baseline));

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

// All macOS-specific imports grouped together
#[cfg(target_os = "macos")]
use {
    acp_thread::{AgentConnection, StubAgentConnection},
    agent_client_protocol::schema::v1 as acp,
    agent_servers::{AgentServer, AgentServerDelegate},
    agent_ui::Agent,
    anyhow::{Context as _, Result},
    assets::Assets,
    editor::display_map::DisplayRow,
    feature_flags::FeatureFlagAppExt as _,
    git_ui::project_diff::ProjectDiff,
    gpui::{
        App, AppContext as _, Bounds, Entity, KeyBinding, Modifiers, VisualTestAppContext,
        WindowBounds, WindowHandle, WindowOptions, point, px, size,
    },
    image::RgbaImage,
    project::{AgentId, Project},
    project_panel::ProjectPanel,
    settings::{NotifyWhenAgentWaiting, PlaySoundWhenAgentDone, Settings as _},
    settings_ui::SettingsWindow,
    std::{
        any::Any,
        path::{Path, PathBuf},
        rc::Rc,
        sync::Arc,
        time::Duration,
    },
    util::ResultExt as _,
    workspace::{AppState, MultiWorkspace, Workspace},
    zed_actions::OpenSettingsAt,
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
    pub const BASELINE_DIR: &str = "crates/zed/test_fixtures/visual_tests";

    /// Embedded test image (Zed app icon) for visual tests.
    pub const EMBEDDED_TEST_IMAGE: &[u8] = include_bytes!("../resources/app-icon.png");

    /// Threshold for image comparison (0.0 to 1.0)
    /// Images must match at least this percentage to pass
    pub const MATCH_THRESHOLD: f64 = 0.99;

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
        call::init(app_state.client.clone(), app_state.user_store.clone(), cx);
        title_bar::init(cx);
        project_panel::init(cx);
        outline_panel::init(cx);
        terminal_view::init(cx);
        image_viewer::init(cx);
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
            println!("\n--- Omega: front door, executor disclosure, route pin ---");
            run_omega_agent_visual_tests(app_state.clone(), &mut cx, update_baseline)
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

    // Run Test 3: Multi-workspace sidebar visual tests
    println!("\n--- Test 3: multi_workspace_sidebar ---");
    match run_multi_workspace_sidebar_visual_tests(app_state.clone(), &mut cx, update_baseline) {
        Ok(TestResult::Passed) => {
            println!("✓ multi_workspace_sidebar: PASSED");
            passed += 1;
        }
        Ok(TestResult::BaselineUpdated(_)) => {
            println!("✓ multi_workspace_sidebar: Baselines updated");
            updated += 1;
        }
        Err(e) => {
            eprintln!("✗ multi_workspace_sidebar: FAILED - {}", e);
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

    // Omega's own rendered proofs: the front door with no project, typing
    // starting a thread, the executor line on three thread kinds, and a pin
    // that could not be honoured. omega#76, #77, #78.
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

    // Run Test: Sidebar with duplicate project names
    println!("\n--- Test: sidebar_duplicate_names ---");
    match run_sidebar_duplicate_project_names_visual_tests(
        app_state.clone(),
        &mut cx,
        update_baseline,
    ) {
        Ok(TestResult::Passed) => {
            println!("✓ sidebar_duplicate_names: PASSED");
            passed += 1;
        }
        Ok(TestResult::BaselineUpdated(_)) => {
            println!("✓ sidebar_duplicate_names: Baselines updated");
            updated += 1;
        }
        Err(e) => {
            eprintln!("✗ sidebar_duplicate_names: FAILED - {}", e);
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
fn run_visual_test(
    test_name: &str,
    window: gpui::AnyWindowHandle,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    // Ensure all pending work is done
    cx.run_until_parked();

    // Refresh the window to ensure it's fully rendered
    cx.update_window(window, |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    // Capture the screenshot using direct texture capture
    let screenshot = cx.capture_screenshot(window)?;

    // Get paths
    let baseline_path = get_baseline_path(test_name);
    let output_dir = std::env::var("VISUAL_TEST_OUTPUT_DIR")
        .unwrap_or_else(|_| "target/visual_tests".to_string());
    let output_path = PathBuf::from(&output_dir).join(format!("{}.png", test_name));

    // Ensure output directory exists
    std::fs::create_dir_all(&output_dir)?;

    // Always save the current screenshot
    screenshot.save(&output_path)?;
    println!("  Screenshot saved to: {}", output_path.display());

    if update_baseline {
        // Update the baseline
        if let Some(parent) = baseline_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        screenshot.save(&baseline_path)?;
        println!("  Baseline updated: {}", baseline_path.display());
        return Ok(TestResult::BaselineUpdated(baseline_path));
    }

    // Compare with baseline
    if !baseline_path.exists() {
        return Err(anyhow::anyhow!(
            "Baseline not found: {}. Run with UPDATE_BASELINE=1 to create it.",
            baseline_path.display()
        ));
    }

    let baseline = image::open(&baseline_path)?.to_rgba8();
    let comparison = compare_images(&screenshot, &baseline);

    println!(
        "  Match: {:.2}% ({} different pixels)",
        comparison.match_percentage * 100.0,
        comparison.diff_pixel_count
    );

    if comparison.match_percentage >= MATCH_THRESHOLD {
        Ok(TestResult::Passed)
    } else {
        // Save diff image
        let diff_path = PathBuf::from(&output_dir).join(format!("{}_diff.png", test_name));
        comparison.diff_image.save(&diff_path)?;
        println!("  Diff image saved to: {}", diff_path.display());

        Err(anyhow::anyhow!(
            "Image mismatch: {:.2}% match (threshold: {:.2}%)",
            comparison.match_percentage * 100.0,
            MATCH_THRESHOLD * 100.0
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
struct ImageComparison {
    match_percentage: f64,
    diff_image: RgbaImage,
    diff_pixel_count: u32,
    #[allow(dead_code)]
    total_pixels: u32,
}

#[cfg(target_os = "macos")]
fn compare_images(actual: &RgbaImage, expected: &RgbaImage) -> ImageComparison {
    let width = actual.width().max(expected.width());
    let height = actual.height().max(expected.height());
    let total_pixels = width * height;

    let mut diff_image = RgbaImage::new(width, height);
    let mut matching_pixels = 0u32;

    for y in 0..height {
        for x in 0..width {
            let actual_pixel = if x < actual.width() && y < actual.height() {
                *actual.get_pixel(x, y)
            } else {
                image::Rgba([0, 0, 0, 0])
            };

            let expected_pixel = if x < expected.width() && y < expected.height() {
                *expected.get_pixel(x, y)
            } else {
                image::Rgba([0, 0, 0, 0])
            };

            if pixels_are_similar(&actual_pixel, &expected_pixel) {
                matching_pixels += 1;
                // Semi-transparent green for matching pixels
                diff_image.put_pixel(x, y, image::Rgba([0, 255, 0, 64]));
            } else {
                // Bright red for differing pixels
                diff_image.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
    }

    let match_percentage = matching_pixels as f64 / total_pixels as f64;
    let diff_pixel_count = total_pixels - matching_pixels;

    ImageComparison {
        match_percentage,
        diff_image,
        diff_pixel_count,
        total_pixels,
    }
}

#[cfg(target_os = "macos")]
fn pixels_are_similar(a: &image::Rgba<u8>, b: &image::Rgba<u8>) -> bool {
    const TOLERANCE: i16 = 2;
    (a.0[0] as i16 - b.0[0] as i16).abs() <= TOLERANCE
        && (a.0[1] as i16 - b.0[1] as i16).abs() <= TOLERANCE
        && (a.0[2] as i16 - b.0[2] as i16).abs() <= TOLERANCE
        && (a.0[3] as i16 - b.0[3] as i16).abs() <= TOLERANCE
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
/// Comparison is `MATCH_THRESHOLD` (0.99): at least 99% of pixels must match
/// the committed baseline. Exact equality is not usable even here — font
/// rasterisation and theme colour rounding differ by a pixel or two between
/// machines — and a threshold nobody states is worse than a loose one.
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
fn run_omega_agent_visual_tests_inner(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    use agent_ui::AgentPanel;

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
        .context("Failed to open the Omega front door window")?;

    cx.run_until_parked();

    // The window really has no project. Asserted rather than assumed, because
    // every claim these captures make rests on it — a capture taken with a
    // worktree quietly present would prove the opposite of what it says.
    let visible_worktrees = workspace_window
        .update(cx, |workspace, _window, cx| {
            workspace.project().read(cx).visible_worktrees(cx).count()
        })
        .context("Failed to read the project's worktrees")?;
    anyhow::ensure!(
        visible_worktrees == 0,
        "the front door capture needs a projectless window; found {visible_worktrees} worktree(s)"
    );

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

    // The shipped front door, called the way `crates/zed/src/main.rs` calls it
    // on a window with nothing to restore. Not a hand-rolled approximation of
    // it: `open_front_door` is the entry `OMEGA-DELTA-0019` added, and driving
    // anything else here would photograph a path no user takes.
    workspace_window
        .update(cx, |_workspace, window, cx| {
            AgentPanel::open_front_door(window, cx);
        })
        .context("Failed to open the front door")?;

    cx.run_until_parked();

    // The panel must actually be the focused, visible dock surface. A capture
    // taken with the panel absent shows the launchpad and would read as a
    // perfectly plausible screenshot of something else entirely — which is what
    // the first run of this test produced, and how it was caught.
    let panel_is_open = workspace_window
        .update(cx, |workspace, _window, cx| {
            workspace
                .panel::<AgentPanel>(cx)
                .is_some_and(|panel| panel.read(cx).active_thread_id(cx).is_some())
        })
        .unwrap_or(false);
    anyhow::ensure!(
        panel_is_open,
        "the agent panel is not the open surface; the capture would show the \
         launchpad rather than the front door"
    );

    // omega#76's exit, first half. Before this delta the panel answered a
    // projectless window with "Open Project / Clone Repository" and there was
    // nothing to type into.
    let thread_id = cx
        .read(|cx| panel.read(cx).active_thread_id(cx))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "the front door produced no thread on a projectless window — \
                 this is the omega#76 defect, and the capture below would show \
                 the empty-project state"
            )
        })?;

    let front_door = run_visual_test(
        "omega_front_door_no_project",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // omega#76's exit, second half: **typing starts a real thread**. The
    // keystrokes go into this window through GPUI's own dispatch, so nothing
    // depends on which application macOS thinks is frontmost.
    cx.simulate_input(workspace_window.into(), "route this thread on purpose");
    cx.run_until_parked();

    let typed = cx
        .read(|cx| {
            panel
                .read(cx)
                .active_thread_view_for_tests()
                .and_then(|conversation| conversation.read(cx).root_thread_view())
                .map(|view| view.read(cx).message_editor.read(cx).text(cx))
        })
        .unwrap_or_default();
    anyhow::ensure!(
        typed.contains("route this thread on purpose"),
        "typing on the front door did not reach the thread's composer; the \
         editor holds {typed:?}"
    );

    let typing = run_visual_test(
        "omega_front_door_typing",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // Zoom the panel for the disclosure captures. The executor line is long by
    // design — class, agent, model, run, and route reason — and a dock-width
    // capture truncates it with an ellipsis, which would make a picture of a
    // *truncated* line the evidence that the line renders.
    cx.update_window(workspace_window.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            use workspace::dock::Panel as _;
            panel.set_zoomed(true, window, cx);
        });
    })
    .log_err();
    cx.run_until_parked();

    // omega#77: the executor line, on the native thread the front door just
    // built. This thread was routed by `OmegaAgentConnection`, so its line also
    // carries omega#78's route reason.
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
    anyhow::ensure!(
        native_line.contains("routed:"),
        "omega#78's wiring is not reaching the rendered line: {native_line:?}"
    );
    println!("  native executor line: {native_line}");

    let native_disclosure = run_visual_test(
        "omega_executor_disclosure_native",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // omega#78, first exit clause: **a pinned executor is always honoured.**
    // The native loop is the one executor this build has registered on the
    // router, so it is the one honourable pin — and pinning it is not a no-op:
    // the line changes from `routed: unpinned` to `routed: pinned`, which is
    // the difference between "nobody chose this" and "a person did".
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
    anyhow::ensure!(
        honoured.reason == omega_front_door::RouteReason::PinHonored,
        "a pin to the fail-closed target must be honoured, not {:?}",
        honoured.reason
    );
    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.refresh();
    })?;
    cx.run_until_parked();

    let honoured_line = cx
        .read(|cx| omega_executor_line(&panel, cx))
        .ok_or_else(|| anyhow::anyhow!("the pinned thread has no executor disclosure"))?;
    anyhow::ensure!(
        honoured_line.contains("routed: pinned"),
        "an honoured pin must say so on the thread's own line: {honoured_line:?}"
    );
    println!("  honoured-pin executor line: {honoured_line}");

    let pin_honoured = run_visual_test(
        "omega_route_pin_honoured",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // omega#78: a pin that cannot be honoured, rendered. No engine is running
    // under this harness, so an engine-lane pin falls closed to the native loop
    // and the line has to say so — a fallback the user cannot see is the defect
    // this packet exists to avoid.
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

    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.refresh();
    })?;
    cx.run_until_parked();

    let pinned_line = cx
        .read(|cx| omega_executor_line(&panel, cx))
        .ok_or_else(|| anyhow::anyhow!("the pinned thread has no executor disclosure"))?;
    anyhow::ensure!(
        pinned_line.contains("fell back to the native loop"),
        "the unhonoured pin is not visible on the thread's line: {pinned_line:?}"
    );
    println!("  unhonoured-pin executor line: {pinned_line}");

    let pin_fallback = run_visual_test(
        "omega_route_pin_not_honoured",
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // omega#77: the external-ACP kind, on a second thread in the same panel.
    let stub: Rc<dyn AgentServer> = Rc::new(StubAgentServer::new(StubAgentConnection::new()));
    cx.update_window(workspace_window.into(), |_, window, cx| {
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
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

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
    cx.update_window(workspace_window.into(), |_, window, _cx| {
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
        workspace_window.into(),
        cx,
        update_baseline,
    )?;

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
    cx.update_window(workspace_window.into(), |_, window, cx| {
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

    cx.update_window(workspace_window.into(), |_, window, _cx| {
        window.remove_window();
    })
    .log_err();
    cx.run_until_parked();

    // Six captures, and `run_visual_test` returns `Err` on a mismatch, so
    // reaching here means every one of them matched its baseline or wrote one.
    // The aggregate reports "baseline updated" if any capture wrote one, so an
    // `UPDATE_BASELINE=1` run is never reported as a pass.
    let results = [
        front_door,
        typing,
        native_disclosure,
        pin_honoured,
        pin_fallback,
        lane_disclosure,
        external_disclosure,
    ];
    for result in &results {
        if let TestResult::BaselineUpdated(path) = result {
            return Ok(TestResult::BaselineUpdated(path.clone()));
        }
    }
    Ok(TestResult::Passed)
}

/// The executor line the agent panel's active thread would render.
///
/// Read through the same `ThreadView::executor_disclosure` the render calls, so
/// the assertion and the pixels cannot disagree about what the line says.
#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn omega_executor_line(panel: &Entity<agent_ui::AgentPanel>, cx: &App) -> Option<String> {
    Some(omega_executor_record(panel, cx)?.label())
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
}

#[cfg(target_os = "macos")]
impl StubAgentServer {
    fn new(connection: StubAgentConnection) -> Self {
        Self { connection }
    }
}

#[cfg(target_os = "macos")]
impl AgentServer for StubAgentServer {
    fn logo(&self) -> ui::IconName {
        ui::IconName::OmegaAssistant
    }

    fn agent_id(&self) -> AgentId {
        "Visual Test Agent".into()
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
    let wide = run_omega_exo_visual_capture(
        app_state.clone(),
        cx,
        lane_path.clone(),
        "omega_exo_workspace_wide",
        size(px(1320.), px(860.)),
        ExoSceneSurface::FullEditor,
        update_baseline,
    )?;
    let narrow = run_omega_exo_visual_capture(
        app_state.clone(),
        cx,
        lane_path.clone(),
        "omega_exo_workspace_narrow",
        size(px(720.), px(900.)),
        ExoSceneSurface::FullEditor,
        update_baseline,
    )?;

    // omega#99. The zero-base scenes come last, and the order is the mechanism.
    // `omega_zero_base` is a process-global entered once and, since
    // `OMEGA-DELTA-0052`, never left, so the two scenes above are ordinary only
    // because they were taken before this line. That is what makes the pair of
    // pairs worth having, and `run_omega_exo_visual_capture` asserts it per
    // scene rather than trusting the reading order of this function.
    //
    // OMEGA-DELTA-0052. Zero base is now the shipped default for `omega`, and
    // this runner is unaffected by that: it is a separate binary with its own
    // `main`, it never parses `Args`, and the only thing that turns the mode on
    // in this process is the call below. So the ordinary-surface baselines still
    // photograph something that happens — a person who starts Omega with
    // `--full-editor` sees exactly it.
    omega_zero_base::enter_from_command_line();
    anyhow::ensure!(
        omega_zero_base::is_active(),
        "the zero-base scenes must be photographed with the mode actually on"
    );
    let zero_base_wide = run_omega_exo_visual_capture(
        app_state.clone(),
        cx,
        lane_path.clone(),
        "omega_zero_base_wide",
        size(px(1320.), px(860.)),
        ExoSceneSurface::ZeroBase,
        update_baseline,
    )?;
    let zero_base_narrow = run_omega_exo_visual_capture(
        app_state,
        cx,
        lane_path,
        "omega_zero_base_narrow",
        size(px(720.), px(900.)),
        ExoSceneSurface::ZeroBase,
        update_baseline,
    )?;

    for result in [&wide, &narrow, &zero_base_wide, &zero_base_narrow] {
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

/// Which Omega surface a scene photographs the Exo turn on.
///
/// omega#99. Named rather than a `bool` because the call sites read at a
/// glance, and because the difference is the whole point of the second pair:
/// the same real Exo turn, on the ordinary surface and on the subtracted one.
#[cfg(all(target_os = "macos", feature = "visual-tests"))]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExoSceneSurface {
    FullEditor,
    ZeroBase,
}

#[cfg(all(target_os = "macos", feature = "visual-tests"))]
fn run_omega_exo_visual_capture(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    lane_path: PathBuf,
    test_name: &'static str,
    window_size: gpui::Size<gpui::Pixels>,
    surface: ExoSceneSurface,
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

    // OMEGA-DELTA-0052, omega#100. Zero base's status bar is empty now.
    //
    // This is where the runner used to call `install_on_workspace` to add the
    // mode's one status-bar control before capturing. There is no control, so
    // there is nothing to install, and the scene is what the shipped binary
    // draws for the same reason it was before: the mode flag is on and the
    // surfaces that read it render the subtracted form.
    //
    // What is left is worth asserting, because it is the property the pair of
    // pairs rests on and it is now the *only* thing that separates them. The
    // mode is a process global, entered once and never left, so a scene ordered
    // wrongly would photograph the subtracted surface and file it under the
    // ordinary one — and the baseline would look plausible.
    anyhow::ensure!(
        omega_zero_base::is_active() == (surface == ExoSceneSurface::ZeroBase),
        "{test_name} asks for {}, and zero base is {}",
        match surface {
            ExoSceneSurface::ZeroBase => "the subtracted surface",
            ExoSceneSurface::FullEditor => "the ordinary surface",
        },
        if omega_zero_base::is_active() {
            "on"
        } else {
            "off"
        }
    );
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
    use settings::ToolPermissionMode;
    use zed_actions::OpenSettingsAt;

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
fn run_multi_workspace_sidebar_visual_tests(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    // Create temporary directories to act as worktrees for active workspaces
    let temp_dir = tempfile::tempdir()?;
    let temp_path = temp_dir.keep();
    let canonical_temp = temp_path.canonicalize()?;

    let workspace1_dir = canonical_temp.join("private-test-remote");
    let workspace2_dir = canonical_temp.join("zed");
    std::fs::create_dir_all(&workspace1_dir)?;
    std::fs::create_dir_all(&workspace2_dir)?;

    // Create both projects upfront so we can build both workspaces during
    // window creation, before the MultiWorkspace entity exists.
    // This avoids a re-entrant read panic that occurs when Workspace::new
    // tries to access the window root (MultiWorkspace) while it's being updated.
    let project1 = cx.update(|cx| {
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

    let project2 = cx.update(|cx| {
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

    let window_size = size(px(1280.0), px(800.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };

    // Open a MultiWorkspace window with both workspaces created at construction time
    let multi_workspace_window: WindowHandle<MultiWorkspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| {
                    let workspace1 = cx.new(|cx| {
                        Workspace::new(None, project1.clone(), app_state.clone(), window, cx)
                    });
                    let workspace2 = cx.new(|cx| {
                        Workspace::new(None, project2.clone(), app_state.clone(), window, cx)
                    });
                    cx.new(|cx| {
                        let mut multi_workspace = MultiWorkspace::new(workspace1, window, cx);
                        multi_workspace.activate(workspace2, None, window, cx);
                        multi_workspace
                    })
                },
            )
        })
        .context("Failed to open MultiWorkspace window")?;

    cx.run_until_parked();

    // Add worktree to workspace 1 (index 0) so it shows as "private-test-remote"
    let add_worktree1_task = multi_workspace_window
        .update(cx, |multi_workspace, _window, cx| {
            let workspace1 = multi_workspace.workspaces().next().unwrap();
            let project = workspace1.read(cx).project().clone();
            project.update(cx, |project, cx| {
                project.find_or_create_worktree(&workspace1_dir, true, cx)
            })
        })
        .context("Failed to start adding worktree 1")?;

    cx.background_executor.allow_parking();
    cx.foreground_executor
        .block_test(add_worktree1_task)
        .context("Failed to add worktree 1")?;
    cx.background_executor.forbid_parking();

    cx.run_until_parked();

    // Add worktree to workspace 2 (index 1) so it shows as "zed"
    let add_worktree2_task = multi_workspace_window
        .update(cx, |multi_workspace, _window, cx| {
            let workspace2 = multi_workspace.workspaces().nth(1).unwrap();
            let project = workspace2.read(cx).project().clone();
            project.update(cx, |project, cx| {
                project.find_or_create_worktree(&workspace2_dir, true, cx)
            })
        })
        .context("Failed to start adding worktree 2")?;

    cx.background_executor.allow_parking();
    cx.foreground_executor
        .block_test(add_worktree2_task)
        .context("Failed to add worktree 2")?;
    cx.background_executor.forbid_parking();

    cx.run_until_parked();

    // Switch to workspace 1 so it's highlighted as active (index 0)
    multi_workspace_window
        .update(cx, |multi_workspace, window, cx| {
            let workspace = multi_workspace.workspaces().next().unwrap().clone();
            multi_workspace.activate(workspace, None, window, cx);
        })
        .context("Failed to activate workspace 1")?;

    cx.run_until_parked();

    // Create the sidebar outside the MultiWorkspace update to avoid a
    // re-entrant read panic (Sidebar::new reads the MultiWorkspace).
    let sidebar = cx
        .update_window(multi_workspace_window.into(), |root_view, window, cx| {
            let multi_workspace_handle: Entity<MultiWorkspace> = root_view.downcast().unwrap();
            cx.new(|cx| sidebar::Sidebar::new(multi_workspace_handle, window, cx))
        })
        .context("Failed to create sidebar")?;

    multi_workspace_window
        .update(cx, |multi_workspace, _window, cx| {
            multi_workspace.register_sidebar(sidebar.clone(), cx);
        })
        .context("Failed to register sidebar")?;

    cx.run_until_parked();

    // Save test threads to the ThreadStore for each workspace
    let save_tasks = multi_workspace_window
        .update(cx, |multi_workspace, _window, cx| {
            let thread_store = agent::ThreadStore::global(cx);
            let workspaces: Vec<_> = multi_workspace.workspaces().cloned().collect();
            let mut tasks = Vec::new();

            for (index, workspace) in workspaces.iter().enumerate() {
                let workspace_ref = workspace.read(cx);
                let mut paths = Vec::new();
                for worktree in workspace_ref.worktrees(cx) {
                    let worktree_ref = worktree.read(cx);
                    if worktree_ref.is_visible() {
                        paths.push(worktree_ref.abs_path().to_path_buf());
                    }
                }
                let path_list = util::path_list::PathList::new(&paths);

                let (session_id, title, updated_at) = match index {
                    0 => (
                        "visual-test-thread-0",
                        "Refine thread view scrolling behavior",
                        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2024, 6, 15, 10, 30, 0)
                            .unwrap(),
                    ),
                    1 => (
                        "visual-test-thread-1",
                        "Add line numbers option to FileEditBlock",
                        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2024, 6, 15, 11, 0, 0)
                            .unwrap(),
                    ),
                    _ => continue,
                };

                let task = thread_store.update(cx, |store, cx| {
                    store.save_thread(
                        acp::SessionId::new(Arc::from(session_id)),
                        agent::DbThread {
                            title: title.to_string().into(),
                            messages: Vec::new(),
                            updated_at,
                            detailed_summary: None,
                            initial_project_snapshot: None,
                            cumulative_token_usage: Default::default(),
                            request_token_usage: Default::default(),
                            model: None,
                            profile: None,
                            subagent_context: None,
                            speed: None,
                            thinking_enabled: false,
                            thinking_effort: None,
                            ui_scroll_position: None,
                            draft_prompt: None,
                            sandboxed_terminal_temp_dir: None,
                            sandbox_grants: Default::default(),
                        },
                        path_list,
                        cx,
                    )
                });
                tasks.push(task);
            }
            tasks
        })
        .context("Failed to create test threads")?;

    cx.background_executor.allow_parking();
    for task in save_tasks {
        cx.foreground_executor
            .block_test(task)
            .context("Failed to save test thread")?;
    }
    cx.background_executor.forbid_parking();

    cx.run_until_parked();

    // Open the sidebar
    multi_workspace_window
        .update(cx, |multi_workspace, window, cx| {
            multi_workspace.toggle_sidebar(window, cx);
        })
        .context("Failed to toggle sidebar")?;

    // Let rendering settle
    for _ in 0..10 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Refresh the window
    cx.update_window(multi_workspace_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    // Capture: sidebar open with active workspaces and recent projects
    let test_result = run_visual_test(
        "multi_workspace_sidebar_open",
        multi_workspace_window.into(),
        cx,
        update_baseline,
    )?;

    // Clean up worktrees
    multi_workspace_window
        .update(cx, |multi_workspace, _window, cx| {
            for workspace in multi_workspace.workspaces() {
                let project = workspace.read(cx).project().clone();
                project.update(cx, |project, cx| {
                    let worktree_ids: Vec<_> =
                        project.worktrees(cx).map(|wt| wt.read(cx).id()).collect();
                    for id in worktree_ids {
                        project.remove_worktree(id, cx);
                    }
                });
            }
        })
        .log_err();

    cx.run_until_parked();

    // Close the window
    cx.update_window(multi_workspace_window.into(), |_, window, _cx| {
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

#[cfg(target_os = "macos")]
/// Helper to create a project, add a worktree at the given path, and return the project.
fn create_project_with_worktree(
    worktree_dir: &Path,
    app_state: &Arc<AppState>,
    cx: &mut VisualTestAppContext,
) -> Result<Entity<Project>> {
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

    let add_task = cx.update(|cx| {
        project.update(cx, |project, cx| {
            project.find_or_create_worktree(worktree_dir, true, cx)
        })
    });

    cx.background_executor.allow_parking();
    cx.foreground_executor
        .block_test(add_task)
        .context("Failed to add worktree")?;
    cx.background_executor.forbid_parking();

    cx.run_until_parked();
    Ok(project)
}

#[cfg(target_os = "macos")]
fn open_sidebar_test_window(
    projects: Vec<Entity<Project>>,
    app_state: &Arc<AppState>,
    cx: &mut VisualTestAppContext,
) -> Result<WindowHandle<MultiWorkspace>> {
    anyhow::ensure!(!projects.is_empty(), "need at least one project");

    let window_size = size(px(400.0), px(600.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size: window_size,
    };

    let mut projects_iter = projects.into_iter();
    let first_project = projects_iter
        .next()
        .ok_or_else(|| anyhow::anyhow!("need at least one project"))?;
    let remaining: Vec<_> = projects_iter.collect();

    let multi_workspace_window: WindowHandle<MultiWorkspace> = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                |window, cx| {
                    let first_ws = cx.new(|cx| {
                        Workspace::new(None, first_project.clone(), app_state.clone(), window, cx)
                    });
                    cx.new(|cx| {
                        let mut mw = MultiWorkspace::new(first_ws, window, cx);
                        for project in remaining {
                            let ws = cx.new(|cx| {
                                Workspace::new(None, project, app_state.clone(), window, cx)
                            });
                            mw.activate(ws, None, window, cx);
                        }
                        mw
                    })
                },
            )
        })
        .context("Failed to open MultiWorkspace window")?;

    cx.run_until_parked();

    // Create the sidebar outside the MultiWorkspace update to avoid a
    // re-entrant read panic (Sidebar::new reads the MultiWorkspace).
    let sidebar = cx
        .update_window(multi_workspace_window.into(), |root_view, window, cx| {
            let mw_handle: Entity<MultiWorkspace> = root_view
                .downcast()
                .map_err(|_| anyhow::anyhow!("Failed to downcast root view to MultiWorkspace"))?;
            Ok::<_, anyhow::Error>(cx.new(|cx| sidebar::Sidebar::new(mw_handle, window, cx)))
        })
        .context("Failed to create sidebar")??;

    multi_workspace_window
        .update(cx, |mw, _window, cx| {
            mw.register_sidebar(sidebar.clone(), cx);
        })
        .context("Failed to register sidebar")?;

    cx.run_until_parked();

    // Open the sidebar
    multi_workspace_window
        .update(cx, |mw, window, cx| {
            mw.toggle_sidebar(window, cx);
        })
        .context("Failed to toggle sidebar")?;

    // Let rendering settle
    for _ in 0..10 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    // Refresh the window
    cx.update_window(multi_workspace_window.into(), |_, window, _cx| {
        window.refresh();
    })?;

    cx.run_until_parked();

    Ok(multi_workspace_window)
}

#[cfg(target_os = "macos")]
fn cleanup_sidebar_test_window(
    window: WindowHandle<MultiWorkspace>,
    cx: &mut VisualTestAppContext,
) -> Result<()> {
    window.update(cx, |mw, _window, cx| {
        for workspace in mw.workspaces() {
            let project = workspace.read(cx).project().clone();
            project.update(cx, |project, cx| {
                let ids: Vec<_> = project.worktrees(cx).map(|wt| wt.read(cx).id()).collect();
                for id in ids {
                    project.remove_worktree(id, cx);
                }
            });
        }
    })?;

    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, _cx| {
        window.remove_window();
    })?;

    cx.run_until_parked();

    for _ in 0..15 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn run_sidebar_duplicate_project_names_visual_tests(
    app_state: Arc<AppState>,
    cx: &mut VisualTestAppContext,
    update_baseline: bool,
) -> Result<TestResult> {
    let temp_dir = tempfile::tempdir()?;
    let temp_path = temp_dir.keep();
    let canonical_temp = temp_path.canonicalize()?;

    // Create directory structure where every leaf directory is named "zed" but
    // lives at a distinct path. This lets us test that the sidebar correctly
    // disambiguates projects whose names would otherwise collide.
    //
    //   code/zed/       — project1 (single worktree)
    //   code/foo/zed/   — project2 (single worktree)
    //   code/bar/zed/   — project3, first worktree
    //   code/baz/zed/   — project3, second worktree
    //
    // No two projects share a worktree path, so ProjectGroupBuilder will
    // place each in its own group.
    let code_zed = canonical_temp.join("code").join("zed");
    let foo_zed = canonical_temp.join("code").join("foo").join("zed");
    let bar_zed = canonical_temp.join("code").join("bar").join("zed");
    let baz_zed = canonical_temp.join("code").join("baz").join("zed");
    std::fs::create_dir_all(&code_zed)?;
    std::fs::create_dir_all(&foo_zed)?;
    std::fs::create_dir_all(&bar_zed)?;
    std::fs::create_dir_all(&baz_zed)?;

    cx.update(|cx| {
        cx.update_flags(true, vec!["agent-v2".to_string()]);
    });

    let mut has_baseline_update = None;

    // Two single-worktree projects whose leaf name is "zed"
    {
        let project1 = create_project_with_worktree(&code_zed, &app_state, cx)?;
        let project2 = create_project_with_worktree(&foo_zed, &app_state, cx)?;

        let window = open_sidebar_test_window(vec![project1, project2], &app_state, cx)?;

        let result = run_visual_test(
            "sidebar_two_projects_same_leaf_name",
            window.into(),
            cx,
            update_baseline,
        );

        cleanup_sidebar_test_window(window, cx)?;
        match result? {
            TestResult::Passed => {}
            TestResult::BaselineUpdated(path) => {
                has_baseline_update = Some(path);
            }
        }
    }

    // Three projects, third has two worktrees (all leaf names "zed")
    //
    // project1: code/zed
    // project2: code/foo/zed
    // project3: code/bar/zed + code/baz/zed
    //
    // Each project has a unique set of worktree paths, so they form
    // separate groups. The sidebar must disambiguate all three.
    {
        let project1 = create_project_with_worktree(&code_zed, &app_state, cx)?;
        let project2 = create_project_with_worktree(&foo_zed, &app_state, cx)?;

        let project3 = create_project_with_worktree(&bar_zed, &app_state, cx)?;
        let add_second_worktree = cx.update(|cx| {
            project3.update(cx, |project, cx| {
                project.find_or_create_worktree(&baz_zed, true, cx)
            })
        });
        cx.background_executor.allow_parking();
        cx.foreground_executor
            .block_test(add_second_worktree)
            .context("Failed to add second worktree to project 3")?;
        cx.background_executor.forbid_parking();
        cx.run_until_parked();

        let window = open_sidebar_test_window(vec![project1, project2, project3], &app_state, cx)?;

        let result = run_visual_test(
            "sidebar_three_projects_with_multi_worktree",
            window.into(),
            cx,
            update_baseline,
        );

        cleanup_sidebar_test_window(window, cx)?;
        match result? {
            TestResult::Passed => {}
            TestResult::BaselineUpdated(path) => {
                has_baseline_update = Some(path);
            }
        }
    }

    if let Some(path) = has_baseline_update {
        Ok(TestResult::BaselineUpdated(path))
    } else {
        Ok(TestResult::Passed)
    }
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
