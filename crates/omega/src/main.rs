// Disable command line from opening on release mode
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod omega_zero_base_ui;
mod plugins;
mod reliability;
mod zed;

// Ensure the binary name stays in sync with the application identity.
const _: () = assert!(
    app_identity::BINARY_NAME
        .as_bytes()
        .eq_ignore_ascii_case(env!("CARGO_BIN_NAME").as_bytes()),
    "app_identity::BINARY_NAME must match the binary name.",
);

use agent_ui::AgentPanel;
use anyhow::{Context as _, Result};
use clap::Parser;
use cli::FORCE_CLI_MODE_ENV_VAR_NAME;
use client::{Client, ProxySettings, RefreshLlmTokenListener, UserStore, parse_zed_link};
use collections::HashMap;
use crashes::InitCrashHandler;
use db::kvp::{GlobalKeyValueStore, KeyValueStore};
use extension::ExtensionHostProxy;
use fs::{Fs, RealFs};
use futures::{FutureExt, StreamExt, channel::oneshot};
use git::GitHostingProviderRegistry;
use git_ui::clone::clone_and_open;
use gpui::{
    App, AppContext, Application, AsyncApp, QuitMode, Task, TaskExt, UpdateGlobal as _, block_on,
};
use gpui_platform;

use gpui_tokio::Tokio;
use language::LanguageRegistry;
use onboarding::await_identity_ready;
use project_panel::ProjectPanel;
use prompt_store::PromptBuilder;
use remote::RemoteConnectionOptions;
use reqwest_client::ReqwestClient;

use assets::Assets;
use node_runtime::{NodeBinaryOptions, NodeRuntime};
use parking_lot::Mutex;
use project::{project_settings::ProjectSettings, trusted_worktrees};
use release_channel::{AppCommitSha, AppVersion, ReleaseChannel};
use session::{AppSession, Session};
use settings::{BaseKeymap, Settings, SettingsStore, watch_config_file};
use smol::future::poll_once;
use std::{
    cell::RefCell,
    env,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process,
    rc::Rc,
    sync::{Arc, LazyLock, OnceLock},
    time::Instant,
};
use theme::{ActiveTheme, GlobalTheme, ThemeRegistry};
use theme_settings::load_user_theme;
use util::{ResultExt, maybe};
use uuid::Uuid;
use workspace::{
    AppState, MultiWorkspace, SerializedWorkspaceLocation, SessionWorkspace, Toast,
    WorkspaceSettings, WorkspaceStore, notifications::NotificationId, restore_multiworkspace,
};
use zed::remote_connections::{RemoteSettings, open_remote_project};
use zed::{
    OpenListener, OpenRequest, RawOpenRequest, app_menus, build_window_options,
    derive_paths_with_position, handle_cli_connection, handle_keymap_file_changes,
    initialize_workspace, open_paths_with_positions,
};

use crate::zed::{CrashHandler, OpenRequestKind, eager_load_active_theme_and_icon_theme};

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn build_application() -> Application {
    let platform = gpui_platform::current_platform(false);
    Application::with_platform(platform)
}

fn files_not_created_on_launch(errors: HashMap<io::ErrorKind, Vec<&Path>>) {
    let message = "Omega failed to launch";
    let error_details = errors
        .into_iter()
        .flat_map(|(kind, paths)| {
            #[allow(unused_mut)] // for non-unix platforms
            let mut error_kind_details = match paths.len() {
                0 => return None,
                1 => format!(
                    "{kind} when creating directory {:?}",
                    paths.first().expect("match arm checks for a single entry")
                ),
                _many => format!("{kind} when creating directories {paths:?}"),
            };

            #[cfg(unix)]
            {
                if kind == io::ErrorKind::PermissionDenied {
                    error_kind_details.push_str("\n\nConsider using chown and chmod tools for altering the directories permissions if your user has corresponding rights.\
                        \nFor example, `sudo chown $(whoami):staff ~/.config` and `chmod +uwrx ~/.config`");
                }
            }

            Some(error_kind_details)
        })
        .collect::<Vec<_>>().join("\n\n");

    eprintln!("{message}: {error_details}");
    build_application()
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            if let Ok(window) = cx.open_window(gpui::WindowOptions::default(), |_, cx| {
                cx.new(|_| gpui::Empty)
            }) {
                window
                    .update(cx, |_, window, cx| {
                        let response = window.prompt(
                            gpui::PromptLevel::Critical,
                            message,
                            Some(&error_details),
                            &["Exit"],
                            cx,
                        );

                        cx.spawn_in(window, async move |_, cx| {
                            response.await?;
                            cx.update(|_, cx| cx.quit())
                        })
                        .detach_and_log_err(cx);
                    })
                    .log_err();
            } else {
                fail_to_open_window(anyhow::anyhow!("{message}: {error_details}"), cx)
            }
        })
}

fn fail_to_open_window_async(e: anyhow::Error, cx: &mut AsyncApp) {
    cx.update(|cx| fail_to_open_window(e, cx));
}

fn fail_to_open_window(e: anyhow::Error, _cx: &mut App) {
    eprintln!(
        "Omega failed to open a window: {e:?}. See {} for troubleshooting steps.",
        app_identity::PRODUCT_DOCS_URL
    );
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        process::exit(1);
    }

    // Maybe unify this with gpui::platform::linux::platform::ResultExt::notify_err(..)?
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        use ashpd::desktop::notification::{Notification, NotificationProxy, Priority};
        _cx.spawn(async move |_cx| {
            let Ok(proxy) = NotificationProxy::new().await else {
                process::exit(1);
            };

            let notification_id = "com.openagents.omega.Oops";
            proxy
                .add_notification(
                    notification_id,
                    Notification::new("Omega failed to launch")
                        .body(Some(
                            format!(
                                "{e:?}. See {} for troubleshooting steps.",
                                app_identity::PRODUCT_DOCS_URL
                            )
                            .as_str(),
                        ))
                        .priority(Priority::High)
                        .icon(ashpd::desktop::Icon::with_names(&[
                            "dialog-question-symbolic",
                        ])),
                )
                .await
                .ok();

            process::exit(1);
        })
        .detach();
    }
}

static STARTUP_TIME: OnceLock<Instant> = OnceLock::new();

fn main() {
    STARTUP_TIME.get_or_init(|| Instant::now());

    // If this process was re-executed as a Linux sandbox helper, run that mode
    // without returning. Must run before argument parsing: the wrapped command's
    // args are appended verbatim and would otherwise be misinterpreted as Zed's
    // own arguments.
    sandbox::run_sandbox_launcher_if_invoked();

    #[cfg(unix)]
    util::prevent_root_execution();

    // omega#161. The full-editor mode split is removed: Omega has one surface,
    // the flag-free launch. For one release a stale invocation gets this named
    // error instead of clap's plain unknown-argument refusal, so a script that
    // carried the flag learns what happened rather than guessing at a typo.
    for removed in [
        "--full-editor",
        "--diff",
        "--dev-container",
        "--demo-workroom",
    ] {
        if std::env::args().any(|argument| argument == removed) {
            eprintln!(
                "{removed} was removed (omega#161): Omega has one surface now, so run `omega` without it."
            );
            process::exit(1);
        }
    }

    let mut args = Args::parse();
    // OMEGA-DELTA-0052. The primary interface is the application, including
    // for a plain local Cargo build. The hidden argument remains accepted for
    // compatibility, but no build-time marker or launch flag selects a second
    // presentation.
    args.primary_interface = true;

    // `zed --askpass` Makes zed operate in nc/netcat mode for use with askpass
    #[cfg(not(target_os = "windows"))]
    if let Some(socket) = &args.askpass {
        askpass::main(socket);
        return;
    }

    // `zed --crash-handler` Makes zed operate in minidump crash handler mode
    if let Some(socket) = &args.crash_handler {
        crashes::crash_server(socket.as_path(), paths::logs_dir().clone());
        return;
    }

    #[cfg(target_os = "windows")]
    if args.record_etw_trace {
        let zed_pid = args
            .etw_zed_pid
            .and_then(|pid| if pid >= 0 { Some(pid as u32) } else { None });
        let Some(output_path) = args.etw_output else {
            eprintln!("--etw-output is required for --record-etw-trace");
            process::exit(1);
        };

        let Some(etw_socket) = args.etw_socket else {
            eprintln!("--etw-socket is required for --record-etw-trace");
            process::exit(1);
        };

        if let Err(error) =
            etw_tracing::record_etw_trace(zed_pid, &output_path, etw_socket.as_str())
        {
            eprintln!("ETW trace recording failed: {error:#}");
            process::exit(1);
        }
        return;
    }

    #[cfg(all(not(debug_assertions), target_os = "windows"))]
    unsafe {
        use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

        if args.foreground {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }

    // `zed --printenv` Outputs environment variables as JSON to stdout
    if args.printenv {
        util::shell_env::print_env();
        return;
    }

    if args.dump_all_actions {
        dump_all_gpui_actions();
        return;
    }

    // OMEGA-DELTA-0144. Exo is absent unless the person launching this exact
    // process opts in. Read this before paths, logs, settings, or agent UI
    // initialization so none of those surfaces can discover or start Exo on a
    // default launch.
    if args.enable_exo {
        omega_front_door::enable_exo_from_command_line();
    }

    if args.primary_interface {
        omega_zero_base::enable_primary_interface();
    }

    // omega#161. Zero base remains the application and `--zero-base` remains
    // a compatibility no-op. The primary interface is an additive scaffold
    // inside that seal, not a route back to the removed editor surface.
    //
    // OMEGA-DELTA-0053, amended by omega#161. Seal here, before any window
    // opens, so the editor chrome is never drawn — not even for a frame. The
    // seal used to wait for `OMEGA-DELTA-0040`'s centre-pane identity
    // onboarding to be answered inside `initialize_panels`; omega#164 deleted
    // that page and provisions the Nostr identity silently in the background,
    // so nothing renders in the centre before the thread and sealing at
    // startup is safe.
    omega_zero_base::seal();

    // OMEGA-DELTA-0116. A path argument names the project, never a mode. The
    // surface draws no buffer, so the argument is resolved to the directory
    // the thread can see rather than left as a file to open in a pane that is
    // not there.
    if !args.primary_interface {
        resolve_zero_base_project_arguments(&mut args);
    }

    // OMEGA-DELTA-0093. Read beside zero base, and for the same reason: both
    // are command-line facts that a surface deep in startup has to consult
    // later, and neither is written anywhere.
    read_omega_send_from_command_line(&args);

    // Set custom data directory.
    if let Some(dir) = &args.user_data_dir {
        paths::set_custom_data_dir(dir);
    }

    #[cfg(target_os = "windows")]
    match util::get_zed_cli_path() {
        Ok(path) => askpass::set_askpass_program(path),
        Err(err) => {
            eprintln!("Error: {}", err);
            if std::option_env!("ZED_BUNDLE").is_some() {
                process::exit(1);
            }
        }
    }

    let file_errors = init_paths();
    if !file_errors.is_empty() {
        files_not_created_on_launch(file_errors);
        return;
    }

    zlog::init();

    let stdout_is_tty = stdout_is_a_pty();
    let result = zlog::init_output_file(paths::log_file(), Some(paths::old_log_file()));
    if let Err(err) = result {
        eprintln!("Could not open log file: {}... Defaulting to stdout", err);
        zlog::init_output_stdout();
    } else if stdout_is_tty {
        zlog::init_output_stdout();
    }
    ztracing::init();

    let version = option_env!("ZED_BUILD_ID");
    let app_commit_sha =
        option_env!("ZED_COMMIT_SHA").map(|commit_sha| AppCommitSha::new(commit_sha.to_string()));
    let app_version = AppVersion::load(env!("CARGO_PKG_VERSION"), version, app_commit_sha.clone());

    if args.system_specs {
        let system_specs = system_specs::SystemSpecs::new_stateless(
            app_version,
            app_commit_sha,
            *release_channel::RELEASE_CHANNEL,
            client::telemetry::os_name(),
            client::telemetry::os_version(),
        );
        println!("Omega System Specs (from CLI):\n{}", system_specs);
        return;
    }

    rayon::ThreadPoolBuilder::new()
        .num_threads(std::thread::available_parallelism().map_or(1, |n| n.get().div_ceil(2)))
        .stack_size(10 * 1024 * 1024)
        .thread_name(|ix| format!("RayonWorker{}", ix))
        .build_global()
        .unwrap();

    log::info!(
        "========== starting omega version {}, sha {} ==========",
        app_version,
        app_commit_sha
            .as_ref()
            .map(|sha| sha.short())
            .as_deref()
            .unwrap_or("unknown"),
    );

    #[cfg(windows)]
    check_for_conpty_dll();

    let app = build_application().with_assets(Assets);

    let app_db = db::AppDatabase::new();
    let system_id = app.background_executor().spawn(system_id());
    let installation_id = app
        .background_executor()
        .spawn(installation_id(KeyValueStore::from_app_db(&app_db)));
    let session_id = Uuid::new_v4().to_string();
    let session = app.background_executor().spawn(Session::new(
        session_id.clone(),
        KeyValueStore::from_app_db(&app_db),
    ));
    let background_executor = app.background_executor();

    let (open_listener, mut open_rx) = OpenListener::new();

    let failed_single_instance_check = if *zed_env_vars::ZED_STATELESS
        || *release_channel::RELEASE_CHANNEL == ReleaseChannel::Dev
    {
        false
    } else {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            crate::zed::listen_for_cli_connections(open_listener.clone()).is_err()
        }

        #[cfg(target_os = "windows")]
        {
            !crate::zed::windows_only_instance::handle_single_instance(open_listener.clone(), &args)
        }

        #[cfg(target_os = "macos")]
        {
            use zed::mac_only_instance::*;
            ensure_only_instance() != IsOnlyInstance::Yes
        }
    };
    if failed_single_instance_check {
        println!("Omega is already running");
        return;
    }

    let should_install_crash_handler =
        client::telemetry::should_install_crash_handler(*release_channel::RELEASE_CHANNEL);

    let crash_handler = if should_install_crash_handler {
        Some(
            app.background_executor().spawn(crashes::init(
                InitCrashHandler {
                    session_id,
                    // strip the build and channel information from the version string, we send them separately
                    zed_version: semver::Version::new(
                        app_version.major,
                        app_version.minor,
                        app_version.patch,
                    )
                    .to_string(),
                    binary: "zed".to_string(),
                    release_channel: release_channel::RELEASE_CHANNEL_NAME.clone(),
                    commit_sha: app_commit_sha
                        .as_ref()
                        .map(|sha| sha.full())
                        .unwrap_or_else(|| "no sha".to_owned()),
                },
                {
                    let background_executor1 = app.background_executor();
                    move |task| {
                        background_executor1.spawn(task).detach();
                    }
                },
                |pid| paths::temp_dir().join(format!("zed-crash-handler-{pid}")),
                move |duration| background_executor.timer(duration),
            )),
        )
    } else {
        crashes::force_backtrace();
        None
    };

    let git_hosting_provider_registry = Arc::new(GitHostingProviderRegistry::new());
    let git_binary_path =
        if cfg!(target_os = "macos") && option_env!("ZED_BUNDLE").as_deref() == Some("true") {
            // omega#170. The Omega bundle does not package an auxiliary git,
            // so this lookup fails on every launch of the shipped app and the
            // PATH fallback below is the ordinary, fully supported path. That
            // is a quiet fact, not an ERROR: it used to be the first red line
            // in every clean-profile log a tester attached.
            app.path_for_auxiliary_executable("git").ok()
        } else {
            None
        };
    match &git_binary_path {
        Some(git_binary_path) => log::info!("Using git binary path: {:?}", git_binary_path),
        None => log::info!("No bundled git; using git from PATH"),
    }

    let fs = Arc::new(RealFs::new(git_binary_path, app.background_executor()));
    let (user_keymap_file_rx, user_keymap_watcher) = watch_config_file(
        &app.background_executor(),
        fs.clone(),
        paths::keymap_file().clone(),
    );

    let (shell_env_loaded_tx, shell_env_loaded_rx) = oneshot::channel();
    if !stdout_is_a_pty() {
        app.background_executor()
            .spawn(async {
                #[cfg(unix)]
                util::load_login_shell_environment().await.log_err();
                shell_env_loaded_tx.send(()).ok();
            })
            .detach();
    } else {
        drop(shell_env_loaded_tx)
    }

    app.on_open_urls({
        let open_listener = open_listener.clone();
        move |urls| {
            open_listener.open(RawOpenRequest {
                urls,
                diff_paths: Vec::new(),
                ..Default::default()
            })
        }
    });
    app.on_reopen(move |cx| {
        if let Some(app_state) = AppState::try_global(cx) {
            cx.spawn({
                async move |cx| {
                    if let Err(e) = restore_or_create_workspace(app_state, cx).await {
                        fail_to_open_window_async(e, cx)
                    }
                }
            })
            .detach();
        }
    });

    app.run(move |cx| {
        cx.set_global(app_db);
        let db_trusted_paths = match workspace::WorkspaceDb::global(cx).fetch_trusted_worktrees() {
            Ok(trusted_paths) => trusted_paths,
            Err(e) => {
                log::error!("Failed to do initial trusted worktrees fetch: {e:#}");
                HashMap::default()
            }
        };
        trusted_worktrees::init(db_trusted_paths, cx);
        menu::init();
        omega_actions::init();

        release_channel::init(app_version, cx);
        gpui_tokio::init(cx);
        if let Some(app_commit_sha) = app_commit_sha {
            AppCommitSha::set_global(app_commit_sha, cx);
        }
        if let Some(build_number) = option_env!("OMEGA_BUILD_NUMBER")
            .and_then(|build_number| build_number.parse::<u32>().ok())
        {
            release_channel::AppBuildNumber::set_global(
                release_channel::AppBuildNumber::new(build_number),
                cx,
            );
        }
        settings::init(cx);
        zlog_settings::init(cx);
        zed::watch_settings_files(fs.clone(), cx);
        handle_keymap_file_changes(user_keymap_file_rx, user_keymap_watcher, cx);

        let user_agent = format!(
            "Omega/{} ({}; {})",
            AppVersion::global(cx),
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        let proxy_url = ProxySettings::get_global(cx).proxy_url();
        let http = {
            let _guard = Tokio::handle(cx).enter();

            ReqwestClient::proxy_and_user_agent(proxy_url, &user_agent)
                .expect("could not start HTTP client")
        };
        cx.set_http_client(Arc::new(http));

        <dyn Fs>::set_global(fs.clone(), cx);

        GitHostingProviderRegistry::set_global(git_hosting_provider_registry, cx);
        git_hosting_providers::init(cx);

        OpenListener::set_global(cx, open_listener.clone());

        extension::init(cx);
        let extension_host_proxy = ExtensionHostProxy::global(cx);

        let client = Client::production(cx);
        cx.set_http_client(client.http_client());
        let mut languages = LanguageRegistry::new(cx.background_executor().clone());
        languages.set_language_server_download_dir(paths::languages_dir().clone());
        let languages = Arc::new(languages);
        let (mut tx, rx) = watch::channel(None);
        cx.observe_global::<SettingsStore>(move |cx| {
            let settings = &ProjectSettings::get_global(cx).node;
            let options = NodeBinaryOptions {
                allow_path_lookup: !settings.ignore_system_version,
                // TODO: Expose this setting
                allow_binary_download: true,
                use_paths: settings.path.as_ref().map(|node_path| {
                    let node_path = PathBuf::from(shellexpand::tilde(node_path).as_ref());
                    let npm_path = settings
                        .npm_path
                        .as_ref()
                        .map(|path| PathBuf::from(shellexpand::tilde(&path).as_ref()));
                    (
                        node_path.clone(),
                        npm_path.unwrap_or_else(|| {
                            let base_path = PathBuf::new();
                            node_path.parent().unwrap_or(&base_path).join("npm")
                        }),
                    )
                }),
            };
            tx.send(Some(options)).log_err();
        })
        .detach();
        ui::on_new_scrollbars::<SettingsStore>(cx);

        let node_runtime = NodeRuntime::new(client.http_client(), Some(shell_env_loaded_rx), rx);

        debug_adapter_extension::init(extension_host_proxy.clone(), cx);
        languages::init(languages.clone(), fs.clone(), node_runtime.clone(), cx);
        let user_store = cx.new(|cx| UserStore::new(client.clone(), cx));
        let workspace_store = cx.new(|cx| WorkspaceStore::new(client.clone(), cx));

        language_extension::init(
            language_extension::LspAccess::ViaWorkspaces({
                let workspace_store = workspace_store.clone();
                Arc::new(move |cx: &mut App| {
                    workspace_store.update(cx, |workspace_store, cx| {
                        Ok(workspace_store
                            .workspaces()
                            .filter_map(|weak| weak.upgrade())
                            .map(|workspace: gpui::Entity<workspace::Workspace>| {
                                workspace.read(cx).project().read(cx).lsp_store()
                            })
                            .collect())
                    })
                })
            }),
            extension_host_proxy.clone(),
            languages.clone(),
        );

        Client::set_global(client.clone(), cx);

        zed::init(cx);
        #[cfg(target_os = "macos")]
        project::Project::init(&client, cx);
        omega_effectd::init_openagents_session(cx);
        omega_convex::init(cx);
        omega_effectd::init_openagents_binding(cx);
        omega_effectd::init_with_host_handler(Some(agent_ui::omega_effectd_host_handler(cx)), cx);
        agent_computer_ui::init(cx);
        workroom_ui::init(cx);
        client::init(&client, cx);
        feature_flags::FeatureFlagStore::init(cx);

        let system_id = cx.foreground_executor().block_on(system_id).ok();
        let installation_id = cx.foreground_executor().block_on(installation_id).ok();
        let session = cx.foreground_executor().block_on(session);

        let telemetry = client.telemetry();
        telemetry.start(
            system_id.as_ref().map(|id| id.to_string()),
            installation_id.as_ref().map(|id| id.to_string()),
            session.id().to_owned(),
            cx,
        );
        cx.subscribe(&user_store, {
            let telemetry = telemetry.clone();
            move |_, evt: &client::user::Event, cx| match evt {
                client::user::Event::PrivateUserInfoUpdated => {
                    if let Some(crash_client) = cx.try_global::<CrashHandler>() {
                        crashes::set_user_info(
                            &crash_client.0,
                            crashes::UserInfo {
                                metrics_id: telemetry.metrics_id().map(|s| s.to_string()),
                                is_staff: telemetry.is_staff(),
                            },
                        );
                    }
                }
                _ => {}
            }
        })
        .detach();

        let is_new_install = matches!(&installation_id, Some(IdType::New(_)));

        // We should rename these in the future to `first app open`, `first app open for release channel`, and `app open`
        if let (Some(system_id), Some(installation_id)) = (&system_id, &installation_id) {
            match (&system_id, &installation_id) {
                (IdType::New(_), IdType::New(_)) => {
                    telemetry::event!("App First Opened");
                    telemetry::event!("App First Opened For Release Channel");
                }
                (IdType::Existing(_), IdType::New(_)) => {
                    telemetry::event!("App First Opened For Release Channel");
                }
                (_, IdType::Existing(_)) => {
                    telemetry::event!("App Opened");
                }
            }
        }
        let app_session = cx.new(|cx| AppSession::new(session, cx));

        let app_state = Arc::new(AppState {
            languages,
            client: client.clone(),
            user_store,
            fs: fs.clone(),
            build_window_options,
            workspace_store,
            node_runtime,
            session: app_session,
        });
        AppState::set_global(app_state.clone(), cx);

        auto_update::init(client.clone(), cx);
        reliability::init(client.clone(), app_state.workspace_store.clone(), cx);
        extension_host::init(
            extension_host_proxy.clone(),
            app_state.fs.clone(),
            app_state.client.clone(),
            app_state.node_runtime.clone(),
            cx,
        );

        theme_settings::init(theme::LoadThemes::All(Box::new(Assets)), cx);
        eager_load_active_theme_and_icon_theme(fs.clone(), cx);
        theme_extension::init(
            extension_host_proxy,
            ThemeRegistry::global(cx),
            cx.background_executor().clone(),
        );
        command_palette::init(cx);
        language_model::init(cx);
        RefreshLlmTokenListener::register(
            app_state.client.clone(),
            app_state.user_store.clone(),
            cx,
        );
        language_models::init(app_state.user_store.clone(), app_state.client.clone(), cx);
        acp_tools::init(cx);
        component_library::init(cx);
        // omega#244: registers the Markets panel's palette toggle; inert
        // unless OMEGA_MARKET_PANEL=1 (the panel itself loads in zed.rs).
        market_ui::init(cx);
        // OMEGA-DELTA-0268: the optional testnet-only execution engine is app-owned.
        nautilus_sidecar::init(cx);
        zed::telemetry_log::init(cx);
        zed::remote_debug::init(cx);
        web_search::init(cx);
        web_search_providers::init(app_state.client.clone(), app_state.user_store.clone(), cx);
        snippet_provider::init(cx);
        let prompt_builder = PromptBuilder::load(app_state.fs.clone(), stdout_is_a_pty(), cx);
        project::AgentRegistryStore::init_global(
            cx,
            app_state.fs.clone(),
            app_state.client.http_client(),
        );
        agent_ui::init(
            app_state.fs.clone(),
            prompt_builder,
            app_state.languages.clone(),
            is_new_install,
            false,
            cx,
        );
        zed::watch_user_agents_md(app_state.fs.clone(), cx);

        cx.observe_new(zed::disconnected_overlay::DisconnectedOverlay::register)
            .detach();

        load_embedded_fonts(cx);

        editor::init(cx);

        audio::init(cx);
        workspace::init(app_state.clone(), cx);
        // `OMEGA-DELTA-0157`. The titlebar view installs itself here, and for
        // two days it installed nowhere. `title_bar::init` used to be called
        // from `collab_ui::init`, and "Retire Zed collab" (990e4c6cc6) deleted
        // that crate with the call inside it. Nothing else calls it, so
        // `Workspace::titlebar_item` stayed `None` in every window of every
        // mode: no `PlatformTitleBar`, no `WindowControlArea::Drag`, no
        // `start_window_move` listener, and a window a person could not move.
        // `OMEGA-DELTA-0147` then wrote the sealed-zero-base drag strip against
        // an item that was never set, so its checks passed over a dead branch.
        title_bar::init(cx);
        ui_prompt::init(cx);

        go_to_line::init(cx);
        file_finder::init(cx);
        project_panel::init(cx);
        channel::init(&app_state.client.clone(), app_state.user_store.clone(), cx);
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
        vim::init(cx);
        terminal_view::init(cx);
        language_tools::init(cx);
        notifications::init(app_state.client.clone(), app_state.user_store.clone(), cx);
        git_ui::init(cx);
        account_ui::init(cx);
        onboarding::init(cx);
        settings_ui::init(cx);
        plugins::init(cx);
        json_schema_store::init(cx);
        #[cfg(target_os = "windows")]
        etw_tracing::init(cx);

        cx.observe_global::<SettingsStore>({
            let http = app_state.client.http_client();
            let client = app_state.client.clone();
            move |cx| {
                for &mut window in cx.windows().iter_mut() {
                    let background_appearance =
                        if omega_zero_base::is_primary_interface() && cfg!(target_os = "macos") {
                            gpui::WindowBackgroundAppearance::Blurred
                        } else {
                            cx.theme().window_background_appearance()
                        };
                    window
                        .update(cx, |_, window, _| {
                            window.set_background_appearance(background_appearance)
                        })
                        .ok();
                }

                cx.set_text_rendering_mode(
                    match WorkspaceSettings::get_global(cx).text_rendering_mode {
                        settings::TextRenderingMode::PlatformDefault => {
                            gpui::TextRenderingMode::PlatformDefault
                        }
                        settings::TextRenderingMode::Subpixel => gpui::TextRenderingMode::Subpixel,
                        settings::TextRenderingMode::Grayscale => {
                            gpui::TextRenderingMode::Grayscale
                        }
                    },
                );

                let new_host = &client::ClientSettings::get_global(cx).server_url;
                if &http.base_url() != new_host {
                    http.set_base_url(new_host);
                    if client.status().borrow().is_connected() {
                        client.reconnect(&cx.to_async());
                    }
                }
            }
        })
        .detach();
        app_state.languages.set_theme(cx.theme().clone());
        cx.observe_global::<GlobalTheme>({
            let languages = app_state.languages.clone();
            move |cx| {
                languages.set_theme(cx.theme().clone());
            }
        })
        .detach();
        telemetry::event!(
            "Settings Changed",
            setting = "theme",
            value = cx.theme().name.to_string()
        );
        telemetry::event!(
            "Settings Changed",
            setting = "keymap",
            value = BaseKeymap::get_global(cx).to_string()
        );
        telemetry.flush_events().detach();

        let fs = app_state.fs.clone();
        load_user_themes_in_background(fs.clone(), cx);
        watch_themes(fs.clone(), cx);
        #[cfg(debug_assertions)]
        watch_languages(fs.clone(), app_state.languages.clone(), cx);

        let menus = app_menus(cx);
        cx.set_menus(menus);

        if let Some(mut crash_handler) = crash_handler {
            let crash_handler2 = block_on(poll_once(&mut crash_handler));
            match crash_handler2 {
                Some(crash_handler) => {
                    cx.set_global(CrashHandler(crash_handler));
                }
                None => {
                    cx.spawn(async move |cx| {
                        let client1 = crash_handler.await;
                        cx.update(|cx| {
                            cx.set_global(CrashHandler(client1));
                        });
                    })
                    .detach();
                }
            }
        }

        initialize_workspace(app_state.clone(), cx);

        // omega#99, amended by omega#161. After every other `init` has
        // registered its actions and filled the palette, so the restriction
        // and the gate see the whole registry. Unconditional now: there is
        // one surface, and the gate is part of it.
        omega_zero_base_ui::init(cx);

        cx.activate(true);

        cx.spawn({
            let client = app_state.client.clone();
            async move |cx| authenticate(client, cx).await
        })
        .detach_and_log_err(cx);

        let urls: Vec<_> = args
            .paths_or_urls
            .iter()
            .map(|arg| parse_url_arg(arg, cx))
            .collect();

        #[cfg(target_os = "windows")]
        let wsl = args.wsl;
        #[cfg(not(target_os = "windows"))]
        let wsl = None;

        if !urls.is_empty() {
            open_listener.open(RawOpenRequest {
                urls,
                wsl,
                ..Default::default()
            })
        }

        let (current_session_id, last_session_id) = {
            let session = app_state.session.read(cx);
            (
                session.id().to_owned(),
                session.last_session_id().map(|id| id.to_owned()),
            )
        };

        let restore_task = match open_rx
            .try_recv()
            .ok()
            .and_then(|request| OpenRequest::parse(request, cx).log_err())
        {
            Some(request) if request.is_focus_app_only() => cx.spawn({
                let app_state = app_state.clone();
                async move |cx| {
                    if let Err(e) = restore_or_create_workspace(app_state, cx).await {
                        fail_to_open_window_async(e, cx)
                    }
                }
            }),
            Some(request) => {
                handle_open_request(request, app_state.clone(), cx);
                Task::ready(())
            }
            None => cx.spawn({
                let app_state = app_state.clone();
                async move |cx| {
                    if let Err(e) = restore_or_create_workspace(app_state, cx).await {
                        fail_to_open_window_async(e, cx)
                    }
                }
            }),
        };

        let (first_window_tx, first_window_rx) = oneshot::channel::<()>();
        let first_window_tx = Rc::new(RefCell::new(Some(first_window_tx)));
        let first_window_subscription = cx.observe_new::<MultiWorkspace>(move |_, _, _| {
            if let Some(tx) = first_window_tx.borrow_mut().take() {
                tx.send(()).ok();
            }
        });

        let restore_finished = cx.background_spawn(restore_task).shared();

        cx.spawn({
            let db = workspace::WorkspaceDb::global(cx);
            let fs = app_state.fs.clone();
            let restore_finished = restore_finished.clone();
            async move |_cx| {
                restore_finished.await;
                db.garbage_collect_workspaces(
                    fs.as_ref(),
                    &current_session_id,
                    last_session_id.as_deref(),
                )
                .await
            }
        })
        .detach_and_log_err(cx);

        let app_state = app_state.clone();

        cx.spawn(async move |cx| {
            let _first_window_subscription = first_window_subscription;
            let first_window_placed = first_window_rx.shared();
            while let Some(urls) = open_rx.next().await {
                // On a macOS cold launch, `omega <path>` arrives here after startup
                // already began restoring the session. Wait for a restored window
                // before matching so this request does not spawn a redundant window.
                futures::select_biased! {
                    _ = restore_finished.clone() => {}
                    _ = first_window_placed.clone() => {}
                }
                cx.update(|cx| {
                    if let Some(request) = OpenRequest::parse(urls, cx).log_err() {
                        handle_open_request(request, app_state.clone(), cx);
                    }
                });
            }
        })
        .detach();
    });
}

fn handle_open_request(request: OpenRequest, app_state: Arc<AppState>, cx: &mut App) {
    cx.spawn(async move |cx| {
        if let Err(error) = await_identity_ready(cx).await {
            fail_to_open_window_async(error, cx);
            return;
        }
        cx.update(|cx| dispatch_open_request(request, app_state, cx));
    })
    .detach();
}

fn dispatch_open_request(request: OpenRequest, app_state: Arc<AppState>, cx: &mut App) {
    if let Some(kind) = request.kind {
        match kind {
            OpenRequestKind::CliConnection(connection) => {
                cx.spawn(async move |cx| handle_cli_connection(connection, app_state, cx).await)
                    .detach();
            }
            OpenRequestKind::FocusApp => {
                cx.spawn(async move |cx| {
                    if workspace::activate_any_workspace_window(cx).is_some() {
                        return anyhow::Ok(());
                    }
                    restore_or_create_workspace(app_state, cx).await
                })
                .detach_and_log_err(cx);
            }
            OpenRequestKind::Extension { extension_id } => {
                cx.spawn(async move |cx| {
                    let workspace =
                        workspace::get_any_active_multi_workspace(app_state, cx.clone()).await?;
                    workspace.update(cx, |_, window, cx| {
                        window.dispatch_action(
                            Box::new(omega_actions::Extensions {
                                category_filter: None,
                                id: Some(extension_id),
                            }),
                            cx,
                        );
                    })
                })
                .detach_and_log_err(cx);
            }
            OpenRequestKind::AgentPanel {
                external_source_prompt,
            } => {
                cx.spawn(async move |cx| {
                    let multi_workspace =
                        workspace::get_any_active_multi_workspace(app_state, cx.clone()).await?;

                    let panels_task = multi_workspace.update(cx, |multi_workspace, _, cx| {
                        multi_workspace
                            .workspace()
                            .update(cx, |workspace, _| workspace.take_panels_task())
                    })?;
                    if let Some(task) = panels_task {
                        task.await.log_err();
                    }

                    multi_workspace.update(cx, |multi_workspace, window, cx| {
                        multi_workspace.workspace().update(cx, |workspace, cx| {
                            if let Some(panel) = workspace.focus_panel::<AgentPanel>(window, cx) {
                                panel.update(cx, |panel, cx| {
                                    panel.new_agent_thread_with_external_source_prompt(
                                        external_source_prompt,
                                        window,
                                        cx,
                                    );
                                });
                            } else {
                                log::warn!(
                                    "zed://agent received but the AgentPanel is not registered \
                                     (is `disable_ai` enabled?)"
                                );
                            }
                        });
                    })
                })
                .detach_and_log_err(cx);
            }
            OpenRequestKind::InstallSkill { content } => {
                cx.spawn(async move |cx| {
                    let multi_workspace =
                        workspace::get_any_active_multi_workspace(app_state, cx.clone()).await?;

                    multi_workspace.update(cx, |_multi_workspace, _window, cx| {
                        settings_ui::open_skill_creator(
                            settings_ui::pages::SkillCreatorOpenMode::Install { content },
                            Some(multi_workspace),
                            cx,
                        );
                    })
                })
                .detach_and_log_err(cx);
            }
            OpenRequestKind::DockMenuAction { index } => {
                cx.perform_dock_menu_action(index);
            }
            OpenRequestKind::BuiltinJsonSchema { schema_path } => {
                workspace::with_active_or_new_workspace(cx, |_workspace, window, cx| {
                    cx.spawn_in(window, async move |workspace, cx| {
                        let res = async move {
                            let json = app_state.languages.language_for_name("JSONC").await.ok();
                            let lsp_store = workspace.update(cx, |workspace, cx| {
                                workspace
                                    .project()
                                    .update(cx, |project, _| project.lsp_store())
                            })?;
                            let uri = format!("zed://schemas/{}", schema_path);
                            let json_schema_content =
                                json_schema_store::handle_schema_request(lsp_store, uri, cx)
                                    .await?;
                            let json_schema_value: serde_json::Value =
                                serde_json::from_str(&json_schema_content)
                                    .context("Failed to parse JSON Schema")?;
                            let json_schema_content =
                                serde_json::to_string_pretty(&json_schema_value)
                                    .context("Failed to serialize JSON Schema as JSON")?;
                            let buffer_task = workspace.update(cx, |workspace, cx| {
                                workspace.project().update(cx, |project, cx| {
                                    project.create_buffer(json, false, cx)
                                })
                            })?;

                            let buffer = buffer_task.await?;

                            workspace.update_in(cx, |workspace, window, cx| {
                                buffer.update(cx, |buffer, cx| {
                                    buffer.edit([(0..0, json_schema_content)], None, cx);
                                    buffer.edit(
                                        [(0..0, format!("// {} JSON Schema\n", schema_path))],
                                        None,
                                        cx,
                                    );
                                });

                                workspace.add_item_to_active_pane(
                                    Box::new(cx.new(|cx| {
                                        let mut editor =
                                            editor::Editor::for_buffer(buffer, None, window, cx);
                                        editor.set_read_only(true);
                                        editor
                                    })),
                                    None,
                                    true,
                                    window,
                                    cx,
                                );
                            })
                        }
                        .await;
                        res.context("Failed to open builtin JSON Schema").log_err();
                    })
                    .detach();
                });
            }
            OpenRequestKind::Setting { setting_path } => {
                // zed://settings/languages/$(language)/tab_size  - DONT SUPPORT
                // zed://settings/languages/Rust/tab_size  - SUPPORT
                // languages.$(language).tab_size
                // [ languages $(language) tab_size]
                cx.spawn(async move |cx| {
                    let workspace =
                        workspace::get_any_active_multi_workspace(app_state, cx.clone()).await?;

                    workspace.update(cx, |_, window, cx| match setting_path {
                        None => window.dispatch_action(Box::new(omega_actions::OpenSettings), cx),
                        Some(setting_path) => window.dispatch_action(
                            Box::new(omega_actions::OpenSettingsAt {
                                path: setting_path,
                                target: None,
                            }),
                            cx,
                        ),
                    })
                })
                .detach_and_log_err(cx);
            }
            OpenRequestKind::GitClone { repo_url } => {
                workspace::with_active_or_new_workspace(cx, |_workspace, window, cx| {
                    if window.is_window_active() {
                        clone_and_open(
                            repo_url,
                            cx.weak_entity(),
                            window,
                            cx,
                            Arc::new(|workspace: &mut workspace::Workspace, window, cx| {
                                workspace.focus_panel::<ProjectPanel>(window, cx);
                            }),
                        );
                        return;
                    }

                    let subscription = Rc::new(RefCell::new(None));
                    subscription.replace(Some(cx.observe_in(&cx.entity(), window, {
                        let subscription = subscription.clone();
                        let repo_url = repo_url;
                        move |_, workspace_entity, window, cx| {
                            if window.is_window_active() && subscription.take().is_some() {
                                clone_and_open(
                                    repo_url.clone(),
                                    workspace_entity.downgrade(),
                                    window,
                                    cx,
                                    Arc::new(|workspace: &mut workspace::Workspace, window, cx| {
                                        workspace.focus_panel::<ProjectPanel>(window, cx);
                                    }),
                                );
                            }
                        }
                    })));
                });
            }
            OpenRequestKind::GitCommit { sha } => {
                let base_open_options = zed::open_options_for_request(
                    request.open_behavior,
                    &workspace::SerializedWorkspaceLocation::Local,
                    cx,
                );
                cx.spawn(async move |cx| {
                    let paths_with_position =
                        derive_paths_with_position(app_state.fs.as_ref(), request.open_paths).await;
                    let (workspace, _results) = open_paths_with_positions(
                        &paths_with_position,
                        &[],
                        false,
                        app_state,
                        base_open_options,
                        cx,
                    )
                    .await?;

                    workspace
                        .update(cx, |multi_workspace, window, cx| {
                            multi_workspace
                                .workspace()
                                .clone()
                                .update(cx, |workspace, cx| {
                                    let Some(repo) =
                                        workspace.project().read(cx).active_repository(cx)
                                    else {
                                        log::error!("no active repository found for commit view");
                                        return Err(anyhow::anyhow!("no active repository found"));
                                    };

                                    git_ui::commit_view::CommitView::open(
                                        sha,
                                        repo.downgrade(),
                                        workspace.weak_handle(),
                                        None,
                                        None,
                                        window,
                                        cx,
                                    );
                                    Ok(())
                                })
                        })
                        .log_err();

                    anyhow::Ok(())
                })
                .detach_and_log_err(cx);
            }
        }

        return;
    }

    if let Some(connection_options) = request.remote_connection {
        let open_behavior = request.open_behavior;
        let location = workspace::SerializedWorkspaceLocation::Remote(connection_options.clone());
        let base_open_options = zed::open_options_for_request(open_behavior, &location, cx);
        cx.spawn(async move |cx| {
            let paths: Vec<PathBuf> = request.open_paths.into_iter().map(PathBuf::from).collect();
            open_remote_project(connection_options, paths, app_state, base_open_options, cx).await
        })
        .detach_and_log_err(cx);
        return;
    }

    let mut task = None;
    if !request.open_paths.is_empty() || !request.diff_paths.is_empty() {
        let app_state = app_state.clone();
        let base_open_options = zed::open_options_for_request(
            request.open_behavior,
            &workspace::SerializedWorkspaceLocation::Local,
            cx,
        );
        task = Some(cx.spawn(async move |cx| {
            let paths_with_position =
                derive_paths_with_position(app_state.fs.as_ref(), request.open_paths).await;
            let (_window, results) = open_paths_with_positions(
                &paths_with_position,
                &request.diff_paths,
                request.diff_all,
                app_state,
                workspace::OpenOptions {
                    ..base_open_options
                },
                cx,
            )
            .await?;
            for result in results.into_iter().flatten() {
                if let Err(err) = result {
                    log::error!("Error opening path: {err:#}");
                }
            }
            anyhow::Ok(())
        }));
    }

    if !request.open_channel_notes.is_empty() || request.join_channel.is_some() {
        cx.spawn(async move |cx| {
            let result = maybe!(async {
                if let Some(task) = task {
                    task.await?;
                }
                let client = app_state.client.clone();
                // we continue even if connection fails as join_channel/ open channel notes will
                // show a visible error message.
                client.connect(true, cx).await.into_response().log_err();

                if let Some(channel_id) = request.join_channel {
                    cx.update(|cx| {
                        workspace::join_channel(
                            client::ChannelId(channel_id),
                            app_state.clone(),
                            None,
                            None,
                            cx,
                        )
                    })
                    .await?;
                }

                // Zed channel notes are retired in Omega. See omega#59.
                anyhow::Ok(())
            })
            .await;
            if let Err(err) = result {
                fail_to_open_window_async(err, cx);
            }
        })
        .detach()
    } else if let Some(task) = task {
        cx.spawn(async move |cx| {
            if let Err(err) = task.await {
                fail_to_open_window_async(err, cx);
            }
        })
        .detach();
    }
}

async fn authenticate(client: Arc<Client>, cx: &AsyncApp) -> Result<()> {
    if !app_identity::zed_production_services_enabled() {
        log::info!(
            "Skipping Zed account authentication; Omega production services isolation is enabled"
        );
        return Ok(());
    }

    if stdout_is_a_pty() {
        if client::IMPERSONATE_LOGIN.is_some() {
            client.sign_in_with_optional_connect(false, cx).await?;
        } else if client.has_credentials(cx).await {
            client.sign_in_with_optional_connect(true, cx).await?;
        }
    } else if client.has_credentials(cx).await {
        client.sign_in_with_optional_connect(true, cx).await?;
    }

    Ok(())
}

async fn system_id() -> Result<IdType> {
    let key_name = "system_id".to_string();
    let db = GlobalKeyValueStore::global();

    if let Ok(Some(system_id)) = db.read_kvp(&key_name) {
        return Ok(IdType::Existing(system_id));
    }

    let system_id = Uuid::new_v4().to_string();

    db.write_kvp(key_name, system_id.clone()).await?;

    Ok(IdType::New(system_id))
}

async fn installation_id(db: KeyValueStore) -> Result<IdType> {
    let legacy_key_name = "device_id".to_string();
    let key_name = "installation_id".to_string();

    // Migrate legacy key to new key
    if let Ok(Some(installation_id)) = db.read_kvp(&legacy_key_name) {
        db.write_kvp(key_name, installation_id.clone()).await?;
        db.delete_kvp(legacy_key_name).await?;
        return Ok(IdType::Existing(installation_id));
    }

    if let Ok(Some(installation_id)) = db.read_kvp(&key_name) {
        return Ok(IdType::Existing(installation_id));
    }

    let installation_id = Uuid::new_v4().to_string();

    db.write_kvp(key_name, installation_id.clone()).await?;

    Ok(IdType::New(installation_id))
}

pub(crate) async fn restore_or_create_workspace(
    app_state: Arc<AppState>,
    cx: &mut AsyncApp,
) -> Result<()> {
    await_identity_ready(cx).await?;
    if omega_zero_base::is_primary_interface() {
        if !open_zero_base_project(&app_state, cx).await {
            cx.update(|cx| {
                workspace::open_new(
                    Default::default(),
                    app_state,
                    cx,
                    |_workspace, window, cx| {
                        agent_ui::AgentPanel::open_front_door(window, cx);
                    },
                )
            })
            .await?;
        }
        drive_omega_send(cx).await;
        return Ok(());
    }

    if let Some(multi_workspaces) = restorable_workspaces(cx, &app_state).await {
        let mut error_count = 0;
        for multi_workspace in multi_workspaces {
            let result = match &multi_workspace.active_workspace.location {
                SerializedWorkspaceLocation::Local => {
                    restore_multiworkspace(multi_workspace, app_state.clone(), cx)
                        .await
                        .map(|_| ())
                }
                SerializedWorkspaceLocation::Remote(connection_options) => {
                    let mut connection_options = connection_options.clone();
                    if let RemoteConnectionOptions::Ssh(options) = &mut connection_options {
                        cx.update(|cx| {
                            RemoteSettings::get_global(cx)
                                .fill_connection_options_from_settings(options)
                        });
                    }

                    let paths = multi_workspace
                        .active_workspace
                        .paths
                        .paths()
                        .iter()
                        .map(PathBuf::from)
                        .collect::<Vec<_>>();
                    let state = multi_workspace.state.clone();
                    async {
                        let window = open_remote_project(
                            connection_options,
                            paths,
                            app_state.clone(),
                            workspace::OpenOptions::default(),
                            cx,
                        )
                        .await?;
                        workspace::apply_restored_multiworkspace_state(
                            window,
                            &state,
                            app_state.fs.clone(),
                            cx,
                        )
                        .await;
                        Ok::<(), anyhow::Error>(())
                    }
                    .await
                }
            };

            if let Err(error) = result {
                log::error!("Failed to restore workspace: {error:#}");
                error_count += 1;
            }
        }

        if error_count > 0 {
            let message = if error_count == 1 {
                "Failed to restore 1 workspace. Check logs for details.".to_string()
            } else {
                format!(
                    "Failed to restore {} workspaces. Check logs for details.",
                    error_count
                )
            };

            // Try to find an active workspace to show the toast
            let toast_shown = cx.update(|cx| {
                if let Some(window) = cx.active_window()
                    && let Some(multi_workspace) = window.downcast::<MultiWorkspace>()
                {
                    multi_workspace
                        .update(cx, |multi_workspace, _, cx| {
                            multi_workspace.workspace().update(cx, |workspace, cx| {
                                workspace.show_toast(
                                    Toast::new(NotificationId::unique::<()>(), message.clone()),
                                    cx,
                                )
                            });
                        })
                        .ok();
                    return true;
                }
                false
            });

            // If we couldn't show a toast (no windows opened successfully),
            // open a fallback empty workspace and show the error there
            if !toast_shown {
                log::error!("All workspace restorations failed. Opening fallback empty workspace.");
                cx.update(|cx| {
                    workspace::open_new(
                        Default::default(),
                        app_state.clone(),
                        cx,
                        |workspace, _window, cx| {
                            workspace.show_toast(
                                Toast::new(NotificationId::unique::<()>(), message),
                                cx,
                            );
                        },
                    )
                })
                .await?;
            }
        }

        // If the user cancelled a failed remote connection at startup,
        // open_remote_project returns Ok but removes the window, so error_count
        // stays 0 and the toast fallback above does not trigger. Without this
        // check, Zed would exit silently.
        if cx.update(|cx| cx.windows().is_empty()) {
            cx.update(|cx| {
                workspace::open_new(
                    Default::default(),
                    app_state.clone(),
                    cx,
                    |_workspace, window, cx| {
                        let restore_on_startup =
                            WorkspaceSettings::get_global(cx).restore_on_startup;
                        match restore_on_startup {
                            // OMEGA-DELTA-0019. The launchpad behaviour opens
                            // no content, and overriding it would be Omega
                            // ignoring a setting the user set.
                            workspace::RestoreOnStartupBehavior::Launchpad => {}
                            // OMEGA-DELTA-0019. Upstream Zed opens an empty
                            // untitled buffer here. Omega opens its front
                            // door: the New Agent Thread surface, focused.
                            _ => {
                                agent_ui::AgentPanel::open_front_door(window, cx);
                            }
                        }
                    },
                )
            })
            .await?;
        }
    } else {
        // OMEGA-DELTA-0054, omega#100. Zero base opens the directory it was
        // started in, when that is a directory somebody chose.
        //
        // Without this the workspace has no worktrees, so every file tool the
        // thread holds — `grep`, `find_path`, `list_directory`, `read_file`,
        // `terminal` — has nothing to operate on. The owner ran several
        // searches, every one returned no matches, and the agent reported that
        // the workspace appeared to be empty. Routing the first message to a
        // coding agent is worth nothing if the agent opens on nothing.
        if !open_zero_base_project(&app_state, cx).await {
            cx.update(|cx| {
                workspace::open_new(
                    Default::default(),
                    app_state,
                    cx,
                    |_workspace, window, cx| {
                        let restore_on_startup =
                            WorkspaceSettings::get_global(cx).restore_on_startup;
                        match restore_on_startup {
                            // OMEGA-DELTA-0019. See above: the launchpad choice
                            // stands, and the empty buffer becomes the front door.
                            workspace::RestoreOnStartupBehavior::Launchpad => {}
                            _ => {
                                agent_ui::AgentPanel::open_front_door(window, cx);
                            }
                        }
                    },
                )
            })
            .await?;
        }
    }

    // OMEGA-DELTA-0093. After every branch above, so the driven send reaches
    // the window a person would have been looking at. The zero-base branch used
    // to return early here, which would have made `--omega-send` work on an
    // empty workspace and silently do nothing on the one case that matters —
    // and `OMEGA-DELTA-0040`'s identity wait stays where it is, at the top of
    // this same function, because the ordering it binds is not this delta's to
    // move.
    drive_omega_send(cx).await;

    Ok(())
}

/// Turn zero base's path arguments into the project they name.
///
/// `OMEGA-DELTA-0116`, omega#111. `omega <path>` no longer changes the mode, so
/// it has to mean something in the mode it stays in — and in zero base a path
/// can only mean one thing, because there is no pane to open a file into. Each
/// argument becomes the directory the thread's `grep`, `read_file`,
/// `list_directory` and `terminal` operate on, which is also the directory an
/// external agent is spawned in, which is the answer the owner did not get when
/// he asked one where it was.
///
/// Rewriting the parsed arguments, rather than deciding again further down, is
/// deliberate: everything after this point — the open listener, the workspace,
/// the worktree, the agent's `cwd` — already agrees on what a path argument
/// means, and adding a second path for zero base would be a second answer to a
/// question `OMEGA-DELTA-0054` already answers once.
///
/// # What it does not touch
///
/// Anything containing `://`. An argument carrying a URL scheme is a request
/// the open listener parses itself, and reinterpreting one here would be this
/// function guessing at a scheme it does not own. An argument that names no
/// directory is left exactly as typed and logged: refusing to open it is
/// `OMEGA-DELTA-0054`'s rule, and quietly dropping it would turn a typo into a
/// window that opens on something else without saying so.
fn resolve_zero_base_project_arguments(args: &mut Args) {
    if args.paths_or_urls.is_empty() {
        return;
    }
    let Ok(cwd) = std::env::current_dir() else {
        log::info!(
            "OMEGA-DELTA-0116: the working directory cannot be read, so a \
             relative path argument names nothing; arguments are left as typed"
        );
        return;
    };
    let home = std::env::var_os("HOME").map(PathBuf::from);

    let mut roots: Vec<String> = Vec::new();
    for argument in std::mem::take(&mut args.paths_or_urls) {
        if argument.contains("://") {
            roots.push(argument);
            continue;
        }
        match omega_workdir::project_root_named(Path::new(&argument), &cwd, home.as_deref()) {
            Ok(root) => {
                log::info!(
                    "OMEGA-DELTA-0116: `{argument}` names {} as zero base's project",
                    root.display()
                );
                let root = root.to_string_lossy().into_owned();
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
            Err(reason) => {
                log::info!(
                    "OMEGA-DELTA-0116: `{argument}` names no project ({reason:?}); \
                     it is left as typed"
                );
                roots.push(argument);
            }
        }
    }
    args.paths_or_urls = roots;
}

/// Open the working directory as zero base's project, if it is one.
///
/// `OMEGA-DELTA-0054`, omega#100. Returns whether a workspace was opened, so
/// the caller falls through to its ordinary empty workspace when the answer is
/// no. It never guesses: an implausible working directory opens no project and
/// the composer says so in one line, because putting an agent's file tools in a
/// directory nobody named is worse than having none.
///
/// `OMEGA-DELTA-0116`. This is the *bare* `omega` path, and only that. When a
/// path argument was given it has already become the workspace by the time this
/// runs, through the open listener, so the working directory is the fallback
/// for a launch that named nothing rather than a second opinion about one that
/// did.
async fn open_zero_base_project(app_state: &Arc<AppState>, cx: &mut AsyncApp) -> bool {
    let root = match omega_workdir::from_env() {
        Ok(root) => root,
        Err(reason) => {
            log::info!(
                "OMEGA-DELTA-0054: no project opened; the working directory is \
                 not one somebody chose ({reason:?})"
            );
            return false;
        }
    };
    log::info!(
        "OMEGA-DELTA-0054: opening {} as zero base's project",
        root.display()
    );
    let opening = cx.update(|cx| {
        workspace::open_paths(
            std::slice::from_ref(&root),
            app_state.clone(),
            workspace::OpenOptions::default(),
            cx,
        )
    });
    match opening.await {
        Ok(_) => true,
        Err(error) => {
            log::warn!(
                "OMEGA-DELTA-0054: {} could not be opened ({error:#}); zero base \
                 opens with no project",
                root.display()
            );
            false
        }
    }
}

/// What `--omega-send` asked for, read from this process's command line.
///
/// `OMEGA-DELTA-0093`, omega#100. A process global set once at startup, for the
/// reason `OMEGA-DELTA-0047` gives about zero base: `restore_or_create_workspace`
/// is four call sites deep and takes no `Args`, and threading the flag through
/// them would put a command-line concern into four signatures that have nothing
/// to do with it. Like zero base, it is read from the command line and from
/// nowhere else — no setting, no environment variable, nothing persisted — so
/// ending the process leaves nothing to repair.
#[derive(Debug)]
pub(crate) struct OmegaSend {
    text: String,
    transcript: Option<PathBuf>,
    route_receipt: Option<PathBuf>,
    quit: bool,
    timeout: std::time::Duration,
}

static OMEGA_SEND: OnceLock<Option<OmegaSend>> = OnceLock::new();

/// Record `--omega-send` and its companions, once.
fn read_omega_send_from_command_line(args: &Args) {
    let _ = OMEGA_SEND.set(args.omega_send.clone().map(|text| OmegaSend {
        text,
        transcript: args.omega_send_transcript.clone(),
        route_receipt: args.omega_route_receipt.clone(),
        quit: args.omega_quit_after_send,
        timeout: std::time::Duration::from_secs(args.omega_send_timeout_secs),
    }));
}

/// The send this process was started to make, if it was started to make one.
fn omega_send_from_command_line() -> Option<&'static OmegaSend> {
    OMEGA_SEND.get()?.as_ref()
}

/// Send one message on the thread this process opened, and wait for the turn.
///
/// `OMEGA-DELTA-0093`, omega#100. The whole sequence a person would do by hand
/// — wait for the window, wait for the thread, type, press Enter, watch the
/// turn finish, read the transcript — with nobody present for any of it.
///
/// Everything here waits by polling with a deadline rather than by sleeping for
/// a duration somebody guessed. Connecting an external agent and completing ACP
/// initialization are real I/O whose length is a property of the machine, and a
/// fixed sleep is how an unattended run becomes either slow or flaky depending
/// on the day.
///
/// The send itself is [`AgentPanel::omega_send_first_message`], which is a thin
/// wrapper over the call the Git panel's "review this branch diff" action
/// already makes. Nothing in this function talks to a connection, builds a
/// prompt, or touches an `AcpThread` except to read its status: a control
/// surface that bypassed the production path would prove nothing about the
/// production path.
async fn drive_omega_send(cx: &mut AsyncApp) {
    let Some(send) = omega_send_from_command_line() else {
        return;
    };
    let deadline = Instant::now() + send.timeout;
    let outcome = run_omega_send(send, deadline, cx).await;
    let logged_refusals = omega_zero_base::logged_refusal_count();
    let outcome = if omega_zero_base::proof_is_refusal_free(logged_refusals) {
        outcome
    } else {
        Err(anyhow::anyhow!(
            "--omega-send logged {logged_refusals} refused action(s)"
        ))
    };
    match &outcome {
        Ok(()) => log::info!("OMEGA-DELTA-0093: the driven turn completed"),
        Err(error) => log::error!("OMEGA-DELTA-0093: the driven turn did not complete: {error:#}"),
    }
    if send.quit {
        // Exit status, not a log line: an unattended run has to be a command a
        // script can branch on. A window that stayed alive is not a turn that
        // happened, and that mistake has been made here before.
        let code = i32::from(outcome.is_err());
        log::info!("OMEGA-DELTA-0093: quitting after the driven turn, status {code}");
        cx.update(|cx| cx.quit());
        process::exit(code);
    }
}

async fn run_omega_send(send: &OmegaSend, deadline: Instant, cx: &mut AsyncApp) -> Result<()> {
    let window = omega_send_wait_for(deadline, cx, "a workspace window", |cx| {
        cx.windows()
            .into_iter()
            .find_map(|handle| handle.downcast::<workspace::Workspace>())
    })
    .await?;

    let panel = omega_send_wait_for(deadline, cx, "the agent panel", |cx| {
        window
            .read_with(cx, |workspace, cx| workspace.panel::<AgentPanel>(cx))
            .ok()
            .flatten()
    })
    .await?;

    // `omega_send_first_message` refuses when the panel has no project, and the
    // refusal is reported rather than retried: a thread whose file tools have
    // no worktree is `OMEGA-DELTA-0054`'s failure, and waiting for it to stop
    // being true would just spend the whole budget.
    let opened = cx.update_window(window.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.omega_send_first_message(send.text.clone(), window, cx)
        })
    })?;
    anyhow::ensure!(
        opened,
        "no thread was opened: the panel has no project, so the thread's file \
         tools would have had nothing to operate on"
    );

    let thread = omega_send_wait_for(deadline, cx, "the thread to connect", |cx| {
        panel.read(cx).omega_active_acp_thread(cx)
    })
    .await?;

    // Idle *after* generating, never the idle before it. A thread is idle for
    // the moment between being built and the turn starting, so a plain "wait
    // for idle" reports a completed turn before the first token — which is a
    // green unattended run that proves nothing, the exact failure this whole
    // deliverable exists to stop being possible.
    omega_send_wait_for(deadline, cx, "the turn to start", |cx| {
        (thread.read(cx).status() != acp_thread::ThreadStatus::Idle).then_some(())
    })
    .await?;
    omega_send_wait_for(deadline, cx, "the turn to finish", |cx| {
        (thread.read(cx).status() == acp_thread::ThreadStatus::Idle).then_some(())
    })
    .await?;

    if let Some(path) = &send.transcript {
        let transcript = cx.update(|cx| thread.read(cx).to_markdown(cx));
        std::fs::write(path, transcript)
            .with_context(|| format!("writing the transcript to {}", path.display()))?;
        log::info!("OMEGA-DELTA-0093: transcript written to {}", path.display());
    }
    if let Some(path) = &send.route_receipt {
        let receipt = cx.update(|cx| {
            let session_id = thread.read(cx).session_id();
            agent_ui::omega_router::recorded_route_receipt(session_id)
        });
        let receipt = receipt.context("the completed Omega turn has no durable route receipt")?;
        std::fs::write(path, receipt.canonical_record())
            .with_context(|| format!("writing the route receipt to {}", path.display()))?;
        log::info!(
            "OMEGA-DELTA-0179: route receipt written to {}",
            path.display()
        );
    }
    Ok(())
}

/// Poll `look` until it answers or the deadline passes.
///
/// `what` is what the caller was waiting for, and it is in the error rather
/// than in a log line, because "the driven turn timed out" and "the driven turn
/// timed out waiting for the agent panel" send a reader to different places.
async fn omega_send_wait_for<T>(
    deadline: Instant,
    cx: &mut AsyncApp,
    what: &str,
    mut look: impl FnMut(&mut App) -> Option<T>,
) -> Result<T> {
    loop {
        if let Some(found) = cx.update(&mut look) {
            return Ok(found);
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "timed out waiting for {what}; raise --omega-send-timeout-secs if \
             this machine is simply slow"
        );
        cx.background_executor()
            .timer(std::time::Duration::from_millis(50))
            .await;
    }
}

async fn restorable_workspaces(
    cx: &mut AsyncApp,
    app_state: &Arc<AppState>,
) -> Option<Vec<workspace::SerializedMultiWorkspace>> {
    let locations = restorable_workspace_locations(cx, app_state).await?;
    Some(cx.update(|cx| workspace::read_serialized_multi_workspaces(locations, cx)))
}

pub(crate) async fn restorable_workspace_locations(
    cx: &mut AsyncApp,
    app_state: &Arc<AppState>,
) -> Option<Vec<SessionWorkspace>> {
    let (mut restore_behavior, db) = cx.update(|cx| {
        (
            WorkspaceSettings::get(None, cx).restore_on_startup,
            workspace::WorkspaceDb::global(cx),
        )
    });

    let session_handle = app_state.session.clone();
    let (last_session_id, last_session_window_stack) = cx.update(|cx| {
        let session = session_handle.read(cx);

        (
            session.last_session_id().map(|id| id.to_string()),
            session.last_session_window_stack(),
        )
    });

    if last_session_id.is_none()
        && matches!(
            restore_behavior,
            workspace::RestoreOnStartupBehavior::LastSession
        )
    {
        restore_behavior = workspace::RestoreOnStartupBehavior::LastWorkspace;
    }

    match restore_behavior {
        workspace::RestoreOnStartupBehavior::LastWorkspace => {
            workspace::last_opened_workspace_location(&db, app_state.fs.as_ref())
                .await
                .map(|(workspace_id, location, paths)| {
                    vec![SessionWorkspace {
                        workspace_id,
                        location,
                        paths,
                        window_id: None,
                    }]
                })
        }
        workspace::RestoreOnStartupBehavior::LastSession => {
            if let Some(last_session_id) = last_session_id {
                let ordered = last_session_window_stack.is_some();

                let mut locations = workspace::last_session_workspace_locations(
                    &db,
                    &last_session_id,
                    last_session_window_stack,
                    app_state.fs.as_ref(),
                )
                .await
                .filter(|locations| !locations.is_empty());

                // Since last_session_window_order returns the windows ordered front-to-back
                // we need to open the window that was frontmost last.
                if ordered && let Some(locations) = locations.as_mut() {
                    locations.reverse();
                }

                locations
            } else {
                None
            }
        }
        _ => None,
    }
}

fn init_paths() -> HashMap<io::ErrorKind, Vec<&'static Path>> {
    [
        paths::config_dir(),
        paths::extensions_dir(),
        paths::languages_dir(),
        paths::debug_adapters_dir(),
        paths::database_dir(),
        paths::logs_dir(),
        paths::temp_dir(),
        paths::hang_traces_dir(),
    ]
    .into_iter()
    .fold(HashMap::default(), |mut errors, path| {
        if let Err(e) = std::fs::create_dir_all(path) {
            errors.entry(e.kind()).or_insert_with(Vec::new).push(path);
        }
        errors
    })
}

pub(crate) static FORCE_CLI_MODE: LazyLock<bool> = LazyLock::new(|| {
    let env_var = std::env::var(FORCE_CLI_MODE_ENV_VAR_NAME).ok().is_some();
    unsafe { std::env::remove_var(FORCE_CLI_MODE_ENV_VAR_NAME) };
    env_var
});

fn stdout_is_a_pty() -> bool {
    !*FORCE_CLI_MODE && io::stdout().is_terminal()
}

#[derive(Parser, Debug)]
#[command(name = "omega", disable_version_flag = true, max_term_width = 100)]
struct Args {
    /// A sequence of space-separated paths or urls that you want to open.
    ///
    /// Use `path:line:row` syntax to open a file at a specific location.
    /// Non-existing paths and directories will ignore `:line:row` suffix.
    ///
    /// URLs can use `file://`, the current Omega channel scheme, or the legacy `zed://` scheme.
    ///
    /// OMEGA-DELTA-0116. A path names the folder the thread works in — a file
    /// argument names the folder that holds it — and it never opens an editor
    /// pane.
    paths_or_urls: Vec<String>,

    /// Accepted and ignored: the surface this flag used to name is what Omega
    /// always does now.
    ///
    /// OMEGA-DELTA-0052, omega#161. Kept so commands and scripts that carry it
    /// keep working.
    #[arg(long)]
    zero_base: bool,

    /// Accepted and ignored: the primary Omega interface is always active.
    #[arg(long, hide = true)]
    primary_interface: bool,

    /// Enables the optional Exo integration for this process.
    ///
    /// OMEGA-DELTA-0144. Exo is otherwise absent: Omega does not inspect its
    /// configuration, offer it in the UI, or attempt to connect to it.
    #[arg(long)]
    enable_exo: bool,

    /// Sends one message on the thread Omega opens, with nobody at the keyboard.
    ///
    /// OMEGA-DELTA-0093. The text is put in the composer and submitted through
    /// the same call the Enter key reaches, so what this drives is the shipped
    /// send and not a second one beside it. Synthetic keystrokes are unusable
    /// on a busy desktop — twice, keys meant for Omega landed in another
    /// application — and every visual claim about a turn depends on being able
    /// to send one without the window having focus.
    #[arg(long, value_name = "TEXT")]
    omega_send: Option<String>,

    /// Writes the thread's transcript here once the turn settles.
    ///
    /// OMEGA-DELTA-0093. What a script checks. A window that stayed alive is
    /// not a turn that happened, and a transcript is the smallest artefact that
    /// tells the two apart without a screen capture.
    #[arg(long, value_name = "PATH", requires = "omega_send")]
    omega_send_transcript: Option<PathBuf>,

    /// Writes the thread's durable route receipt here once the turn settles.
    #[arg(long, value_name = "PATH", requires = "omega_send")]
    omega_route_receipt: Option<PathBuf>,

    /// Quits when the turn started by --omega-send settles.
    ///
    /// OMEGA-DELTA-0093. Exit status 0 when the turn completed, non-zero when
    /// it did not — so an unattended run is a command a script can branch on
    /// rather than a process somebody has to go and look at.
    #[arg(long, requires = "omega_send")]
    omega_quit_after_send: bool,

    /// How long to wait for that turn before giving up, in seconds.
    #[arg(
        long,
        value_name = "SECONDS",
        default_value = "300",
        requires = "omega_send"
    )]
    omega_send_timeout_secs: u64,

    /// Sets a custom directory for all user data (e.g., database, extensions, logs).
    ///
    /// This overrides the default platform-specific data directory location.
    /// On macOS, the default is `~/Library/Application Support/<Omega channel>`.
    /// On Linux/FreeBSD, the default is `$XDG_DATA_HOME/<omega-channel>`.
    /// On Windows, the default is `%LOCALAPPDATA%\<Omega channel>`.
    #[arg(long, value_name = "DIR", verbatim_doc_comment)]
    user_data_dir: Option<String>,

    /// The username and WSL distribution to use when opening paths. If not specified,
    /// Omega will attempt to open the paths directly.
    ///
    /// The username is optional, and if not specified, the default user for the distribution
    /// will be used.
    ///
    /// Example: `me@Ubuntu` or `Ubuntu`.
    ///
    /// WARN: You should not fill in this field by hand.
    #[cfg(target_os = "windows")]
    #[arg(long, value_name = "USER@DISTRO")]
    wsl: Option<String>,

    /// Instructs Omega to run as a dev server on this machine. (not implemented)
    #[arg(long)]
    dev_server_token: Option<String>,

    /// Prints system specs.
    ///
    /// Useful for submitting issues on GitHub when encountering a bug that
    /// prevents Omega from starting, so you can't run `omega: copy system specs to
    /// clipboard`
    #[arg(long)]
    system_specs: bool,

    /// Used for recording minidumps on crashes by having Omega run a separate
    /// process communicating over a socket.
    #[arg(long, hide = true)]
    crash_handler: Option<PathBuf>,

    /// Run Omega in the foreground, only used on Windows, to match the behavior on macOS.
    #[arg(long)]
    #[cfg(target_os = "windows")]
    #[arg(hide = true)]
    foreground: bool,

    /// The dock action to perform. This is used on Windows only.
    #[arg(long)]
    #[cfg(target_os = "windows")]
    #[arg(hide = true)]
    dock_action: Option<usize>,

    /// Used for SSH/Git password authentication, to remove the need for netcat as a dependency,
    /// by having Omega act like netcat communicating over a Unix socket.
    #[arg(long)]
    #[cfg(not(target_os = "windows"))]
    #[arg(hide = true)]
    askpass: Option<String>,

    #[arg(long, hide = true)]
    dump_all_actions: bool,

    /// Output current environment variables as JSON to stdout
    #[arg(long, hide = true)]
    printenv: bool,

    /// Record an ETW trace. Must be run as administrator.
    #[cfg(target_os = "windows")]
    #[arg(long, hide = true)]
    record_etw_trace: bool,

    /// The PID of the Omega process to trace for heap analysis.
    #[cfg(target_os = "windows")]
    #[arg(long, hide = true, allow_hyphen_values = true)]
    etw_zed_pid: Option<i64>,

    /// Output path for the ETW trace file.
    #[cfg(target_os = "windows")]
    #[arg(long, hide = true)]
    etw_output: Option<PathBuf>,

    /// Unix socket path for IPC with the parent Omega process.
    #[cfg(target_os = "windows")]
    #[arg(long, hide = true)]
    etw_socket: Option<String>,
}

#[derive(Clone, Debug)]
enum IdType {
    New(String),
    Existing(String),
}

impl ToString for IdType {
    fn to_string(&self) -> String {
        match self {
            IdType::New(id) | IdType::Existing(id) => id.clone(),
        }
    }
}

fn parse_url_arg(arg: &str, cx: &App) -> String {
    match std::fs::canonicalize(Path::new(&arg)) {
        Ok(path) => format!("file://{}", path.display()),
        Err(_) => {
            if arg.starts_with("file://")
                || arg.starts_with("zed://")
                || arg.starts_with("zed-cli://")
                || arg.starts_with("ssh://")
                || arg.starts_with(&format!(
                    "{}://",
                    ReleaseChannel::try_global(cx)
                        .unwrap_or(*release_channel::RELEASE_CHANNEL)
                        .protocol_scheme()
                ))
                || parse_zed_link(arg, cx).is_some()
            {
                arg.into()
            } else {
                format!("file://{arg}")
            }
        }
    }
}

fn load_embedded_fonts(cx: &App) {
    let asset_source = cx.asset_source();
    let font_paths = asset_source.list("fonts").unwrap();
    let embedded_fonts = Mutex::new(Vec::new());
    let executor = cx.background_executor();

    cx.foreground_executor().block_on(executor.scoped(|scope| {
        for font_path in &font_paths {
            if !font_path.ends_with(".ttf") {
                continue;
            }

            scope.spawn(async {
                let font_bytes = asset_source.load(font_path).unwrap().unwrap();
                embedded_fonts.lock().push(font_bytes);
            });
        }
    }));

    cx.text_system()
        .add_fonts(embedded_fonts.into_inner())
        .unwrap();
}

/// Spawns a background task to load the user themes from the themes directory.
fn load_user_themes_in_background(fs: Arc<dyn fs::Fs>, cx: &mut App) {
    cx.spawn({
        let fs = fs.clone();
        async move |cx| {
            let theme_registry = cx.update(|cx| ThemeRegistry::global(cx));
            let themes_dir = paths::themes_dir().as_ref();
            match fs
                .metadata(themes_dir)
                .await
                .ok()
                .flatten()
                .map(|m| m.is_dir)
            {
                Some(is_dir) => {
                    anyhow::ensure!(is_dir, "Themes dir path {themes_dir:?} is not a directory")
                }
                None => {
                    fs.create_dir(themes_dir).await.with_context(|| {
                        format!("Failed to create themes dir at path {themes_dir:?}")
                    })?;
                }
            }

            let mut theme_paths = fs
                .read_dir(themes_dir)
                .await
                .with_context(|| format!("reading themes from {themes_dir:?}"))?;

            while let Some(theme_path) = theme_paths.next().await {
                let Some(theme_path) = theme_path.log_err() else {
                    continue;
                };
                let Some(bytes) = fs.load_bytes(&theme_path).await.log_err() else {
                    continue;
                };

                load_user_theme(&theme_registry, &bytes).log_err();
            }

            cx.update(theme_settings::reload_theme);
            anyhow::Ok(())
        }
    })
    .detach_and_log_err(cx);
}

/// Spawns a background task to watch the themes directory for changes.
fn watch_themes(fs: Arc<dyn fs::Fs>, cx: &mut App) {
    use std::time::Duration;
    cx.spawn(async move |cx| {
        let (mut events, _) = fs
            .watch(paths::themes_dir(), Duration::from_millis(100))
            .await;

        while let Some(paths) = events.next().await {
            for event in paths {
                if fs
                    .metadata(&event.path)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|m| !m.is_dir)
                {
                    let theme_registry = cx.update(|cx| ThemeRegistry::global(cx));
                    if let Some(bytes) = fs.load_bytes(&event.path).await.log_err()
                        && load_user_theme(&theme_registry, &bytes).log_err().is_some()
                    {
                        cx.update(theme_settings::reload_theme);
                    }
                }
            }
        }
    })
    .detach()
}

#[cfg(debug_assertions)]
fn watch_languages(fs: Arc<dyn fs::Fs>, languages: Arc<LanguageRegistry>, cx: &mut App) {
    use std::time::Duration;

    cx.background_spawn(async move {
        let languages_src = Path::new("crates/grammars/src");
        let Some(languages_src) = fs.canonicalize(languages_src).await.log_err() else {
            return;
        };

        let (mut events, watcher) = fs.watch(&languages_src, Duration::from_millis(100)).await;

        // add subdirectories since fs.watch is not recursive on Linux
        if let Some(mut paths) = fs.read_dir(&languages_src).await.log_err() {
            while let Some(path) = paths.next().await {
                if let Some(path) = path.log_err()
                    && fs.is_dir(&path).await
                {
                    watcher.add(&path).log_err();
                }
            }
        }

        while let Some(event) = events.next().await {
            let has_language_file = event
                .iter()
                .any(|event| event.path.extension().is_some_and(|ext| ext == "scm"));
            if has_language_file {
                languages.reload();
            }
        }
    })
    .detach();
}

fn dump_all_gpui_actions() {
    #[derive(Debug, serde::Serialize)]
    struct ActionDef {
        name: &'static str,
        human_name: String,
        schema: Option<serde_json::Value>,
        deprecated_aliases: &'static [&'static str],
        deprecation_message: Option<&'static str>,
        documentation: Option<&'static str>,
    }
    let mut generator = settings::KeymapFile::action_schema_generator();
    let mut actions = gpui::generate_list_of_all_registered_actions()
        .map(|action| {
            let schema = (action.json_schema)(&mut generator)
                .map(|s| serde_json::to_value(s).expect("Failed to serialize action schema"));
            ActionDef {
                name: action.name,
                human_name: command_palette::humanize_action_name(action.name),
                schema,
                deprecated_aliases: action.deprecated_aliases,
                deprecation_message: action.deprecation_message,
                documentation: action.documentation,
            }
        })
        .collect::<Vec<ActionDef>>();

    actions.sort_by_key(|a| a.name);

    let schema_definitions = serde_json::to_value(generator.definitions())
        .expect("Failed to serialize schema definitions");

    let output = serde_json::json!({
        "actions": actions,
        "schema_definitions": schema_definitions,
    });

    io::Write::write(
        &mut std::io::stdout(),
        serde_json::to_string_pretty(&output).unwrap().as_bytes(),
    )
    .unwrap();
}

#[cfg(target_os = "windows")]
fn check_for_conpty_dll() {
    use windows::{
        Win32::{Foundation::FreeLibrary, System::LibraryLoader::LoadLibraryW},
        core::w,
    };

    if let Ok(hmodule) = unsafe { LoadLibraryW(w!("conpty.dll")) } {
        unsafe {
            FreeLibrary(hmodule)
                .context("Failed to free conpty.dll")
                .log_err();
        }
    } else {
        log::warn!("Failed to load conpty.dll. Terminal will work with reduced functionality.");
    }
}
